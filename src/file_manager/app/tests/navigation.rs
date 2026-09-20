use super::*;

#[test]
/// 驗證 normal mode 按下第一個 `g` 會先打開 `g` 系列命令面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_g_opens_go_picker() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("open go picker");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::GoPicker { pane_id: 1 })
    ));
    assert_eq!(app.status, "go: choose g/t/d/k/l from the panel");
}

#[test]
/// 驗證 normal mode 按下 `gt` 會先經過 `g` 面板，再打開預填好的 `goto ` 命令輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_gt_opens_prefilled_goto_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("pending g");
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("open prefilled goto command");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "goto ");
    assert_eq!(app.status, "command mode");
}

#[test]
/// 驗證 `gd` 會直接切到使用者的 Documents 目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_gd_jumps_to_documents_directory() {
    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let documents = home.join("Documents");
    fs::create_dir_all(&documents).expect("documents");

    let original_home = std::env::var_os("HOME");
    let original_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("open go picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("jump documents");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, documents);
    }

    unsafe {
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}

#[test]
/// 驗證 `gk` 會直接切到使用者的 Desktop 目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_gk_jumps_to_desktop_directory() {
    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let desktop = home.join("Desktop");
    fs::create_dir_all(&desktop).expect("desktop");

    let original_home = std::env::var_os("HOME");
    let original_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("open go picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("jump desktop");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, desktop);
    }

    unsafe {
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}

#[test]
/// 驗證 `gl` 會直接切到使用者的 Downloads 目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_gl_jumps_to_downloads_directory() {
    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let downloads = home.join("Downloads");
    fs::create_dir_all(&downloads).expect("downloads");

    let original_home = std::env::var_os("HOME");
    let original_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("open go picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("jump downloads");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, downloads);
    }

    unsafe {
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}

#[test]
/// 驗證 `:goto <path>` 會讓目前 pane 跳到指定子目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_goto_command_changes_to_target_directory() {
    let dir = tempdir().expect("tempdir");
    let docs = dir.path().join("docs");
    fs::create_dir(&docs).expect("docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("goto docs").expect("goto command");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
    assert_eq!(app.status, format!("jumped to path: {}", docs.display()));
}

#[test]
/// 驗證 Windows 磁碟機路徑會被當成絕對路徑，而不是相對於目前目錄拼接。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn windows_drive_path_is_recognized_as_absolute_like_path() {
    assert!(is_windows_drive_path("C:/"));
    assert!(is_windows_drive_path("D:\\work"));
    assert!(looks_like_navigation_path("R:/repo"));
    assert!(!is_windows_drive_path("docs/readme"));
}

#[test]
/// 驗證 Windows UNC 與 macOS `/Volumes` 目的地都會被視為背景傳輸目標。
///
/// 保護目的：兩個正式支援平台使用不同網路路徑形式；若任一形式漏判，大檔案貼上
/// 就可能退回主執行緒並再次凍結 TUI。
fn network_destination_detection_covers_windows_and_macos() {
    assert!(is_probably_network_or_external_path(std::path::Path::new(
        "//server/share"
    )));
    assert!(is_probably_network_or_external_path(std::path::Path::new(
        "/Volumes/company/share"
    )));
    assert!(!is_probably_network_or_external_path(std::path::Path::new(
        "/Users/otto/Documents"
    )));
}

#[test]
/// 驗證 UNC goto 的 loader 即使尚未完成，啟動函式仍會立即把控制權交回 TUI。
///
/// 保護目的：公司網路主機不存在或 SMB 回應緩慢時，Windows 檔案系統呼叫可能
/// 等待很久；此測試用 channel 人為暫停 worker，確認主執行緒仍可用 Esc 取消，
/// 且取消後晚到的結果不會覆蓋原本 panel。
fn app_unc_goto_runs_in_background_and_escape_discards_late_result() {
    let dir = tempdir().expect("tempdir");
    let original_cwd = dir.path().to_path_buf();
    let mut app = App::new(original_cwd.clone(), default_loaded_config()).expect("app");
    let target = std::path::PathBuf::from("//192.0.2.10/share");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    app.start_network_goto_with(target, move |mut pane, target| {
        started_tx.send(()).expect("report worker started");
        release_rx.recv().expect("release worker");
        pane.cwd = target;
        Ok(pane)
    })
    .expect("start background goto");

    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("background loader should start without blocking caller");
    assert!(app.active_network_goto_task_id.is_some());

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel network goto");
    release_tx.send(()).expect("finish stale worker");
    thread::sleep(Duration::from_millis(10));
    app.poll_background_tasks();

    assert!(app.active_network_goto_task_id.is_none());
    assert_eq!(app.panes.get(&1).expect("pane").cwd, original_cwd);
    assert!(matches!(
        app.task_log.last().map(|task| task.state),
        Some(TaskState::Cancelled)
    ));
}

