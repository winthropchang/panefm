//! 主題、排序、Linemode、Go、視窗、剪貼、垃圾桶、任務、比對等選單面板的啟動與套用。

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;

use super::super::*;
use crate::config::persist_theme;
use crate::theme::ThemePreset;

impl App {
    /// 將主題切換到下一個內建預設值。
    pub(crate) fn cycle_theme(&mut self) {
        let next = self.theme_preset.next();
        self.apply_theme(next);
    }

    /// 打開主題選擇視窗，並將選項焦點設在目前主題。
    pub(crate) fn open_theme_picker(&mut self) {
        let original = self.theme_preset;
        let selected = ThemePreset::ALL
            .iter()
            .position(|preset| *preset == original)
            .unwrap_or(0);
        self.pending_action = Some(PendingAction::ThemePicker { selected, original });
        self.status = String::from("theme picker: use j/k preview, l apply, h cancel");
    }

    /// 即時預覽主題列表目前選到的色盤，但不會寫入設定檔。
    ///
    /// 參數：
    /// - `selected: usize`，`ThemePreset::ALL` 中目前選取的索引。
    /// - `original: ThemePreset`，開啟列表前使用的主題，供取消操作時還原。
    ///
    /// 回傳：`()`，函數會更新畫面主題並保留主題列表狀態。
    pub(crate) fn preview_theme_picker_selection(
        &mut self,
        selected: usize,
        original: ThemePreset,
    ) {
        let preset = ThemePreset::ALL[selected];
        self.theme = preset.into();
        self.pending_action = Some(PendingAction::ThemePicker { selected, original });
        self.status = format!("theme preview: {}", preset.name());
    }

