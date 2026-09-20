use super::*;

#[test]
/// 驗證 `:tasks` 會打開目前 pane 的任務面板，且空清單時狀態訊息正確。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tasks_command_opens_task_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.execute_command("tasks").expect("open tasks");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TaskPanel {
            pane_id: 1,
            selected: 0,
            ..
        })
    ));
    assert_eq!(app.status, "tasks: empty");
}

#[test]
/// 驗證一般外部開啟會建立 task，並在主事件迴圈回報成功後標記完成。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_open_task_is_created_and_can_finish() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("queue open");

    assert_eq!(app.task_log.len(), 1);
    let task = app.task_log.last().expect("task");
    assert_eq!(task.kind, "open");
    assert_eq!(task.state, TaskState::Running);

    let queued = app.take_pending_launch().expect("queued launch");
    let task_id = queued.task_id;
    app.finish_launch_task(task_id, Ok(()));

    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.state, TaskState::Done);
    assert_eq!(task.detail, "completed");
    assert!(task.finished_at_unix_ms.is_some());
}

#[test]
/// 驗證 `z` 打開 fzf jump 時會建立 task，取消後也會正確標成 cancelled。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_fzf_jump_task_is_created_and_cancelled() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_fzf_jump();

    assert_eq!(app.task_log.len(), 1);
    let request = app.take_pending_fzf_jump().expect("fzf request");
    let task_id = request.task_id;
    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.kind, "jump");
    assert_eq!(task.state, TaskState::Running);

    app.apply_fzf_jump_selection(request, None);

    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.state, TaskState::Cancelled);
    assert_eq!(task.detail, "fzf cancelled");
}

#[test]
/// 驗證 task 面板支援 `f` 搜尋，且來源／目的地也能用來篩選任務。
/// 保護目的：檔案操作完成後，使用者常只記得 share 或目錄名稱；若搜尋只比對標題，
/// 新增的診斷位置雖然看得到，卻無法在長期歷史中快速找回。
fn app_task_panel_supports_filtering() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.task_log.push(TaskRecord {
        id: 1,
        pane_id: 1,
        kind: String::from("search"),
        title: String::from("alpha task"),
        detail: String::from("first detail"),
        source_locations: Vec::new(),
        destination_location: None,
        state: TaskState::Done,
        progress_percent: None,
        completed_bytes: None,
        total_bytes: None,
        started_at_unix_ms: 0,
        finished_at_unix_ms: Some(1),
    });
    app.task_log.push(TaskRecord {
        id: 2,
        pane_id: 1,
        kind: String::from("search"),
        title: String::from("beta task"),
        detail: String::from("second detail"),
        source_locations: vec![String::from("/source/report.txt")],
        destination_location: Some(String::from("/share/beta-destination")),
        state: TaskState::Running,
        progress_percent: None,
        completed_bytes: None,
        total_bytes: None,
        started_at_unix_ms: 2,
        finished_at_unix_ms: None,
    });

    app.open_task_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start task filter");
    for ch in "destination".chars() {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type task query");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock task filter");

    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel {
            selected, search, ..
        }) => {
            assert_eq!(*selected, 0);
            assert_eq!(search.buffer, "destination");
            assert!(!search.editing);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
    assert_eq!(
        app.status,
        "tasks: 1/1 (d delete, D clear all, v visual, Space mark, x/c cancel, f search)"
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("show filtered task detail");
    assert_eq!(app.status, "task 2 [search] second detail");
}

#[test]
/// 驗證按下 `T` 會直接打開目前 pane 的 task 面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_t_opens_task_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .expect("open tasks with T");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TaskPanel {
            pane_id: 1,
            selected: 0,
            ..
        })
    ));
    assert_eq!(app.status, "tasks: empty");
}

#[test]
/// 驗證 PaneFM 正常關閉時會把尚未完成的 task 標成 `Interrupted` 並保存到檔案。
///
/// 保護目的：大型本機或 SMB copy 可能執行超過半小時；若使用者關閉程式，舊版
/// 記憶體 task 會完全消失。此測試確保關閉後仍可追查開始時間、進度與中斷原因，
/// 且不會錯誤顯示為仍在執行。
fn app_shutdown_persists_running_tasks_as_interrupted() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "paste",
        String::from("copy large.zip"),
        String::from("destination: share"),
        vec![String::from("/source/large.zip")],
        Some(String::from("/destination/share")),
    );
    app.update_task_progress(task_id, 42, 100);

    app.prepare_for_shutdown().expect("persist shutdown");
    let tasks = super::load_task_history(&app.task_history_path).expect("load history");
    let task = tasks.last().expect("persisted task");

    assert_eq!(task.state, TaskState::Interrupted);
    assert_eq!(task.progress_percent, Some(42));
    assert_eq!(task.completed_bytes, Some(42));
    assert_eq!(task.total_bytes, Some(100));
    assert!(task.finished_at_unix_ms.is_some());
    assert!(task.detail.contains("interrupted when PaneFM closed"));
}

