//! Centered ratatui form: directory → name → profile cards → cancel/save.

use crate::apply;
use crate::complete;
use crate::config::{self, Profile};
use crate::popup;
use crate::theme::Theme;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io::{self, stdout, Stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Dir,
    Name,
    Profile,
    Cancel,
    Save,
}

pub fn run() -> i32 {
    popup::run(run_inner)
}

fn run_inner(agent_starts: &mut Vec<Vec<String>>) -> Result<i32, String> {
    let cfg = config::load().map_err(|e| e.message)?;
    let mut app = App::new(cfg.profiles, Theme::load());

    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;
    let result = app_loop(&mut terminal, &mut app, agent_starts);
    let _ = disable_raw_mode();
    let mut out = io::stdout();
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen);
    result
}

fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    agent_starts: &mut Vec<Vec<String>>,
) -> Result<i32, String> {
    loop {
        terminal.draw(|f| app.draw(f)).map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(200)).map_err(io_err)? {
            match event::read().map_err(io_err)? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(code) = app.on_key(key.code, key.modifiers) {
                        return Ok(code);
                    }
                }
                Event::Mouse(mouse) => {
                    if let Some(code) = app.on_mouse(mouse) {
                        return Ok(code);
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        app.poll_job(agent_starts);
        if app.done {
            return Ok(app.exit_code);
        }
    }
}

fn io_err(e: io::Error) -> String {
    e.to_string()
}

struct DirectoryPicker {
    path: PathBuf,
    children: Vec<PathBuf>,
    selected: usize,
    error: String,
}

impl DirectoryPicker {
    fn read(path: PathBuf) -> Result<Self, String> {
        if path.to_str().is_none() {
            return Err("Directory path is not UTF-8.".into());
        }
        let entries = std::fs::read_dir(&path)
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        let mut children = Vec::new();
        for entry in entries.flatten() {
            let child = entry.path();
            if child.to_str().is_none() {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() || kind.is_symlink() && child.is_dir() {
                children.push(child);
            }
        }
        children.sort();
        Ok(Self {
            path,
            children,
            selected: 0,
            error: String::new(),
        })
    }

    fn selected_path(&self) -> &Path {
        if self.selected == 0 {
            &self.path
        } else {
            &self.children[self.selected - 1]
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.children.len() as i32 + 1;
        self.selected = (self.selected as i32 + delta).rem_euclid(count) as usize;
    }

    fn navigate(&mut self, path: PathBuf) {
        match Self::read(path) {
            Ok(next) => *self = next,
            Err(error) => self.error = error,
        }
    }

    fn draw(&self, f: &mut Frame, theme: &Theme) {
        let area = f.area();
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .title(" Directory picker ")
            .style(theme.base())
            .border_style(theme.input_border(true));
        let inner = pad(block.inner(area), 1, 1);
        f.render_widget(block, area);
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
        f.render_widget(
            Paragraph::new(self.path.to_string_lossy()).style(theme.input(true)),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new(format!(
                "{} folders · ./ chooses this directory",
                self.children.len()
            ))
            .style(theme.muted()),
            chunks[1],
        );
        let start = self
            .selected
            .saturating_add(1)
            .saturating_sub(chunks[2].height as usize);
        let lines: Vec<_> = (start..self.children.len() + 1)
            .take(chunks[2].height as usize)
            .map(|index| {
                let label = if index == 0 {
                    "  ./ [Use this directory]".to_owned()
                } else {
                    format!(
                        "  {}/",
                        self.children[index - 1]
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                    )
                };
                Line::from(Span::styled(label, theme.suggest(index == self.selected)))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), chunks[2]);
        f.render_widget(
            Paragraph::new(self.error.as_str()).style(theme.error()),
            chunks[3],
        );
        f.render_widget(
            Paragraph::new("↑/↓ select · → open · ←/backspace parent · enter choose · esc back")
                .style(theme.muted()),
            chunks[4],
        );
    }
}

struct App {
    theme: Theme,
    profiles: Vec<Profile>,
    dir: String,
    name: String,
    directory_history: Vec<String>,
    project_directories: Vec<String>,
    suggestions: Vec<String>,
    suggest_idx: usize,
    picker: Option<DirectoryPicker>,
    dir_committed: bool,
    name_committed: bool,
    name_auto: bool,
    profile_idx: usize,
    focus: Focus,
    error: String,
    job: Option<apply::SaveJob>,
    done: bool,
    exit_code: i32,
    // hit boxes from last draw
    dir_box: Rect,
    browse_box: Rect,
    name_box: Rect,
    card_boxes: Vec<Rect>,
    cancel_box: Rect,
    save_box: Rect,
    card_cols: usize,
}

impl App {
    fn new(profiles: Vec<Profile>, theme: Theme) -> Self {
        let mut app = Self::with_history(profiles, theme, complete::directory_history());
        app.project_directories = complete::project_directories();
        app.refresh_suggestions();
        app
    }

    fn with_history(profiles: Vec<Profile>, theme: Theme, history: Vec<String>) -> Self {
        Self {
            theme,
            profiles,
            dir: String::new(),
            name: String::new(),
            directory_history: history,
            project_directories: Vec::new(),
            suggestions: Vec::new(),
            suggest_idx: 0,
            picker: None,
            dir_committed: false,
            name_committed: false,
            name_auto: true,
            profile_idx: 0,
            focus: Focus::Dir,
            error: String::new(),
            job: None,
            done: false,
            exit_code: 0,
            dir_box: Rect::default(),
            browse_box: Rect::default(),
            name_box: Rect::default(),
            card_boxes: Vec::new(),
            cancel_box: Rect::default(),
            save_box: Rect::default(),
            card_cols: 1,
        }
    }

    fn refresh_suggestions(&mut self) {
        self.suggestions = complete::suggestions(
            &self.dir,
            &self.directory_history,
            &self.project_directories,
        );
        if self.suggest_idx >= self.suggestions.len() {
            self.suggest_idx = 0;
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<i32> {
        if self.job.is_some() {
            return None;
        }
        if self.picker.is_some() {
            self.key_picker(code, mods);
            return None;
        }
        if code == KeyCode::Esc {
            return Some(0);
        }
        match self.focus {
            Focus::Dir => self.key_dir(code, mods),
            Focus::Name => self.key_name(code, mods),
            Focus::Profile => self.key_profile(code),
            Focus::Cancel => self.key_cancel(code, mods),
            Focus::Save => self.key_save(code, mods),
        }
    }

    fn key_dir(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<i32> {
        match code {
            KeyCode::Char('o') if mods.contains(KeyModifiers::CONTROL) => self.open_picker(),
            KeyCode::Char('j') if mods.contains(KeyModifiers::CONTROL) => self.move_suggest(1),
            KeyCode::Char('k') if mods.contains(KeyModifiers::CONTROL) => self.move_suggest(-1),
            KeyCode::Char('n') if mods.contains(KeyModifiers::CONTROL) => self.move_suggest(1),
            KeyCode::Char('p') if mods.contains(KeyModifiers::CONTROL) => self.move_suggest(-1),
            KeyCode::Char(c) if !mods.contains(KeyModifiers::CONTROL) => {
                self.dir.push(c);
                self.dir_committed = false;
                self.name_committed = false;
                self.error.clear();
                self.suggest_idx = 0;
                self.refresh_suggestions();
            }
            KeyCode::Backspace => {
                self.dir.pop();
                self.dir_committed = false;
                self.name_committed = false;
                self.error.clear();
                self.suggest_idx = 0;
                self.refresh_suggestions();
            }
            KeyCode::Down => self.move_suggest(1),
            KeyCode::Up => self.move_suggest(-1),
            KeyCode::Tab => {
                if let Some(s) = self.suggestions.get(self.suggest_idx).cloned() {
                    self.dir = complete::accept(&s);
                    self.suggest_idx = 0;
                    self.refresh_suggestions();
                }
            }
            KeyCode::Enter => {
                if let Some(s) = self.suggestions.get(self.suggest_idx).cloned() {
                    self.apply_dir(&complete::accept(&s));
                } else {
                    self.commit_dir();
                }
            }
            _ => {}
        }
        None
    }

    fn open_picker(&mut self) {
        let selected = if !self.dir.is_empty() && !complete::is_existing_dir(&self.dir) {
            self.suggestions
                .get(self.suggest_idx)
                .map(|path| complete::expand_user(path))
        } else {
            None
        };
        let path = selected.unwrap_or_else(|| complete::browse_root(&self.dir));
        match DirectoryPicker::read(path) {
            Ok(picker) => self.picker = Some(picker),
            Err(error) => self.error = error,
        }
    }

    fn key_picker(&mut self, code: KeyCode, mods: KeyModifiers) {
        match code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter | KeyCode::Tab => {
                let picker = self.picker.take().unwrap();
                self.apply_dir(picker.selected_path().to_str().unwrap());
            }
            KeyCode::Up => self.picker.as_mut().unwrap().move_selection(-1),
            KeyCode::Down => self.picker.as_mut().unwrap().move_selection(1),
            KeyCode::Char('j') if mods.contains(KeyModifiers::CONTROL) => {
                self.picker.as_mut().unwrap().move_selection(1)
            }
            KeyCode::Char('k') if mods.contains(KeyModifiers::CONTROL) => {
                self.picker.as_mut().unwrap().move_selection(-1)
            }
            KeyCode::Right => {
                let picker = self.picker.as_mut().unwrap();
                if picker.selected > 0 {
                    picker.navigate(picker.selected_path().to_path_buf());
                }
            }
            KeyCode::Left | KeyCode::Backspace => {
                let picker = self.picker.as_mut().unwrap();
                if let Some(parent) = picker.path.parent() {
                    picker.navigate(parent.to_path_buf());
                }
            }
            KeyCode::Home => self.picker.as_mut().unwrap().selected = 0,
            KeyCode::End => {
                let picker = self.picker.as_mut().unwrap();
                picker.selected = picker.children.len();
            }
            _ => {}
        }
    }

    fn move_suggest(&mut self, delta: i32) {
        if self.suggestions.is_empty() {
            if delta > 0 && self.dir_committed {
                self.focus = Focus::Name;
            }
            return;
        }
        let n = self.suggestions.len() as i32;
        let next = (self.suggest_idx as i32 + delta).rem_euclid(n);
        self.suggest_idx = next as usize;
    }

    fn key_name(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<i32> {
        match code {
            KeyCode::Char(c) if !mods.contains(KeyModifiers::CONTROL) => {
                self.name.push(c);
                self.name_auto = false;
                self.name_committed = false;
                self.error.clear();
            }
            KeyCode::Backspace => {
                self.name.pop();
                self.name_auto = self.name.is_empty();
                self.name_committed = false;
                self.error.clear();
            }
            KeyCode::Enter => self.commit_name(),
            KeyCode::Up | KeyCode::BackTab => self.focus = Focus::Dir,
            KeyCode::Down if !self.profiles.is_empty() => self.focus = Focus::Profile,
            KeyCode::Tab => {
                if self.name_committed {
                    self.focus = Focus::Profile;
                } else {
                    self.commit_name();
                }
            }
            _ => {}
        }
        None
    }

    fn key_profile(&mut self, code: KeyCode) -> Option<i32> {
        let n = self.profiles.len();
        if n == 0 {
            return None;
        }
        let cols = self.card_cols.max(1);
        match code {
            KeyCode::Left | KeyCode::BackTab => {
                self.profile_idx = if self.profile_idx == 0 {
                    n - 1
                } else {
                    self.profile_idx - 1
                };
            }
            KeyCode::Right | KeyCode::Tab => {
                self.profile_idx = (self.profile_idx + 1) % n;
            }
            KeyCode::Up => {
                if self.profile_idx < cols {
                    self.focus = Focus::Name;
                } else {
                    self.profile_idx -= cols;
                }
            }
            KeyCode::Down => {
                if self.profile_idx + cols >= n {
                    self.focus = Focus::Save;
                } else {
                    self.profile_idx += cols;
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.focus = Focus::Save,
            _ => {}
        }
        None
    }

    fn key_cancel(&mut self, code: KeyCode, _mods: KeyModifiers) -> Option<i32> {
        match code {
            KeyCode::Enter | KeyCode::Char(' ') => Some(0),
            KeyCode::Right | KeyCode::Tab => {
                self.focus = Focus::Save;
                None
            }
            KeyCode::Up => {
                self.focus = if self.profiles.is_empty() {
                    Focus::Name
                } else {
                    Focus::Profile
                };
                None
            }
            KeyCode::Left | KeyCode::BackTab => None,
            _ => None,
        }
    }

    fn key_save(&mut self, code: KeyCode, _mods: KeyModifiers) -> Option<i32> {
        match code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.save();
                None
            }
            KeyCode::Left | KeyCode::BackTab => {
                self.focus = Focus::Cancel;
                None
            }
            KeyCode::Up => {
                self.focus = if self.profiles.is_empty() {
                    Focus::Name
                } else {
                    Focus::Profile
                };
                None
            }
            _ => None,
        }
    }

    fn commit_dir(&mut self) {
        if let Some(s) = self.suggestions.get(self.suggest_idx).cloned() {
            self.apply_dir(&complete::accept(&s));
            return;
        }
        self.apply_dir(&self.dir.clone());
    }

    fn apply_dir(&mut self, path: &str) {
        let trimmed = path.trim();
        if trimmed.is_empty() || !complete::is_existing_dir(trimmed) {
            self.error = "not a directory".into();
            return;
        }
        let expanded = complete::expand_user(trimmed);
        self.dir = expanded.to_string_lossy().into_owned();
        if !self.dir.ends_with('/') {
            self.dir.push('/');
        }
        self.dir_committed = true;
        self.sync_name_from_dir();
        self.focus = Focus::Name;
        self.error.clear();
        self.suggestions.clear();
    }

    fn sync_name_from_dir(&mut self) {
        if self.name_auto || self.name.trim().is_empty() {
            self.name = complete::default_name_from_dir(&self.dir);
            self.name_auto = true;
            self.name_committed = !self.name.is_empty();
        }
    }

    fn commit_name(&mut self) {
        if !self.dir_committed {
            self.commit_dir();
            if !self.dir_committed {
                return;
            }
        }
        if self.name.trim().is_empty() {
            self.error = "workspace name is empty".into();
            return;
        }
        self.name = self.name.trim().to_string();
        self.name_committed = true;
        self.error.clear();
        if self.profiles.is_empty() {
            self.error = "no profiles in config.yaml".into();
            return;
        }
        self.focus = Focus::Profile;
    }

    fn save(&mut self) {
        if !self.dir_committed {
            self.commit_dir();
        }
        if !self.name_committed {
            self.commit_name();
        }
        if !self.dir_committed || !self.name_committed {
            return;
        }
        let Some(profile) = self.profiles.get(self.profile_idx).cloned() else {
            self.error = "select a profile".into();
            return;
        };
        let cwd = complete::expand_user(self.dir.trim())
            .to_string_lossy()
            .into_owned();
        let name = self.name.trim().to_string();
        self.error.clear();
        self.job = Some(apply::SaveJob::spawn(move |on_progress| {
            apply::apply_with_progress(&mut apply::HerdrCli, &cwd, &name, &profile, on_progress)
        }));
    }

    fn poll_job(&mut self, agent_starts: &mut Vec<Vec<String>>) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        job.poll();
        if let Some(result) = job.take_done() {
            self.job = None;
            match result {
                Ok(r) => {
                    agent_starts.extend(r.agent_starts);
                    self.done = true;
                    self.exit_code = 0;
                }
                Err(e) => self.error = e,
            }
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> Option<i32> {
        if self.job.is_some() {
            return None;
        }
        if let Some(picker) = self.picker.as_mut() {
            match mouse.kind {
                MouseEventKind::ScrollDown => picker.move_selection(1),
                MouseEventKind::ScrollUp => picker.move_selection(-1),
                _ => {}
            }
            return None;
        }
        let pos = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.browse_box.contains(pos) {
                    self.open_picker();
                    return None;
                }
                if self.dir_box.contains(pos) {
                    self.focus = Focus::Dir;
                } else if self.name_box.contains(pos) {
                    self.focus = Focus::Name;
                } else {
                    for (i, r) in self.card_boxes.iter().enumerate() {
                        if r.contains(pos) {
                            self.profile_idx = i;
                            self.focus = Focus::Profile;
                        }
                    }
                }
                if self.cancel_box.contains(pos) {
                    return Some(0);
                }
                if self.save_box.contains(pos) {
                    self.save();
                }
            }
            _ => {}
        }
        None
    }

    fn show_suggestions(&self) -> bool {
        !self.dir_committed
    }

    fn draw(&mut self, f: &mut Frame) {
        // Herdr's popup already draws the title and outer border. Fill the
        // whole terminal so we don't nest another "New workspace" frame.
        let area = f.area();
        if let Some(job) = self.job.as_ref() {
            draw_progress(f, area, &self.theme, job.progress());
            return;
        }
        if let Some(picker) = self.picker.as_ref() {
            picker.draw(f, &self.theme);
            return;
        }
        f.render_widget(Clear, area);
        f.render_widget(Block::default().style(self.theme.base()), area);

        let inner = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
        let body = pad(inner[0], 1, 2);
        f.render_widget(
            Paragraph::new(Span::styled(
                "  ctrl+o browse · ctrl+j/k select dir · tab complete · enter next · esc cancel",
                self.theme.muted(),
            )),
            inner[1],
        );

        let suggest_h = if self.show_suggestions() { 4 } else { 0 };
        let cards_h = cards_area_height(body.width, self.profiles.len());
        let chunks = Layout::vertical([
            Constraint::Length(3),         // dir input (bordered)
            Constraint::Length(suggest_h), // suggestions, hidden after commit
            Constraint::Length(1),         // gap
            Constraint::Length(3),         // name input (bordered)
            Constraint::Length(1),         // gap
            Constraint::Length(cards_h),   // cards, fixed height
            Constraint::Length(1),         // error
            Constraint::Length(1),         // buttons
        ])
        .split(body);

        let directory = Layout::horizontal([Constraint::Min(3), Constraint::Length(19)])
            .spacing(1)
            .split(chunks[0]);
        self.dir_box = directory[0];
        self.browse_box = Rect::new(
            directory[1].x,
            directory[1].y.saturating_add(1),
            directory[1].width,
            directory[1].height.saturating_sub(1).min(1),
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "[ Browse (Ctrl-O) ]",
                self.theme.button(false, false),
            ))
            .centered(),
            self.browse_box,
        );
        self.draw_input(
            f,
            self.dir_box,
            "Directory",
            &self.dir,
            self.focus == Focus::Dir,
            "",
        );
        if self.show_suggestions() {
            self.draw_suggestions(f, chunks[1]);
        }

        self.name_box = chunks[3];
        self.draw_input(
            f,
            chunks[3],
            "Name",
            &self.name,
            self.focus == Focus::Name,
            "",
        );
        self.draw_cards(f, chunks[5]);

        if !self.error.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled(self.error.as_str(), self.theme.error()))
                    .wrap(Wrap { trim: true }),
                chunks[6],
            );
        }

        self.draw_buttons(f, chunks[7]);

        match self.focus {
            Focus::Dir => {
                f.set_cursor_position(cursor_in_input(self.dir_box, self.dir.chars().count()))
            }
            Focus::Name => {
                f.set_cursor_position(cursor_in_input(self.name_box, self.name.chars().count()))
            }
            _ => {}
        }
    }

    fn draw_input(
        &self,
        f: &mut Frame,
        area: Rect,
        title: &str,
        value: &str,
        focused: bool,
        placeholder: &str,
    ) {
        let block = Block::bordered()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
            .border_style(self.theme.input_border(focused))
            .style(self.theme.base());
        let inner = block.inner(area);
        f.render_widget(block, area);
        let (text, style) = if value.is_empty() && !placeholder.is_empty() {
            (placeholder, self.theme.muted())
        } else {
            (value, self.theme.input(focused))
        };
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {text}"), style)),
            inner,
        );
    }

    fn draw_suggestions(&self, f: &mut Frame, area: Rect) {
        if self.suggestions.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "  no matching directories",
                    self.theme.muted(),
                )),
                area,
            );
            return;
        }
        let start = self
            .suggest_idx
            .saturating_add(1)
            .saturating_sub(area.height as usize);
        let total = self.suggestions.len();
        let lines: Vec<Line> = self
            .suggestions
            .iter()
            .enumerate()
            .skip(start)
            .take(area.height as usize)
            .map(|(i, s)| {
                let selected = self.focus == Focus::Dir && i == self.suggest_idx;
                let text = if total > area.height as usize {
                    format!("  {}/{total} {s}", i + 1)
                } else {
                    format!("  {s}")
                };
                Line::from(Span::styled(text, self.theme.suggest(selected)))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), area);
    }

    fn draw_cards(&mut self, f: &mut Frame, area: Rect) {
        let n = self.profiles.len();
        if n == 0 {
            self.card_boxes.clear();
            return;
        }
        let cols = card_columns(area.width, n);
        self.card_cols = cols;
        let rows = n.div_ceil(cols);
        let row_constraints: Vec<Constraint> =
            (0..rows).map(|_| Constraint::Length(CARD_HEIGHT)).collect();
        let row_rects = Layout::vertical(row_constraints).spacing(1).split(area);

        self.card_boxes = Vec::with_capacity(n);
        for (row_i, row) in row_rects.iter().enumerate() {
            let start = row_i * cols;
            let count = (n - start).min(cols);
            let mut constraints = Vec::with_capacity(count + 1);
            for _ in 0..count {
                constraints.push(Constraint::Length(CARD_WIDTH));
            }
            constraints.push(Constraint::Fill(1));
            let cells = Layout::horizontal(constraints).spacing(1).split(*row);
            for col_i in 0..count {
                let i = start + col_i;
                let profile = &self.profiles[i];
                let cell = cells[col_i];
                self.card_boxes.push(cell);
                let focused = self.focus == Focus::Profile && i == self.profile_idx;
                let selected = i == self.profile_idx;
                let block = Block::bordered()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", profile.name))
                    .border_style(self.theme.card_border(focused, selected))
                    .style(self.theme.base());
                let body: Vec<Line> = profile
                    .card_lines()
                    .into_iter()
                    .map(|l| Line::from(Span::styled(l, self.theme.muted())))
                    .collect();
                f.render_widget(
                    Paragraph::new(body).block(block).wrap(Wrap { trim: true }),
                    cell,
                );
            }
        }
    }

    fn draw_buttons(&mut self, f: &mut Frame, area: Rect) {
        let row = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(12),
            Constraint::Length(1),
            Constraint::Length(12),
        ])
        .split(area);
        self.cancel_box = row[1];
        self.save_box = row[3];
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "[ Cancel ]",
                self.theme.button(self.focus == Focus::Cancel, false),
            )))
            .centered(),
            row[1],
        );
        let save_ok = self.dir_committed && self.name_committed && !self.profiles.is_empty();
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "[ Save ]",
                self.theme.button(self.focus == Focus::Save, save_ok),
            )))
            .centered(),
            row[3],
        );
    }
}

