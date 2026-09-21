use super::*;

#[test]
/// 驗證刪除確認流程在確認後會真正刪除選取項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_delete_confirmation_removes_selected_entry() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("delete-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete { .. })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm delete");

    assert!(!file_path.exists());
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "trashed delete-me.txt");
}

#[test]
/// 驗證刪除確認視窗再次按 `d` 會關閉視窗，而不會執行刪除或移入 trash。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_delete_confirmation_d_closes_without_deleting() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("keep-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("close delete confirmation");

    assert!(file_path.exists());
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "trash cancelled: keep-me.txt");
}

#[test]
/// 驗證兩個開在同一目錄的 panel，其中一個刪除檔案後，另一個也會同步刷新列表。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_delete_refreshes_other_panels_in_same_directory() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("shared.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.focus_pane_by_id(1);

    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");

    assert!(!file_path.exists());
    for pane_id in [1, 2] {
        let pane = app.panes.get(&pane_id).expect("pane");
        assert!(
            pane.entries.iter().all(|entry| entry.path != file_path),
            "panel {pane_id} still shows deleted file"
        );
    }
}

#[test]
/// 驗證在一般列表按下 Enter 會依預設外部開啟規則排入文字編輯器啟動。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_enter_queues_default_open_for_text_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("default open");

    let launch = app.take_pending_launch().expect("launch");
    let expected = if std::env::var("EDITOR")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        LaunchMode::TerminalBlocking
    } else {
        LaunchMode::Detached
    };
    assert_eq!(launch.launch.mode, expected);
    assert_eq!(app.status, "opening notes.txt with editor");
}

#[test]
/// 驗證 Move / LineMode 面板打開後，按 `m` 會開啟預填 `:move ` 的命令列。
fn app_move_shortcut_mm_opens_move_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open move linemode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("trigger move command");

    assert!(app.pending_action.is_none());
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "move ");
}

#[test]
/// 驗證 Move / LineMode 面板打開後，按 `p` 會開啟預填 `:move-panel ` 的命令列。
fn app_move_shortcut_mp_opens_move_panel_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open move linemode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("trigger move-panel command");

    assert!(app.pending_action.is_none());
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "move-panel ");
}

#[test]
/// 驗證 Move / LineMode 面板打開後，按數字 `2` 會直接將檔案搬移到 Pane 2。
fn app_move_shortcut_digit_moves_directly_to_target_pane() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("sample.txt");
    fs::write(&source_file, "move sample").expect("write");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open move linemode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .expect("trigger move to pane 2");

    assert!(app.pending_action.is_none());
    assert!(!source_file.exists());
    assert!(target_dir.join("sample.txt").exists());
    assert_eq!(
        app.status,
        format!("moved 1 item -> {}", target_dir.display())
    );
}

#[test]
/// 驗證目錄讀取時即使開啟 show_hidden 也會自動過濾掉 .panefm-transfer-* 暫存檔案。
fn app_directory_listing_ignores_internal_temporary_files() {
    let dir = tempdir().expect("tempdir");
    let test_dir = dir.path().join("project");
    fs::create_dir(&test_dir).expect("create dir");

    fs::write(test_dir.join("regular.txt"), "regular").expect("write regular");
    fs::write(test_dir.join(".dotfile"), "dotfile").expect("write dotfile");
    fs::write(test_dir.join(".panefm-transfer-999-1.part"), "temp").expect("write temp");

    let mut app = App::new(test_dir, default_loaded_config()).expect("app");
    let pane = app.current_pane_mut().expect("pane");
    pane.set_show_hidden(true);
    pane.reload().expect("reload");

    let names: Vec<_> = pane.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"regular.txt"));
    assert!(names.contains(&".dotfile"));
    assert!(!names.contains(&".panefm-transfer-999-1.part"));
}

#[test]
/// 驗證按下 `Space` 會切換目前選取項目的標記狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_space_toggles_mark_on_selected_entry() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .expect("mark selected");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 1);
    assert_eq!(app.status, "marked alpha.txt");

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .expect("unmark selected");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
    assert_eq!(app.status, "unmarked alpha.txt");
}

#[test]
/// 驗證按下 `Ctrl-r` 會反轉目前所有可見項目的標記狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_r_inverts_visible_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .expect("invert marks");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);
    assert_eq!(app.status, "inverted visible marks (+2, -0, total 2)");

    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .expect("invert marks again");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
    assert_eq!(app.status, "inverted visible marks (+0, -2, total 0)");
}

