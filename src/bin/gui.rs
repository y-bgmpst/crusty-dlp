use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    path::PathBuf,
    process::Command as StdCommand,
    sync::{
        mpsc::{self as std_mpsc, Receiver, TryRecvError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use arboard::Clipboard;
use crusty_dlp::{
    app::{validate_url, DownloadMode, DownloadState},
    config::Config,
    downloader::{
        available_impersonation_targets, current_executable_path, dependency_path,
        expand_playlist_urls, playlist_title, resolve_network_tuning, resolved_plugin_directory,
        sanitize_filename_component, supports_playlist_expansion, validate_output_template,
        validate_rate_limit, validate_retry_count, validate_socket_timeout, DownloadEvent,
        DownloadOptions, Downloader, PlaylistEntry,
    },
    search::{open_platform_search, SearchPlatform},
};
use eframe::egui::{self, Color32, RichText};
use image::ImageReader;
use tokio::sync::{mpsc, oneshot};

const BLUE: Color32 = Color32::from_rgb(47, 128, 237);
const GREEN: Color32 = Color32::from_rgb(72, 180, 90);
const RED: Color32 = Color32::from_rgb(235, 87, 87);
const AMBER: Color32 = Color32::from_rgb(217, 154, 34);
const PANEL: Color32 = Color32::from_rgb(31, 36, 41);
const PANEL_ALT: Color32 = Color32::from_rgb(36, 42, 48);
const APP_ID: &str = "crusty-dlp";
const THUMBNAIL_CACHE_LIMIT: usize = 96;
const THUMBNAIL_RESPONSE_LIMIT: u64 = 5 * 1024 * 1024;
const THUMBNAIL_DISK_CACHE_LIMIT: u64 = 128 * 1024 * 1024;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_GIT_SHA: &str = match option_env!("CRUSTY_GIT_SHA") {
    Some(value) => value,
    None => "unknown",
};
const BUILD_GIT_DIRTY: &str = match option_env!("CRUSTY_GIT_DIRTY") {
    Some(value) => value,
    None => "unknown",
};
const BUILD_TIMESTAMP: &str = match option_env!("CRUSTY_BUILD_TIMESTAMP") {
    Some(value) => value,
    None => "unknown",
};
const BUILD_PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiTheme {
    Graphite,
    Midnight,
    Oled,
    Warm,
}

impl GuiTheme {
    const ALL: [Self; 4] = [Self::Graphite, Self::Midnight, Self::Oled, Self::Warm];

    fn from_config(value: &str) -> Self {
        match value {
            "midnight" => Self::Midnight,
            "oled" => Self::Oled,
            "warm" => Self::Warm,
            _ => Self::Graphite,
        }
    }

    fn config_value(self) -> &'static str {
        match self {
            Self::Graphite => "graphite",
            Self::Midnight => "midnight",
            Self::Oled => "oled",
            Self::Warm => "warm",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Graphite => "Graphite",
            Self::Midnight => "Midnight",
            Self::Oled => "OLED",
            Self::Warm => "Warm Slate",
        }
    }
}

fn main() -> eframe::Result {
    let app_title = format!("crusty-dlp v{APP_VERSION}");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&app_title)
            .with_app_id(APP_ID)
            .with_icon(app_icon())
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native(
        &app_title,
        options,
        Box::new(|cc| Ok(Box::new(GuiApp::new(cc)))),
    )
}

fn build_label() -> String {
    let dirty_suffix = if BUILD_GIT_DIRTY == "dirty" {
        "-dirty"
    } else {
        ""
    };
    format!("v{APP_VERSION}+{BUILD_GIT_SHA}{dirty_suffix}")
}

fn app_icon() -> egui::IconData {
    let size = 64usize;
    let mut rgba = vec![0u8; size * size * 4];

    fill_rounded_rect(&mut rgba, size, (4, 4), (56, 56), 14.0, [18, 22, 27, 255]);
    fill_circle(&mut rgba, size, 32.0, 27.0, 16.0, [234, 93, 36, 255]);
    fill_circle(&mut rgba, size, 32.0, 27.0, 10.0, [27, 32, 38, 255]);
    fill_triangle(
        &mut rgba,
        size,
        (28.0, 21.0),
        (41.0, 27.0),
        (28.0, 33.0),
        [74, 163, 255, 255],
    );
    fill_circle(&mut rgba, size, 18.0, 17.0, 4.0, [255, 138, 61, 255]);
    fill_circle(&mut rgba, size, 46.0, 17.0, 4.0, [255, 138, 61, 255]);
    fill_circle(&mut rgba, size, 18.0, 42.0, 6.0, [255, 138, 61, 255]);
    fill_circle(&mut rgba, size, 46.0, 42.0, 6.0, [255, 138, 61, 255]);
    fill_rounded_rect(&mut rgba, size, (24, 44), (16, 7), 3.5, [47, 128, 237, 255]);

    egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    }
}

struct GuiQueueItem {
    id: u64,
    title: Option<Box<str>>,
    url: Box<str>,
    thumbnail_url: Option<Box<str>>,
    playlist_folder: Option<Box<str>>,
    thumbnail_requested: bool,
    thumbnail_failed: bool,
    thumbnail_image: Option<DecodedThumbnail>,
    thumbnail_texture: Option<egui::TextureHandle>,
    state: DownloadState,
    progress: f32,
    progress_text: String,
    error: Option<String>,
}

#[derive(Debug)]
struct JobEvent {
    id: u64,
    event: DownloadEvent,
}

struct DownloadRequest {
    id: u64,
    downloader: Downloader,
    args: Vec<std::ffi::OsString>,
    cancel_rx: oneshot::Receiver<()>,
}

struct QueueThumbnailEvent {
    id: u64,
    result: Result<DecodedThumbnail, String>,
}

struct ThumbnailRequest {
    id: u64,
    url: String,
}

struct PlaylistRequest {
    executable: PathBuf,
    url: String,
}

struct PlaylistExpansionEvent {
    url: String,
    playlist_title: Option<String>,
    result: Result<Option<Vec<PlaylistEntry>>, String>,
}

#[derive(Clone)]
struct DecodedThumbnail {
    rgba: Vec<u8>,
    width: usize,
    height: usize,
}

#[derive(Clone)]
struct ThumbnailCacheEntry {
    image: Option<DecodedThumbnail>,
    texture: Option<egui::TextureHandle>,
}

