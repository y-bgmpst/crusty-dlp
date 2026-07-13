mod input;
mod ui;

use std::io;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use crusty_dlp::{
    app::App,
    config::Config,
    downloader::{available_impersonation_targets, dependency_path, DownloadEvent, Downloader},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

struct JobEvent {
    id: u64,
    event: DownloadEvent,
}

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Print yt-dlp arguments instead of starting downloads
    #[arg(long)]
    dry_run: bool,
    /// Show verbose diagnostic messages in the status panel
    #[arg(long)]
    debug: bool,
    /// Add URLs to the initial queue
    #[arg(value_name = "URL")]
    urls: Vec<String>,
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = Config::path()?;
    let config = Config::load(&config_path)?;
    let yt_dlp = dependency_path("yt-dlp");
    let impersonation_targets = yt_dlp
        .as_deref()
        .map(available_impersonation_targets)
        .unwrap_or_default();
    let mut app = App::new(
        config,
        config_path,
        cli.dry_run,
        cli.debug,
        impersonation_targets,
        dependency_path("aria2c").is_some(),
    );
    for url in cli.urls {
        app.add_url(url);
    }

    // A CLI dry run is intentionally non-interactive so its output can be piped
    // or inspected without terminal control sequences.
    if cli.dry_run && !app.queue.is_empty() {
        let downloader = Downloader::new("yt-dlp".into(), app.config.output_dir.clone());
        for item in &app.queue {
            let options = app
                .download_options(&item.url)
                .map_err(anyhow::Error::msg)?;
            let args = downloader
                .arguments(&item.url, &app.mode, options)
                .map_err(anyhow::Error::msg)?;
            println!("{}", downloader.display_command(&args));
        }
        return Ok(());
    }

    enable_raw_mode().context("could not enable terminal raw mode")?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)
        .context("could not enter alternate screen")?;
    let _guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if key_tx.send(event).is_err() {
                break;
            }
        }
    });

    let (download_tx, mut download_rx) = mpsc::channel::<JobEvent>(256);
    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;
        tokio::select! {
            Some(event) = key_rx.recv() => input::handle_event(&mut app, event),
            Some(message) = download_rx.recv() => app.handle_download_event(message.id, message.event),
        }

        if app.take_start_request() {
            while app.active_downloads() < app.max_active_downloads() {
                let started = start_next(&mut app, download_tx.clone()).await;
                if !started
                    && !app
                        .queue
                        .iter()
                        .any(|item| item.state == crusty_dlp::app::DownloadState::Waiting)
                {
                    break;
                }
            }
        }
        if app.should_quit {
            app.cancel();
            break;
        }
    }
    Ok(())
}

async fn start_next(app: &mut App, tx: mpsc::Sender<JobEvent>) -> bool {
    let Some(item) = app.next_queued() else {
        app.message = "Queue is empty".into();
        return false;
    };

    let yt_dlp = match dependency_path("yt-dlp") {
        Some(path) => path,
        None => {
            app.fail_item(item, "yt-dlp was not found in PATH");
            return false;
        }
    };
    if (app.mode.needs_ffmpeg() || app.config.embed_metadata) && dependency_path("ffmpeg").is_none()
    {
        app.fail_item(
            item,
            if app.config.embed_metadata && !app.mode.needs_ffmpeg() {
                "ffmpeg is required to embed metadata"
            } else {
                "ffmpeg is required for this download type"
            },
        );
        return false;
    }

    let downloader = Downloader::new(yt_dlp, app.config.output_dir.clone());
    if app.requires_impersonation(&item.url) && app.impersonation_targets.is_empty() {
        app.fail_item(
            item,
            "BoyfriendTV requires impersonation; install: sudo pacman -S python-curl_cffi",
        );
        return false;
    }
    let options = match app.download_options(&item.url) {
        Ok(options) => options,
        Err(error) => {
            app.fail_item(item, &error);
            return false;
        }
    };
    let args = match downloader.arguments(&item.url, &app.mode, options) {
        Ok(args) => args,
        Err(error) => {
            app.fail_item(item, &error);
            return false;
        }
    };
    if app.dry_run {
        app.finish_dry_run(item, downloader.display_command(&args));
        return true;
    }

    let (id, cancel) = app.begin_download(item);
    tokio::spawn(async move {
        let (job_tx, mut job_rx) = mpsc::channel(256);
        let forward = tokio::spawn(async move {
            while let Some(event) = job_rx.recv().await {
                let is_progress = matches!(event, DownloadEvent::Progress { .. });
                let message = JobEvent { id, event };
                match tx.try_send(message) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(message)) => {
                        if !is_progress {
                            let _ = tx.send(message).await;
                        }
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
        });
        downloader.run(args, cancel, job_tx).await;
        let _ = forward.await;
    });
    true
}
