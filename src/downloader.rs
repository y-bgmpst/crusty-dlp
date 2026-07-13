use std::{
    collections::{HashSet, VecDeque},
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command as TokioCommand,
    sync::{mpsc, oneshot},
};

use crate::{app::DownloadMode, urls};

const PROGRESS_PREFIX: &str = "crusty-dlp:";
const MAX_PLAYLIST_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PLAYLIST_ENTRIES: usize = 5_000;
const MAX_IMPERSONATION_OUTPUT_BYTES: u64 = 512 * 1024;
const IMPERSONATION_TIMEOUT: Duration = Duration::from_secs(8);

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
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct DownloadOptions<'a> {
    pub impersonation: Option<&'a str>,
    pub cookies_browser: Option<&'a str>,
    pub concurrent_fragments: u8,
    pub use_aria2: bool,
    pub output_template: Option<&'a str>,
    pub rate_limit: Option<&'a str>,
    pub socket_timeout: Option<u32>,
    pub retries: Option<u32>,
    pub fragment_retries: Option<u32>,
    pub extractor_args: Option<&'a str>,
    pub playlist_subfolder: Option<&'a str>,
    pub playlist_subfolders: bool,
    pub embed_metadata: bool,
    pub write_info_json: bool,
    pub allow_playlists: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkTuning {
    pub socket_timeout: Option<u32>,
    pub retries: Option<u32>,
    pub fragment_retries: Option<u32>,
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
            socket_timeout: None,
            retries: None,
            fragment_retries: None,
            extractor_args: None,
            playlist_subfolder: None,
            playlist_subfolders: false,
            embed_metadata: false,
            write_info_json: false,
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
    ) -> Result<Vec<OsString>, String> {
        let output_template = options
            .output_template
            .unwrap_or("%(title)s [%(id)s].%(ext)s");
        validate_output_template(output_template)?;
        if let Some(rate_limit) = options.rate_limit {
            validate_rate_limit(rate_limit)?;
        }
        if let Some(extractor_args) = options.extractor_args {
            validate_extractor_args(extractor_args)?;
        }
        let concurrent_fragments = options.concurrent_fragments.clamp(1, 16);
        let output_template = if let Some(folder) = options.playlist_subfolder {
            let folder = sanitize_filename_component(folder)?;
            format!("{folder}/{output_template}")
        } else if options.playlist_subfolders && options.allow_playlists {
            format!("%(playlist_title)s/{output_template}")
        } else {
            output_template.to_owned()
        };
        let mut args = vec![
            OsString::from("--newline"),
            OsString::from("--progress-template"),
            OsString::from("download:crusty-dlp:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s"),
            OsString::from("--output"),
            self.output_dir.join(output_template)
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
        args.push(OsString::from(concurrent_fragments.to_string()));
        if let Some(rate_limit) = options.rate_limit {
            args.push(OsString::from("--limit-rate"));
            args.push(OsString::from(rate_limit));
        }
        if let Some(socket_timeout) = options.socket_timeout {
            args.push(OsString::from("--socket-timeout"));
            args.push(OsString::from(socket_timeout.to_string()));
        }
        if let Some(retries) = options.retries {
            args.push(OsString::from("--retries"));
            args.push(OsString::from(retries.to_string()));
        }
        if let Some(fragment_retries) = options.fragment_retries {
            args.push(OsString::from("--fragment-retries"));
            args.push(OsString::from(fragment_retries.to_string()));
        }
        if let Some(extractor_args) = options.extractor_args.filter(|value| !value.is_empty()) {
            args.push(OsString::from("--extractor-args"));
            args.push(OsString::from(extractor_args));
        }
        if options.embed_metadata {
            args.push(OsString::from("--embed-metadata"));
            args.extend([
                OsString::from("--parse-metadata"),
                OsString::from("%(tags, ,)s:%(meta_comment)s"),
            ]);
        }
        if options.write_info_json {
            args.push(OsString::from("--write-info-json"));
        }
        if options.use_aria2 {
            args.extend([
                OsString::from("--downloader"),
                OsString::from("http,ftp:aria2c"),
                OsString::from("--downloader-args"),
                OsString::from(format!(
                    "aria2c:-x {} -s {} -k 1M",
                    concurrent_fragments, concurrent_fragments
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
        Ok(args)
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
        tx: mpsc::Sender<DownloadEvent>,
    ) {
        let mut command = TokioCommand::new(&self.executable);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // yt-dlp may launch ffmpeg or aria2c. A dedicated Unix process group lets
        // cancellation terminate the complete download tree rather than only its parent.
        #[cfg(unix)]
        command.process_group(0);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = tx
                    .send(DownloadEvent::Failed(format!(
                        "could not start yt-dlp: {error}"
                    )))
                    .await;
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
                    let _ = progress_tx.try_send(event);
                }
            }
        });
        let error_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut tail = VecDeque::with_capacity(12);
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    if tail.len() == 12 {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            }
            failure_message(&tail)
        });

        tokio::select! {
            status = child.wait() => {
                let _ = stdout_task.await;
                let error = error_task.await.unwrap_or_default();
                match status {
                    Ok(status) if status.success() => { let _ = tx.send(DownloadEvent::Finished).await; }
                    Ok(status) => { let _ = tx.send(DownloadEvent::Failed(if error.is_empty() { format!("yt-dlp exited with {status}") } else { error })).await; }
                    Err(error) => { let _ = tx.send(DownloadEvent::Failed(format!("could not wait for yt-dlp: {error}"))).await; }
                }
            }
            _ = &mut cancel => {
                terminate_process_tree(&mut child).await;
                stdout_task.abort();
                error_task.abort();
                let _ = tx.send(DownloadEvent::Cancelled).await;
            }
        }
    }
}