#[derive(Debug, Clone)]
struct ToolDiagnostic {
    label: &'static str,
    path: Option<PathBuf>,
    version: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeDiagnostics {
    yt_dlp: ToolDiagnostic,
    ffmpeg: ToolDiagnostic,
    yt_dlp_ejs: ToolDiagnostic,
    js_runtime: Option<ToolDiagnostic>,
    plugin_dir: Option<PathBuf>,
    yt_dlp_age_days: Option<i64>,
    refreshed_at: Instant,
}

impl RuntimeDiagnostics {
    fn placeholder() -> Self {
        Self {
            yt_dlp: ToolDiagnostic {
                label: "yt-dlp",
                path: None,
                version: None,
            },
            ffmpeg: ToolDiagnostic {
                label: "ffmpeg",
                path: None,
                version: None,
            },
            yt_dlp_ejs: ToolDiagnostic {
                label: "yt-dlp-ejs",
                path: None,
                version: None,
            },
            js_runtime: None,
            plugin_dir: None,
            yt_dlp_age_days: None,
            refreshed_at: Instant::now(),
        }
    }
}

struct GuiApp {
    config: Config,
    config_path: Option<PathBuf>,
    config_load_error: Option<String>,
    input: String,
    search_query: String,
    output_dir_text: String,
    output_template_text: String,
    rate_limit_text: String,
    socket_timeout_text: String,
    retries_text: String,
    fragment_retries_text: String,
    mode: DownloadMode,
    queue: Vec<GuiQueueItem>,
    next_queue_id: u64,
    queue_row_offsets: Vec<f32>,
    queue_layout_width: f32,
    queue_layout_dirty: bool,
    queue_running: bool,
    active_downloads: HashMap<u64, oneshot::Sender<()>>,
    event_rx: mpsc::UnboundedReceiver<JobEvent>,
    download_request_tx: mpsc::UnboundedSender<DownloadRequest>,
    impersonation_targets: Vec<String>,
    logs: VecDeque<String>,
    status: String,
    folder_picker_rx: Option<Receiver<Result<Option<PathBuf>, String>>>,
    thumbnail_rx: Receiver<QueueThumbnailEvent>,
    thumbnail_request_tx: std_mpsc::Sender<ThumbnailRequest>,
    playlist_request_tx: std_mpsc::Sender<PlaylistRequest>,
    playlist_rx: Receiver<PlaylistExpansionEvent>,
    pending_playlists: HashSet<String>,
    thumbnail_cache: HashMap<String, ThumbnailCacheEntry>,
    thumbnail_lru: VecDeque<String>,
    thumbnail_texture_ready: VecDeque<u64>,
    thumbnail_cache_dir: Option<PathBuf>,
    diagnostics: RuntimeDiagnostics,
    diagnostics_rx: Receiver<RuntimeDiagnostics>,
    diagnostics_refresh_pending: bool,
}

impl GuiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config_path = Config::path().ok();
        let (config, config_load_error) = match config_path.as_deref() {
            Some(path) => match Config::load(path) {
                Ok(config) => (config, None),
                Err(error) => (Config::default(), Some(error.to_string())),
            },
            None => (
                Config::default(),
                Some("Could not determine the configuration path".into()),
            ),
        };
        configure_style(&cc.egui_ctx, &config);
        let mode = match config.default_mode.as_str() {
            "audio" => DownloadMode::Audio,
            "mp3" => DownloadMode::Mp3,
            "custom" => DownloadMode::Custom(config.custom_format.clone()),
            _ => DownloadMode::Video,
        };
        let yt_dlp = dependency_path("yt-dlp");
        let impersonation_targets = yt_dlp
            .as_deref()
            .map(available_impersonation_targets)
            .unwrap_or_default();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (download_request_tx, mut download_request_rx) =
            mpsc::unbounded_channel::<DownloadRequest>();
        let (thumbnail_tx, thumbnail_rx) = std_mpsc::channel();
        let (thumbnail_request_tx, thumbnail_request_rx) = std_mpsc::channel::<ThumbnailRequest>();
        let (diagnostics_tx, diagnostics_rx) = std_mpsc::channel();
        let (playlist_request_tx, playlist_request_rx) = std_mpsc::channel::<PlaylistRequest>();
        let (playlist_tx, playlist_rx) = std_mpsc::channel();
        let thumbnail_request_rx = Arc::new(Mutex::new(thumbnail_request_rx));
        let thumbnail_cache_dir = thumbnail_cache_directory();
        let thumbnail_client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| format!("thumbnail client error: {error}"));
        for _ in 0..4 {
            let request_rx = Arc::clone(&thumbnail_request_rx);
            let result_tx = thumbnail_tx.clone();
            let cache_dir = thumbnail_cache_dir.clone();
            let client = thumbnail_client.clone();
            std::thread::spawn(move || loop {
                let request = match request_rx.lock() {
                    Ok(receiver) => receiver.recv(),
                    Err(_) => break,
                };
                let Ok(request) = request else {
                    break;
                };
                let result = client.as_ref().map_err(Clone::clone).and_then(|client| {
                    download_thumbnail(client, &request.url, cache_dir.as_deref())
                });
                let _ = result_tx.send(QueueThumbnailEvent {
                    id: request.id,
                    result,
                });
            });
        }
        std::thread::spawn(move || {
            while let Ok(request) = playlist_request_rx.recv() {
                let result = expand_playlist_urls(&request.executable, &request.url);
                let title = playlist_title(&request.executable, &request.url);
                let _ = playlist_tx.send(PlaylistExpansionEvent {
                    url: request.url,
                    playlist_title: title,
                    result,
                });
            }
        });
        let worker_event_tx = event_tx;
        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                while let Some(request) = download_request_rx.recv().await {
                    let event_tx = worker_event_tx.clone();
                    tokio::spawn(async move {
                        let (download_tx, mut download_rx) = mpsc::unbounded_channel();
                        let forward = tokio::spawn(async move {
                            while let Some(event) = download_rx.recv().await {
                                let _ = event_tx.send(JobEvent {
                                    id: request.id,
                                    event,
                                });
                            }
                        });
                        request
                            .downloader
                            .run(request.args, request.cancel_rx, download_tx)
                            .await;
                        let _ = forward.await;
                    });
                }
            });
        });
        let status = if let Some(error) = &config_load_error {
            format!("Configuration not loaded: {error}")
        } else if yt_dlp.is_some() {
            "Ready".to_owned()
        } else {
            "yt-dlp was not found in PATH or beside the application".to_owned()
        };
        let mut app = Self {
            output_dir_text: config.output_dir.to_string_lossy().into_owned(),
            output_template_text: config.output_template.clone(),
            rate_limit_text: config.rate_limit.clone(),
            socket_timeout_text: config.socket_timeout.clone(),
            retries_text: config.retries.clone(),
            fragment_retries_text: config.fragment_retries.clone(),
            search_query: String::new(),
            config,
            config_path,
            config_load_error,
            input: String::new(),
            mode,
            queue: Vec::new(),
            next_queue_id: 1,
            queue_row_offsets: vec![0.0],
            queue_layout_width: 0.0,
            queue_layout_dirty: true,
            queue_running: false,
            active_downloads: HashMap::new(),
            event_rx,
            download_request_tx,
            impersonation_targets,
            logs: VecDeque::new(),
            status,
            folder_picker_rx: None,
            thumbnail_rx,
            thumbnail_request_tx,
            playlist_request_tx,
            playlist_rx,
            pending_playlists: HashSet::new(),
            thumbnail_cache: HashMap::new(),
            thumbnail_lru: VecDeque::new(),
            thumbnail_texture_ready: VecDeque::new(),
            thumbnail_cache_dir,
            diagnostics: RuntimeDiagnostics::placeholder(),
            diagnostics_rx,
            diagnostics_refresh_pending: false,
        };
        app.request_diagnostics_refresh(diagnostics_tx);
        app
    }

    fn save_config(&mut self) {
        if let Some(error) = &self.config_load_error {
            self.status = format!(
                "Configuration was not saved because the existing file could not be loaded: {error}"
            );
            return;
        }
        if let Some(path) = &self.config_path {
            if let Err(error) = self.config.save(path) {
                self.status = error.to_string();
            }
        }
    }

    fn current_theme(&self) -> GuiTheme {
        GuiTheme::from_config(&self.config.gui_theme)
    }

    fn set_theme(&mut self, theme: GuiTheme) {
        self.config.gui_theme = theme.config_value().into();
        self.save_config();
    }

    fn search_platform(&self) -> SearchPlatform {
        SearchPlatform::from_config(&self.config.search_platform)
    }

    fn set_search_platform(&mut self, platform: SearchPlatform) {
        self.config.search_platform = platform.config_value().into();
        self.save_config();
    }

    fn open_search(&mut self) {
        match open_platform_search(&self.search_query, self.search_platform()) {
            Ok(url) => {
                self.status = format!("Opened {} search: {url}", self.search_platform().label())
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn log(&mut self, message: impl Into<String>) {
        if self.logs.len() == 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(message.into());
    }

    fn add_input(&mut self) {
        let values: Vec<_> = self.input.split_whitespace().map(str::to_owned).collect();
        if values.is_empty() {
            self.status = "Enter at least one URL".into();
            return;
        }
        let mut added = 0;
        let pending_before = self.pending_playlists.len();
        for url in values {
            match self.expand_or_enqueue(&url) {
                Ok(count) => added += count,
                Err(error) => self.log(format!("Rejected URL: {error}")),
            }
        }
        if added > 0 {
            self.input.clear();
            self.status = format!("Added {added} item(s) to the queue");
        } else if self.pending_playlists.len() > pending_before {
            self.input.clear();
            self.status = "Inspecting playlist in the background…".into();
        } else {
            self.status = "No valid URLs were added".into();
        }
    }

    fn paste_input_from_clipboard(&mut self) -> bool {
        match read_clipboard_text() {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    self.status = "Clipboard is empty".into();
                    false
                } else {
                    self.input = trimmed.to_owned();
                    self.status = "Pasted URL(s) from clipboard".into();
                    true
                }
            }
            Err(error) => {
                self.status = error;
                false
            }
        }
    }

    fn expand_or_enqueue(&mut self, url: &str) -> Result<usize, String> {
        validate_url(url).map_err(|error| error.to_string())?;
        let supports_playlists = supports_playlist_expansion(url);
        if supports_playlists && !self.config.allow_playlists {
            return Err(
                "Allow playlists is disabled. Enable it in Advanced yt-dlp options before adding supported playlist URLs."
                    .into(),
            );
        }
        if self.config.allow_playlists && supports_playlists {
            let yt_dlp = dependency_path("yt-dlp").ok_or_else(|| {
                "yt-dlp was not found in PATH or beside the application".to_owned()
            })?;
            if self.pending_playlists.contains(url) {
                return Err(format!("playlist inspection is already running: {url}"));
            }
            self.playlist_request_tx
                .send(PlaylistRequest {
                    executable: yt_dlp,
                    url: url.to_owned(),
                })
                .map_err(|_| "playlist worker is unavailable".to_owned())?;
            self.pending_playlists.insert(url.to_owned());
            self.log(format!("Inspecting playlist: {url}"));
            return Ok(0);
        }
        Ok(usize::from(self.enqueue_url(url)))
    }

    fn enqueue_url(&mut self, url: &str) -> bool {
        self.enqueue_queue_item(None, url, None, None, true)
    }

    fn enqueue_playlist_entry(
        &mut self,
        entry: &PlaylistEntry,
        playlist_folder: Option<&str>,
    ) -> bool {
        self.enqueue_queue_item(
            entry.title.clone(),
            &entry.url,
            entry.thumbnail_url.clone(),
            playlist_folder,
            false,
        )
    }

    fn enqueue_queue_item(
        &mut self,
        title: Option<String>,
        url: &str,
        thumbnail_url: Option<String>,
        playlist_folder: Option<&str>,
        log_add: bool,
    ) -> bool {
        if self.queue.iter().any(|item| item.url.as_ref() == url) {
            return false;
        }
        if log_add {
            self.log(format!("Added to queue: {url}"));
        }
        let id = self.next_queue_id;
        self.next_queue_id = self.next_queue_id.wrapping_add(1).max(1);
        self.queue.push(GuiQueueItem {
            id,
            title: title.map(String::into_boxed_str),
            url: url.to_owned().into_boxed_str(),
            thumbnail_url: thumbnail_url.map(String::into_boxed_str),
            playlist_folder: playlist_folder
                .map(str::to_owned)
                .map(String::into_boxed_str),
            thumbnail_requested: false,
            thumbnail_failed: false,
            thumbnail_image: None,
            thumbnail_texture: None,
            state: DownloadState::Waiting,
            progress: 0.0,
            progress_text: String::new(),
            error: None,
        });
        self.queue_layout_dirty = true;
        true
    }

    fn request_thumbnail(&mut self, index: usize) {
        let Some(item) = self.queue.get(index) else {
            return;
        };
        let id = item.id;
        if item.thumbnail_requested
            || item.thumbnail_failed
            || item.thumbnail_texture.is_some()
            || item.thumbnail_image.is_some()
        {
            return;
        }
        let Some(thumbnail_url) = item.thumbnail_url.clone() else {
            return;
        };
        let thumbnail_key = thumbnail_url.to_string();
        if let Some(entry) = self.thumbnail_cache.get(&thumbnail_key).cloned() {
            self.touch_thumbnail_cache(&thumbnail_key);
            if let Some(item) = self.queue.get_mut(index) {
                if let Some(texture) = entry.texture {
                    item.thumbnail_texture = Some(texture);
                } else if let Some(image) = entry.image {
                    item.thumbnail_image = Some(image);
                    self.thumbnail_texture_ready.push_back(id);
                }
            }
            return;
        }
        if let Some(item) = self.queue.get_mut(index) {
            item.thumbnail_requested = true;
        }
        let _ = self.thumbnail_request_tx.send(ThumbnailRequest {
            id,
            url: thumbnail_key,
        });
    }

    fn touch_thumbnail_cache(&mut self, key: &str) {
        self.thumbnail_lru.retain(|entry| entry != key);
        self.thumbnail_lru.push_back(key.to_owned());
        while self.thumbnail_lru.len() > THUMBNAIL_CACHE_LIMIT {
            if let Some(expired) = self.thumbnail_lru.pop_front() {
                self.thumbnail_cache.remove(&expired);
            }
        }
    }

    fn cache_thumbnail_image(&mut self, key: &str, image: DecodedThumbnail) {
        let entry = self
            .thumbnail_cache
            .entry(key.to_owned())
            .or_insert(ThumbnailCacheEntry {
                image: None,
                texture: None,
            });
        entry.image = Some(image);
        self.touch_thumbnail_cache(key);
    }

    fn cache_thumbnail_texture(&mut self, key: &str, texture: egui::TextureHandle) {
        let entry = self
            .thumbnail_cache
            .entry(key.to_owned())
            .or_insert(ThumbnailCacheEntry {
                image: None,
                texture: None,
            });
        entry.texture = Some(texture);
        entry.image = None;
        self.touch_thumbnail_cache(key);
    }

    fn request_diagnostics_refresh(&mut self, tx: std_mpsc::Sender<RuntimeDiagnostics>) {
        self.diagnostics_refresh_pending = true;
        std::thread::spawn(move || {
            let _ = tx.send(collect_runtime_diagnostics());
        });
    }

    fn apply_output_dir(&mut self) -> bool {
        let trimmed = self.output_dir_text.trim();
        if trimmed.is_empty() {
            self.status = "Output folder cannot be empty".into();
            return false;
        }
        self.config.output_dir = PathBuf::from(trimmed);
        self.output_dir_text = trimmed.to_owned();
        self.save_config();
        true
    }

    fn apply_output_template(&mut self) -> bool {
        let trimmed = self.output_template_text.trim();
        if let Err(error) = validate_output_template(trimmed) {
            self.status = error;
            return false;
        }
        self.config.output_template = trimmed.to_owned();
        self.output_template_text = trimmed.to_owned();
        self.save_config();
        true
    }

    fn apply_rate_limit(&mut self) -> bool {
        let trimmed = self.rate_limit_text.trim();
        if let Err(error) = validate_rate_limit(trimmed) {
            self.status = error;
            return false;
        }
        self.config.rate_limit = trimmed.to_owned();
        self.rate_limit_text = trimmed.to_owned();
        self.save_config();
        true
    }

    fn apply_socket_timeout(&mut self) -> bool {
        let trimmed = self.socket_timeout_text.trim();
        if let Err(error) = validate_socket_timeout(trimmed) {
            self.status = error;
            return false;
        }
        self.config.socket_timeout = trimmed.to_owned();
        self.socket_timeout_text = trimmed.to_owned();
        self.save_config();
        true
    }

    fn apply_retries(&mut self) -> bool {
        let trimmed = self.retries_text.trim();
        if let Err(error) = validate_retry_count(trimmed, "Retries") {
            self.status = error;
            return false;
        }
        self.config.retries = trimmed.to_owned();
        self.retries_text = trimmed.to_owned();
        self.save_config();
        true
    }

    fn apply_fragment_retries(&mut self) -> bool {
        let trimmed = self.fragment_retries_text.trim();
        if let Err(error) = validate_retry_count(trimmed, "Fragment retries") {
            self.status = error;
            return false;
        }
        self.config.fragment_retries = trimmed.to_owned();
        self.fragment_retries_text = trimmed.to_owned();
        self.save_config();
        true
    }

    fn apply_network_tuning(&mut self) -> bool {
        self.apply_socket_timeout() && self.apply_retries() && self.apply_fragment_retries()
    }

    fn start_queue(&mut self) {
        if self.active_downloads.is_empty()
            && !self
                .queue
                .iter()
                .any(|item| item.state == DownloadState::Waiting)
        {
            self.status = "Queue is empty".into();
            return;
        }
        self.queue_running = true;
        self.fill_slots();
    }

    fn pause_queue(&mut self) {
        self.queue_running = false;
        if self.active_downloads.is_empty() {
            self.status = "Queue paused".into();
        } else {
            self.status = format!(
                "Queue paused, letting {} active download(s) finish",
                self.active_downloads.len()
            );
        }
    }

    fn fill_slots(&mut self) {
        let max_active = usize::from(self.config.max_active_downloads.clamp(1, 8));
        while self.queue_running && self.active_downloads.len() < max_active {
            let Some(index) = self
                .queue
                .iter()
                .position(|item| item.state == DownloadState::Waiting)
            else {
                if self.active_downloads.is_empty() {
                    self.queue_running = false;
                    self.status = "Queue finished".into();
                }
                return;
            };
            if !self.start_job(index) {
                self.queue_running = false;
                return;
            }
        }
    }

    fn start_job(&mut self, index: usize) -> bool {
        let Some(yt_dlp) = dependency_path("yt-dlp") else {
            self.fail(
                index,
                "yt-dlp was not found in PATH or beside the application",
            );
            return false;
        };
        if (self.mode.needs_ffmpeg() || self.config.embed_metadata)
            && dependency_path("ffmpeg").is_none()
        {
            self.fail(
                index,
                if self.config.embed_metadata && !self.mode.needs_ffmpeg() {
                    "ffmpeg is required to embed metadata"
                } else {
                    "ffmpeg is required for this download mode"
                },
            );
            return false;
        }
        if !self.apply_output_dir()
            || !self.apply_output_template()
            || !self.apply_rate_limit()
            || !self.apply_network_tuning()
        {
            self.fail(index, &self.status.clone());
            return false;
        }

        let url = self.queue[index].url.clone();
        let id = self.queue[index].id;
        let mode = self.mode.clone();
        let playlist_folder = self.queue[index].playlist_folder.clone();
        let options =
            match OwnedDownloadOptions::from_config(&self.config, &url, playlist_folder.as_deref())
            {
                Ok(options) => options,
                Err(message) => {
                    self.fail(index, &message);
                    return false;
                }
            };
        let downloader = Downloader::new(yt_dlp, self.config.output_dir.clone());
        let args = match downloader.arguments(&url, &mode, options.borrow()) {
            Ok(args) => args,
            Err(message) => {
                self.fail(index, &message);
                return false;
            }
        };
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.active_downloads.insert(id, cancel_tx);
        self.queue[index].state = DownloadState::Downloading;
        self.queue[index].progress = 0.0;
        self.queue[index].progress_text.clear();
        self.queue[index].error = None;
        self.queue_layout_dirty = true;
        self.status = format!("Running {} active download(s)", self.active_downloads.len());
        self.log(format!("Starting download: {url}"));

        if self
            .download_request_tx
            .send(DownloadRequest {
                id,
                downloader,
                args,
                cancel_rx,
            })
            .is_err()
        {
            self.active_downloads.remove(&id);
            self.fail(index, "download worker is unavailable");
            return false;
        }
        true
    }

    fn fail(&mut self, index: usize, message: &str) {
        let message = friendly_error(&self.queue[index].url, message);
        self.queue[index].state = DownloadState::Failed;
        self.queue[index].error = Some(message.clone());
        self.queue_layout_dirty = true;
        self.status = message.clone();
        self.log(format!("ERROR: {message}"));
    }

    fn cancel(&mut self) {
        self.queue_running = false;
        if self.active_downloads.is_empty() {
            self.status = "No active downloads".into();
            return;
        }
        for (_, cancel_tx) in self.active_downloads.drain() {
            let _ = cancel_tx.send(());
        }
        self.status = "Cancelling active downloads…".into();
    }

    fn restart_failed_or_cancelled(&mut self) {
        let mut restarted = 0usize;
        for item in &mut self.queue {
            if matches!(item.state, DownloadState::Failed | DownloadState::Cancelled) {
                item.state = DownloadState::Waiting;
                item.progress = 0.0;
                item.progress_text.clear();
                item.error = None;
                restarted += 1;
            }
        }

        if restarted == 0 {
            self.status = "No failed or cancelled items to restart".into();
            return;
        }
        self.queue_layout_dirty = true;

        self.status = format!("Restarted {restarted} item(s)");
        self.log(format!("Restarted {restarted} failed/cancelled item(s)"));
        self.start_queue();
    }

    fn has_restartable_items(&self) -> bool {
        self.queue
            .iter()
            .any(|item| matches!(item.state, DownloadState::Failed | DownloadState::Cancelled))
    }

    fn has_waiting_items(&self) -> bool {
        self.queue
            .iter()
            .any(|item| item.state == DownloadState::Waiting)
    }

    fn process_events(&mut self) {
        while let Ok(job) = self.event_rx.try_recv() {
            let Some(index) = self.queue.iter().position(|item| item.id == job.id) else {
                continue;
            };
            match job.event {
                DownloadEvent::Progress { percent, text } => {
                    self.queue[index].progress = percent.unwrap_or_default() as f32 / 100.0;
                    self.queue[index].progress_text = text;
                }
                DownloadEvent::Finished => {
                    self.queue_layout_dirty = true;
                    self.queue[index].state = DownloadState::Finished;
                    self.queue[index].progress = 1.0;
                    self.queue[index].progress_text = "done".into();
                    self.queue[index].error = None;
                    self.active_downloads.remove(&job.id);
                    self.log(format!("Finished: {}", self.queue[index].url));
                    self.status = "Download finished".into();
                }
                DownloadEvent::Failed(message) => {
                    self.queue_layout_dirty = true;
                    let message = friendly_error(&self.queue[index].url, &message);
                    self.queue[index].state = DownloadState::Failed;
                    self.queue[index].error = Some(message.clone());
                    self.active_downloads.remove(&job.id);
                    self.log(format!("ERROR: {message}"));
                    self.status = message;
                }
                DownloadEvent::Cancelled => {
                    self.queue_layout_dirty = true;
                    self.queue[index].state = DownloadState::Cancelled;
                    self.queue[index].progress_text = "cancelled".into();
                    self.active_downloads.remove(&job.id);
                    self.log(format!("Cancelled: {}", self.queue[index].url));
                    self.status = "Download cancelled".into();
                }
            }
        }

        while let Ok(event) = self.thumbnail_rx.try_recv() {
            let Some(index) = self.queue.iter().position(|item| item.id == event.id) else {
                continue;
            };
            let thumbnail_url = self.queue[index]
                .thumbnail_url
                .as_deref()
                .map(str::to_owned);
            if let Some(item) = self.queue.get_mut(index) {
                match event.result {
                    Ok(image) => {
                        item.thumbnail_requested = false;
                        item.thumbnail_failed = false;
                        item.thumbnail_image = Some(image.clone());
                        self.thumbnail_texture_ready.push_back(event.id);
                        if let Some(url) = thumbnail_url.as_deref() {
                            self.cache_thumbnail_image(url, image);
                        }
                    }
                    Err(_) => {
                        item.thumbnail_requested = false;
                        item.thumbnail_failed = true;
                    }
                }
            }
        }

        while let Ok(event) = self.playlist_rx.try_recv() {
            self.pending_playlists.remove(&event.url);
            match event.result {
                Ok(Some(entries)) if !entries.is_empty() => {
                    let mut count = 0;
                    let folder = playlist_folder_name(&event.url, event.playlist_title.as_deref());
                    for entry in &entries {
                        count += usize::from(self.enqueue_playlist_entry(entry, folder.as_deref()));
                    }
                    self.log(format!(
                        "Expanded playlist into {count} new item(s): {}",
                        event.url
                    ));
                    self.status = format!("Added {count} new playlist item(s) to the queue");
                }
                Ok(_) => {
                    let message = format!(
                        "playlist expansion returned no entries for supported URL: {}",
                        event.url
                    );
                    self.log(format!("Playlist expansion failed: {message}"));
                    self.status = message;
                }
                Err(error) => {
                    self.log(format!("Playlist expansion failed: {error}"));
                    self.status = error;
                }
            }
        }

        while let Ok(diagnostics) = self.diagnostics_rx.try_recv() {
            self.diagnostics = diagnostics;
            self.diagnostics_refresh_pending = false;
            self.status = "Diagnostics refreshed".into();
        }

        if self.queue_running {
            self.fill_slots();
        } else if self.active_downloads.is_empty()
            && self
                .queue
                .iter()
                .all(|item| item.state != DownloadState::Waiting)
        {
            self.status = "Queue idle".into();
        }

        self.poll_folder_picker();
    }

    fn ensure_thumbnail_textures(&mut self, ctx: &egui::Context) {
        while let Some(id) = self.thumbnail_texture_ready.pop_front() {
            let Some(index) = self.queue.iter().position(|item| item.id == id) else {
                continue;
            };
            if self.queue[index].thumbnail_texture.is_some() {
                continue;
            }
            let thumbnail_url = self.queue[index]
                .thumbnail_url
                .as_deref()
                .map(str::to_owned);
            let Some(image) = self.queue[index].thumbnail_image.take() else {
                continue;
            };
            let texture = ctx.load_texture(
                format!("queue-thumb-{index}"),
                egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba),
                egui::TextureOptions::LINEAR,
            );
            if let Some(url) = thumbnail_url.as_deref() {
                self.cache_thumbnail_texture(url, texture.clone());
            }
            self.queue[index].thumbnail_texture = Some(texture);
        }
    }

    fn ensure_queue_layout(&mut self, width: f32) {
        if !self.queue_layout_dirty
            && (self.queue_layout_width - width).abs() < 1.0
            && self.queue_row_offsets.len() == self.queue.len() + 1
        {
            return;
        }
        self.queue_row_offsets.clear();
        self.queue_row_offsets.reserve(self.queue.len() + 1);
        self.queue_row_offsets.push(0.0);
        let mut offset = 0.0;
        for item in &self.queue {
            offset += queue_row_height(item, width);
            self.queue_row_offsets.push(offset);
        }
        self.queue_layout_width = width;
        self.queue_layout_dirty = false;
    }

    fn poll_folder_picker(&mut self) {
        let Some(receiver) = self.folder_picker_rx.take() else {
            return;
        };

        match receiver.try_recv() {
            Ok(Ok(Some(path))) => {
                self.config.output_dir = path.clone();
                self.output_dir_text = path.display().to_string();
                self.save_config();
                self.status = format!("Selected output folder: {}", path.display());
            }
            Ok(Ok(None)) => {
                self.status = "Folder selection cancelled".into();
            }
            Ok(Err(error)) => {
                self.status = error;
            }
            Err(TryRecvError::Empty) => {
                self.folder_picker_rx = Some(receiver);
            }
            Err(TryRecvError::Disconnected) => {
                self.status = "Folder picker closed unexpectedly".into();
            }
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        section_frame(ui, "URL", |ui| {
            let mut paste_clicked = false;
            let response = ui
                .horizontal(|ui| {
                    let response = text_edit_with_context_menu(
                        ui,
                        &mut self.input,
                        "https://example.com/video",
                        (ui.available_width() - 78.0).max(120.0),
                    );
                    paste_clicked = ui.button("Paste").clicked();
                    response
                })
                .inner;
            if paste_clicked {
                response.request_focus();
                if !self.paste_input_from_clipboard() {
                    // eframe owns the native clipboard integration and turns
                    // this request into an egui Paste event for the focused field.
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    self.status = "Paste requested from the desktop clipboard…".into();
                }
                ui.ctx().request_repaint();
            }
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.add_input();
            }
            ui.add_space(8.0);
            if ui
                .add_sized(
                    [ui.available_width(), 38.0],
                    egui::Button::new(RichText::new("＋  Add to queue").strong()).fill(BLUE),
                )
                .clicked()
            {
                self.add_input();
            }
        });

        ui.add_space(14.0);
        section_frame(ui, "Appearance", |ui| {
            egui::ComboBox::from_id_salt("gui-theme")
                .selected_text(self.current_theme().label())
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for theme in GuiTheme::ALL {
                        if ui
                            .selectable_label(self.current_theme() == theme, theme.label())
                            .clicked()
                        {
                            self.set_theme(theme);
                        }
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Opacity");
                if ui
                    .add(
                        egui::Slider::new(&mut self.config.gui_opacity, 0.70..=1.0)
                            .show_value(true),
                    )
                    .changed()
                {
                    self.config.gui_opacity = self.config.gui_opacity.clamp(0.70, 1.0);
                    self.save_config();
                }
            });
            ui.small("Theme changes apply live. Lower opacity softens panel chrome.");
        });

        ui.add_space(14.0);
        section_frame(ui, "Search in browser", |ui| {
            let response = text_edit_with_context_menu(
                ui,
                &mut self.search_query,
                "Search query",
                f32::INFINITY,
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.open_search();
            }
            ui.add_space(8.0);
            egui::ComboBox::from_id_salt("search-platform")
                .selected_text(self.search_platform().label())
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for platform in SearchPlatform::ALL {
                        if ui
                            .selectable_label(self.search_platform() == platform, platform.label())
                            .clicked()
                        {
                            self.set_search_platform(platform);
                        }
                    }
                });
            ui.add_space(8.0);
            if ui
                .add_sized(
                    [ui.available_width(), 34.0],
                    egui::Button::new(RichText::new("Open search in browser").strong()),
                )
                .clicked()
            {
                self.open_search();
            }
        });

        ui.add_space(14.0);
        section_frame(ui, "Download mode", |ui| {
            let previous_mode = self.mode.clone();
            egui::ComboBox::from_id_salt("download-mode")
                .selected_text(self.mode.label())
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.mode,
                        DownloadMode::Video,
                        "Video - best quality",
                    );
                    ui.selectable_value(
                        &mut self.mode,
                        DownloadMode::Audio,
                        "Audio - best quality",
                    );
                    ui.selectable_value(&mut self.mode, DownloadMode::Mp3, "MP3 - convert audio");
                    ui.selectable_value(
                        &mut self.mode,
                        DownloadMode::Custom(self.config.custom_format.clone()),
                        "Custom format",
                    );
                });
            if matches!(self.mode, DownloadMode::Custom(_)) {
                ui.add_space(8.0);
                if text_edit_with_context_menu(
                    ui,
                    &mut self.config.custom_format,
                    "yt-dlp format selector",
                    f32::INFINITY,
                )
                .changed()
                {
                    self.save_config();
                }
                self.mode = DownloadMode::Custom(self.config.custom_format.clone());
            }
            if self.mode != previous_mode {
                self.config.default_mode = match self.mode {
                    DownloadMode::Video => "video",
                    DownloadMode::Audio => "audio",
                    DownloadMode::Mp3 => "mp3",
                    DownloadMode::Custom(_) => "custom",
                }
                .into();
                self.save_config();
            }
        });

        ui.add_space(14.0);
        section_frame(ui, "Output folder", |ui| {
            let folder_response = text_edit_with_context_menu(
                ui,
                &mut self.output_dir_text,
                "/home/user/Downloads",
                f32::INFINITY,
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Use typed path").clicked() {
                    self.apply_output_dir();
                }
                let browse_label = if self.folder_picker_rx.is_some() {
                    "Browse… (opening)"
                } else {
                    "Browse..."
                };
                if ui
                    .add_enabled(
                        self.folder_picker_rx.is_none(),
                        egui::Button::new(browse_label),
                    )
                    .clicked()
                {
                    self.browse_output_dir();
                }
            });
            if folder_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                self.apply_output_dir();
            }
        });

        ui.add_space(14.0);
        section_frame(ui, "Cookies browser", |ui| {
            egui::ComboBox::from_id_salt("cookies-browser")
                .selected_text(display_none(&self.config.cookies_browser))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for browser in [
                        "none", "firefox", "chrome", "chromium", "brave", "edge", "vivaldi",
                        "safari",
                    ] {
                        if ui
                            .selectable_value(
                                &mut self.config.cookies_browser,
                                browser.to_owned(),
                                display_none(browser),
                            )
                            .changed()
                        {
                            self.save_config();
                        }
                    }
                });
        });

        ui.add_space(14.0);
        section_frame(ui, "Impersonation", |ui| {
            egui::ComboBox::from_id_salt("impersonation")
                .selected_text(display_none(&self.config.impersonation))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut self.config.impersonation, "none".into(), "None")
                        .changed()
                    {
                        self.save_config();
                    }
                    if ui
                        .selectable_value(
                            &mut self.config.impersonation,
                            "any".into(),
                            "Any available",
                        )
                        .changed()
                    {
                        self.save_config();
                    }
                    for target in self.impersonation_targets.clone() {
                        if ui
                            .selectable_value(
                                &mut self.config.impersonation,
                                target.clone(),
                                &target,
                            )
                            .changed()
                        {
                            self.save_config();
                        }
                    }
                });
        });

        ui.add_space(14.0);
        section_frame(ui, "Advanced yt-dlp options", |ui| {
            self.advanced_settings_panel(ui)
        });

        ui.add_space(14.0);
        section_frame(ui, "Environment", |ui| self.diagnostics_panel(ui));
    }

    fn browse_output_dir(&mut self) {
        if self.folder_picker_rx.is_some() {
            return;
        }

        let start_dir = self.config.output_dir.clone();
        let (tx, rx) = std_mpsc::channel();
        self.folder_picker_rx = Some(rx);
        self.status = "Opening folder picker…".into();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let result = match runtime {
                Ok(runtime) => runtime.block_on(async move {
                    Ok::<_, String>(
                        rfd::AsyncFileDialog::new()
                            .set_directory(&start_dir)
                            .pick_folder()
                            .await
                            .map(|handle| handle.path().to_path_buf()),
                    )
                }),
                Err(error) => Err(format!("Could not start folder picker runtime: {error}")),
            };
            let _ = tx.send(result);
        });
    }

    fn advanced_settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Filename template").strong());
        let template_response = text_edit_with_context_menu(
            ui,
            &mut self.output_template_text,
            "%(title)s [%(id)s].%(ext)s",
            f32::INFINITY,
        );
        if template_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.apply_output_template();
        }
        ui.small("Maps to yt-dlp --output and must stay relative to the output folder.");

        ui.add_space(12.0);
        ui.label(RichText::new("Transfer").strong());
        ui.horizontal(|ui| {
            ui.label("Connections");
            if ui
                .add(egui::Slider::new(
                    &mut self.config.concurrent_fragments,
                    1..=16,
                ))
                .changed()
            {
                self.save_config();
            }
        });
        if ui
            .checkbox(&mut self.config.use_aria2, "Use aria2")
            .changed()
        {
            self.save_config();
        }
        ui.small("More than 8 connections can increase throttling or HTTP 403 errors.");

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Parallel downloads");
            if ui
                .add(
                    egui::DragValue::new(&mut self.config.max_active_downloads)
                        .range(1..=8)
                        .speed(0.1),
                )
                .changed()
            {
                self.config.max_active_downloads = self.config.max_active_downloads.clamp(1, 8);
                self.save_config();
            }
        });

        if ui
            .checkbox(
                &mut self.config.allow_playlists,
                "Allow playlists (supported: YouTube, PMVHaven, SpankBang)",
            )
            .changed()
        {
            self.save_config();
        }
        ui.small(
            "Enabled by default. Supported playlist URLs are expanded into queue entries before downloading.",
        );
        if ui
            .checkbox(
                &mut self.config.playlist_subfolders,
                "Create a folder for each playlist",
            )
            .changed()
        {
            self.save_config();
        }
        if ui
            .checkbox(&mut self.config.embed_metadata, "Embed metadata and tags")
            .changed()
        {
            self.save_config();
        }
        if ui
            .checkbox(
                &mut self.config.write_info_json,
                "Write .info.json metadata",
            )
            .changed()
        {
            self.save_config();
        }
        ui.small("Tags are copied only when the extractor reports them from the source page.");

        ui.horizontal(|ui| {
            ui.label("Speed limit");
            let response = text_edit_with_context_menu(
                ui,
                &mut self.rate_limit_text,
                "blank = unlimited, e.g. 5M",
                170.0,
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.apply_rate_limit();
            }
            if ui.button("Apply").clicked() {
                self.apply_rate_limit();
            }
        });
        ui.small("The limit applies per active download process.");

        ui.add_space(8.0);
        ui.label(RichText::new("Network resilience").strong());

        ui.horizontal(|ui| {
            ui.label("Socket timeout");
            let response = text_edit_with_context_menu(
                ui,
                &mut self.socket_timeout_text,
                "auto / 60 for PMVHaven",
                170.0,
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.apply_socket_timeout();
            }
        });

        ui.horizontal(|ui| {
            ui.label("Retries");
            let response = text_edit_with_context_menu(
                ui,
                &mut self.retries_text,
                "auto / 10 for PMVHaven",
                170.0,
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.apply_retries();
            }
        });

        ui.horizontal(|ui| {
            ui.label("Fragment retries");
            let response = text_edit_with_context_menu(
                ui,
                &mut self.fragment_retries_text,
                "auto / 10 for PMVHaven",
                170.0,
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.apply_fragment_retries();
            }
        });

        if ui.button("Apply tuning").clicked() {
            self.apply_network_tuning();
        }
        ui.small("Blank keeps yt-dlp defaults, except PMVHaven uses 60 / 10 / 10.");
    }

    fn diagnostics_panel(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                !self.diagnostics_refresh_pending,
                egui::Button::new("Refresh diagnostics"),
            )
            .clicked()
        {
            let (tx, rx) = std_mpsc::channel();
            self.diagnostics_rx = rx;
            self.request_diagnostics_refresh(tx);
        }
        if self.diagnostics_refresh_pending {
            ui.small("Refreshing diagnostics…");
        } else {
            ui.small(format!(
                "Last refreshed: {}s ago",
                self.diagnostics.refreshed_at.elapsed().as_secs()
            ));
        }
        ui.add_space(8.0);
        diagnostic_row(
            ui,
            &self.diagnostics.yt_dlp,
            self.diagnostics.yt_dlp_age_days,
        );
        diagnostic_row(ui, &self.diagnostics.ffmpeg, None);
        diagnostic_row(ui, &self.diagnostics.yt_dlp_ejs, None);
        if let Some(runtime) = &self.diagnostics.js_runtime {
            diagnostic_row(ui, runtime, None);
        } else {
            ui.label(RichText::new("js runtime: missing").color(AMBER));
        }
        ui.label(format!(
            "plugins: {}",
            self.diagnostics
                .plugin_dir
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "not found".into())
        ));
        ui.add_space(8.0);
        if self.diagnostics.yt_dlp.path.is_none() {
            ui.label(RichText::new("Install yt-dlp").strong());
            selectable_command(ui, "sudo pacman -S yt-dlp");
        }
        if self.diagnostics.ffmpeg.path.is_none() {
            ui.label(RichText::new("Install ffmpeg for muxing / audio conversion").strong());
            selectable_command(ui, "sudo pacman -S ffmpeg");
        }
        if self.diagnostics.yt_dlp_ejs.path.is_none() || self.diagnostics.js_runtime.is_none() {
            ui.label(
                RichText::new("Full YouTube support is incomplete")
                    .color(AMBER)
                    .strong(),
            );
            if self.diagnostics.yt_dlp_ejs.path.is_none() {
                selectable_command(ui, "python -m pip install -U yt-dlp-ejs");
            }
            if self.diagnostics.js_runtime.is_none() {
                selectable_command(ui, "sudo pacman -S deno");
            }
            ui.small("yt-dlp upstream expects yt-dlp-ejs plus a supported JavaScript runtime for full YouTube challenge handling.");
        }
    }

    fn queue_panel(&mut self, ui: &mut egui::Ui, max_height: f32) {
        section_frame(ui, &format!("Queue ({})", self.queue.len()), |ui| {
            let full_width = ui.available_width();
            let number_width = 22.0;
            let thumb_width = if full_width < 760.0 { 72.0 } else { 84.0 };
            let status_width = if full_width < 760.0 { 92.0 } else { 112.0 };
            let progress_width = if full_width < 760.0 { 132.0 } else { 170.0 };
            let gap = 12.0;
            let info_width = (full_width
                - number_width
                - thumb_width
                - status_width
                - progress_width
                - gap * 4.0)
                .max(180.0);
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(number_width, 20.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new("#").strong().color(Color32::GRAY));
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(thumb_width, 20.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |_| {},
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(info_width, 20.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new("Item").strong().color(Color32::GRAY));
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(status_width, 20.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new("Status").strong().color(Color32::GRAY));
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(progress_width, 20.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new("Progress").strong().color(Color32::GRAY));
                    },
                );
            });
            ui.separator();
            if self.queue.is_empty() {
                ui.add_space(50.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Queue is empty").size(18.0));
                    ui.label("Add one or more URLs to begin.");
                });
            } else {
                let mut visible_ids = HashSet::new();
                self.ensure_queue_layout(full_width);
                egui::ScrollArea::vertical()
                    .max_height(max_height.max(120.0))
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, viewport| {
                        let start_y = viewport.min.y.max(0.0);
                        let end_y = viewport.max.y;
                        let start_index = self.queue_row_offsets[1..]
                            .partition_point(|row_end| *row_end < start_y);
                        let end_index = self.queue_row_offsets[..self.queue.len()]
                            .partition_point(|row_start| *row_start <= end_y)
                            .max(start_index)
                            .min(self.queue.len());
                        let top_space = self.queue_row_offsets[start_index];
                        let bottom_space = (self.queue_row_offsets[self.queue.len()]
                            - self.queue_row_offsets[end_index])
                            .max(0.0);

                        if top_space > 0.0 {
                            ui.add_space(top_space);
                        }

                        for index in start_index..end_index {
                            visible_ids.insert(self.queue[index].id);
                            self.request_thumbnail(index);
                            let item = &self.queue[index];
                            queue_row(ui, index, item);
                            ui.add_space(6.0);
                        }

                        if bottom_space > 0.0 {
                            ui.add_space(bottom_space);
                        }
                    });
                // Queue rows do not own off-screen GPU textures. The bounded LRU
                // remains the sole long-lived owner, so scrolling a huge queue
                // cannot retain one texture per item indefinitely.
                for item in &mut self.queue {
                    if !visible_ids.contains(&item.id) {
                        item.thumbnail_texture = None;
                        item.thumbnail_image = None;
                    }
                }
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.active_downloads.is_empty(),
                        egui::Button::new("Clear completed"),
                    )
                    .clicked()
                {
                    self.queue.retain(|item| {
                        !matches!(
                            item.state,
                            DownloadState::Finished
                                | DownloadState::Failed
                                | DownloadState::Cancelled
                        )
                    });
                    self.queue_layout_dirty = true;
                }
            });
        });
    }

    fn queue_controls_panel(&mut self, ui: &mut egui::Ui) {
        section_frame(
            ui,
            &format!(
                "Now downloading ({}/{})",
                self.active_downloads.len(),
                self.config.max_active_downloads.clamp(1, 8)
            ),
            |ui| {
                if let Some(id) = active_ids(&self.active_downloads).into_iter().next() {
                    if let Some(item) = self.queue.iter().find(|item| item.id == id) {
                        ui.horizontal(|ui| {
                            thumbnail_placeholder(ui, item);
                            ui.vertical(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(queue_item_title(item)).strong().size(18.0),
                                    )
                                    .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(item.url.as_ref())
                                            .size(12.0)
                                            .color(Color32::GRAY),
                                    )
                                    .truncate(),
                                );
                                ui.add_space(8.0);
                                ui.add(
                                    egui::ProgressBar::new(item.progress)
                                        .desired_width(ui.available_width())
                                        .show_percentage(),
                                );
                                if !item.progress_text.is_empty() {
                                    ui.small(&item.progress_text);
                                }
                            });
                        });
                    }
                } else {
                    ui.label("No active downloads");
                }

                ui.add_space(12.0);
                let has_waiting = self.has_waiting_items();
                let has_active = !self.active_downloads.is_empty();
                let has_restartable = self.has_restartable_items();

                egui::Grid::new("queue_controls")
                    .num_columns(2)
                    .spacing([12.0, 12.0])
                    .show(ui, |ui| {
                        if ui
                            .add_sized(
                                [ui.available_width(), 40.0],
                                egui::Button::new(RichText::new("Start / Resume").strong())
                                    .fill(BLUE),
                            )
                            .clicked()
                        {
                            self.start_queue();
                        }

                        if ui
                            .add_enabled(
                                self.queue_running || has_active,
                                egui::Button::new("Pause queue"),
                            )
                            .clicked()
                        {
                            self.pause_queue();
                        }
                        ui.end_row();

                        if ui
                            .add_enabled(
                                has_active,
                                egui::Button::new(RichText::new("Stop active").color(RED)),
                            )
                            .clicked()
                        {
                            self.cancel();
                        }

                        if ui
                            .add_enabled(has_restartable, egui::Button::new("Restart failed"))
                            .clicked()
                        {
                            self.restart_failed_or_cancelled();
                        }
                        ui.end_row();
                    });

                ui.add_space(4.0);
                ui.small(if self.queue_running {
                    "Queue is running. New waiting items will start until the active limit is reached."
                } else if has_active {
                    "Queue dispatch is paused. Active downloads keep running until they finish or you stop them."
                } else if has_waiting {
                    "Queue is idle. Press Start / Resume to launch waiting items."
                } else {
                    "No waiting items. Add URLs or restart failed entries."
                });
            },
        );
    }

    fn queue_log_panel(&mut self, ui: &mut egui::Ui) {
        section_frame(ui, "Log", |ui| {
            egui::ScrollArea::both()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &self.logs {
                        ui.add(
                            egui::Label::new(egui::RichText::new(line).monospace())
                                .selectable(true),
                        );
                    }
                });
            ui.add_space(8.0);
            if ui.small_button("Clear log").clicked() {
                self.logs.clear();
            }
        });
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        configure_style(ctx, &self.config);
        self.process_events();
        self.ensure_thumbnail_textures(ctx);
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(
                    RichText::new(format!(
                        "crusty-dlp {} · built {}",
                        build_label(),
                        BUILD_TIMESTAMP
                    ))
                    .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&self.status).color(Color32::LIGHT_GRAY))
                            .truncate(),
                    );
                });
            });
        });
        egui::SidePanel::left("settings")
            .resizable(false)
            .default_width(392.0)
            .min_width(352.0)
            .max_width(420.0)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.settings_panel(ui));
                    });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            // Controls come first so Start/Resume stays visible even when the
            // queue or log contains enough content to consume the panel.
            self.queue_controls_panel(ui);
            ui.add_space(10.0);
            let queue_max_height = (ui.available_height() * 0.48).clamp(150.0, 340.0);
            self.queue_panel(ui, queue_max_height);
            ui.add_space(10.0);
            self.queue_log_panel(ui);
        });
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let executable = current_executable_path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not found".into());
                    let plugin_dir = self
                        .diagnostics
                        .plugin_dir
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not found".into());
                    let thumbnail_cache = self
                        .thumbnail_cache_dir
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not found".into());
                    ui.label(format!("exe: {executable}"));
                    ui.separator();
                    ui.label(format!("plugins: {plugin_dir}"));
                    ui.separator();
                    ui.label(format!("thumb-cache: {thumbnail_cache}"));
                });
                ui.horizontal(|ui| {
                    let yt_dlp = self
                        .diagnostics
                        .yt_dlp
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not found".into());
                    ui.label(format!("yt-dlp: {yt_dlp}"));
                    if let Some(version) = &self.diagnostics.yt_dlp.version {
                        ui.separator();
                        ui.label(format!("ver: {version}"));
                    }
                    if let Some(age_days) = self.diagnostics.yt_dlp_age_days {
                        ui.separator();
                        ui.label(format!("age: {age_days}d"));
                    }
                    ui.separator();
                    ui.label(format!("crusty-dlp {} ({BUILD_PROFILE})", build_label()));
                    ui.separator();
                    ui.label(format!("built: {BUILD_TIMESTAMP}"));
                    ui.separator();
                    ui.label(format!(
                        "playlists: {}",
                        if self.config.allow_playlists {
                            "on"
                        } else {
                            "off"
                        }
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let waiting = self
                            .queue
                            .iter()
                            .filter(|item| item.state == DownloadState::Waiting)
                            .count();
                        ui.label(format!("{waiting} queued"));
                        ui.separator();
                        ui.label(format!("{} active", self.active_downloads.len()));
                    });
                });
            });
        });
        if !self.active_downloads.is_empty()
            || self.folder_picker_rx.is_some()
            || self.queue_running
            || self.diagnostics_refresh_pending
            || !self.pending_playlists.is_empty()
            || self.queue.iter().any(|item| item.thumbnail_requested)
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