#[test]
/// 驗證按下 `D` 後確認，會直接永久刪除目前選取項目而不是丟進 trash。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_d_deletes_selected_entry_permanently() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("delete-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("start permanent delete");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete {
            permanent: true,
            ..
        })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm permanent delete");

    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(!file_path.exists());
    assert!(
        app.trash_store
            .list_entries()
            .expect("trash entries")
            .is_empty()
    );
    assert_eq!(app.status, "deleted permanently delete-me.txt");
}

#[test]
/// 驗證 `:move <path>` 會把目前選取的檔案直接移到指定目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_move_command_moves_selected_entry_to_target_dir() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("gamma.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.execute_command(&format!("move {}", target_dir.display()))
        .expect("move command");

    assert!(!source_file.exists());
    assert!(target_dir.join("gamma.txt").exists());
    assert_eq!(
        app.status,
        format!("moved 1 item -> {}", target_dir.display())
    );
}

#[test]
/// 驗證 `:move-panel <id>` 會把目前選取的檔案移到指定 pane 的目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_move_panel_command_moves_selected_entry_to_target_pane_dir() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("delta.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);

    app.execute_command("move-panel 2").expect("move panel");

    assert!(!source_file.exists());
    assert!(target_dir.join("delta.txt").exists());
    assert_eq!(
        app.status,
        format!("moved 1 item -> {}", target_dir.display())
    );
}

#[test]
/// 驗證 `:move-panel` 移動大檔案或目錄時，會自動交給背景工作處理以避免卡住 TUI。
fn app_move_panel_command_runs_in_background_when_target_is_external() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("large.zip");
    fs::File::create(&source_file)
        .expect("create file")
        .set_len(BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
        .expect("set len");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    app.execute_command("move-panel 2").expect("move panel");

    assert!(!app.file_job_receivers.is_empty());
    assert!(app.task_log.iter().any(|t| t.title.starts_with("move")));
}

#[test]
/// 驗證 `:compress` 會把目前選取項目壓成 zip，並把游標帶到新壓縮檔。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_compress_command_creates_zip_and_reveals_result() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello zip").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("compress").expect("compress");

    let archive_path = dir.path().join("notes.txt.zip");
    assert!(archive_path.exists());
    assert_eq!(app.status, "compressed notes.txt -> notes.txt.zip");
    assert_eq!(
        app.current_pane_mut()
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "notes.txt.zip"
    );
}

#[test]
/// 驗證 `:extract` 會解開目前選取的 zip，並將游標帶到輸出目錄。
/// 保護目的：同時確認資料夾壓縮會先排入背景 task，完成後仍能選中新 ZIP 並接續
/// 解壓，避免非阻塞重構破壞原本的完整操作流程。
fn app_extract_command_unpacks_zip_and_reveals_output() {
    let dir = tempdir().expect("tempdir");
    let folder = dir.path().join("demo");
    fs::create_dir(&folder).expect("dir");
    fs::write(folder.join("alpha.txt"), "hello").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("compress").expect("compress dir");
    assert!(
        !app.file_job_receivers.is_empty(),
        "directory compression must not block the TUI thread"
    );
    wait_for_file_jobs(&mut app);

    let archive_path = dir.path().join("demo.zip");
    assert!(archive_path.exists());

    app.execute_command("extract").expect("extract zip");

    let extracted_dir = dir.path().join("demo copy");
    assert!(extracted_dir.is_dir());
    assert!(extracted_dir.join("demo").join("alpha.txt").exists());
    assert_eq!(app.status, "extracted demo copy");
    assert_eq!(
        app.current_pane_mut()
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "demo copy"
    );
}

#[test]
/// 驗證 `:move-panel <id>` 若指定不存在的 pane，會提示目前可用的 pane 編號。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_move_panel_command_reports_available_panes_for_unknown_target() {
    let dir = tempdir().expect("tempdir");
    let source_file = dir.path().join("epsilon.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("move-panel 9").expect("move panel");

    assert!(source_file.exists());
    assert_eq!(app.status, "unknown panel 9. available: 1");
}