#[test]
/// 驗證啟動時會載入上次 task 歷史，並修正來不及正常關閉的 `Running` 紀錄。
///
/// 保護目的：使用者可能直接關閉 terminal 或系統終止程序，導致關閉 hook 沒機會
/// 執行。下次啟動必須把磁碟上最後一次 RUNNING 快照轉為 `Interrupted`，不能讓 task
/// 面板永久顯示不存在的工作，也不能自動重複覆寫目的檔案。
fn app_startup_recovers_unclean_running_task_history() {
    let dir = tempdir().expect("tempdir");
    let history_path = super::task_history_file_path(dir.path(), None);
    super::save_task_history(
        &history_path,
        &[TaskRecord {
            id: 12,
            pane_id: 8,
            kind: String::from("paste"),
            title: String::from("copy build"),
            detail: String::from("destination: share"),
            source_locations: vec![String::from("/source/build")],
            destination_location: Some(String::from("/destination/share")),
            state: TaskState::Running,
            progress_percent: Some(37),
            completed_bytes: Some(37),
            total_bytes: Some(100),
            started_at_unix_ms: 1_700_000_000_000,
            finished_at_unix_ms: None,
        }],
    )
    .expect("seed history");

    let app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task = app.task_log.last().expect("recovered task");

    assert_eq!(task.state, TaskState::Interrupted);
    assert_eq!(
        task.pane_id, 1,
        "舊 session 的 task 必須出現在目前可見 panel"
    );
    assert_eq!(app.next_task_id, 13);
    assert!(app.status.contains("recovered 1 interrupted task"));
}

#[test]
/// 驗證背景進度採用最新 byte 估算，且 task 面板不再只顯示百分比。
///
/// 保護目的：目錄只走訪一次時總量會逐步增加，畫面必須能從早期估算校正成最新
/// 比例，否則前幾個檔案完成後可能長時間錯誤停在 99%。
fn task_progress_uses_latest_dynamic_estimate_and_is_visible_in_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "paste",
        String::from("copy large.zip"),
        String::from("destination: target"),
        vec![String::from("/source/large.zip")],
        Some(String::from("/destination/target")),
    );

    app.update_task_progress(task_id, 60, 100);
    app.update_task_progress(task_id, 20, 100);
    let lines = super::task_panel_lines(&app.task_log, &[]);

    assert_eq!(app.task_log[0].progress_percent, Some(20));
    assert_eq!(lines[0].state, "RUNNING");
    assert_eq!(app.task_log[0].completed_bytes, Some(20));
    assert_eq!(app.task_log[0].total_bytes, Some(100));
    assert_eq!(lines[0].progress, "20B / 100B");
    assert_eq!(lines[0].finished_at, "--:--:--");

    app.finish_task(task_id, TaskState::Done, String::from("completed"));
    assert_eq!(app.task_log[0].progress_percent, Some(100));
    assert_eq!(app.task_log[0].completed_bytes, Some(100));
    let lines = super::task_panel_lines(&app.task_log, &[]);
    assert_ne!(lines[0].finished_at, "--:--:--");
}

#[test]
/// 驗證 task byte 會依數量級切換單位，且不再輸出百分比。
///
/// 保護目的：大型本機與 SMB 傳輸即使百分比長時間不變，使用者仍要從 byte 數判斷
/// 工作是否前進；格式重構不能把資訊退回只有 `31%`。
fn task_progress_label_uses_compact_byte_units_without_percentage() {
    let mut task = TaskRecord {
        id: 1,
        pane_id: 1,
        kind: String::from("paste"),
        title: String::from("copy project"),
        detail: String::new(),
        source_locations: vec![String::from("/source/project")],
        destination_location: Some(String::from("/destination")),
        state: TaskState::Running,
        progress_percent: Some(31),
        completed_bytes: Some(25_589_858_714),
        total_bytes: Some(82_893_350_912),
        started_at_unix_ms: 0,
        finished_at_unix_ms: None,
    };

    assert_eq!(super::task_progress_label(&task), "23.8G / 77.2G");
    assert!(!super::task_progress_label(&task).contains('%'));
    task.completed_bytes = None;
    task.total_bytes = None;
    assert_eq!(super::task_progress_label(&task), "-");
}

