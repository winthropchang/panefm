//! 視窗分割、平均尺寸分配、連續尺寸調整與邊界保護的整合測試。
//!
//! 依據 `DEVELOPMENT_GUIDELINES.md` 規範，測試程式碼獨立放置於 `tests/` 目錄，
//! 驗證：
//! 1. 視窗分割同欄/同列自動均等分配（如 2 panes 各 50%、3 panes 各 33%、多出餘數給最後一個建立的 pane）。
//! 2. 巢狀視窗分割保持外層欄寬，並在內部均等分配高度。
//! 3. 視窗尺寸連續微調（寬度 ±4、高度 ±2）與互鎖邊界保護（最小寬度 10 欄、最小高度 3 列）。
//! 4. 一鍵重設均等（Equalize）重置所有權重為平衡狀態。
//! 5. 關閉視窗後剩餘兄弟視窗自動平分，只剩單一視窗時自動向上收合。

use std::collections::BTreeMap;

use panefm::file_manager::layout::{
    LayoutNode, MIN_PANE_HEIGHT, MIN_PANE_WIDTH, SplitDirection, SplitPlacement,
    calculate_split_rects,
};
use ratatui::layout::Rect;

#[test]
/// 驗證同列連續垂直分割（增加欄數）時，各視窗自動均等分配寬度，餘數精準給予最後一個視窗。
/// 使用者需求情境：
/// - 一行 2 個 pane：高 100%，寬度各 50%。
/// - 一行 3 個 pane：高 100%，寬度各 33%，多出來的給最後一個建立的 pane。
fn test_same_row_split_auto_equalizes_width_with_remainder_to_last() {
    let screen = Rect::new(0, 0, 100, 40);

    // 初始狀態：單一 pane 1
    let mut layout = LayoutNode::Leaf { pane_id: 1 };
    let mut rects = BTreeMap::new();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1), Some(&Rect::new(0, 0, 100, 40)));

    // 第一次分割：在 pane 1 右側新增 pane 2 (wl)
    layout = layout.split_leaf(1, SplitDirection::Vertical, SplitPlacement::After, 2);
    rects.clear();
    layout.render_rects(screen, &mut rects);

    // 2 個 pane：高 100%，寬度各 50%
    assert_eq!(rects.get(&1), Some(&Rect::new(0, 0, 50, 40)));
    assert_eq!(rects.get(&2), Some(&Rect::new(50, 0, 50, 40)));

    // 第二次分割：在 pane 2 右側新增 pane 3 (wl)
    layout = layout.split_leaf(2, SplitDirection::Vertical, SplitPlacement::After, 3);
    rects.clear();
    layout.render_rects(screen, &mut rects);

    // 3 個 pane：高 100%，寬度 33%, 33%, 34%（餘數分配給最後建立的 pane 3）
    assert_eq!(rects.get(&1), Some(&Rect::new(0, 0, 33, 40)));
    assert_eq!(rects.get(&2), Some(&Rect::new(33, 0, 33, 40)));
    assert_eq!(rects.get(&3), Some(&Rect::new(66, 0, 34, 40)));
}

