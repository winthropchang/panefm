//! 多 panel 分割樹與畫面矩形計算。
//!
//! Layout 使用樹狀結構保存 split 關係，leaf 只引用穩定的 panel id。新增、關閉或
//! only panel 時先修改此樹，再由 `App` 同步 panel map；不要用畫面順序當作永久 id，
//! 否則關閉中間 panel 後快捷鍵與背景 task 會指到錯誤目標。

use std::collections::BTreeMap;

use ratatui::layout::Rect;

/// Panel 最小寬度保護極限（欄數）。左右邊框各 1 欄 + 內部至少 8 欄可讀空間。
pub const MIN_PANE_WIDTH: u16 = 10;

/// Panel 最小高度保護極限（列數）。頂部標題 1 列 + 底部邊框 1 列 + 中間至少 1 列檔案項目。
pub const MIN_PANE_HEIGHT: u16 = 3;

/// 表示 pane 分割的方向。
///
/// `Horizontal` 代表上下分割，`Vertical` 代表左右分割。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

/// 表示新 split 出來的 pane 要放在目前 pane 的哪一側。
///
/// `Before` 代表新 pane 會出現在左側或上方，
/// `After` 代表新 pane 會出現在右側或下方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitPlacement {
    Before,
    After,
}

/// 表示整個多視窗布局的樹狀結構。
///
/// 葉節點代表單一 pane，中間節點代表一個方向的多子視窗分割（N-ary Split），
/// 支援同向均等分配與權重比例縮放。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutNode {
    Leaf {
        pane_id: usize,
    },
    Split {
        direction: SplitDirection,
        children: Vec<LayoutNode>,
        weights: Vec<u16>,
    },
}

impl LayoutNode {
    /// 將指定 pane 分割，產生新的 pane。
    ///
    /// 若目標 pane 已處於相同分割方向的節點中，則直接吸收為同級視窗並均等重新分配寬度/高度；
    /// 若處於不同方向，則以目標 pane 為基準建立巢狀分割節點。
    pub fn split_leaf(
        self,
        target: usize,
        direction: SplitDirection,
        placement: SplitPlacement,
        new_pane_id: usize,
    ) -> Self {
        match self {
            LayoutNode::Leaf { pane_id } if pane_id == target => {
                let current_leaf = LayoutNode::Leaf { pane_id };
                let new_leaf = LayoutNode::Leaf {
                    pane_id: new_pane_id,
                };
                let children = match placement {
                    SplitPlacement::Before => vec![new_leaf, current_leaf],
                    SplitPlacement::After => vec![current_leaf, new_leaf],
                };

                LayoutNode::Split {
                    direction,
                    children,
                    weights: vec![100, 100],
                }
            }
            LayoutNode::Leaf { pane_id } => LayoutNode::Leaf { pane_id },
            LayoutNode::Split {
                direction: split_direction,
                mut children,
                weights: _,
            } => {
                // 若分割方向相同，且目標為此節點的直接 Leaf 子節點，直接吸收為同級視窗
                if split_direction == direction
                    && let Some(pos) = children.iter().position(
                        |child| matches!(child, LayoutNode::Leaf { pane_id } if *pane_id == target),
                    )
                {
                    let new_leaf = LayoutNode::Leaf {
                        pane_id: new_pane_id,
                    };
                    let insert_idx = match placement {
                        SplitPlacement::Before => pos,
                        SplitPlacement::After => pos + 1,
                    };
                    children.insert(insert_idx, new_leaf);
                    let count = children.len();
                    return LayoutNode::Split {
                        direction: split_direction,
                        children,
                        weights: vec![100; count],
                    };
                }

                // 否則遞迴進入子樹尋找 target
                let new_children: Vec<LayoutNode> = children
                    .into_iter()
                    .map(|child| child.split_leaf(target, direction, placement, new_pane_id))
                    .collect();
                let count = new_children.len();
                LayoutNode::Split {
                    direction: split_direction,
                    children: new_children,
                    weights: vec![100; count],
                }
            }
        }
    }

