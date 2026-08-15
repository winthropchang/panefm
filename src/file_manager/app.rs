use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    config::{AppConfig, LoadedConfig, StartupSort},
    theme::{Theme, ThemePreset},
};

use super::{
    bookmark::{BookmarkEntry, BookmarkStore, bookmark_file_path},
    layout::{LayoutNode, SplitDirection},
    open::{
        LaunchSpec, OpenAction, OpenTarget, build_launch_spec, default_open_action,
        open_picker_options,
    },
    pane::{PaneState, SortMode},
    search::{GlobalSearchEntry, GlobalSearchEvent, stream_search_entries},
    trash::{TrashListEntry, TrashStore},
    ui::{
        BookmarkPanelLine, HelpPanelLine, InlineEditorState, InlinePickerState, PaneListState,
        SearchListState, TrashPanelLine, render_bookmark_picker, render_command_palette,
        render_confirm_dialog, render_filter_input, render_global_search_panel, render_pane,
        render_preview_search_input, render_theme_picker,
    },
};

/// 表示 rename 輸入框目前採用的編輯模式。
///
/// `Insert` 代表可以直接插入文字，游標會顯示成細線；
/// `Normal` 代表遵循 Vim 的一般模式，只負責移動游標與切換模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameMode {
    Insert,
    Normal,
}

/// 表示目前剪貼簿保存的是複製還是剪下操作。
///
/// 這個模式會決定 `p` 貼上時，是保留來源還是把來源移動到新位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardOperation {
    Copy,
    Cut,
}

/// 記錄目前暫存在檔案管理器內部剪貼簿中的單一項目。
///
/// 這一版先支援單一檔案或資料夾，之後若要擴充多選，
/// 可以再把這個結構改成清單形式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardEntry {
    pub(crate) source_path: PathBuf,
    pub(crate) display_name: String,
}

/// 表示目前內部剪貼簿保存的一批項目與其操作模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardState {
    pub(crate) entries: Vec<ClipboardEntry>,
    pub(crate) operation: ClipboardOperation,
}

/// 記錄目前 filter 的目標 pane、查詢字串與是否仍在輸入中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
}

/// 記錄目前 preview search 的目標 pane、查詢字串與是否仍在輸入中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreviewSearchState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
}

/// 記錄目前 global search 的目標 pane、查詢文字與搜尋結果狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalSearchState {
    pub(crate) pane_id: usize,
    pub(crate) root_dir: PathBuf,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
    pub(crate) loading: bool,
    pub(crate) searched: bool,
    pub(crate) selected: usize,
    pub(crate) results: Vec<GlobalSearchEntry>,
}

/// 記錄目前是否處於範圍標記模式，以及起點和目前游標位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisualSelectionState {
    pub(crate) pane_id: usize,
    pub(crate) anchor: usize,
    pub(crate) current: usize,
}

/// 表示目前是否正在等待使用者補上書籤按鍵。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BookmarkPrompt {
    Set,
    Jump,
}

/// 描述暫時面板中的搜尋輸入狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelSearchState {
    pub(crate) buffer: String,
    pub(crate) editing: bool,
}

/// 表示目前正在等待使用者完成的暫時互動。
///
/// 只要有 pending action，輸入會先被它攔截，
/// 而不會直接進到一般檔案瀏覽模式。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PendingAction {
    ConfirmDelete {
        pane_id: usize,
        target_name: String,
    },
    SortPicker {
        pane_id: usize,
    },
    ThemePicker {
        selected: usize,
    },
    TrashPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    },
    HelpPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
    },
    BookmarkList {
        pane_id: usize,
        selected: usize,
    },
    OpenPicker {
        pane_id: usize,
        target: OpenTarget,
        selected: usize,
    },
    Rename {
        pane_id: usize,
        original_name: String,
        buffer: String,
        cursor: usize,
        mode: RenameMode,
    },
    CreateEntry {
        pane_id: usize,
        buffer: String,
        cursor: usize,
        mode: RenameMode,
    },
}

/// 表示整個應用程式的核心狀態。
///
/// 這個結構整合了設定、主題、視窗布局、焦點與互動模式，
/// 是整個 TUI 運作時最主要的狀態容器。
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) config: AppConfig,
    pub(crate) theme: Theme,
    pub(crate) theme_preset: ThemePreset,
    pub(crate) trash_store: TrashStore,
    pub(crate) bookmark_store: BookmarkStore,
    pub(crate) panes: BTreeMap<usize, PaneState>,
    pub(crate) layout: LayoutNode,
    pub(crate) focused_pane: usize,
    pub(crate) next_pane_id: usize,
    pub(crate) status: String,
    pub(crate) command_mode: bool,
    pub(crate) command_buffer: String,
    pub(crate) awaiting_ctrl_w: bool,
    pub(crate) pending_g: bool,
    pub(crate) pending_y: bool,
    pub(crate) pending_bookmark: Option<BookmarkPrompt>,
    pub(crate) clipboard: Option<ClipboardState>,
    pub(crate) filter: Option<FilterState>,
    pub(crate) preview_search: Option<PreviewSearchState>,
    pub(crate) global_search: Option<GlobalSearchState>,
    pub(crate) global_search_rx: Option<Receiver<GlobalSearchEvent>>,
    pub(crate) global_search_cancelled: Option<Arc<AtomicBool>>,
    pub(crate) visual_selection: Option<VisualSelectionState>,
    pub(crate) pending_action: Option<PendingAction>,
    pub(crate) help_return: Option<HelpReturnState>,
    pub(crate) pending_launch: Option<LaunchSpec>,
    pub(crate) preview_focus: Option<usize>,
}

/// 記錄 F1 help 關閉後應回復到哪一種互動上下文。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HelpReturnState {
    Pending(PendingAction),
    Filter(FilterState),
    PreviewSearch(PreviewSearchState),
    GlobalSearch(GlobalSearchState),
    VisualSelection(VisualSelectionState),
    CommandMode(String),
    AwaitingCtrlW,
    PendingBookmark(BookmarkPrompt),
    PreviewFocus(usize),
}

impl App {
    /// 建立一個新的應用程式狀態。
    ///
    /// 參數：
    /// - `cwd: PathBuf`，啟動時第一個 pane 要打開的目錄。
    /// - `loaded_config: LoadedConfig`，啟動時已載入的設定與來源資訊。
    ///
    /// 回傳：`io::Result<App>`。
    /// - 成功時回傳完整初始化的應用程式狀態。
    /// - 失敗時回傳建立第一個 pane 或載入目錄時的 I/O 錯誤。
    pub(crate) fn new(cwd: PathBuf, loaded_config: LoadedConfig) -> io::Result<Self> {
        let trash_store = TrashStore::new(&cwd)?;
        let LoadedConfig { config, source } = loaded_config;
        let bookmark_store = BookmarkStore::load(bookmark_file_path(&cwd, source.as_deref()))
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut pane = PaneState::new(cwd)?;
        apply_config_to_pane(&config, &mut pane);
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);
        let theme_preset = config.ui.theme_preset;
        let startup_status = match source {
            Some(path) => format!("loaded config: {}", path.display()),
            None => String::from("normal mode"),
        };

