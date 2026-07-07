use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
};

use directories::ProjectDirs;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command as TokioCommand,
    sync::{mpsc, oneshot},
};

use crate::app::DownloadMode;

const PROGRESS_PREFIX: &str = "crusty-dlp:";

#[derive(Debug)]
pub enum DownloadEvent {
    Progress { percent: Option<f64>, text: String },
    Finished,
    Failed(String),
    Cancelled,
}

pub struct Downloader {
    executable: PathBuf,
    output_dir: PathBuf,
    plugin_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    pub title: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Copy)]
pub struct DownloadOptions<'a> {
    pub impersonation: Option<&'a str>,
    pub cookies_browser: Option<&'a str>,
    pub concurrent_fragments: u8,
    pub use_aria2: bool,
    pub output_template: Option<&'a str>,
    pub rate_limit: Option<&'a str>,
    pub allow_playlists: bool,
}

impl Default for DownloadOptions<'_> {
    fn default() -> Self {
        Self {
            impersonation: None,
            cookies_browser: None,
            concurrent_fragments: 4,
            use_aria2: false,
            output_template: None,
            rate_limit: None,
            allow_playlists: false,
        }
    }
}

impl Downloader {
    pub fn new(executable: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            executable,
            output_dir,
            plugin_dir: plugin_directory(),
        }
    }

    pub fn arguments(
        &self,
        url: &str,
        mode: &DownloadMode,
        options: DownloadOptions<'_>,
    ) -> Vec<OsString> {
        let mut args = vec![
            OsString::from("--newline"),
            OsString::from("--progress-template"),
            OsString::from("download:crusty-dlp:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s"),
            OsString::from("--output"),
            self.output_dir
                .join(
                    options
                        .output_template
                        .unwrap_or("%(title)s [%(id)s].%(ext)s"),
                )
                .into_os_string(),
        ];
        if !options.allow_playlists {
            args.push(OsString::from("--no-playlist"));
        }
        if let Some(plugin_dir) = &self.plugin_dir {
            args.push(OsString::from("--plugin-dirs"));
            args.push(plugin_dir.as_os_str().to_owned());
        }
        args.push(OsString::from("--concurrent-fragments"));
        args.push(OsString::from(options.concurrent_fragments.to_string()));
        if let Some(rate_limit) = options.rate_limit {
            args.push(OsString::from("--limit-rate"));
            args.push(OsString::from(rate_limit));
        }
        if options.use_aria2 {
            args.extend([
                OsString::from("--downloader"),
                OsString::from("http,ftp:aria2c"),
                OsString::from("--downloader-args"),
                OsString::from(format!(
                    "aria2c:-x {} -s {} -k 1M",
                    options.concurrent_fragments, options.concurrent_fragments
                )),
            ]);
        }
        match mode {
            DownloadMode::Video => {
                args.extend(["--format".into(), "bestvideo*+bestaudio/best".into()])
            }
            DownloadMode::Audio => args.extend([
                "--format".into(),
                "bestaudio/best".into(),
                "--extract-audio".into(),
            ]),
            DownloadMode::Mp3 => args.extend([
                "--format".into(),
                "bestaudio/best".into(),
                "--extract-audio".into(),
                "--audio-format".into(),
                "mp3".into(),
            ]),
            DownloadMode::Custom(format) => args.extend(["--format".into(), format.into()]),
        }
        if let Some(target) = options.impersonation {
            args.push(OsString::from("--impersonate"));
            args.push(OsString::from(if target == "any" { "" } else { target }));
        }
        if let Some(browser) = options.cookies_browser {
            args.push(OsString::from("--cookies-from-browser"));
            args.push(OsString::from(browser));
        }
        args.push(OsString::from("--"));
        args.push(OsString::from(url));
        args
    }

    pub fn display_command(&self, args: &[OsString]) -> String {
        std::iter::once(self.executable.as_os_str())
            .chain(args.iter().map(OsString::as_os_str))
            .map(|arg| format!("{arg:?}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub async fn run(
        self,
        args: Vec<OsString>,
        mut cancel: oneshot::Receiver<()>,
        tx: mpsc::UnboundedSender<DownloadEvent>,
    ) {
        let mut child = match TokioCommand::new(&self.executable)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let _ = tx.send(DownloadEvent::Failed(format!(
                    "could not start yt-dlp: {error}"
                )));
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");
        let progress_tx = tx.clone();
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(event) = parse_progress(&line) {
                    let _ = progress_tx.send(event);
                }
            }
        });
        let error_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut last = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    last = line;
                }
            }
            last
        });

        tokio::select! {
            status = child.wait() => {
                let _ = stdout_task.await;
                let error = error_task.await.unwrap_or_default();
                match status {
                    Ok(status) if status.success() => { let _ = tx.send(DownloadEvent::Finished); }
                    Ok(status) => { let _ = tx.send(DownloadEvent::Failed(if error.is_empty() { format!("yt-dlp exited with {status}") } else { error })); }
                    Err(error) => { let _ = tx.send(DownloadEvent::Failed(format!("could not wait for yt-dlp: {error}"))); }
                }
            }
            _ = &mut cancel => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                stdout_task.abort();
                error_task.abort();
                let _ = tx.send(DownloadEvent::Cancelled);
            }
        }
    }
}

