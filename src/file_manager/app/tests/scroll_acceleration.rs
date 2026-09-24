use super::*;
use std::thread::sleep;

#[test]
/// 驗證智能長按滾動加速的步長階梯遞增邏輯：
/// 1~3 次為單格微調 (1 格)，4~7 次為平滑雙倍速 (2 格)，8 次以上為巡航極速 (3 格)。
fn test_scroll_acceleration_step_progression() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 第 1 ~ 3 次連按：精準 1 格
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 1);
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 1);
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 1);

    // 第 4 ~ 7 次連按：平滑雙倍速 2 格
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 2);
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 2);
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 2);
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 2);

    // 第 8 次以上：巡航極速 3 格
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 3);
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 3);
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 3);
}

#[test]
/// 驗證方向反轉（Down 改為 Up）時加速狀態瞬時歸零，重新從 1 格精準微調起跳。
fn test_scroll_acceleration_direction_reversal_resets() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 先累積到 2 倍速 (5 次)
    for _ in 0..5 {
        app.take_vertical_nav_step(NavDirection::Down);
    }
    assert_eq!(app.nav_acceleration.map(|a| a.repeat_count), Some(5));

    // 突然往反方向 Up 移動：瞬時歸 1
    assert_eq!(app.take_vertical_nav_step(NavDirection::Up), 1);
    assert_eq!(
        app.nav_acceleration.map(|a| (a.direction, a.repeat_count)),
        Some((NavDirection::Up, 1))
    );
}

#[test]
/// 驗證連按間隔超過連發閾值（> 150ms）時加速狀態自動過期重設。
fn test_scroll_acceleration_timeout_resets() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 連按 5 次
    for _ in 0..5 {
        app.take_vertical_nav_step(NavDirection::Down);
    }
    assert_eq!(app.nav_acceleration.map(|a| a.repeat_count), Some(5));

    // 模擬手放開超過連發閾值（休眠 160ms）
    sleep(Duration::from_millis(160));

    // 下一次按下時重新歸為 1 格
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 1);
    assert_eq!(app.nav_acceleration.map(|a| a.repeat_count), Some(1));
}

#[test]
/// 驗證當設定檔停用 scroll_acceleration 時，無論連按幾次步長固定為 1。
fn test_scroll_acceleration_disabled_via_config() {
    let dir = tempdir().expect("tempdir");
    let mut config = default_loaded_config();
    config.config.navigation.scroll_acceleration = false;
    let mut app = App::new(dir.path().to_path_buf(), config).expect("app");

    for _ in 0..10 {
        assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 1);
    }
}

#[test]
/// 驗證當帶有手動輸入 count prefix（如 5j）時，優先使用手動 count 並重設加速。
fn test_scroll_acceleration_with_pending_count_priority() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 先累積加速
    for _ in 0..5 {
        app.take_vertical_nav_step(NavDirection::Down);
    }

    // 設定手動 count = 7
    app.pending_count = Some(7);

    // 呼叫 take_vertical_nav_step：回傳 7 且加速狀態清空
    assert_eq!(app.take_vertical_nav_step(NavDirection::Down), 7);
    assert!(app.nav_acceleration.is_none());
}

#[test]
/// 驗證 reset_pending_motion_state 會同時清空導航加速狀態。
fn test_scroll_acceleration_cleared_by_reset_motion() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    for _ in 0..5 {
        app.take_vertical_nav_step(NavDirection::Down);
    }
    assert!(app.nav_acceleration.is_some());

    app.reset_pending_motion_state();
    assert!(app.nav_acceleration.is_none());
}

#[test]
/// 驗證真實 App 收到 j 與 k 按鍵時游標的加速行為。
fn test_app_j_and_k_navigation_accelerates_in_real_pane() {
    let dir = tempdir().expect("tempdir");
    // 建立 30 個測試檔案
    for i in 0..30 {
        fs::write(dir.path().join(format!("file_{:02}.txt", i)), "hello").expect("write file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane 1")
        .reload()
        .expect("reload");

    assert_eq!(app.panes.get(&1).expect("pane 1").selected, 0);

    // 連按 3 次 j：每次 +1 -> 游標到 3
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("key j");
    }
    assert_eq!(app.panes.get(&1).expect("pane 1").selected, 3);

    // 第 4 次 j：進入 Tier 1 加速 (+2) -> 游標到 5
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("key j");
    assert_eq!(app.panes.get(&1).expect("pane 1").selected, 5);

    // 第 5 次 j：持續 Tier 1 加速 (+2) -> 游標到 7
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("key j");
    assert_eq!(app.panes.get(&1).expect("pane 1").selected, 7);

    // 按 k 向上：方向反轉，重設為 +1 -> 游標回到 6
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("key k");
    assert_eq!(app.panes.get(&1).expect("pane 1").selected, 6);
}
