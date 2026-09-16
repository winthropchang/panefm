use std::time::SystemTime;

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::file_manager::archive::ExtractedArchive;
use crate::theme::Theme;

use super::*;

/// 描述底部快捷鍵列中的一組高頻操作提示。
///
/// 欄位：
/// - `key: &'static str`，實際要按下的快捷鍵。
/// - `label: &'static str`，簡短的英文功能名稱。
///
/// 清單本身的順序就是顯示優先度；畫面不足時只會捨棄尾端低優先項目，避免把
/// `~/F1 help` 等重要入口裁掉，或顯示只剩一半的快捷鍵說明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusShortcutHint {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
}

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
            match action {
                PendingAction::TaskPanel {
                    search,
                    marked_ids,
                    visual_anchor,
                    ..
                } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else if visual_anchor.is_some() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "select range",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "commit visual",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "cancel visual",
                            },
                        ]);
                    } else if !marked_ids.is_empty() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "d",
                                label: "delete marked",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "unmark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "clear marks",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "visual",
                            },
                            StatusShortcutHint {
                                key: "x/c",
                                label: "cancel task",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "clear marks",
                            },
                        ]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "visual",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "mark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "mark all",
                            },
                            StatusShortcutHint {
                                key: "d",
                                label: "delete",
                            },
                            StatusShortcutHint {
                                key: "D",
                                label: "clear all",
                            },
                            StatusShortcutHint {
                                key: "x/c",
                                label: "cancel task",
                            },
                            StatusShortcutHint {
                                key: "X",
                                label: "cancel all",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::TrashPanel {
                    search,
                    marked_ids,
                    visual_anchor,
                    ..
                } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else if visual_anchor.is_some() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "select range",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "commit visual",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "cancel visual",
                            },
                        ]);
                    } else if !marked_ids.is_empty() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "u",
                                label: "restore marked",
                            },
                            StatusShortcutHint {
                                key: "d",
                                label: "delete marked",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "unmark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "clear marks",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "clear marks",
                            },
                        ]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "visual",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "mark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "mark all",
                            },
                            StatusShortcutHint {
                                key: "u",
                                label: "restore",
                            },
                            StatusShortcutHint {
                                key: "U",
                                label: "restore all",
                            },
                            StatusShortcutHint {
                                key: "d",
                                label: "delete",
                            },
                            StatusShortcutHint {
                                key: "D",
                                label: "clear all",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::DiffMatrix(diff_state) => {
                    if diff_state.search_active {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "Enter",
                                label: "diff file",
                            },
                            StatusShortcutHint {
                                key: "i",
                                label: "gitignore",
                            },
                            StatusShortcutHint {
                                key: ".",
                                label: "hidden",
                            },
                            StatusShortcutHint {
                                key: "r",
                                label: "rescan",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::HelpPanel { search, .. } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "Enter",
                                label: "execute",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::BookmarkList { search, mode, .. } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        match mode {
                            BookmarkListMode::Jump => {
                                hints.extend_from_slice(&[
                                    StatusShortcutHint {
                                        key: "j/k",
                                        label: "move",
                                    },
                                    StatusShortcutHint {
                                        key: "Enter",
                                        label: "jump",
                                    },
                                    StatusShortcutHint {
                                        key: "f",
                                        label: "search",
                                    },
                                    StatusShortcutHint {
                                        key: "q/Esc",
                                        label: "close",
                                    },
                                ]);
                            }
                            BookmarkListMode::Delete => {
                                hints.extend_from_slice(&[
                                    StatusShortcutHint {
                                        key: "j/k",
                                        label: "move",
                                    },
                                    StatusShortcutHint {
                                        key: "d/Enter",
                                        label: "delete",
                                    },
                                    StatusShortcutHint {
                                        key: "D",
                                        label: "clear all",
                                    },
                                    StatusShortcutHint {
                                        key: "f",
                                        label: "search",
                                    },
                                    StatusShortcutHint {
                                        key: "q/Esc",
                                        label: "close",
                                    },
                                ]);
                            }
                        }
                    }
                }
                PendingAction::ZoxideList { search, .. } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "Enter",
                                label: "jump",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::BookmarkPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "a",
                            label: "add",
                        },
                        StatusShortcutHint {
                            key: "g",
                            label: "jump list",
                        },
                        StatusShortcutHint {
                            key: "d",
                            label: "delete list",
                        },
                        StatusShortcutHint {
                            key: "D",
                            label: "clear all",
                        },
                        StatusShortcutHint {
                            key: "b/Esc",
                            label: "close",
                        },
                    ]);
                }
                PendingAction::WindowPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "s/v",
                            label: "split h/v",
                        },
                        StatusShortcutHint {
                            key: "r",
                            label: "resize",
                        },
                        StatusShortcutHint {
                            key: "=",
                            label: "equal",
                        },
                        StatusShortcutHint {
                            key: "W/H",
                            label: "width/height",
                        },
                        StatusShortcutHint {
                            key: "q",
                            label: "close",
                        },
                        StatusShortcutHint {
                            key: "o",
                            label: "only",
                        },
                        StatusShortcutHint {
                            key: "d",
                            label: "diff",
                        },
                        StatusShortcutHint {
                            key: "t",
                            label: "terminal",
                        },
                        StatusShortcutHint {
                            key: "1..9",
                            label: "focus",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "close",
                        },
                    ]);
                }
                PendingAction::WindowResize { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "h/l",
                            label: "width",
                        },
                        StatusShortcutHint {
                            key: "j/k",
                            label: "height",
                        },
                        StatusShortcutHint {
                            key: "=",
                            label: "equal",
                        },
                        StatusShortcutHint {
                            key: "Esc/Enter",
                            label: "done",
                        },
                    ]);
                }
                PendingAction::SortPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "n",
                            label: "name",
                        },
                        StatusShortcutHint {
                            key: "s",
                            label: "size",
                        },
                        StatusShortcutHint {
                            key: "m",
                            label: "mtime",
                        },
                        StatusShortcutHint {
                            key: "e",
                            label: "ext",
                        },
                        StatusShortcutHint {
                            key: "r",
                            label: "reverse",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::GoPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "g",
                            label: "top",
                        },
                        StatusShortcutHint {
                            key: "d",
                            label: "documents",
                        },
                        StatusShortcutHint {
                            key: "k",
                            label: "desktop",
                        },
                        StatusShortcutHint {
                            key: "h",
                            label: "home",
                        },
                        StatusShortcutHint {
                            key: "t",
                            label: "goto path",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::LineModePicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "s",
                            label: "size",
                        },
                        StatusShortcutHint {
                            key: "m",
                            label: "mtime",
                        },
                        StatusShortcutHint {
                            key: "p",
                            label: "perms",
                        },
                        StatusShortcutHint {
                            key: "n",
                            label: "none",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::YankPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "y",
                            label: "clipboard",
                        },
                        StatusShortcutHint {
                            key: "p",
                            label: "panel",
                        },
                        StatusShortcutHint {
                            key: "1..9",
                            label: "pane",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::ThemePicker { .. } | PendingAction::ThemeCommandPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "preview",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "apply",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::OpenPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "open with",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::CopyPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "copy text",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::Rename { .. } | PendingAction::CreateEntry { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "Enter",
                            label: "confirm",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::RegexRename { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "apply",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::ToolPanel { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "q/Esc",
                            label: "close",
                        },
                    ]);
                }
                PendingAction::EasyMotion { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "key",
                            label: "jump",
                        },
                        StatusShortcutHint {
                            key: "Esc/q",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::ConfirmDelete { .. }
                | PendingAction::ConfirmPasteOverwrite { .. }
                | PendingAction::ConfirmTrashAction { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "y",
                            label: "confirm",
                        },
                        StatusShortcutHint {
                            key: "n/Esc",
                            label: "cancel",
                        },
                    ]);
                }
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