fn plugin_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let current_directory = std::env::current_dir().ok();
    let user_data_directory = ProjectDirs::from("org", "crusty-dlp", "crusty-dlp")
        .map(|dirs| dirs.data_local_dir().to_path_buf());
    #[cfg(unix)]
    let system_directory = Some(PathBuf::from("/usr/share/crusty-dlp"));
    #[cfg(not(unix))]
    let system_directory: Option<PathBuf> = None;
    current_directory
        .into_iter()
        .chain(directory.ancestors().take(5).map(Path::to_owned))
        .chain(user_data_directory)
        .chain(system_directory)
        .find(|path| path.join("plugins/yt_dlp_plugins/extractor").is_dir())
}

pub fn resolved_plugin_directory() -> Option<PathBuf> {
    plugin_directory()
}

pub fn current_executable_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Ask yt-dlp itself which impersonation targets are usable. This avoids
/// assuming that an installed Python package is visible to every yt-dlp build.
pub fn available_impersonation_targets(executable: &Path) -> Vec<String> {
    let Ok(output) = StdCommand::new(executable)
        .arg("--list-impersonate-targets")
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_impersonation_targets(&String::from_utf8_lossy(&output.stdout))
}

fn parse_impersonation_targets(stdout: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in stdout.lines() {
        if line.contains("unavailable") {
            continue;
        }
        let Some(client) = line.split_whitespace().next() else {
            continue;
        };
        if matches!(client, "Client" | "---" | "[info]") {
            continue;
        }
        let client = client.to_ascii_lowercase();
        if !targets.contains(&client) {
            targets.push(client);
        }
    }
    targets
}

fn parse_progress(line: &str) -> Option<DownloadEvent> {
    let content = line.strip_prefix(PROGRESS_PREFIX)?;
    let percent = content
        .split('|')
        .next()
        .and_then(|value| value.trim().trim_end_matches('%').parse().ok());
    Some(DownloadEvent::Progress {
        percent,
        text: content.trim().to_owned(),
    })
}

pub fn dependency_path(name: &str) -> Option<PathBuf> {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            #[cfg(windows)]
            let candidate = directory.join(format!("{name}.exe"));
            #[cfg(not(windows))]
            let candidate = directory.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        #[cfg(windows)]
        let candidate = dir.join(format!("{name}.exe"));
        #[cfg(not(windows))]
        let candidate = dir.join(name);
        is_executable(&candidate).then_some(candidate)
    })
}

