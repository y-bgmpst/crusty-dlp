use std::{collections::VecDeque, path::PathBuf, time::Duration};

use crusty_dlp::{
    app::{validate_url, DownloadMode, DownloadState},
    config::Config,
    downloader::{
        available_impersonation_targets, dependency_path, DownloadEvent, DownloadOptions,
        Downloader,
    },
};
use eframe::egui::{self, Color32, RichText};
use tokio::sync::{mpsc, oneshot};

const BLUE: Color32 = Color32::from_rgb(47, 128, 237);
const GREEN: Color32 = Color32::from_rgb(72, 180, 90);
const RED: Color32 = Color32::from_rgb(235, 87, 87);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("crusty-dlp")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([920.0, 620.0]),
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

struct GuiApp {
    config: Config,
    config_path: Option<PathBuf>,
    input: String,
    mode: DownloadMode,
    queue: Vec<GuiQueueItem>,
    current: Option<usize>,
    auto_continue: bool,
    cancel_tx: Option<oneshot::Sender<()>>,
    event_tx: mpsc::UnboundedSender<DownloadEvent>,
    event_rx: mpsc::UnboundedReceiver<DownloadEvent>,
    impersonation_targets: Vec<String>,
    logs: VecDeque<String>,
    status: String,
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
            config,
            config_path,
            input: String::new(),
            mode,
            queue: Vec::new(),
            current: None,
            auto_continue: false,
            cancel_tx: None,
            event_tx,
            event_rx,
            impersonation_targets,
            logs: VecDeque::new(),
            status,
        }
    }

    fn save_config(&mut self) {
        if let Some(path) = &self.config_path {
            if let Err(error) = self.config.save(path) {
                self.status = error.to_string();
            }
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
            match validate_url(&url) {
                Ok(()) => {
                    self.log(format!("Added to queue: {url}"));
                    self.queue.push(GuiQueueItem {
                        url,
                        state: DownloadState::Waiting,
                        progress: 0.0,
                        progress_text: String::new(),
                        error: None,
                    });
                    added += 1;
                }
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

    fn start_queue(&mut self) {
        self.auto_continue = true;
        self.start_next();
    }

    fn start_next(&mut self) {
        if self.current.is_some() {
            return;
        }
        let Some(index) = self
            .queue
            .iter()
            .position(|item| item.state == DownloadState::Waiting)
        else {
            self.auto_continue = false;
            self.status = "Queue finished".into();
            return;
        };
        let Some(yt_dlp) = dependency_path("yt-dlp") else {
            self.fail(
                index,
                "yt-dlp was not found in PATH or beside the application",
            );
            return;
        };
        if self.mode.needs_ffmpeg() && dependency_path("ffmpeg").is_none() {
            self.fail(index, "ffmpeg is required for this download mode");
            return;
        }

        let url = self.queue[index].url.clone();
        let mode = self.mode.clone();
        let options = OwnedDownloadOptions::from_config(&self.config, &url);
        let downloader = Downloader::new(yt_dlp, self.config.output_dir.clone());
        let args = downloader.arguments(&url, &mode, options.borrow());
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let event_tx = self.event_tx.clone();
        self.cancel_tx = Some(cancel_tx);
        self.current = Some(index);
        self.queue[index].state = DownloadState::Downloading;
        self.status = "Downloading".into();
        self.log(format!("Starting download: {url}"));

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(downloader.run(args, cancel_rx, event_tx)),
                Err(error) => {
                    let _ = event_tx.send(DownloadEvent::Failed(format!(
                        "could not start async runtime: {error}"
                    )));
                }
            }
        });
    }

    fn fail(&mut self, index: usize, message: &str) {
        self.queue[index].state = DownloadState::Failed;
        self.queue[index].error = Some(message.to_owned());
        self.status = message.to_owned();
        self.log(format!("ERROR: {message}"));
        self.current = None;
        self.auto_continue = false;
    }

    fn cancel(&mut self) {
        self.auto_continue = false;
        if let Some(cancel) = self.cancel_tx.take() {
            let _ = cancel.send(());
            self.status = "Cancelling download…".into();
        }
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            let Some(index) = self.current else {
                continue;
            };
            match event {
                DownloadEvent::Progress { percent, text } => {
                    self.queue[index].progress = percent.unwrap_or_default() as f32 / 100.0;
                    self.queue[index].progress_text = text;
                }
                DownloadEvent::Finished => {
                    self.queue[index].state = DownloadState::Finished;
                    self.queue[index].progress = 1.0;
                    self.log(format!("Finished: {}", self.queue[index].url));
                    self.status = "Download finished".into();
                    self.current = None;
                    self.cancel_tx = None;
                    if self.auto_continue {
                        self.start_next();
                    }
                }
                DownloadEvent::Failed(message) => {
                    self.queue[index].state = DownloadState::Failed;
                    self.queue[index].error = Some(message.clone());
                    self.log(format!("ERROR: {message}"));
                    self.status = "Download failed".into();
                    self.current = None;
                    self.cancel_tx = None;
                    if self.auto_continue {
                        self.start_next();
                    }
                }
                DownloadEvent::Cancelled => {
                    self.queue[index].state = DownloadState::Cancelled;
                    self.log(format!("Cancelled: {}", self.queue[index].url));
                    self.status = "Download cancelled".into();
                    self.current = None;
                    self.cancel_tx = None;
                }
            }
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("URL");
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.input)
                .hint_text("https://example.com/video")
                .desired_width(f32::INFINITY),
        );
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.add_input();
        }
        if ui
            .add_sized(
                [ui.available_width(), 38.0],
                egui::Button::new(RichText::new("＋  Add to queue").strong()).fill(BLUE),
            )
            .clicked()
        {
            self.add_input();
        }

        ui.add_space(18.0);
        ui.heading("Download mode");
        let previous_mode = self.mode.clone();
        egui::ComboBox::from_id_salt("download-mode")
            .selected_text(self.mode.label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.mode, DownloadMode::Video, "Video — best quality");
                ui.selectable_value(&mut self.mode, DownloadMode::Audio, "Audio — best quality");
                ui.selectable_value(&mut self.mode, DownloadMode::Mp3, "MP3 — convert audio");
                ui.selectable_value(
                    &mut self.mode,
                    DownloadMode::Custom(self.config.custom_format.clone()),
                    "Custom format",
                );
            });
        if matches!(self.mode, DownloadMode::Custom(_)) {
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

        ui.add_space(18.0);
        ui.heading("Output folder");
        ui.horizontal(|ui| {
            let mut folder = self.config.output_dir.to_string_lossy().into_owned();
            ui.add_enabled(
                false,
                egui::TextEdit::singleline(&mut folder).desired_width(ui.available_width() - 80.0),
            );
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.config.output_dir)
                    .pick_folder()
                {
                    self.config.output_dir = path;
                    self.save_config();
                }
            }
        });

        ui.add_space(18.0);
        ui.heading("Cookies browser");
        egui::ComboBox::from_id_salt("cookies-browser")
            .selected_text(display_none(&self.config.cookies_browser))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for browser in [
                    "none", "firefox", "chrome", "chromium", "brave", "edge", "vivaldi", "safari",
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

        ui.add_space(18.0);
        ui.heading("Impersonation");
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
                for target in self.impersonation_targets.clone() {
                    if ui
                        .selectable_value(&mut self.config.impersonation, target.clone(), &target)
                        .changed()
                    {
                        self.save_config();
                    }
                }
            });

        ui.add_space(18.0);
        ui.heading("Connections");
        ui.horizontal(|ui| {
            if ui
                .add(egui::Slider::new(
                    &mut self.config.concurrent_fragments,
                    1..=16,
                ))
                .changed()
            {
                self.save_config();
            }
            if ui.checkbox(&mut self.config.use_aria2, "aria2").changed() {
                self.save_config();
            }
        });
        ui.small("More than 8 connections can increase throttling or HTTP 403 errors.");
    }

    fn queue_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(format!("Queue ({})", self.queue.len()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.current.is_none(), egui::Button::new("Clear completed"))
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
                    self.current = None;
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(390.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.queue.is_empty() {
                    ui.add_space(60.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Queue is empty").size(18.0));
                        ui.label("Add one or more URLs to begin.");
                    });
                }
                for (index, item) in self.queue.iter().enumerate() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}", index + 1)).strong());
                            ui.vertical(|ui| {
                                ui.label(RichText::new(short_url(&item.url)).strong());
                                ui.small(&item.url);
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(item.state.label())
                                            .color(state_color(item.state)),
                                    );
                                },
                            );
                        });
                        if item.state == DownloadState::Downloading {
                            ui.add(
                                egui::ProgressBar::new(item.progress)
                                    .desired_width(ui.available_width())
                                    .show_percentage(),
                            );
                            ui.small(&item.progress_text);
                        }
                        if let Some(error) = &item.error {
                            ui.label(RichText::new(error).color(RED));
                        }
                    });
                    ui.add_space(4.0);
                }
            });

        ui.add_space(10.0);
        if let Some(index) = self.current {
            ui.heading("Now downloading");
            ui.label(RichText::new(short_url(&self.queue[index].url)).strong());
            ui.add(
                egui::ProgressBar::new(self.queue[index].progress)
                    .desired_width(ui.available_width())
                    .show_percentage(),
            );
        }
        ui.horizontal(|ui| {
            let start = ui.add_sized(
                [ui.available_width() * 0.65, 40.0],
                egui::Button::new(RichText::new("▶  Start Queue").strong()).fill(BLUE),
            );
            if start.clicked() {
                self.start_queue();
            }
            if ui
                .add_sized([ui.available_width(), 40.0], egui::Button::new("■  Cancel"))
                .clicked()
            {
                self.cancel();
            }
        });

        ui.add_space(10.0);
        egui::CollapsingHeader::new("Log")
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(130.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.logs {
                            ui.monospace(line);
                        }
                    });
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
                ui.heading(RichText::new("🦀  crusty-dlp").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&self.status);
                });
            });
        });
        egui::SidePanel::left("settings")
            .resizable(false)
            .default_width(350.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_panel(ui));
            });
        egui::CentralPanel::default().show(ctx, |ui| self.queue_panel(ui));
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let yt_dlp = dependency_path("yt-dlp")
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not found".into());
                ui.label(format!("● yt-dlp: {yt_dlp}"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let waiting = self
                        .queue
                        .iter()
                        .filter(|item| item.state == DownloadState::Waiting)
                        .count();
                    ui.label(format!("{waiting} queued"));
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
}

impl OwnedDownloadOptions {
    fn from_config(config: &Config, url: &str) -> Self {
        let browser = (config.cookies_browser != "none").then(|| config.cookies_browser.clone());
        let mut impersonation =
            (config.impersonation != "none").then(|| config.impersonation.clone());
        if url.contains("spankbang.com") && impersonation.is_none() {
            impersonation = browser
                .as_deref()
                .map(browser_impersonation)
                .map(str::to_owned);
        }
        Self {
            impersonation,
            cookies_browser: browser,
            concurrent_fragments: config.concurrent_fragments.clamp(1, 16),
            use_aria2: config.use_aria2 && dependency_path("aria2c").is_some(),
        }
    }

    fn borrow(&self) -> DownloadOptions<'_> {
        DownloadOptions {
            impersonation: self.impersonation.as_deref(),
            cookies_browser: self.cookies_browser.as_deref(),
            concurrent_fragments: self.concurrent_fragments,
            use_aria2: self.use_aria2,
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
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    ctx.set_style(style);
}

fn state_color(state: DownloadState) -> Color32 {
    match state {
        DownloadState::Downloading => BLUE,
        DownloadState::Finished => GREEN,
        DownloadState::Failed | DownloadState::Cancelled => RED,
        DownloadState::Waiting => Color32::LIGHT_GRAY,
    }
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