fn plugin_directory() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    let development_root = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_owned))
        .and_then(|directory| {
            directory
                .ancestors()
                .take(4)
                .find(|path| {
                    path.join("Cargo.toml").is_file()
                        && path.join("src/downloader.rs").is_file()
                        && path.join("plugins/yt_dlp_plugins/extractor").is_dir()
                })
                .map(Path::to_owned)
        });
    #[cfg(unix)]
    let system_directory = Some(PathBuf::from("/usr/share/crusty-dlp"));
    #[cfg(not(unix))]
    let system_directory: Option<PathBuf> = None;
    #[cfg(debug_assertions)]
    let mut roots = std::iter::once(development_root)
        .chain(system_directory.map(Some))
        .flatten();
    #[cfg(not(debug_assertions))]
    let mut roots = system_directory.into_iter();
    roots.find(|path| path.join("plugins/yt_dlp_plugins/extractor").is_dir())
}

fn failure_message(lines: &VecDeque<String>) -> String {
    lines
        .iter()
        .rev()
        .find(|line| line.trim_start().starts_with("ERROR:"))
        .cloned()
        .or_else(|| lines.back().cloned())
        .unwrap_or_default()
}

fn bounded_command_output(
    command: &mut StdCommand,
    limit: u64,
) -> Result<std::process::Output, String> {
    bounded_command_output_with_timeout(command, limit, None)
}