#[test]
/// 驗證背景貼上刷新目的根目錄時，也會更新已經進入其子目錄的 panel。
///
/// 保護目的：使用者可能在大型 copy 尚未完成時進入新建立的 `target/`；舊流程只
/// 刷新目的父目錄，子目錄 panel 會一直保持空白直到整批結束。測試同時確認目的
/// 樹以外的 panel 不會被無關進度反覆 reload。
fn background_destination_refresh_updates_open_descendant_panels_only() {
    let dir = tempdir().expect("tempdir");
    let destination = dir.path().join("destination");
    let child = destination.join("target");
    let unrelated = dir.path().join("unrelated");
    fs::create_dir_all(&child).expect("destination child");
    fs::create_dir(&unrelated).expect("unrelated dir");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .insert(2, PaneState::new(child.clone()).expect("child panel"));
    app.panes.insert(
        3,
        PaneState::new(unrelated.clone()).expect("unrelated panel"),
    );
    fs::write(child.join("copied.txt"), b"visible").expect("copied file");
    fs::write(unrelated.join("not-reloaded.txt"), b"hidden").expect("unrelated file");

    app.reload_panes_in_tree(&destination)
        .expect("refresh destination tree");

    assert!(
        app.panes[&2]
            .entries
            .iter()
            .any(|entry| entry.name == "copied.txt")
    );
    assert!(
        app.panes[&3]
            .entries
            .iter()
            .all(|entry| entry.name != "not-reloaded.txt")
    );
}

#[test]
/// 驗證 worker 會保留原始 byte 變化，而不是只在整數百分比改變時回報。
///
/// 保護目的：若每個 1 MiB buffer 都排入 channel，數十 GB 傳輸會讓主執行緒忙於
/// 處理重複資料；時間節流由 worker 負責，這裡確保完全相同的快照不會重送。
fn progress_events_preserve_byte_changes_and_skip_exact_duplicates() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut last_progress = None;

    super::send_progress_if_changed(&sender, 7, 10, 1_000, &mut last_progress);
    super::send_progress_if_changed(&sender, 7, 10, 1_000, &mut last_progress);
    super::send_progress_if_changed(&sender, 7, 11, 1_000, &mut last_progress);

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(last_progress, Some((11, 1_000)));
    assert!(matches!(
        events.last(),
        Some(super::FileJobEvent::Progress {
            task_id: 7,
            completed_bytes: 11,
            total_bytes: 1_000,
        })
    ));
}

#[test]
/// 驗證單次走訪的動態總量增加時，task 會保留新的 byte 分母。
///
/// 保護目的：目錄 producer 會一邊發現檔案、一邊複製；若只允許百分比增加，前幾個
/// 小檔完成時若只保留百分比，後續增加總量會掩蓋真實進度。
fn progress_events_follow_dynamic_discovered_total() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut last_progress = None;

    super::send_progress_if_changed(&sender, 8, 90, 100, &mut last_progress);
    super::send_progress_if_changed(&sender, 8, 90, 1_000, &mut last_progress);

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(last_progress, Some((90, 1_000)));
    assert!(matches!(
        events.last(),
        Some(super::FileJobEvent::Progress {
            task_id: 8,
            completed_bytes: 90,
            total_bytes: 1_000,
        })
    ));
}

#[test]
/// 驗證 task 面板中的 `x` 可以取消目前正在進行的 search task。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_task_panel_x_cancels_running_search_task() {
    let dir = tempdir().expect("tempdir");
    let cancelled = Arc::new(AtomicBool::new(false));

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "search",
        String::from("content search: needle"),
        format!("root: {}", dir.path().display()),
        vec![dir.path().display().to_string()],
        None,
    );
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        mode: SearchMode::Content,
        buffer: String::from("needle"),
        editing: false,
        loading: true,
        searched: false,
        selected: 0,
        results: Vec::new(),
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
        task_id: Some(task_id),
    });
    app.active_global_search_task_id = Some(task_id);
    app.global_search_cancelled = Some(cancelled.clone());
    app.open_task_panel();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .expect("cancel task");

    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.state, TaskState::Cancelled);
    assert!(app.global_search.is_none());
    assert!(app.global_search_rx.is_none());
    assert!(app.global_search_cancelled.is_none());
    assert!(cancelled.load(Ordering::Relaxed));
    assert_eq!(app.status, format!("cancelled task {task_id}"));
}

