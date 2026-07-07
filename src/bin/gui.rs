use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::mpsc::{self as std_mpsc, Receiver, TryRecvError},
    time::Duration,
};

use crusty_dlp::{
    app::{validate_url, DownloadMode, DownloadState},
    config::Config,
    downloader::{
        available_impersonation_targets, dependency_path, expand_playlist_urls,
        supports_playlist_expansion, validate_output_template, validate_rate_limit, DownloadEvent,
        DownloadOptions, Downloader,
    },
    search::{open_platform_search, SearchPlatform},
};
use eframe::egui::{self, Color32, RichText};
use tokio::sync::{mpsc, oneshot};

const BLUE: Color32 = Color32::from_rgb(47, 128, 237);
const GREEN: Color32 = Color32::from_rgb(72, 180, 90);
const RED: Color32 = Color32::from_rgb(235, 87, 87);
const AMBER: Color32 = Color32::from_rgb(217, 154, 34);
const PANEL: Color32 = Color32::from_rgb(31, 36, 41);
const PANEL_ALT: Color32 = Color32::from_rgb(36, 42, 48);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("crusty-dlp")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native(
        "crusty-dlp",
        options,
        Box::new(|cc| Ok(Box::new(GuiApp::new(cc)))),
    )
}

#[derive(Debug)]
struct GuiQueueItem {
    url: String,
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

struct GuiApp {
    config: Config,
    config_path: Option<PathBuf>,
    input: String,
    search_query: String,
    output_dir_text: String,
    output_template_text: String,
    rate_limit_text: String,
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
}

impl GuiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let config_path = Config::path().ok();
        let config = config_path
            .as_deref()
            .and_then(|path| Config::load(path).ok())
            .unwrap_or_default();
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
        let status = if yt_dlp.is_some() {
            "Ready".to_owned()
        } else {
            "yt-dlp was not found in PATH or beside the application".to_owned()
        };
        Self {
            output_dir_text: config.output_dir.to_string_lossy().into_owned(),
            output_template_text: config.output_template.clone(),
            rate_limit_text: config.rate_limit.clone(),
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
        }
    }

    fn save_config(&mut self) {
        if let Some(path) = &self.config_path {
            if let Err(error) = self.config.save(path) {
                self.status = error.to_string();
            }
        }
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

    fn expand_or_enqueue(&mut self, url: &str) -> Result<usize, String> {
        validate_url(url).map_err(|error| error.to_string())?;
        if self.config.allow_playlists && supports_playlist_expansion(url) {
            let yt_dlp = dependency_path("yt-dlp").ok_or_else(|| {
                "yt-dlp was not found in PATH or beside the application".to_owned()
            })?;
            match expand_playlist_urls(&yt_dlp, url) {
                Ok(Some(entries)) => {
                    for entry in &entries {
                        self.enqueue_url(entry);
                    }
                    self.log(format!(
                        "Expanded playlist into {} item(s): {url}",
                        entries.len()
                    ));
                    return Ok(entries.len());
                }
                Ok(None) => {}
                Err(error) => return Err(error),
            }
        }
        self.enqueue_url(url);
        Ok(1)
    }

    fn enqueue_url(&mut self, url: &str) {
        self.log(format!("Added to queue: {url}"));
        self.queue.push(GuiQueueItem {
            url: url.to_owned(),
            state: DownloadState::Waiting,
            progress: 0.0,
            progress_text: String::new(),
            error: None,
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
        if !self.apply_output_dir() || !self.apply_output_template() || !self.apply_rate_limit() {
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
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .hint_text("https://example.com/video")
                    .desired_width(f32::INFINITY),
            );
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
        section_frame(ui, "Search in browser", |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search query")
                    .desired_width(f32::INFINITY),
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
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.config.custom_format)
                            .hint_text("yt-dlp format selector")
                            .desired_width(f32::INFINITY),
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
            let folder_response = ui.add(
                egui::TextEdit::singleline(&mut self.output_dir_text)
                    .hint_text("/home/user/Downloads")
                    .desired_width(f32::INFINITY),
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
        let template_response = ui.add(
            egui::TextEdit::singleline(&mut self.output_template_text)
                .hint_text("%(title)s [%(id)s].%(ext)s")
                .desired_width(f32::INFINITY),
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
        ui.small("Disabled by default. Supported playlist URLs are expanded into queue entries before downloading.");

        ui.horizontal(|ui| {
            ui.label("Speed limit");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.rate_limit_text)
                    .hint_text("blank = unlimited, e.g. 5M")
                    .desired_width(170.0),
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.apply_rate_limit();
            }
            if ui.button("Apply").clicked() {
                self.apply_rate_limit();
            }
        });
        ui.small("The limit applies per active download process.");
    }

    fn queue_panel(&mut self, ui: &mut egui::Ui) {
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
                .max_height(360.0)
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

        ui.add_space(10.0);
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
                                        RichText::new(short_url(&item.url)).strong().size(18.0),
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

        ui.add_space(10.0);
        section_frame(ui, "Log", |ui| {
            egui::ScrollArea::vertical()
                .max_height(130.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.logs {
                        ui.monospace(line);
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
        self.process_events();
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("crusty-dlp").strong());
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
        egui::CentralPanel::default().show(ctx, |ui| self.queue_panel(ui));
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let yt_dlp = dependency_path("yt-dlp")
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not found".into());
                ui.label(format!("yt-dlp: {yt_dlp}"));
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
    allow_playlists: bool,
}

impl OwnedDownloadOptions {
    fn from_config(config: &Config, url: &str) -> Result<Self, String> {
        validate_output_template(&config.output_template)?;
        validate_rate_limit(&config.rate_limit)?;

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
            allow_playlists: self.allow_playlists,
        }
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(28, 32, 36);
    visuals.window_fill = Color32::from_rgb(31, 36, 41);
    visuals.extreme_bg_color = Color32::from_rgb(23, 27, 31);
    visuals.selection.bg_fill = BLUE;
    visuals.widgets.active.bg_fill = BLUE;
    visuals.widgets.inactive.bg_fill = PANEL_ALT;
    visuals.widgets.noninteractive.bg_fill = PANEL_ALT;
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
                            egui::Label::new(RichText::new(short_url(&item.url)).strong())
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
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("▶").color(tint).size(20.0));
                });
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
