mod app;
mod config;
mod downloader;
mod errors;
mod input;
mod ui;

use std::io;

use anyhow::{Context, Result};
use app::App;
use clap::Parser;
use config::Config;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use downloader::{dependency_path, DownloadEvent, Downloader};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

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
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = Config::path()?;
    let config = Config::load(&config_path)?;
    let mut app = App::new(config, config_path, cli.dry_run, cli.debug);
    for url in cli.urls {
        app.add_url(url);
    }

    // A CLI dry run is intentionally non-interactive so its output can be piped
    // or inspected without terminal control sequences.
    if cli.dry_run && !app.queue.is_empty() {
        let downloader = Downloader::new("yt-dlp".into(), app.config.output_dir.clone());
        for item in &app.queue {
            let args = downloader.arguments(&item.url, &app.mode);
            println!("{}", downloader.display_command(&args));
        }
        return Ok(());
    }

    enable_raw_mode().context("could not enable terminal raw mode")?;
    execute!(io::stdout(), EnterAlternateScreen).context("could not enter alternate screen")?;
    let _guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(Event::Key(key)) if key_tx.send(key).is_err() => break,
            Err(_) => break,
            _ => {}
        }
    });

    let (download_tx, mut download_rx) = mpsc::unbounded_channel();
    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;
        tokio::select! {
            Some(key) = key_rx.recv() => input::handle_key(&mut app, key),
            Some(message) = download_rx.recv() => app.handle_download_event(message),
        }

        if app.take_start_request() {
            start_next(&mut app, download_tx.clone()).await;
        }
        if app.should_quit {
            app.cancel();
            break;
        }
    }
    Ok(())
}

async fn start_next(app: &mut App, tx: mpsc::UnboundedSender<DownloadEvent>) {
    let Some(item) = app.next_queued() else {
        app.message = "Queue is empty".into();
        return;
    };

    let yt_dlp = match dependency_path("yt-dlp") {
        Some(path) => path,
        None => {
            app.fail_item(item, "yt-dlp was not found in PATH");
            return;
        }
    };
    if app.mode.needs_ffmpeg() && dependency_path("ffmpeg").is_none() {
        app.fail_item(item, "ffmpeg is required for this download type");
        return;
    }

    let downloader = Downloader::new(yt_dlp, app.config.output_dir.clone());
    let args = downloader.arguments(&item.url, &app.mode);
    if app.dry_run {
        app.finish_dry_run(item, downloader.display_command(&args));
        return;
    }

    let cancel = app.begin_download(item);
    tokio::spawn(async move {
        downloader.run(args, cancel, tx).await;
    });
}
