use std::collections::BTreeMap;

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// 表示 pane 分割的方向。
///
/// `Horizontal` 代表上下分割，`Vertical` 代表左右分割。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitDirection {
    Horizontal,
    Vertical,
}

/// 表示新 split 出來的 pane 要放在目前 pane 的哪一側。
///
/// `Before` 代表新 pane 會出現在左側或上方，
/// `After` 代表新 pane 會出現在右側或下方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitPlacement {
    Before,
    After,
}

/// 表示整個多視窗布局的樹狀結構。
///
/// 葉節點代表單一 pane，中間節點代表一次分割行為，
/// 因此可以自然表達巢狀 split 的畫面配置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayoutNode {
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
    /// 將指定 pane 的葉節點替換成新的 split 節點。
    ///
    /// 參數：
    /// - `self: LayoutNode`，目前的布局樹。
    /// - `target: usize`，要被分割的 pane id。
    /// - `direction: SplitDirection`，新的分割方向。
    /// - `placement: SplitPlacement`，新 pane 要放在目前 pane 前面還是後面。
    /// - `new_pane_id: usize`，新建立 pane 的 id。
    ///
    /// 回傳：`LayoutNode`，套用分割後的新布局樹。
    pub(crate) fn split_leaf(
        self,
        target: usize,
        direction: SplitDirection,
        placement: SplitPlacement,
        new_pane_id: usize,
    ) -> Self {
        match self {
            LayoutNode::Leaf { pane_id } if pane_id == target => {
                let current_leaf = Box::new(LayoutNode::Leaf { pane_id });
                let new_leaf = Box::new(LayoutNode::Leaf {
                    pane_id: new_pane_id,
                });
                let (first, second) = match placement {
                    SplitPlacement::Before => (new_leaf, current_leaf),
                    SplitPlacement::After => (current_leaf, new_leaf),
                };

                LayoutNode::Split {
                    direction,
                    first,
                    second,
                }
            }
            LayoutNode::Leaf { pane_id } => LayoutNode::Leaf { pane_id },
            LayoutNode::Split {
                direction: split_direction,
                first,
                second,
            } => LayoutNode::Split {
                direction: split_direction,
                first: Box::new(first.split_leaf(target, direction, placement, new_pane_id)),
                second: Box::new(second.split_leaf(target, direction, placement, new_pane_id)),
            },
        }
    }

    /// 從布局樹中移除指定 pane。
    ///
    /// 參數：
    /// - `self: LayoutNode`，目前的布局樹。
    /// - `target: usize`，要關閉的 pane id。
    ///
    /// 回傳：`Option<LayoutNode>`。
    /// - `Some(...)` 代表移除後仍有可用布局。
    /// - `None` 代表移除後已沒有任何 pane。
    pub(crate) fn close_pane(self, target: usize) -> Option<Self> {
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

    /// 依照布局樹順序收集所有 pane id。
    ///
    /// 參數：
    /// - `self: &LayoutNode`，目前的布局樹。
    /// - `output: &mut Vec<usize>`，要寫入結果的容器。
    ///
    /// 回傳：`()`
    pub(crate) fn pane_ids(&self, output: &mut Vec<usize>) {
        match self {
            LayoutNode::Leaf { pane_id } => output.push(*pane_id),
            LayoutNode::Split { first, second, .. } => {
                first.pane_ids(output);
                second.pane_ids(output);
            }
        }
    }

    /// 計算每個 pane 在畫面上應該佔據的矩形區域。
    ///
    /// 參數：
    /// - `self: &LayoutNode`，目前的布局樹。
    /// - `area: Rect`，目前節點可使用的畫面範圍。
    /// - `map: &mut BTreeMap<usize, Rect>`，收集 pane id 與畫面區塊的對應表。
    ///
    /// 回傳：`()`
    pub(crate) fn render_rects(&self, area: Rect, map: &mut BTreeMap<usize, Rect>) {
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

#[cfg(test)]
mod tests {
    use super::{LayoutNode, SplitDirection, SplitPlacement};

    #[test]
    /// 驗證 split 操作會將目標葉節點替換成新的分割節點。
    ///
    /// 參數：無。
    /// 回傳：無；若布局結果不正確則測試失敗。
    fn split_leaf_replaces_target_with_split_node() {
        let layout = LayoutNode::Leaf { pane_id: 1 };
        let updated = layout.split_leaf(1, SplitDirection::Vertical, SplitPlacement::After, 2);

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
    /// 驗證當指定 `Before` 時，新 pane 會出現在目前 pane 的前面。
    fn split_leaf_can_insert_new_pane_before_current_one() {
        let layout = LayoutNode::Leaf { pane_id: 1 };
        let updated = layout.split_leaf(1, SplitDirection::Horizontal, SplitPlacement::Before, 2);

        assert_eq!(
            updated,
            LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                first: Box::new(LayoutNode::Leaf { pane_id: 2 }),
                second: Box::new(LayoutNode::Leaf { pane_id: 1 }),
            }
        );
    }

    #[test]
    /// 驗證關閉其中一個 pane 後，父 split 會正確收斂。
    ///
    /// 參數：無。
    /// 回傳：無；若布局未正確收斂則測試失敗。
    fn close_pane_collapses_parent_split() {
        let layout = LayoutNode::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Leaf { pane_id: 1 }),
            second: Box::new(LayoutNode::Leaf { pane_id: 2 }),
        };

        assert_eq!(layout.close_pane(2), Some(LayoutNode::Leaf { pane_id: 1 }));
    }
}
