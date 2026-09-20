use super::*;

#[test]
/// 驗證可針對指定路徑建立 preview，並套用搜尋高亮，供搜尋列表下方預覽使用。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_search_preview_for_path_supports_search_highlight() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("notes.txt");
    fs::write(&path, "alpha\nbeta target\ngamma\n").expect("notes");

    let preview = PaneState::search_preview_for_entry(
        &GlobalSearchEntry {
            path: path.clone(),
            relative_path: String::from("notes.txt"),
            is_dir: false,
            match_line_number: Some(2),
            match_column: Some(6),
            match_preview: Some(String::from("beta target")),
        },
        8,
        "target",
        None,
        None,
        false,
        Theme::default(),
    );
    let preview_text = preview
        .lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(preview_text.iter().any(|line| line == "  2 beta target"));
    assert!(preview.lines.iter().any(|line| {
        line.spans
            .iter()
            .any(|span| span.content.as_ref() == "target")
    }));

    let title = PaneState::preview_title_for_path(&path, false, Some("target"));
    assert_eq!(title, "Preview: notes.txt  [/target]");
}

#[test]
/// 驗證一般 preview 不會再顯示舊的資訊區，搜尋也只會針對檔案內容運作。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_preview_search_ignores_metadata_lines() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("test copy.md"), "this is body text\n").expect("notes");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(6);
    pane.set_preview_search_query("t");
    pane.preview_scroll = 0;

    let preview = pane.preview_lines(12, Theme::default());
    let preview_text = preview
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(!preview_text.iter().any(|line| line.contains("Information")));
    assert!(!preview_text.iter().any(|line| line.contains("path: ")));
    assert!(preview.iter().any(|line| {
        line.spans.iter().any(|span| {
            span.content.as_ref() == "t" && span.style.fg == Some(Theme::default().preview_match_fg)
        })
    }));
}

#[test]
/// 驗證搜尋 preview 會讓所有命中維持紅字，只有目前焦點命中帶黃色背景。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_search_preview_marks_current_match_line() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("notes.txt");
    fs::write(&path, "alpha\nbeta target\ngamma target\n").expect("notes");

    let theme = Theme::default();
    let preview = PaneState::search_preview_for_entry(
        &GlobalSearchEntry {
            path: path.clone(),
            relative_path: String::from("notes.txt"),
            is_dir: false,
            match_line_number: Some(2),
            match_column: Some(6),
            match_preview: Some(String::from("beta target")),
        },
        10,
        "target",
        Some(0),
        Some(3),
        false,
        theme,
    );

    let target_spans = preview
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .filter(|span| span.content.as_ref() == "target")
        .collect::<Vec<_>>();

    assert_eq!(target_spans.len(), 2);
    assert!(target_spans.iter().any(|span| {
        span.style.bg == Some(theme.preview_match_bg)
            && span.style.fg == Some(theme.preview_match_fg)
    }));
    assert!(target_spans.iter().any(|span| {
        span.style.bg != Some(theme.preview_match_bg)
            && span.style.fg == Some(theme.preview_match_fg)
    }));
    assert!(preview.lines.iter().any(|line| {
        line.spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    }));
}