#[test]
/// 驗證按下 `o` 後會打開建立新檔案的 inline 輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_start_create_entry_opens_inline_editor() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.start_create_entry();

    assert_eq!(
        app.pending_action,
        Some(PendingAction::CreateEntry {
            pane_id: 1,
            buffer: String::new(),
            cursor: 0,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證命令模式可以直接建立一般檔案與結尾 `/` 的資料夾。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_create_commands_create_entries_without_inline_prompt() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.execute_command("create alpha.txt")
        .expect("create file");
    assert!(dir.path().join("alpha.txt").exists());
    assert_eq!(app.status, "created file: alpha.txt");

    app.execute_command("create docs/").expect("create dir");
    assert!(dir.path().join("docs").is_dir());
    assert_eq!(app.status, "created directory: docs/");
}

#[test]
/// 驗證建立流程的 inline 輸入框在 Enter 後會真的建立檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_create_file_confirm_creates_entry() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::CreateEntry {
        pane_id: 1,
        buffer: String::from("draft.md"),
        cursor: 8,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("create file");

    assert!(dir.path().join("draft.md").exists());
    assert_eq!(app.status, "created file: draft.md");
}

#[test]
/// 驗證建立流程支援巢狀路徑，會先補齊父目錄再建立檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_create_nested_file_from_inline_prompt() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::CreateEntry {
        pane_id: 1,
        buffer: String::from("test/gg.txt"),
        cursor: 11,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("create nested file");

    assert!(dir.path().join("test").is_dir());
    assert!(dir.path().join("test").join("gg.txt").exists());
    assert_eq!(app.status, "created file: test/gg.txt");
}

#[test]
/// 驗證按下 `.` 後會顯示隱藏檔，並可與 filter 一起使用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_toggle_hidden_reveals_hidden_entries_and_works_with_filter() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".secret"), "s").expect("hidden");
    fs::write(dir.path().join("alpha.txt"), "a").expect("normal");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let initial_names: Vec<String> = app
        .panes
        .get(&1)
        .expect("pane")
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();
    assert_eq!(initial_names, vec![String::from("alpha.txt")]);

    app.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
        .expect("toggle hidden");
    assert_eq!(app.status, "showing hidden files");

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("filter hidden");

    let filtered_names: Vec<String> = app
        .panes
        .get(&1)
        .expect("pane")
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();
    assert_eq!(filtered_names, vec![String::from(".secret")]);
}

#[test]
/// 驗證 normal mode 按下單鍵 `A` (Shift+a) 會把目前 pane 的所有可見項目全部標記起來。
/// 保護目的：提供最直覺、好按的單手/雙鍵全選體驗。
fn app_capital_a_marks_all_visible_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT))
        .expect("mark all via capital A");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.marked_count(), 3);
    assert_eq!(app.status, "marked all visible items (+3, total 3)");
}

#[test]
/// 驗證 `:mark-all` 命令也能把目前 pane 的所有可見項目全部標記起來。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_mark_all_command_marks_all_visible_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("mark-all").expect("mark-all command");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.marked_count(), 2);
    assert_eq!(app.status, "marked all visible items (+2, total 2)");
}

#[test]
/// 驗證 normal mode 按下單鍵 `U` (Shift+u) 會清除目前 pane 標記。
/// 保護目的：避免全選後無法以單鍵直覺清空標記。
fn app_capital_u_clears_all_visible_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("mark-all").expect("mark-all command");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT))
        .expect("shift+u");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
    assert_eq!(app.status, "cleared 2 marks");
}

#[test]
/// 驗證 `:unmark-all` 命令能把目前 pane 的所有標記全部清除。
/// 保護目的：避免命令列與 cheatsheet 執行 unmark-all 時狀態不如預期。
fn app_unmark_all_command_clears_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("mark-all").expect("mark-all");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);

    app.execute_command("unmark-all")
        .expect("unmark-all command");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
    assert_eq!(app.status, "cleared 2 marks");
}

#[test]
/// 驗證建立新檔案與刪除檔案後，快取會同步更新，重新進入目錄不會讀到陳舊快取。
/// 保護目的：防止使用者在目錄操作後離開再進入時，出現已刪除檔案回魂或新檔案消失的現象。
fn file_creation_and_deletion_updates_cache_synchronously() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 建立檔案
    app.create_entry_from_command("new_item.txt")
        .expect("create file");

    let cached = app
        .directory_entry_cache
        .get(dir.path())
        .expect("must have cache");
    assert!(
        cached.iter().any(|entry| entry.name == "new_item.txt"),
        "快取必須包含剛建立的檔案"
    );

    // 刪除檔案
    app.start_delete_confirmation(true);
    app.confirm_delete(1, "new_item.txt", true)
        .expect("delete file");
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let cached_after = app
        .directory_entry_cache
        .get(dir.path())
        .expect("must have cache after delete");
    assert!(
        !cached_after
            .iter()
            .any(|entry| entry.name == "new_item.txt"),
        "快取中不能殘留已刪除的檔案"
    );
}