#[test]
/// 驗證一般列表模式下，方向鍵會走和 `hjkl` 相同的移動與進出目錄邏輯。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_normal_mode_arrow_keys_map_to_vim_movement() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let initial_cwd = app.panes.get(&1).expect("pane").cwd.clone();

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("move down");
    let selected_after_down = app
        .panes
        .get(&1)
        .expect("pane")
        .selected_entry()
        .expect("selected")
        .path
        .clone();
    assert_eq!(selected_after_down, beta);

    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .expect("move up");
    let selected_after_up = app
        .panes
        .get(&1)
        .expect("pane")
        .selected_entry()
        .expect("selected")
        .path
        .clone();
    assert_eq!(selected_after_up, alpha);

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .expect("enter directory");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, alpha);

    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("go parent");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, initial_cwd);
}

#[test]
/// 驗證一般列表模式用 `l` / `Left` / `Right` 切換目錄後，zoxide 也會同步學習這些位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_normal_mode_directory_navigation_updates_zoxide() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    fs::create_dir(&alpha).expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .expect("enter directory");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, alpha);

    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("go parent");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, dir.path());

    let tracked = query_zoxide_directories().expect("query zoxide");
    assert!(
        tracked.iter().any(|path| path == &alpha),
        "expected zoxide to contain {}",
        alpha.display()
    );
    assert!(
        tracked.iter().any(|path| path == dir.path()),
        "expected zoxide to contain {}",
        dir.path().display()
    );
}

#[test]
/// 驗證 normal mode 的 `J / K` 會用固定大步長快速移動列表游標。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_j_and_k_move_by_large_step() {
    let dir = tempdir().expect("tempdir");
    for index in 0..12 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
        .expect("fast down");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
    assert_eq!(app.status, "fast down: 5");

    app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
        .expect("fast up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(app.status, "fast up: 5");
}

#[test]
/// 驗證已掛載的 SMB share 可以直接經由 `goto smb://...` 切進目前 pane。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_goto_smb_location_enters_mounted_share() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_root = mount_root.join("shared");
    fs::create_dir_all(share_root.join("docs")).expect("share docs");
    fs::write(share_root.join("docs").join("report.txt"), "hello").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root("smb://192.0.2.10/shared/docs", &mount_root)
        .expect("goto smb");

    let pane = app.current_pane_mut().expect("pane");
    assert_eq!(pane.cwd, share_root.join("docs"));
    assert_eq!(app.status, "jumped to smb: smb://192.0.2.10/shared/docs");
    assert!(app.take_full_redraw_request());
    assert!(!app.take_full_redraw_request());
}

#[test]
/// 驗證尚未掛載的 SMB share 在 `goto smb://...` 時會先發出系統掛載請求。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_goto_smb_location_requests_mount_when_share_missing() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    fs::create_dir_all(&mount_root).expect("mount root");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root("smb://192.0.2.10/shared/docs", &mount_root)
        .expect("goto smb");

    assert!(app.pending_launch.is_some());
    assert_eq!(
        app.status,
        format!(
            "已請求系統掛載 SMB：smb://192.0.2.10/shared/docs；若系統連線失敗，請檢查主機、share 名稱、網路與權限，成功後再重試。預期掛載位置：{}",
            mount_root.join("shared").join("docs").display()
        )
    );
}

#[test]
/// 驗證 UNC 正斜線格式 `//host/share/path` 在已掛載時可直接切入 pane。
fn app_goto_smb_location_with_unc_forward_slash_enters_mounted_share() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_root = mount_root.join("shared");
    fs::create_dir_all(share_root.join("docs")).expect("share docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root("//192.0.2.10/shared/docs", &mount_root)
        .expect("goto unc forward slash");

    let pane = app.current_pane_mut().expect("pane");
    assert_eq!(pane.cwd, share_root.join("docs"));
    assert_eq!(app.status, "jumped to smb: smb://192.0.2.10/shared/docs");
}