#[test]
/// 驗證巢狀分割：當第二個 pane 內部新增三個水平 pane 時，
/// 其寬度保持為第二個 pane 的寬度，高度則均等分配，餘數給最後一個 pane。
fn test_nested_column_split_keeps_column_width_and_equalizes_height() {
    let screen = Rect::new(0, 0, 100, 40);

    // 先建立三個水平並排的欄 (pane 1, pane 2, pane 3)
    let layout = LayoutNode::Leaf { pane_id: 1 }
        .split_leaf(1, SplitDirection::Vertical, SplitPlacement::After, 2)
        .split_leaf(2, SplitDirection::Vertical, SplitPlacement::After, 3);

    // 在第二個 pane 內部向下分割 pane 4 (wj)
    let layout = layout.split_leaf(2, SplitDirection::Horizontal, SplitPlacement::After, 4);

    // 再在 pane 4 下方新增 pane 5 (wj)
    let layout = layout.split_leaf(4, SplitDirection::Horizontal, SplitPlacement::After, 5);

    let mut rects = BTreeMap::new();
    layout.render_rects(screen, &mut rects);

    // 第一欄 pane 1：寬 33，高 40
    assert_eq!(rects.get(&1), Some(&Rect::new(0, 0, 33, 40)));

    // 第三欄 pane 3：寬 34，高 40
    assert_eq!(rects.get(&3), Some(&Rect::new(66, 0, 34, 40)));

    // 第二欄內部的三個 pane (2, 4, 5)：
    // 每個 pane 的寬度皆為 33（x = 33）
    // 高度均等分配：40 / 3 = 13，13，14
    let r2 = rects.get(&2).unwrap();
    let r4 = rects.get(&4).unwrap();
    let r5 = rects.get(&5).unwrap();

    assert_eq!(r2.x, 33);
    assert_eq!(r2.width, 33);
    assert_eq!(r2.y, 0);
    assert_eq!(r2.height, 13);

    assert_eq!(r4.x, 33);
    assert_eq!(r4.width, 33);
    assert_eq!(r4.y, 13);
    assert_eq!(r4.height, 13);

    assert_eq!(r5.x, 33);
    assert_eq!(r5.width, 33);
    assert_eq!(r5.y, 26);
    assert_eq!(r5.height, 14); // 餘數給最後一個 pane 5
}

#[test]
/// 驗證尺寸調整（resize_pane）能平滑放大縮小，且受最小寬高邊界保護（MIN_PANE_WIDTH=10, MIN_PANE_HEIGHT=3）。
fn test_resize_pane_with_boundary_protection() {
    let screen = Rect::new(0, 0, 80, 20);

    // 建立 2 個視窗：寬度各 40
    let mut layout = LayoutNode::Leaf { pane_id: 1 }.split_leaf(
        1,
        SplitDirection::Vertical,
        SplitPlacement::After,
        2,
    );

    let mut rects = BTreeMap::new();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 40);
    assert_eq!(rects.get(&2).unwrap().width, 40);

    // pane 1 增加寬度 +4 欄
    let res = layout.resize_pane(1, SplitDirection::Vertical, 4, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 44);
    assert_eq!(rects.get(&2).unwrap().width, 36);

    // pane 1 減少寬度 -8 欄
    let res = layout.resize_pane(1, SplitDirection::Vertical, -8, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 36);
    assert_eq!(rects.get(&2).unwrap().width, 44);

    // 嘗試持續縮小 pane 1，應平滑收縮至最小寬度極限 MIN_PANE_WIDTH (10)
    let _ = layout.resize_pane(1, SplitDirection::Vertical, -100, screen);
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, MIN_PANE_WIDTH);

    // 已在最小寬度極限，再次縮小應回傳錯誤拒絕
    let res = layout.resize_pane(1, SplitDirection::Vertical, -4, screen);
    assert!(res.is_err(), "縮小超過極限應回傳錯誤拒絕");

    // 嘗試放大 pane 1，擠壓 pane 2 直到 pane 2 達到最小寬度 MIN_PANE_WIDTH (10)
    let _ = layout.resize_pane(1, SplitDirection::Vertical, 100, screen);
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&2).unwrap().width, MIN_PANE_WIDTH);

    // pane 2 已在極限，再次放大 pane 1 應回傳錯誤拒絕
    let res = layout.resize_pane(1, SplitDirection::Vertical, 4, screen);
    assert!(res.is_err(), "擠壓鄰居超過極限應回傳錯誤拒絕");
}

#[test]
/// 驗證高度調整與高度邊界保護（MIN_PANE_HEIGHT=3）。
fn test_resize_pane_height_with_boundary_protection() {
    let screen = Rect::new(0, 0, 60, 20);

    // 建立 2 個上下視窗：高度各 10
    let mut layout = LayoutNode::Leaf { pane_id: 1 }.split_leaf(
        1,
        SplitDirection::Horizontal,
        SplitPlacement::After,
        2,
    );

    let mut rects = BTreeMap::new();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().height, 10);
    assert_eq!(rects.get(&2).unwrap().height, 10);

    // pane 1 增加高度 +2 列
    let res = layout.resize_pane(1, SplitDirection::Horizontal, 2, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().height, 12);
    assert_eq!(rects.get(&2).unwrap().height, 8);

    // 縮小 pane 1 直到最小高度極限 MIN_PANE_HEIGHT (3)
    let _ = layout.resize_pane(1, SplitDirection::Horizontal, -50, screen);
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().height, MIN_PANE_HEIGHT);

    // 已在最小高度極限，再次縮小應回傳錯誤拒絕
    let res = layout.resize_pane(1, SplitDirection::Horizontal, -2, screen);
    assert!(res.is_err(), "縮小超過極限應回傳錯誤拒絕");
}