#[test]
/// 驗證檔案系統 watcher 觸發重新整理時，快取會同步更新為磁碟最新狀態。
/// 保護目的：確保外部編輯器或 Git 操作產生變更後，快取與畫面保持一致。
fn watcher_reload_synchronizes_cache() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    fs::write(dir.path().join("external.txt"), "external content").expect("external write");
    let mut set = BTreeSet::new();
    set.insert(dir.path().to_path_buf());

    app.reload_watched_directories(&set)
        .expect("reload watched");

    let cached = app
        .directory_entry_cache
        .get(dir.path())
        .expect("must have cache");
    assert!(
        cached.iter().any(|entry| entry.name == "external.txt"),
        "watcher 刷新後快取必須包含外部新增的檔案"
    );
}

#[test]
/// 驗證 watcher 觸發重新整理時，不會要求全螢幕 terminal.clear()，由 ratatui diff 機制平滑繪製避免閃爍。
fn watcher_reload_does_not_request_full_terminal_clear() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    fs::write(dir.path().join("external.txt"), "content").expect("write");
    let mut set = BTreeSet::new();
    set.insert(dir.path().to_path_buf());

    app.reload_watched_directories(&set).expect("reload");

    assert!(
        !app.take_full_redraw_request(),
        "watcher 刷新不應請求 full_redraw（避免觸發 terminal.clear 產生黑畫面閃爍）"
    );
}

#[test]
fn diff_command_requires_two_panels() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    // 只有 1 個 panel 時執行 :diff
    app.execute_command("diff").expect("diff cmd");
    assert!(app.status.contains("diff requires at least 2 open panels"));
    assert!(app.pending_action.is_none());
}

#[test]
fn diff_command_opens_and_navigates_matrix() {
    let dir1 = tempdir().expect("dir1");
    let dir2 = tempdir().expect("dir2");
    let dir3 = tempdir().expect("dir3");

    fs::write(dir1.path().join("a.txt"), b"aaa").expect("write a");
    fs::write(dir2.path().join("a.txt"), b"aaa").expect("write a");
    fs::write(dir3.path().join("a.txt"), b"different").expect("write a");

    fs::write(dir1.path().join("only1.txt"), b"111").expect("write 1");
    fs::write(dir2.path().join("only2.txt"), b"222").expect("write 2");

    let mut app = App::new(dir1.path().to_path_buf(), default_loaded_config()).expect("app");
    // 分割出 panel 2 與 panel 3
    app.split_current(SplitDirection::Vertical)
        .expect("split 1");
    app.change_directory_from_command(&dir2.path().to_string_lossy())
        .expect("goto dir2");

    app.split_current(SplitDirection::Vertical)
        .expect("split 2");
    app.change_directory_from_command(&dir3.path().to_string_lossy())
        .expect("goto dir3");

    // 執行 :diff 比對全部 3 個 Panel
    app.execute_command("diff").expect("execute diff");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::DiffMatrix(_))
    ));

    // 輪詢等待背景 diff 完成
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.poll_background_tasks();
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action
            && !state.loading
        {
            break;
        }
    }

    // 測試按鍵導航
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("down");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert_eq!(state.selected_index, 1);
    }

    // 測試篩選切換 (f)
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("filter cycle");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert_eq!(
            state.filter_mode,
            crate::file_manager::diff::DiffFilterMode::DiffOnly
        );
    }

    // 測試 gitignore 切換 (i)
    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("toggle gitignore");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert!(!state.git_ignore);
    }
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.poll_background_tasks();
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action
            && !state.loading
        {
            break;
        }
    }

    // 測試隱藏檔切換 (.)
    app.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
        .expect("toggle hidden");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert!(!state.include_hidden);
    }
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.poll_background_tasks();
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action
            && !state.loading
        {
            break;
        }
    }

    // 測試搜尋 (/)
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("search start");
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .expect("type o");
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("type n");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("confirm search");

    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert!(!state.search_active);
        assert_eq!(state.search_query, "on");
        assert_eq!(state.filtered_indices.len(), 2); // only1.txt, only2.txt
    }

    // 測試退出比對 (q)
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("quit diff");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "diff matrix closed");

    // 測試快速指令別名 :d 1 2
    app.execute_command("d 1 2").expect("execute d 1 2");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::DiffMatrix(_))
    ));
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert_eq!(state.panel_ids, vec![1, 2]);
    }

    // 退出
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert!(app.pending_action.is_none());

    // 測試快捷鍵 wd (WindowPicker -> d)
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("w");
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::DiffMatrix(_))
    ));

    // 退出
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert!(app.pending_action.is_none());

    // 測試快捷鍵 wD (WindowPicker -> D: prefilled :diff )
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("w");
    app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("D");
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "diff ");
    app.command_mode = false;
    app.command_buffer.clear();

    // 測試快捷鍵 wr (WindowPicker -> r 進入連續尺寸調整模式)
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("w");
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("r");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::WindowResize { .. })
    ));

    // 在 Resize 模式下按 l 調整寬度，模式維持不變
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("l");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::WindowResize { .. })
    ));

    // 在 Resize 模式下按 = 平衡所有視窗，模式維持不變
    app.handle_key(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE))
        .expect("=");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::WindowResize { .. })
    ));

    // 按 Esc 退出連續調整模式
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert!(app.pending_action.is_none());

    // 測試快捷鍵 w= (WindowPicker -> = 直接重設均等)
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("w");
    app.handle_key(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE))
        .expect("=");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "equalized all panels");

    // 測試指令 :resize-mode
    app.execute_command("resize-mode").expect("resize-mode");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::WindowResize { .. })
    ));
    app.pending_action = None;

    // 測試指令 :equal
    app.execute_command("equal").expect("equal");
    assert_eq!(app.status, "equalized all panels");
}