#[test]
/// 驗證一般檔案 preview 會依據 preview_cursor 將游標所在行標示高亮與粗體行號。
/// 保護目的：確保 preview mode 游標移動時，使用者能明確識別目前聚焦在檔案哪一行。
fn pane_state_preview_lines_highlights_cursor_line() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("source.rs");
    fs::write(
        &path,
        "fn main() {\n    let a = 10;\n    let b = 20;\n    println!(\"{}\", a + b);\n}\n",
    )
    .expect("write source.rs");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(5);
    pane.set_preview_viewport_size(60, 5);

    let theme = Theme::default();

    // 1. 預設游標在第 0 行（行號 1）
    let lines = pane.preview_lines(5, theme);
    assert_eq!(lines.len(), 5);

    // 第 0 行應有 preview_current_line_bg，且行號 span 應為 accent 色與 BOLD 樣式
    assert!(
        lines[0]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    );
    let first_line_num_span = &lines[0].spans[0];
    assert_eq!(first_line_num_span.style.fg, Some(theme.accent));
    assert!(
        first_line_num_span
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );

    // 第 1 行不應有 preview_current_line_bg，行號亦非 accent
    assert!(
        !lines[1]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    );
    assert_ne!(lines[1].spans[0].style.fg, Some(theme.accent));

    // 2. 游標移動到第 2 行（行號 3）
    pane.move_preview_cursor_to_line(3);
    assert_eq!(pane.preview_cursor, 2);

    let lines_after = pane.preview_lines(5, theme);
    // 第 0 行不再是焦點
    assert!(
        !lines_after[0]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    );
    // 第 2 行（行號 3）成為焦點
    assert!(
        lines_after[2]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    );
    let target_line_num_span = &lines_after[2].spans[0];
    assert_eq!(target_line_num_span.style.fg, Some(theme.accent));
    assert!(
        target_line_num_span
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
/// 驗證搜尋 preview 即使遇到大檔案，也會顯示命中片段而不是只顯示 skipped 訊息。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_search_preview_for_large_file_shows_match_snippet() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("large.txt");
    let mut content = String::new();
    for _ in 0..9000 {
        content.push_str("padding padding padding padding\n");
    }
    content.push_str("needle appears here\n");
    fs::write(&path, content).expect("large");

    let preview = PaneState::search_preview_for_entry(
        &GlobalSearchEntry {
            path: path.clone(),
            relative_path: String::from("large.txt"),
            is_dir: false,
            match_line_number: Some(9001),
            match_column: Some(1),
            match_preview: Some(String::from("needle appears here")),
        },
        8,
        "needle",
        None,
        None,
        false,
        Theme::default(),
    );

    let text = preview
        .lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    assert!(text.iter().any(|line| line.contains("needle appears here")));
    assert!(
        !text
            .iter()
            .any(|line| line.contains("preview skipped for files larger than 128 KiB"))
    );
}

#[test]
/// 驗證預覽模式下使用 `/` 搜尋過濾關鍵字時，所有未命中與命中行的語法高亮色彩皆完整保留，不會退化為黑白單色。
/// 保護目的：確保搜尋標記覆蓋在語法樹之上，而非抹除原有的語法著色與行號色彩。
fn pane_state_preview_search_preserves_syntax_highlighting_colors() {
    let dir = tempdir().expect("tempdir");
    let test_file = dir.path().join("main.rs");
    let code = "fn main() {\n    let greeting = \"hello world\";\n}\n";
    fs::write(&test_file, code).expect("write rust file");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(10);
    pane.set_preview_viewport_size(80, 10);

    // 1. 無搜尋時的初始預覽，確認有彩色 span
    let initial_lines = pane.preview_lines(10, Theme::default());
    assert_eq!(initial_lines.len(), 3);
    let line1_has_kw = initial_lines[0]
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("fn") && s.style.fg.is_some());
    assert!(line1_has_kw, "初始狀態 fn 必須帶有語法色彩");

    // 2. 模擬使用者按下 `/` 並輸入 "world"
    pane.set_preview_search_query("world");
    let searched_lines = pane.preview_lines(10, Theme::default());
    assert_eq!(searched_lines.len(), 3);

    // 第 1 行並未命中 "world"，但關鍵字 "fn" 必須依然維持語法高亮色彩！
    let line1_searched_has_kw = searched_lines[0]
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("fn") && s.style.fg.is_some());
    assert!(
        line1_searched_has_kw,
        "使用 / 搜尋時，未命中行（第 1 行 fn）的語法色彩不可消失"
    );

    // 第 2 行命中 "world"：
    // - 搜尋關鍵字 "world" 應帶有 match 高亮樣式
    // - 前方的 "let" 關鍵字必須依然保持語法色彩！
    let line2 = &searched_lines[1];
    let line2_has_let_kw = line2
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("let") && s.style.fg.is_some());
    assert!(
        line2_has_let_kw,
        "使用 / 搜尋時，命中行（第 2 行 let）的語法色彩不可消失"
    );

    let line2_has_match = line2.spans.iter().any(|s| {
        s.content.as_ref() == "world"
            && (s.style.bg.is_some() || s.style.fg == Some(Theme::default().preview_match_fg))
    });
    assert!(line2_has_match, "搜尋關鍵字 world 必須具備命中高亮樣式");

    // 3. 取消搜尋後，色彩依然正常
    pane.clear_preview_search();
    let cleared_lines = pane.preview_lines(10, Theme::default());
    let cleared_has_kw = cleared_lines[0]
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("fn") && s.style.fg.is_some());
    assert!(cleared_has_kw, "取消搜尋後語法色彩依然維持");
}