/// 依終端 cell 寬度把 status 文字預先切成實際要繪製的多行內容。
///
/// 不直接使用 Rust 字串長度，因為中文等寬字元通常占兩個 terminal cell。預先換行後
/// 再交給 `Paragraph`，可確保高度計算與真正畫面使用完全相同的內容，也避免 ratatui
/// 私有的 rendered-line API。長路徑會按 cell 邊界切開，不會因為沒有空白而被截斷。
///
/// 參數：
/// - `status: &str`，準備顯示的完整狀態文字，可包含換行。
/// - `width: u16`，status area 可使用的終端欄寬。
///
/// 回傳：`String`，已插入必要換行、可直接交給 `Paragraph` 的文字。
pub(crate) fn wrap_status_text(status: &str, width: u16) -> String {
    let max_width = usize::from(width.max(1));
    let mut wrapped = Vec::new();

    for logical_line in status.split('\n') {
        let mut current = String::new();
        let mut current_width = 0usize;

        for character in logical_line.chars() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if current_width > 0 && current_width.saturating_add(character_width) > max_width {
                wrapped.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(character);
            current_width = current_width.saturating_add(character_width);
        }
        wrapped.push(current);
    }

    wrapped.join("\n")
}

/// 計算已換行 status 內容應占用的畫面高度。
///
/// 參數：
/// - `wrapped_status: &str`，經 `wrap_status_text` 處理後的狀態文字。
/// - `max_height: u16`，扣除主列表最低高度與快捷鍵區後可使用的最大高度。
///
/// 回傳：`u16`，至少一行且不超過可用畫面的 status area 高度。
pub(crate) fn status_area_height(wrapped_status: &str, max_height: u16) -> u16 {
    let required = wrapped_status.split('\n').count().max(1) as u16;
    required.min(max_height.max(1))
}