#[test]
/// 驗證 task 面板中的 `X` 會取消目前 panel 內所有可取消的任務。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_task_panel_shift_x_cancels_all_running_tasks() {
    let dir = tempdir().expect("tempdir");
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "search",
        String::from("content search: needle"),
        format!("root: {}", dir.path().display()),
        vec![dir.path().display().to_string()],
        None,
    );
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        mode: SearchMode::Content,
        buffer: String::from("needle"),
        editing: false,
        loading: true,
        searched: false,
        selected: 0,
        results: Vec::new(),
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
        task_id: Some(task_id),
    });
    app.active_global_search_task_id = Some(task_id);
    app.global_search_cancelled = Some(cancelled.clone());
    app.open_task_panel();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("cancel all tasks");

    assert_eq!(
        app.task_log
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.state),
        Some(TaskState::Cancelled)
    );
    assert!(cancelled.load(Ordering::Relaxed));
    assert_eq!(app.status, "cancelled 1 tasks");
}

#[test]
/// 驗證當背景有檔案傳輸時，active_file_job_busy_paths 能正確識別忙碌目錄，避免 watcher 在傳輸期間反覆重刷。
fn active_file_job_busy_paths_identifies_busy_parent_directory() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let target_file = dir.path().join("archive.zip");

    let task_id = app.push_task(
        1,
        "paste",
        "copy file".into(),
        "dest".into(),
        vec![],
        Some("dest".into()),
    );
    app.active_file_job_busy_paths
        .insert(task_id, vec![target_file.clone()]);

    let is_busy = app.active_file_job_busy_paths.values().any(|busy_paths| {
        busy_paths.iter().any(|busy| {
            busy == dir.path() || busy.starts_with(dir.path()) || busy.parent() == Some(dir.path())
        })
    });
    assert!(
        is_busy,
        "傳輸目標所在目錄必須被識別為忙碌中，避免傳輸分塊寫入觸發反覆刷新"
    );
}

#[test]
/// 驗證大型目錄的 task badge 查詢只接觸 viewport，不會走訪完整檔案列表。
///
/// 保護目的：舊版 render 會對每個項目執行 `canonicalize()`；實際 `deps` 超過六萬筆
/// 時，每次 j/k 都被同步檔案 I/O 阻塞約半秒。此測試建立 200 筆並固定 viewport
/// 為 18 列，確保未來重構不能再次把完整列表送進 badge 查詢。
fn task_badge_paths_are_limited_to_visible_viewport() {
    let dir = tempdir().expect("tempdir");
    for index in 0..200 {
        fs::write(dir.path().join(format!("file_{index:03}.txt")), b"x").expect("write");
    }
    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.selected = 150;

    let paths = visible_job_badge_paths(&pane, 18);

    assert_eq!(paths.len(), 18);
    assert!(paths.iter().all(|path| path.starts_with(dir.path())));
}

#[test]
/// 驗證永久刪除大型目錄時使用背景 worker 執行，主執行緒不卡死且完成後正確移除。
fn background_delete_removes_directory_and_reports_done() {
    let dir = tempdir().expect("tempdir");
    let to_delete = dir.path().join("large_dir");
    fs::create_dir_all(to_delete.join("nested")).expect("create nested");
    fs::write(to_delete.join("nested/a.bin"), "payload").expect("write payload");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.panes.get_mut(&1).unwrap().select_path(&to_delete);
    app.start_delete_confirmation(true);
    app.confirm_delete(1, "large_dir", true)
        .expect("confirm delete");

    assert!(
        !app.file_job_receivers.is_empty(),
        "刪除工作必須進入背景佇列"
    );
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(!to_delete.exists(), "背景刪除完成後目錄必須已從磁碟移除");
    assert!(app.status.contains("deleted permanently"));
    let task = app.task_log.last().expect("background delete task");
    assert_eq!(task.title, "delete 1 item(s)");
    assert_eq!(task.source_locations, vec![to_delete.display().to_string()]);
    assert_eq!(task.destination_location, None);
}

