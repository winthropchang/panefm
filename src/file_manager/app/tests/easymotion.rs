use super::*;

#[test]
/// 驗證按下 e 鍵進入 EasyMotion 兩階段模式，輸入目標字元篩選多個匹配項，並按下標籤跳轉。
fn easymotion_press_e_opens_mode_and_press_key_jumps_instantly() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("file_0.txt"), "0").expect("write");
    fs::write(dir.path().join("file_1.txt"), "1").expect("write");
    fs::write(dir.path().join("file_2.txt"), "2").expect("write");
    fs::write(dir.path().join("file_3.txt"), "3").expect("write");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    if let Some(pane) = app.panes.get_mut(&app.focused_pane) {
        pane.set_list_viewport_height(10);
    }

    // 初始游標停在第 0 行
    assert_eq!(app.panes.get(&app.focused_pane).unwrap().selected, 0);

    // 1. 按下 e 進入 EasyMotion 第一階段：等待輸入目標字元
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("handle e");

    match &app.pending_action {
        Some(PendingAction::EasyMotion {
            pane_id,
            target_char,
            labels,
        }) => {
            assert_eq!(*pane_id, app.focused_pane);
            assert_eq!(*target_char, None);
            assert!(labels.is_empty(), "第一階段不應有標籤，避免畫面泛黃變色");
        }
        other => panic!("預期 PendingAction::EasyMotion，實際為: {:?}", other),
    };
    assert!(app.status.contains("EASYMOTION"));

    // 2. 輸入目標開頭字母 'f'（多個匹配項，進入標籤指派階段）
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("handle target char f");

    let (jump_key, target_idx) = match &app.pending_action {
        Some(PendingAction::EasyMotion {
            pane_id,
            target_char,
            labels,
        }) => {
            assert_eq!(*pane_id, app.focused_pane);
            assert_eq!(*target_char, Some('f'));
            assert_eq!(labels.len(), 4, "4 個檔案開頭皆為 f，應被分配標籤");
            let (ch, idx) = labels[2];
            assert_eq!(idx, 2);
            (ch, idx)
        }
        other => panic!("預期進入 EasyMotion 標籤階段，實際為: {:?}", other),
    };
    assert!(app.status.contains("EASYMOTION [f]"));

    // 3. 按下對應字母標籤，游標應瞬間跳轉至 index 2 並自動退出 EasyMotion 回到 normal 模式
    app.handle_key(KeyEvent::new(KeyCode::Char(jump_key), KeyModifiers::NONE))
        .expect("handle jump char");

    assert!(app.pending_action.is_none(), "跳轉後應自動回到 normal 模式");
    assert_eq!(
        app.panes.get(&app.focused_pane).unwrap().selected,
        target_idx,
        "游標應瞬間跳至目標 index"
    );
    assert!(app.status.contains("jumped to"));
}

#[test]
/// 驗證 EasyMotion 單一匹配項時直接自動瞬移，不需額外按鍵。
fn easymotion_single_match_auto_jumps_immediately() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("apple.txt"), "a").expect("write");
    fs::write(dir.path().join("banana.txt"), "b").expect("write");
    fs::write(dir.path().join("cherry.txt"), "c").expect("write");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    if let Some(pane) = app.panes.get_mut(&app.focused_pane) {
        pane.set_list_viewport_height(10);
    }

    // 1. 按下 e 進入 EasyMotion
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("handle e");

    // 2. 輸入目標字母 'b'（只有 banana.txt 1 個匹配）
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("handle b");

    // 應直接完成跳轉並退出 EasyMotion
    assert!(app.pending_action.is_none(), "單一匹配應直接退出模式");
    let pane = app.panes.get(&app.focused_pane).unwrap();
    assert_eq!(pane.selected_entry().unwrap().name, "banana.txt");
    assert_eq!(app.status, "jumped to banana.txt");
}

#[test]
/// 驗證 EasyMotion 無匹配項目時乾淨退出並於狀態列提示。
fn easymotion_no_match_reports_status() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("apple.txt"), "a").expect("write");
    fs::write(dir.path().join("banana.txt"), "b").expect("write");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    if let Some(pane) = app.panes.get_mut(&app.focused_pane) {
        pane.set_list_viewport_height(10);
    }

    // 按下 e 進入
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("handle e");

    // 輸入 'z'（無檔案以 z 開頭）
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
        .expect("handle z");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "easymotion: no items starting with 'z'");
}

#[test]
/// 驗證 EasyMotion 模式下僅使用 Esc 鍵取消離開。
fn easymotion_cancels_on_esc() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("file_0.txt"), "0").expect("write");
    fs::write(dir.path().join("file_1.txt"), "1").expect("write");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    if let Some(pane) = app.panes.get_mut(&app.focused_pane) {
        pane.set_list_viewport_height(10);
    }

    // 第一階段：按 e 進入後按 Esc 取消
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("handle e");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::EasyMotion { .. })
    ));

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("handle esc");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "easymotion cancelled");

    // 第二階段：輸入字元後在標籤階段按 Esc 取消
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("handle e");
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("handle f");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::EasyMotion {
            target_char: Some('f'),
            ..
        })
    ));

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("handle esc");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "easymotion cancelled");
}

#[test]
/// 驗證 :easymotion 命令亦可開啟跳轉模式。
fn easymotion_command_activates_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("file_0.txt"), "0").expect("write");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    if let Some(pane) = app.panes.get_mut(&app.focused_pane) {
        pane.set_list_viewport_height(10);
    }

    app.execute_command("easymotion").expect("exec easymotion");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::EasyMotion { .. })
    ));
}
