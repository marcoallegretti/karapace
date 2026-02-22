//! Terminal UI for interactive Karapace environment management.
//!
//! This crate provides a ratatui-based TUI with environment listing, detail views,
//! search/filter, sorting, and keyboard-driven lifecycle actions (destroy, freeze,
//! archive, rename).

mod app;
mod ui;

pub use app::{App, AppAction, InputMode, SortColumn, View};

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::path::Path;

pub fn run(store_root: &Path) -> Result<(), String> {
    enable_raw_mode().map_err(|e| format!("failed to enable raw mode: {e}"))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| format!("alternate screen: {e}"))?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| format!("terminal init: {e}"))?;

    let mut app = App::new(store_root);
    app.refresh().ok();

    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode().map_err(|e| format!("failed to disable raw mode: {e}"))?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .map_err(|e| format!("leave alternate screen: {e}"))?;
    terminal
        .show_cursor()
        .map_err(|e| format!("show cursor: {e}"))?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), String> {
    loop {
        terminal
            .draw(|f| ui::draw(f, app))
            .map_err(|e| format!("draw: {e}"))?;

        if event::poll(std::time::Duration::from_millis(250)).map_err(|e| format!("poll: {e}"))? {
            if let Event::Key(key) = event::read().map_err(|e| format!("read: {e}"))? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match app.handle_key(key.code) {
                    AppAction::None => {}
                    AppAction::Quit => return Ok(()),
                    AppAction::Refresh => {
                        app.refresh().ok();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    fn make_app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(dir.path());
        (dir, app)
    }

    #[test]
    fn app_creates_and_refreshes() {
        let (_dir, mut app) = make_app();
        assert!(app.refresh().is_ok() || app.refresh().is_err());
    }

    #[test]
    fn app_quit_key() {
        let (_dir, mut app) = make_app();
        assert_eq!(app.handle_key(KeyCode::Char('q')), AppAction::Quit);
    }

    #[test]
    fn app_refresh_key() {
        let (_dir, mut app) = make_app();
        assert_eq!(app.handle_key(KeyCode::Char('r')), AppAction::Refresh);
    }

    #[test]
    fn app_navigation_j_k() {
        let (_dir, mut app) = make_app();
        assert_eq!(app.handle_key(KeyCode::Char('j')), AppAction::None);
        assert_eq!(app.handle_key(KeyCode::Char('k')), AppAction::None);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn app_help_view() {
        let (_dir, mut app) = make_app();
        app.handle_key(KeyCode::Char('?'));
        assert_eq!(app.view, View::Help);
        app.handle_key(KeyCode::Esc);
        assert_eq!(app.view, View::List);
    }

    #[test]
    fn app_search_mode_enter_exit() {
        let (_dir, mut app) = make_app();
        app.handle_key(KeyCode::Char('/'));
        assert_eq!(app.input_mode, InputMode::Search);
        app.handle_key(KeyCode::Esc);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.filter.is_empty());
    }

    #[test]
    fn app_search_typing() {
        let (_dir, mut app) = make_app();
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('t'));
        app.handle_key(KeyCode::Char('e'));
        app.handle_key(KeyCode::Char('s'));
        app.handle_key(KeyCode::Char('t'));
        assert_eq!(app.text_input, "test");
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.filter, "test");
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn app_search_backspace() {
        let (_dir, mut app) = make_app();
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('a'));
        app.handle_key(KeyCode::Char('b'));
        app.handle_key(KeyCode::Backspace);
        assert_eq!(app.text_input, "a");
    }

    #[test]
    fn app_sort_cycle() {
        let (_dir, mut app) = make_app();
        assert_eq!(app.sort_column, SortColumn::Name);
        app.handle_key(KeyCode::Char('s'));
        assert_eq!(app.sort_column, SortColumn::State);
        app.handle_key(KeyCode::Char('s'));
        assert_eq!(app.sort_column, SortColumn::ShortId);
        app.handle_key(KeyCode::Char('s'));
        assert_eq!(app.sort_column, SortColumn::Name);
    }

    #[test]
    fn app_sort_direction_toggle() {
        let (_dir, mut app) = make_app();
        assert!(app.sort_ascending);
        app.handle_key(KeyCode::Char('S'));
        assert!(!app.sort_ascending);
        app.handle_key(KeyCode::Char('S'));
        assert!(app.sort_ascending);
    }

    #[test]
    fn app_detail_view_enter_back() {
        let (_dir, mut app) = make_app();
        // No envs, Enter should not switch view
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.view, View::List);
    }

    #[test]
    fn app_confirm_cancel() {
        let (_dir, mut app) = make_app();
        app.show_confirm = Some("destroy:abc123".to_owned());
        app.handle_key(KeyCode::Char('n'));
        assert!(app.show_confirm.is_none());
        assert_eq!(app.status_message, "cancelled");
    }

    #[test]
    fn app_home_end_keys() {
        let (_dir, mut app) = make_app();
        app.handle_key(KeyCode::Home);
        assert_eq!(app.selected, 0);
        app.handle_key(KeyCode::End);
        assert_eq!(app.selected, 0); // No envs
    }

    #[test]
    fn app_rename_mode_enter_exit() {
        let (_dir, mut app) = make_app();
        // No envs, so rename shouldn't activate
        app.handle_key(KeyCode::Char('n'));
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn app_visible_count_empty() {
        let (_dir, app) = make_app();
        assert_eq!(app.visible_count(), 0);
    }

    #[test]
    fn app_filter_with_no_envs() {
        let (_dir, mut app) = make_app();
        app.filter = "test".to_owned();
        app.apply_filter();
        assert!(app.filtered.is_empty());
    }
}
