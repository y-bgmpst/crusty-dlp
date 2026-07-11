use std::{collections::VecDeque, path::PathBuf};

use tokio::sync::oneshot;

use crate::{
    config::{validate_output_dir, Config},
    downloader::{
        resolve_network_tuning, supports_playlist_expansion, validate_extractor_args,
        validate_output_template, validate_rate_limit, DownloadEvent, DownloadOptions,
    },
    errors::AppError,
    redaction::{display_url, redact_message},
    search::{open_platform_search, SearchPlatform},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadMode {
    Video,
    Audio,
    Mp3,
    Custom(String),
}

impl DownloadMode {
    pub fn label(&self) -> &str {
        match self {
            Self::Video => "Best video",
            Self::Audio => "Best audio only",
            Self::Mp3 => "MP3 audio",
            Self::Custom(_) => "Custom format",
        }
    }

    pub fn needs_ffmpeg(&self) -> bool {
        matches!(self, Self::Audio | Self::Mp3)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    Waiting,
    Downloading,
    Finished,
    Failed,
    Cancelled,
}

impl DownloadState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting",
            Self::Downloading => "Downloading",
            Self::Finished => "Finished",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub url: String,
    pub state: DownloadState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Url,
    Search,
    Mode,
    Impersonation,
    Connections,
    Output,
    Queue,
}

pub struct App {
    pub config: Config,
    pub config_path: PathBuf,
    pub mode: DownloadMode,
    pub queue: VecDeque<QueueItem>,
    pub current: Option<QueueItem>,
    pub input: String,
    pub search_query: String,
    pub panel: Panel,
    pub editing: bool,
    pub show_help: bool,
    pub should_quit: bool,
    pub dry_run: bool,
    pub debug: bool,
    pub message: String,
    pub progress: Option<f64>,
    pub progress_text: String,
    pub queue_offset: usize,
    pub impersonation_targets: Vec<String>,
    pub show_install_prompt: bool,
    pub aria2_available: bool,
    start_requested: bool,
    cancel_tx: Option<oneshot::Sender<()>>,
}

impl App {
    fn set_message(&mut self, message: impl Into<String>) {
        self.message = redact_message(&message.into(), self.debug);
    }

    pub fn new(
        config: Config,
        config_path: PathBuf,
        dry_run: bool,
        debug: bool,
        impersonation_targets: Vec<String>,
        aria2_available: bool,
    ) -> Self {
        let mode = match config.default_mode.as_str() {
            "audio" => DownloadMode::Audio,
            "mp3" => DownloadMode::Mp3,
            "custom" => DownloadMode::Custom(config.custom_format.clone()),
            _ => DownloadMode::Video,
        };
        Self {
            config,
            config_path,
            mode,
            queue: VecDeque::new(),
            current: None,
            input: String::new(),
            search_query: String::new(),
            panel: Panel::Url,
            editing: false,
            show_help: false,
            should_quit: false,
            dry_run,
            debug,
            message: "Ready".into(),
            progress: None,
            progress_text: String::new(),
            queue_offset: 0,
            impersonation_targets,
            show_install_prompt: false,
            aria2_available,
            start_requested: false,
            cancel_tx: None,
        }
    }

    pub fn add_input(&mut self) {
        let input = std::mem::take(&mut self.input);
        let mut values = input.split_whitespace().peekable();
        if values.peek().is_none() {
            self.message = "Enter at least one URL".into();
            return;
        }
        for value in values {
            self.add_url(value.to_owned());
        }
        self.editing = false;
    }

    pub fn add_url(&mut self, url: String) {
        match validate_url(&url) {
            Ok(()) => {
                let needs_spankbang_session =
                    is_spankbang_url(&url) && self.config.cookies_browser == "none";
                self.queue.push_back(QueueItem {
                    url,
                    state: DownloadState::Waiting,
                });
                self.clamp_queue_offset();
                self.message = if needs_spankbang_session {
                    "SpankBang may require fresh browser cookies; press b to select that browser"
                        .into()
                } else {
                    "Added to queue".into()
                };
            }
            Err(error) => self.message = error.to_string(),
        }
    }

    pub fn cycle_panel(&mut self) {
        self.panel = match self.panel {
            Panel::Url => Panel::Search,
            Panel::Search => Panel::Mode,
            Panel::Mode => Panel::Impersonation,
            Panel::Impersonation => Panel::Connections,
            Panel::Connections => Panel::Output,
            Panel::Output => Panel::Queue,
            Panel::Queue => Panel::Url,
        };
        self.editing = false;
    }

    pub fn cycle_mode(&mut self) {
        self.mode = match &self.mode {
            DownloadMode::Video => DownloadMode::Audio,
            DownloadMode::Audio => DownloadMode::Mp3,
            DownloadMode::Mp3 => DownloadMode::Custom(self.config.custom_format.clone()),
            DownloadMode::Custom(_) => DownloadMode::Video,
        };
        self.config.default_mode = match self.mode {
            DownloadMode::Video => "video",
            DownloadMode::Audio => "audio",
            DownloadMode::Mp3 => "mp3",
            DownloadMode::Custom(_) => "custom",
        }
        .into();
        self.save_config();
    }

    pub fn edit_current_panel(&mut self) {
        match self.panel {
            Panel::Url => self.editing = true,
            Panel::Search => {
                self.input = self.search_query.clone();
                self.editing = true;
            }
            Panel::Output => {
                self.input = self.config.output_dir.to_string_lossy().into();
                self.editing = true;
            }
            Panel::Mode if matches!(self.mode, DownloadMode::Custom(_)) => {
                self.input = self.config.custom_format.clone();
                self.editing = true;
            }
            Panel::Mode => self.cycle_mode(),
            Panel::Impersonation => self.cycle_impersonation(),
            Panel::Connections => self.cycle_connections(),
            Panel::Queue => {}
        }
    }

    pub fn commit_edit(&mut self) {
        match self.panel {
            Panel::Url => self.add_input(),
            Panel::Search => {
                if self.input.trim().is_empty() {
                    self.message = "Search query cannot be empty".into();
                    return;
                }
                self.search_query = self.input.trim().into();
                self.editing = false;
                self.input.clear();
            }
            Panel::Output => {
                match validate_output_dir(&self.input) {
                    Ok(path) => self.config.output_dir = path,
                    Err(error) => {
                        self.message = error;
                        return;
                    }
                }
                self.editing = false;
                self.input.clear();
                self.save_config();
            }
            Panel::Mode => {
                if self.input.trim().is_empty() {
                    self.message = AppError::EmptyFormat.to_string();
                    return;
                }
                self.config.custom_format = self.input.trim().into();
                self.mode = DownloadMode::Custom(self.config.custom_format.clone());
                self.editing = false;
                self.input.clear();
                self.save_config();
            }
            Panel::Impersonation => {}
            Panel::Connections => {}
            Panel::Queue => {}
        }
    }

    pub fn search_platform(&self) -> SearchPlatform {
        SearchPlatform::from_config(&self.config.search_platform)
    }

    pub fn cycle_search_platform(&mut self) {
        self.config.search_platform = self.search_platform().next().config_value().into();
        self.save_config();
    }

    pub fn open_search(&mut self) {
        match open_platform_search(&self.search_query, self.search_platform()) {
            Ok(url) => self.set_message(format!(
                "Opened {} search: {}",
                self.search_platform().label(),
                display_url(&url, self.debug)
            )),
            Err(error) => self.message = error.to_string(),
        }
    }

    pub fn cycle_impersonation(&mut self) {
        if self.impersonation_targets.is_empty() {
            self.show_install_prompt = true;
            return;
        }

        let mut choices = Vec::with_capacity(self.impersonation_targets.len() + 2);
        choices.push("none");
        choices.push("any");
        choices.extend(self.impersonation_targets.iter().map(String::as_str));
        let current = choices
            .iter()
            .position(|choice| *choice == self.config.impersonation)
            .unwrap_or(0);
        self.config.impersonation = choices[(current + 1) % choices.len()].to_owned();
        self.save_config();
    }

    pub fn impersonation_label(&self) -> &str {
        match self.config.impersonation.as_str() {
            "none" => "None",
            "any" => "Any available",
            target => target,
        }
    }

    pub fn requires_impersonation(&self, url: &str) -> bool {
        is_boyfriendtv_url(url) || is_spankbang_url(url)
    }

    pub fn effective_impersonation<'a>(&'a self, url: &str) -> Option<&'a str> {
        match self.config.impersonation.as_str() {
            "none" if is_spankbang_url(url) => match self.config.cookies_browser.as_str() {
                "firefox" => Some("firefox"),
                "edge" => Some("edge"),
                "chrome" | "chromium" | "brave" | "vivaldi" => Some("chrome"),
                _ => Some("any"),
            },
            "none" if self.requires_impersonation(url) => Some("any"),
            "none" => None,
            target => Some(target),
        }
    }

    pub fn download_options<'a>(&'a self, url: &str) -> Result<DownloadOptions<'a>, String> {
        validate_output_template(&self.config.output_template)?;
        validate_rate_limit(&self.config.rate_limit)?;
        validate_extractor_args(&self.config.extractor_args)?;
        let tuning = resolve_network_tuning(
            url,
            &self.config.socket_timeout,
            &self.config.retries,
            &self.config.fragment_retries,
        )?;
        Ok(DownloadOptions {
            impersonation: self.effective_impersonation(url),
            cookies_browser: match self.config.cookies_browser.as_str() {
                "none" => None,
                browser => Some(browser),
            },
            concurrent_fragments: self.config.concurrent_fragments.clamp(1, 16),
            use_aria2: self.config.use_aria2 && self.aria2_available,
            output_template: Some(self.config.output_template.as_str()),
            rate_limit: (!self.config.rate_limit.trim().is_empty())
                .then_some(self.config.rate_limit.trim()),
            socket_timeout: tuning.socket_timeout,
            retries: tuning.retries,
            fragment_retries: tuning.fragment_retries,
            extractor_args: (!self.config.extractor_args.trim().is_empty())
                .then_some(self.config.extractor_args.trim()),
            playlist_subfolder: None,
            playlist_subfolders: self.config.playlist_subfolders
                && supports_playlist_expansion(url),
            embed_metadata: self.config.embed_metadata,
            write_info_json: self.config.write_info_json,
            allow_playlists: self.config.allow_playlists,
        })
    }

    pub fn cycle_connections(&mut self) {
        const CONNECTIONS: &[u8] = &[1, 2, 4, 8, 12, 16];
        let current = CONNECTIONS
            .iter()
            .position(|value| *value == self.config.concurrent_fragments)
            .unwrap_or(2);
        self.config.concurrent_fragments = CONNECTIONS[(current + 1) % CONNECTIONS.len()];
        self.save_config();
    }

    pub fn toggle_aria2(&mut self) {
        if !self.aria2_available {
            self.message = "aria2c not found; install: sudo pacman -S aria2".into();
            return;
        }
        self.config.use_aria2 = !self.config.use_aria2;
        self.save_config();
    }

    pub fn cycle_cookies_browser(&mut self) {
        const BROWSERS: &[&str] = &[
            "none", "firefox", "chrome", "chromium", "brave", "vivaldi", "edge",
        ];
        let current = BROWSERS
            .iter()
            .position(|browser| *browser == self.config.cookies_browser)
            .unwrap_or(0);
        self.config.cookies_browser = BROWSERS[(current + 1) % BROWSERS.len()].to_owned();
        self.save_config();
    }

    pub fn cookies_browser_label(&self) -> &str {
        match self.config.cookies_browser.as_str() {
            "none" => "off",
            browser => browser,
        }
    }

    fn save_config(&mut self) {
        match self.config.save(&self.config_path) {
            Ok(()) => self.message = "Configuration saved".into(),
            Err(error) => self.message = error.to_string(),
        }
    }

    pub fn request_start(&mut self) {
        if self.current.is_some() {
            self.message = "A download is already running".into();
        } else {
            self.start_requested = true;
        }
    }

    pub fn take_start_request(&mut self) -> bool {
        std::mem::take(&mut self.start_requested)
    }

    pub fn next_queued(&mut self) -> Option<QueueItem> {
        let position = self
            .queue
            .iter()
            .position(|item| item.state == DownloadState::Waiting)?;
        let item = self.queue.remove(position);
        self.clamp_queue_offset();
        item
    }

    pub fn begin_download(&mut self, mut item: QueueItem) -> oneshot::Receiver<()> {
        item.state = DownloadState::Downloading;
        self.current = Some(item);
        self.message = "Downloading".into();
        self.progress = None;
        self.progress_text.clear();
        let (tx, rx) = oneshot::channel();
        self.cancel_tx = Some(tx);
        rx
    }

    pub fn fail_item(&mut self, mut item: QueueItem, message: &str) {
        item.state = DownloadState::Failed;
        self.queue.push_front(item);
        self.clamp_queue_offset();
        self.set_message(message);
    }

    pub fn finish_dry_run(&mut self, mut item: QueueItem, command: String) {
        item.state = DownloadState::Finished;
        self.queue.push_back(item);
        self.clamp_queue_offset();
        self.set_message(format!("Dry run: {command}"));
        if self
            .queue
            .iter()
            .any(|item| item.state == DownloadState::Waiting)
        {
            self.start_requested = true;
        }
    }

    pub fn cancel(&mut self) {
        if let Some(tx) = self.cancel_tx.take() {
            let _ = tx.send(());
            self.message = "Cancelling…".into();
        } else {
            self.message = "No active download".into();
        }
    }

    pub fn handle_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Progress { percent, text } => {
                self.progress = percent;
                self.progress_text = text;
            }
            DownloadEvent::Finished => {
                self.complete_current(DownloadState::Finished, "Download finished")
            }
            DownloadEvent::Failed(error) => self.complete_current(DownloadState::Failed, &error),
            DownloadEvent::Cancelled => {
                self.complete_current(DownloadState::Cancelled, "Download cancelled")
            }
        }
    }

    fn complete_current(&mut self, state: DownloadState, message: &str) {
        if let Some(mut item) = self.current.take() {
            item.state = state;
            self.queue.push_back(item);
        }
        self.clamp_queue_offset();
        self.cancel_tx = None;
        self.set_message(message);
        if state == DownloadState::Finished
            && self
                .queue
                .iter()
                .any(|item| item.state == DownloadState::Waiting)
        {
            self.start_requested = true;
        }
    }

    pub fn scroll_queue(&mut self, delta: isize) {
        if delta < 0 {
            self.queue_offset = self.queue_offset.saturating_sub(delta.unsigned_abs());
        } else {
            self.queue_offset = self.queue_offset.saturating_add(delta as usize);
        }
        self.clamp_queue_offset();
    }

    fn clamp_queue_offset(&mut self) {
        let total = self.queue.len() + usize::from(self.current.is_some());
        self.queue_offset = self.queue_offset.min(total.saturating_sub(1));
    }
}