#[test]
/// 驗證一鍵平衡（equalize）：手動縮放多個 pane 後，呼叫 equalize 可重設回均等分配。
fn test_equalize_resets_modified_weights_to_balanced() {
    let screen = Rect::new(0, 0, 90, 30);

    // 建立 3 個並排視窗：原本各 30
    let mut layout = LayoutNode::Leaf { pane_id: 1 }
        .split_leaf(1, SplitDirection::Vertical, SplitPlacement::After, 2)
        .split_leaf(2, SplitDirection::Vertical, SplitPlacement::After, 3);

    // 調整 pane 1 寬度
    let _ = layout.resize_pane(1, SplitDirection::Vertical, 10, screen);

    let mut rects = BTreeMap::new();
    layout.render_rects(screen, &mut rects);
    assert_ne!(rects.get(&1).unwrap().width, 30);

    // 執行一鍵平衡
    layout.equalize();

    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 30);
    assert_eq!(rects.get(&2).unwrap().width, 30);
    assert_eq!(rects.get(&3).unwrap().width, 30);
}

#[test]
/// 驗證關閉 pane（close_pane）：
/// 1. 關閉中間 pane 後，剩餘兄弟視窗自動重新均分。
/// 2. 只剩一個視窗時，節點自動向上收合（Collapse）。
fn test_close_pane_rebalances_and_collapses() {
    let screen = Rect::new(0, 0, 90, 30);

    // 建立 3 個視窗 (1, 2, 3)
    let layout = LayoutNode::Leaf { pane_id: 1 }
        .split_leaf(1, SplitDirection::Vertical, SplitPlacement::After, 2)
        .split_leaf(2, SplitDirection::Vertical, SplitPlacement::After, 3);

    // 關閉中間 pane 2
    let layout = layout.close_pane(2).expect("not empty");
    let mut rects = BTreeMap::new();
    layout.render_rects(screen, &mut rects);

    // 剩餘 1 與 3 應均等平分 90 欄（各 45 欄）
    assert_eq!(rects.len(), 2);
    assert_eq!(rects.get(&1), Some(&Rect::new(0, 0, 45, 30)));
    assert_eq!(rects.get(&3), Some(&Rect::new(45, 0, 45, 30)));

    // 再關閉 pane 3
    let layout = layout.close_pane(3).expect("not empty");
    rects.clear();
    layout.render_rects(screen, &mut rects);

    // 應自動收合為單一 Leaf { pane_id: 1 }，獨佔全螢幕 90 欄
    assert_eq!(rects.len(), 1);
    assert_eq!(rects.get(&1), Some(&Rect::new(0, 0, 90, 30)));
    assert_eq!(layout, LayoutNode::Leaf { pane_id: 1 });
}

#[test]
/// 驗證餘數分配函數 calculate_split_rects：
/// 1. 寬度為 100，分成 3 份，總長度嚴格等於 100，無任何遺漏或縫隙。
/// 2. 總和計算驗證。
fn test_calculate_split_rects_integer_remainder_no_gap() {
    let area = Rect::new(0, 0, 100, 30);
    let weights = [100, 100, 100];
    let rects = calculate_split_rects(area, SplitDirection::Vertical, &weights);

    assert_eq!(rects.len(), 3);
    assert_eq!(rects[0].width, 33);
    assert_eq!(rects[1].width, 33);
    assert_eq!(rects[2].width, 34);

    let total_width: u16 = rects.iter().map(|r| r.width).sum();
    assert_eq!(total_width, 100, "子視窗寬度總和必須嚴格等於父容器寬度");
    assert_eq!(rects[2].x + rects[2].width, 100);
}

