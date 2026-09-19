//! 快捷選擇器、導航清單、書籤、視窗、主題與 EasyMotion 面板的快捷鍵提示產生邏輯。

use super::super::*;
use super::StatusShortcutHint;

impl App {
    /// 針對選擇器、說明、歷史路徑等導航選單產出對應的快捷鍵提示。
    pub(crate) fn pending_picker_hints(
        &self,
        action: &PendingAction,
    ) -> Option<Vec<StatusShortcutHint>> {
        let mut hints = Vec::new();
        match action {
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
            }
            _ => None,
        }
    }
}