pub fn validate_url(value: &str) -> Result<(), AppError> {
    let parsed = url::Url::parse(value).map_err(|_| AppError::InvalidUrl)?;
    if matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
    {
        Ok(())
    } else {
        Err(AppError::InvalidUrl)
    }
}

pub fn is_boyfriendtv_url(value: &str) -> bool {
    url_host_matches(value, "boyfriendtv.com")
}

pub fn is_spankbang_url(value: &str) -> bool {
    url_host_matches(value, "spankbang.com")
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_http_urls() {
        assert!(validate_url("https://example.com/a?b=c").is_ok());
    }
    #[test]
    fn rejects_shell_text_and_relative_values() {
        assert!(validate_url("https://exa mple.com/video").is_err());
        assert!(validate_url("example.com/video").is_err());
    }
    #[test]
    fn rejects_empty_host() {
        assert!(validate_url("https://").is_err());
    }

    #[test]
    fn rejects_urls_with_credentials() {
        assert!(validate_url("https://user:secret@example.com/video").is_err());
    }

    #[test]
    fn next_queued_skips_completed_items() {
        let mut app = App::new(
            Config::default(),
            "config.toml".into(),
            false,
            false,
            Vec::new(),
            false,
        );
        app.queue.push_back(QueueItem {
            url: "https://example.com/old".into(),
            state: DownloadState::Finished,
        });
        app.queue.push_back(QueueItem {
            url: "https://example.com/new".into(),
            state: DownloadState::Waiting,
        });
        assert_eq!(app.next_queued().unwrap().url, "https://example.com/new");
        assert_eq!(app.queue.front().unwrap().state, DownloadState::Finished);
    }

    #[test]
    fn identifies_only_boyfriendtv_hosts() {
        assert!(is_boyfriendtv_url(
            "https://www.boyfriendtv.com/videos/123/example"
        ));
        assert!(!is_boyfriendtv_url(
            "https://boyfriendtv.com.example.org/videos/123"
        ));
        assert!(!is_boyfriendtv_url(
            "https://example.org/?next=boyfriendtv.com"
        ));
    }

    #[test]
    fn identifies_only_spankbang_hosts() {
        assert!(is_spankbang_url(
            "https://spankbang.com/7ubnq/video/example"
        ));
        assert!(!is_spankbang_url("https://spankbang.com.example.org/x"));
    }

    #[test]
    fn spankbang_uses_matching_cookie_browser_impersonation() {
        let config = Config {
            cookies_browser: "firefox".into(),
            ..Config::default()
        };
        let app = App::new(
            config,
            "config.toml".into(),
            false,
            false,
            vec!["firefox".into()],
            false,
        );
        assert_eq!(
            app.effective_impersonation("https://spankbang.com/7ubnq/video/example"),
            Some("firefox")
        );
    }

    #[test]
    fn pmvhaven_download_options_apply_resilient_defaults() {
        let app = App::new(
            Config::default(),
            "config.toml".into(),
            false,
            false,
            Vec::new(),
            false,
        );
        let options = app
            .download_options("https://pmvhaven.com/video/example")
            .unwrap();
        assert_eq!(options.socket_timeout, Some(60));
        assert_eq!(options.retries, Some(10));
        assert_eq!(options.fragment_retries, Some(10));
    }
}