        Ok(Self {
            config,
            theme: theme_preset.into(),
            theme_preset,
            trash_store,
            bookmark_store,
            panes,
            layout: LayoutNode::Leaf { pane_id: 1 },
            focused_pane: 1,
            next_pane_id: 2,
            status: startup_status,
            command_mode: false,
            command_buffer: String::new(),
            awaiting_ctrl_w: false,
            pending_g: false,
            pending_y: false,
            pending_bookmark: None,
            clipboard: None,
            filter: None,
            preview_search: None,
            global_search: None,
            global_search_rx: None,
            global_search_cancelled: None,
            visual_selection: None,
            pending_action: None,
            help_return: None,
            pending_launch: None,
            preview_focus: None,
        })
    }

    /// 處理一般輸入事件的總入口。
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.code == KeyCode::F(1) {
            if matches!(self.pending_action, Some(PendingAction::HelpPanel { .. })) {
                return self.handle_pending_action_key(key);
            }
            self.open_help_from_current();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if self.pending_action.is_some() {
            return self.handle_pending_action_key(key);
        }
        if self.filter.as_ref().is_some_and(|filter| filter.editing) {
            return self.handle_filter_input_key(key);
        }
        if self
            .preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
        {
            return self.handle_preview_search_input_key(key);
        }
        if self.global_search.is_some() {
            return self.handle_global_search_key(key);
        }
        if self.visual_selection.is_some() {
            return self.handle_visual_selection_key(key);
        }
        if self.command_mode {
            return self.handle_command_key(key);
        }
        if self.awaiting_ctrl_w {
            self.awaiting_ctrl_w = false;
            return self.handle_ctrl_w(key);
        }
        if self.pending_bookmark.is_some() {
            return self.handle_bookmark_key(key);
        }
        if self.preview_focus == Some(self.focused_pane) {
            return self.handle_preview_key(key);
        }
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
            self.open_selected_with_picker()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'O') {
            self.open_selected_with_picker()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            self.current_pane_mut()?.move_bottom();
            self.pending_g = false;
            self.pending_y = false;
            self.status = String::from("jumped to bottom");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'V') {
            self.open_visual_selection()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'P') {
            self.open_preview_focus();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }

        let should_continue = match key.code {
            KeyCode::Char('q') => false,
            KeyCode::Char(':') | KeyCode::Char(';')
                if key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.command_mode = true;
                self.command_buffer.clear();
                self.status = String::from("command mode");
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = None;
                true
            }
            KeyCode::Char('j') => {
                self.current_pane_mut()?.move_down();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('k') => {
                self.current_pane_mut()?.move_up();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('h') => {
                self.current_pane_mut()?.go_parent()?;
                self.status = String::from("moved to parent directory");
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('l') => {
                self.current_pane_mut()?.enter_selected()?;
                self.status = String::from("opened directory");
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('o') | KeyCode::Enter => {
                self.open_selected_with_default()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('g') => {
                self.pending_y = false;
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
            KeyCode::Char('d') => {
                self.start_delete_confirmation();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('r') => {
                self.start_rename();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char(',') => {
                self.open_sort_picker();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('f') => {
                self.open_filter_input();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('s') => {
                self.open_global_search()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('.') => {
                self.toggle_hidden_files()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('a') => {
                self.start_create_entry();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('x') => {
                self.cut_selected();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('p') => {
                self.paste_into_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('y') => {
                self.pending_g = false;
                self.pending_bookmark = None;
                if self.pending_y {
                    self.copy_selected();
                    self.pending_y = false;
                } else {
                    self.pending_y = true;
                    self.status = String::from("pending: y");
                }
                true
            }
            KeyCode::Char('m') => {
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = Some(BookmarkPrompt::Set);
                self.status = String::from("bookmark: press a key to save current directory");
                true
            }
            KeyCode::Char('\'') => {
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = Some(BookmarkPrompt::Jump);
                self.status = String::from("bookmark: press a key to jump");
                true
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.awaiting_ctrl_w = true;
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = None;
                self.status = String::from("Ctrl-w");
                true
            }
            KeyCode::Esc => {
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = None;
                self.handle_escape_in_normal_mode();
                true
            }
            _ => {
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = None;
                true
            }
        };

        Ok(should_continue)
    }

    /// 處理 preview mode 的鍵盤輸入，讓使用者可以專心在預覽區捲動內容。
    pub(crate) fn handle_preview_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key_matches_shifted_letter(&key, 'P') {
            if self.clear_preview_search_if_active() {
                self.pending_g = false;
                return Ok(true);
            }
            self.preview_focus = None;
            self.pending_g = false;
            self.pending_y = false;
            self.status = String::from("normal mode");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'N') {
            self.pending_g = false;
            self.status = self.jump_preview_match(false)?;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            self.current_pane_mut()?.scroll_preview_bottom();
            self.pending_g = false;
            self.status = String::from("preview: bottom");
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc => {
                if self.clear_preview_search_if_active() {
                    self.pending_g = false;
                    return Ok(true);
                }
                self.preview_focus = None;
                self.pending_g = false;
                self.pending_y = false;
                self.status = String::from("normal mode");
            }
            KeyCode::Char('/') => {
                self.open_preview_search_input();
                self.pending_g = false;
            }
            KeyCode::Char('n') => {
                self.pending_g = false;
                self.status = self.jump_preview_match(true)?;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.current_pane_mut()?.scroll_preview_down(1);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.current_pane_mut()?.scroll_preview_up(1);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            KeyCode::Char('g') => {
                if self.pending_g {
                    self.current_pane_mut()?.scroll_preview_top();
                    self.pending_g = false;
                    self.status = String::from("preview: top");
                } else {
                    self.pending_g = true;
                    self.status = String::from("preview: pending g");
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.current_pane_mut()?.page_preview_down();
                self.pending_g = false;
                self.status = String::from("preview: page down");
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.current_pane_mut()?.page_preview_up();
                self.pending_g = false;
                self.status = String::from("preview: page up");
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.awaiting_ctrl_w = true;
                self.pending_g = false;
                self.pending_y = false;
                self.status = String::from("Ctrl-w");
            }
            _ => {
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
        }

        Ok(true)
    }

    /// 處理 visual selection 模式下的鍵盤輸入。
    pub(crate) fn handle_visual_selection_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key_matches_shifted_letter(&key, 'V') {
            self.commit_visual_selection()?;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            self.current_pane_mut()?.move_bottom();
            self.sync_visual_selection_cursor();
            self.pending_g = false;
            self.status = self.visual_status_label();
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc => {
                self.commit_visual_selection()?;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.current_pane_mut()?.move_down();
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.current_pane_mut()?.move_up();
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            KeyCode::Char('g') => {
                if self.pending_g {
                    self.current_pane_mut()?.move_top();
                    self.sync_visual_selection_cursor();
                    self.pending_g = false;
                    self.status = self.visual_status_label();
                } else {
                    self.pending_g = true;
                    self.status = String::from("visual: pending g");
                }
            }
            _ => {
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
        }

        Ok(true)
    }

    /// 處理 preview search 輸入框中的鍵盤輸入，並在每次輸入後立即更新命中位置。
    pub(crate) fn handle_preview_search_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.preview_search.take() else {
            return Ok(true);
        };

        match key.code {
            KeyCode::Char(c) => {
                search.buffer.push(c);
                self.apply_preview_search_buffer(&search);
                self.status =
                    preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
                self.preview_search = Some(search);
            }
            KeyCode::Backspace => {
                search.buffer.pop();
                self.apply_preview_search_buffer(&search);
                self.status =
                    preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
                self.preview_search = Some(search);
            }
            KeyCode::Esc | KeyCode::Enter => {
                search.editing = false;
                self.status = if search.buffer.is_empty() {
                    String::from("preview mode")
                } else {
                    format!(
                        "preview search locked: {} ({})",
                        search.buffer,
                        self.preview_match_count(search.pane_id)
                    )
                };
                self.preview_search = Some(search);
            }
            _ => {
                self.preview_search = Some(search);
            }
        }

        Ok(true)
    }

    /// 處理 global search 面板中的輸入、結果瀏覽與跳轉。
    pub(crate) fn handle_global_search_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.global_search.take() else {
            return Ok(true);
        };

        if search.editing {
            match key.code {
                KeyCode::Char(c) => {
                    search.buffer.push(c);
                    search.searched = false;
                    search.loading = false;
                    search.selected = 0;
                    search.results.clear();
                }
                KeyCode::Backspace => {
                    search.buffer.pop();
                    search.searched = false;
                    search.loading = false;
                    search.selected = 0;
                    search.results.clear();
                }
                KeyCode::Enter => {
                    self.start_global_search(&mut search)?;
                    search.editing = false;
                }
                KeyCode::Esc => {
                    self.cancel_global_search();
                }
                _ => {}
            }

            if !matches!(key.code, KeyCode::Esc) {
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            return Ok(true);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                search.selected = (search.selected + 1).min(search.results.len().saturating_sub(1));
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                search.selected = search.selected.saturating_sub(1);
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            KeyCode::Char('g') => {
                if self.pending_g {
                    search.selected = 0;
                    self.pending_g = false;
                } else {
                    self.pending_g = true;
                }
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            KeyCode::Char('G') => {
                if !search.results.is_empty() {
                    search.selected = search.results.len() - 1;
                }
                self.pending_g = false;
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            KeyCode::Char('i') | KeyCode::Char('s') => {
                search.editing = true;
                self.pending_g = false;
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    true,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                self.pending_g = false;
                self.open_global_search_result(search)?;
            }
            KeyCode::Esc | KeyCode::Char('h') => {
                self.pending_g = false;
                self.cancel_global_search();
            }
            _ => {
                self.pending_g = false;
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
        }

        Ok(true)
    }

    /// 處理 filter 輸入框中的鍵盤輸入，並在每次輸入後立即更新列表。
    pub(crate) fn handle_filter_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut filter) = self.filter.take() else {
            return Ok(true);
        };

        match key.code {
            KeyCode::Char(c) => {
                filter.buffer.push(c);
                self.apply_filter_buffer(&filter);
                self.status = if filter.buffer.is_empty() {
                    String::from("filter: all")
                } else {
                    format!("filter: {}", filter.buffer)
                };
                self.filter = Some(filter);
            }
            KeyCode::Backspace => {
                filter.buffer.pop();
                self.apply_filter_buffer(&filter);
                self.status = if filter.buffer.is_empty() {
                    String::from("filter: all")
                } else {
                    format!("filter: {}", filter.buffer)
                };
                self.filter = Some(filter);
            }
            KeyCode::Esc | KeyCode::Enter => {
                filter.editing = false;
                self.status = if filter.buffer.is_empty() {
                    String::from("filter active")
                } else {
                    format!("filter locked: {}", filter.buffer)
                };
                self.filter = Some(filter);
            }
            _ => {
                self.filter = Some(filter);
            }
        }

        Ok(true)
    }

    /// 處理暫時互動視窗的按鍵事件。
    pub(crate) fn handle_pending_action_key(&mut self, key: KeyEvent) -> Result<bool> {
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
                    self.status = format!("trash cancelled: {target_name}");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmDelete {
                        pane_id,
                        target_name: target_name.clone(),
                    });
                    self.status = format!("confirm trash {target_name}: y/n");
                }
            },
            PendingAction::SortPicker { pane_id } => match key.code {
                _ if key_matches_shifted_letter(&key, 'M') => {
                    self.apply_sort_mode(pane_id, SortMode::Modified { reverse: true })?
                }
                KeyCode::Char('m') => {
                    self.apply_sort_mode(pane_id, SortMode::Modified { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'B') => {
                    self.apply_sort_mode(pane_id, SortMode::Created { reverse: true })?
                }
                KeyCode::Char('b') => {
                    self.apply_sort_mode(pane_id, SortMode::Created { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'A') => {
                    self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: true })?
                }
                KeyCode::Char('a') => {
                    self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'N') => {
                    self.apply_sort_mode(pane_id, SortMode::Natural { reverse: true })?
                }
                KeyCode::Char('n') => {
                    self.apply_sort_mode(pane_id, SortMode::Natural { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'E') => {
                    self.apply_sort_mode(pane_id, SortMode::Extension { reverse: true })?
                }
                KeyCode::Char('e') => {
                    self.apply_sort_mode(pane_id, SortMode::Extension { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'S') => {
                    self.apply_sort_mode(pane_id, SortMode::Size { reverse: true })?
                }
                KeyCode::Char('s') => {
                    self.apply_sort_mode(pane_id, SortMode::Size { reverse: false })?
                }
                KeyCode::Char('r') => self.apply_sort_mode(pane_id, SortMode::Random)?,
                KeyCode::Esc => {
                    self.status = String::from("sort cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::SortPicker { pane_id });
                    self.status = String::from("sort: choose a key from the panel");
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
                KeyCode::Enter => self.apply_theme(ThemePreset::ALL[selected]),
                KeyCode::Esc => {
                    self.status = String::from("theme picker cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ThemePicker { selected });
                    self.status = String::from("theme picker: use j/k and Enter");
                }
            },
            PendingAction::TrashPanel {
                pane_id,
                mut selected,
                mut search,
                mut marked_ids,
                mut visual_anchor,
            } => {
                let entries = trash_panel_entries(&self.trash_store, &search.buffer)?;
                let len = entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Char(c) => {
                            search.buffer.push(c);
                            selected = 0;
                        }
                        KeyCode::Backspace => {
                            search.buffer.pop();
                            selected = 0;
                        }
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
                    let status = trash_panel_status(
                        &search.buffer,
                        next_len,
                        selected,
                        search.editing,
                        marked_ids.len(),
                    );
                    self.pending_action = Some(PendingAction::TrashPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    self.status = status;
                } else {
                    if key_matches_shifted_letter(&key, 'G') {
                        if len > 0 {
                            selected = len - 1;
                        }
                        self.pending_g = false;
                        let search_buffer = search.buffer.clone();
                        let search_editing = search.editing;
                        let marked_count = marked_ids.len();
                        self.pending_action = Some(PendingAction::TrashPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        self.status = trash_panel_status(
                            &search_buffer,
                            len,
                            selected,
                            search_editing,
                            marked_count,
                        );
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'V') {
                        self.pending_g = false;
                        if let Some(anchor) = visual_anchor.take() {
                            let added = self.commit_trash_visual_selection(
                                &entries,
                                &mut marked_ids,
                                anchor,
                                selected,
                            );
                            self.status = if added == 0 {
                                format!("trash: kept {} marked items", marked_ids.len())
                            } else {
                                format!("trash: marked {} items", marked_ids.len())
                            };
                        } else if len > 0 {
                            visual_anchor = Some(selected);
                            self.status = self.trash_visual_status_label(
                                selected,
                                selected,
                                marked_ids.len(),
                            );
                        }
                        self.pending_action = Some(PendingAction::TrashPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'R') {
                        self.pending_g = false;
                        self.restore_all_trash_entries(&entries)?;
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'D') {
                        self.pending_g = false;
                        let target_ids =
                            self.selected_or_marked_trash_ids(&entries, selected, &marked_ids);
                        if target_ids.len() <= 1 {
                            self.delete_trash_entry(
                                pane_id,
                                &entries,
                                selected,
                                search,
                                marked_ids,
                                visual_anchor,
                            )?;
                        } else {
                            let selected_entries = entries
                                .iter()
                                .filter(|entry| target_ids.iter().any(|id| id == &entry.id))
                                .cloned()
                                .collect::<Vec<_>>();
                            self.clear_filtered_trash_entries(
                                pane_id,
                                &selected_entries,
                                search,
                                Vec::new(),
                                None,
                            )?;
                        }
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'C') {
                        self.pending_g = false;
                        self.clear_filtered_trash_entries(
                            pane_id,
                            &entries,
                            search,
                            marked_ids,
                            visual_anchor,
                        )?;
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            if len > 0 {
                                selected = (selected + 1).min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            selected = selected.saturating_sub(1);
                            self.pending_g = false;
                        }
                        KeyCode::Char('g') => {
                            if self.pending_g {
                                selected = 0;
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Char('f') => {
                            search.editing = true;
                            self.pending_g = false;
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if len > 0 {
                                selected = (selected + 10).min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            selected = selected.saturating_sub(10);
                            self.pending_g = false;
                        }
                        KeyCode::Enter | KeyCode::Char('l') => {
                            self.pending_g = false;
                            let target_ids =
                                self.selected_or_marked_trash_ids(&entries, selected, &marked_ids);
                            if target_ids.len() <= 1 {
                                self.restore_trash_entry(&entries, selected)?;
                            } else {
                                let selected_entries = entries
                                    .iter()
                                    .filter(|entry| target_ids.iter().any(|id| id == &entry.id))
                                    .cloned()
                                    .collect::<Vec<_>>();
                                self.restore_all_trash_entries(&selected_entries)?;
                            }
                            return Ok(true);
                        }
                        KeyCode::Esc => {
                            self.pending_g = false;
                            if let Some(anchor) = visual_anchor.take() {
                                let added = self.commit_trash_visual_selection(
                                    &entries,
                                    &mut marked_ids,
                                    anchor,
                                    selected,
                                );
                                self.status = if added == 0 {
                                    format!("trash: kept {} marked items", marked_ids.len())
                                } else {
                                    format!("trash: marked {} items", marked_ids.len())
                                };
                            } else if !marked_ids.is_empty() {
                                let cleared = marked_ids.len();
                                marked_ids.clear();
                                self.status = format!("trash: cleared {cleared} marks");
                            } else {
                                self.status = String::from("normal mode");
                                return Ok(true);
                            }
                        }
                        KeyCode::Char('q') | KeyCode::Char('h') => {
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ => {
                            self.pending_g = false;
                        }
                    }
                    let status = if let Some(anchor) = visual_anchor {
                        self.trash_visual_status_label(selected, anchor, marked_ids.len())
                    } else {
                        trash_panel_status(
                            &search.buffer,
                            len,
                            selected,
                            search.editing,
                            marked_ids.len(),
                        )
                    };
                    self.pending_action = Some(PendingAction::TrashPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    self.status = status;
                }
            }
            PendingAction::HelpPanel {
                pane_id,
                mut selected,
                mut search,
            } => {
                let filtered_entries = help_entries(&search.buffer);
                let filtered_len = filtered_entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Char(c) => {
                            search.buffer.push(c);
                            selected = 0;
                        }
                        KeyCode::Backspace => {
                            search.buffer.pop();
                            selected = 0;
                        }
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = help_entries(&search.buffer).len();
                    let status = help_panel_status(&search.buffer, next_len, search.editing);
                    self.pending_action = Some(PendingAction::HelpPanel {
                        pane_id,
                        selected,
                        search,
                    });
                    self.status = status;
                } else {
                    if key_matches_shifted_letter(&key, 'G') {
                        if filtered_len > 0 {
                            selected = filtered_len - 1;
                        }
                        self.pending_g = false;
                        let status = help_panel_status(&search.buffer, filtered_len, false);
                        self.pending_action = Some(PendingAction::HelpPanel {
                            pane_id,
                            selected,
                            search,
                        });
                        self.status = status;
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            if filtered_len > 0 {
                                selected = (selected + 1).min(filtered_len.saturating_sub(1));
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            selected = selected.saturating_sub(1);
                        }
                        KeyCode::Char('g') => {
                            if self.pending_g {
                                selected = 0;
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Char('f') => {
                            search.editing = true;
                            self.pending_g = false;
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if filtered_len > 0 {
                                selected = (selected + 10).min(filtered_len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            selected = selected.saturating_sub(10);
                            self.pending_g = false;
                        }
                        KeyCode::Enter | KeyCode::Char('l') => {
                            self.pending_g = false;
                            self.execute_help_entry(&filtered_entries, selected)?;
                            return Ok(true);
                        }
                        KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('q') => {
                            self.pending_g = false;
                            self.restore_help_return_state(false)?;
                            return Ok(true);
                        }
                        _ => {
                            self.pending_g = false;
                        }
                    }
                    let next_count = help_entries(&search.buffer).len();
                    let status = help_panel_status(&search.buffer, next_count, false);
                    self.pending_action = Some(PendingAction::HelpPanel {
                        pane_id,
                        selected,
                        search,
                    });
                    self.status = status;
                }
            }
            PendingAction::BookmarkList {
                pane_id,
                mut selected,
            } => {
                let entries = self.bookmark_store.list();
                let len = entries.len();
                if key_matches_shifted_letter(&key, 'G') {
                    if len > 0 {
                        selected = len - 1;
                    }
                    self.pending_g = false;
                    self.pending_action = Some(PendingAction::BookmarkList { pane_id, selected });
                    self.status = bookmark_list_status(len, selected);
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        if len > 0 {
                            selected = (selected + 1).min(len.saturating_sub(1));
                        }
                        self.pending_g = false;
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        self.pending_g = false;
                    }
                    KeyCode::Char('g') => {
                        if self.pending_g {
                            selected = 0;
                            self.pending_g = false;
                        } else {
                            self.pending_g = true;
                        }
                    }
                    KeyCode::Enter | KeyCode::Char('l') => {
                        self.pending_g = false;
                        self.open_bookmark_from_list(pane_id, &entries, selected)?;
                        return Ok(true);
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => {
                        self.pending_g = false;
                        self.status = String::from("normal mode");
                        return Ok(true);
                    }
                    _ => {
                        self.pending_g = false;
                    }
                }
                self.pending_action = Some(PendingAction::BookmarkList { pane_id, selected });
                self.status = bookmark_list_status(len, selected);
            }
            PendingAction::OpenPicker {
                pane_id,
                target,
                mut selected,
            } => {
                let options = open_picker_options(&target);
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        if !options.is_empty() {
                            selected = (selected + 1).min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    KeyCode::Enter | KeyCode::Char('l') => {
                        if let Some(option) = options.get(selected) {
                            self.queue_open_action(target.clone(), option.action)?;
                        } else {
                            self.status = String::from("open with: no option selected");
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => {
                        self.status = String::from("normal mode");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                }
            }
            PendingAction::Rename {
                pane_id,
                original_name,
                mut buffer,
                mut cursor,
                mut mode,
            } => match mode {
                RenameMode::Insert => match key.code {
                    KeyCode::Char(c) => {
                        insert_char(&mut buffer, &mut cursor, c);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Backspace => {
                        backspace_char(&mut buffer, &mut cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Enter => {
                        self.confirm_rename(pane_id, &original_name, &buffer)?;
                    }
                    KeyCode::Esc => {
                        mode = RenameMode::Normal;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: normal");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
                RenameMode::Normal if key_matches_shifted_letter(&key, 'A') => {
                    cursor = buffer.chars().count();
                    mode = RenameMode::Insert;
                    self.pending_action = Some(PendingAction::Rename {
                        pane_id,
                        original_name,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = String::from("rename: insert");
                }
                RenameMode::Normal => match key.code {
                    KeyCode::Char('h') | KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('w') => {
                        cursor = rename_next_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('b') => {
                        cursor = rename_previous_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('e') => {
                        cursor = rename_word_end(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('i') => {
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Char('a') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Enter => {
                        self.confirm_rename(pane_id, &original_name, &buffer)?;
                    }
                    KeyCode::Esc => {
                        self.status = format!("rename cancelled: {original_name}");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
            },
            PendingAction::CreateEntry {
                pane_id,
                mut buffer,
                mut cursor,
                mut mode,
            } => match mode {
                RenameMode::Insert => match key.code {
                    KeyCode::Char(c) => {
                        insert_char(&mut buffer, &mut cursor, c);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Backspace => {
                        backspace_char(&mut buffer, &mut cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Enter => {
                        self.confirm_create_entry(pane_id, &buffer)?;
                    }
                    KeyCode::Esc => {
                        mode = RenameMode::Normal;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("normal");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
                RenameMode::Normal if key_matches_shifted_letter(&key, 'A') => {
                    cursor = buffer.chars().count();
                    mode = RenameMode::Insert;
                    self.pending_action = Some(PendingAction::CreateEntry {
                        pane_id,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = create_status_label("insert");
                }
                RenameMode::Normal => match key.code {
                    KeyCode::Char('h') | KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('w') => {
                        cursor = rename_next_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('b') => {
                        cursor = rename_previous_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('e') => {
                        cursor = rename_word_end(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('i') => {
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Char('a') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Enter => {
                        self.confirm_create_entry(pane_id, &buffer)?;
                    }
                    KeyCode::Esc => {
                        self.status = String::from("create cancelled");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
            },
        }

        Ok(true)
    }

    /// 處理 command mode 中的按鍵編輯與送出行為。
    pub(crate) fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
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
            KeyCode::Char(c) => self.command_buffer.push(c),
            _ => {}
        }
        Ok(true)
    }

    /// 在一般模式按下 `Esc` 時，優先處理 filter 的兩段式離開流程。
    fn handle_escape_in_normal_mode(&mut self) {
        if let Some(filter) = self.filter.take() {
            if filter.editing {
                self.filter = Some(FilterState {
                    editing: false,
                    ..filter
                });
                self.status = String::from("filter active");
            } else {
                if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
                    pane.clear_filter();
                }
                self.status = String::from("normal mode");
            }
            return;
        }

        if self.has_any_marks() {
            self.clear_all_marks();
            return;
        }

        self.status = String::from("normal mode");
    }

    /// 處理 `Ctrl-w` 前綴後的 pane 操作命令。
    pub(crate) fn handle_ctrl_w(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('h') | KeyCode::Char('k') => self.focus_previous_pane(),
            KeyCode::Char('l') | KeyCode::Char('j') => self.focus_next_pane(),
            KeyCode::Char('v') => self.split_current(SplitDirection::Vertical)?,
            KeyCode::Char('s') => self.split_current(SplitDirection::Horizontal)?,
            KeyCode::Char('c') => self.close_current_pane(),
            KeyCode::Char('o') => self.only_current_pane(),
            _ => self.status = String::from("unknown Ctrl-w command"),
        }
        Ok(true)
    }

    /// 執行 command mode 送出的命令字串。
    pub(crate) fn execute_command(&mut self, command: &str) -> Result<()> {
        match command {
            "q" => self.status = String::from("use q in normal mode to quit"),
            "rename" => self.start_rename(),
            "create" => self.start_create_entry(),
            "copy" => self.copy_selected(),
            "cut" => self.cut_selected(),
            "paste" => self.paste_into_focused_pane()?,
            "open" => self.open_selected_with_default()?,
            "open-picker" => self.open_selected_with_picker()?,
            "vim" => {
                let Some(target) = self.selected_open_target() else {
                    self.status = String::from("nothing selected to open");
                    return Ok(());
                };
                self.queue_open_action(target, OpenAction::Vim)?;
            }
            "reveal" => {
                let Some(target) = self.selected_open_target() else {
                    self.status = String::from("nothing selected to reveal");
                    return Ok(());
                };
                self.queue_open_action(target, OpenAction::Reveal)?;
            }
            "unmark" | "unmark-all" => self.clear_marks_in_focused_pane()?,
            "preview" => self.open_preview_focus(),
            "preview-search" => self.open_preview_search_input(),
            "search" => self.open_global_search()?,
            "trash" => self.open_trash_panel()?,
            "help" => self.open_help_panel(),
            "bookmark list" => self.open_bookmark_list(),
            "restore" => self.restore_latest_from_trash()?,
            "trash clear" => self.clear_trash()?,
            "trash restore-all" => self.restore_all_from_trash()?,
            "theme" => self.open_theme_picker(),
            "theme next" => self.cycle_theme(),
            "split" => self.split_current(SplitDirection::Horizontal)?,
            "vsplit" => self.split_current(SplitDirection::Vertical)?,
            "close" => self.close_current_pane(),
            "only" => self.only_current_pane(),
            "" => self.status = String::from("normal mode"),
            other => {
                if let Some(name) = other.strip_prefix("theme ") {
                    self.set_theme_by_name(name.trim());
                } else if let Some(name) = other.strip_prefix("create ") {
                    self.create_entry_from_command(name)?;
                } else if let Some(args) = other.strip_prefix("move-panel ") {
                    self.move_selected_to_pane_id(args.trim())?;
                } else if let Some(path) = other.strip_prefix("move ") {
                    self.move_selected_to_path(path.trim())?;
                } else if let Some(args) = other.strip_prefix("bookmark set ") {
                    self.set_bookmark_from_command(args.trim())?;
                } else if let Some(args) = other.strip_prefix("bookmark jump ") {
                    self.jump_to_bookmark_from_command(args.trim())?;
                } else {
                    self.status = format!("unknown command: {other}");
                }
            }
        }
        Ok(())
    }

    /// 取得目前有焦點的 pane 可變參考。
    pub(crate) fn current_pane_mut(&mut self) -> io::Result<&mut PaneState> {
        self.panes
            .get_mut(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused pane"))
    }

    /// 將目前焦點 pane 依指定方向分割成兩個 pane。
    pub(crate) fn split_current(&mut self, direction: SplitDirection) -> io::Result<()> {
        let source_pane = self
            .panes
            .get(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused pane"))?;
        let cwd = source_pane.cwd.clone();
        let show_hidden = source_pane.show_hidden;
        let sort_mode = source_pane.sort_mode;

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let mut pane = PaneState::new(cwd)?;
        pane.set_show_hidden(show_hidden);
        pane.set_sort_mode(sort_mode);
        self.panes.insert(new_id, pane);
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

    /// 依照目前布局順序取得所有 pane id。
    pub(crate) fn ordered_pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.layout.pane_ids(&mut ids);
        ids
    }

    /// 將焦點切換到下一個 pane。
    pub(crate) fn focus_next_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if let Some(index) = ids.iter().position(|id| *id == self.focused_pane) {
            self.focused_pane = ids[(index + 1) % ids.len()];
            self.status = format!("focused pane {}", self.focused_pane);
        }
    }

    /// 將焦點切換到上一個 pane。
    pub(crate) fn focus_previous_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if let Some(index) = ids.iter().position(|id| *id == self.focused_pane) {
            self.focused_pane = ids[(index + ids.len() - 1) % ids.len()];
            self.status = format!("focused pane {}", self.focused_pane);
        }
    }

    /// 關閉目前有焦點的 pane。
    pub(crate) fn close_current_pane(&mut self) {
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
                if self.preview_focus == Some(old_focus) {
                    self.preview_focus = None;
                }
                if self
                    .global_search
                    .as_ref()
                    .is_some_and(|search| search.pane_id == old_focus)
                {
                    self.global_search = None;
                }
                self.focused_pane = fallback;
                self.status = format!("closed pane {old_focus}");
            }
        }
    }

    /// 僅保留目前有焦點的 pane，其餘全部關閉。
    pub(crate) fn only_current_pane(&mut self) {
        let focused = self.focused_pane;
        self.panes.retain(|id, _| *id == focused);
        self.layout = LayoutNode::Leaf { pane_id: focused };
        if self.preview_focus != Some(focused) {
            self.preview_focus = None;
        }
        if self
            .global_search
            .as_ref()
            .is_some_and(|search| search.pane_id != focused)
        {
            self.global_search = None;
        }
        self.status = String::from("kept only focused pane");
    }

    /// 將主題切換到下一個內建預設值。
    pub(crate) fn cycle_theme(&mut self) {
        let next = self.theme_preset.next();
        self.apply_theme(next);
    }

    /// 打開主題選擇視窗，並將選項焦點設在目前主題。
    pub(crate) fn open_theme_picker(&mut self) {
        let selected = ThemePreset::ALL
            .iter()
            .position(|preset| *preset == self.theme_preset)
            .unwrap_or(0);
        self.pending_action = Some(PendingAction::ThemePicker { selected });
        self.status = String::from("theme picker: use j/k and Enter");
    }

    /// 打開底部排序面板，等待使用者輸入排序快捷鍵。
    pub(crate) fn open_sort_picker(&mut self) {
        self.pending_action = Some(PendingAction::SortPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("sort: choose a key from the panel");
    }

    /// 打開 trash 面板，列出目前可還原的項目。
    pub(crate) fn open_trash_panel(&mut self) -> io::Result<()> {
        self.pending_action = Some(PendingAction::TrashPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
            marked_ids: Vec::new(),
            visual_anchor: None,
        });
        self.help_return = None;
        self.status = trash_panel_status("", self.trash_store.list_entries()?.len(), 0, false, 0);
        Ok(())
    }

    /// 打開 F1 功能說明面板，支援 Vim 式滾動與面板內搜尋。
    pub(crate) fn open_help_panel(&mut self) {
        self.help_return = None;
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 打開書籤列表彈窗，讓使用者可以用列表方式查看與跳轉書籤。
    pub(crate) fn open_bookmark_list(&mut self) {
        self.pending_action = Some(PendingAction::BookmarkList {
            pane_id: self.focused_pane,
            selected: 0,
        });
        self.status = bookmark_list_status(self.bookmark_store.list().len(), 0);
    }

    /// 處理等待書籤按鍵時的輸入。
    pub(crate) fn handle_bookmark_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(prompt) = self.pending_bookmark.take() else {
            return Ok(true);
        };

        match key.code {
            KeyCode::Esc => {
                self.status = String::from("normal mode");
            }
            KeyCode::Char(bookmark) => match prompt {
                BookmarkPrompt::Set => self.set_bookmark(bookmark)?,
                BookmarkPrompt::Jump => self.jump_to_bookmark(bookmark)?,
            },
            _ => {
                self.pending_bookmark = Some(prompt);
                self.status = match prompt {
                    BookmarkPrompt::Set => String::from("bookmark: use a single character key"),
                    BookmarkPrompt::Jump => String::from("bookmark: choose an existing key"),
                };
            }
        }

        Ok(true)
    }

    /// 將目前焦點 pane 的目錄存成書籤。
    fn set_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };
        self.bookmark_store
            .set(key, pane.cwd.clone())
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.status = format!("bookmark [{key}] = {}", pane.cwd.display());
        Ok(())
    }

    /// 跳到指定書籤對應的路徑。
    fn jump_to_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(path) = self.bookmark_store.get(key).cloned() else {
            self.status = format!("bookmark [{key}] not found");
            return Ok(());
        };

        let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        if !path.exists() {
            self.status = format!("bookmark [{key}] missing: {}", path.display());
            return Ok(());
        }

        pane.go_to_path(&path)?;
        self.status = format!("jumped to bookmark [{key}]");
        Ok(())
    }

    /// 讓 `:bookmark set <key>` 可以直接把目前目錄存成指定書籤。
    fn set_bookmark_from_command(&mut self, args: &str) -> io::Result<()> {
        let Some(key) = parse_bookmark_argument(args) else {
            self.status = String::from("usage: bookmark set <key>");
            return Ok(());
        };
        self.set_bookmark(key)
    }

    /// 讓 `:bookmark jump <key>` 可以直接跳到指定書籤。
    fn jump_to_bookmark_from_command(&mut self, args: &str) -> io::Result<()> {
        let Some(key) = parse_bookmark_argument(args) else {
            self.status = String::from("usage: bookmark jump <key>");
            return Ok(());
        };
        self.jump_to_bookmark(key)
    }

    /// 從書籤列表彈窗中打開目前選取的書籤。
    fn open_bookmark_from_list(
        &mut self,
        pane_id: usize,
        entries: &[BookmarkEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("bookmark list: empty");
            return Ok(());
        };
        let Some(path) = self.bookmark_store.get(entry.key).cloned() else {
            self.status = format!("bookmark [{}] not found", entry.key);
            return Ok(());
        };
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };
        if !path.exists() {
            self.status = format!("bookmark [{}] missing: {}", entry.key, path.display());
            return Ok(());
        }
        pane.go_to_path(&path)?;
        self.focused_pane = pane_id;
        self.status = format!("jumped to bookmark [{}]", entry.key);
        Ok(())
    }

    /// 取得目前選取項目的外部開啟目標資訊。
    fn selected_open_target(&self) -> Option<OpenTarget> {
        let pane = self.panes.get(&self.focused_pane)?;
        let entry = pane.selected_entry()?;
        Some(OpenTarget {
            path: entry.path.clone(),
            display_name: entry.display_name(),
            is_dir: entry.is_dir,
        })
    }

    /// 使用預設動作開啟目前選取項目。
    pub(crate) fn open_selected_with_default(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to open");
            return Ok(());
        };
        self.queue_open_action(target, default_open_action())
    }

    /// 打開 `Open with` 面板，讓使用者選擇外部開啟方式。
    pub(crate) fn open_selected_with_picker(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to open");
            return Ok(());
        };

        self.pending_action = Some(PendingAction::OpenPicker {
            pane_id: self.focused_pane,
            target: target.clone(),
            selected: 0,
        });
        self.status = format!("open with: {}", target.display_name);
        Ok(())
    }

    /// 將外部開啟動作排入待執行佇列。
    fn queue_open_action(&mut self, target: OpenTarget, action: OpenAction) -> io::Result<()> {
        let launch = build_launch_spec(&target, action)?;
        self.pending_launch = Some(launch);
        self.status = match action {
            OpenAction::Editor => format!("opening {} with editor", target.display_name),
            OpenAction::Vim => format!("opening {} with vim", target.display_name),
            OpenAction::Open => format!("opening {}", target.display_name),
            OpenAction::Reveal => format!("revealing {}", target.display_name),
        };
        Ok(())
    }

    /// 取出目前排隊中的外部開啟請求，交給主事件迴圈處理。
    pub(crate) fn take_pending_launch(&mut self) -> Option<LaunchSpec> {
        self.pending_launch.take()
    }

    /// 以目前正在操作的上下文為返回點，打開 help 面板。
    pub(crate) fn open_help_from_current(&mut self) {
        self.help_return = self.capture_help_return_state();
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 擷取目前互動狀態，供 help 面板關閉後回復。
    fn capture_help_return_state(&mut self) -> Option<HelpReturnState> {
        if let Some(action) = self.pending_action.take() {
            return Some(HelpReturnState::Pending(action));
        }
        if let Some(filter) = self.filter.take() {
            return Some(HelpReturnState::Filter(filter));
        }
        if let Some(search) = self.preview_search.take() {
            return Some(HelpReturnState::PreviewSearch(search));
        }
        if let Some(search) = self.global_search.take() {
            self.cancel_global_search();
            return Some(HelpReturnState::GlobalSearch(search));
        }
        if let Some(selection) = self.visual_selection.take() {
            return Some(HelpReturnState::VisualSelection(selection));
        }
        if self.command_mode {
            self.command_mode = false;
            return Some(HelpReturnState::CommandMode(std::mem::take(
                &mut self.command_buffer,
            )));
        }
        if self.awaiting_ctrl_w {
            self.awaiting_ctrl_w = false;
            return Some(HelpReturnState::AwaitingCtrlW);
        }
        if let Some(prompt) = self.pending_bookmark.take() {
            return Some(HelpReturnState::PendingBookmark(prompt));
        }
        if let Some(pane_id) = self.preview_focus {
            return Some(HelpReturnState::PreviewFocus(pane_id));
        }
        None
    }

    /// 從 help 面板回到先前的互動上下文。
    fn restore_help_return_state(&mut self, preserve_status: bool) -> io::Result<()> {
        let previous_status = self.status.clone();
        let Some(state) = self.help_return.take() else {
            self.status = String::from("normal mode");
            return Ok(());
        };

        match state {
            HelpReturnState::Pending(action) => {
                self.status = self.status_for_pending_action(&action)?;
                self.pending_action = Some(action);
            }
            HelpReturnState::Filter(filter) => {
                self.status = if filter.editing {
                    if filter.buffer.is_empty() {
                        String::from("filter: all")
                    } else {
                        format!("filter: {}", filter.buffer)
                    }
                } else if filter.buffer.is_empty() {
                    String::from("filter active")
                } else {
                    format!("filter locked: {}", filter.buffer)
                };
                self.filter = Some(filter);
            }
            HelpReturnState::PreviewSearch(search) => {
                self.status =
                    preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
                self.preview_search = Some(search);
            }
            HelpReturnState::GlobalSearch(search) => {
                self.status = global_search_status(
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            HelpReturnState::VisualSelection(selection) => {
                self.visual_selection = Some(selection);
                self.status = self.visual_status_label();
            }
            HelpReturnState::CommandMode(buffer) => {
                self.command_mode = true;
                self.command_buffer = buffer;
                self.status = String::from("command mode");
            }
            HelpReturnState::AwaitingCtrlW => {
                self.awaiting_ctrl_w = true;
                self.status = String::from("Ctrl-w");
            }
            HelpReturnState::PendingBookmark(prompt) => {
                self.pending_bookmark = Some(prompt);
                self.status = match prompt {
                    BookmarkPrompt::Set => {
                        String::from("bookmark: press a key to save current directory")
                    }
                    BookmarkPrompt::Jump => String::from("bookmark: press a key to jump"),
                };
            }
            HelpReturnState::PreviewFocus(pane_id) => {
                self.preview_focus = Some(pane_id);
                self.status = String::from("preview mode");
            }
        }

        if preserve_status {
            self.status = previous_status;
        }

        Ok(())
    }

    /// 依 pending action 類型回傳適合顯示的狀態文字。
    fn status_for_pending_action(&self, action: &PendingAction) -> io::Result<String> {
        Ok(match action {
            PendingAction::ConfirmDelete { target_name, .. } => {
                format!("confirm trash {target_name}: y/n")
            }
            PendingAction::SortPicker { .. } => String::from("sort: choose a key from the panel"),
            PendingAction::ThemePicker { selected } => {
                format!("theme picker: {}", ThemePreset::ALL[*selected].name())
            }
            PendingAction::TrashPanel {
                selected,
                search,
                marked_ids,
                ..
            } => {
                let visible = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
                trash_panel_status(
                    &search.buffer,
                    visible,
                    *selected,
                    search.editing,
                    marked_ids.len(),
                )
            }
            PendingAction::HelpPanel { search, .. } => help_panel_status(
                &search.buffer,
                help_entries(&search.buffer).len(),
                search.editing,
            ),
            PendingAction::BookmarkList { selected, .. } => {
                bookmark_list_status(self.bookmark_store.list().len(), *selected)
            }
            PendingAction::OpenPicker { target, .. } => {
                format!("open with: {}", target.display_name)
            }
            PendingAction::Rename { mode, .. } => match mode {
                RenameMode::Insert => String::from("rename: insert"),
                RenameMode::Normal => String::from("rename: normal"),
            },
            PendingAction::CreateEntry { mode, .. } => match mode {
                RenameMode::Insert => create_status_label("insert"),
                RenameMode::Normal => create_status_label("normal"),
            },
        })
    }

    /// 依照主題名稱字串套用指定主題。
    pub(crate) fn set_theme_by_name(&mut self, name: &str) {
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

    /// 直接套用指定的主題預設值。
    pub(crate) fn apply_theme(&mut self, preset: ThemePreset) {
        self.theme_preset = preset;
        self.theme = preset.into();
        self.status = format!("theme: {}", preset.name());
    }

    /// 開始重新命名流程，建立一個待輸入的新名稱互動。
    pub(crate) fn start_rename(&mut self) {
        let Some(entry) = self
            .panes
            .get(&self.focused_pane)
            .and_then(PaneState::selected_entry)
            .cloned()
        else {
            self.status = String::from("nothing selected to rename");
            return;
        };

        self.pending_action = Some(PendingAction::Rename {
            pane_id: self.focused_pane,
            original_name: entry.display_name(),
            cursor: rename_basename_cursor(&entry.name),
            buffer: entry.name,
            mode: RenameMode::Insert,
        });
        self.status = String::from("rename: insert");
    }

    /// 開始建立新檔案流程，打開一個可直接輸入名稱的 inline 編輯器。
    ///
    /// 參數：無。
    /// 回傳：`()`
    /// 打開一個可直接輸入建立路徑的 inline 編輯器。
    ///
    /// 回傳：`()`
    pub(crate) fn start_create_entry(&mut self) {
        self.pending_action = Some(PendingAction::CreateEntry {
            pane_id: self.focused_pane,
            buffer: String::new(),
            cursor: 0,
            mode: RenameMode::Insert,
        });
        self.status = create_status_label("insert");
    }

    /// 打開 filter 輸入框，並以目前焦點 pane 作為過濾目標。
    pub(crate) fn open_filter_input(&mut self) {
        let filter = FilterState {
            pane_id: self.focused_pane,
            buffer: String::new(),
            editing: true,
        };
        self.apply_filter_buffer(&filter);
        self.status = String::from("filter: all");
        self.filter = Some(filter);
    }

    /// 打開 global search 面板，遞迴建立目前目錄下的搜尋候選資料集。
    pub(crate) fn open_global_search(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        let search = GlobalSearchState {
            pane_id: self.focused_pane,
            root_dir: pane.cwd.clone(),
            buffer: String::new(),
            editing: true,
            loading: false,
            searched: false,
            selected: 0,
            results: Vec::new(),
        };
        self.status = String::from("global search (insert): type query and Enter");
        self.global_search = Some(search);
        self.cancel_global_search_worker();
        Ok(())
    }

    /// 進入 preview mode，讓目前焦點 pane 的預覽區放大並接手捲動按鍵。
    pub(crate) fn open_preview_focus(&mut self) {
        self.preview_focus = Some(self.focused_pane);
        self.pending_g = false;
        self.pending_y = false;
        self.status = String::from("preview mode");
    }

    /// 打開 preview search 輸入框，並沿用目前已存在的搜尋字串。
    pub(crate) fn open_preview_search_input(&mut self) {
        let existing = self
            .panes
            .get(&self.focused_pane)
            .and_then(|pane| pane.preview_search_query())
            .unwrap_or_default()
            .to_string();
        let search = PreviewSearchState {
            pane_id: self.focused_pane,
            buffer: existing,
            editing: true,
        };
        self.apply_preview_search_buffer(&search);
        self.status =
            preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
        self.preview_search = Some(search);
    }

    /// 進入 visual selection 模式，準備用移動游標的方式框選一段範圍。
    fn open_visual_selection(&mut self) -> io::Result<()> {
        let pane_id = self.focused_pane;
        let selected = self.current_pane_mut()?.selected;
        self.visual_selection = Some(VisualSelectionState {
            pane_id,
            anchor: selected,
            current: selected,
        });
        self.pending_g = false;
        self.pending_y = false;
        self.status = String::from("visual: range selection");
        Ok(())
    }

    /// 將目前 visual selection 範圍加入已標記清單，並回到一般模式。
    fn commit_visual_selection(&mut self) -> io::Result<()> {
        let Some(selection) = self.visual_selection.take() else {
            return Ok(());
        };
        let Some(pane) = self.panes.get_mut(&selection.pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        let added = pane.mark_range(selection.anchor, selection.current);
        self.status = format!("marked {added} items");
        self.pending_g = false;
        Ok(())
    }

    /// 當 visual selection 模式中游標移動後，更新目前選取範圍的尾端。
    fn sync_visual_selection_cursor(&mut self) {
        if let Some(selection) = &mut self.visual_selection
            && let Some(pane) = self.panes.get(&selection.pane_id)
        {
            selection.current = pane.selected;
        }
    }

    /// 回傳 visual selection 狀態列文字，包含目前暫時框選的項目數量。
    fn visual_status_label(&self) -> String {
        match &self.visual_selection {
            Some(selection) => {
                let count = selection.anchor.abs_diff(selection.current) + 1;
                format!("visual: selecting {count} items")
            }
            None => String::from("normal mode"),
        }
    }

    /// 將目前 trash 視覺選取範圍加入已標記清單。
    fn commit_trash_visual_selection(
        &self,
        entries: &[TrashListEntry],
        marked_ids: &mut Vec<String>,
        anchor: usize,
        current: usize,
    ) -> usize {
        let start = anchor.min(current);
        let end = anchor.max(current);
        let mut added = 0usize;

        for entry in entries
            .iter()
            .skip(start)
            .take(end.saturating_sub(start) + 1)
        {
            if !marked_ids.iter().any(|id| id == &entry.id) {
                marked_ids.push(entry.id.clone());
                added += 1;
            }
        }

        added
    }

    /// 回傳 trash 面板目前應套用的目標 id 清單。
    fn selected_or_marked_trash_ids(
        &self,
        entries: &[TrashListEntry],
        selected: usize,
        marked_ids: &[String],
    ) -> Vec<String> {
        if !marked_ids.is_empty() {
            return marked_ids.to_vec();
        }

        entries
            .get(selected)
            .map(|entry| vec![entry.id.clone()])
            .unwrap_or_default()
    }

    /// 回傳 trash 視覺選取狀態列文字。
    fn trash_visual_status_label(
        &self,
        selected: usize,
        anchor: usize,
        marked_count: usize,
    ) -> String {
        let range_count = anchor.abs_diff(selected) + 1;
        if marked_count == 0 {
            format!("trash visual: selecting {range_count} items")
        } else {
            format!("trash visual: selecting {range_count} items ({marked_count} marked)")
        }
    }

    /// 清除目前焦點 pane 中所有標記。
    fn clear_marks_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let count = pane.marked_count();
        pane.clear_marks();
        self.status = if count == 0 {
            String::from("no marks to clear")
        } else {
            format!("cleared {count} marks")
        };
        Ok(())
    }

    /// 判斷目前是否仍有任何 pane 保留已提交的標記。
    fn has_any_marks(&self) -> bool {
        self.panes.values().any(|pane| pane.marked_count() > 0)
    }

    /// 清掉所有 pane 的已提交標記，讓整個畫面回到一般模式。
    fn clear_all_marks(&mut self) {
        let mut cleared = 0usize;
        for pane in self.panes.values_mut() {
            cleared += pane.marked_count();
            pane.clear_marks();
        }

        self.pending_g = false;
        self.pending_y = false;
        self.status = if cleared == 0 {
            String::from("normal mode")
        } else {
            format!("cleared {cleared} marks")
        };
    }

    /// 開始刪除確認流程，建立一個待確認的刪除互動。
    pub(crate) fn start_delete_confirmation(&mut self) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("nothing selected to trash");
            return;
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to trash");
            return;
        }

        let target_name = if entries.len() == 1 {
            entries[0].display_name()
        } else {
            format!("{} items", entries.len())
        };

        self.pending_action = Some(PendingAction::ConfirmDelete {
            pane_id: self.focused_pane,
            target_name: target_name.clone(),
        });
        self.status = format!("confirm trash {target_name}: y/n");
    }

    /// 真正執行將目前待確認項目移到 trash 的檔案系統操作。
    pub(crate) fn confirm_delete(&mut self, pane_id: usize, target_name: &str) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        let trash_store = self.trash_store.clone();
        match pane.trash_selected_or_marked(&trash_store) {
            Ok(trashed_names) if trashed_names.is_empty() => {
                self.status = String::from("nothing selected to trash");
            }
            Ok(trashed_names) if trashed_names.len() == 1 => {
                self.status = format!("trashed {}", trashed_names[0]);
            }
            Ok(trashed_names) => {
                self.status = format!("trashed {} items", trashed_names.len());
            }
            Err(error) => self.status = format!("failed to trash {target_name}: {error}"),
        }

        Ok(())
    }

    /// 還原最近一次放進 trash 的項目，並盡量在目前 pane 對焦到還原結果。
    pub(crate) fn restore_latest_from_trash(&mut self) -> io::Result<()> {
        match self.trash_store.restore_latest()? {
            Some(result) => {
                self.reload_all_panes()?;
                if let Some(pane) = self.panes.get_mut(&self.focused_pane) {
                    let _ = pane.reveal_path(&result.restored_path);
                }
                self.status = format!("restored {}", result.display_name);
            }
            None => {
                self.status = String::from("trash is empty");
            }
        }
        Ok(())
    }

    /// 還原目前 trash 中的所有項目。
    pub(crate) fn restore_all_from_trash(&mut self) -> io::Result<()> {
        let entries = self.trash_store.list_entries()?;
        let ids = entries
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        let results = self.trash_store.restore_many_by_ids(&ids)?;

        if results.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }

        self.reload_all_panes()?;
        if let Some(first) = results.first() {
            if let Some(pane) = self.panes.get_mut(&self.focused_pane) {
                let _ = pane.reveal_path(&first.restored_path);
            }
        }
        if results.len() == 1 {
            self.status = format!("restored {}", results[0].display_name);
        } else {
            self.status = format!("restored {} items", results.len());
        }
        Ok(())
    }

    /// 永久清空整個 trash。
    pub(crate) fn clear_trash(&mut self) -> io::Result<()> {
        let cleared = self.trash_store.clear()?;
        if cleared == 0 {
            self.status = String::from("trash is empty");
        } else {
            self.status = format!("cleared {cleared} trash items");
        }
        Ok(())
    }

    /// 依照 trash 面板中目前選到的項目，執行還原。
    fn restore_trash_entry(
        &mut self,
        entries: &[TrashListEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("trash is empty");
            return Ok(());
        };

        match self.trash_store.restore_by_id(&entry.id)? {
            Some(result) => {
                self.reload_all_panes()?;
                if let Some(pane) = self.panes.get_mut(&self.focused_pane) {
                    let _ = pane.reveal_path(&result.restored_path);
                }
                self.pending_action = None;
                self.status = format!("restored {}", result.display_name);
            }
            None => {
                self.pending_action = Some(PendingAction::TrashPanel {
                    pane_id: self.focused_pane,
                    selected: selected.saturating_sub(1),
                    search: PanelSearchState {
                        buffer: String::new(),
                        editing: false,
                    },
                    marked_ids: Vec::new(),
                    visual_anchor: None,
                });
                self.status = String::from("trash item no longer exists");
            }
        }
        Ok(())
    }

    /// 依照 trash 面板中目前篩選後的結果，批次還原全部項目。
    fn restore_all_trash_entries(&mut self, entries: &[TrashListEntry]) -> io::Result<()> {
        if entries.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }

        let ids = entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let results = self.trash_store.restore_many_by_ids(&ids)?;
        self.reload_all_panes()?;
        self.pending_action = None;
        if results.len() == 1 {
            self.status = format!("restored {}", results[0].display_name);
        } else {
            self.status = format!("restored {} items", results.len());
        }
        Ok(())
    }

    /// 永久刪除 trash 面板中目前選到的單一項目。
    fn delete_trash_entry(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("trash is empty");
            return Ok(());
        };

        match self.trash_store.delete_by_id(&entry.id)? {
            Some(display_name) => {
                let remaining_count = entries.len().saturating_sub(1);
                let next_selected = if remaining_count == 0 {
                    0
                } else {
                    selected.min(remaining_count.saturating_sub(1))
                };
                self.pending_action = Some(PendingAction::TrashPanel {
                    pane_id,
                    selected: next_selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                self.status = format!("deleted permanently {display_name}");
            }
            None => {
                self.pending_action = Some(PendingAction::TrashPanel {
                    pane_id,
                    selected: selected.saturating_sub(1),
                    search,
                    marked_ids,
                    visual_anchor,
                });
                self.status = String::from("trash item no longer exists");
            }
        }
        Ok(())
    }

    /// 永久刪除 trash 面板中目前篩選到的所有項目。
    fn clear_filtered_trash_entries(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        search: PanelSearchState,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        if entries.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }

        let ids = entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let deleted_names = self.trash_store.delete_many_by_ids(&ids)?;
        self.pending_action = Some(PendingAction::TrashPanel {
            pane_id,
            selected: 0,
            search,
            marked_ids,
            visual_anchor,
        });
        if deleted_names.len() == 1 {
            self.status = format!("deleted permanently {}", deleted_names[0]);
        } else {
            self.status = format!("deleted permanently {} items", deleted_names.len());
        }
        Ok(())
    }

    /// 執行 help 面板中選到的功能，直接跳到對應模式或命令。
    fn execute_help_entry(&mut self, entries: &[HelpEntry], selected: usize) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("help: no command selected");
            return Ok(());
        };

        self.pending_action = None;
        match entry.action {
            HelpAction::Command(command) => self
                .execute_command(command)
                .map_err(|error| io::Error::other(error.to_string()))?,
            HelpAction::Delete => self.start_delete_confirmation(),
            HelpAction::Filter => self.open_filter_input(),
            HelpAction::Sort => self.open_sort_picker(),
            HelpAction::Hidden => {
                self.toggle_hidden_files()?;
            }
            HelpAction::Visual => {
                self.open_visual_selection()?;
            }
            HelpAction::QuitHint => {
                self.status = String::from("use q in normal mode to quit");
            }
        }

        if self.pending_action.is_none() && self.help_return.is_some() {
            self.restore_help_return_state(true)?;
        }
        Ok(())
    }

    /// 真正執行重新命名目前待確認項目的檔案系統操作。
    pub(crate) fn confirm_rename(
        &mut self,
        pane_id: usize,
        original_name: &str,
        new_name: &str,
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        match pane.rename_selected(new_name) {
            Ok(Some(renamed_name)) => {
                self.status = format!("renamed {original_name} -> {renamed_name}");
            }
            Ok(None) => {
                self.status = String::from("nothing selected to rename");
            }
            Err(error) => {
                self.status = format!("failed to rename {original_name}: {error}");
            }
        }

        Ok(())
    }

    /// 真正執行建立新項目的檔案系統操作。
    ///
    /// 參數：
    /// - `pane_id: usize`，要建立項目的目標 pane。
    /// - `path: &str`，新項目的相對路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn confirm_create_entry(&mut self, pane_id: usize, path: &str) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        match pane.create_entry(path) {
            Ok(created_name) => {
                let item_type = if created_name.ends_with('/') {
                    "directory"
                } else {
                    "file"
                };
                self.status = format!("created {item_type}: {created_name}");
            }
            Err(error) => {
                self.status = format!("failed to create entry: {error}");
            }
        }

        Ok(())
    }

    /// 讓命令模式可以直接建立新項目，而不必再開啟 inline 輸入框。
    ///
    /// 參數：
    /// - `path: &str`，命令列中指定的新路徑。
    ///
    /// 回傳：`io::Result<()>`。
    fn create_entry_from_command(&mut self, path: &str) -> io::Result<()> {
        self.confirm_create_entry(self.focused_pane, path)
    }

    /// 將 filter 文字套用到指定 pane。
    fn apply_filter_buffer(&mut self, filter: &FilterState) {
        if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
            pane.set_filter_query(&filter.buffer);
        }
    }

    /// 將 preview search 文字套用到指定 pane，並讓 preview 跳到命中位置。
    fn apply_preview_search_buffer(&mut self, search: &PreviewSearchState) {
        if let Some(pane) = self.panes.get_mut(&search.pane_id) {
            pane.set_preview_search_query(&search.buffer);
        }
    }

    /// 啟動一個背景 global search 工作，避免在大型目錄中阻塞主介面。
    fn start_global_search(&mut self, search: &mut GlobalSearchState) -> io::Result<()> {
        self.cancel_global_search_worker();

        let pane_id = search.pane_id;
        let root_dir = search.root_dir.clone();
        let query = search.buffer.clone();
        let show_hidden = self
            .panes
            .get(&pane_id)
            .map(|pane| pane.show_hidden)
            .unwrap_or(false);
        let limit = self.config.search.global_search_limit;
        let chunk_size = self.config.search.global_search_chunk_size;

        let (tx, rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            stream_search_entries(
                pane_id,
                &root_dir,
                show_hidden,
                &query,
                limit,
                chunk_size,
                worker_cancelled,
                tx,
            );
        });

        search.loading = true;
        search.searched = false;
        search.selected = 0;
        search.results.clear();
        self.global_search_rx = Some(rx);
        self.global_search_cancelled = Some(cancelled);
        Ok(())
    }

    /// 要求目前的 global search 背景工作停止，避免使用者離開畫面後仍持續掃描。
    fn cancel_global_search_worker(&mut self) {
        if let Some(cancelled) = self.global_search_cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.global_search_rx = None;
    }

    /// 關閉 global search 畫面，並同步停止正在進行中的背景搜尋。
    fn cancel_global_search(&mut self) {
        self.cancel_global_search_worker();
        self.global_search = None;
        self.status = String::from("normal mode");
    }

    /// 回傳指定 pane 目前 preview 搜尋命中的數量。
    fn preview_match_count(&self, pane_id: usize) -> usize {
        self.panes
            .get(&pane_id)
            .map(PaneState::preview_match_count)
            .unwrap_or(0)
    }

    /// 清除目前 preview 的搜尋狀態；若有清除任何內容則回傳 `true`。
    fn clear_preview_search_if_active(&mut self) -> bool {
        if let Some(search) = self.preview_search.take() {
            if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                pane.clear_preview_search();
            }
            self.status = String::from("preview search cleared");
            return true;
        }

        if let Some(pane) = self
            .preview_focus
            .and_then(|pane_id| self.panes.get_mut(&pane_id))
            && pane.has_preview_search()
        {
            pane.clear_preview_search();
            self.status = String::from("preview search cleared");
            return true;
        }

        false
    }

    /// 在 preview mode 中跳到下一個或上一個搜尋結果，並回傳狀態訊息。
    fn jump_preview_match(&mut self, forward: bool) -> io::Result<String> {
        let Some(pane) = self
            .preview_focus
            .and_then(|pane_id| self.panes.get_mut(&pane_id))
        else {
            return Ok(String::from("pane no longer exists"));
        };

        let Some(query) = pane.preview_search_query().map(str::to_string) else {
            return Ok(String::from("preview search is empty"));
        };

        let found = if forward {
            pane.jump_to_next_preview_match()
        } else {
            pane.jump_to_previous_preview_match()
        };
        let count = pane.preview_match_count();

        Ok(if found {
            format!("preview search: {query} ({count})")
        } else {
            format!("preview search: {query} (0)")
        })
    }

    /// 將目前 global search 選到的結果打開到原 pane 中，並把游標移到該項目。
    fn open_global_search_result(&mut self, search: GlobalSearchState) -> io::Result<()> {
        let Some(entry) = search.results.get(search.selected).cloned() else {
            self.status = String::from("global search: no result selected");
            self.global_search = Some(search);
            return Ok(());
        };

        let Some(pane) = self.panes.get_mut(&search.pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        pane.reveal_path(&entry.path)?;
        self.cancel_global_search_worker();
        self.global_search = None;
        self.status = format!("search opened: {}", entry.relative_path);
        Ok(())
    }

    /// 切換目前焦點 pane 的隱藏檔顯示狀態。
    fn toggle_hidden_files(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        pane.toggle_hidden();
        self.status = if pane.show_hidden {
            String::from("showing hidden files")
        } else {
            String::from("hiding hidden files")
        };
        Ok(())
    }

    /// 將指定 pane 套用某一種排序模式。
    fn apply_sort_mode(&mut self, pane_id: usize, sort_mode: SortMode) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };
        pane.set_sort_mode(sort_mode);
        self.status = format!("sort: {}", pane.sort_mode.label());
        Ok(())
    }

    /// 將目前選取項目放進內部剪貼簿，模式為複製。
    ///
    /// 參數：無。
    /// 回傳：`()`
    pub(crate) fn copy_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Copy);
    }

    /// 將目前選取項目放進內部剪貼簿，模式為剪下。
    ///
    /// 參數：無。
    /// 回傳：`()`
    pub(crate) fn cut_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Cut);
    }

    /// 把目前焦點 pane 的選取項目寫入剪貼簿。
    ///
    /// 參數：
    /// - `operation: ClipboardOperation`，要記錄成複製或剪下。
    ///
    /// 回傳：`()`
    fn store_selected_in_clipboard(&mut self, operation: ClipboardOperation) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        };

        let entries: Vec<ClipboardEntry> = pane
            .selected_or_marked_entries()
            .into_iter()
            .map(|entry| {
                let display_name = entry.display_name();
                ClipboardEntry {
                    source_path: entry.path,
                    display_name,
                }
            })
            .collect();

        if entries.is_empty() {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        }

        let count = entries.len();
        self.clipboard = Some(ClipboardState { entries, operation });

        self.status = match operation {
            ClipboardOperation::Copy if count == 1 => String::from("copied 1 item"),
            ClipboardOperation::Copy => format!("copied {count} items"),
            ClipboardOperation::Cut if count == 1 => String::from("cut 1 item"),
            ClipboardOperation::Cut => format!("cut {count} items"),
        };
    }

    /// 將內部剪貼簿中的項目貼到目前有焦點的 pane 目錄。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表貼上流程已完成，並同步更新所有 pane。
    /// - 失敗時代表目標目錄或檔案系統操作失敗。
    pub(crate) fn paste_into_focused_pane(&mut self) -> io::Result<()> {
        let Some(clipboard) = self.clipboard.clone() else {
            self.status = String::from("clipboard is empty");
            return Ok(());
        };

        let target_dir = match self.panes.get(&self.focused_pane) {
            Some(pane) => pane.cwd.clone(),
            None => {
                self.status = String::from("pane no longer exists");
                return Ok(());
            }
        };

        let mut pasted_count = 0usize;
        for entry in &clipboard.entries {
            if entry.source_path.parent() == Some(target_dir.as_path())
                && clipboard.operation == ClipboardOperation::Cut
            {
                continue;
            }

            let paste_result = match self.panes.get_mut(&self.focused_pane) {
                Some(pane) => match clipboard.operation {
                    ClipboardOperation::Copy => {
                        pane.copy_entry_into_current_dir(&entry.source_path)
                    }
                    ClipboardOperation::Cut => pane.move_entry_into_current_dir(&entry.source_path),
                },
                None => {
                    self.status = String::from("pane no longer exists");
                    return Ok(());
                }
            };

            if let Err(error) = paste_result {
                self.status = format!("paste failed for {}: {error}", entry.display_name);
                return Ok(());
            }

            pasted_count += 1;
        }

        if pasted_count == 0 {
            self.status = String::from("nothing to paste into this directory");
            return Ok(());
        }

        self.reload_all_panes()?;
        self.status = match clipboard.operation {
            ClipboardOperation::Copy if pasted_count == 1 => String::from("pasted copy: 1 item"),
            ClipboardOperation::Copy => format!("pasted copy: {pasted_count} items"),
            ClipboardOperation::Cut if pasted_count == 1 => String::from("moved: 1 item"),
            ClipboardOperation::Cut => format!("moved: {pasted_count} items"),
        };

        if clipboard.operation == ClipboardOperation::Cut {
            self.clipboard = None;
        }

        Ok(())
    }

    /// 將目前選取或已標記的項目直接移動到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，目標目錄，可以是絕對路徑或相對於目前 pane 的路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_path(&mut self, target: &str) -> io::Result<()> {
        let Some(target_dir) = self.resolve_move_target_dir(target) else {
            self.status = String::from("usage: move <target-dir>");
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 將目前選取或已標記的項目移到指定 pane 目前所在的目錄。
    ///
    /// 參數：
    /// - `target: &str`，目標 pane 編號字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_pane_id(&mut self, target: &str) -> io::Result<()> {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: move-panel <pane-id>. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        let Some(target_dir) = self.panes.get(&target_pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = format!(
                "unknown pane {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 將目前焦點 pane 的選取項目批次移動到目標目錄。
    fn move_selected_entries_into_dir(&mut self, target_dir: &std::path::Path) -> io::Result<()> {
        let Some(source_pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };
        let source_dir = source_pane.cwd.clone();
        let entries = source_pane.selected_or_marked_entries();

        if entries.is_empty() {
            self.status = String::from("nothing selected to move");
            return Ok(());
        }

        if !target_dir.exists() {
            self.status = format!("move target does not exist: {}", target_dir.display());
            return Ok(());
        }
        if !target_dir.is_dir() {
            self.status = format!("move target is not a directory: {}", target_dir.display());
            return Ok(());
        }
        if source_dir == target_dir {
            self.status = String::from("move target is the current directory");
            return Ok(());
        }

        let mut moved_count = 0usize;
        for entry in &entries {
            if let Err(error) = PaneState::move_path_to_dir(&entry.path, target_dir) {
                self.status = format!("move failed for {}: {error}", entry.display_name());
                return Ok(());
            }
            moved_count += 1;
        }

        self.reload_all_panes()?;
        self.status = if moved_count == 1 {
            format!("moved 1 item -> {}", target_dir.display())
        } else {
            format!("moved {moved_count} items -> {}", target_dir.display())
        };
        Ok(())
    }

    /// 將命令列中的 move 目標字串解析成實際目錄路徑。
    fn resolve_move_target_dir(&self, target: &str) -> Option<PathBuf> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return None;
        }

        let base_dir = self.panes.get(&self.focused_pane)?.cwd.clone();
        let path = PathBuf::from(trimmed);
        Some(if path.is_absolute() {
            path
        } else {
            base_dir.join(path)
        })
    }

    /// 將目前可用的 pane 編號整理成易讀字串，供錯誤訊息與提示使用。
    fn available_pane_ids_label(&self) -> String {
        self.ordered_pane_ids()
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// 重新整理所有 pane，讓跨目錄操作後的內容保持同步。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表所有 pane 都已重新載入。
    /// - 失敗時代表至少有一個 pane 在重新讀取時發生錯誤。
    fn reload_all_panes(&mut self) -> io::Result<()> {
        for pane in self.panes.values_mut() {
            pane.reload()?;
        }
        Ok(())
    }

    /// 根據目前應用程式狀態繪製整個畫面。
    pub(crate) fn render(&mut self, frame: &mut ratatui::Frame<'_>) {
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
        let mut cursor_position = None;
        for (&pane_id, &rect) in &pane_rects {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                let rename_buffer = match &self.pending_action {
                    Some(PendingAction::Rename {
                        pane_id: rename_pane_id,
                        buffer,
                        cursor,
                        mode,
                        ..
                    }) if *rename_pane_id == pane_id => Some(InlineEditorState {
                        buffer: buffer.as_str(),
                        cursor: *cursor,
                        title: match mode {
                            RenameMode::Insert => " Rename (insert): ",
                            RenameMode::Normal => " Rename (normal): ",
                        },
                    }),
                    Some(PendingAction::CreateEntry {
                        pane_id: create_pane_id,
                        buffer,
                        cursor,
                        mode,
                    }) if *create_pane_id == pane_id => Some(InlineEditorState {
                        buffer: buffer.as_str(),
                        cursor: *cursor,
                        title: create_editor_title(*mode),
                    }),
                    _ => None,
                };
                let picker_options = match &self.pending_action {
                    Some(PendingAction::OpenPicker {
                        pane_id: open_pane_id,
                        target,
                        ..
                    }) if *open_pane_id == pane_id => Some(
                        open_picker_options(target)
                            .into_iter()
                            .map(|option| option.label.to_string())
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                };
                let picker_state = match &self.pending_action {
                    Some(PendingAction::OpenPicker {
                        pane_id: open_pane_id,
                        selected,
                        ..
                    }) if *open_pane_id == pane_id => {
                        picker_options.as_ref().map(|options| InlinePickerState {
                            title: " Open with: ",
                            options,
                            selected: *selected,
                        })
                    }
                    _ => None,
                };
                let trash_lines = if let Some(PendingAction::TrashPanel {
                    pane_id: action_pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(
                            trash_panel_lines(
                                &self.trash_store,
                                &search.buffer,
                                marked_ids,
                                visual_anchor.map(|anchor| (anchor, *selected)),
                            )
                            .unwrap_or_default(),
                        )
                    } else {
                        None
                    }
                } else {
                    None
                };
                let help_lines = if let Some(PendingAction::HelpPanel {
                    pane_id: action_pane_id,
                    search,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(help_panel_lines(&search.buffer))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let panel_state = if let Some(search) = self.global_search.as_ref() {
                    (search.pane_id == pane_id && (search.loading || search.searched)).then_some(
                        PaneListState::Search(SearchListState {
                            results: &search.results,
                            selected: search.selected,
                            loading: search.loading && self.config.search.show_loading,
                        }),
                    )
                } else if let Some(PendingAction::TrashPanel {
                    pane_id: action_pane_id,
                    selected,
                    search,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Trash {
                            lines: trash_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                            search: &search.buffer,
                            editing: search.editing,
                        })
                    } else {
                        None
                    }
                } else if let Some(PendingAction::HelpPanel {
                    pane_id: action_pane_id,
                    selected,
                    search,
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Help {
                            lines: help_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                            search: &search.buffer,
                            editing: search.editing,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let pane_cursor = render_pane(
                    frame,
                    rect,
                    pane_id,
                    pane,
                    pane_id == self.focused_pane,
                    self.preview_focus == Some(pane_id),
                    self.visual_selection.as_ref().and_then(|selection| {
                        (selection.pane_id == pane_id)
                            .then_some((selection.anchor, selection.current))
                    }),
                    panel_state,
                    self.theme,
                    &self.config,
                    rename_buffer,
                    picker_state,
                );
                if cursor_position.is_none() {
                    cursor_position = pane_cursor;
                }
            }
        }

        let help = Paragraph::new(Line::from(vec![
            Span::styled("hjkl", self.theme.accent_style()),
            Span::raw(" move  "),
            Span::styled("gg/G", self.theme.accent_style()),
            Span::raw(" jump  "),
            Span::styled("m / '", self.theme.accent_style()),
            Span::raw(" bookmark  "),
            Span::styled("V", self.theme.accent_style()),
            Span::raw(" visual mark  "),
            Span::styled("yy", self.theme.accent_style()),
            Span::raw(" copy  "),
            Span::styled("x", self.theme.accent_style()),
            Span::raw(" cut  "),
            Span::styled("p", self.theme.accent_style()),
            Span::raw(" paste  "),
            Span::styled("Ctrl-w s/v", self.theme.accent_style()),
            Span::raw(" split  "),
            Span::styled("Ctrl-w h/j/k/l", self.theme.accent_style()),
            Span::raw(" focus  "),
            Span::styled("d", self.theme.accent_style()),
            Span::raw(" trash  "),
            Span::styled("r", self.theme.accent_style()),
            Span::raw(" rename  "),
            Span::styled("P", self.theme.accent_style()),
            Span::raw(" preview  "),
            Span::styled("/ n N", self.theme.accent_style()),
            Span::raw(" search  "),
            Span::styled("a", self.theme.accent_style()),
            Span::raw(" create  "),
            Span::styled("s", self.theme.accent_style()),
            Span::raw(" global search  "),
            Span::styled("f", self.theme.accent_style()),
            Span::raw(" filter  "),
            Span::styled(".", self.theme.accent_style()),
            Span::raw(" hidden  "),
            Span::styled(",", self.theme.accent_style()),
            Span::raw(" sort  "),
            Span::styled(":rename", self.theme.accent_style()),
            Span::raw(" dialog  "),
            Span::styled(":create", self.theme.accent_style()),
            Span::raw("  "),
            Span::styled(":restore", self.theme.accent_style()),
            Span::raw("  "),
            Span::styled(":preview-search", self.theme.accent_style()),
            Span::raw("  "),
            Span::styled(":preview", self.theme.accent_style()),
            Span::raw("  "),
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

        if self.command_mode
            && let Some(area) = pane_rects.get(&self.focused_pane)
        {
            let command_cursor =
                render_command_palette(frame, *area, self.theme, &self.command_buffer);
            if cursor_position.is_none() {
                cursor_position = Some(command_cursor);
            }
        }

        if let Some(filter) = &self.filter
            && filter.editing
        {
            let filter_cursor = render_filter_input(frame, outer[0], self.theme, &filter.buffer);
            if cursor_position.is_none() {
                cursor_position = Some(filter_cursor);
            }
        }

        if let Some(search) = &self.preview_search
            && search.editing
        {
            let search_cursor =
                render_preview_search_input(frame, outer[0], self.theme, &search.buffer);
            if cursor_position.is_none() {
                cursor_position = Some(search_cursor);
            }
        }

        if let Some(search) = &self.global_search
            && let Some(area) = pane_rects.get(&search.pane_id)
        {
            let search_cursor = render_global_search_panel(
                frame,
                *area,
                self.theme,
                &search.buffer,
                search.editing,
            );
            if search.editing && cursor_position.is_none() {
                cursor_position = Some(search_cursor);
            }
        }

        match &self.pending_action {
            Some(PendingAction::ConfirmDelete { target_name, .. }) => {
                render_confirm_dialog(frame, frame.area(), target_name, self.theme, &self.config);
            }
            Some(PendingAction::SortPicker { .. }) => {
                super::ui::render_sort_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::ThemePicker { selected }) => {
                render_theme_picker(frame, frame.area(), self.theme, *selected, &self.config);
            }
            Some(PendingAction::BookmarkList { pane_id, selected }) => {
                let lines = bookmark_panel_lines(self.bookmark_store.list());
                if let Some(area) = pane_rects.get(pane_id) {
                    render_bookmark_picker(frame, *area, self.theme, &lines, *selected);
                }
            }
            Some(PendingAction::TrashPanel { .. })
            | Some(PendingAction::HelpPanel { .. })
            | Some(PendingAction::OpenPicker { .. }) => {}
            Some(PendingAction::Rename { .. }) | Some(PendingAction::CreateEntry { .. }) => {}
            None => {}
        }

        if let Some((x, y)) = cursor_position {
            frame.set_cursor_position((x, y));
        }
    }

    /// 回傳目前畫面應該呈現的 rename 游標模式。
    ///
    /// 回傳：`Option<RenameMode>`。
    /// - `Some(RenameMode::Insert)` 代表應顯示細線游標。
    /// - `Some(RenameMode::Normal)` 代表應顯示方塊游標。
    /// - `None` 代表目前沒有 rename 輸入框，不需要特別切換。
    pub(crate) fn rename_cursor_mode(&self) -> Option<RenameMode> {
        match self.pending_action {
            Some(PendingAction::Rename { mode, .. })
            | Some(PendingAction::CreateEntry { mode, .. }) => Some(mode),
            _ if self
                .global_search
                .as_ref()
                .is_some_and(|search| search.editing) =>
            {
                Some(RenameMode::Insert)
            }
            _ => None,
        }
    }

    /// 在每一輪事件迴圈中檢查背景 global search 是否已完成。
    pub(crate) fn poll_background_tasks(&mut self) {
        let Some(receiver) = &self.global_search_rx else {
            return;
        };
        let messages: Vec<GlobalSearchEvent> = receiver.try_iter().collect();
        if messages.is_empty() {
            return;
        }

        let mut finished = false;
        for message in messages {
            let Some(search) = &mut self.global_search else {
                break;
            };

            match message {
                GlobalSearchEvent::Chunk {
                    pane_id,
                    query,
                    mut entries,
                } => {
                    if search.pane_id != pane_id || search.buffer != query {
                        continue;
                    }
                    search.results.append(&mut entries);
                    search.results.sort_by(|left, right| {
                        left.relative_path
                            .to_lowercase()
                            .cmp(&right.relative_path.to_lowercase())
                            .then_with(|| left.relative_path.cmp(&right.relative_path))
                    });
                    search.results.truncate(200);
                    search.selected = search.selected.min(search.results.len().saturating_sub(1));
                    search.searched = true;
                    self.status = global_search_status(
                        &search.buffer,
                        search.results.len(),
                        search.editing,
                        search.searched,
                        true,
                    );
                }
                GlobalSearchEvent::Done { pane_id, query } => {
                    if search.pane_id != pane_id || search.buffer != query {
                        continue;
                    }
                    search.loading = false;
                    search.searched = true;
                    self.status = global_search_status(
                        &search.buffer,
                        search.results.len(),
                        search.editing,
                        search.searched,
                        search.loading,
                    );
                    finished = true;
                }
            }
        }

        if finished {
            self.cancel_global_search_worker();
        }
    }
}

/// 將設定檔中的啟動偏好套用到新建立的 pane。
///
/// 參數：
/// - `config: &AppConfig`，目前啟動所使用的設定。
/// - `pane: &mut PaneState`，要被套用預設值的 pane。
///
/// 回傳：`()`
fn apply_config_to_pane(config: &AppConfig, pane: &mut PaneState) {
    pane.set_show_hidden(config.pane.show_hidden);
    pane.set_sort_mode(sort_mode_from_config(
        config.pane.default_sort,
        config.pane.default_sort_reverse,
    ));
}

/// 將設定檔中的排序偏好轉成 pane 實際使用的排序模式。
///
/// 參數：
/// - `sort: StartupSort`，設定檔指定的排序種類。
/// - `reverse: bool`，是否使用反向排序。
///
/// 回傳：`SortMode`，可直接套用到 pane 的排序模式。
fn sort_mode_from_config(sort: StartupSort, reverse: bool) -> SortMode {
    match sort {
        StartupSort::Alphabetical => SortMode::Alphabetical { reverse },
        StartupSort::Natural => SortMode::Natural { reverse },
        StartupSort::Size => SortMode::Size { reverse },
        StartupSort::Modified => SortMode::Modified { reverse },
        StartupSort::Created => SortMode::Created { reverse },
        StartupSort::Extension => SortMode::Extension { reverse },
        StartupSort::Random => SortMode::Random,
    }
}

/// 將書籤資料轉成彈窗可直接顯示的列內容。
fn bookmark_panel_lines(entries: Vec<BookmarkEntry>) -> Vec<BookmarkPanelLine> {
    entries
        .into_iter()
        .map(|entry| BookmarkPanelLine {
            key: format!("[{}]", entry.key),
            path: entry.path.display().to_string(),
        })
        .collect()
}

/// 從命令列參數中取出單一書籤按鍵。
///
/// 參數：
/// - `args: &str`，使用者在 `:bookmark ...` 後輸入的內容。
///
/// 回傳：`Option<char>`。
/// - `Some(char)` 代表成功解析出唯一按鍵。
/// - `None` 代表輸入為空，或不是單一字元。
fn parse_bookmark_argument(args: &str) -> Option<char> {
    let trimmed = args.trim();
    let mut chars = trimmed.chars();
    let key = chars.next()?;
    if chars.next().is_some() || key.is_whitespace() {
        return None;
    }
    Some(key)
}

/// 從命令列參數中取出 pane 編號。
///
/// 參數：
/// - `args: &str`，使用者在 `:move-panel ...` 後輸入的內容。
///
/// 回傳：`Option<usize>`。
/// - `Some(usize)` 代表成功解析成有效編號。
/// - `None` 代表輸入為空、不是數字或小於 1。
fn parse_pane_id_argument(args: &str) -> Option<usize> {
    let trimmed = args.trim();
    let id = trimmed.parse::<usize>().ok()?;
    (id > 0).then_some(id)
}

/// 判斷某些終端送出的 `Shift+字母` 是否應視為大寫命令。
///
/// 參數：
/// - `key: &KeyEvent`，目前收到的鍵盤事件。
/// - `uppercase: char`，邏輯上希望匹配的大寫英文字母。
///
/// 回傳：`bool`。
/// - `true` 代表事件要視為這個大寫命令。
/// - `false` 代表不是。
fn key_matches_shifted_letter(key: &KeyEvent, uppercase: char) -> bool {
    let lower = uppercase.to_ascii_lowercase();
    key.code == KeyCode::Char(uppercase)
        || (key.code == KeyCode::Char(lower) && key.modifiers.contains(KeyModifiers::SHIFT))
}

/// 根據目前書籤彈窗的內容，產生適合顯示在狀態列的提示文字。
fn bookmark_list_status(count: usize, selected: usize) -> String {
    if count == 0 {
        String::from("bookmark list: empty")
    } else {
        format!(
            "bookmarks: {}/{} (j/k move, Enter open, Esc close)",
            selected.saturating_add(1).min(count),
            count
        )
    }
}

/// 描述 help 面板中某一列按下 Enter 後要執行的行為。
#[derive(Clone, Copy)]
enum HelpAction {
    Command(&'static str),
    Delete,
    Filter,
    Sort,
    Hidden,
    Visual,
    QuitHint,
}

/// 描述 help 面板中完整的一筆資料。
#[derive(Clone)]
struct HelpEntry {
    line: HelpPanelLine,
    action: HelpAction,
}

/// 先依搜尋條件過濾 trash 原始資料，再提供給面板使用。
fn trash_panel_entries(trash_store: &TrashStore, query: &str) -> io::Result<Vec<TrashListEntry>> {
    let trimmed = query.trim().to_lowercase();
    let entries = trash_store.list_entries()?;
    if trimmed.is_empty() {
        return Ok(entries);
    }

    Ok(entries
        .into_iter()
        .filter(|entry| {
            entry.display_name.to_lowercase().contains(&trimmed)
                || entry
                    .original_path
                    .display()
                    .to_string()
                    .to_lowercase()
                    .contains(&trimmed)
        })
        .collect())
}

/// 將目前 trash store 中的項目轉成面板可直接顯示的列內容。
fn trash_panel_lines(
    trash_store: &TrashStore,
    query: &str,
    marked_ids: &[String],
    visual_range: Option<(usize, usize)>,
) -> io::Result<Vec<TrashPanelLine>> {
    Ok(trash_panel_entries(trash_store, query)?
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let visually_selected = visual_range
                .map(|(start, end)| {
                    let range_start = start.min(end);
                    let range_end = start.max(end);
                    index >= range_start && index <= range_end
                })
                .unwrap_or(false);
            TrashPanelLine {
                name: entry.display_name,
                original_path: entry.original_path.display().to_string(),
                deleted_at: format_deleted_at(entry.deleted_at_unix_ms),
                marked: marked_ids.iter().any(|id| id == &entry.id) || visually_selected,
            }
        })
        .collect())
}

/// 依照搜尋條件過濾 F1 功能面板的完整資料。
fn help_entries(query: &str) -> Vec<HelpEntry> {
    let entries = vec![
        help_entry(
            ":rename",
            "r",
            "重新命名目前選取的檔案或資料夾",
            HelpAction::Command("rename"),
        ),
        help_entry(
            ":create",
            "a",
            "建立新檔案、資料夾或巢狀路徑",
            HelpAction::Command("create"),
        ),
        help_entry(
            ":bookmark set",
            "m{key}",
            "把目前 pane 的目錄記成書籤，之後可快速跳回來",
            HelpAction::Command("bookmark list"),
        ),
        help_entry(
            ":bookmark jump",
            "'{key}",
            "跳到已記錄的書籤目錄；設定檔中的固定書籤也能直接使用",
            HelpAction::Command("bookmark list"),
        ),
        help_entry(
            ":bookmark list",
            "",
            "列出目前可用的書籤按鍵與對應路徑",
            HelpAction::Command("bookmark list"),
        ),
        help_entry(
            ":copy",
            "yy",
            "複製目前選取項目到內部剪貼簿",
            HelpAction::Command("copy"),
        ),
        help_entry(
            ":cut",
            "x",
            "剪下目前選取項目到內部剪貼簿",
            HelpAction::Command("cut"),
        ),
        help_entry(
            ":move <path>",
            "",
            "直接把目前選取或標記的項目移動到指定目錄",
            HelpAction::Command("move ."),
        ),
        help_entry(
            ":move-panel",
            "",
            "把目前選取或標記的項目移動到指定 pane 編號目前所在的目錄",
            HelpAction::Command("move-panel 2"),
        ),
        help_entry(
            ":paste",
            "p",
            "貼上剪貼簿項目到目前目錄",
            HelpAction::Command("paste"),
        ),
        help_entry(
            ":open",
            "o / Enter",
            "用預設外部方式打開目前選取項目；文字檔走 $EDITOR，其他交給系統",
            HelpAction::Command("open"),
        ),
        help_entry(
            ":open-picker",
            "O / Shift-Enter",
            "打開 Open with 小視窗，手動選擇 Editor、Vim、Open 或 Reveal",
            HelpAction::Command("open-picker"),
        ),
        help_entry(
            ":vim",
            "",
            "直接用 vim 打開目前選取的檔案或目錄",
            HelpAction::Command("vim"),
        ),
        help_entry(
            ":reveal",
            "",
            "在系統檔案管理器中顯示目前選取的檔案或目錄",
            HelpAction::Command("reveal"),
        ),
        help_entry(
            ":delete",
            "d",
            "將目前選取項目移到 trash，並顯示確認提示",
            HelpAction::Delete,
        ),
        help_entry(
            ":trash",
            "",
            "打開 trash 面板，查看並還原已移入 trash 的項目",
            HelpAction::Command("trash"),
        ),
        help_entry(
            ":restore",
            "",
            "還原最近一次移到 trash 的項目",
            HelpAction::Command("restore"),
        ),
        help_entry(
            ":trash restore-all",
            "R in trash",
            "在 trash 面板中還原目前篩選結果的全部項目",
            HelpAction::Command("trash restore-all"),
        ),
        help_entry(
            ":trash clear",
            "C in trash",
            "永久刪除 trash 內目前篩選結果的全部項目",
            HelpAction::Command("trash clear"),
        ),
        help_entry(
            ":trash mark",
            "V in trash",
            "在 trash 面板中用 Vim 風格選取多個項目後一起還原或刪除",
            HelpAction::Command("trash"),
        ),
        help_entry(
            ":search",
            "s",
            "打開全域搜尋輸入框",
            HelpAction::Command("search"),
        ),
        help_entry(
            ":preview-search",
            "/",
            "在 preview 內容中搜尋文字",
            HelpAction::Command("preview-search"),
        ),
        help_entry(
            ":preview",
            "P",
            "切換到 preview focus 模式",
            HelpAction::Command("preview"),
        ),
        help_entry(
            ":split",
            "Ctrl-w s",
            "水平分割目前 pane",
            HelpAction::Command("split"),
        ),
        help_entry(
            ":vsplit",
            "Ctrl-w v",
            "垂直分割目前 pane",
            HelpAction::Command("vsplit"),
        ),
        help_entry(
            ":close",
            "Ctrl-w c",
            "關閉目前 pane",
            HelpAction::Command("close"),
        ),
        help_entry(
            ":only",
            "Ctrl-w o",
            "只保留目前 pane",
            HelpAction::Command("only"),
        ),
        help_entry(
            ":theme",
            "",
            "打開主題選擇面板",
            HelpAction::Command("theme"),
        ),
        help_entry(
            ":theme next",
            "",
            "直接切到下一個主題",
            HelpAction::Command("theme next"),
        ),
        help_entry(
            ":help",
            "F1",
            "打開這個功能說明面板",
            HelpAction::Command("help"),
        ),
        help_entry(":filter", "f", "即時過濾目前列表內容", HelpAction::Filter),
        help_entry(":sort", ",", "打開排序方式快捷鍵面板", HelpAction::Sort),
        help_entry(":hidden", ".", "切換是否顯示隱藏檔", HelpAction::Hidden),
        help_entry(":visual", "V", "進入視覺範圍標記模式", HelpAction::Visual),
        help_entry(
            ":quit",
            "q",
            "離開 terminal file manager",
            HelpAction::QuitHint,
        ),
    ];

    let trimmed = query.trim().to_lowercase();
    if trimmed.is_empty() {
        return entries;
    }

    entries
        .into_iter()
        .filter(|entry| {
            entry.line.command.to_lowercase().contains(&trimmed)
                || entry.line.shortcut.to_lowercase().contains(&trimmed)
                || entry.line.description.to_lowercase().contains(&trimmed)
        })
        .collect()
}

/// 只取出 help 面板渲染需要的列內容。
fn help_panel_lines(query: &str) -> Vec<HelpPanelLine> {
    help_entries(query)
        .into_iter()
        .map(|entry| entry.line)
        .collect()
}

/// 建立單一功能說明列與其動作。
fn help_entry(command: &str, shortcut: &str, description: &str, action: HelpAction) -> HelpEntry {
    HelpEntry {
        line: HelpPanelLine {
            command: command.to_string(),
            shortcut: shortcut.to_string(),
            description: description.to_string(),
        },
        action,
    }
}

/// 產生 trash 面板底部狀態列訊息。
fn trash_panel_status(
    query: &str,
    count: usize,
    selected: usize,
    editing: bool,
    marked_count: usize,
) -> String {
    if editing {
        if query.is_empty() {
            format!("trash search: all ({count})")
        } else {
            format!("trash search: {query} ({count})")
        }
    } else if count == 0 {
        String::from("trash: empty")
    } else {
        format!(
            "trash: {}/{} [marked: {}] (Enter restore, V mark, R all, D delete, C clear, f search)",
            selected + 1,
            count,
            marked_count
        )
    }
}

/// 產生說明面板底部狀態列訊息。
fn help_panel_status(query: &str, count: usize, editing: bool) -> String {
    if editing {
        format!(
            "help search: {} ({count})",
            if query.is_empty() { "all" } else { query }
        )
    } else if query.is_empty() {
        format!("help: {count} commands (f to search)")
    } else {
        format!("help: {} ({count})", query)
    }
}

/// 將 unix 毫秒時間轉成較容易閱讀的本地時間字串。
fn format_deleted_at(unix_ms: u64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms))
        .format("%m/%d %H:%M")
        .to_string()
}

/// 回傳建立流程的狀態列內容，讓使用者知道目前正處於哪一種編輯模式。
fn create_status_label(mode: &str) -> String {
    format!("create entry: {mode}")
}

/// 依照目前 preview search 文字與命中數量產生狀態列訊息。
fn preview_search_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("preview search: all")
    } else {
        format!("preview search: {buffer} ({matches})")
    }
}

/// 依照目前 global search 文字、結果數與模式，產生狀態列訊息。
fn global_search_status(
    buffer: &str,
    matches: usize,
    editing: bool,
    searched: bool,
    loading: bool,
) -> String {
    let mode = if editing { "insert" } else { "normal" };
    if loading {
        format!("global search ({mode}): loading...")
    } else if !searched {
        if buffer.is_empty() {
            format!("global search ({mode}): type query and Enter")
        } else {
            format!("global search ({mode}): {buffer} (press Enter to search)")
        }
    } else if buffer.is_empty() {
        format!("global search ({mode}): all ({matches})")
    } else {
        format!("global search ({mode}): {buffer} ({matches})")
    }
}

/// 依照編輯模式決定建立輸入框的標題文字。
fn create_editor_title(mode: RenameMode) -> &'static str {
    match mode {
        RenameMode::Insert => " Create (append / for dir): ",
        RenameMode::Normal => " Create (normal): ",
    }
}

/// 將字元插入到指定的字元游標位置，並在插入後把游標往右移一格。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的檔名字串。
/// - `cursor: &mut usize`，以字元數計算的游標位置。
/// - `ch: char`，要插入的新字元。
///
/// 回傳：`()`
fn insert_char(buffer: &mut String, cursor: &mut usize, ch: char) {
    let byte_index = char_to_byte_index(buffer, *cursor);
    buffer.insert(byte_index, ch);
    *cursor += 1;
}

/// 刪除游標左側的一個字元，行為對齊一般文字編輯器的 Backspace。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的檔名字串。
/// - `cursor: &mut usize`，以字元數計算的游標位置。
///
/// 回傳：`()`
fn backspace_char(buffer: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }

    let remove_at = *cursor - 1;
    let start = char_to_byte_index(buffer, remove_at);
    let end = char_to_byte_index(buffer, *cursor);
    buffer.replace_range(start..end, "");
    *cursor -= 1;
}

/// 把字元游標位置轉成 Rust 字串的位元組索引，方便做安全的字串編輯。
///
/// 參數：
/// - `text: &str`，來源字串。
/// - `char_index: usize`，以字元數表示的目標位置。
///
/// 回傳：`usize`，對應的位元組索引。
fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

/// 將游標往右移動一個字元，但不會超過字串末端。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，移動後的新游標位置。
fn move_cursor_right(buffer: &str, cursor: usize) -> usize {
    let end = buffer.chars().count();
    (cursor + 1).min(end)
}

/// 回傳 normal 模式中 `$` 應該停留的位置，也就是最後一個可見字元。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
///
/// 回傳：`usize`，normal 模式游標應停留的字元索引。
fn rename_line_end_cursor(buffer: &str) -> usize {
    buffer.chars().count().saturating_sub(1)
}

/// 判斷某個字元是否應該被視為檔名中的「單字內容」。
///
/// 參數：
/// - `ch: char`，要判斷的字元。
///
/// 回傳：`bool`。
/// - `true` 代表它屬於單字本體，例如英數字。
/// - `false` 代表它是分隔符，例如 `-`、`_`、`.` 或空白。
fn is_rename_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
}

/// 找出 normal 模式下 `w` 應跳到的位置，也就是下一段名稱的開頭。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，下一個單字的起始字元索引。
fn rename_next_word_start(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 0;
    }

    let mut index = cursor.min(len);
    if index < len && is_rename_word_char(chars[index]) {
        while index < len && is_rename_word_char(chars[index]) {
            index += 1;
        }
    }

    while index < len && !is_rename_word_char(chars[index]) {
        index += 1;
    }

    index.min(rename_line_end_cursor(buffer))
}

/// 找出 normal 模式下 `b` 應跳到的位置，也就是前一段名稱的開頭。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，前一個單字的起始字元索引。
fn rename_previous_word_start(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    if chars.is_empty() {
        return 0;
    }

    let mut index = cursor.min(chars.len().saturating_sub(1));
    if !is_rename_word_char(chars[index]) {
        while index > 0 && !is_rename_word_char(chars[index]) {
            index -= 1;
        }
        if index == 0 && !is_rename_word_char(chars[index]) {
            return 0;
        }
    } else if index > 0 {
        index -= 1;
        while index > 0 && !is_rename_word_char(chars[index]) {
            index -= 1;
        }
        if index == 0 && !is_rename_word_char(chars[index]) {
            return 0;
        }
    }

    while index > 0 && is_rename_word_char(chars[index - 1]) {
        index -= 1;
    }

    index
}

/// 找出 normal 模式下 `e` 應跳到的位置，也就是目前或下一段名稱的結尾。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，目標單字最後一個字元的索引。
fn rename_word_end(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 0;
    }

    let mut index = cursor.min(len.saturating_sub(1));
    while index < len && !is_rename_word_char(chars[index]) {
        index += 1;
    }

    if index >= len {
        return rename_line_end_cursor(buffer);
    }

    while index + 1 < len && is_rename_word_char(chars[index + 1]) {
        index += 1;
    }

    index
}

/// 找出 rename 一開始應該停留的位置，預設會停在副檔名前，方便先改主檔名。
///
/// 參數：
/// - `name: &str`，目前選取項目的原始名稱。
///
/// 回傳：`usize`，以字元數表示的初始游標位置。
fn rename_basename_cursor(name: &str) -> usize {
    let dot_index = name.rfind('.').filter(|index| *index > 0);
    match dot_index {
        Some(index) => name[..index].chars().count(),
        None => name.chars().count(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tempfile::tempdir;

    use super::{
        App, BookmarkPrompt, ClipboardOperation, FilterState, PendingAction, RenameMode,
        VisualSelectionState, help_entries, rename_basename_cursor, rename_next_word_start,
        rename_previous_word_start, rename_word_end,
    };
    use crate::{
        config::{AppConfig, LoadedConfig, StartupSort},
        file_manager::{
            layout::{LayoutNode, SplitDirection},
            open::LaunchMode,
            pane::SortMode,
        },
        theme::ThemePreset,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::{fs, thread, time::Duration};

    fn default_loaded_config() -> LoadedConfig {
        LoadedConfig {
            config: AppConfig::default(),
            source: None,
        }
    }

    fn wait_for_global_search(app: &mut App) {
        for _ in 0..50 {
            app.poll_background_tasks();
            if app
                .global_search
                .as_ref()
                .is_some_and(|search| search.searched && !search.loading)
            {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("global search did not complete in time");
    }

    #[test]
    /// 驗證 `only_current_pane` 會只保留目前焦點窗格。
    fn app_only_keeps_focused_pane() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
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
    /// 驗證啟動設定會正確套用到第一個 pane 的隱藏檔與排序偏好。
    fn app_new_applies_startup_pane_preferences() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".hidden"), "secret").expect("hidden");
        fs::write(dir.path().join("visible.txt"), "visible").expect("visible");

        let loaded = LoadedConfig {
            config: AppConfig {
                pane: crate::config::PaneConfig {
                    show_hidden: true,
                    default_sort: StartupSort::Size,
                    default_sort_reverse: true,
                },
                ..AppConfig::default()
            },
            source: None,
        };

        let app = App::new(dir.path().to_path_buf(), loaded).expect("app");
        let pane = app.panes.get(&1).expect("pane");

        assert!(pane.show_hidden);
        assert_eq!(pane.sort_mode, SortMode::Size { reverse: true });
        assert_eq!(pane.visible_indices.len(), 2);
    }

    #[test]
    /// 驗證新分割出來的 pane 會繼承原 pane 的顯示隱藏檔與排序方式。
    fn app_split_inherits_pane_preferences() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".hidden"), "secret").expect("hidden");
        fs::write(dir.path().join("visible.txt"), "visible").expect("visible");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        {
            let pane = app.panes.get_mut(&1).expect("pane");
            pane.set_show_hidden(true);
            pane.set_sort_mode(SortMode::Modified { reverse: true });
        }

        app.split_current(SplitDirection::Vertical).expect("split");

        let pane = app.panes.get(&2).expect("new pane");
        assert!(pane.show_hidden);
        assert_eq!(pane.sort_mode, SortMode::Modified { reverse: true });
        assert_eq!(pane.visible_indices.len(), 2);
    }

    #[test]
    /// 驗證刪除確認流程在確認後會真正刪除選取項目。
    fn app_delete_confirmation_removes_selected_entry() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm delete");

        assert!(!file_path.exists());
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "trashed delete-me.txt");
    }

    #[test]
    /// 驗證移到 trash 的項目可以透過 restore 命令還原。
    fn app_restore_latest_from_trash_recovers_file() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("restore-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");
        assert!(!file_path.exists());

        app.restore_latest_from_trash().expect("restore");

        assert!(file_path.exists());
        assert_eq!(app.status, "restored restore-me.txt");
    }

    #[test]
    /// 驗證 trash 面板可以列出項目，並透過 Enter 還原目前選到的檔案。
    fn app_trash_panel_lists_and_restores_entry() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("panel-restore.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash panel");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { selected: 0, .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("restore from panel");

        assert!(file_path.exists());
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "restored panel-restore.txt");
    }

    #[test]
    /// 驗證 trash 面板可用 `D` 永久刪除目前選到的項目，且面板仍保持開啟。
    fn app_trash_panel_can_delete_selected_entry_permanently() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("purge-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash panel");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
            .expect("delete selected trash entry");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert_eq!(app.trash_store.list_entries().expect("list").len(), 0);
        assert_eq!(app.status, "deleted permanently purge-me.txt");
    }

    #[test]
    /// 驗證 trash 面板可用 `C` 永久刪除目前篩選結果的全部項目。
    fn app_trash_panel_can_clear_filtered_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm alpha");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm beta");

        app.open_trash_panel().expect("open trash panel");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start trash search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock trash filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT))
            .expect("clear filtered trash");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        let remaining = app.trash_store.list_entries().expect("list remaining");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].display_name, "beta.txt");
        assert_eq!(app.status, "deleted permanently alpha.txt");
    }

    #[test]
    /// 驗證 trash 面板可用 `V` 標記多個項目，並透過 Enter 一次還原。
    fn app_trash_panel_visual_mark_restore_multiple_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm first");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm second");

        app.open_trash_panel().expect("open trash");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("start visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .expect("extend visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("commit visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("restore marked items");

        assert!(alpha.exists());
        assert!(beta.exists());
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "restored 2 items");
    }

    #[test]
    /// 驗證從 trash 面板按 F1 打開 help 後，按 Esc 會回到原本的 trash 列表。
    fn app_help_panel_from_trash_returns_to_trash_on_escape() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("from-trash-help.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash");
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help from trash");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
    }

    #[test]
    /// 驗證從 trash 打開 help 並按 Enter 執行命令後，會回到最近的 trash 列表上下文。
    fn app_help_panel_enter_from_trash_executes_and_returns_to_trash() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("clear-via-help.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash");
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help from trash");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start help search");
        for ch in ['c', 'l', 'e', 'a', 'r'] {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type help query");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock help search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute trash clear from help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert!(
            app.trash_store
                .list_entries()
                .expect("trash entries")
                .is_empty()
        );
        assert_eq!(app.status, "cleared 1 trash items");
    }

    #[test]
    /// 驗證在一般列表按下 Enter 會依預設外部開啟規則排入文字編輯器啟動。
    fn app_enter_queues_default_open_for_text_file() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("default open");

        let launch = app.take_pending_launch().expect("launch");
        let expected = if std::env::var("EDITOR")
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            LaunchMode::TerminalBlocking
        } else {
            LaunchMode::Detached
        };
        assert_eq!(launch.mode, expected);
        assert_eq!(app.status, "opening notes.txt with editor");
    }

    #[test]
    /// 驗證按下 `O` 會打開 inline `Open with` 小視窗。
    fn app_shift_o_opens_open_picker() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
            .expect("open picker");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::OpenPicker { .. })
        ));
    }

    #[test]
    /// 驗證某些終端把 `Shift+p` 回報成 `p + Shift` 時，也能正確進入 preview mode。
    fn app_shift_p_opens_preview_mode() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::SHIFT))
            .expect("open preview with shifted p");

        assert_eq!(app.preview_focus, Some(1));
        assert_eq!(app.status, "preview mode");
    }

    #[test]
    /// 驗證選到資料夾時，預設外部開啟會走系統開啟模式，而不是終端編輯器。
    fn app_open_directory_uses_detached_system_open() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open directory");

        let launch = app.take_pending_launch().expect("launch");
        assert_eq!(launch.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證可以用 `m{key}` 記錄目前目錄，再用 `'{key}` 跳回該書籤。
    fn app_bookmark_set_and_jump_with_keys() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        let src = dir.path().join("src");
        fs::create_dir(&docs).expect("docs");
        fs::create_dir(&src).expect("src");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .go_to_path(&docs)
            .expect("go docs");

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("start bookmark set");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("save bookmark");

        app.panes
            .get_mut(&1)
            .expect("pane")
            .go_to_path(&src)
            .expect("go src");

        app.handle_key(KeyEvent::new(KeyCode::Char('\''), KeyModifiers::NONE))
            .expect("start bookmark jump");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("jump bookmark");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
        assert_eq!(app.status, "jumped to bookmark [a]");
        assert!(
            fs::read_to_string(dir.path().join("bookmark.toml"))
                .expect("bookmark file")
                .contains("a =")
        );
    }

    #[test]
    /// 驗證 `bookmark.toml` 中既有的書籤可以在啟動後直接用命令跳轉。
    fn app_bookmark_jump_command_uses_bookmark_file() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        fs::create_dir(&docs).expect("docs");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("d = \"{}\"\n", docs.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("bookmark jump d")
            .expect("jump command");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
        assert_eq!(app.status, "jumped to bookmark [d]");
    }

    #[test]
    /// 驗證等待書籤按鍵時打開 F1，離開 help 後仍能回到原本的書籤等待狀態。
    fn app_help_panel_restores_pending_bookmark_prompt() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("start bookmark set");
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close help");

        assert_eq!(app.pending_bookmark, Some(BookmarkPrompt::Set));
        assert_eq!(
            app.status,
            "bookmark: press a key to save current directory"
        );
    }

    #[test]
    /// 驗證 `:bookmark list` 會打開彈窗，並可用 Enter 跳到選中的書籤。
    fn app_bookmark_list_popup_opens_and_jumps() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\nb = \"{}\"\n", alpha.display(), beta.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("bookmark list").expect("open list");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::BookmarkList {
                pane_id: 1,
                selected: 0
            })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open bookmark");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, beta);
        assert_eq!(app.status, "jumped to bookmark [b]");
    }

    #[test]
    /// 驗證書籤列表會綁在開啟它的 pane 上，從第二個 pane 打開時也只影響第二個 pane。
    fn app_bookmark_list_is_scoped_to_focused_pane() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\nb = \"{}\"\n", alpha.display(), beta.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);

        app.execute_command("bookmark list").expect("open list");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::BookmarkList {
                pane_id: 2,
                selected: 0
            })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open bookmark");

        assert_eq!(app.panes.get(&2).expect("pane").cwd, beta);
        assert_ne!(app.panes.get(&1).expect("pane").cwd, beta);
    }

    #[test]
    /// 驗證 `Shift+;` 也能正確打開命令模式，避免不同終端的事件格式造成 `:` 失效。
    fn app_shift_semicolon_opens_command_mode() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "");
        assert_eq!(app.status, "command mode");
    }

    #[test]
    /// 驗證 `:trash restore-all` 可一次還原全部 trash 項目。
    fn app_restore_all_from_trash_recovers_every_entry() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm alpha");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm beta");

        app.restore_all_from_trash().expect("restore all");

        assert!(alpha.exists());
        assert!(beta.exists());
        assert_eq!(app.status, "restored 2 items");
    }

    #[test]
    /// 驗證 `:trash clear` 可永久清空全部 trash 項目。
    fn app_clear_trash_removes_all_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm alpha");
        app.start_delete_confirmation();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm beta");

        app.clear_trash().expect("clear trash");

        assert!(app.trash_store.list_entries().expect("entries").is_empty());
        assert_eq!(app.status, "cleared 2 trash items");
    }

    #[test]
    /// 驗證 F1 說明面板可以打開，並在面板內用 `f` 進行搜尋。
    fn app_help_panel_supports_filtering() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start help search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock help search");

        match app.pending_action.as_ref() {
            Some(PendingAction::HelpPanel { search, .. }) => {
                assert_eq!(search.buffer, "res");
                assert!(!search.editing);
            }
            other => panic!("unexpected pending action: {other:?}"),
        }
        assert_eq!(app.status, "help: res (2)");
    }

    #[test]
    /// 驗證 help 面板按下 Enter 後，會直接切到對應的互動模式。
    fn app_help_panel_enter_executes_selected_action() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_help_panel();

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute rename from help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::Rename { .. })
        ));
    }

    #[test]
    /// 驗證 help 面板中的 `:delete` 會保留 `d` 快捷鍵，並透過 Enter 進入刪除確認。
    fn app_help_panel_delete_entry_matches_delete_behavior() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-from-help.txt");
        fs::write(&file_path, "hello").expect("file");

        let entries = help_entries("");
        let delete_entry = entries
            .iter()
            .find(|entry| entry.line.command == ":delete")
            .expect("delete help entry");
        let trash_entry = entries
            .iter()
            .find(|entry| entry.line.command == ":trash")
            .expect("trash help entry");
        assert_eq!(delete_entry.line.shortcut, "d");
        assert!(trash_entry.line.shortcut.is_empty());

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_help_panel();

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start help search");
        for ch in ['d', 'e', 'l'] {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type help query");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock help search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute delete from help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { .. })
        ));
    }

    #[test]
    /// 驗證輪替主題時會切換到下一個預設值。
    fn app_cycle_theme_switches_to_next_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.cycle_theme();

        assert_eq!(app.theme_preset, ThemePreset::Forest);
        assert_eq!(app.theme, ThemePreset::Forest.into());
        assert_eq!(app.status, "theme: forest");
    }

    #[test]
    /// 驗證打開主題選擇視窗時，游標會落在目前主題。
    fn app_open_theme_picker_tracks_current_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_theme_picker();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker { selected: 0 })
        );
    }

    #[test]
    /// 驗證依主題名稱字串指定主題時會正確更新狀態。
    fn app_set_theme_by_name_updates_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.set_theme_by_name("ocean");

        assert_eq!(app.theme_preset, ThemePreset::Ocean);
        assert_eq!(app.theme, ThemePreset::Ocean.into());
        assert_eq!(app.status, "theme: ocean");
    }

    #[test]
    /// 驗證在主題選擇視窗按下 Enter 後會套用目前選取的主題。
    fn app_theme_picker_confirm_applies_selected_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::ThemePicker { selected: 2 });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("apply theme");

        assert_eq!(app.theme_preset, ThemePreset::Ocean);
        assert_eq!(app.theme, ThemePreset::Ocean.into());
        assert_eq!(app.status, "theme: ocean");
    }

    #[test]
    /// 驗證打開重新命名視窗時，會帶入目前選取項目的原名稱與預設輸入值。
    fn app_start_rename_opens_dialog_with_selected_name() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_rename();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("alpha.txt"),
                buffer: String::from("alpha.txt"),
                cursor: 5,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證在重新命名視窗按下 Enter 後會套用新的檔名。
    fn app_rename_confirm_updates_selected_entry() {
        let dir = tempdir().expect("tempdir");
        let old_path = dir.path().join("alpha.txt");
        let new_path = dir.path().join("beta.txt");
        fs::write(&old_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("beta.txt"),
            cursor: 4,
            mode: RenameMode::Insert,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("rename");

        assert!(!old_path.exists());
        assert!(new_path.exists());
        assert_eq!(app.status, "renamed alpha.txt -> beta.txt");
    }

    #[test]
    /// 驗證 rename 預設游標會停在副檔名前，方便優先修改主檔名。
    fn rename_basename_cursor_stops_before_extension() {
        assert_eq!(rename_basename_cursor("alpha.txt"), 5);
        assert_eq!(rename_basename_cursor("archive.tar.gz"), 11);
        assert_eq!(rename_basename_cursor(".gitignore"), 10);
        assert_eq!(rename_basename_cursor("folder"), 6);
    }

    #[test]
    /// 驗證 rename 可以在 insert 與 normal 模式之間切換，並保留游標位置。
    fn rename_mode_switches_between_insert_and_normal() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_rename();

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("switch to normal");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("alpha.txt"),
                buffer: String::from("alpha.txt"),
                cursor: 5,
                mode: RenameMode::Normal,
            })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("move left");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
            .expect("back to insert");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("alpha.txt"),
                buffer: String::from("alpha.txt"),
                cursor: 4,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證 rename 的 Vim 單字移動會依照檔名分隔符正確跳轉。
    fn rename_word_motion_helpers_follow_filename_segments() {
        let name = "my-long_file.txt";

        assert_eq!(rename_next_word_start(name, 0), 3);
        assert_eq!(rename_next_word_start(name, 3), 8);
        assert_eq!(rename_next_word_start(name, 8), 13);

        assert_eq!(rename_previous_word_start(name, 13), 8);
        assert_eq!(rename_previous_word_start(name, 8), 3);
        assert_eq!(rename_previous_word_start(name, 3), 0);

        assert_eq!(rename_word_end(name, 0), 1);
        assert_eq!(rename_word_end(name, 3), 6);
        assert_eq!(rename_word_end(name, 8), 11);
        assert_eq!(rename_word_end(name, 12), 15);
    }

    #[test]
    /// 驗證 rename 的 normal 模式支援 `w`、`b`、`e`、`a`、`A` 這些 Vim 風格操作。
    fn rename_normal_mode_supports_vim_word_motions_and_insert_shortcuts() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("my-long_file.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_rename();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("switch to normal");

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("move to previous word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("move to next word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("move to word end");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("append after cursor");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("my-long_file.txt"),
                buffer: String::from("my-long_file.txt"),
                cursor: 16,
                mode: RenameMode::Insert,
            })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("back to normal");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE))
            .expect("jump to start");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("jump to next word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("jump to end of word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("append inside basename");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("my-long_file.txt"),
                buffer: String::from("my-long_file.txt"),
                cursor: 7,
                mode: RenameMode::Insert,
            })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("back to normal again");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE))
            .expect("append at end");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("my-long_file.txt"),
                buffer: String::from("my-long_file.txt"),
                cursor: 16,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證 `yy` 複製後可以用 `p` 把檔案貼到另一個目錄，且來源會保留。
    fn app_copy_and_paste_preserves_source_file() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("alpha.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("pending copy");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy");

        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.operation),
            Some(ClipboardOperation::Copy)
        );
        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.entries.len()),
            Some(1)
        );

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("paste");

        assert!(source_file.exists());
        assert!(target_dir.join("alpha.txt").exists());
        assert_eq!(app.status, "pasted copy: 1 item");
    }

    #[test]
    /// 驗證 `x` 剪下後可以用 `p` 移動檔案，且剪貼簿會在成功後清空。
    fn app_cut_and_paste_moves_file_and_clears_clipboard() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("beta.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .expect("cut");

        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.operation),
            Some(ClipboardOperation::Cut)
        );
        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.entries.len()),
            Some(1)
        );

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("paste");

        assert!(!source_file.exists());
        assert!(target_dir.join("beta.txt").exists());
        assert!(app.clipboard.is_none());
        assert_eq!(app.status, "moved: 1 item");
    }

    #[test]
    /// 驗證 `:move <path>` 會把目前選取的檔案直接移到指定目錄。
    fn app_move_command_moves_selected_entry_to_target_dir() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("gamma.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.execute_command(&format!("move {}", target_dir.display()))
            .expect("move command");

        assert!(!source_file.exists());
        assert!(target_dir.join("gamma.txt").exists());
        assert_eq!(
            app.status,
            format!("moved 1 item -> {}", target_dir.display())
        );
    }

    #[test]
    /// 驗證 `:move-panel <id>` 會把目前選取的檔案移到指定 pane 的目錄。
    fn app_move_panel_command_moves_selected_entry_to_target_pane_dir() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("delta.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.focus_previous_pane();

        app.execute_command("move-panel 2").expect("move panel");

        assert!(!source_file.exists());
        assert!(target_dir.join("delta.txt").exists());
        assert_eq!(
            app.status,
            format!("moved 1 item -> {}", target_dir.display())
        );
    }

    #[test]
    /// 驗證 `:move-panel <id>` 若指定不存在的 pane，會提示目前可用的 pane 編號。
    fn app_move_panel_command_reports_available_panes_for_unknown_target() {
        let dir = tempdir().expect("tempdir");
        let source_file = dir.path().join("epsilon.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("move-panel 9").expect("move panel");

        assert!(source_file.exists());
        assert_eq!(app.status, "unknown pane 9. available: 1");
    }

    #[test]
    /// 驗證按下 `o` 後會打開建立新檔案的 inline 輸入框。
    fn app_start_create_entry_opens_inline_editor() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.start_create_entry();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::CreateEntry {
                pane_id: 1,
                buffer: String::new(),
                cursor: 0,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證命令模式可以直接建立一般檔案與結尾 `/` 的資料夾。
    fn app_create_commands_create_entries_without_inline_prompt() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.execute_command("create alpha.txt")
            .expect("create file");
        assert!(dir.path().join("alpha.txt").exists());
        assert_eq!(app.status, "created file: alpha.txt");

        app.execute_command("create docs/").expect("create dir");
        assert!(dir.path().join("docs").is_dir());
        assert_eq!(app.status, "created directory: docs/");
    }

    #[test]
    /// 驗證建立流程的 inline 輸入框在 Enter 後會真的建立檔案。
    fn app_create_file_confirm_creates_entry() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::CreateEntry {
            pane_id: 1,
            buffer: String::from("draft.md"),
            cursor: 8,
            mode: RenameMode::Insert,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("create file");

        assert!(dir.path().join("draft.md").exists());
        assert_eq!(app.status, "created file: draft.md");
    }

    #[test]
    /// 驗證建立流程支援巢狀路徑，會先補齊父目錄再建立檔案。
    fn app_create_nested_file_from_inline_prompt() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::CreateEntry {
            pane_id: 1,
            buffer: String::from("test/gg.txt"),
            cursor: 11,
            mode: RenameMode::Insert,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("create nested file");

        assert!(dir.path().join("test").is_dir());
        assert!(dir.path().join("test").join("gg.txt").exists());
        assert_eq!(app.status, "created file: test/gg.txt");
    }

    #[test]
    /// 驗證 filter 輸入時會立即過濾列表，第一次 Esc 只收起輸入框。
    fn app_filter_esc_once_keeps_filtered_results() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close input");

        let pane = app.panes.get(&1).expect("pane");
        let visible_names: Vec<String> = pane
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();

        assert_eq!(
            visible_names,
            vec![String::from("alpha.txt"), String::from("beta.txt")]
        );
        assert!(app.filter.as_ref().is_some_and(|filter| !filter.editing));
        assert_eq!(app.status, "filter locked: a");
    }

    #[test]
    /// 驗證 filter 第二次 Esc 會完全清掉過濾狀態並恢復一般畫面。
    fn app_filter_esc_twice_clears_filter() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close input");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear filter");

        let pane = app.panes.get(&1).expect("pane");
        let visible_names: Vec<String> = pane
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();

        assert_eq!(
            visible_names,
            vec![String::from("alpha.txt"), String::from("beta.txt")]
        );
        assert!(app.filter.is_none());
        assert!(!pane.has_active_filter());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證連續重新開啟 filter 時，不會殘留上一輪輸入的關鍵字。
    fn app_reopening_filter_starts_with_empty_buffer() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close input");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear filter");

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("reopen filter");

        assert_eq!(
            app.filter,
            Some(FilterState {
                pane_id: 1,
                buffer: String::new(),
                editing: true,
            })
        );
        assert_eq!(app.status, "filter: all");
    }

    #[test]
    /// 驗證按下 `.` 後會顯示隱藏檔，並可與 filter 一起使用。
    fn app_toggle_hidden_reveals_hidden_entries_and_works_with_filter() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".secret"), "s").expect("hidden");
        fs::write(dir.path().join("alpha.txt"), "a").expect("normal");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        let initial_names: Vec<String> = app
            .panes
            .get(&1)
            .expect("pane")
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();
        assert_eq!(initial_names, vec![String::from("alpha.txt")]);

        app.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
            .expect("toggle hidden");
        assert_eq!(app.status, "showing hidden files");

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("filter hidden");

        let filtered_names: Vec<String> = app
            .panes
            .get(&1)
            .expect("pane")
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();
        assert_eq!(filtered_names, vec![String::from(".secret")]);
    }

    #[test]
    /// 驗證按下 `,` 後可以用排序面板快捷鍵套用排序模式。
    fn app_sort_picker_applies_selected_mode() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("small.txt"), "a").expect("small");
        fs::write(dir.path().join("large.txt"), "abcdef").expect("large");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
            .expect("open sort picker");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::SortPicker { pane_id: 1 })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("sort by size");
        assert_eq!(app.status, "sort: size");
        assert_eq!(
            app.panes.get(&1).expect("pane").sort_mode,
            SortMode::Size { reverse: false }
        );

        app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
            .expect("open sort picker again");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE))
            .expect("sort by modified reverse");
        assert_eq!(app.status, "sort: modified (reverse)");
        assert_eq!(
            app.panes.get(&1).expect("pane").sort_mode,
            SortMode::Modified { reverse: true }
        );
    }

    #[test]
    /// 驗證 sort panel 也接受 `m + Shift` 這類終端事件，正確套用反向排序。
    fn app_sort_picker_shift_m_applies_reverse_modified() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
            .expect("open sort picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::SHIFT))
            .expect("sort by modified reverse");

        assert_eq!(app.status, "sort: modified (reverse)");
        assert_eq!(
            app.panes.get(&1).expect("pane").sort_mode,
            SortMode::Modified { reverse: true }
        );
    }

    #[test]
    /// 驗證進入 preview mode 後，`j/k` 會改成捲動 preview，Esc 會離開該模式。
    fn app_preview_mode_scrolls_and_exits_cleanly() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("notes.txt"),
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
        )
        .expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(4);

        app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE))
            .expect("open preview");
        assert_eq!(app.preview_focus, Some(1));
        assert_eq!(app.status, "preview mode");

        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("scroll down");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("scroll up");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("leave preview");
        assert_eq!(app.preview_focus, None);
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 preview mode 支援半頁捲動與 `gg/G` 的上下端跳轉。
    fn app_preview_mode_supports_paging_and_boundary_jumps() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("readme.md"),
            "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n",
        )
        .expect("readme");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(4);
        app.open_preview_focus();

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("page down");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE))
            .expect("bottom");
        let bottom_scroll = app.panes.get(&1).expect("pane").preview_scroll;
        assert!(bottom_scroll > 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("pending g");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("top");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("page down again");
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .expect("page up");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
    }

    #[test]
    /// 驗證 preview mode 中的 `/` 會打開搜尋輸入框，並在輸入時立即更新搜尋結果。
    fn app_preview_search_opens_and_tracks_matches() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("notes.txt"),
            "alpha\nbeta\ngamma\nbeta line\n",
        )
        .expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(3);
        app.open_preview_focus();

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("open preview search");
        assert!(
            app.preview_search
                .as_ref()
                .is_some_and(|search| search.editing)
        );

        for ch in ['b', 'e', 't', 'a'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type search");
        }
        assert_eq!(app.panes.get(&1).expect("pane").preview_match_count(), 2);
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 6);
        assert_eq!(app.status, "preview search: beta (2)");
    }

    #[test]
    /// 驗證 preview search 支援 `n/N` 跳轉命中結果，Esc 先清搜尋再離開 preview mode。
    fn app_preview_search_navigation_and_escape_flow() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("readme.md"),
            "zero\nmatch one\nmiddle\nmatch two\nend\n",
        )
        .expect("readme");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(3);
        app.open_preview_focus();
        app.open_preview_search_input();
        for ch in ['m', 'a', 't', 'c', 'h'] {
            app.handle_preview_search_input_key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ))
            .expect("type query");
        }
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock search");

        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 6);

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("next match");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 7);

        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE))
            .expect("previous match");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 6);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear search");
        assert_eq!(app.preview_focus, Some(1));
        assert!(!app.panes.get(&1).expect("pane").has_preview_search());
        assert_eq!(app.status, "preview search cleared");

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("leave preview");
        assert_eq!(app.preview_focus, None);
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 preview mode 中按下 `Ctrl-w l` 可以切換到另一個 pane。
    fn app_preview_mode_supports_ctrl_w_pane_navigation() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "preview target").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);

        app.open_preview_focus();
        assert_eq!(app.preview_focus, Some(2));

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
            .expect("start ctrl-w");
        assert!(app.awaiting_ctrl_w);
        assert_eq!(app.status, "Ctrl-w");

        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("focus previous pane");
        assert_eq!(app.focused_pane, 1);
        assert_eq!(app.preview_focus, Some(2));
    }

    #[test]
    /// 驗證 preview 狀態只屬於原本的 pane，切到其他 pane 不會被強制進入 preview mode。
    fn app_preview_mode_is_scoped_to_its_own_pane() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);

        app.open_preview_focus();
        assert_eq!(app.preview_focus, Some(2));

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
            .expect("start ctrl-w");
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("focus previous pane");

        assert_eq!(app.focused_pane, 1);
        assert_eq!(app.preview_focus, Some(2));

        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move in normal mode");

        assert_eq!(app.panes.get(&1).expect("pane").selected, 1);
        assert_eq!(app.panes.get(&2).expect("pane").preview_scroll, 0);
    }

    #[test]
    /// 驗證 global search 在輸入階段不會立即掃描，按下 Enter 後才真正執行搜尋。
    fn app_global_search_filters_nested_entries() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("Readme.md"), "doc").expect("readme");
        fs::create_dir(dir.path().join("src")).expect("src");
        fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").expect("main");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");

        for ch in ['r', 'e', 'a', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        let search = app.global_search.as_ref().expect("search");
        assert!(search.editing);
        assert_eq!(search.results.len(), 0);
        assert!(!search.searched);
        assert_eq!(
            app.status,
            "global search (insert): read (press Enter to search)"
        );

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("run search");
        wait_for_global_search(&mut app);
        let search = app.global_search.as_ref().expect("search after run");
        assert!(!search.editing);
        assert!(search.searched);
        assert_eq!(search.results.len(), 1);
        assert_eq!(search.results[0].relative_path, "docs/Readme.md");
        assert_eq!(app.status, "global search (normal): read (1)");
    }

    #[test]
    /// 驗證 global search 提交查詢後，再按一次 Enter 會跳到選中的搜尋結果。
    fn app_global_search_enter_reveals_selected_file() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");
        for ch in ['g', 'u', 'i', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock search");
        wait_for_global_search(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open result");

        assert!(app.global_search.is_none());
        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.cwd, dir.path().join("docs"));
        assert_eq!(
            pane.selected_entry().map(|entry| entry.display_name()),
            Some(String::from("guide.md"))
        );
        assert_eq!(app.status, "search opened: docs/guide.md");
    }

    #[test]
    /// 驗證在 global search 執行中按下 Esc，會關閉介面並要求背景搜尋停止。
    fn app_global_search_escape_cancels_background_work() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");
        for ch in ['g', 'u', 'i', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("start search");
        let cancelled = app
            .global_search_cancelled
            .as_ref()
            .expect("cancel flag")
            .clone();

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel search");

        assert!(app.global_search.is_none());
        assert!(app.global_search_rx.is_none());
        assert!(app.global_search_cancelled.is_none());
        assert!(cancelled.load(Ordering::Relaxed));
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證在 global search 結果列表中按下 h，會安全返回一般列表。
    fn app_global_search_h_leaves_results_list() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");
        for ch in ['g', 'u', 'i', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("start search");
        wait_for_global_search(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("leave search");

        assert!(app.global_search.is_none());
        assert!(app.global_search_rx.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證可以用 `V` 視覺標記多個項目，並一次放進剪貼簿。
    fn app_visual_marked_entries_copy_into_clipboard_as_batch() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("pending copy");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy batch");

        let clipboard = app.clipboard.as_ref().expect("clipboard");
        assert_eq!(clipboard.operation, ClipboardOperation::Copy);
        assert_eq!(clipboard.entries.len(), 2);
        assert_eq!(app.status, "copied 2 items");
    }

    #[test]
    /// 驗證 `V` 視覺標記多個項目後，刪除確認會一次刪掉整批項目。
    fn app_visual_marked_entries_delete_as_batch() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "a").expect("alpha");
        fs::write(&beta, "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.start_delete_confirmation();

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { ref target_name, .. }) if target_name == "2 items"
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm delete batch");

        assert!(!alpha.exists());
        assert!(!beta.exists());
        assert_eq!(app.status, "trashed 2 items");
    }

    #[test]
    /// 驗證 `V` 進入 visual selection 後，移動游標再按一次 `V` 會提交整段標記。
    fn app_visual_selection_commits_range_marks() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");
        fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down again");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");

        let pane = app.panes.get(&1).expect("pane");
        assert!(app.visual_selection.is_none());
        assert_eq!(pane.marked_count(), 3);
        assert_eq!(app.status, "marked 3 items");
    }

    #[test]
    /// 驗證某些終端把 `Shift+v` 回報成 `v + Shift` 時，也能正確進入 visual selection。
    fn app_shift_v_opens_visual_selection() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SHIFT))
            .expect("open visual with shifted v");

        assert_eq!(
            app.visual_selection,
            Some(VisualSelectionState {
                pane_id: 1,
                anchor: 0,
                current: 0,
            })
        );
        assert_eq!(app.status, "visual: range selection");
    }

    #[test]
    /// 驗證某些終端把 `Shift+g` 回報成 `g + Shift` 時，也能正確執行 `G` 跳到列表底部。
    fn app_shift_g_jumps_to_bottom() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");
        fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT))
            .expect("jump bottom with shifted g");

        assert_eq!(app.panes.get(&1).expect("pane").selected, 2);
        assert_eq!(app.status, "jumped to bottom");
    }

    #[test]
    /// 驗證 visual selection 按下 `Esc` 會先提交這一段範圍並離開選取模式。
    fn app_visual_selection_escape_commits_current_range() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");
        fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit first range");

        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move to third");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open second visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("move back");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("commit second visual");

        let pane = app.panes.get(&1).expect("pane");
        assert!(app.visual_selection.is_none());
        assert_eq!(pane.marked_count(), 3);
        assert_eq!(app.status, "marked 1 items");
    }

    #[test]
    /// 驗證離開選取模式後再按一次 `Esc`，會清掉目前所有已提交標記。
    fn app_escape_in_normal_mode_clears_all_marks() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear all marks");

        let pane = app.panes.get(&1).expect("pane");
        assert!(app.visual_selection.is_none());
        assert_eq!(pane.marked_count(), 0);
        assert_eq!(app.status, "cleared 2 marks");
    }
}