struct OwnedDownloadOptions {
    impersonation: Option<String>,
    cookies_browser: Option<String>,
    concurrent_fragments: u8,
    use_aria2: bool,
    output_template: String,
    rate_limit: Option<String>,
    socket_timeout: Option<u32>,
    retries: Option<u32>,
    fragment_retries: Option<u32>,
    playlist_subfolder: Option<String>,
    playlist_subfolders: bool,
    embed_metadata: bool,
    write_info_json: bool,
    allow_playlists: bool,
}

impl OwnedDownloadOptions {
    fn from_config(
        config: &Config,
        url: &str,
        playlist_subfolder: Option<&str>,
    ) -> Result<Self, String> {
        validate_output_template(&config.output_template)?;
        validate_rate_limit(&config.rate_limit)?;
        let tuning = resolve_network_tuning(
            url,
            &config.socket_timeout,
            &config.retries,
            &config.fragment_retries,
        )?;

        let browser = (config.cookies_browser != "none").then(|| config.cookies_browser.clone());
        let mut impersonation =
            (config.impersonation != "none").then(|| config.impersonation.clone());
        if url.contains("spankbang.com") && impersonation.is_none() {
            impersonation = browser
                .as_deref()
                .map(browser_impersonation)
                .map(str::to_owned);
        } else if url.contains("boyfriendtv.com") && impersonation.is_none() {
            impersonation = Some("any".into());
        }

        Ok(Self {
            impersonation,
            cookies_browser: browser,
            concurrent_fragments: config.concurrent_fragments.clamp(1, 16),
            use_aria2: config.use_aria2 && dependency_path("aria2c").is_some(),
            output_template: config.output_template.trim().to_owned(),
            rate_limit: (!config.rate_limit.trim().is_empty())
                .then(|| config.rate_limit.trim().to_owned()),
            socket_timeout: tuning.socket_timeout,
            retries: tuning.retries,
            fragment_retries: tuning.fragment_retries,
            playlist_subfolder: playlist_subfolder.map(str::to_owned),
            // GUI playlist expansion supplies an explicit sanitized folder per
            // queue item; do not add %(playlist_title)s to direct-video jobs.
            playlist_subfolders: false,
            embed_metadata: config.embed_metadata,
            write_info_json: config.write_info_json,
            allow_playlists: config.allow_playlists,
        })
    }