fn bounded_command_output_with_timeout(
    command: &mut StdCommand,
    limit: u64,
    timeout: Option<Duration>,
) -> Result<std::process::Output, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not capture command stdout".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "could not capture command stderr".to_owned())?;
    let stdout_limit = limit.saturating_add(1) as usize;
    let stdout_thread = std::thread::spawn(move || read_limited_and_drain(stdout, stdout_limit));
    let stderr_thread = std::thread::spawn(move || read_limited_and_drain(stderr, stdout_limit));
    let status = if let Some(timeout) = timeout {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("command timed out after {}s", timeout.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    } else {
        child.wait().map_err(|error| error.to_string())?
    };
    let (stdout, stdout_truncated) = stdout_thread
        .join()
        .map_err(|_| "stdout reader thread panicked".to_owned())?
        .map_err(|error| error.to_string())?;
    let (stderr, stderr_truncated) = stderr_thread
        .join()
        .map_err(|_| "stderr reader thread panicked".to_owned())?
        .map_err(|error| error.to_string())?;
    if stdout_truncated
        || stderr_truncated
        || stdout.len() as u64 > limit
        || stderr.len() as u64 > limit
    {
        return Err(format!("command output exceeds the {limit} byte limit"));
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn read_limited_and_drain<R: Read>(
    mut reader: R,
    limit_plus_one: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::with_capacity(limit_plus_one.min(8192));
    let mut truncated = false;
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if bytes.len() < limit_plus_one {
            let remaining = limit_plus_one - bytes.len();
            let keep = remaining.min(read);
            bytes.extend_from_slice(&buffer[..keep]);
            if keep < read {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    Ok((bytes, truncated))
}

#[cfg(unix)]
async fn terminate_process_tree(child: &mut tokio::process::Child) {
    let Some(pid) = child.id() else {
        let _ = child.kill().await;
        return;
    };
    // SAFETY: `pid` came from the live child. Negating it addresses the process
    // group created with `process_group(0)` above; no Rust memory is accessed.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    if tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
        .await
        .is_err()
    {
        // SAFETY: same process-group argument as above, now forcing shutdown.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        let _ = child.wait().await;
    }
}

#[cfg(windows)]
async fn terminate_process_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = TokioCommand::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status()
            .await;
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(not(any(unix, windows)))]
async fn terminate_process_tree(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
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
    let mut command = StdCommand::new(executable);
    command.arg("--list-impersonate-targets");
    let Ok(output) = bounded_command_output_with_timeout(
        &mut command,
        MAX_IMPERSONATION_OUTPUT_BYTES,
        Some(IMPERSONATION_TIMEOUT),
    ) else {
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
            if is_trusted_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| {
            #[cfg(windows)]
            let candidate = dir.join(format!("{name}.exe"));
            #[cfg(not(windows))]
            let candidate = dir.join(name);
            is_trusted_executable(&candidate).then_some(candidate)
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
        return expand_pmvhaven_playlist_urls(executable, url);
    }

    let mut command = StdCommand::new(executable);
    command.args(["--socket-timeout", "30", "--retries", "2"]);
    if let Some(plugin_dir) = plugin_directory() {
        command.arg("--plugin-dirs").arg(plugin_dir);
    }
    command.args(playlist_probe_arguments());
    let output = bounded_command_output(command.arg(url), MAX_PLAYLIST_OUTPUT_BYTES)
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
    let mut parsed_entries = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(entry) = parse_flat_playlist_line(line, source) {
            parsed_entries.push(entry);
            if parsed_entries.len() > MAX_PLAYLIST_ENTRIES {
                return Err(format!(
                    "playlist inspection exceeded the {MAX_PLAYLIST_ENTRIES} entry limit for {url}"
                ));
            }
        }
    }
    let entries = dedupe_playlist_entries(parsed_entries);
    if entries.is_empty() {
        return Err(format!("playlist inspection found 0 entries for {url}"));
    }
    Ok(Some(entries))
}

/// Return the site-provided playlist title for a queue folder. This is best
/// effort; the playlist entries remain usable when a site omits its title.
pub fn playlist_title(executable: &Path, url: &str) -> Option<String> {
    if is_pmvhaven_playlist_url(url) {
        let canonical = canonical_pmvhaven_playlist_url(url);
        let curl = dependency_path("curl")?;
        let output = bounded_command_output(
            StdCommand::new(curl)
                .args(["-fsSL", "--connect-timeout", "15", "--max-time", "60", "--"])
                .arg(canonical),
            MAX_PLAYLIST_OUTPUT_BYTES,
        )
        .ok()?;
        let html = String::from_utf8_lossy(&output.stdout);
        return extract_html_title(&html);
    }
    let mut command = StdCommand::new(executable);
    command.args([
        "--flat-playlist",
        "--playlist-items",
        "1",
        "--skip-download",
        "--no-warnings",
        "--print",
        "%(playlist_title)s",
    ]);
    if let Some(plugin_dir) = plugin_directory() {
        command.arg("--plugin-dirs").arg(plugin_dir);
    }
    let output =
        bounded_command_output(command.arg("--").arg(url), MAX_PLAYLIST_OUTPUT_BYTES).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.eq_ignore_ascii_case("NA"))
        .map(str::to_owned)
}

fn extract_html_title(html: &str) -> Option<String> {
    for marker in [
        r#"property="og:title" content=""#,
        r#"property='og:title' content='"#,
        "<title>",
    ] {
        if let Some((_, rest)) = html.split_once(marker) {
            let end = if marker == "<title>" {
                "</title>"
            } else if marker.contains("content=\"") {
                "\""
            } else {
                "'"
            };
            if let Some((title, _)) = rest.split_once(end) {
                let title = title.trim();
                if !title.is_empty() {
                    return Some(title.to_owned());
                }
            }
        }
    }
    None
}

fn playlist_probe_arguments() -> [&'static str; 4] {
    [
        "--flat-playlist",
        "--print",
        "%(title)s\t%(id)s\t%(webpage_url)s\t%(url)s\t%(thumbnail)s",
        "--",
    ]
}

fn expand_pmvhaven_playlist_urls(
    executable: &Path,
    url: &str,
) -> Result<Option<Vec<PlaylistEntry>>, String> {
    let canonical_url = canonical_pmvhaven_playlist_url(url);
    let curl = dependency_path("curl")
        .ok_or_else(|| "curl was not found in trusted executable paths".to_owned())?;
    let html_output = bounded_command_output(
        StdCommand::new(curl)
            .args(["-fsSL", "--connect-timeout", "15", "--max-time", "60", "--"])
            .arg(&canonical_url),
        MAX_PLAYLIST_OUTPUT_BYTES,
    )
    .map_err(|error| format!("could not inspect PMVHaven playlist HTML: {error}"))?;
    if html_output.status.success() {
        let html = String::from_utf8_lossy(&html_output.stdout);
        let entries = dedupe_playlist_entries(parse_pmvhaven_itemlist_entries(&html));
        if !entries.is_empty() {
            return Ok(Some(enforce_playlist_entry_limit(entries, &canonical_url)?));
        }
        let href_entries = dedupe_playlist_entries(parse_pmvhaven_href_entries(&html));
        if !href_entries.is_empty() {
            return Ok(Some(enforce_playlist_entry_limit(
                href_entries,
                &canonical_url,
            )?));
        }
    }

    let mut command = StdCommand::new(executable);
    command.args(["--socket-timeout", "30", "--retries", "2"]);
    if let Some(plugin_dir) = plugin_directory() {
        command.arg("--plugin-dirs").arg(plugin_dir);
    }
    command.args(playlist_probe_arguments());
    let output = bounded_command_output(command.arg(&canonical_url), MAX_PLAYLIST_OUTPUT_BYTES)
        .map_err(|error| format!("could not inspect PMVHaven playlist: {error}"))?;
    let mut yt_dlp_error = None;
    let entries = if output.status.success() {
        let mut parsed_entries = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(entry) = parse_flat_playlist_line(line, PlaylistSource::PmvHaven) {
                parsed_entries.push(entry);
                if parsed_entries.len() > MAX_PLAYLIST_ENTRIES {
                    return Err(format!(
                        "playlist inspection exceeded the {MAX_PLAYLIST_ENTRIES} entry limit for {canonical_url}"
                    ));
                }
            }
        }
        enforce_playlist_entry_limit(dedupe_playlist_entries(parsed_entries), &canonical_url)?
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        yt_dlp_error = Some(
            stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("yt-dlp could not inspect the PMVHaven playlist")
                .to_owned(),
        );
        Vec::new()
    };

    if entries.is_empty() {
        if let Some(message) = yt_dlp_error {
            return Err(message);
        }
        return Err(format!(
            "PMVHaven playlist parser found 0 entries for {canonical_url}"
        ));
    }
    Ok(Some(entries))
}

fn parse_pmvhaven_itemlist_entries(html: &str) -> Vec<PlaylistEntry> {
    let mut entries = Vec::new();
    let mut cursor = html;
    let name_key = "\"name\":\"";
    let thumbnail_key = "\"thumbnailUrl\":[\"";
    let embed_key = "embedUrl\":\"";

    while let Some((before_embed, after_embed)) = cursor.split_once(embed_key) {
        let Some((embed_url_raw, rest)) = after_embed.split_once('"') else {
            break;
        };
        let embed_url = decode_json_escapes(embed_url_raw);
        if embed_url.starts_with("http://") || embed_url.starts_with("https://") {
            let title = before_embed
                .rsplit(name_key)
                .next()
                .and_then(|fragment| fragment.split_once('"').map(|(value, _)| value))
                .map(decode_json_escapes)
                .filter(|value| !value.trim().is_empty());
            let thumbnail_url = before_embed
                .rsplit(thumbnail_key)
                .next()
                .and_then(|fragment| fragment.split_once('"').map(|(value, _)| value))
                .map(decode_json_escapes)
                .filter(|value| value.starts_with("http://") || value.starts_with("https://"));
            entries.push(PlaylistEntry {
                title,
                url: embed_url,
                thumbnail_url,
            });
        }
        cursor = rest;
    }
    entries
}

fn parse_pmvhaven_href_entries(html: &str) -> Vec<PlaylistEntry> {
    let mut entries = Vec::new();
    let mut cursor = html;
    let href_key = r#"href="/video/"#;
    while let Some((_, after_href)) = cursor.split_once(href_key) {
        let Some((slug, rest)) = after_href.split_once('"') else {
            break;
        };
        if !slug.trim().is_empty() {
            entries.push(PlaylistEntry {
                title: None,
                url: urls::pmvhaven_video_url(slug),
                thumbnail_url: None,
            });
        }
        cursor = rest;
    }
    entries
}

fn canonical_pmvhaven_playlist_url(url: &str) -> String {
    let trimmed = url.trim();
    match trimmed.find(['?', '#']) {
        Some(index) => trimmed[..index].to_owned(),
        None => trimmed.to_owned(),
    }
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

fn enforce_playlist_entry_limit(
    entries: Vec<PlaylistEntry>,
    url: &str,
) -> Result<Vec<PlaylistEntry>, String> {
    if entries.len() > MAX_PLAYLIST_ENTRIES {
        return Err(format!(
            "playlist inspection exceeded the {MAX_PLAYLIST_ENTRIES} entry limit for {url}"
        ));
    }
    Ok(entries)
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

pub fn sanitize_filename_component(value: &str) -> Result<String, String> {
    if value.contains("..") {
        return Err("Playlist name contains a path traversal sequence".into());
    }
    let mut output = String::with_capacity(value.len().min(120));
    for character in value.chars() {
        let allowed = character.is_alphanumeric()
            || matches!(character, ' ' | '-' | '_' | '.' | '(' | ')' | '[' | ']');
        if allowed {
            output.push(character);
        } else if !output.ends_with('_') {
            output.push('_');
        }
        if output.chars().count() >= 120 {
            break;
        }
    }
    let cleaned = output.trim().trim_matches('.').trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err("Playlist name does not contain a safe folder name".into());
    }
    Ok(cleaned.to_owned())
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

pub fn validate_extractor_args(value: &str) -> Result<(), String> {
    if value.contains('\0') || value.contains('\n') || value.contains('\r') {
        return Err("Extractor arguments cannot contain NUL or newline characters".into());
    }
    Ok(())
}

pub fn prepared_extractor_args(value: &str) -> Result<Option<&str>, String> {
    let trimmed = value.trim();
    validate_extractor_args(trimmed)?;
    Ok((!trimmed.is_empty()).then_some(trimmed))
}

pub fn validate_socket_timeout(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let parsed = trimmed
        .parse::<u32>()
        .map_err(|_| "Socket timeout must be a whole number of seconds".to_owned())?;
    if parsed == 0 {
        return Err("Socket timeout must be greater than 0 seconds".into());
    }
    Ok(())
}

pub fn validate_retry_count(value: &str, field_name: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    trimmed
        .parse::<u32>()
        .map(|_| ())
        .map_err(|_| format!("{field_name} must be a whole number"))
}

pub fn resolve_network_tuning(
    url: &str,
    socket_timeout: &str,
    retries: &str,
    fragment_retries: &str,
) -> Result<NetworkTuning, String> {
    let pmvhaven_defaults = url_host_matches(url, "pmvhaven.com");
    Ok(NetworkTuning {
        socket_timeout: parse_socket_timeout(socket_timeout)?
            .or_else(|| pmvhaven_defaults.then_some(60)),
        retries: parse_retry_count(retries, "Retries")?.or_else(|| pmvhaven_defaults.then_some(10)),
        fragment_retries: parse_retry_count(fragment_retries, "Fragment retries")?
            .or_else(|| pmvhaven_defaults.then_some(10)),
    })
}

pub fn requires_impersonation(url: &str) -> bool {
    is_boyfriendtv_url(url) || is_spankbang_url(url)
}

pub fn effective_impersonation_target<'a>(
    url: &str,
    configured_impersonation: Option<&'a str>,
    cookies_browser: Option<&str>,
) -> Option<&'a str> {
    configured_impersonation
        .filter(|target| *target != "none")
        .or_else(|| {
            if is_spankbang_url(url) {
                Some(browser_impersonation_target(
                    cookies_browser.unwrap_or("none"),
                ))
            } else if requires_impersonation(url) {
                Some("any")
            } else {
                None
            }
        })
}

pub fn is_boyfriendtv_url(value: &str) -> bool {
    url_host_matches(value, "boyfriendtv.com")
}

pub fn is_spankbang_url(value: &str) -> bool {
    url_host_matches(value, "spankbang.com")
}

fn parse_socket_timeout(value: &str) -> Result<Option<u32>, String> {
    validate_socket_timeout(value)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<u32>()
        .map(Some)
        .map_err(|_| "Socket timeout must be a whole number of seconds".to_owned())
}

fn parse_retry_count(value: &str, field_name: &str) -> Result<Option<u32>, String> {
    validate_retry_count(value, field_name)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<u32>()
        .map(Some)
        .map_err(|_| format!("{field_name} must be a whole number"))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn is_trusted_executable(path: &Path) -> bool {
    if !is_executable(path) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.parent()
            .and_then(|parent| parent.metadata().ok())
            .is_some_and(|metadata| metadata.permissions().mode() & 0o022 == 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
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
    let mut fields = line.splitn(5, '\t');
    let title = fields
        .next()
        .and_then(non_empty_trimmed)
        .filter(|value| !value.eq_ignore_ascii_case("na"))
        .map(str::to_owned);
    let id = fields.next().and_then(non_empty_trimmed);
    let webpage_url = fields.next().and_then(non_empty_trimmed);
    let media_url = fields.next().and_then(non_empty_trimmed);
    let thumbnail_url = fields
        .next()
        .and_then(non_empty_trimmed)
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .map(str::to_owned);

    webpage_url
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .map(str::to_owned)
        .or_else(|| {
            media_url
                .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
                .map(str::to_owned)
        })
        .or_else(|| fallback_playlist_entry_url(source, id))
        .map(|url| PlaylistEntry {
            title,
            url,
            thumbnail_url,
        })
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn browser_impersonation_target(browser: &str) -> &'static str {
    match browser {
        "firefox" => "firefox",
        "edge" => "edge",
        "chrome" | "chromium" | "brave" | "vivaldi" => "chrome",
        _ => "any",
    }
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
        PlaylistSource::YouTube => Some(urls::youtube_watch_url(id)),
        PlaylistSource::PmvHaven => Some(urls::pmvhaven_video_url(id)),
        PlaylistSource::SpankBang => Some(urls::spankbang_video_url(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_arguments_are_separate_and_url_is_last() {
        let downloader = Downloader::new("yt-dlp".into(), "/tmp/out".into());
        let args = downloader
            .arguments(
                "https://example.com/watch?v=x",
                &DownloadMode::Mp3,
                DownloadOptions::default(),
            )
            .unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--audio-format" && pair[1] == "mp3"));
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args.last().unwrap(), "https://example.com/watch?v=x");
    }

    #[test]
    fn custom_format_is_one_argument() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader
            .arguments(
                "https://example.com/x",
                &DownloadMode::Custom("18; rm -rf /".into()),
                DownloadOptions::default(),
            )
            .unwrap();
        assert!(args.contains(&OsString::from("18; rm -rf /")));
    }

    #[test]
    fn impersonation_is_passed_as_separate_arguments() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader
            .arguments(
                "https://example.com/x",
                &DownloadMode::Video,
                DownloadOptions {
                    impersonation: Some("chrome"),
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
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
    fn shared_impersonation_rules_match_special_hosts() {
        assert_eq!(
            effective_impersonation_target(
                "https://spankbang.com/7ubnq/video/example",
                None,
                Some("firefox"),
            ),
            Some("firefox")
        );
        assert_eq!(
            effective_impersonation_target(
                "https://www.boyfriendtv.com/videos/123/example",
                None,
                None,
            ),
            Some("any")
        );
        assert_eq!(
            effective_impersonation_target("https://example.com/video", Some("chrome"), None),
            Some("chrome")
        );
        assert_eq!(
            effective_impersonation_target(
                "https://www.boyfriendtv.com/videos/123/example",
                Some("none"),
                None,
            ),
            Some("any")
        );
    }

    #[test]
    fn prepared_extractor_args_trim_and_validate() {
        assert_eq!(
            prepared_extractor_args("  youtube:player_client=default  ").unwrap(),
            Some("youtube:player_client=default")
        );
        assert_eq!(prepared_extractor_args("   ").unwrap(), None);
        assert!(prepared_extractor_args("youtube:foo=bar\nnext:bad").is_err());
    }

    #[test]
    fn browser_cookie_source_is_passed_as_one_argument() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader
            .arguments(
                "https://example.com/x",
                &DownloadMode::Video,
                DownloadOptions {
                    cookies_browser: Some("firefox"),
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--cookies-from-browser" && pair[1] == "firefox"));
    }

    #[test]
    fn connection_count_and_aria2_are_bounded_arguments() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader
            .arguments(
                "https://example.com/video.mp4",
                &DownloadMode::Video,
                DownloadOptions {
                    concurrent_fragments: 8,
                    use_aria2: true,
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
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
        let args = downloader
            .arguments(
                "https://example.com/video.mp4",
                &DownloadMode::Video,
                DownloadOptions {
                    output_template: Some("custom/%(title)s.%(ext)s"),
                    rate_limit: Some("5M"),
                    allow_playlists: true,
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--limit-rate" && pair[1] == "5M"));
        let expected_output = PathBuf::from("/tmp/out")
            .join("custom/%(title)s.%(ext)s")
            .into_os_string();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--output" && pair[1] == expected_output));
    }

    #[test]
    fn playlist_folder_and_metadata_arguments_are_safe() {
        let downloader = Downloader::new("yt-dlp".into(), "/tmp/out".into());
        let args = downloader
            .arguments(
                "https://example.com/video",
                &DownloadMode::Video,
                DownloadOptions {
                    playlist_subfolder: Some("A/B: playlist"),
                    embed_metadata: true,
                    write_info_json: true,
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
        let expected_output = PathBuf::from("/tmp/out")
            .join("A_B_ playlist/%(title)s [%(id)s].%(ext)s")
            .into_os_string();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--output" && pair[1] == expected_output));
        assert!(args.contains(&OsString::from("--embed-metadata")));
        assert!(args.contains(&OsString::from("--write-info-json")));
        assert!(args.contains(&OsString::from("%(tags, ,)s:%(meta_comment)s")));
    }

    #[test]
    fn rejects_empty_or_unsafe_playlist_folder() {
        assert_eq!(sanitize_filename_component("A/B"), Ok("A_B".into()));
        assert!(sanitize_filename_component("../").is_err());
        assert!(sanitize_filename_component("   ").is_err());
    }

    #[test]
    fn socket_timeout_and_retry_arguments_are_separate() {
        let downloader = Downloader::new("yt-dlp".into(), "/tmp/out".into());
        let args = downloader
            .arguments(
                "https://pmvhaven.com/video/example",
                &DownloadMode::Video,
                DownloadOptions {
                    socket_timeout: Some(60),
                    retries: Some(10),
                    fragment_retries: Some(10),
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--socket-timeout" && pair[1] == "60"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--retries" && pair[1] == "10"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--fragment-retries" && pair[1] == "10"));
    }

    #[test]
    fn rejects_unsafe_output_template() {
        assert!(validate_output_template("../bad.%(ext)s").is_err());
        assert!(validate_output_template("/abs/bad.%(ext)s").is_err());
        assert!(validate_output_template("safe/%(title)s.%(ext)s").is_ok());

        let downloader = Downloader::new("yt-dlp".into(), "/tmp/out".into());
        assert!(downloader
            .arguments(
                "https://example.com/video",
                &DownloadMode::Video,
                DownloadOptions {
                    output_template: Some("../escape.%(ext)s"),
                    ..DownloadOptions::default()
                },
            )
            .is_err());
    }

    #[test]
    fn command_builder_clamps_connection_count() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader
            .arguments(
                "https://example.com/video",
                &DownloadMode::Video,
                DownloadOptions {
                    concurrent_fragments: u8::MAX,
                    use_aria2: true,
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--concurrent-fragments" && pair[1] == "16"));
        assert!(args.contains(&OsString::from("aria2c:-x 16 -s 16 -k 1M")));
    }

    #[test]
    fn failure_message_prefers_error_line_from_bounded_tail() {
        let lines = VecDeque::from([
            "WARNING: retrying".to_owned(),
            "ERROR: useful failure".to_owned(),
            "cleanup complete".to_owned(),
        ]);
        assert_eq!(failure_message(&lines), "ERROR: useful failure");
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
    fn validates_extractor_args_as_one_safe_value() {
        assert!(validate_extractor_args("youtube:player_client=default").is_ok());
        assert!(validate_extractor_args("youtube:player_client=default\n--exec").is_err());
    }

    #[test]
    fn validates_network_tuning_syntax() {
        assert!(validate_socket_timeout("").is_ok());
        assert!(validate_socket_timeout("60").is_ok());
        assert!(validate_socket_timeout("0").is_err());
        assert!(validate_retry_count("", "Retries").is_ok());
        assert!(validate_retry_count("0", "Retries").is_ok());
        assert!(validate_retry_count("10", "Retries").is_ok());
        assert!(validate_retry_count("many", "Retries").is_err());
    }

    #[test]
    fn pmvhaven_defaults_network_tuning_when_blank() {
        assert_eq!(
            resolve_network_tuning("https://pmvhaven.com/video/example", "", "", "",).unwrap(),
            NetworkTuning {
                socket_timeout: Some(60),
                retries: Some(10),
                fragment_retries: Some(10),
            }
        );
    }

    #[test]
    fn non_pmvhaven_leaves_blank_network_tuning_unset() {
        assert_eq!(
            resolve_network_tuning("https://www.youtube.com/watch?v=abc123", "", "", "",).unwrap(),
            NetworkTuning::default()
        );
    }

    #[test]
    fn explicit_network_tuning_overrides_pmvhaven_defaults() {
        assert_eq!(
            resolve_network_tuning("https://pmvhaven.com/video/example", "75", "4", "6",).unwrap(),
            NetworkTuning {
                socket_timeout: Some(75),
                retries: Some(4),
                fragment_retries: Some(6),
            }
        );
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
        let args = downloader
            .arguments(
                "https://www.youtube.com/playlist?list=abc",
                &DownloadMode::Video,
                DownloadOptions::default(),
            )
            .unwrap();
        assert!(args.contains(&OsString::from("--no-playlist")));
    }

    #[test]
    fn omits_no_playlist_when_enabled() {
        let downloader = Downloader::new("yt-dlp".into(), ".".into());
        let args = downloader
            .arguments(
                "https://www.youtube.com/playlist?list=abc",
                &DownloadMode::Video,
                DownloadOptions {
                    allow_playlists: true,
                    ..DownloadOptions::default()
                },
            )
            .unwrap();
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
            "Title A\tabc123\thttps://www.youtube.com/watch?v=abc123\tabc123\thttps://img.example/a.jpg",
            "Title B\tdef456\t\tdef456\t",
            "Title C\tghi789\t\thttps://spankbang.com/ghi789/video/ghi789\thttps://img.example/c.webp",
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
                    thumbnail_url: Some("https://img.example/a.jpg".into()),
                },
                PlaylistEntry {
                    title: Some("Title B".into()),
                    url: "https://www.youtube.com/watch?v=def456".into(),
                    thumbnail_url: None,
                },
                PlaylistEntry {
                    title: Some("Title C".into()),
                    url: "https://spankbang.com/ghi789/video/ghi789".into(),
                    thumbnail_url: Some("https://img.example/c.webp".into()),
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
                thumbnail_url: None,
            })
        );
        assert_eq!(
            parse_flat_playlist_line("Title Y\tsb001\t\t", PlaylistSource::SpankBang),
            Some(PlaylistEntry {
                title: Some("Title Y".into()),
                url: "https://spankbang.com/sb001/video/sb001".into(),
                thumbnail_url: None,
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
                "%(title)s\t%(id)s\t%(webpage_url)s\t%(url)s\t%(thumbnail)s",
                "--"
            ]
        );
    }

    #[test]
    fn strips_pmvhaven_playlist_query_params() {
        assert_eq!(
            canonical_pmvhaven_playlist_url(
                "https://pmvhaven.com/playlists/692b70e2d7984d93b13f83c2?index=8"
            ),
            "https://pmvhaven.com/playlists/692b70e2d7984d93b13f83c2"
        );
        assert_eq!(
            canonical_pmvhaven_playlist_url(
                "https://pmvhaven.com/playlists/692b70e2d7984d93b13f83c2#foo"
            ),
            "https://pmvhaven.com/playlists/692b70e2d7984d93b13f83c2"
        );
    }

    #[test]
    fn parses_pmvhaven_href_fallback_entries() {
        let html = r#"
            <a href="/video/bimbo-candy-1_656b815db380dff74beb2d65">one</a>
            <a href="/video/another-one_123">two</a>
        "#;
        assert_eq!(
            parse_pmvhaven_href_entries(html),
            vec![
                PlaylistEntry {
                    title: None,
                    url: "https://pmvhaven.com/video/bimbo-candy-1_656b815db380dff74beb2d65".into(),
                    thumbnail_url: None,
                },
                PlaylistEntry {
                    title: None,
                    url: "https://pmvhaven.com/video/another-one_123".into(),
                    thumbnail_url: None,
                }
            ]
        );
    }

    #[test]
    fn parses_pmvhaven_embed_entries() {
        let html = r#"
            <script type="application/ld+json">{"@type":"ItemList","itemListElement":[
                {"item":{"@type":"VideoObject","name":"Bambi TikTok 7","thumbnailUrl":["https://img.example/a.webp"],"embedUrl":"https://pmvhaven.com/videos/68f6982abdaea7d82ecae26f"}},
                {"item":{"@type":"VideoObject","name":"Best friend","thumbnailUrl":["https://img.example/b.webp"],"embedUrl":"https://pmvhaven.com/videos/68f69f02bdaea7d82ecb0064"}}
            ]}</script>
        "#;
        assert_eq!(
            parse_pmvhaven_itemlist_entries(html),
            vec![
                PlaylistEntry {
                    title: Some("Bambi TikTok 7".into()),
                    url: "https://pmvhaven.com/videos/68f6982abdaea7d82ecae26f".into(),
                    thumbnail_url: Some("https://img.example/a.webp".into()),
                },
                PlaylistEntry {
                    title: Some("Best friend".into()),
                    url: "https://pmvhaven.com/videos/68f69f02bdaea7d82ecb0064".into(),
                    thumbnail_url: Some("https://img.example/b.webp".into()),
                }
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
                thumbnail_url: Some("https://img.example/first.jpg".into()),
            },
            PlaylistEntry {
                title: Some("Duplicate".into()),
                url: "https://example.com/a".into(),
                thumbnail_url: Some("https://img.example/duplicate.jpg".into()),
            },
            PlaylistEntry {
                title: Some("Second".into()),
                url: "https://example.com/b".into(),
                thumbnail_url: None,
            },
        ];
        assert_eq!(
            dedupe_playlist_entries(entries),
            vec![
                PlaylistEntry {
                    title: Some("First".into()),
                    url: "https://example.com/a".into(),
                    thumbnail_url: Some("https://img.example/first.jpg".into()),
                },
                PlaylistEntry {
                    title: Some("Second".into()),
                    url: "https://example.com/b".into(),
                    thumbnail_url: None,
                },
            ]
        );
    }

    #[test]
    fn rejects_playlist_entry_lists_over_the_limit() {
        let entries = (0..=MAX_PLAYLIST_ENTRIES)
            .map(|index| PlaylistEntry {
                title: Some(format!("Item {index}")),
                url: format!("https://example.com/{index}"),
                thumbnail_url: None,
            })
            .collect();

        let error =
            enforce_playlist_entry_limit(entries, "https://example.com/playlist").unwrap_err();
        assert!(error.contains("entry limit"));
    }
}
