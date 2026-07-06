use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, Panel};

pub fn handle_key(app: &mut App, key: KeyEvent) {
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
        KeyCode::Char('d') => app.request_start(),
        KeyCode::Char('c') => app.cancel(),
        KeyCode::Char('b') => app.cycle_cookies_browser(),
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Tab => app.cycle_panel(),
        KeyCode::Enter | KeyCode::Char(' ') => app.edit_current_panel(),
        _ => {}
    }
}