#[test]
/// 驗證 `PaneState::toggle_preview_diff_mode` 能正確在 Diff 模式與全文模式間切換，
/// 且在預覽未開啟時會自動開啟預覽並設為 Diff 模式。
fn pane_state_toggle_preview_diff_mode_states() {
    let dir = tempdir().expect("tempdir");
    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

    assert!(!pane.preview_open);
    assert!(!pane.preview_diff_mode);

    // 1. 預覽未開時按 Ctrl+d：直接開啟預覽並設為 diff 模式
    let is_diff = pane.toggle_preview_diff_mode();
    assert!(is_diff);
    assert!(pane.preview_open);
    assert!(pane.preview_diff_mode);

    // 2. 預覽已在 diff 模式時按 Ctrl+d：保持預覽開啟，切換回全文模式
    let is_diff = pane.toggle_preview_diff_mode();
    assert!(!is_diff);
    assert!(pane.preview_open);
    assert!(!pane.preview_diff_mode);

    // 3. 預覽在全文模式時按 Ctrl+d：切回 diff 模式
    let is_diff = pane.toggle_preview_diff_mode();
    assert!(is_diff);
    assert!(pane.preview_open);
    assert!(pane.preview_diff_mode);

    // 4. 按 Tab 關閉預覽：diff 模式重設為 false
    let is_open = pane.toggle_preview_open();
    assert!(!is_open);
    assert!(!pane.preview_diff_mode);

    // 5. 按 Tab 重新開啟預覽：預設為一般全文預覽
    let is_open = pane.toggle_preview_open();
    assert!(is_open);
    assert!(!pane.preview_diff_mode);
}

#[test]
/// 驗證 VCS Diff 模式下，游標所在行會被正確套用 `preview_current_line_bg` 高亮，
/// 且移動游標時高亮位置能同步更新。
fn pane_diff_mode_applies_cursor_highlight_to_active_line() {
    use ratatui::text::Line;
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("main.rs");
    fs::write(&file_path, "fn main() {}\n").expect("write");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let theme = Theme::default_theme();

    // 模擬已載入快取的 Diff 行
    let sample_diff_lines = vec![
        Line::from("--- a/main.rs"),
        Line::from("+++ b/main.rs"),
        Line::from("+// added line"),
    ];
    let entry = pane.selected_entry().expect("entry").clone();
    if let Ok(mut guard) = pane.preview_diff_cache.lock() {
        *guard = Some((entry.path.clone(), Some(entry.modified), sample_diff_lines));
    }

    pane.preview_open = true;
    pane.preview_diff_mode = true;
    pane.preview_cursor = 0;
    pane.preview_scroll = 0;

    // 1. 游標在第 0 行時，第 0 行應有 preview_current_line_bg，第 1 行沒有
    let rendered = pane.preview_lines(10, theme);
    assert!(
        rendered[0]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.preview_current_line_bg)),
        "第 0 行必須帶有游標高亮背景色"
    );
    assert!(
        !rendered[1]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.preview_current_line_bg)),
        "第 1 行不應有游標高亮"
    );

    // 2. 移動游標至第 1 行
    pane.move_preview_cursor_down(1);
    assert_eq!(pane.preview_cursor, 1);

    let rendered_after_move = pane.preview_lines(10, theme);
    assert!(
        !rendered_after_move[0]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.preview_current_line_bg)),
        "第 0 行已非游標行，不應有高亮"
    );
    assert!(
        rendered_after_move[1]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.preview_current_line_bg)),
        "第 1 行現在是游標行，必須帶有游標高亮背景色"
    );

    // 3. 驗證 preview_focused 時標題包含 [preview]
    pane.set_preview_focused(true);
    let title = pane.preview_title_for_entry(&entry);
    assert!(title.contains("[preview]"));
}

