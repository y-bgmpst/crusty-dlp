use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        mpsc::{self as std_mpsc, Receiver, TryRecvError},
        Arc, Mutex,
    },
    time::Duration,
};

use arboard::Clipboard;
use crusty_dlp::{
    app::{validate_url, DownloadMode, DownloadState},
    config::Config,
    downloader::{
        available_impersonation_targets, current_executable_path, dependency_path,
        expand_playlist_urls, resolve_network_tuning, resolved_plugin_directory,
        supports_playlist_expansion, validate_output_template, validate_rate_limit,
        validate_retry_count, validate_socket_timeout, DownloadEvent, DownloadOptions, Downloader,
        PlaylistEntry,
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
    title: Option<String>,
    url: String,
    thumbnail_url: Option<String>,
    thumbnail_image: Option<DecodedThumbnail>,
    thumbnail_texture: Option<egui::TextureHandle>,
    state: DownloadState,
    progress: f32,
    progress_text: String,
    error: Option<String>,
}

#[derive(Debug)]
struct JobEvent {
    index: usize,
    event: DownloadEvent,
}

struct QueueThumbnailEvent {
    index: usize,
    result: Result<DecodedThumbnail, String>,
}

struct ThumbnailRequest {
    index: usize,
    url: String,
}

struct DecodedThumbnail {
    rgba: Vec<u8>,
    width: usize,
    height: usize,
}

struct GuiApp {
    config: Config,
    config_path: Option<PathBuf>,
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
    queue_running: bool,
    active_downloads: HashMap<usize, oneshot::Sender<()>>,
    event_tx: mpsc::UnboundedSender<JobEvent>,
    event_rx: mpsc::UnboundedReceiver<JobEvent>,
    impersonation_targets: Vec<String>,
    logs: VecDeque<String>,
    status: String,
    folder_picker_rx: Option<Receiver<Result<Option<PathBuf>, String>>>,
    thumbnail_rx: Receiver<QueueThumbnailEvent>,
    thumbnail_request_tx: std_mpsc::Sender<ThumbnailRequest>,
}

impl GuiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config_path = Config::path().ok();
        let config = config_path
            .as_deref()
            .and_then(|path| Config::load(path).ok())
            .unwrap_or_default();
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
        let (thumbnail_tx, thumbnail_rx) = std_mpsc::channel();
        let (thumbnail_request_tx, thumbnail_request_rx) = std_mpsc::channel::<ThumbnailRequest>();
        let thumbnail_request_rx = Arc::new(Mutex::new(thumbnail_request_rx));
        for _ in 0..4 {
            let request_rx = Arc::clone(&thumbnail_request_rx);
            let result_tx = thumbnail_tx.clone();
            std::thread::spawn(move || loop {
                let request = match request_rx.lock() {
                    Ok(receiver) => receiver.recv(),
                    Err(_) => break,
                };
                let Ok(request) = request else {
                    break;
                };
                let result = download_thumbnail(&request.url);
                let _ = result_tx.send(QueueThumbnailEvent {
                    index: request.index,
                    result,
                });
            });
        }
        let status = if yt_dlp.is_some() {
            "Ready".to_owned()
        } else {
            "yt-dlp was not found in PATH or beside the application".to_owned()
        };
        Self {
            output_dir_text: config.output_dir.to_string_lossy().into_owned(),
            output_template_text: config.output_template.clone(),
            rate_limit_text: config.rate_limit.clone(),
            socket_timeout_text: config.socket_timeout.clone(),
            retries_text: config.retries.clone(),
            fragment_retries_text: config.fragment_retries.clone(),
            search_query: String::new(),
            config,
            config_path,
            input: String::new(),
            mode,
            queue: Vec::new(),
            queue_running: false,
            active_downloads: HashMap::new(),
            event_tx,
            event_rx,
            impersonation_targets,
            logs: VecDeque::new(),
            status,
            folder_picker_rx: None,
            thumbnail_rx,
            thumbnail_request_tx,
        }
    }

    fn save_config(&mut self) {
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
        for url in values {
            match self.expand_or_enqueue(&url) {
                Ok(count) => added += count,
                Err(error) => self.log(format!("Rejected URL: {error}")),
            }
        }
        if added > 0 {
            self.input.clear();
            self.status = format!("Added {added} item(s) to the queue");
        } else {
            self.status = "No valid URLs were added".into();
        }
    }

    fn paste_input_from_clipboard(&mut self) {
        match read_clipboard_text() {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    self.status = "Clipboard is empty".into();
                } else {
                    self.input = trimmed.to_owned();
                    self.status = "Pasted URL(s) from clipboard".into();
                }
            }
            Err(error) => self.status = error,
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
            match expand_playlist_urls(&yt_dlp, url) {
                Ok(Some(entries)) => {
                    self.log(format!(
                        "Expanded playlist into {} item(s): {url}",
                        entries.len()
                    ));
                    for entry in &entries {
                        self.enqueue_playlist_entry(entry);
                    }
                    self.status = format!("Expanded playlist into {} item(s)", entries.len());
                    return Ok(entries.len());
                }
                Ok(None) => {
                    return Err(format!(
                        "playlist expansion returned no entries for supported URL: {url}"
                    ));
                }
                Err(error) => {
                    self.log(format!("Playlist expansion failed: {error}"));
                    return Err(error);
                }
            }
        }
        self.enqueue_url(url);
        Ok(1)
    }

    fn enqueue_url(&mut self, url: &str) {
        self.enqueue_queue_item(None, url, None, true);
    }

    fn enqueue_playlist_entry(&mut self, entry: &PlaylistEntry) {
        self.enqueue_queue_item(
            entry.title.clone(),
            &entry.url,
            entry.thumbnail_url.clone(),
            false,
        );
    }

    fn enqueue_queue_item(
        &mut self,
        title: Option<String>,
        url: &str,
        thumbnail_url: Option<String>,
        log_add: bool,
    ) {
        if log_add {
            self.log(format!("Added to queue: {url}"));
        }
        let index = self.queue.len();
        self.queue.push(GuiQueueItem {
            title,
            url: url.to_owned(),
            thumbnail_url,
            thumbnail_image: None,
            thumbnail_texture: None,
            state: DownloadState::Waiting,
            progress: 0.0,
            progress_text: String::new(),
            error: None,
        });
        self.request_thumbnail(index);
    }

    fn request_thumbnail(&self, index: usize) {
        let Some(item) = self.queue.get(index) else {
            return;
        };
        let Some(thumbnail_url) = item.thumbnail_url.clone() else {
            return;
        };
        let _ = self.thumbnail_request_tx.send(ThumbnailRequest {
            index,
            url: thumbnail_url,
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
        if self.mode.needs_ffmpeg() && dependency_path("ffmpeg").is_none() {
            self.fail(index, "ffmpeg is required for this download mode");
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
        let mode = self.mode.clone();
        let options = match OwnedDownloadOptions::from_config(&self.config, &url) {
            Ok(options) => options,
            Err(message) => {
                self.fail(index, &message);
                return false;
            }
        };
        let downloader = Downloader::new(yt_dlp, self.config.output_dir.clone());
        let args = downloader.arguments(&url, &mode, options.borrow());
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let (download_tx, mut download_rx) = mpsc::unbounded_channel();
        let event_tx = self.event_tx.clone();

        self.active_downloads.insert(index, cancel_tx);
        self.queue[index].state = DownloadState::Downloading;
        self.queue[index].progress = 0.0;
        self.queue[index].progress_text.clear();
        self.queue[index].error = None;
        self.status = format!("Running {} active download(s)", self.active_downloads.len());
        self.log(format!("Starting download: {url}"));

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(async move {
                    let forward_tx = event_tx.clone();
                    let forward = tokio::spawn(async move {
                        while let Some(event) = download_rx.recv().await {
                            let _ = forward_tx.send(JobEvent { index, event });
                        }
                    });
                    downloader.run(args, cancel_rx, download_tx).await;
                    let _ = forward.await;
                }),
                Err(error) => {
                    let _ = event_tx.send(JobEvent {
                        index,
                        event: DownloadEvent::Failed(format!(
                            "could not start async runtime: {error}"
                        )),
                    });
                }
            }
        });
        true
    }

    fn fail(&mut self, index: usize, message: &str) {
        let message = friendly_error(&self.queue[index].url, message);
        self.queue[index].state = DownloadState::Failed;
        self.queue[index].error = Some(message.clone());
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
            if job.index >= self.queue.len() {
                continue;
            }
            match job.event {
                DownloadEvent::Progress { percent, text } => {
                    self.queue[job.index].progress = percent.unwrap_or_default() as f32 / 100.0;
                    self.queue[job.index].progress_text = text;
                }
                DownloadEvent::Finished => {
                    self.queue[job.index].state = DownloadState::Finished;
                    self.queue[job.index].progress = 1.0;
                    self.queue[job.index].progress_text = "done".into();
                    self.queue[job.index].error = None;
                    self.active_downloads.remove(&job.index);
                    self.log(format!("Finished: {}", self.queue[job.index].url));
                    self.status = "Download finished".into();
                }
                DownloadEvent::Failed(message) => {
                    self.queue[job.index].state = DownloadState::Failed;
                    self.queue[job.index].error = Some(message.clone());
                    self.active_downloads.remove(&job.index);
                    self.log(format!("ERROR: {message}"));
                    self.status = "Download failed".into();
                }
                DownloadEvent::Cancelled => {
                    self.queue[job.index].state = DownloadState::Cancelled;
                    self.queue[job.index].progress_text = "cancelled".into();
                    self.active_downloads.remove(&job.index);
                    self.log(format!("Cancelled: {}", self.queue[job.index].url));
                    self.status = "Download cancelled".into();
                }
            }
        }

        while let Ok(event) = self.thumbnail_rx.try_recv() {
            if let Some(item) = self.queue.get_mut(event.index) {
                if let Ok(image) = event.result {
                    item.thumbnail_image = Some(image);
                }
            }
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
        for (index, item) in self.queue.iter_mut().enumerate() {
            if item.thumbnail_texture.is_some() {
                continue;
            }
            let Some(image) = item.thumbnail_image.take() else {
                continue;
            };
            let texture = ctx.load_texture(
                format!("queue-thumb-{index}"),
                egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba),
                egui::TextureOptions::LINEAR,
            );
            item.thumbnail_texture = Some(texture);
        }
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
                self.paste_input_from_clipboard();
                response.request_focus();
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

    fn queue_panel(&mut self, ui: &mut egui::Ui, max_height: f32) {
        section_frame(ui, &format!("Queue ({})", self.queue.len()), |ui| {
            let full_width = ui.available_width();
            let number_width = 22.0;
            let thumb_width = 84.0;
            let status_width = 112.0;
            let progress_width = 170.0;
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
            egui::ScrollArea::vertical()
                .max_height(max_height.max(120.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.queue.is_empty() {
                        ui.add_space(50.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Queue is empty").size(18.0));
                            ui.label("Add one or more URLs to begin.");
                        });
                    }
                    for (index, item) in self.queue.iter().enumerate() {
                        queue_row(ui, index, item);
                        ui.add_space(6.0);
                    }
                });
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
                if let Some(index) = active_indices(&self.active_downloads).into_iter().next() {
                    if let Some(item) = self.queue.get(index) {
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
                                        RichText::new(&item.url).size(12.0).color(Color32::GRAY),
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
                    let waiting_indices: Vec<_> = self
                        .queue
                        .iter()
                        .enumerate()
                        .filter_map(|(index, item)| {
                            (item.state == DownloadState::Waiting).then_some(index)
                        })
                        .collect();
                    if !waiting_indices.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new("Waiting items").strong());
                        egui::ScrollArea::vertical()
                            .max_height(92.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for index in waiting_indices {
                                    if let Some(item) = self.queue.get(index) {
                                        ui.horizontal(|ui| {
                                            thumbnail_placeholder(ui, item);
                                            ui.vertical(|ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(queue_item_title(item))
                                                            .strong(),
                                                    )
                                                    .truncate(),
                                                );
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(&item.url)
                                                            .size(12.0)
                                                            .color(Color32::GRAY),
                                                    )
                                                    .truncate(),
                                                );
                                            });
                                        });
                                        ui.add_space(6.0);
                                    }
                                }
                            });
                    }
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
            let queue_max_height = (ui.available_height() * 0.42).clamp(180.0, 360.0);
            self.queue_panel(ui, queue_max_height);
            ui.add_space(10.0);
            self.queue_controls_panel(ui);
            ui.add_space(10.0);
            self.queue_log_panel(ui);
        });
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let executable = current_executable_path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not found".into());
                    let plugin_dir = resolved_plugin_directory()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not found".into());
                    ui.label(format!("exe: {executable}"));
                    ui.separator();
                    ui.label(format!("plugins: {plugin_dir}"));
                });
                ui.horizontal(|ui| {
                    let yt_dlp = dependency_path("yt-dlp")
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not found".into());
                    ui.label(format!("yt-dlp: {yt_dlp}"));
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
        ctx.request_repaint_after(Duration::from_millis(100));
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
    allow_playlists: bool,
}

impl OwnedDownloadOptions {
    fn from_config(config: &Config, url: &str) -> Result<Self, String> {
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
        .stroke(egui::Stroke::new(1.0, Color32::from_gray(60)))
        .corner_radius(10.0)
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong().size(18.0));
            ui.add_space(10.0);
            add_contents(ui);
        });
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
            if let Ok(text) = read_clipboard_text() {
                *value = text;
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
    egui::Frame::group(ui.style())
        .fill(PANEL_ALT)
        .inner_margin(egui::Margin::same(12))
        .stroke(egui::Stroke::new(1.0, Color32::from_gray(54)))
        .corner_radius(10.0)
        .show(ui, |ui| {
            let full_width = ui.available_width();
            let number_width = 22.0;
            let thumb_width = 84.0;
            let status_width = 112.0;
            let progress_width = 170.0;
            let gap = 12.0;
            let info_width = (full_width
                - number_width
                - thumb_width
                - status_width
                - progress_width
                - gap * 4.0)
                .max(180.0);

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
                                RichText::new(&item.url).size(12.0).color(Color32::GRAY),
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
        .stroke(egui::Stroke::new(1.0, tint))
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
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(state.label()).color(color).strong());
        });
}

fn active_indices(active_downloads: &HashMap<usize, oneshot::Sender<()>>) -> Vec<usize> {
    let mut indices: Vec<_> = active_downloads.keys().copied().collect();
    indices.sort_unstable();
    indices
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

fn download_thumbnail(url: &str) -> Result<DecodedThumbnail, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("thumbnail client error: {error}"))?;
    let bytes = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("thumbnail request failed: {error}"))?
        .bytes()
        .map_err(|error| format!("thumbnail read failed: {error}"))?;
    let image = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("thumbnail format error: {error}"))?
        .decode()
        .map_err(|error| format!("thumbnail decode failed: {error}"))?
        .thumbnail(160, 90)
        .to_rgba8();
    let width = image.width() as usize;
    let height = image.height() as usize;
    Ok(DecodedThumbnail {
        rgba: image.into_raw(),
        width,
        height,
    })
}
