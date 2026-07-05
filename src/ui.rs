use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Panel};

const MIN_WIDTH: u16 = 70;
const MIN_HEIGHT: u16 = 22;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal too small\nNeed at least {MIN_WIDTH}×{MIN_HEIGHT}\nCurrent: {}×{}",
                area.width, area.height
            ))
            .alignment(Alignment::Center)
            .block(Block::bordered().title(" crusty-dlp ")),
            area,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Min(7),
            Constraint::Length(3),
        ])
        .split(area);
    render_header(frame, rows[0], app);
    render_controls(frame, rows[1], app);
    render_queue(frame, rows[2], app);
    render_status(frame, rows[3], app);
    if app.show_help {
        render_help(frame, area);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let suffix = if app.dry_run { "  [DRY RUN]" } else { "" };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "crusty-dlp",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" — safe yt-dlp queue"),
            Span::styled(suffix, Style::default().fg(Color::Yellow)),
        ]))
        .block(Block::bordered()),
        area,
    );
}

fn render_controls(frame: &mut Frame, area: Rect, app: &App) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(25),
            Constraint::Percentage(35),
        ])
        .split(area);
    let input_text = if app.editing && app.panel == Panel::Url {
        app.input.as_str()
    } else {
        "Press a or Enter to add URL(s)"
    };
    frame.render_widget(
        Paragraph::new(input_text)
            .wrap(Wrap { trim: false })
            .block(panel_block(" URL input ", app.panel == Panel::Url)),
        columns[0],
    );
    let mode_detail = if app.editing && app.panel == Panel::Mode {
        format!("Custom format\n{}", app.input)
    } else {
        match &app.mode {
            crate::app::DownloadMode::Custom(format) => format!("{}\n{}", app.mode.label(), format),
            _ => format!("{}\nEnter/Space cycles", app.mode.label()),
        }
    };
    frame.render_widget(
        Paragraph::new(mode_detail)
            .wrap(Wrap { trim: true })
            .block(panel_block(" Download type ", app.panel == Panel::Mode)),
        columns[1],
    );
    let output = if app.editing && app.panel == Panel::Output {
        app.input.as_str().into()
    } else {
        app.config.output_dir.to_string_lossy()
    };
    frame.render_widget(
        Paragraph::new(output)
            .wrap(Wrap { trim: false })
            .block(panel_block(" Output folder ", app.panel == Panel::Output)),
        columns[2],
    );
}

fn panel_block(title: &'static str, selected: bool) -> Block<'static> {
    let style = if selected {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(style)
}

fn render_queue(frame: &mut Frame, area: Rect, app: &App) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);
    let mut items = Vec::new();
    if let Some(item) = &app.current {
        items.push(ListItem::new(Line::from(vec![
            Span::styled("▶ Downloading  ", Style::default().fg(Color::Yellow)),
            Span::raw(&item.url),
        ])));
    }
    items.extend(app.queue.iter().map(|item| {
        let color = match item.state {
            crate::app::DownloadState::Finished => Color::Green,
            crate::app::DownloadState::Failed => Color::Red,
            crate::app::DownloadState::Cancelled => Color::Yellow,
            _ => Color::Gray,
        };
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("{:<11}  ", item.state.label()),
                Style::default().fg(color),
            ),
            Span::raw(&item.url),
        ]))
    }));
    if items.is_empty() {
        items.push(ListItem::new("Queue is empty"));
    }
    frame.render_widget(
        List::new(items).block(panel_block(" Queue ", app.panel == Panel::Queue)),
        vertical[0],
    );

    let ratio = app.progress.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0;
    let label = if app.progress_text.is_empty() {
        "Waiting"
    } else {
        &app.progress_text
    };
    frame.render_widget(
        Gauge::default()
            .block(Block::bordered().title(" Progress "))
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(label),
        vertical[1],
    );
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let debug = if app.debug {
        format!("  config={}", app.config_path.display())
    } else {
        String::new()
    };
    let text = format!(
        "{}{}   │   q quit  a add  d download  c cancel  Tab panels  ? help",
        app.message, debug
    );
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .block(Block::bordered().title(" Status ")),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(58, 16, area);
    frame.render_widget(Clear, popup);
    let text = "Keyboard\n\n  q       Quit safely\n  a       Add one or more URLs\n  d       Start/continue queue\n  c       Cancel active download\n  Tab     Switch panels\n  Enter   Edit/select current panel\n  Esc     Cancel editing\n  ?       Toggle this help\n\nPress any key to close";
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().title(" Help "))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
