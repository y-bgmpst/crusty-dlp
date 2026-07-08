use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind},
    terminal,
};

use crusty_dlp::app::{App, Panel};

pub fn handle_event(app: &mut App, event: Event) {
    match event {
        Event::Key(key) => handle_key(app, key),
        Event::Mouse(mouse) => handle_mouse(app, mouse),
        _ => {}
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    if app.show_install_prompt {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                app.message = "Install with: sudo pacman -S python-curl_cffi".into();
                app.show_install_prompt = false;
            }
            KeyCode::Char('n') | KeyCode::Esc => app.show_install_prompt = false,
            _ => {}
        }
        return;
    }
    if app.show_help {
        app.show_help = false;
        return;
    }
    if app.editing {
        match key.code {
            KeyCode::Esc => {
                app.editing = false;
                app.input.clear();
            }
            KeyCode::Enter => app.commit_edit(),
            KeyCode::Backspace => {
                app.input.pop();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.input.push(character)
            }
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('a') => {
            app.panel = Panel::Url;
            app.editing = true;
        }
        KeyCode::Char('s') => {
            app.panel = Panel::Search;
            app.input = app.search_query.clone();
            app.editing = true;
        }
        KeyCode::Char('d') => app.request_start(),
        KeyCode::Char('c') => app.cancel(),
        KeyCode::Char('b') => app.cycle_cookies_browser(),
        KeyCode::Char('p') => app.cycle_search_platform(),
        KeyCode::Char('o') => app.open_search(),
        KeyCode::Char('r') => app.toggle_aria2(),
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Tab => app.cycle_panel(),
        KeyCode::Enter | KeyCode::Char(' ') => app.edit_current_panel(),
        _ => {}
    }
}

fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    if app.show_help || app.show_install_prompt || app.editing {
        return;
    }

    match mouse.kind {
        MouseEventKind::ScrollDown => app.scroll_queue(1),
        MouseEventKind::ScrollUp => app.scroll_queue(-1),
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            if let Ok((width, height)) = terminal::size() {
                select_panel_from_click(app, mouse.column, mouse.row, width, height);
            }
        }
        _ => {}
    }
}

fn select_panel_from_click(app: &mut App, column: u16, row: u16, width: u16, height: u16) {
    const MIN_WIDTH: u16 = 70;
    const MIN_HEIGHT: u16 = 22;
    if width < MIN_WIDTH || height < MIN_HEIGHT {
        return;
    }

    let header_height = 3;
    let controls_height = 10;
    let top_controls_height = controls_height / 2;
    let queue_start = header_height + controls_height;

    if row < header_height {
        return;
    }

    if row < header_height + top_controls_height {
        let first = width * 38 / 100;
        let second = first + width * 30 / 100;
        app.panel = if column < first {
            Panel::Url
        } else if column < second {
            Panel::Search
        } else {
            Panel::Output
        };
        return;
    }

    if row < header_height + controls_height {
        let first = width * 34 / 100;
        let second = first + width * 33 / 100;
        app.panel = if column < first {
            Panel::Mode
        } else if column < second {
            Panel::Impersonation
        } else {
            Panel::Connections
        };
        return;
    }

    if row >= queue_start {
        app.panel = Panel::Queue;
    }
}
