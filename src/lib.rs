use std::{
    collections::BTreeMap,
    fs,
    io::{self, Stdout},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
pub mod theme;

use crate::theme::{Theme, ThemePreset};

pub fn run() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut app = App::new(cwd)?;

    loop {
        terminal.draw(|frame| app.render(frame))?;

        if event::poll(Duration::from_millis(150))?
            && let Event::Key(key) = event::read()?
            && !app.handle_key(key)?
        {
            break;
        }
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

impl FileEntry {
    fn display_name(&self) -> String {
        if self.is_dir {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Debug)]
struct PaneState {
    cwd: PathBuf,
    entries: Vec<FileEntry>,
    selected: usize,
    list_state: ListState,
}

impl PaneState {
    fn new(cwd: PathBuf) -> io::Result<Self> {
        let mut pane = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
        };
        pane.reload()?;
        Ok(pane)
    }

    fn reload(&mut self) -> io::Result<()> {
        self.entries = read_dir_entries(&self.cwd)?;
        if self.entries.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
        } else {
            self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            self.list_state.select(Some(self.selected));
        }
        Ok(())
    }

    fn move_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        self.list_state.select(Some(self.selected));
    }

    fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
        self.list_state.select(Some(self.selected));
    }

    fn move_top(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = 0;
        self.list_state.select(Some(self.selected));
    }

    fn move_bottom(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = self.entries.len() - 1;
        self.list_state.select(Some(self.selected));
    }

    fn selected_entry(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected)
    }

    fn enter_selected(&mut self) -> io::Result<()> {
        if let Some(entry) = self.selected_entry()
            && entry.is_dir
        {
            self.cwd = entry.path.clone();
            self.selected = 0;
            self.reload()?;
        }
        Ok(())
    }

    fn go_parent(&mut self) -> io::Result<()> {
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.selected = 0;
            self.reload()?;
        }
        Ok(())
    }

    fn preview_lines(&self, max_lines: usize) -> Vec<Line<'static>> {
        match self.selected_entry() {
            Some(entry) if entry.is_dir => vec![
                Line::from(format!("dir: {}", entry.path.display())),
                Line::from(format!("items: {}", count_items(&entry.path))),
            ],
            Some(entry) => preview_file(&entry.path, max_lines),
            None => vec![Line::from("empty directory")],
        }
    }

    fn delete_selected(&mut self) -> io::Result<Option<String>> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(None);
        };

        if entry.is_dir {
            fs::remove_dir_all(&entry.path)?;
        } else {
            fs::remove_file(&entry.path)?;
        }

        let removed_name = entry.display_name();
        self.reload()?;
        Ok(Some(removed_name))
    }
}

fn count_items(path: &Path) -> usize {
    fs::read_dir(path).map(|iter| iter.count()).unwrap_or(0)
}

fn preview_file(path: &Path, max_lines: usize) -> Vec<Line<'static>> {
    let Ok(metadata) = fs::metadata(path) else {
        return vec![Line::from("unable to read metadata")];
    };

    if metadata.len() > 128 * 1024 {
        return vec![
            Line::from(format!("file: {}", path.display())),
            Line::from(format!("size: {} bytes", metadata.len())),
            Line::from("preview skipped for files larger than 128 KiB"),
        ];
    }

    match fs::read_to_string(path) {
        Ok(contents) => contents
            .lines()
            .take(max_lines.max(1))
            .map(|line| Line::from(line.to_owned()))
            .collect(),
        Err(_) => vec![
            Line::from(format!("file: {}", path.display())),
            Line::from(format!("size: {} bytes", metadata.len())),
            Line::from("binary or non-utf8 file"),
        ],
    }
}