#[test]
/// 驗證 Windows UNC 反斜線格式 `\\host\share\path` 在已掛載時可直接切入 pane。
fn app_goto_smb_location_with_unc_backslash_enters_mounted_share() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_root = mount_root.join("shared");
    fs::create_dir_all(share_root.join("docs")).expect("share docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root(r"\\192.0.2.10\shared\docs", &mount_root)
        .expect("goto unc backslash");

    let pane = app.current_pane_mut().expect("pane");
    assert_eq!(pane.cwd, share_root.join("docs"));
    assert_eq!(app.status, "jumped to smb: smb://192.0.2.10/shared/docs");
}

#[test]
/// 驗證 UNC 格式在尚未掛載時亦能正確轉為 `smb://` 併發出系統掛載請求。
fn app_goto_smb_location_with_unc_requests_mount_when_share_missing() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    fs::create_dir_all(&mount_root).expect("mount root");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root("//192.0.2.10/shared/docs", &mount_root)
        .expect("goto unc");

    assert!(app.pending_launch.is_some());
    assert_eq!(
        app.status,
        format!(
            "已請求系統掛載 SMB：smb://192.0.2.10/shared/docs；若系統連線失敗，請檢查主機、share 名稱、網路與權限，成功後再重試。預期掛載位置：{}",
            mount_root.join("shared").join("docs").display()
        )
    );
}

#[test]
/// 驗證在 macOS 模式下，直接以 UNC 路徑呼叫 command 跳轉會自動進入 SMB 掛載與跳轉機制。
fn app_change_directory_from_command_as_macos_routes_unc_to_smb() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_root = mount_root.join("shared");
    fs::create_dir_all(share_root.join("docs")).expect("share docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.change_directory_from_command_as_macos("//192.0.2.10/shared/docs", &mount_root)
        .expect("goto unc as macos");

    let pane = app.current_pane_mut().expect("pane");
    assert_eq!(pane.cwd, share_root.join("docs"));
    assert_eq!(app.status, "jumped to smb: smb://192.0.2.10/shared/docs");
}

#[test]
/// 驗證 SMB 指令帶有引號或大小寫不同時仍能正確跳轉。
fn app_change_directory_from_command_as_macos_handles_quotes_and_case() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_root = mount_root.join("shared");
    fs::create_dir_all(share_root.join("docs")).expect("share docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.change_directory_from_command_as_macos(r#""SMB://192.0.2.10/shared/docs""#, &mount_root)
        .expect("goto quoted smb");

    let pane = app.current_pane_mut().expect("pane");
    assert_eq!(pane.cwd, share_root.join("docs"));
    assert_eq!(app.status, "jumped to smb: SMB://192.0.2.10/shared/docs");
}

#[test]
/// 驗證 SMB 目標的子路徑不存在但掛載根目錄存在時，會平滑降級進入根目錄並提示。
fn app_goto_smb_location_falls_back_to_root_when_subpath_missing() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_root = mount_root.join("shared");
    fs::create_dir_all(&share_root).expect("share root");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.change_directory_from_command_as_macos(
        "smb://192.0.2.10/shared/nonexistent_folder",
        &mount_root,
    )
    .expect("goto smb missing subpath");

    let pane = app.current_pane_mut().expect("pane");
    assert_eq!(pane.cwd, share_root);
    assert!(app.status.starts_with("SMB 子路徑不存在"));
}

#[test]
/// 驗證按下 `z` 後會建立 `fzf` 跳轉請求，並記住目前 pane 的根目錄設定。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_jump_key_queues_fzf_request() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("readme.md"), "b").expect("readme");
    fs::write(dir.path().join("report.txt"), "c").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
        .expect("open jump");

    let request = app.take_pending_fzf_jump().expect("fzf request");

    assert_eq!(request.pane_id, 1);
    assert_eq!(request.root_dir, dir.path());
    assert!(request.show_hidden);
    assert!(request.follow_links);
    assert!(app.pending_fzf_jump.is_none());
    assert_eq!(app.status, "jump: fzf loading");
}