pub fn supports_playlist_expansion(url: &str) -> bool {
    is_youtube_playlist_url(url) || is_pmvhaven_playlist_url(url) || is_spankbang_playlist_url(url)
}

pub fn expand_playlist_urls(
    executable: &Path,
    url: &str,
) -> Result<Option<Vec<PlaylistEntry>>, String> {
    if !supports_playlist_expansion(url) {
        return Ok(None);
    }

    if is_pmvhaven_playlist_url(url) {
        return expand_pmvhaven_playlist_urls(url);
    }

    let mut command = StdCommand::new(executable);
    command.args(playlist_probe_arguments());
    if let Some(plugin_dir) = plugin_directory() {
        command.arg("--plugin-dirs").arg(plugin_dir);
    }
    let output = command
        .arg(url)
        .output()
        .map_err(|error| format!("could not inspect playlist: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("yt-dlp could not inspect the playlist");
        return Err(message.to_owned());
    }

    let source = playlist_source(url);
    let entries = dedupe_playlist_entries(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| parse_flat_playlist_line(line, source))
            .collect::<Vec<_>>(),
    );
    if entries.is_empty() {
        return Err(format!("playlist inspection found 0 entries for {url}"));
    }
    Ok(Some(entries))
}

fn playlist_probe_arguments() -> [&'static str; 4] {
    [
        "--flat-playlist",
        "--print",
        "%(title)s\t%(id)s\t%(webpage_url)s\t%(url)s",
        "--",
    ]
}

fn expand_pmvhaven_playlist_urls(url: &str) -> Result<Option<Vec<PlaylistEntry>>, String> {
    let output = StdCommand::new("curl")
        .args(["-fsSL", "--"])
        .arg(url)
        .output()
        .map_err(|error| format!("could not inspect PMVHaven playlist: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("curl could not inspect the PMVHaven playlist");
        return Err(message.to_owned());
    }

    let html = String::from_utf8_lossy(&output.stdout);
    let entries = dedupe_playlist_entries(parse_pmvhaven_itemlist_entries(&html));
    if entries.is_empty() {
        return Err(format!(
            "PMVHaven playlist parser found 0 entries for {url}"
        ));
    }
    Ok(Some(entries))
}

fn parse_pmvhaven_itemlist_entries(html: &str) -> Vec<PlaylistEntry> {
    let mut entries = Vec::new();
    let name_key = r#""name":"#;
    let embed_key = r#""embedUrl":"#;
    let Some(list_start) = html.find(r#""itemListElement""#) else {
        return entries;
    };
    let mut cursor = &html[list_start..];

    while let Some((before_embed, after_embed)) = cursor.split_once(embed_key) {
        let Some((embed_url_raw, rest)) = after_embed.split_once('"') else {
            break;
        };
        let title = before_embed
            .rsplit(name_key)
            .next()
            .and_then(|fragment| fragment.split_once('"').map(|(value, _)| value.to_owned()));
        let embed_url = decode_json_escapes(&embed_url_raw);
        if embed_url.starts_with("http://") || embed_url.starts_with("https://") {
            entries.push(PlaylistEntry {
                title: title.map(|value| decode_json_escapes(&value)),
                url: embed_url,
            });
        }
        cursor = rest;
    }
    entries
}

fn decode_json_escapes(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => output.push('"'),
            Some('\\') => output.push('\\'),
            Some('/') => output.push('/'),
            Some('b') => output.push('\u{0008}'),
            Some('f') => output.push('\u{000C}'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Ok(codepoint) = u32::from_str_radix(&hex, 16) {
                    if let Some(decoded) = char::from_u32(codepoint) {
                        output.push(decoded);
                    }
                }
            }
            Some(other) => output.push(other),
            None => break,
        }
    }
    output
}

fn dedupe_playlist_entries(entries: Vec<PlaylistEntry>) -> Vec<PlaylistEntry> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for entry in entries {
        if seen.insert(entry.url.clone()) {
            deduped.push(entry);
        }
    }
    deduped
}

