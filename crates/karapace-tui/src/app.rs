use crossterm::event::KeyCode;
use karapace_core::Engine;
use karapace_store::EnvMetadata;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Detail,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    ShortId,
    Name,
    State,
}

pub struct App {
    pub store_root: PathBuf,
    pub environments: Vec<EnvMetadata>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub view: View,
    pub input_mode: InputMode,
    pub text_input: String,
    pub input_cursor: usize,
    pub filter: String,
    pub sort_column: SortColumn,
    pub sort_ascending: bool,
    pub status_message: String,
    pub show_confirm: Option<String>,
}

impl App {
    pub fn new(store_root: &Path) -> Self {
        Self {
            store_root: store_root.to_path_buf(),
            environments: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            view: View::List,
            input_mode: InputMode::Normal,
            text_input: String::new(),
            input_cursor: 0,
            filter: String::new(),
            sort_column: SortColumn::Name,
            sort_ascending: true,
            status_message: String::new(),
            show_confirm: None,
        }
    }

    pub fn engine(&self) -> Engine {
        Engine::new(&self.store_root)
    }

    pub fn refresh(&mut self) -> Result<(), String> {
        match self.engine().list() {
            Ok(envs) => {
                self.environments = envs;
                self.apply_sort();
                self.apply_filter();
                self.status_message = format!("{} environment(s)", self.environments.len());
                Ok(())
            }
            Err(e) => {
                self.status_message = format!("error: {e}");
                Err(e.to_string())
            }
        }
    }