/// 判斷狀態列文字是否代表錯誤或目前操作無法執行。
///
/// 參數：
/// - `status: &str`，目前要顯示在畫面底部的狀態訊息。
///
/// 回傳：`bool`。
/// - `true` 代表應使用主題的危險色顯示。
/// - `false` 代表一般通知，維持預設文字顏色。
///
/// 這裡集中判斷訊息前綴，避免在每一個產生錯誤的操作中額外傳遞 UI 顏色狀態。
pub(crate) fn status_is_error(status: &str) -> bool {
    let normalized = status.trim().to_ascii_lowercase();
    [
        "error",
        "failed",
        "invalid",
        "usage:",
        "unknown",
        "cannot",
        "nothing selected",
        "panel no longer exists",
        "paste failed",
        "rename-regex: resolve conflicts",
        "rename-regex: nothing to apply",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

/// 根據解壓結果數量與略過項目數，整理出適合顯示在狀態列的訊息。
pub(crate) fn extraction_status_label(extracted: &[ExtractedArchive], skipped: usize) -> String {
    if extracted.len() == 1 {
        let output_name = extracted[0]
            .output_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("output");
        if skipped == 0 {
            format!("extracted {output_name}")
        } else {
            format!("extracted {output_name} (skipped {skipped})")
        }
    } else if skipped == 0 {
        format!("extracted {} archives", extracted.len())
    } else {
        format!("extracted {} archives (skipped {skipped})", extracted.len())
    }
}

/// 依照本次貼上衝突的名稱與數量，產生覆蓋確認視窗的狀態列文字。
pub(crate) fn paste_overwrite_confirm_status(target_name: &str, entry_count: usize) -> String {
    if entry_count <= 1 {
        format!("confirm overwrite {target_name}: y/n")
    } else {
        format!("confirm overwrite {target_name} ({entry_count} items): y/n")
    }
}

/// 當使用者取消這次覆蓋貼上時，回傳狀態列要顯示的訊息。
pub(crate) fn paste_overwrite_cancelled_status(target_name: &str, entry_count: usize) -> String {
    if entry_count <= 1 {
        format!("paste cancelled: {target_name}")
    } else {
        format!("paste cancelled: {target_name} ({entry_count} items)")
    }
}

/// 取得目前系統時間的 unix 毫秒。
pub(crate) fn unix_time_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 回傳建立流程的狀態列內容，讓使用者知道目前正處於哪一種編輯模式。
pub(crate) fn create_status_label(mode: &str) -> String {
    format!("create entry: {mode}")
}

/// 依照目前 preview search 文字與命中數量產生狀態列訊息。
pub(crate) fn preview_search_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("preview search: all")
    } else {
        format!("preview search: {buffer} ({matches})")
    }
}