pub(crate) const CARD_WIDTH: u16 = 22;
pub(crate) const CARD_HEIGHT: u16 = 5;
const CARD_COLS_MAX: usize = 4;

pub(crate) fn card_columns(width: u16, n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let stride = CARD_WIDTH + 1;
    let by_width = (width / stride).max(1) as usize;
    by_width.min(n).min(CARD_COLS_MAX)
}

pub(crate) fn cards_area_height(width: u16, n: usize) -> u16 {
    if n == 0 {
        return 1;
    }
    let cols = card_columns(width, n);
    let rows = n.div_ceil(cols) as u16;
    rows * CARD_HEIGHT + rows.saturating_sub(1)
}

pub(crate) fn pad(area: Rect, v: u16, h: u16) -> Rect {
    let chunks = Layout::vertical([
        Constraint::Length(v),
        Constraint::Min(1),
        Constraint::Length(v),
    ])
    .split(area);
    let mid = Layout::horizontal([
        Constraint::Length(h),
        Constraint::Min(1),
        Constraint::Length(h),
    ])
    .split(chunks[1]);
    mid[1]
}

pub(crate) fn draw_progress(f: &mut Frame, area: Rect, theme: &Theme, progress: &apply::Progress) {
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(theme.base()), area);
    let body = pad(area, 2, 4);
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(body);
    f.render_widget(
        Paragraph::new(Span::styled("Working", theme.input(true))),
        chunks[0],
    );
    let ratio = if progress.total == 0 {
        0.0
    } else {
        (progress.current as f64 / progress.total as f64).clamp(0.0, 1.0)
    };
    f.render_widget(
        Gauge::default()
            .block(
                Block::bordered()
                    .borders(Borders::ALL)
                    .title(" Progress ")
                    .border_style(theme.input_border(true))
                    .style(theme.base()),
            )
            .gauge_style(Style::default().fg(theme.accent).bg(theme.surface))
            .ratio(ratio)
            .label(format!("{} / {}", progress.current, progress.total)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(Span::styled(progress.label.as_str(), theme.muted()))
            .wrap(Wrap { trim: true }),
        chunks[3],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            "  applying layout — popup closes when this finishes",
            theme.muted(),
        )),
        chunks[4],
    );
}