#[test]
/// 驗證背景刪除任務在執行期間與完成後，Task 紀錄會包含實際刪除的 byte 進度資訊，而非未知的 `-`。
fn background_delete_reports_byte_progress_in_task_record() {
    let dir = tempdir().expect("tempdir");
    let to_delete = dir.path().join("data_folder");
    fs::create_dir_all(&to_delete).expect("create dir");
    fs::write(to_delete.join("file1.dat"), vec![0u8; 1024 * 10]).expect("write file1");
    fs::write(to_delete.join("file2.dat"), vec![0u8; 1024 * 20]).expect("write file2");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.panes.get_mut(&1).unwrap().select_path(&to_delete);
    app.start_delete_confirmation(true);
    app.confirm_delete(1, "data_folder", true)
        .expect("confirm delete");

    // 輪詢等待背景刪除工作完成
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let task = app.task_log.last().expect("task record");
    assert_eq!(task.state, TaskState::Done);
    assert!(
        task.completed_bytes.unwrap_or(0) >= 1024 * 30,
        "必須記錄實際刪除的 byte 數: {:?}",
        task.completed_bytes
    );
    assert_eq!(task.completed_bytes, task.total_bytes);
    let progress_label = task_progress_label(task);
    assert_ne!(progress_label, "-", "進度標籤不可為未知的 `-`");
    assert!(
        progress_label.contains("30K")
            || progress_label.contains("K")
            || progress_label.contains("M")
    );
}

#[test]
/// 驗證任務面板可以使用 v 進行 visual 選取並使用 d 批次刪除任務。
fn task_panel_supports_visual_selection_and_batch_delete_with_v_and_d() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let t1 = app.push_task(
        1,
        "copy",
        "copy a.txt".into(),
        "running".into(),
        vec!["/a.txt".into()],
        None,
    );
    let t2 = app.push_task(
        1,
        "move",
        "move b.txt".into(),
        "running".into(),
        vec!["/b.txt".into()],
        None,
    );
    let t3 = app.push_task(
        1,
        "delete",
        "delete c.txt".into(),
        "running".into(),
        vec!["/c.txt".into()],
        None,
    );

    // 打開任務面板
    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .expect("open task panel");

    // 目前有 3 筆任務，最新排在最上面 (t3, t2, t1)
    assert_eq!(app.tasks_for_pane(1).len(), 3);

    // 按下 v 開啟 visual 選取模式
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("v start");

    // 向下移動一格 (j)
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("j down");

    // 再次按下 v 提交 visual 選取 (選取了 t3 與 t2)
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("v commit");

    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert_eq!(marked_ids.len(), 2);
            assert!(marked_ids.contains(&t3));
            assert!(marked_ids.contains(&t2));
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 按下 d 批次刪除已選取的任務
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d delete marked");

    assert_eq!(app.tasks_for_pane(1).len(), 1);
    assert_eq!(app.tasks_for_pane(1)[0].id, t1);
    assert_eq!(app.status, "tasks: deleted 2 tasks");
}

#[test]
/// 驗證任務面板可以使用 Space 標記個別任務、使用 a 全選、使用 d/D 刪除與清空。
fn task_panel_supports_space_mark_all_and_clear_all_with_shifted_d() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let t1 = app.push_task(
        1,
        "copy",
        "copy a.txt".into(),
        "running".into(),
        vec!["/a.txt".into()],
        None,
    );
    let t2 = app.push_task(
        1,
        "move",
        "move b.txt".into(),
        "running".into(),
        vec!["/b.txt".into()],
        None,
    );

    // 打開任務面板
    app.open_task_panel();

    // 按 Space 標記第一筆任務 (t2)
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .expect("space mark");

    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert_eq!(marked_ids, &vec![t2]);
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 按 d 刪除被標記的 t2
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d delete");
    assert_eq!(app.tasks_for_pane(1).len(), 1);
    assert_eq!(app.tasks_for_pane(1)[0].id, t1);

    // 按 a 標記全部剩餘任務
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("a mark all");
    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert_eq!(marked_ids, &vec![t1]);
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 按 a 取消全部標記
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("a clear all marks");
    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert!(marked_ids.is_empty());
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 在沒有標記狀態下按 d，直接刪除游標所在任務
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d delete single");
    assert_eq!(app.tasks_for_pane(1).len(), 0);

    // 重新新增任務並測試 Shift+D (清空所有任務)
    app.push_task(1, "copy", "task 1".into(), "running".into(), vec![], None);
    app.push_task(1, "copy", "task 2".into(), "running".into(), vec![], None);
    assert_eq!(app.tasks_for_pane(1).len(), 2);

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("D clear all");
    assert_eq!(app.tasks_for_pane(1).len(), 0);
    assert_eq!(app.status, "tasks: cleared 2 tasks");
}