    fn borrow(&self) -> DownloadOptions<'_> {
        DownloadOptions {
            impersonation: self.impersonation.as_deref(),
            cookies_browser: self.cookies_browser.as_deref(),
            concurrent_fragments: self.concurrent_fragments,
            use_aria2: self.use_aria2,
            output_template: Some(self.output_template.as_str()),
            rate_limit: self.rate_limit.as_deref(),
            socket_timeout: self.socket_timeout,
            retries: self.retries,
            fragment_retries: self.fragment_retries,
            playlist_subfolder: self.playlist_subfolder.as_deref(),
            playlist_subfolders: self.playlist_subfolders,
            embed_metadata: self.embed_metadata,
            write_info_json: self.write_info_json,
            allow_playlists: self.allow_playlists,
        }
    }
}

fn configure_style(ctx: &egui::Context, config: &Config) {
    let opacity = config.gui_opacity.clamp(0.70, 1.0);
    let theme = GuiTheme::from_config(&config.gui_theme);
    let (panel_fill, window_fill, extreme_bg, accent) = match theme {
        GuiTheme::Graphite => (
            rgba(28, 32, 36, opacity),
            rgba(31, 36, 41, opacity),
            rgba(23, 27, 31, opacity),
            BLUE,
        ),
        GuiTheme::Midnight => (
            rgba(18, 25, 34, opacity),
            rgba(23, 31, 43, opacity),
            rgba(12, 17, 25, opacity),
            Color32::from_rgb(72, 149, 239),
        ),
        GuiTheme::Oled => (
            rgba(10, 10, 12, opacity),
            rgba(14, 14, 16, opacity),
            rgba(5, 5, 7, opacity),
            Color32::from_rgb(76, 161, 255),
        ),
        GuiTheme::Warm => (
            rgba(36, 31, 30, opacity),
            rgba(42, 36, 34, opacity),
            rgba(24, 21, 20, opacity),
            Color32::from_rgb(214, 127, 74),
        ),
    };
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = panel_fill;
    visuals.window_fill = window_fill;
    visuals.extreme_bg_color = extreme_bg;
    visuals.selection.bg_fill = accent;
    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.hovered.bg_fill = accent.gamma_multiply(0.85);
    visuals.widgets.inactive.bg_fill = panel_fill;
    visuals.widgets.noninteractive.bg_fill = panel_fill;
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.visuals.widgets.noninteractive.corner_radius = 8.into();
    style.visuals.widgets.inactive.corner_radius = 8.into();
    style.visuals.widgets.active.corner_radius = 8.into();
    style.visuals.widgets.hovered.corner_radius = 8.into();
    ctx.set_style(style);
}