fn read_dir_entries(path: &Path) -> io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    for item in fs::read_dir(path)? {
        let item = item?;
        let file_type = item.file_type()?;
        let metadata = item.metadata()?;
        entries.push(FileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: item.path(),
            is_dir: file_type.is_dir(),
            size: metadata.len(),
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LayoutNode {
    Leaf {
        pane_id: usize,
    },
    Split {
        direction: SplitDirection,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    fn split_leaf(self, target: usize, direction: SplitDirection, new_pane_id: usize) -> Self {
        match self {
            LayoutNode::Leaf { pane_id } if pane_id == target => LayoutNode::Split {
                direction,
                first: Box::new(LayoutNode::Leaf { pane_id }),
                second: Box::new(LayoutNode::Leaf {
                    pane_id: new_pane_id,
                }),
            },
            LayoutNode::Leaf { pane_id } => LayoutNode::Leaf { pane_id },
            LayoutNode::Split {
                direction: split_direction,
                first,
                second,
            } => LayoutNode::Split {
                direction: split_direction,
                first: Box::new(first.split_leaf(target, direction, new_pane_id)),
                second: Box::new(second.split_leaf(target, direction, new_pane_id)),
            },
        }
    }

    fn close_pane(self, target: usize) -> Option<Self> {
        match self {
            LayoutNode::Leaf { pane_id } => {
                if pane_id == target {
                    None
                } else {
                    Some(LayoutNode::Leaf { pane_id })
                }
            }
            LayoutNode::Split {
                direction,
                first,
                second,
            } => {
                let first = first.close_pane(target);
                let second = second.close_pane(target);
                match (first, second) {
                    (None, None) => None,
                    (Some(node), None) | (None, Some(node)) => Some(node),
                    (Some(first), Some(second)) => Some(LayoutNode::Split {
                        direction,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                }
            }
        }
    }

    fn pane_ids(&self, output: &mut Vec<usize>) {
        match self {
            LayoutNode::Leaf { pane_id } => output.push(*pane_id),
            LayoutNode::Split { first, second, .. } => {
                first.pane_ids(output);
                second.pane_ids(output);
            }
        }
    }

    fn render_rects(&self, area: Rect, map: &mut BTreeMap<usize, Rect>) {
        match self {
            LayoutNode::Leaf { pane_id } => {
                map.insert(*pane_id, area);
            }
            LayoutNode::Split {
                direction,
                first,
                second,
            } => {
                let chunks = Layout::default()
                    .direction(match direction {
                        SplitDirection::Horizontal => Direction::Vertical,
                        SplitDirection::Vertical => Direction::Horizontal,
                    })
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(area);
                first.render_rects(chunks[0], map);
                second.render_rects(chunks[1], map);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PendingAction {
    ConfirmDelete { pane_id: usize, target_name: String },
    ThemePicker { selected: usize },
}

#[derive(Debug)]
struct App {
    theme: Theme,
    theme_preset: ThemePreset,
    panes: BTreeMap<usize, PaneState>,
    layout: LayoutNode,
    focused_pane: usize,
    next_pane_id: usize,
    status: String,
    command_mode: bool,
    command_buffer: String,
    awaiting_ctrl_w: bool,
    pending_g: bool,
    pending_action: Option<PendingAction>,
}

impl App {
    fn new(cwd: PathBuf) -> io::Result<Self> {
        let pane = PaneState::new(cwd)?;
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);

        Ok(Self {
            theme: ThemePreset::Default.into(),
            theme_preset: ThemePreset::Default,
            panes,
            layout: LayoutNode::Leaf { pane_id: 1 },
            focused_pane: 1,
            next_pane_id: 2,
            status: String::from("normal mode"),
            command_mode: false,
            command_buffer: String::new(),
            awaiting_ctrl_w: false,
            pending_g: false,
            pending_action: None,
        })
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.pending_action.is_some() {
            return self.handle_pending_action_key(key);
        }

        if self.command_mode {
            return self.handle_command_key(key);
        }

        if self.awaiting_ctrl_w {
            self.awaiting_ctrl_w = false;
            return self.handle_ctrl_w(key);
        }

        let should_continue = match key.code {
            KeyCode::Char('q') => false,
            KeyCode::Char(':') => {
                self.command_mode = true;
                self.command_buffer.clear();
                self.status = String::from("command mode");
                true
            }
            KeyCode::Char('j') => {
                self.current_pane_mut()?.move_down();
                self.pending_g = false;
                true
            }
            KeyCode::Char('k') => {
                self.current_pane_mut()?.move_up();
                self.pending_g = false;
                true
            }
            KeyCode::Char('h') => {
                self.current_pane_mut()?.go_parent()?;
                self.status = String::from("moved to parent directory");
                self.pending_g = false;
                true
            }
            KeyCode::Char('l') | KeyCode::Enter => {
                self.current_pane_mut()?.enter_selected()?;
                self.status = String::from("opened directory");
                self.pending_g = false;
                true
            }
            KeyCode::Char('g') => {
                if self.pending_g {
                    self.current_pane_mut()?.move_top();
                    self.pending_g = false;
                    self.status = String::from("jumped to top");
                } else {
                    self.pending_g = true;
                    self.status = String::from("pending: g");
                }
                true
            }
            KeyCode::Char('G') => {
                self.current_pane_mut()?.move_bottom();
                self.pending_g = false;
                self.status = String::from("jumped to bottom");
                true
            }
            KeyCode::Char('d') => {
                self.start_delete_confirmation();
                self.pending_g = false;
                true
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.awaiting_ctrl_w = true;
                self.pending_g = false;
                self.status = String::from("Ctrl-w");
                true
            }
            KeyCode::Esc => {
                self.pending_g = false;
                self.status = String::from("normal mode");
                true
            }
            _ => {
                self.pending_g = false;
                true
            }
        };

        Ok(should_continue)
    }

    fn handle_pending_action_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(action) = self.pending_action.take() else {
            return Ok(true);
        };

        match action {
            PendingAction::ConfirmDelete {
                pane_id,
                target_name,
            } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_delete(pane_id, &target_name)?;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.status = format!("delete cancelled: {target_name}");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmDelete {
                        pane_id,
                        target_name: target_name.clone(),
                    });
                    self.status = format!("confirm delete {target_name}: y/n");
                }
            },
            PendingAction::ThemePicker { mut selected } => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    selected = (selected + 1) % ThemePreset::ALL.len();
                    self.pending_action = Some(PendingAction::ThemePicker { selected });
                    self.status = format!("theme picker: {}", ThemePreset::ALL[selected].name());
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                    self.pending_action = Some(PendingAction::ThemePicker { selected });
                    self.status = format!("theme picker: {}", ThemePreset::ALL[selected].name());
                }
                KeyCode::Enter => {
                    self.apply_theme(ThemePreset::ALL[selected]);
                }
                KeyCode::Esc => {
                    self.status = String::from("theme picker cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ThemePicker { selected });
                    self.status = String::from("theme picker: use j/k and Enter");
                }
            },
        }

        Ok(true)
    }

    fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.command_mode = false;
                self.command_buffer.clear();
                self.status = String::from("normal mode");
            }
            KeyCode::Backspace => {
                self.command_buffer.pop();
            }
            KeyCode::Enter => {
                let command = std::mem::take(&mut self.command_buffer);
                self.command_mode = false;
                self.execute_command(command.trim())?;
            }
            KeyCode::Char(c) => {
                self.command_buffer.push(c);
            }
            _ => {}
        }
        Ok(true)
    }

    fn handle_ctrl_w(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('h') | KeyCode::Char('k') => self.focus_previous_pane(),
            KeyCode::Char('l') | KeyCode::Char('j') => self.focus_next_pane(),
            KeyCode::Char('v') => self.split_current(SplitDirection::Vertical)?,
            KeyCode::Char('s') => self.split_current(SplitDirection::Horizontal)?,
            KeyCode::Char('c') => self.close_current_pane(),
            KeyCode::Char('o') => self.only_current_pane(),
            _ => {
                self.status = String::from("unknown Ctrl-w command");
            }
        }
        Ok(true)
    }

    fn execute_command(&mut self, command: &str) -> Result<()> {
        match command {
            "q" => {
                self.status = String::from("use q in normal mode to quit");
            }
            "theme" => self.open_theme_picker(),
            "theme next" => self.cycle_theme(),
            "split" => self.split_current(SplitDirection::Horizontal)?,
            "vsplit" => self.split_current(SplitDirection::Vertical)?,
            "close" => self.close_current_pane(),
            "only" => self.only_current_pane(),
            "" => {
                self.status = String::from("normal mode");
            }
            other => {
                if let Some(name) = other.strip_prefix("theme ") {
                    self.set_theme_by_name(name.trim());
                } else {
                    self.status = format!("unknown command: {other}");
                }
            }
        }
        Ok(())
    }

    fn current_pane_mut(&mut self) -> io::Result<&mut PaneState> {
        self.panes
            .get_mut(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused pane"))
    }

    fn split_current(&mut self, direction: SplitDirection) -> io::Result<()> {
        let cwd = self
            .panes
            .get(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused pane"))?
            .cwd
            .clone();

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        self.panes.insert(new_id, PaneState::new(cwd)?);
        self.layout = self
            .layout
            .clone()
            .split_leaf(self.focused_pane, direction, new_id);
        self.focused_pane = new_id;
        self.status = match direction {
            SplitDirection::Horizontal => String::from("horizontal split"),
            SplitDirection::Vertical => String::from("vertical split"),
        };
        Ok(())
    }

    fn ordered_pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.layout.pane_ids(&mut ids);
        ids
    }

    fn focus_next_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if let Some(index) = ids.iter().position(|id| *id == self.focused_pane) {
            self.focused_pane = ids[(index + 1) % ids.len()];
            self.status = format!("focused pane {}", self.focused_pane);
        }
    }

    fn focus_previous_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if let Some(index) = ids.iter().position(|id| *id == self.focused_pane) {
            self.focused_pane = ids[(index + ids.len() - 1) % ids.len()];
            self.status = format!("focused pane {}", self.focused_pane);
        }
    }

    fn close_current_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if ids.len() <= 1 {
            self.status = String::from("cannot close the last pane");
            return;
        }

        let old_focus = self.focused_pane;
        if let Some(index) = ids.iter().position(|id| *id == old_focus) {
            let fallback = if index > 0 {
                ids[index - 1]
            } else {
                ids[index + 1]
            };
            if let Some(layout) = self.layout.clone().close_pane(old_focus) {
                self.layout = layout;
                self.panes.remove(&old_focus);
                self.focused_pane = fallback;
                self.status = format!("closed pane {old_focus}");
            }
        }
    }

    fn only_current_pane(&mut self) {
        let focused = self.focused_pane;
        self.panes.retain(|id, _| *id == focused);
        self.layout = LayoutNode::Leaf { pane_id: focused };
        self.status = String::from("kept only focused pane");
    }

    fn cycle_theme(&mut self) {
        let next = self.theme_preset.next();
        self.apply_theme(next);
    }

    fn open_theme_picker(&mut self) {
        let selected = ThemePreset::ALL
            .iter()
            .position(|preset| *preset == self.theme_preset)
            .unwrap_or(0);
        self.pending_action = Some(PendingAction::ThemePicker { selected });
        self.status = String::from("theme picker: use j/k and Enter");
    }

    fn set_theme_by_name(&mut self, name: &str) {
        match ThemePreset::from_name(name) {
            Some(preset) => self.apply_theme(preset),
            None => {
                let available = ThemePreset::ALL
                    .iter()
                    .map(|preset| preset.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.status = format!("unknown theme: {name}. available: {available}");
            }
        }
    }

    fn apply_theme(&mut self, preset: ThemePreset) {
        self.theme_preset = preset;
        self.theme = preset.into();
        self.status = format!("theme: {}", preset.name());
    }

    fn start_delete_confirmation(&mut self) {
        let Some(entry) = self
            .panes
            .get(&self.focused_pane)
            .and_then(PaneState::selected_entry)
            .cloned()
        else {
            self.status = String::from("nothing selected to delete");
            return;
        };

        self.pending_action = Some(PendingAction::ConfirmDelete {
            pane_id: self.focused_pane,
            target_name: entry.display_name(),
        });
        self.status = format!("confirm delete {}: y/n", entry.display_name());
    }

    fn confirm_delete(&mut self, pane_id: usize, target_name: &str) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        match pane.delete_selected() {
            Ok(Some(removed_name)) => {
                self.status = format!("deleted {removed_name}");
            }
            Ok(None) => {
                self.status = String::from("nothing selected to delete");
            }
            Err(error) => {
                self.status = format!("failed to delete {target_name}: {error}");
            }
        }

        Ok(())
    }

    fn render(&mut self, frame: &mut ratatui::Frame<'_>) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ])
            .split(frame.area());

        let mut pane_rects = BTreeMap::new();
        self.layout.render_rects(outer[0], &mut pane_rects);
        for (pane_id, rect) in pane_rects {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                render_pane(
                    frame,
                    rect,
                    pane_id,
                    pane,
                    pane_id == self.focused_pane,
                    self.theme,
                );
            }
        }

        let help = Paragraph::new(Line::from(vec![
            Span::styled("hjkl", self.theme.accent_style()),
            Span::raw(" move  "),
            Span::styled("gg/G", self.theme.accent_style()),
            Span::raw(" jump  "),
            Span::styled("Ctrl-w s/v", self.theme.accent_style()),
            Span::raw(" split  "),
            Span::styled("Ctrl-w h/j/k/l", self.theme.accent_style()),
            Span::raw(" focus  "),
            Span::styled("d", self.theme.accent_style()),
            Span::raw(" delete  "),
            Span::styled(":split :vsplit :close :only", self.theme.accent_style()),
            Span::raw("  "),
            Span::styled(":theme", self.theme.accent_style()),
        ]))
        .block(Block::default().borders(Borders::TOP));
        frame.render_widget(help, outer[1]);

        let status_text = if self.command_mode {
            format!(":{}", self.command_buffer)
        } else {
            self.status.clone()
        };
        frame.render_widget(Paragraph::new(status_text), outer[2]);

        if self.command_mode {
            let area = centered_rect(frame.area(), 70, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!(":{}", self.command_buffer))
                    .block(Block::default().title("Command").borders(Borders::ALL)),
                area,
            );
        }

        match &self.pending_action {
            Some(PendingAction::ConfirmDelete { target_name, .. }) => {
                render_confirm_dialog(frame, frame.area(), target_name, self.theme);
            }
            Some(PendingAction::ThemePicker { selected }) => {
                render_theme_picker(frame, frame.area(), self.theme, *selected);
            }
            None => {}
        }
    }
}

