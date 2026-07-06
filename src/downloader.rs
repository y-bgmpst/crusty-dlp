use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
};

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
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DownloadOptions<'a> {
    pub impersonation: Option<&'a str>,
    pub cookies_browser: Option<&'a str>,
}

impl Downloader {
    pub fn new(executable: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            executable,
            output_dir,
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
            OsString::from("--no-playlist"),
            OsString::from("--progress-template"),
            OsString::from("download:crusty-dlp:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s"),
            OsString::from("--output"),
            self.output_dir.join("%(title)s [%(id)s].%(ext)s").into_os_string(),
        ];
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
    fn parses_private_progress_lines() {
        match parse_progress("crusty-dlp: 42.5%|2MiB/s|00:10").unwrap() {
            DownloadEvent::Progress { percent, .. } => assert_eq!(percent, Some(42.5)),
            _ => panic!("wrong event"),
        }
    }
}