fn rgba(red: u8, green: u8, blue: u8, opacity: f32) -> Color32 {
    Color32::from_rgba_premultiplied(
        red,
        green,
        blue,
        (255.0 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

fn read_clipboard_text() -> Result<String, String> {
    Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .map_err(|error| format!("Could not read clipboard: {error}"))
}

fn fill_rounded_rect(
    rgba: &mut [u8],
    size: usize,
    origin: (usize, usize),
    dimensions: (usize, usize),
    radius: f32,
    color: [u8; 4],
) {
    let (x, y) = origin;
    let (width, height) = dimensions;
    let radius_sq = radius * radius;
    for py in y..(y + height).min(size) {
        for px in x..(x + width).min(size) {
            let dx = if px < x + radius as usize {
                (x + radius as usize) as f32 - px as f32
            } else if px >= x + width - radius as usize {
                px as f32 - (x + width - radius as usize - 1) as f32
            } else {
                0.0
            };
            let dy = if py < y + radius as usize {
                (y + radius as usize) as f32 - py as f32
            } else if py >= y + height - radius as usize {
                py as f32 - (y + height - radius as usize - 1) as f32
            } else {
                0.0
            };
            if dx == 0.0 && dy == 0.0 || dx * dx + dy * dy <= radius_sq {
                set_pixel(rgba, size, px, py, color);
            }
        }
    }
}

fn fill_circle(rgba: &mut [u8], size: usize, cx: f32, cy: f32, radius: f32, color: [u8; 4]) {
    let radius_sq = radius * radius;
    let min_x = (cx - radius).floor().max(0.0) as usize;
    let max_x = (cx + radius).ceil().min(size as f32 - 1.0) as usize;
    let min_y = (cy - radius).floor().max(0.0) as usize;
    let max_y = (cy + radius).ceil().min(size as f32 - 1.0) as usize;
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let dx = px as f32 - cx;
            let dy = py as f32 - cy;
            if dx * dx + dy * dy <= radius_sq {
                set_pixel(rgba, size, px, py, color);
            }
        }
    }
}

fn fill_triangle(
    rgba: &mut [u8],
    size: usize,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    color: [u8; 4],
) {
    let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as usize;
    let max_x = a.0.max(b.0).max(c.0).ceil().min(size as f32 - 1.0) as usize;
    let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as usize;
    let max_y = a.1.max(b.1).max(c.1).ceil().min(size as f32 - 1.0) as usize;

    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let point = (px as f32 + 0.5, py as f32 + 0.5);
            if point_in_triangle(point, a, b, c) {
                set_pixel(rgba, size, px, py, color);
            }
        }
    }
}

fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let area = |p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)| {
        (p1.0 * (p2.1 - p3.1) + p2.0 * (p3.1 - p1.1) + p3.0 * (p1.1 - p2.1)) / 2.0
    };
    let total = area(a, b, c).abs();
    let a1 = area(p, b, c).abs();
    let a2 = area(a, p, c).abs();
    let a3 = area(a, b, p).abs();
    (total - (a1 + a2 + a3)).abs() <= 0.5
}

fn set_pixel(rgba: &mut [u8], size: usize, x: usize, y: usize, color: [u8; 4]) {
    let index = (y * size + x) * 4;
    if index + 3 < rgba.len() {
        rgba[index..index + 4].copy_from_slice(&color);
    }
}

fn section_frame(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(PANEL)
        .inner_margin(egui::Margin::same(14))
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_gray(60)))
        .corner_radius(10.0)
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong().size(18.0));
            ui.add_space(10.0);
            add_contents(ui);
        });
}

fn diagnostic_row(ui: &mut egui::Ui, tool: &ToolDiagnostic, age_days: Option<i64>) {
    let status = if tool.path.is_some() { GREEN } else { AMBER };
    let mut line = format!(
        "{}: {}",
        tool.label,
        tool.path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "not found".into())
    );
    if let Some(version) = &tool.version {
        line.push_str(&format!(" · {version}"));
    }
    if let Some(age_days) = age_days {
        line.push_str(&format!(" · {age_days}d old"));
    }
    ui.label(RichText::new(line).color(status));
}