fn render_pane(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    pane_id: usize,
    pane: &mut PaneState,
    focused: bool,
    theme: Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(6)])
        .split(area);

    let border_style = if focused {
        theme.focused_border_style()
    } else {
        theme.muted_style()
    };

    let title = format!(" pane {}  {}", pane_id, pane.cwd.display());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let items: Vec<ListItem<'static>> = if pane.entries.is_empty() {
        vec![ListItem::new(Line::from("empty directory"))]
    } else {
        pane.entries
            .iter()
            .map(|entry| {
                let detail = if entry.is_dir {
                    String::from("dir")
                } else {
                    format!("{}b", entry.size)
                };
                ListItem::new(Line::from(vec![
                    Span::raw(entry.display_name()),
                    Span::styled(format!("  [{detail}]"), theme.muted_style()),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, chunks[0], &mut pane.list_state);

    let preview = Paragraph::new(pane.preview_lines(4)).block(
        Block::default()
            .title("Preview")
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(border_style),
    );
    frame.render_widget(preview, chunks[1]);
}

fn centered_rect(area: Rect, width_percent: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(height),
            Constraint::Min(1),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn render_confirm_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    target_name: &str,
    theme: Theme,
) {
    let dialog_area = centered_rect(area, 60, 5);
    frame.render_widget(Clear, dialog_area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("Delete {target_name}?")),
            Line::from("Press y to confirm, n or Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Confirm Delete ",
                    theme.danger_title_style(),
                )))
                .borders(Borders::ALL),
        ),
        dialog_area,
    );
}

