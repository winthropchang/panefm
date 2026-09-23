use std::time::Duration;

use super::*;

#[test]
/// 驗證應用程式在閒置無背景任務時正確判定，且給予放寬之待機睡眠逾時（消除待機 CPU 負擔）。
fn idle_state_and_event_timeout_detection() {
    let dir = tempdir().expect("tempdir");
    let app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 初始無任務時，應判定為無活躍背景工作
    assert!(
        !app.has_active_background_tasks(),
        "初始待機狀態不應有活躍背景工作"
    );

    // 待機逾時應放寬至 300ms，使作業系統真正休眠
    let idle_timeout = app.next_event_timeout(Duration::from_millis(150));
    assert_eq!(
        idle_timeout,
        Duration::from_millis(300),
        "待機狀態應採用 300ms 放寬逾時"
    );
}

#[test]
/// 驗證在沒有任何背景事件產生時，poll_background_tasks() 回傳 false，避免無謂重繪。
fn poll_background_tasks_returns_false_when_idle() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let changed = app.poll_background_tasks();
    assert!(
        !changed,
        "完全無任何異動與事件時，poll_background_tasks 必須回傳 false 以免觸發 CPU 重繪"
    );
}