fn selectable_command(ui: &mut egui::Ui, command: &str) {
    ui.add(egui::Label::new(RichText::new(command).monospace()).selectable(true));
}

fn text_edit_with_context_menu(
    ui: &mut egui::Ui,
    value: &mut String,
    hint_text: &str,
    desired_width: f32,
) -> egui::Response {
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint_text)
            .desired_width(desired_width),
    );
    response.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(value.clone());
            ui.close_menu();
        }
        if ui.button("Cut").clicked() {
            ui.ctx().copy_text(value.clone());
            value.clear();
            ui.close_menu();
        }
        if ui.button("Paste").clicked() {
            let pasted = read_clipboard_text()
                .ok()
                .filter(|text| !text.is_empty())
                .map(|text| *value = text)
                .is_some();
            if !pasted {
                response.request_focus();
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
            ui.close_menu();
        }
        if ui.button("Clear").clicked() {
            value.clear();
            ui.close_menu();
        }
    });
    response
}

fn queue_row(ui: &mut egui::Ui, index: usize, item: &GuiQueueItem) {
    let expected_outer_height = queue_row_height(item, ui.available_width()) - 6.0;
    egui::Frame::group(ui.style())
        .fill(PANEL_ALT)
        .inner_margin(egui::Margin::same(12))
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_gray(54)))
        .corner_radius(10.0)
        .show(ui, |ui| {
            ui.set_min_height((expected_outer_height - 24.0).max(0.0));
            let full_width = ui.available_width();
            let number_width = 22.0;
            let thumb_width = if full_width < 760.0 { 72.0 } else { 84.0 };
            let status_width = if full_width < 760.0 { 92.0 } else { 112.0 };
            let progress_width = if full_width < 760.0 { 132.0 } else { 170.0 };
            let gap = 12.0;
            let info_width = (full_width
                - number_width
                - thumb_width
                - status_width
                - progress_width
                - gap * 4.0)
                .max(180.0);

            if full_width < 700.0 {
                ui.vertical(|ui| {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(number_width, 52.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.label(RichText::new(format!("{}", index + 1)).strong());
                            },
                        );

                        thumbnail_placeholder(ui, item);

                        ui.vertical(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(queue_item_title(item)).strong())
                                    .truncate(),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(item.url.as_ref())
                                        .size(12.0)
                                        .color(Color32::GRAY),
                                )
                                .truncate(),
                            );
                        });

                        status_badge(ui, item.state);
                    });

                    if let Some(error) = &item.error {
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(RichText::new(error).color(RED).size(12.5)).truncate(),
                        );
                    }

                    if matches!(
                        item.state,
                        DownloadState::Downloading | DownloadState::Finished
                    ) {
                        ui.add_space(6.0);
                        ui.add(
                            egui::ProgressBar::new(item.progress)
                                .desired_width(ui.available_width())
                                .show_percentage(),
                        );
                        if !item.progress_text.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&item.progress_text)
                                        .size(12.0)
                                        .color(Color32::GRAY),
                                )
                                .truncate(),
                            );
                        }
                    }
                });
                return;
            }

            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(number_width, 52.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new(format!("{}", index + 1)).strong());
                    },
                );

                thumbnail_placeholder(ui, item);

                ui.allocate_ui_with_layout(
                    egui::vec2(info_width, 64.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(queue_item_title(item)).strong())
                                .truncate(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(item.url.as_ref())
                                    .size(12.0)
                                    .color(Color32::GRAY),
                            )
                            .truncate(),
                        );
                        if let Some(error) = &item.error {
                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(RichText::new(error).color(RED).size(12.5))
                                    .truncate(),
                            );
                        }
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(status_width, 56.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| status_badge(ui, item.state),
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(progress_width, 56.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if matches!(
                            item.state,
                            DownloadState::Downloading | DownloadState::Finished
                        ) {
                            ui.add(
                                egui::ProgressBar::new(item.progress)
                                    .desired_width(progress_width - 8.0)
                                    .show_percentage(),
                            );
                            if !item.progress_text.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&item.progress_text)
                                            .size(12.0)
                                            .color(Color32::GRAY),
                                    )
                                    .truncate(),
                                );
                            }
                        } else {
                            ui.add_space(22.0);
                        }
                    },
                );
            });
        });
}