fn render_theme_picker(frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme, selected: usize) {
    let dialog_area = centered_rect(area, 42, 8);
    frame.render_widget(Clear, dialog_area);

    let items: Vec<ListItem<'static>> = ThemePreset::ALL
        .iter()
        .map(|preset| ListItem::new(Line::from(preset.name().to_string())))
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let list = List::new(items)
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Theme Picker ",
                    theme.accent_style().add_modifier(Modifier::BOLD),
                )))
                .borders(Borders::ALL),
        )
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, dialog_area, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn split_leaf_replaces_target_with_split_node() {
        let layout = LayoutNode::Leaf { pane_id: 1 };
        let updated = layout.split_leaf(1, SplitDirection::Vertical, 2);

        assert_eq!(
            updated,
            LayoutNode::Split {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Leaf { pane_id: 1 }),
                second: Box::new(LayoutNode::Leaf { pane_id: 2 }),
            }
        );
    }

    #[test]
    fn close_pane_collapses_parent_split() {
        let layout = LayoutNode::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Leaf { pane_id: 1 }),
            second: Box::new(LayoutNode::Leaf { pane_id: 2 }),
        };

        assert_eq!(layout.close_pane(2), Some(LayoutNode::Leaf { pane_id: 1 }));
    }

    #[test]
    fn pane_state_lists_directories_before_files() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("nested")).expect("nested dir");
        fs::write(dir.path().join("alpha.txt"), "hello").expect("file");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let names: Vec<String> = pane.entries.iter().map(FileEntry::display_name).collect();

        assert_eq!(
            names,
            vec![String::from("nested/"), String::from("alpha.txt")]
        );
    }

    #[test]
    fn pane_state_enters_and_leaves_directories() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        fs::create_dir(&child).expect("child dir");
        fs::write(child.join("note.txt"), "hello").expect("note");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.enter_selected().expect("enter child");
        assert_eq!(pane.cwd, child);

        pane.go_parent().expect("back parent");
        assert_eq!(pane.cwd, dir.path());
    }

    #[test]
    fn app_only_keeps_focused_pane() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.only_current_pane();

        assert_eq!(app.ordered_pane_ids().len(), 1);
        assert_eq!(
            app.layout,
            LayoutNode::Leaf {
                pane_id: app.focused_pane
            }
        );
    }

    #[test]
    fn pane_state_delete_selected_file_removes_it() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let removed = pane.delete_selected().expect("delete");

        assert_eq!(removed, Some(String::from("alpha.txt")));
        assert!(!file_path.exists());
        assert!(pane.entries.is_empty());
    }

    #[test]
    fn app_delete_confirmation_removes_selected_entry() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf()).expect("app");
        app.start_delete_confirmation();
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm delete");

        assert!(!file_path.exists());
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "deleted delete-me.txt");
    }

    #[test]
    fn app_cycle_theme_switches_to_next_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf()).expect("app");

        app.cycle_theme();

        assert_eq!(app.theme_preset, ThemePreset::Forest);
        assert_eq!(app.theme, ThemePreset::Forest.into());
        assert_eq!(app.status, "theme: forest");
    }

    #[test]
    fn app_open_theme_picker_tracks_current_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf()).expect("app");

        app.open_theme_picker();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker { selected: 0 })
        );
    }

    #[test]
    fn app_set_theme_by_name_updates_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf()).expect("app");

        app.set_theme_by_name("ocean");

        assert_eq!(app.theme_preset, ThemePreset::Ocean);
        assert_eq!(app.theme, ThemePreset::Ocean.into());
        assert_eq!(app.status, "theme: ocean");
    }

    #[test]
    fn app_theme_picker_confirm_applies_selected_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf()).expect("app");
        app.pending_action = Some(PendingAction::ThemePicker { selected: 2 });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("apply theme");

        assert_eq!(app.theme_preset, ThemePreset::Ocean);
        assert_eq!(app.theme, ThemePreset::Ocean.into());
        assert_eq!(app.status, "theme: ocean");
    }
}