    /// 從布局樹中移除指定 pane。
    ///
    /// 移除後自動重新均等分配剩餘兄弟視窗；若節點只剩單一子視窗，則自動向上收合（Collapse）。
    pub fn close_pane(self, target: usize) -> Option<Self> {
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
                children,
                weights: _,
            } => {
                let mut new_children = Vec::new();
                for child in children {
                    if let Some(c) = child.close_pane(target) {
                        new_children.push(c);
                    }
                }
                match new_children.len() {
                    0 => None,
                    1 => Some(new_children.remove(0)),
                    len => Some(LayoutNode::Split {
                        direction,
                        children: new_children,
                        weights: vec![100; len],
                    }),
                }
            }
        }
    }

    /// 依照布局樹順序收集所有 pane id。
    pub fn pane_ids(&self, output: &mut Vec<usize>) {
        match self {
            LayoutNode::Leaf { pane_id } => output.push(*pane_id),
            LayoutNode::Split { children, .. } => {
                for child in children {
                    child.pane_ids(output);
                }
            }
        }
    }

    /// 檢查樹中是否包含指定 pane id。
    pub fn contains_pane(&self, target: usize) -> bool {
        match self {
            LayoutNode::Leaf { pane_id } => *pane_id == target,
            LayoutNode::Split { children, .. } => {
                children.iter().any(|child| child.contains_pane(target))
            }
        }
    }

    /// 檢查子樹中是否存在指定方向且包含 target 的分割節點。
    fn has_matching_ancestor(&self, target: usize, direction: SplitDirection) -> bool {
        match self {
            LayoutNode::Leaf { .. } => false,
            LayoutNode::Split {
                direction: split_dir,
                children,
                ..
            } => {
                if *split_dir == direction && children.iter().any(|c| c.contains_pane(target)) {
                    true
                } else {
                    children
                        .iter()
                        .any(|c| c.has_matching_ancestor(target, direction))
                }
            }
        }
    }

    /// 一鍵平衡：遞迴重置所有分割節點的權重為均等。
    pub fn equalize(&mut self) {
        match self {
            LayoutNode::Leaf { .. } => {}
            LayoutNode::Split {
                children, weights, ..
            } => {
                for w in weights.iter_mut() {
                    *w = 100;
                }
                for child in children.iter_mut() {
                    child.equalize();
                }
            }
        }
    }

    /// 調整指定 pane 的尺寸（欄寬或列高）。
    ///
    /// 參數：
    /// - `target: usize`：要縮放的面板 id。
    /// - `direction: SplitDirection`：`Vertical` 調整寬度，`Horizontal` 調整高度。
    /// - `delta_cells: i32`：增減的儲存格數（正數放大，負數縮小）。
    /// - `current_area: Rect`：目前節點所在的畫面矩形。
    pub fn resize_pane(
        &mut self,
        target: usize,
        direction: SplitDirection,
        delta_cells: i32,
        current_area: Rect,
    ) -> Result<(), &'static str> {
        match self {
            LayoutNode::Leaf { pane_id } => {
                if *pane_id == target {
                    if direction == SplitDirection::Vertical {
                        Err("panel spans full width")
                    } else {
                        Err("panel spans full height")
                    }
                } else {
                    Err("panel not found")
                }
            }
            LayoutNode::Split {
                direction: split_dir,
                children,
                weights,
            } => {
                let child_idx = children
                    .iter()
                    .position(|c| c.contains_pane(target))
                    .ok_or("panel not found")?;

                let child_rects = calculate_split_rects(current_area, *split_dir, weights);
                let child_rect = child_rects[child_idx];

                // 若該子樹深處還有相同方向的分割節點，優先遞迴深入更底層調整
                if children[child_idx].has_matching_ancestor(target, direction) {
                    return children[child_idx].resize_pane(
                        target,
                        direction,
                        delta_cells,
                        child_rect,
                    );
                }

                // 若目前節點的分割方向與調整方向不同，繼續向下尋找
                if *split_dir != direction {
                    return children[child_idx].resize_pane(
                        target,
                        direction,
                        delta_cells,
                        child_rect,
                    );
                }

                // 目前節點即為控制目標尺寸的分割層
                if children.len() <= 1 {
                    return if direction == SplitDirection::Vertical {
                        Err("panel spans full width")
                    } else {
                        Err("panel spans full height")
                    };
                }

                let min_size = if direction == SplitDirection::Vertical {
                    MIN_PANE_WIDTH
                } else {
                    MIN_PANE_HEIGHT
                };

                let cur_lengths: Vec<u16> = child_rects
                    .iter()
                    .map(|r| {
                        if direction == SplitDirection::Vertical {
                            r.width
                        } else {
                            r.height
                        }
                    })
                    .collect();

                let cur_size = cur_lengths[child_idx];

                let other_indices: Vec<usize> =
                    (0..children.len()).filter(|&i| i != child_idx).collect();

                if delta_cells > 0 {
                    // 放大目前面板，需同時向外擠壓所有其他兄弟面板（current pane 權限最高，其他面板同步變小）
                    let total_capacity: u16 = other_indices
                        .iter()
                        .map(|&i| cur_lengths[i].saturating_sub(min_size))
                        .sum();

                    if total_capacity == 0 {
                        return Err("all other panels reached minimum size");
                    }

                    let actual_grow = (delta_cells as u16).min(total_capacity);
                    if actual_grow == 0 {
                        return Err("all other panels reached minimum size");
                    }

                    let mut new_lengths = cur_lengths;
                    // 將 actual_grow 均勻分配給其他兄弟面板進行扣減（round-robin 輪流扣減）
                    let mut remaining = actual_grow;
                    let mut round_idx = 0;
                    while remaining > 0 {
                        let eligible: Vec<usize> = other_indices
                            .iter()
                            .copied()
                            .filter(|&i| new_lengths[i] > min_size)
                            .collect();
                        if eligible.is_empty() {
                            break;
                        }
                        let chosen = eligible[round_idx % eligible.len()];
                        new_lengths[chosen] = new_lengths[chosen].saturating_sub(1);
                        remaining -= 1;
                        round_idx += 1;
                    }

                    new_lengths[child_idx] = new_lengths[child_idx].saturating_add(actual_grow);
                    *weights = new_lengths;
                    Ok(())
                } else if delta_cells < 0 {
                    // 縮小目前面板，釋放空間同時讓所有其他兄弟面板同步放大
                    if cur_size <= min_size {
                        return Err("panel reached minimum size");
                    }

                    let max_shrink = cur_size.saturating_sub(min_size);
                    let actual_shrink = ((-delta_cells) as u16).min(max_shrink);
                    if actual_shrink == 0 {
                        return Err("panel reached minimum size");
                    }

                    let mut new_lengths = cur_lengths;
                    new_lengths[child_idx] = new_lengths[child_idx].saturating_sub(actual_shrink);

                    // 將 actual_shrink 均勻分配給所有其他兄弟面板放大（round-robin 輪流增加）
                    let mut remaining = actual_shrink;
                    let mut round_idx = 0;
                    while remaining > 0 {
                        let chosen = other_indices[round_idx % other_indices.len()];
                        new_lengths[chosen] = new_lengths[chosen].saturating_add(1);
                        remaining -= 1;
                        round_idx += 1;
                    }

                    *weights = new_lengths;
                    Ok(())
                } else {
                    Ok(())
                }
            }
        }
    }

    /// 計算每個 pane 在畫面上應該佔據的矩形區域。
    pub fn render_rects(&self, area: Rect, map: &mut BTreeMap<usize, Rect>) {
        match self {
            LayoutNode::Leaf { pane_id } => {
                map.insert(*pane_id, area);
            }
            LayoutNode::Split {
                direction,
                children,
                weights,
            } => {
                let rects = calculate_split_rects(area, *direction, weights);
                for (child, rect) in children.iter().zip(rects) {
                    child.render_rects(rect, map);
                }
            }
        }
    }
}

