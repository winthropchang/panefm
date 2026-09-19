//! 依互動模式與焦點視窗狀態動態產生底部快捷鍵提示列。

use ratatui::text::{Line, Span};

use crate::theme::Theme;

use super::super::*;
use super::StatusShortcutHint;

impl App {
    /// 依目前開啟的畫面、面板或互動模式，動態產出最相關且有用的快捷鍵清單。
    /// 第一個項目永遠固定為 Help。
    pub(crate) fn active_status_shortcut_hints(&self) -> Vec<StatusShortcutHint> {
        let mut hints = Vec::new();
        // 第一個項目永遠固定為 Help，第二個固定為當前面板的 Cheatsheet
        hints.push(StatusShortcutHint {
            key: "~/F1",
            label: "help",
        });
        hints.push(StatusShortcutHint {
            key: "?",
            label: "cheat",
        });

        if self.command_mode {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "execute",
                },
                StatusShortcutHint {
                    key: "Tab",
                    label: "complete",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(filter) = &self.filter
            && filter.editing
        {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "confirm",
                },
                StatusShortcutHint {
                    key: "Tab",
                    label: "fuzzy/normal",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(search) = &self.preview_search
            && search.editing
        {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "confirm",
                },
                StatusShortcutHint {
                    key: "n/N",
                    label: "match",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(_find) = &self.list_find {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "confirm",
                },
                StatusShortcutHint {
                    key: "n/N",
                    label: "match",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(search) = &self.global_search {
            if search.editing {
                hints.extend_from_slice(&[
                    StatusShortcutHint {
                        key: "Enter",
                        label: "start search",
                    },
                    StatusShortcutHint {
                        key: "Esc",
                        label: "cancel",
                    },
                ]);
            } else if search.filter.editing {
                hints.extend_from_slice(&[StatusShortcutHint {
                    key: "Enter/Esc",
                    label: "done filter",
                }]);
            } else {
                hints.extend_from_slice(&[
                    StatusShortcutHint {
                        key: "j/k",
                        label: "move",
                    },
                    StatusShortcutHint {
                        key: "Enter/l",
                        label: "jump",
                    },
                    StatusShortcutHint {
                        key: "f",
                        label: "filter",
                    },
                    StatusShortcutHint {
                        key: "i/s",
                        label: "edit query",
                    },
                    StatusShortcutHint {
                        key: "q/Esc",
                        label: "exit",
                    },
                ]);
            }
            return hints;
        }

        if let Some(action) = &self.pending_action {
            if let Some(action_hints) = self
                .pending_action_hints(action)
                .or_else(|| self.pending_picker_hints(action))
            {
                hints.extend(action_hints);
            }
            return hints;
        }

        if self.visual_selection.is_some() {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "j/k",
                    label: "select range",
                },
                StatusShortcutHint {
                    key: "y",
                    label: "copy",
                },
                StatusShortcutHint {
                    key: "x",
                    label: "cut",
                },
                StatusShortcutHint {
                    key: "d",
                    label: "trash",
                },
                StatusShortcutHint {
                    key: "D",
                    label: "delete",
                },
                StatusShortcutHint {
                    key: "C",
                    label: "compress",
                },
                StatusShortcutHint {
                    key: "Space",
                    label: "mark",
                },
                StatusShortcutHint {
                    key: "v/Esc",
                    label: "exit visual",
                },
            ]);
            return hints;
        }

        if let Some(pane) = self.panes.get(&self.focused_pane)
            && pane.is_preview_active()
        {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "[/]",
                    label: "prev/next file",
                },
                StatusShortcutHint {
                    key: "j/k",
                    label: "scroll",
                },
                StatusShortcutHint {
                    key: "Ctrl+d/u",
                    label: "page scroll",
                },
                StatusShortcutHint {
                    key: "h",
                    label: "list",
                },
                StatusShortcutHint {
                    key: "Tab",
                    label: "close",
                },
            ]);
            return hints;
        }

        if let Some(pane) = self.panes.get(&self.focused_pane)
            && pane.is_preview_open()
        {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "j/k",
                    label: "move",
                },
                StatusShortcutHint {
                    key: "l",
                    label: "preview",
                },
                StatusShortcutHint {
                    key: "Tab",
                    label: "close preview",
                },
                StatusShortcutHint {
                    key: "q",
                    label: "quit",
                },
            ]);
            return hints;
        }

        // 預設（一般列表瀏覽模式）
        hints.extend_from_slice(status_shortcut_hints());
        // 移除重複的 help (因為 status_shortcut_hints() 本身第一項也是 help)
        hints.dedup();
        hints
    }
}