#[test]
/// 驗證包含唯讀檔案、多層子目錄及模擬 macOS .DS_Store 隱藏檔時，remove_dir_all_parallel_with_progress 能徹底移除整個目錄結構。
fn test_remove_dir_all_parallel_removes_readonly_and_hidden_entries() {
    let dir = tempdir().expect("tempdir");
    let target_dir = dir.path().join("target_folder");
    fs::create_dir_all(&target_dir).expect("create target");

    let sub_dir = target_dir.join("nested");
    fs::create_dir_all(&sub_dir).expect("create sub");

    let ds_store = target_dir.join(".DS_Store");
    fs::write(&ds_store, "mock ds_store").expect("write ds_store");

    let normal_file = target_dir.join("file.txt");
    fs::write(&normal_file, "normal").expect("write normal");

    let readonly_file = sub_dir.join("readonly.txt");
    fs::write(&readonly_file, "readonly content").expect("write readonly");
    let mut perms = fs::metadata(&readonly_file).expect("meta").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&readonly_file, perms).expect("set readonly");

    let mut progress_bytes = 0u64;
    let res = remove_dir_all_parallel_with_progress(&target_dir, &mut |bytes| {
        progress_bytes += bytes;
    });

    assert!(res.is_ok(), "delete failed: {:?}", res);
    assert!(
        !target_dir.exists(),
        "target_dir should be completely removed"
    );
    assert!(!sub_dir.exists(), "sub_dir should be completely removed");
    assert!(!ds_store.exists(), ".DS_Store should be removed");
    assert!(!readonly_file.exists(), "readonly file should be removed");
}

#[test]
/// 驗證在空目錄上執行 remove_dir_all_parallel_with_progress 與 remove_dir_with_retry 能正確刪除且回報 Ok。
fn test_remove_dir_empty_directory() {
    let dir = tempdir().expect("tempdir");
    let empty_dir = dir.path().join("empty_folder");
    fs::create_dir(&empty_dir).expect("create empty");

    let res = remove_dir_with_retry(&empty_dir);
    assert!(res.is_ok());
    assert!(!empty_dir.exists());

    let empty_dir2 = dir.path().join("empty_folder_2");
    fs::create_dir(&empty_dir2).expect("create empty 2");
    let res2 = remove_dir_all_parallel_with_progress(&empty_dir2, &mut |_| {});
    assert!(res2.is_ok());
    assert!(!empty_dir2.exists());
}
