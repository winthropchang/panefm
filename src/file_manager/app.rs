use std::{collections::BTreeMap, io, path::PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    config::{AppConfig, LoadedConfig},
    theme::{Theme, ThemePreset},
};

use super::{
    layout::{LayoutNode, SplitDirection},
    pane::{PaneState, SortMode},
    ui::{
        InlineEditorState, centered_rect, render_confirm_dialog, render_filter_input, render_pane,
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

/// 記錄目前是否處於範圍標記模式，以及起點和目前游標位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisualSelectionState {
    pub(crate) pane_id: usize,
    pub(crate) anchor: usize,
    pub(crate) current: usize,
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
    pub(crate) clipboard: Option<ClipboardState>,
    pub(crate) filter: Option<FilterState>,
    pub(crate) preview_search: Option<PreviewSearchState>,
    pub(crate) visual_selection: Option<VisualSelectionState>,
    pub(crate) pending_action: Option<PendingAction>,
    pub(crate) preview_focus: Option<usize>,
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
        let pane = PaneState::new(cwd)?;
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);
        let theme_preset = loaded_config.config.theme_preset;
        let startup_status = match loaded_config.source {
            Some(path) => format!("loaded config: {}", path.display()),
            None => String::from("normal mode"),
        };

        Ok(Self {
            config: loaded_config.config,
            theme: theme_preset.into(),
            theme_preset,
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
            clipboard: None,
            filter: None,
            preview_search: None,
            visual_selection: None,
            pending_action: None,
            preview_focus: None,
        })
    }

    /// 處理一般輸入事件的總入口。
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.pending_action.is_some() {
            return self.handle_pending_action_key(key);
        }
        if self.filter.as_ref().is_some_and(|filter| filter.editing) {
            return self.handle_filter_input_key(key);
        }
        if self.preview_search.as_ref().is_some_and(|search| search.editing) {
            return self.handle_preview_search_input_key(key);
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
        if self.preview_focus == Some(self.focused_pane) {
            return self.handle_preview_key(key);
        }

        let should_continue = match key.code {
            KeyCode::Char('q') => false,
            KeyCode::Char(':') => {
                self.command_mode = true;
                self.command_buffer.clear();
                self.status = String::from("command mode");
                self.pending_g = false;
                self.pending_y = false;
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
            KeyCode::Char('l') | KeyCode::Enter => {
                self.current_pane_mut()?.enter_selected()?;
                self.status = String::from("opened directory");
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
            KeyCode::Char('G') => {
                self.current_pane_mut()?.move_bottom();
                self.pending_g = false;
                self.pending_y = false;
                self.status = String::from("jumped to bottom");
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
            KeyCode::Char('V') => {
                self.open_visual_selection()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('P') => {
                self.open_preview_focus();
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
                if self.pending_y {
                    self.copy_selected();
                    self.pending_y = false;
                } else {
                    self.pending_y = true;
                    self.status = String::from("pending: y");
                }
                true
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.awaiting_ctrl_w = true;
                self.pending_g = false;
                self.pending_y = false;
                self.status = String::from("Ctrl-w");
                true
            }
            KeyCode::Esc => {
                self.pending_g = false;
                self.pending_y = false;
                self.handle_escape_in_normal_mode();
                true
            }
            _ => {
                self.pending_g = false;
                self.pending_y = false;
                true
            }
        };

        Ok(should_continue)
    }

    /// 處理 preview mode 的鍵盤輸入，讓使用者可以專心在預覽區捲動內容。
    pub(crate) fn handle_preview_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('P') => {
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
            KeyCode::Char('N') => {
                self.pending_g = false;
                self.status = self.jump_preview_match(false)?;
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
            KeyCode::Char('G') => {
                self.current_pane_mut()?.scroll_preview_bottom();
                self.pending_g = false;
                self.status = String::from("preview: bottom");
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
        match key.code {
            KeyCode::Char('V') => {
                self.commit_visual_selection()?;
            }
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
            KeyCode::Char('G') => {
                self.current_pane_mut()?.move_bottom();
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
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
            PendingAction::SortPicker { pane_id } => match key.code {
                KeyCode::Char('m') => self.apply_sort_mode(pane_id, SortMode::Modified { reverse: false })?,
                KeyCode::Char('M') => self.apply_sort_mode(pane_id, SortMode::Modified { reverse: true })?,
                KeyCode::Char('b') => self.apply_sort_mode(pane_id, SortMode::Created { reverse: false })?,
                KeyCode::Char('B') => self.apply_sort_mode(pane_id, SortMode::Created { reverse: true })?,
                KeyCode::Char('a') => self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: false })?,
                KeyCode::Char('A') => self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: true })?,
                KeyCode::Char('n') => self.apply_sort_mode(pane_id, SortMode::Natural { reverse: false })?,
                KeyCode::Char('N') => self.apply_sort_mode(pane_id, SortMode::Natural { reverse: true })?,
                KeyCode::Char('e') => self.apply_sort_mode(pane_id, SortMode::Extension { reverse: false })?,
                KeyCode::Char('E') => self.apply_sort_mode(pane_id, SortMode::Extension { reverse: true })?,
                KeyCode::Char('s') => self.apply_sort_mode(pane_id, SortMode::Size { reverse: false })?,
                KeyCode::Char('S') => self.apply_sort_mode(pane_id, SortMode::Size { reverse: true })?,
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
                    KeyCode::Char('A') => {
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
                    KeyCode::Char('A') => {
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
            "unmark" | "unmark-all" => self.clear_marks_in_focused_pane()?,
            "preview" => self.open_preview_focus(),
            "preview-search" => self.open_preview_search_input(),
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
        self.status = preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
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
            self.status = String::from("nothing selected to delete");
            return;
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to delete");
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
        self.status = format!("confirm delete {target_name}: y/n");
    }

    /// 真正執行刪除目前待確認項目的檔案系統操作。
    pub(crate) fn confirm_delete(&mut self, pane_id: usize, target_name: &str) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        match pane.delete_selected_or_marked() {
            Ok(removed_names) if removed_names.is_empty() => {
                self.status = String::from("nothing selected to delete");
            }
            Ok(removed_names) if removed_names.len() == 1 => {
                self.status = format!("deleted {}", removed_names[0]);
            }
            Ok(removed_names) => {
                self.status = format!("deleted {} items", removed_names.len());
            }
            Err(error) => self.status = format!("failed to delete {target_name}: {error}"),
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
    pub(crate) fn confirm_create_entry(
        &mut self,
        pane_id: usize,
        path: &str,
    ) -> io::Result<()> {
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
                    ClipboardOperation::Copy => pane.copy_entry_into_current_dir(&entry.source_path),
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
        for (pane_id, rect) in pane_rects {
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
                let pane_cursor = render_pane(
                    frame,
                    rect,
                    pane_id,
                    pane,
                    pane_id == self.focused_pane,
                    self.preview_focus == Some(pane_id),
                    self.visual_selection.as_ref().and_then(|selection| {
                        (selection.pane_id == pane_id).then_some((selection.anchor, selection.current))
                    }),
                    self.theme,
                    rename_buffer,
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
            Span::raw(" delete  "),
            Span::styled("r", self.theme.accent_style()),
            Span::raw(" rename  "),
            Span::styled("P", self.theme.accent_style()),
            Span::raw(" preview  "),
            Span::styled("/ n N", self.theme.accent_style()),
            Span::raw(" search  "),
            Span::styled("a", self.theme.accent_style()),
            Span::raw(" create  "),
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

        if self.command_mode {
            let area = centered_rect(frame.area(), 70, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!(":{}", self.command_buffer))
                    .block(Block::default().title("Command").borders(Borders::ALL)),
                area,
            );
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
            _ => None,
        }
    }
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
    use tempfile::tempdir;

    use super::{
        App, ClipboardOperation, FilterState, PendingAction, RenameMode, rename_basename_cursor, rename_next_word_start,
        rename_previous_word_start, rename_word_end,
    };
    use crate::{
        config::{AppConfig, LoadedConfig},
        file_manager::{
            layout::{LayoutNode, SplitDirection},
            pane::SortMode,
        },
        theme::ThemePreset,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;

    fn default_loaded_config() -> LoadedConfig {
        LoadedConfig {
            config: AppConfig::default(),
            source: None,
        }
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
        assert_eq!(app.status, "deleted delete-me.txt");
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

        let mut app =
            App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("pending copy");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy");

        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.operation),
            Some(ClipboardOperation::Copy)
        );
        assert_eq!(app.clipboard.as_ref().map(|entry| entry.entries.len()), Some(1));

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut().expect("pane").reload().expect("reload");
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

        let mut app =
            App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .expect("cut");

        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.operation),
            Some(ClipboardOperation::Cut)
        );
        assert_eq!(app.clipboard.as_ref().map(|entry| entry.entries.len()), Some(1));

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut().expect("pane").reload().expect("reload");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("paste");

        assert!(!source_file.exists());
        assert!(target_dir.join("beta.txt").exists());
        assert!(app.clipboard.is_none());
        assert_eq!(app.status, "moved: 1 item");
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

        app.execute_command("create alpha.txt").expect("create file");
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

        assert_eq!(visible_names, vec![String::from("alpha.txt"), String::from("beta.txt")]);
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
        assert!(app.preview_search.as_ref().is_some_and(|search| search.editing));

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
            app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
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
        assert_eq!(app.status, "deleted 2 items");
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