#[test]
/// 驗證使用者特別指定的情境：
/// 當一行有多個視窗時，current pane 權限最高；
/// 當 current pane 擴大時，所有其他兄弟視窗同步一起變小；
/// 當 current pane 縮小時，所有其他兄弟視窗同步一起放大。
fn test_resize_pane_multi_sibling_co_shrinking_and_growing() {
    let screen = Rect::new(0, 0, 100, 40);

    // 建立 3 個並排視窗 (pane 1: 33, pane 2: 33, pane 3: 34)
    let mut layout = LayoutNode::Leaf { pane_id: 1 }
        .split_leaf(1, SplitDirection::Vertical, SplitPlacement::After, 2)
        .split_leaf(2, SplitDirection::Vertical, SplitPlacement::After, 3);

    let mut rects = BTreeMap::new();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 33);
    assert_eq!(rects.get(&2).unwrap().width, 33);
    assert_eq!(rects.get(&3).unwrap().width, 34);

    // 1. 焦點在 pane 3：擴大 pane 3 (+4 欄)
    // 預期：pane 1 和 pane 2 一起變小（各減 2 欄，33 -> 31）
    let res = layout.resize_pane(3, SplitDirection::Vertical, 4, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 31, "pane 1 應同步變小至 31");
    assert_eq!(rects.get(&2).unwrap().width, 31, "pane 2 應同步變小至 31");
    assert_eq!(rects.get(&3).unwrap().width, 38, "pane 3 應擴大至 38");

    // 再次擴大 pane 3 (+4 欄)
    // 預期：pane 1 和 pane 2 再次一起變小（各減 2 欄，31 -> 29）
    let res = layout.resize_pane(3, SplitDirection::Vertical, 4, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 29, "pane 1 應同步變小至 29");
    assert_eq!(rects.get(&2).unwrap().width, 29, "pane 2 應同步變小至 29");
    assert_eq!(rects.get(&3).unwrap().width, 42, "pane 3 應擴大至 42");

    // 2. 焦點在 pane 3：縮小 pane 3 (-4 欄)
    // 預期：pane 1 和 pane 2 一起放大（各加 2 欄，29 -> 31）
    let res = layout.resize_pane(3, SplitDirection::Vertical, -4, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 31, "pane 1 應同步放大至 31");
    assert_eq!(rects.get(&2).unwrap().width, 31, "pane 2 應同步放大至 31");
    assert_eq!(rects.get(&3).unwrap().width, 38, "pane 3 應縮小至 38");

    // 再次縮小 pane 3 (-4 欄)
    // 預期：pane 1 和 pane 2 再次一起放大回到 (33, 33, 34)
    let res = layout.resize_pane(3, SplitDirection::Vertical, -4, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 33, "pane 1 應回復至 33");
    assert_eq!(rects.get(&2).unwrap().width, 33, "pane 2 應回復至 33");
    assert_eq!(rects.get(&3).unwrap().width, 34, "pane 3 應回復至 34");

    // 3. 焦點切換至中間的 pane 2：擴大 pane 2 (+4 欄)
    // 預期：兩側的 pane 1 和 pane 3 一起變小（各減 2 欄）
    let res = layout.resize_pane(2, SplitDirection::Vertical, 4, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 31, "pane 1 應同步變小至 31");
    assert_eq!(rects.get(&2).unwrap().width, 37, "pane 2 應擴大至 37");
    assert_eq!(rects.get(&3).unwrap().width, 32, "pane 3 應同步變小至 32");

    // 縮小 pane 2 (-4 欄)
    // 預期：兩側的 pane 1 和 pane 3 一起放大
    let res = layout.resize_pane(2, SplitDirection::Vertical, -4, screen);
    assert!(res.is_ok());
    rects.clear();
    layout.render_rects(screen, &mut rects);
    assert_eq!(rects.get(&1).unwrap().width, 33, "pane 1 應同步放大至 33");
    assert_eq!(rects.get(&2).unwrap().width, 33, "pane 2 應縮小至 33");
    assert_eq!(rects.get(&3).unwrap().width, 34, "pane 3 應同步放大至 34");
}