fn thumbnail_placeholder(ui: &mut egui::Ui, item: &GuiQueueItem) {
    let tint = match item.state {
        DownloadState::Downloading => BLUE,
        DownloadState::Finished => GREEN,
        DownloadState::Failed | DownloadState::Cancelled => RED,
        DownloadState::Waiting => AMBER,
    };
    egui::Frame::group(ui.style())
        .fill(Color32::from_black_alpha(24))
        .stroke(egui::Stroke::new(1.0_f32, tint))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.allocate_ui(egui::vec2(70.0, 44.0), |ui| {
                if let Some(texture) = &item.thumbnail_texture {
                    ui.centered_and_justified(|ui| {
                        ui.image((texture.id(), egui::vec2(66.0, 40.0)));
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("▶").color(tint).size(20.0));
                    });
                }
            });
        });
}

fn status_badge(ui: &mut egui::Ui, state: DownloadState) {
    let color = state_color(state);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .stroke(egui::Stroke::new(1.0_f32, color))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(state.label()).color(color).strong());
        });
}

fn active_ids(active_downloads: &HashMap<u64, oneshot::Sender<()>>) -> Vec<u64> {
    let mut ids: Vec<_> = active_downloads.keys().copied().collect();
    ids.sort_unstable();
    ids
}

fn state_color(state: DownloadState) -> Color32 {
    match state {
        DownloadState::Downloading => BLUE,
        DownloadState::Finished => GREEN,
        DownloadState::Failed | DownloadState::Cancelled => RED,
        DownloadState::Waiting => AMBER,
    }
}

fn friendly_error(url: &str, message: &str) -> String {
    if (url.contains("youtube.com") || url.contains("youtu.be"))
        && (message.contains("Video unavailable") || message.contains("HTTP Error 403"))
    {
        return format!(
            "{message}. If the video plays in your browser, select a Cookies browser in the left panel and retry."
        );
    }
    message.to_owned()
}

fn display_none(value: &str) -> &str {
    if value == "none" {
        "None"
    } else {
        value
    }
}

fn browser_impersonation(browser: &str) -> &str {
    match browser {
        "firefox" => "firefox",
        "edge" => "edge",
        "chrome" | "chromium" | "brave" | "vivaldi" => "chrome",
        _ => "any",
    }
}

fn short_url(url: &str) -> &str {
    url.split_once("://")
        .map(|(_, remainder)| remainder)
        .unwrap_or(url)
}

fn queue_item_title(item: &GuiQueueItem) -> Cow<'_, str> {
    item.title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(Cow::Borrowed)
        .unwrap_or_else(|| pretty_title_from_url(&item.url))
}

fn queue_row_height(item: &GuiQueueItem, full_width: f32) -> f32 {
    let narrow = full_width < 700.0;
    let mut height = if narrow { 92.0 } else { 94.0 };
    if item.error.is_some() {
        height += if narrow { 30.0 } else { 20.0 };
    }
    if matches!(
        item.state,
        DownloadState::Downloading | DownloadState::Finished
    ) {
        height += if narrow { 48.0 } else { 0.0 };
    }
    height + 6.0
}

fn pretty_title_from_url(url: &str) -> Cow<'_, str> {
    let trimmed = short_url(url);
    let slug = trimmed
        .split('/')
        .next_back()
        .unwrap_or(trimmed)
        .split('?')
        .next()
        .unwrap_or(trimmed)
        .split('#')
        .next()
        .unwrap_or(trimmed)
        .trim();

    if slug.is_empty() {
        return Cow::Borrowed(trimmed);
    }

    let display = slug
        .rsplit_once('_')
        .map(|(prefix, suffix)| {
            if suffix.chars().all(|ch| ch.is_ascii_hexdigit()) {
                prefix
            } else {
                slug
            }
        })
        .unwrap_or(slug)
        .replace(['-', '_', '+'], " ");

    if display.trim().is_empty() {
        Cow::Borrowed(trimmed)
    } else {
        Cow::Owned(display)
    }
}

fn playlist_folder_name(url: &str, title: Option<&str>) -> Option<String> {
    title
        .and_then(|value| sanitize_filename_component(value).ok())
        .or_else(|| {
            url.split_once("/playlists/")
                .and_then(|(_, value)| value.split(['?', '#', '/']).next())
                .and_then(|value| sanitize_filename_component(value).ok())
        })
}

fn download_thumbnail(
    client: &reqwest::blocking::Client,
    url: &str,
    cache_dir: Option<&std::path::Path>,
) -> Result<DecodedThumbnail, String> {
    if let Some(cache_path) = cache_dir.and_then(|dir| thumbnail_cache_path(dir, url)) {
        if cache_path.is_file() {
            match decode_thumbnail_from_path(&cache_path) {
                Ok(image) => return Ok(image),
                Err(_) => {
                    let _ = std::fs::remove_file(cache_path);
                }
            }
        }
    }

    let response = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("thumbnail request failed: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > THUMBNAIL_RESPONSE_LIMIT)
    {
        return Err("thumbnail response exceeds the 5 MiB limit".into());
    }
    let mut bytes = Vec::new();
    response
        .take(THUMBNAIL_RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("thumbnail read failed: {error}"))?;
    if bytes.len() as u64 > THUMBNAIL_RESPONSE_LIMIT {
        return Err("thumbnail response exceeds the 5 MiB limit".into());
    }
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("thumbnail format error: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("thumbnail decode failed: {error}"))?
        .thumbnail(160, 90)
        .to_rgba8();
    let width = image.width() as usize;
    let height = image.height() as usize;
    let rgba = image.into_raw();

    if let Some(cache_path) = cache_dir.and_then(|dir| thumbnail_cache_path(dir, url)) {
        let _ = persist_thumbnail_cache(&cache_path, &rgba, width, height);
    }

    Ok(DecodedThumbnail {
        rgba,
        width,
        height,
    })
}

fn thumbnail_cache_directory() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "crusty-dlp", "crusty-dlp").map(|dirs| {
        let dir = dirs.cache_dir().join("thumbnails");
        let _ = std::fs::create_dir_all(&dir);
        let _ = prune_thumbnail_disk_cache(&dir, THUMBNAIL_DISK_CACHE_LIMIT);
        dir
    })
}

fn prune_thumbnail_disk_cache(directory: &std::path::Path, limit: u64) -> std::io::Result<()> {
    let mut files = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (entry.path(), metadata.modified().ok(), metadata.len()))
        })
        .collect::<Vec<_>>();
    let mut total: u64 = files.iter().map(|(_, _, size)| size).sum();
    files.sort_by_key(|(_, modified, _)| *modified);
    for (path, _, size) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

fn thumbnail_cache_path(directory: &std::path::Path, url: &str) -> Option<PathBuf> {
    let hash = stable_u64_hash(url.as_bytes());
    Some(directory.join(format!("{hash:016x}.png")))
}

fn stable_u64_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn persist_thumbnail_cache(
    path: &std::path::Path,
    rgba: &[u8],
    width: usize,
    height: usize,
) -> Result<(), String> {
    image::save_buffer_with_format(
        path,
        rgba,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|error| format!("thumbnail cache write failed: {error}"))
}

fn decode_thumbnail_from_path(path: &std::path::Path) -> Result<DecodedThumbnail, String> {
    let image = ImageReader::open(path)
        .map_err(|error| format!("thumbnail cache open failed: {error}"))?
        .decode()
        .map_err(|error| format!("thumbnail cache decode failed: {error}"))?
        .to_rgba8();
    let width = image.width() as usize;
    let height = image.height() as usize;
    Ok(DecodedThumbnail {
        rgba: image.into_raw(),
        width,
        height,
    })
}

fn collect_runtime_diagnostics() -> RuntimeDiagnostics {
    let yt_dlp = collect_tool_diagnostic("yt-dlp", &["--version"]);
    let yt_dlp_age_days = yt_dlp.version.as_deref().and_then(yt_dlp_release_age_days);
    RuntimeDiagnostics {
        yt_dlp,
        ffmpeg: collect_tool_diagnostic("ffmpeg", &["-version"]),
        yt_dlp_ejs: collect_tool_diagnostic("yt-dlp-ejs", &["--version"]),
        js_runtime: first_js_runtime(),
        plugin_dir: resolved_plugin_directory(),
        yt_dlp_age_days,
        refreshed_at: Instant::now(),
    }
}

fn collect_tool_diagnostic(name: &'static str, args: &[&str]) -> ToolDiagnostic {
    let path = dependency_path(name);
    let version = path
        .as_deref()
        .and_then(|path| command_first_line(path, args))
        .map(|line| normalize_version_line(name, &line));
    ToolDiagnostic {
        label: name,
        path,
        version,
    }
}

fn first_js_runtime() -> Option<ToolDiagnostic> {
    ["deno", "node", "bun", "qjs", "quickjs"]
        .into_iter()
        .find_map(|name| {
            let diagnostic = collect_tool_diagnostic(name, &["--version"]);
            diagnostic.path.is_some().then_some(diagnostic)
        })
}

fn command_first_line(executable: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = StdCommand::new(executable).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_version_line(name: &str, line: &str) -> String {
    match name {
        "ffmpeg" => line
            .strip_prefix("ffmpeg version ")
            .unwrap_or(line)
            .to_owned(),
        _ => line.to_owned(),
    }
}

fn yt_dlp_release_age_days(version: &str) -> Option<i64> {
    let release_date = parse_yt_dlp_release_date(version)?;
    let current_date = current_utc_date()?;
    Some(days_from_civil(current_date) - days_from_civil(release_date))
}

fn parse_yt_dlp_release_date(version: &str) -> Option<(i32, u32, u32)> {
    let parts: Vec<_> = version.trim().split('.').take(3).collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

fn current_utc_date() -> Option<(i32, u32, u32)> {
    command_text("date", &["-u", "+%Y-%m-%d"])
        .or_else(|| {
            command_text(
                "powershell",
                &["-NoProfile", "-Command", "Get-Date -Format 'yyyy-MM-dd'"],
            )
        })
        .and_then(|value| {
            let parts: Vec<_> = value.trim().split('-').collect();
            if parts.len() != 3 {
                return None;
            }
            Some((
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            ))
        })
}

fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let output = StdCommand::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn days_from_civil((year, month, day): (i32, u32, u32)) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe - 719468) as i64
}