#[test]
/// 驗證分割成多個 pane 後，在目前 focus 的 pane 按下 `z` 仍會建立 `fzf` 請求。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_jump_key_works_from_focused_split_pane() {
    let dir = tempdir().expect("tempdir");
    let left_dir = dir.path().join("left");
    let right_dir = dir.path().join("right");
    fs::create_dir(&left_dir).expect("left");
    fs::create_dir(&right_dir).expect("right");
    fs::write(left_dir.join("alpha.txt"), "a").expect("alpha");
    fs::write(right_dir.join("beta.txt"), "b").expect("beta");

    let mut app = App::new(left_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = right_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    assert_eq!(app.focused_pane, 2);
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
        .expect("open jump");

    let request = app.take_pending_fzf_jump().expect("fzf request");

    assert_eq!(request.pane_id, 2);
    assert_eq!(request.root_dir, right_dir);
    assert!(request.show_hidden);
    assert!(request.follow_links);
    assert_eq!(app.status, "jump: fzf loading");
}

#[test]
/// 驗證 `z` 使用的 `fzf` 搜尋會固定包含 hidden 內容，不受 pane 顯示設定影響。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_jump_key_always_searches_hidden_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".secret.txt"), "secret").expect("secret");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.current_pane_mut().expect("pane").show_hidden = false;
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
        .expect("open jump");

    let request = app.take_pending_fzf_jump().expect("fzf request");
    assert!(request.show_hidden);
    assert!(request.follow_links);
}

#[test]
/// 驗證套用 `fzf` 選取結果後，游標會跳到對應的檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_apply_fzf_jump_selection_moves_cursor_to_match() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("readme.md"), "b").expect("readme");
    fs::write(dir.path().join("report.txt"), "c").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_fzf_jump();
    let request = app.take_pending_fzf_jump().expect("fzf request");
    app.apply_fzf_jump_selection(request, Some("report.txt"));

    assert_eq!(
        app.panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "report.txt"
    );
    assert_eq!(app.status, "jumped: report.txt");
}

#[test]
/// 驗證套用巢狀 `fzf` 結果後，pane 會切到檔案所在目錄並聚焦正確項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_apply_fzf_jump_selection_reveals_nested_file() {
    let dir = tempdir().expect("tempdir");
    let nested_dir = dir.path().join("docs");
    fs::create_dir(&nested_dir).expect("docs");
    fs::write(nested_dir.join("guide.md"), "guide").expect("guide");
    fs::write(dir.path().join("root.txt"), "root").expect("root");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_fzf_jump();
    let request = app.take_pending_fzf_jump().expect("fzf request");
    app.apply_fzf_jump_selection(request, Some("docs/guide.md"));

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.cwd, nested_dir);
    assert_eq!(pane.selected_entry().expect("selected").name, "guide.md");
    assert_eq!(app.status, "jumped: docs/guide.md");
}

#[test]
/// 驗證取消 `fzf` 選擇時，不會改動目前游標位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_apply_fzf_jump_selection_cancel_keeps_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("readme.md"), "b").expect("readme");
    fs::write(dir.path().join("report.txt"), "c").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move to readme");
    let original = app.panes.get(&1).expect("pane").selected;

    app.open_fzf_jump();
    let request = app.take_pending_fzf_jump().expect("fzf request");
    app.apply_fzf_jump_selection(request, None);

    assert_eq!(app.panes.get(&1).expect("pane").selected, original);
    assert_eq!(app.status, "jump cancelled");
}

#[test]
/// 驗證在目錄背景載入尚未完成時按 h 離開，會即時取消舊工作且不把空列表寫入快取。
/// 保護目的：確保使用者在大型目錄快速切出時不卡死，且不會把未完成的空清單當作快取。
fn navigating_away_during_load_cancels_load_and_guards_cache() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    fs::create_dir_all(&child).expect("child dir");
    fs::write(child.join("payload.txt"), "data").expect("payload");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    // 進入 child 目錄，此時啟動背景載入
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter child");
    assert_eq!(app.panes[&1].cwd, child);
    assert!(app.directory_load_jobs.contains_key(&1));

    // 尚未 poll 完成前立刻按 h 離開回到 parent
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("return to parent");
    assert_eq!(app.panes[&1].cwd, dir.path());

    // 快取中不能存入 child 的空清單
    assert_ne!(
        app.directory_entry_cache.get(&child),
        Some(&Vec::new()),
        "載入中離開目錄不得將空陣列存入快取"
    );
}