    pub fn apply_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered = (0..self.environments.len()).collect();
        } else {
            let needle = self.filter.to_lowercase();
            self.filtered = self
                .environments
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.short_id.to_lowercase().contains(&needle)
                        || e.env_id.to_lowercase().contains(&needle)
                        || e.name
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&needle)
                        || e.state.to_string().to_lowercase().contains(&needle)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.selected >= self.filtered.len() && !self.filtered.is_empty() {
            self.selected = self.filtered.len() - 1;
        } else if self.filtered.is_empty() {
            self.selected = 0;
        }
    }

    pub fn apply_sort(&mut self) {
        let asc = self.sort_ascending;
        match self.sort_column {
            SortColumn::ShortId => self.environments.sort_by(|a, b| {
                let ord = a.short_id.cmp(&b.short_id);
                if asc {
                    ord
                } else {
                    ord.reverse()
                }
            }),
            SortColumn::Name => self.environments.sort_by(|a, b| {
                let ord = a
                    .name
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.name.as_deref().unwrap_or(""));
                if asc {
                    ord
                } else {
                    ord.reverse()
                }
            }),
            SortColumn::State => self.environments.sort_by(|a, b| {
                let ord = a.state.to_string().cmp(&b.state.to_string());
                if asc {
                    ord
                } else {
                    ord.reverse()
                }
            }),
        }
    }

    pub fn selected_env(&self) -> Option<&EnvMetadata> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.environments.get(i))
    }

    pub fn visible_count(&self) -> usize {
        self.filtered.len()
    }

    pub fn handle_key(&mut self, key: KeyCode) -> AppAction {
        // Search input mode
        if self.input_mode == InputMode::Search {
            return self.handle_search_key(key);
        }

        // Rename input mode
        if self.input_mode == InputMode::Rename {
            return self.handle_rename_key(key);
        }

        // Confirmation dialog active
        if let Some(ref action) = self.show_confirm.clone() {
            if let KeyCode::Char('y' | 'Y') = key {
                self.execute_confirmed_action(action);
                self.show_confirm = None;
                return AppAction::Refresh;
            }
            self.show_confirm = None;
            "cancelled".clone_into(&mut self.status_message);
            return AppAction::None;
        }

        match self.view {
            View::Help => match key {
                KeyCode::Char('q') | KeyCode::Esc => {
                    self.view = View::List;
                    AppAction::None
                }
                _ => AppAction::None,
            },
            View::Detail => self.handle_detail_key(key),
            View::List => self.handle_list_key(key),
        }
    }

    fn handle_detail_key(&mut self, key: KeyCode) -> AppAction {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.view = View::List;
                AppAction::None
            }
            KeyCode::Char('d') => {
                self.prompt_destroy();
                AppAction::None
            }
            KeyCode::Char('f') => {
                self.action_freeze();
                AppAction::Refresh
            }
            KeyCode::Char('a') => {
                self.action_archive();
                AppAction::Refresh
            }
            KeyCode::Char('n') => {
                self.start_rename();
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_list_key(&mut self, key: KeyCode) -> AppAction {
        match key {
            KeyCode::Char('q') => AppAction::Quit,
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.filtered.is_empty() {
                    self.selected = (self.selected + 1).min(self.filtered.len() - 1);
                }
                AppAction::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                AppAction::None
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.selected = 0;
                AppAction::None
            }
            KeyCode::Char('G') | KeyCode::End => {
                if !self.filtered.is_empty() {
                    self.selected = self.filtered.len() - 1;
                }
                AppAction::None
            }
            KeyCode::Enter => {
                if self.selected_env().is_some() {
                    self.view = View::Detail;
                }
                AppAction::None
            }
            KeyCode::Char('r') => AppAction::Refresh,
            KeyCode::Char('d') => {
                self.prompt_destroy();
                AppAction::None
            }
            KeyCode::Char('f') => {
                self.action_freeze();
                AppAction::Refresh
            }
            KeyCode::Char('a') => {
                self.action_archive();
                AppAction::Refresh
            }
            KeyCode::Char('n') => {
                self.start_rename();
                AppAction::None
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.text_input.clear();
                self.input_cursor = 0;
                "search: ".clone_into(&mut self.status_message);
                AppAction::None
            }
            KeyCode::Char('s') => {
                self.cycle_sort();
                AppAction::None
            }
            KeyCode::Char('S') => {
                self.sort_ascending = !self.sort_ascending;
                self.apply_sort();
                self.apply_filter();
                AppAction::None
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_search_key(&mut self, key: KeyCode) -> AppAction {
        match key {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.filter.clear();
                self.apply_filter();
                self.status_message = format!("{} environment(s)", self.environments.len());
                AppAction::None
            }
            KeyCode::Enter => {
                self.input_mode = InputMode::Normal;
                self.filter = self.text_input.clone();
                self.apply_filter();
                self.status_message = if self.filter.is_empty() {
                    format!("{} environment(s)", self.environments.len())
                } else {
                    format!(
                        "filter '{}': {} match(es)",
                        self.filter,
                        self.filtered.len()
                    )
                };
                AppAction::None
            }
            KeyCode::Char(c) => {
                self.text_input.insert(self.input_cursor, c);
                self.input_cursor += 1;
                self.filter = self.text_input.clone();
                self.apply_filter();
                self.status_message = format!("search: {}", self.text_input);
                AppAction::None
            }
            KeyCode::Backspace => {
                if self.input_cursor > 0 {
                    self.input_cursor -= 1;
                    self.text_input.remove(self.input_cursor);
                    self.filter = self.text_input.clone();
                    self.apply_filter();
                }
                self.status_message = format!("search: {}", self.text_input);
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_rename_key(&mut self, key: KeyCode) -> AppAction {
        match key {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                "rename cancelled".clone_into(&mut self.status_message);
                AppAction::None
            }
            KeyCode::Enter => {
                self.input_mode = InputMode::Normal;
                let new_name = self.text_input.clone();
                if let Some(env) = self.selected_env() {
                    let env_id = env.env_id.clone();
                    match self.engine().rename(&env_id, &new_name) {
                        Ok(()) => {
                            self.status_message = format!("renamed to '{new_name}'");
                        }
                        Err(e) => {
                            self.status_message = format!("rename failed: {e}");
                        }
                    }
                }
                AppAction::Refresh
            }
            KeyCode::Char(c) => {
                self.text_input.insert(self.input_cursor, c);
                self.input_cursor += 1;
                self.status_message = format!("rename: {}", self.text_input);
                AppAction::None
            }
            KeyCode::Backspace => {
                if self.input_cursor > 0 {
                    self.input_cursor -= 1;
                    self.text_input.remove(self.input_cursor);
                }
                self.status_message = format!("rename: {}", self.text_input);
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn prompt_destroy(&mut self) {
        if let Some(env) = self.selected_env() {
            let label = env.name.clone().unwrap_or_else(|| env.short_id.to_string());
            self.show_confirm = Some(format!("destroy:{}", env.env_id));
            self.status_message = format!("destroy '{label}'? (y/n)");
        }
    }

    fn action_freeze(&mut self) {
        if let Some(env) = self.selected_env() {
            let env_id = env.env_id.to_string();
            let label = env.name.clone().unwrap_or_else(|| env.short_id.to_string());
            match self.engine().freeze(&env_id) {
                Ok(()) => self.status_message = format!("frozen '{label}'"),
                Err(e) => self.status_message = format!("freeze failed: {e}"),
            }
        }
    }

    fn action_archive(&mut self) {
        if let Some(env) = self.selected_env() {
            let env_id = env.env_id.to_string();
            let label = env.name.clone().unwrap_or_else(|| env.short_id.to_string());
            match self.engine().archive(&env_id) {
                Ok(()) => self.status_message = format!("archived '{label}'"),
                Err(e) => self.status_message = format!("archive failed: {e}"),
            }
        }
    }

    fn start_rename(&mut self) {
        if self.selected_env().is_some() {
            self.input_mode = InputMode::Rename;
            self.text_input.clear();
            self.input_cursor = 0;
            "rename: ".clone_into(&mut self.status_message);
        }
    }

    fn cycle_sort(&mut self) {
        self.sort_column = match self.sort_column {
            SortColumn::ShortId => SortColumn::Name,
            SortColumn::Name => SortColumn::State,
            SortColumn::State => SortColumn::ShortId,
        };
        self.apply_sort();
        self.apply_filter();
        self.status_message = format!(
            "sort: {:?} {}",
            self.sort_column,
            if self.sort_ascending { "↑" } else { "↓" }
        );
    }

    fn execute_confirmed_action(&mut self, action: &str) {
        if let Some(env_id) = action.strip_prefix("destroy:") {
            match self.engine().destroy(env_id) {
                Ok(()) => {
                    self.status_message = format!("destroyed {}", &env_id[..12.min(env_id.len())]);
                }
                Err(e) => {
                    self.status_message = format!("destroy failed: {e}");
                }
            }
        }
    }
}