pub(crate) fn cursor_in_input(area: Rect, chars: usize) -> Position {
    let inner_x = area.x.saturating_add(1);
    let inner_y = area.y.saturating_add(1);
    let inner_w = area.width.saturating_sub(2);
    let x = inner_x
        .saturating_add(1)
        .saturating_add(chars as u16)
        .min(inner_x.saturating_add(inner_w.saturating_sub(1)));
    Position::new(x, inner_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Profile> {
        config::parse_yaml(config::EXAMPLE_YAML).unwrap().profiles
    }

    fn app() -> App {
        use ratatui::style::Color;
        let theme = Theme {
            panel_bg: Color::Reset,
            text: Color::Reset,
            subtext: Color::Reset,
            overlay: Color::Reset,
            accent: Color::Reset,
            yellow: Color::Reset,
            green: Color::Reset,
            red: Color::Reset,
            surface: Color::Reset,
        };
        App::with_history(sample(), theme, Vec::new())
    }

    #[test]
    fn picker_opens_hidden_children_without_recursive_discovery() {
        let tree = tempfile::tempdir().unwrap();
        let hidden = tree.path().join(".hidden");
        let private = hidden.join("private-project");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(tree.path().join("file.txt"), "file").unwrap();
        let mut app = app();
        app.dir = tree.path().to_str().unwrap().into();
        app.on_key(KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert_eq!(app.picker.as_ref().unwrap().children, [hidden.clone()]);
        app.on_key(KeyCode::Down, KeyModifiers::NONE);
        app.on_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(app.picker.as_ref().unwrap().path, hidden);
        app.on_key(KeyCode::Down, KeyModifiers::NONE);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.dir, complete::accept(private.to_str().unwrap()));
        assert_eq!(app.name, "private-project");
        assert_eq!(app.focus, Focus::Name);
        assert!(app.picker.is_none());
    }

    #[test]
    fn picker_cancel_preserves_form_state_after_parent_navigation() {
        let tree = tempfile::tempdir().unwrap();
        let child = tree.path().join("alpha");
        std::fs::create_dir(&child).unwrap();
        let mut app = app();
        app.project_directories = vec![child.to_str().unwrap().into()];
        app.dir = "alpha".into();
        app.name = "manual-name".into();
        app.name_auto = false;
        app.profile_idx = 1;
        app.refresh_suggestions();
        let suggestions = app.suggestions.clone();
        app.on_key(KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert_eq!(app.picker.as_ref().unwrap().path, child);
        app.on_key(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(app.picker.as_ref().unwrap().path, tree.path());
        assert_eq!(app.on_key(KeyCode::Esc, KeyModifiers::NONE), None);
        assert!(app.picker.is_none());
        assert_eq!(app.dir, "alpha");
        assert_eq!(app.name, "manual-name");
        assert_eq!(app.profile_idx, 1);
        assert_eq!(app.focus, Focus::Dir);
        assert_eq!(app.suggestions, suggestions);
        assert_eq!(app.on_key(KeyCode::Esc, KeyModifiers::NONE), Some(0));
    }

    #[test]
    fn picker_selects_directories_beyond_inline_rows_and_keeps_manual_name() {
        let tree = tempfile::tempdir().unwrap();
        for index in 0..10 {
            std::fs::create_dir(tree.path().join(format!("project-{index}"))).unwrap();
        }
        let mut app = app();
        app.dir = tree.path().to_str().unwrap().into();
        app.name = "manual-name".into();
        app.name_auto = false;
        app.on_key(KeyCode::Char('o'), KeyModifiers::CONTROL);
        for _ in 0..9 {
            app.on_key(KeyCode::Down, KeyModifiers::NONE);
        }
        app.on_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(
            app.picker.as_ref().unwrap().path,
            tree.path().join("project-8")
        );
        assert!(app.picker.as_ref().unwrap().children.is_empty());
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            app.dir,
            complete::accept(tree.path().join("project-8").to_str().unwrap())
        );
        assert_eq!(app.name, "manual-name");
        assert!(app.dir_committed);
    }

    #[test]
    fn picker_reports_failed_navigation_without_losing_current_folder() {
        let tree = tempfile::tempdir().unwrap();
        let child = tree.path().join("removed");
        std::fs::create_dir(&child).unwrap();
        let mut picker = DirectoryPicker::read(tree.path().to_path_buf()).unwrap();
        picker.move_selection(1);
        std::fs::remove_dir(&child).unwrap();
        picker.navigate(picker.selected_path().to_path_buf());
        assert_eq!(picker.path, tree.path());
        assert_eq!(picker.selected_path(), child);
        assert!(!picker.error.is_empty());
    }

    #[test]
    fn picker_can_choose_an_empty_directory() {
        let tree = tempfile::tempdir().unwrap();
        let mut app = app();
        app.dir = tree.path().to_str().unwrap().into();
        app.on_key(KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert!(app.picker.as_ref().unwrap().children.is_empty());
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.dir, complete::accept(tree.path().to_str().unwrap()));
        assert!(app.dir_committed);
        assert_eq!(app.focus, Focus::Name);
    }

    #[test]
    fn starts_without_prefilling_directory_or_name() {
        let mut app = app();
        assert!(app.dir.is_empty());
        assert!(app.name.is_empty());
        app.suggestions.clear();
        app.commit_dir();
        assert!(!app.dir_committed);
        assert_eq!(app.focus, Focus::Dir);
    }

    #[test]
    fn commit_dir_requires_real_path() {
        let mut app = app();
        app.dir = "/no/such/path".into();
        app.refresh_suggestions();
        app.commit_dir();
        assert!(!app.dir_committed);
        assert!(!app.error.is_empty());
    }

    #[test]
    fn commit_name_unlocks_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.dir = dir.path().to_string_lossy().into_owned();
        app.suggestions.clear();
        app.commit_dir();
        assert!(app.dir_committed);
        assert_eq!(app.focus, Focus::Name);
        app.name = "demo".into();
        app.commit_name();
        assert!(app.name_committed);
        assert_eq!(app.focus, Focus::Profile);
    }

    #[test]
    fn enter_selects_highlighted_directory() {
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir(tree.path().join("alpha")).unwrap();
        std::fs::create_dir(tree.path().join("beta")).unwrap();
        let mut app = app();
        app.dir = format!("{}/", tree.path().display());
        app.name_auto = true;
        app.refresh_suggestions();
        app.focus = Focus::Dir;
        assert!(complete::is_existing_dir(&app.dir));
        assert!(
            app.suggestions.iter().any(|s| s.ends_with("beta/")),
            "{:?}",
            app.suggestions
        );
        app.suggest_idx = app
            .suggestions
            .iter()
            .position(|s| s.ends_with("beta/"))
            .unwrap();
        app.key_dir(KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.dir.ends_with("beta/"), "{}", app.dir);
        assert!(app.dir_committed);
        assert_eq!(app.name, "beta");
        assert_eq!(app.focus, Focus::Name);
    }

    #[test]
    fn name_defaults_to_selected_directory() {
        let tree = tempfile::tempdir().unwrap();
        let project = tree.path().join("my-project");
        std::fs::create_dir(&project).unwrap();
        let mut app = app();
        app.dir = project.to_string_lossy().into_owned();
        app.suggestions.clear();
        app.name.clear();
        app.name_auto = true;
        app.commit_dir();
        assert_eq!(app.name, "my-project");
        app.name = "custom".into();
        app.name_auto = false;
        app.dir = tree.path().to_string_lossy().into_owned();
        app.suggestions.clear();
        app.commit_dir();
        assert_eq!(app.name, "custom");
    }

    #[test]
    fn ctrl_jk_moves_directory_suggestions() {
        let mut app = app();
        app.suggestions = vec!["a/".into(), "b/".into(), "c/".into()];
        app.suggest_idx = 0;
        app.focus = Focus::Dir;
        app.key_dir(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(app.suggest_idx, 1);
        app.key_dir(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(app.suggest_idx, 0);
        app.key_dir(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(app.suggest_idx, 2);
    }

    #[test]
    fn enter_selects_match_beyond_four_visible_rows() {
        let tree = tempfile::tempdir().unwrap();
        let mut app = app();
        app.suggestions = (0..6)
            .map(|i| {
                let path = tree.path().join(format!("web-{i}"));
                std::fs::create_dir(&path).unwrap();
                format!("{}/", path.display())
            })
            .collect();
        for _ in 0..5 {
            app.key_dir(KeyCode::Char('j'), KeyModifiers::CONTROL);
        }
        assert_eq!(app.suggest_idx, 5);
        app.key_dir(KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.dir.ends_with("web-5/"));
        assert_eq!(app.name, "web-5");
    }

    #[test]
    fn typing_resets_highlight_before_enter() {
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir(tree.path().join("alpha")).unwrap();
        std::fs::create_dir(tree.path().join("alpine")).unwrap();
        std::fs::create_dir(tree.path().join("alpinist")).unwrap();
        let mut app = app();
        app.directory_history.clear();
        app.dir = format!("{}/alp", tree.path().display());
        app.refresh_suggestions();
        app.key_dir(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(app.suggest_idx, 1);
        app.key_dir(KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(app.suggest_idx, 0);
        app.key_dir(KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.dir.ends_with("alpine/"), "{}", app.dir);
        assert_eq!(app.name, "alpine");
    }

    #[test]
    fn committed_directory_hides_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.dir = dir.path().to_string_lossy().into_owned();
        assert!(app.show_suggestions());
        app.commit_dir();
        assert!(app.dir_committed);
        assert!(!app.show_suggestions());
        app.key_dir(KeyCode::Backspace, KeyModifiers::NONE);
        assert!(!app.dir_committed);
        assert!(app.show_suggestions());
    }
}