pub fn validate_output_template(template: &str) -> Result<(), String> {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return Err("Filename template cannot be empty".into());
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err("Filename template must stay inside the output folder".into());
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return Err("Filename template cannot use absolute paths or '..'".into());
    }
    Ok(())
}

pub fn validate_rate_limit(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let Some(first) = trimmed.chars().next() else {
        return Ok(());
    };
    if !first.is_ascii_digit() {
        return Err("Speed limit must start with a number, for example 5M or 800K".into());
    }

    let mut seen_dot = false;
    let mut suffix = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            if !suffix.is_empty() {
                return Err("Speed limit suffix must appear at the end".into());
            }
            continue;
        }
        if ch == '.' {
            if seen_dot || !suffix.is_empty() {
                return Err("Speed limit can contain at most one decimal point".into());
            }
            seen_dot = true;
            continue;
        }
        suffix.push(ch);
    }

    let suffix = suffix.to_ascii_lowercase();
    if matches!(
        suffix.as_str(),
        "" | "k" | "m" | "g" | "t" | "ki" | "mi" | "gi" | "ti"
    ) {
        Ok(())
    } else {
        Err("Use yt-dlp suffixes such as K, M, G, Ki, or Mi".into())
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn url_host_matches(value: &str, expected: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    host == expected || host.ends_with(&format!(".{expected}"))
}

fn parse_flat_playlist_line(line: &str, source: PlaylistSource) -> Option<PlaylistEntry> {
    let mut fields = line.splitn(4, '\t');
    let title = fields.next().and_then(non_empty_trimmed).map(str::to_owned);
    let id = fields.next().and_then(non_empty_trimmed);
    let webpage_url = fields.next().and_then(non_empty_trimmed);
    let media_url = fields.next().and_then(non_empty_trimmed);

    webpage_url
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .map(str::to_owned)
        .or_else(|| {
            media_url
                .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
                .map(str::to_owned)
        })
        .or_else(|| fallback_playlist_entry_url(source, id))
        .map(|url| PlaylistEntry { title, url })
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn is_youtube_playlist_url(url: &str) -> bool {
    (url_host_matches(url, "youtube.com") || url_host_matches(url, "youtu.be"))
        && (url.contains("list=") || url.contains("/playlist"))
}

fn is_pmvhaven_playlist_url(url: &str) -> bool {
    url_host_matches(url, "pmvhaven.com") && url.contains("/playlists/")
}

fn is_spankbang_playlist_url(url: &str) -> bool {
    url_host_matches(url, "spankbang.com") && url.contains("/playlist/")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaylistSource {
    YouTube,
    PmvHaven,
    SpankBang,
}

fn playlist_source(url: &str) -> PlaylistSource {
    if is_pmvhaven_playlist_url(url) {
        PlaylistSource::PmvHaven
    } else if is_spankbang_playlist_url(url) {
        PlaylistSource::SpankBang
    } else {
        PlaylistSource::YouTube
    }
}

fn fallback_playlist_entry_url(source: PlaylistSource, id: Option<&str>) -> Option<String> {
    let id = id?;
    match source {
        PlaylistSource::YouTube => Some(format!("https://www.youtube.com/watch?v={id}")),
        PlaylistSource::PmvHaven => Some(format!("https://pmvhaven.com/video/{id}")),
        PlaylistSource::SpankBang => Some(format!("https://spankbang.com/{id}/video/{id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_arguments_are_separate_and_url_is_last() {
        let downloader = Downloader::new("yt-dlp".into(), "/tmp/out".into());
        let args = downloader.arguments(
            "https://example.com/watch?v=x",
            &DownloadMode::Mp3,
            DownloadOptions::default(),
        );
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--audio-format" && pair[1] == "mp3"));
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args.last().unwrap(), "https://example.com/watch?v=x");
    }

    #[test]
    fn custom_format_is_one_argument() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader.arguments(
            "https://example.com/x",
            &DownloadMode::Custom("18; rm -rf /".into()),
            DownloadOptions::default(),
        );
        assert!(args.contains(&OsString::from("18; rm -rf /")));
    }

    #[test]
    fn impersonation_is_passed_as_separate_arguments() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader.arguments(
            "https://example.com/x",
            &DownloadMode::Video,
            DownloadOptions {
                impersonation: Some("chrome"),
                ..DownloadOptions::default()
            },
        );
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--impersonate" && pair[1] == "chrome"));
    }

    #[test]
    fn parses_only_available_impersonation_targets() {
        let output = "[info] Available impersonate targets\nClient OS Source\n---\nChrome - curl_cffi\nChrome windows curl_cffi\nFirefox - curl_cffi>=0.10\nSafari - curl_cffi (unavailable)\n";
        assert_eq!(
            parse_impersonation_targets(output),
            vec!["chrome", "firefox"]
        );
    }

    #[test]
    fn browser_cookie_source_is_passed_as_one_argument() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader.arguments(
            "https://example.com/x",
            &DownloadMode::Video,
            DownloadOptions {
                cookies_browser: Some("firefox"),
                ..DownloadOptions::default()
            },
        );
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--cookies-from-browser" && pair[1] == "firefox"));
    }

    #[test]
    fn connection_count_and_aria2_are_bounded_arguments() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader.arguments(
            "https://example.com/video.mp4",
            &DownloadMode::Video,
            DownloadOptions {
                concurrent_fragments: 8,
                use_aria2: true,
                ..DownloadOptions::default()
            },
        );
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--concurrent-fragments" && pair[1] == "8"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--downloader" && pair[1] == "http,ftp:aria2c"));
        assert!(args.contains(&OsString::from("aria2c:-x 8 -s 8 -k 1M")));
    }

    #[test]
    fn output_template_and_rate_limit_are_separate_arguments() {
        let downloader = Downloader::new("yt-dlp".into(), "/tmp/out".into());
        let args = downloader.arguments(
            "https://example.com/video.mp4",
            &DownloadMode::Video,
            DownloadOptions {
                output_template: Some("custom/%(title)s.%(ext)s"),
                rate_limit: Some("5M"),
                allow_playlists: true,
                ..DownloadOptions::default()
            },
        );
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--limit-rate" && pair[1] == "5M"));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--output" && pair[1] == "/tmp/out/custom/%(title)s.%(ext)s"
        }));
    }

    #[test]
    fn rejects_unsafe_output_template() {
        assert!(validate_output_template("../bad.%(ext)s").is_err());
        assert!(validate_output_template("/abs/bad.%(ext)s").is_err());
        assert!(validate_output_template("safe/%(title)s.%(ext)s").is_ok());
    }

    #[test]
    fn validates_rate_limit_syntax() {
        assert!(validate_rate_limit("").is_ok());
        assert!(validate_rate_limit("5M").is_ok());
        assert!(validate_rate_limit("1.5Mi").is_ok());
        assert!(validate_rate_limit("fast").is_err());
        assert!(validate_rate_limit("5MBps").is_err());
    }

    #[test]
    fn parses_private_progress_lines() {
        match parse_progress("crusty-dlp: 42.5%|2MiB/s|00:10").unwrap() {
            DownloadEvent::Progress { percent, .. } => assert_eq!(percent, Some(42.5)),
            _ => panic!("wrong event"),
        }
    }

    #[test]
    fn defaults_to_no_playlist() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader.arguments(
            "https://www.youtube.com/playlist?list=abc",
            &DownloadMode::Video,
            DownloadOptions::default(),
        );
        assert!(args.contains(&OsString::from("--no-playlist")));
    }

    #[test]
    fn omits_no_playlist_when_enabled() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader.arguments(
            "https://www.youtube.com/playlist?list=abc",
            &DownloadMode::Video,
            DownloadOptions {
                allow_playlists: true,
                ..DownloadOptions::default()
            },
        );
        assert!(!args.contains(&OsString::from("--no-playlist")));
    }

    #[test]
    fn detects_supported_playlist_hosts() {
        assert!(supports_playlist_expansion(
            "https://www.youtube.com/playlist?list=abc"
        ));
        assert!(supports_playlist_expansion(
            "https://youtu.be/abc123?list=xyz"
        ));
        assert!(supports_playlist_expansion(
            "https://pmvhaven.com/playlists/6a4c1cd09691afef03ece49b"
        ));
        assert!(supports_playlist_expansion(
            "https://spankbang.com/1abc/playlist/test-list"
        ));
        assert!(!supports_playlist_expansion(
            "https://spankbang.com/1abc/video/test-video"
        ));
    }

    #[test]
    fn parses_flat_playlist_entries() {
        let urls = [
            "Title A\tabc123\thttps://www.youtube.com/watch?v=abc123\tabc123",
            "Title B\tdef456\t\tdef456",
            "Title C\tghi789\t\thttps://spankbang.com/ghi789/video/ghi789",
        ]
        .into_iter()
        .filter_map(|line| parse_flat_playlist_line(line, PlaylistSource::YouTube))
        .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                PlaylistEntry {
                    title: Some("Title A".into()),
                    url: "https://www.youtube.com/watch?v=abc123".into(),
                },
                PlaylistEntry {
                    title: Some("Title B".into()),
                    url: "https://www.youtube.com/watch?v=def456".into(),
                },
                PlaylistEntry {
                    title: Some("Title C".into()),
                    url: "https://spankbang.com/ghi789/video/ghi789".into(),
                },
            ]
        );
    }

    #[test]
    fn falls_back_to_site_specific_playlist_urls() {
        assert_eq!(
            parse_flat_playlist_line("Title X\tpmv001\t\t", PlaylistSource::PmvHaven),
            Some(PlaylistEntry {
                title: Some("Title X".into()),
                url: "https://pmvhaven.com/video/pmv001".into(),
            })
        );
        assert_eq!(
            parse_flat_playlist_line("Title Y\tsb001\t\t", PlaylistSource::SpankBang),
            Some(PlaylistEntry {
                title: Some("Title Y".into()),
                url: "https://spankbang.com/sb001/video/sb001".into(),
            })
        );
    }

    #[test]
    fn playlist_probe_arguments_are_stable() {
        assert_eq!(
            playlist_probe_arguments(),
            [
                "--flat-playlist",
                "--print",
                "%(title)s\t%(id)s\t%(webpage_url)s\t%(url)s",
                "--"
            ]
        );
    }

    #[test]
    fn decodes_pmvhaven_embed_urls() {
        assert_eq!(
            decode_json_escapes(
                r#"https:\u002F\u002Fpmvhaven.com\u002Fvideos\u002F68f99987828ec8d0de64bb98"#
            ),
            "https://pmvhaven.com/videos/68f99987828ec8d0de64bb98"
        );
    }

    #[test]
    fn dedupes_playlist_entries_by_url() {
        let entries = vec![
            PlaylistEntry {
                title: Some("First".into()),
                url: "https://example.com/a".into(),
            },
            PlaylistEntry {
                title: Some("Duplicate".into()),
                url: "https://example.com/a".into(),
            },
            PlaylistEntry {
                title: Some("Second".into()),
                url: "https://example.com/b".into(),
            },
        ];
        assert_eq!(
            dedupe_playlist_entries(entries),
            vec![
                PlaylistEntry {
                    title: Some("First".into()),
                    url: "https://example.com/a".into(),
                },
                PlaylistEntry {
                    title: Some("Second".into()),
                    url: "https://example.com/b".into(),
                },
            ]
        );
    }
}