/// 依照目前列表內 find-next 文字與命中數量產生狀態列訊息。
pub(crate) fn list_find_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("find next: type query")
    } else {
        format!("find next: {buffer} ({matches})")
    }
}

/// 依照目前列表內 find-next 文字與命中數量產生鎖定後的狀態列訊息。
pub(crate) fn list_find_locked_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("find next: empty")
    } else {
        format!("find next locked: {buffer} ({matches})")
    }
}

/// 依照目前 global search 文字、結果數與模式，產生狀態列訊息。
pub(crate) fn global_search_status(
    mode: SearchMode,
    buffer: &str,
    matches: usize,
    editing: bool,
    searched: bool,
    loading: bool,
) -> String {
    let interaction_mode = if editing { "insert" } else { "normal" };
    let label = mode.status_label();
    if loading {
        format!("{label} ({interaction_mode}): loading...")
    } else if !searched {
        if buffer.is_empty() {
            format!("{label} ({interaction_mode}): type query and Enter")
        } else {
            format!("{label} ({interaction_mode}): {buffer} (press Enter to search)")
        }
    } else if buffer.is_empty() {
        format!("{label} ({interaction_mode}): all ({matches})")
    } else {
        format!("{label} ({interaction_mode}): {buffer} ({matches})")
    }
}

/// 回傳 global search 套用結果模糊 filter 後的可見筆數。
///
/// 參數：
/// - `search: &GlobalSearchState`，目前 `s` 或 `S` 搜尋面板的完整狀態。
///
/// 回傳：`usize`，目前可供游標移動與開啟的結果數量。
pub(crate) fn global_search_visible_len(search: &GlobalSearchState) -> usize {
    filtered_global_search_entries(&search.results, &search.filter.buffer).len()
}

/// 建立 filter 狀態列文字。
pub(crate) fn format_filter_status(filter: &FilterState) -> String {
    let mode_label = match filter.mode {
        FilterMode::Normal => "normal",
        FilterMode::Fuzzy => "fuzzy",
    };
    if filter.buffer.is_empty() {
        format!("filter [{mode_label}]: all (Tab to switch)")
    } else if filter.editing {
        format!("filter [{mode_label}]: {}", filter.buffer)
    } else {
        format!("filter locked [{mode_label}]: {}", filter.buffer)
    }
}

/// 依照 global search 的模糊 filter 狀態產生狀態列訊息。
///
/// 參數：
/// - `filter: &PanelSearchState`，filter 查詢與是否仍在輸入中的狀態。
/// - `matches: usize`，套用模糊 filter 後的可見結果數量。
///
/// 回傳：`String`，供狀態列顯示目前查詢、模式與命中數。
pub(crate) fn global_search_filter_status(filter: &PanelSearchState, matches: usize) -> String {
    let mode = if filter.editing { "insert" } else { "locked" };
    if filter.buffer.is_empty() {
        format!("fuzzy filter ({mode}): all ({matches})")
    } else {
        format!("fuzzy filter ({mode}): {} ({matches})", filter.buffer)
    }
}

/// 產生搜尋引擎缺少外部工具時的狀態列訊息。
///
/// 參數：
/// - `mode: SearchMode`，目前執行的是檔名搜尋或內容搜尋。
/// - `tool: &str`，缺少的外部工具名稱，例如 `fd` 或 `rg`。
///
/// 回傳：`String`，包含搜尋類型、工具名稱與 `:status` 操作提示。
pub(crate) fn missing_search_tool_status(mode: SearchMode, tool: &str) -> String {
    format!("{} requires {tool}; run :status", mode.status_label())
}