    /// 打開底部排序面板，等待使用者輸入排序快捷鍵。
    pub(crate) fn open_sort_picker(&mut self) {
        self.pending_action = Some(PendingAction::SortPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("sort: choose a key from the panel");
    }

    /// 打開底部 `g` 系列命令面板，供 `gg`、`gt` 等 leader 指令共用。
    pub(crate) fn open_go_picker(&mut self) {
        self.pending_action = Some(PendingAction::GoPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("go: choose g/t/d/k/l from the panel");
    }

    /// 打開底部 panel 操作面板，讓使用者可視化選擇 `w` 的第二個按鍵。
    pub(crate) fn open_window_picker(&mut self) {
        self.pending_action = Some(PendingAction::WindowPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("panel: choose h/j/k/l/c/o/t/d from the panel");
    }

    /// 開啟 EasyMotion 兩階段精準跳轉模式。
    /// 進入等待輸入目標字元階段，畫面維持原樣，待輸入開頭字母後才指派標籤與高亮。
    pub(crate) fn open_easymotion(&mut self) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            return;
        };
        let visible_total = pane.visible_indices.len();
        if visible_total == 0 {
            self.status = String::from("easymotion: directory is empty");
            return;
        }

        self.pending_action = Some(PendingAction::EasyMotion {
            pane_id: self.focused_pane,
            target_char: None,
            labels: Vec::new(),
        });
        self.status = String::from("-- EASYMOTION -- (type target char, Esc to cancel)");
    }

    /// 打開底部 Move / LineMode 面板，等待使用者輸入搬移或欄位顯示模式。
    pub(crate) fn open_linemode_picker(&mut self) {
        self.pending_action = Some(PendingAction::LineModePicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("move / linemode: choose a key from the panel");
    }

    /// 打開底部 Yank 面板，等待使用者選擇複製到剪貼簿或複製到指定視窗。
    pub(crate) fn open_yank_picker(&mut self) {
        self.pending_action = Some(PendingAction::YankPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from(
            "yank: choose a key from the panel (y: clipboard, p: panel, 1..9: pane id)",
        );
    }

    /// 打開書籤功能面板，列出目前可用的書籤操作。
    pub(crate) fn open_bookmark_picker(&mut self) {
        self.pending_action = Some(PendingAction::BookmarkPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("bookmark: choose a/g/d/D from the panel");
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

    /// 打開 `t` 系列命令面板，讓使用者選擇主題或 Trash 功能。
    ///
    /// 參數：無，功能固定作用於目前取得焦點的 panel。
    /// 回傳：`()`, 只更新目前的互動狀態與提示文字。
    pub(crate) fn open_theme_command_picker(&mut self) {
        self.pending_action = Some(PendingAction::ThemeCommandPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("theme/trash: choose l/n/t/u from the panel");
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
            custom_title: None,
            custom_entries: None,
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 打開 task 面板，查看目前 pane 最近執行過的任務與狀態。
    pub(crate) fn open_task_panel(&mut self) {
        let count = self.tasks_for_pane(self.focused_pane).len();
        self.pending_action = Some(PendingAction::TaskPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
            marked_ids: Vec::new(),
            visual_anchor: None,
        });
        self.status = task_panel_status("", count, 0, false, 0);
    }

    /// 打開全螢幕 N 路目錄與檔案差異比對工作區 (Diff Matrix)。
    pub(crate) fn open_diff_matrix(&mut self, target_pane_ids: Option<Vec<usize>>) -> Result<()> {
        let pane_ids = match target_pane_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => self.panes.keys().copied().collect::<Vec<_>>(),
        };

        if pane_ids.len() < 2 {
            self.status = String::from(
                "diff requires at least 2 open panels (e.g. use Ctrl+s / Ctrl+v to split first)",
            );
            return Ok(());
        }

        let mut valid_ids = Vec::new();
        let mut roots = Vec::new();
        let mut labels = Vec::new();

        for id in pane_ids {
            if let Some(pane) = self.panes.get(&id) {
                valid_ids.push(id);
                roots.push(pane.cwd.clone());
                let tail = pane
                    .cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| pane.cwd.display().to_string());
                labels.push(tail);
            }
        }

        if roots.len() < 2 {
            self.status = String::from("diff requires at least 2 valid panels");
            return Ok(());
        }

        // 取消任何先前的 diff background job
        if let Some(cancelled) = self.diff_job_cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        self.diff_job_cancelled = Some(cancelled.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        self.diff_job_rx = Some(rx);

        spawn_background_diff(roots.clone(), true, true, cancelled, tx);

        let diff_state = DiffMatrixState::new_loading(valid_ids, roots, labels);
        self.pending_action = Some(PendingAction::DiffMatrix(diff_state));
        self.status = format!("diff matrix: scanning {} panels...", self.panes.len());
        Ok(())
    }

    /// 輪詢背景目錄比對工作的接收端，非阻塞更新差異矩陣。
    pub(crate) fn poll_diff_job(&mut self) -> bool {
        let Some(receiver) = &self.diff_job_rx else {
            return false;
        };
        let messages: Vec<DiffJobEvent> = receiver.try_iter().collect();
        if messages.is_empty() {
            return false;
        }

        for message in messages {
            match message {
                DiffJobEvent::Discovered(count) => {
                    if let Some(PendingAction::DiffMatrix(state)) = &mut self.pending_action {
                        state.discovered_count = count;
                    }
                }
                DiffJobEvent::Done(rows) => {
                    let count = rows.len();
                    if let Some(PendingAction::DiffMatrix(state)) = &mut self.pending_action {
                        state.set_completed_rows(rows);
                        self.status =
                            format!("diff matrix: compared {} items (press q to exit)", count);
                    }
                    self.diff_job_rx = None;
                    self.diff_job_cancelled = None;
                    break;
                }
                DiffJobEvent::Error(err) => {
                    self.status = format!("diff error: {err}");
                    self.diff_job_rx = None;
                    self.diff_job_cancelled = None;
                    break;
                }
            }
        }
        true
    }

    /// 打開書籤列表彈窗，讓使用者可以用列表方式跳轉既有書籤。
    pub(crate) fn open_bookmark_list(&mut self) {
        self.open_bookmark_list_with_mode(self.focused_pane, BookmarkListMode::Jump);
    }

    /// 以指定模式打開書籤列表彈窗，供跳轉或刪除流程共用。
    pub(crate) fn open_bookmark_list_with_mode(&mut self, pane_id: usize, mode: BookmarkListMode) {
        self.pending_action = Some(PendingAction::BookmarkList {
            pane_id,
            selected: 0,
            mode,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = bookmark_list_status("", self.bookmark_store.list().len(), 0, mode, false);
    }

    /// 打開 zoxide 目錄列表，讓目前 panel 可依 frecency 快速跳到常用目錄。
    ///
    /// 這個面板是 `:zoxide` 的正式入口，`Z` 也會走這裡。
    pub(crate) fn open_zoxide_list(&mut self) {
        match query_zoxide_directories() {
            Ok(entries) => {
                let count = entries.len();
                self.pending_action = Some(PendingAction::ZoxideList {
                    pane_id: self.focused_pane,
                    selected: 0,
                    entries,
                    search: PanelSearchState {
                        buffer: String::new(),
                        editing: false,
                    },
                });
                self.status = zoxide_list_status("", count, 0, false);
            }
            Err(error) => {
                self.status = format!("zoxide failed: {error}");
                self.open_tool_panel();
            }
        }
    }

    /// 在目前 focus panel 顯示外部工具安裝狀態，讓使用者知道缺少哪些依賴。
    pub(crate) fn open_tool_panel(&mut self) {
        self.pending_action = Some(PendingAction::ToolPanel {
            pane_id: self.focused_pane,
            selected: 0,
        });
        self.status = String::from("dependencies: j/k move, Esc close");
    }

    /// 讓 `:linemode <mode>` 可以直接切換目前 pane 的右側欄位顯示模式。
    ///
    /// 支援：
    /// - `size`
    /// - `permissions`
    /// - `btime`
    /// - `mtime`
    /// - `none`
    pub(crate) fn apply_line_mode_from_command(&mut self, args: &str) -> io::Result<()> {
        let line_mode = match args {
            "size" => LineMode::Size,
            "permissions" => LineMode::Permissions,
            "btime" => LineMode::Btime,
            "mtime" => LineMode::Mtime,
            "none" => LineMode::None,
            _ => {
                self.status = String::from("usage: linemode <size|permissions|btime|mtime|none>");
                return Ok(());
            }
        };

        self.apply_line_mode(self.focused_pane, line_mode)
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
        self.config.ui.theme_preset = preset;
        match persist_theme(&self.config_source, preset) {
            Ok(()) => {
                self.status = format!("theme: {}", preset.name());
            }
            Err(error) => {
                self.status = format!("theme: {} (save failed: {error})", preset.name());
            }
        }
    }

    /// 將指定 pane 套用某一種排序模式。
    pub(crate) fn apply_sort_mode(
        &mut self,
        pane_id: usize,
        sort_mode: SortMode,
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        pane.set_sort_mode(sort_mode);
        let needs_directory_sizes = matches!(pane.active_detail_kind(), SortDetailKind::Size);
        self.status = format!("sort: {}", pane.sort_mode.label());
        if needs_directory_sizes {
            self.start_directory_size_scan(pane_id);
        } else {
            self.cancel_directory_size_scan(pane_id);
        }
        Ok(())
    }

    /// 套用指定 pane 的 linemode，只更新右側欄位顯示方式，不改動原本排序順序。
    ///
    /// 參數：
    /// - `pane_id: usize`，要被套用 linemode 的 pane 編號。
    /// - `line_mode: LineMode`，要切換成的右側欄位模式。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表 linemode 已套用完成。
    /// - 若目標 pane 已不存在，會改寫狀態列並直接結束。
    pub(crate) fn apply_line_mode(
        &mut self,
        pane_id: usize,
        line_mode: LineMode,
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        pane.set_line_mode(line_mode);
        let needs_directory_sizes = matches!(pane.active_detail_kind(), SortDetailKind::Size);
        self.status = format!("linemode: {}", line_mode.label());
        if needs_directory_sizes {
            self.start_directory_size_scan(pane_id);
        } else {
            self.cancel_directory_size_scan(pane_id);
        }
        Ok(())
    }
}
