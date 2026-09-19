//! 操作確認、工作佇列、垃圾桶、Diff 與更名面板的快捷鍵提示產生邏輯。

use super::super::*;
use super::StatusShortcutHint;

impl App {
    /// 針對操作確認、佇列管理與更名等動態面板產出對應的快捷鍵提示。
    pub(crate) fn pending_action_hints(
        &self,
        action: &PendingAction,
    ) -> Option<Vec<StatusShortcutHint>> {
        let mut hints = Vec::new();
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
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
                Some(hints)
            }
            _ => None,
        }
    }
}