#[test]
/// 驗證 VCS Diff 模式下搜尋 `/`：
/// 1. 不會跳回一般全文預覽，依然保留 diff 內容。
/// 2. 能命中 diff 行中的字串（如 `+`、`-`、context 行中的關鍵字）。
/// 3. 能正確計算命中數、套用搜尋顏色高亮，且標題顯示 `[/{query}]`。
/// 4. 支援 `n`/`N` (jump_to_next_preview_match) 切換命中位置。
fn pane_diff_mode_search_highlights_and_retains_diff_content() {
    use ratatui::text::Line;
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("main.rs");
    // 實體檔案內容為純 stub，若跳回全文預覽會顯示 fn main()
    fs::write(&file_path, "fn main() {}\n").expect("write");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_size(80, 20);
    let theme = Theme::default_theme();

    // 模擬已載入快取的 Diff 行
    let sample_diff_lines = vec![
        Line::from("--- a/main.rs"),
        Line::from("+++ b/main.rs"),
        Line::from("@@ -1,1 +1,3 @@"),
        Line::from(" fn main() {"),
        Line::from("+    pub vcs_token: String,"),
        Line::from("-    pub vcs_old: String,"),
        Line::from(" }"),
    ];
    let entry = pane.selected_entry().expect("entry").clone();
    if let Ok(mut guard) = pane.preview_diff_cache.lock() {
        *guard = Some((entry.path.clone(), Some(entry.modified), sample_diff_lines));
    }

    pane.preview_open = true;
    pane.preview_diff_mode = true;
    pane.preview_cursor = 0;
    pane.preview_scroll = 0;

    // 1. 設定搜尋關鍵字 "vcs_token"
    pane.set_preview_search_query("vcs_token");

    // 驗證 match count 為 1
    assert_eq!(pane.preview_match_count(), 1);

    // 取得預覽行：必須依然為 diff 模式內容，絕對不可跳回 full preview
    let rendered = pane.preview_lines(10, theme);
    assert_eq!(rendered.len(), 7);
    assert_eq!(rendered[0].to_string(), "--- a/main.rs");
    assert!(
        rendered[4]
            .to_string()
            .contains("+    pub vcs_token: String,")
    );

    // 驗證命中文字帶有 preview_match_fg 高亮樣式
    let match_line = &rendered[4];
    assert!(
        match_line
            .spans
            .iter()
            .any(|s| s.content == "vcs_token" && s.style.fg == Some(theme.preview_match_fg)),
        "命中行中的 vcs_token 必須套用 preview_match_fg 高亮"
    );

    // 驗證標題包含 [diff] 與 [/{query}]
    let title = pane.preview_title_for_entry(&entry);
    assert!(title.contains("Preview [diff]:"));
    assert!(title.contains("[/vcs_token]"));

    // 2. 搜尋多次出現的詞彙 "pub"
    pane.set_preview_search_query("pub");
    assert_eq!(pane.preview_match_count(), 2);
    assert_eq!(pane.preview_current_match, Some(0));

    // 切換到下一個命中 (N)
    assert!(pane.jump_to_next_preview_match());
    assert_eq!(pane.preview_current_match, Some(1));

    // 3. 清除搜尋，應恢復無搜尋狀態，標題不再有 [/pub]
    pane.clear_preview_search();
    assert_eq!(pane.preview_match_count(), 0);
    let title_after_clear = pane.preview_title_for_entry(&entry);
    assert!(!title_after_clear.contains("[/pub]"));
}