/// 依據方向與權重陣列，將給定矩形精準劃分為子矩形清單。
///
/// 任何因整數除法產生的餘數像素/字元，皆分配給最後一個子矩形，杜絕破圖縫隙。
pub fn calculate_split_rects(area: Rect, direction: SplitDirection, weights: &[u16]) -> Vec<Rect> {
    let count = weights.len();
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![area];
    }

    let (total_length, is_vertical_split) = match direction {
        SplitDirection::Vertical => (area.width, true),
        SplitDirection::Horizontal => (area.height, false),
    };

    let total_weight: u32 = weights.iter().map(|&w| (w.max(1)) as u32).sum();
    let mut rects = Vec::with_capacity(count);
    let mut offset: u16 = 0;

    for (i, &w) in weights.iter().enumerate() {
        let length = if i == count - 1 {
            total_length.saturating_sub(offset)
        } else {
            let weight_val = w.max(1) as u32;
            ((total_length as u32 * weight_val) / total_weight) as u16
        };

        if is_vertical_split {
            rects.push(Rect {
                x: area.x.saturating_add(offset),
                y: area.y,
                width: length,
                height: area.height,
            });
        } else {
            rects.push(Rect {
                x: area.x,
                y: area.y.saturating_add(offset),
                width: area.width,
                height: length,
            });
        }
        offset = offset.saturating_add(length);
    }

    rects
}

#[cfg(test)]
mod tests {
    use super::{LayoutNode, SplitDirection, SplitPlacement};

    #[test]
    /// 驗證 split 操作會將目標葉節點替換成新的分割節點。
    fn split_leaf_replaces_target_with_split_node() {
        let layout = LayoutNode::Leaf { pane_id: 1 };
        let updated = layout.split_leaf(1, SplitDirection::Vertical, SplitPlacement::After, 2);

        assert_eq!(
            updated,
            LayoutNode::Split {
                direction: SplitDirection::Vertical,
                children: vec![
                    LayoutNode::Leaf { pane_id: 1 },
                    LayoutNode::Leaf { pane_id: 2 },
                ],
                weights: vec![100, 100],
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
                children: vec![
                    LayoutNode::Leaf { pane_id: 2 },
                    LayoutNode::Leaf { pane_id: 1 },
                ],
                weights: vec![100, 100],
            }
        );
    }

    #[test]
    /// 驗證關閉其中一個 pane 後，父 split 會正確收斂為單一節點。
    fn close_pane_collapses_parent_split() {
        let layout = LayoutNode::Split {
            direction: SplitDirection::Horizontal,
            children: vec![
                LayoutNode::Leaf { pane_id: 1 },
                LayoutNode::Leaf { pane_id: 2 },
            ],
            weights: vec![100, 100],
        };

        assert_eq!(layout.close_pane(2), Some(LayoutNode::Leaf { pane_id: 1 }));
    }
}