/// 回傳底部 status bar 允許顯示的標準預設快捷鍵。第一筆固定為 Help。
pub(crate) fn status_shortcut_hints() -> &'static [StatusShortcutHint] {
    &[
        StatusShortcutHint {
            key: "~/F1",
            label: "help",
        },
        StatusShortcutHint {
            key: "hjkl",
            label: "move",
        },
        StatusShortcutHint {
            key: "Enter",
            label: "open",
        },
        StatusShortcutHint {
            key: "b",
            label: "bookmark",
        },
        StatusShortcutHint {
            key: "Tab",
            label: "preview",
        },
        StatusShortcutHint {
            key: "y",
            label: "copy",
        },
        StatusShortcutHint {
            key: "x",
            label: "cut",
        },
        StatusShortcutHint {
            key: "p/P",
            label: "paste/overwrite",
        },
        StatusShortcutHint {
            key: "v",
            label: "select",
        },
        StatusShortcutHint {
            key: "s/S",
            label: "search",
        },
        StatusShortcutHint {
            key: "f/F",
            label: "filter/fuzzy",
        },
        StatusShortcutHint {
            key: "r",
            label: "rename",
        },
        StatusShortcutHint {
            key: "a",
            label: "create",
        },
        StatusShortcutHint {
            key: "d/D",
            label: "delete",
        },
        StatusShortcutHint {
            key: "u",
            label: "undo",
        },
        StatusShortcutHint {
            key: "w",
            label: "panel",
        },
        StatusShortcutHint {
            key: "T",
            label: "tasks",
        },
    ]
}

/// 依 terminal 寬度與目前情境快捷鍵清單建立底部快捷鍵列，並把版本固定在最右側。
///
/// 參數：
/// - `width: u16`，目前快捷鍵列可使用的 terminal cell 寬度。
/// - `theme: Theme`，用來替按鍵套用目前主題的 accent 顏色。
/// - `hints: &[StatusShortcutHint]`，依照目前畫面或模式產生的快捷鍵清單。
///
/// 回傳：`Line<'static>`，可直接交給 ratatui `Paragraph` 繪製的單行內容。
pub(crate) fn status_shortcut_line(
    width: u16,
    theme: Theme,
    hints: &[StatusShortcutHint],
) -> Line<'static> {
    let available_width = usize::from(width);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let version_width = version.len();
    let version_gap = 2usize;
    let mut used_width = 0usize;
    let mut spans = Vec::new();

    for (index, hint) in hints.iter().enumerate() {
        let separator = if index == 0 { "" } else { "  " };
        let item_width = separator.len() + hint.key.len() + 1 + hint.label.len();
        let required_width = used_width
            .saturating_add(item_width)
            .saturating_add(version_gap)
            .saturating_add(version_width);
        if required_width > available_width {
            break;
        }

        if !separator.is_empty() {
            spans.push(Span::raw(separator));
        }
        spans.push(Span::styled(hint.key, theme.accent_style()));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(hint.label));
        used_width = used_width.saturating_add(item_width);
    }

    let padding_width = available_width.saturating_sub(used_width.saturating_add(version_width));
    if padding_width > 0 {
        spans.push(Span::raw(" ".repeat(padding_width)));
    }
    spans.push(Span::styled(version, theme.accent_style()));

    Line::from(spans)
}