#[test]
/// 驗證大型目錄以串流載入時，首批清單到達後游標即可立刻以 j/k 移動，不需等待全量掃描結束。
/// 保護目的：確保使用者在幾萬個檔案的目錄中，畫面在毫秒級反應，游標絕不被背景 I/O 凍結。
fn streaming_directory_load_allows_cursor_movement_before_completion() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    fs::create_dir_all(&child).expect("child");
    for i in 0..300 {
        fs::write(child.join(format!("file_{i:03}.txt")), b"test").expect("write");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter child");

    // 等待首批快速 chunk 到達
    for _ in 0..100 {
        app.poll_background_tasks();
        if !app.panes[&1].entries.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    // 首批項目已在畫面上，游標在第 0 筆
    assert!(!app.panes[&1].entries.is_empty());
    assert_eq!(app.panes[&1].selected, 0);

    // 立刻按下 j 移動游標，游標必須成功移動到第 1 筆
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    assert_eq!(app.panes[&1].selected, 1, "游標必須在載入中立即響應移動");

    // 等待背景全量載入結束
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.directory_load_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(app.panes[&1].entries.len(), 300);
    assert_eq!(
        app.panes[&1].selected, 1,
        "全量完成後仍必須保留使用者剛剛移動的游標位置"
    );
}

#[test]
/// 驗證當背景有檔案傳輸或寫入進行時，使用者不可進入該正在寫入的目錄。
fn cannot_enter_directory_while_transfer_is_in_progress() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let child = app.panes[&1].cwd.join("copying_target");
    fs::create_dir(&child).expect("create child");
    app.panes.get_mut(&1).unwrap().reload().expect("reload");

    // 模擬一個正在進行中的背景傳輸工作綁定到 child
    let task_id = app.push_task(
        1,
        "paste",
        "copy files".to_string(),
        "dest".to_string(),
        vec![child.display().to_string()],
        Some(dir.path().display().to_string()),
    );
    app.active_file_job_busy_paths
        .insert(task_id, vec![child.clone()]);
    app.update_task_progress(task_id, 45, 100);

    // 選中該目錄並嘗試進入
    app.panes.get_mut(&1).unwrap().select_path(&child);
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("press l");

    // 工作目錄不可被切換，且狀態列必須提示傳輸進行中
    assert_eq!(app.panes[&1].cwd, child.parent().unwrap());
    assert!(app.status.contains("cannot enter 'copying_target/'"));
    assert!(app.status.contains("transfer in progress"));
    assert!(app.status.contains("45%"));
}

#[test]
/// 驗證當子目錄或檔案在進行背景傳輸時，其父目錄依然可以正常進入，且進入後該子項目會顯示進度標籤。
fn parent_directory_is_enterable_while_child_is_being_transferred() {
    let dir = tempdir().expect("tempdir");
    let parent = dir.path().join("AB_Demo");
    let child = parent.join("terminal-file-manager");
    fs::create_dir_all(&child).expect("create child dir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 模擬背景傳輸工作鎖定子項目 child
    let task_id = app.push_task(
        1,
        "paste",
        "copy 1 item(s)".to_string(),
        "dest".to_string(),
        vec![child.display().to_string()],
        Some(parent.display().to_string()),
    );
    app.active_file_job_busy_paths
        .insert(task_id, vec![child.clone()]);
    app.update_task_progress(task_id, 99, 100);

    // 父目錄 parent 必須不受影響，不可被視為 busy
    assert!(
        app.active_file_job_for_path(&parent).is_none(),
        "父目錄不可被子項目的傳輸鎖定"
    );
    // 子項目 child 必須被鎖定並能提供 progress badge
    assert!(app.active_file_job_for_path(&child).is_some());
    assert_eq!(
        app.active_job_badge_for_path(&child).as_deref(),
        Some("[copying 99%]")
    );

    // 嘗試進入父目錄 parent
    app.panes.get_mut(&1).unwrap().select_path(&parent);
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter parent dir");

    // 必須成功進入父目錄
    assert_eq!(
        app.panes[&1].cwd.canonicalize().unwrap(),
        parent.canonicalize().unwrap()
    );
}
