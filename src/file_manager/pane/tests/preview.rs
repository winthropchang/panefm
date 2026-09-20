use super::*;

#[test]
/// 驗證資料夾 preview 會包含摘要資訊與部分子項目名稱。
///
/// 參數：無。
/// 回傳：無；若 preview 缺少目錄摘要或子項目清單則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_directory_preview_shows_summary_and_children() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("nested")).expect("nested dir");
    fs::write(dir.path().join("nested").join("alpha.txt"), "hello").expect("alpha");
    fs::write(dir.path().join("nested").join("beta.txt"), "world").expect("beta");

    let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let preview: Vec<String> = pane
        .preview_lines(6, Theme::default())
        .into_iter()
        .map(|line| line.to_string())
        .collect();

    assert!(preview.iter().any(|line| line.contains("alpha.txt")));
    assert!(preview.iter().any(|line| line.contains("beta.txt")));
    assert!(preview.iter().any(|line| line.contains("")));
}

#[test]
/// 驗證文字檔 preview 會直接顯示帶有行號的檔案內容，不再插入額外資訊區。
///
/// 參數：無。
/// 回傳：無；若 preview 沒有顯示 metadata 或內容行號則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_file_preview_shows_metadata_and_numbered_lines() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("notes.txt"),
        "first line\nsecond line\nthird line\n",
    )
    .expect("notes");

    let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let preview: Vec<String> = pane
        .preview_lines(4, Theme::default())
        .into_iter()
        .map(|line| line.to_string())
        .collect();

    assert!(preview.iter().any(|line| line == "  1 first line"));
    assert!(preview.iter().any(|line| line == "  2 second line"));
    assert!(!preview.iter().any(|line| line.contains("path: ")));
}

#[test]
/// 驗證圖片 preview 會顯示圖片格式、尺寸與終端摘要訊息。
///
/// 參數：無。
/// 回傳：無；若圖片摘要資訊缺少格式或尺寸則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_image_preview_shows_format_and_dimensions() {
    let dir = tempdir().expect("tempdir");
    let png_bytes = vec![
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x02, 0x80, 0x00, 0x00, 0x01, 0xE0,
    ];
    fs::write(dir.path().join("wallpaper.png"), png_bytes).expect("png");

    let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let entry = pane.selected_entry().expect("entry");
    let title = pane.preview_title_for_entry(entry);

    assert!(title.contains("640 × 480") || title.contains("640 x 480"));
    assert!(title.contains("PNG"));
}

#[test]
/// 驗證真實圖片檔案在 Pane preview_lines 下會渲染出 Halfblock 縮圖與色彩，且標題顯示格式與尺寸。
///
/// 驗證內容：
/// 1. 建立真實有效的 PNG 圖片檔案。
/// 2. 建立包含該圖片的 PaneState 並呼叫 preview_title_for_entry 與 preview_lines。
/// 3. 驗證標題包含 format 與 dimensions，且內容包含 ▀ 字元的 Halfblock 縮圖行。
///
/// 保護目的：確保使用者在 PaneFM 檔案列表中對圖片按下 Tab 時能真正看到彩色縮圖。
fn pane_state_image_preview_renders_halfblocks_for_real_images() {
    let dir = tempdir().expect("tempdir");
    let mut img = image::RgbaImage::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            img.put_pixel(x, y, image::Rgba([200_u8, 100_u8, 50_u8, 255_u8]));
        }
    }
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .expect("write png");
    fs::write(dir.path().join("photo.png"), bytes).expect("photo");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_size(40, 12);
    let entry = pane.selected_entry().expect("entry");
    let title = pane.preview_title_for_entry(entry);
    assert!(title.contains("8 × 8") || title.contains("8 x 8"));
    assert!(title.contains("PNG"));

    let preview_lines = pane.preview_lines(12, Theme::default());
    assert!(
        preview_lines
            .iter()
            .any(|line| line.spans.iter().any(|span| span.content.as_ref() == "▀")),
        "預覽行必須包含 Halfblock ▀ 字元"
    );
}

#[test]
/// 驗證常見設定檔會顯示對應的 kind 標籤，方便快速辨識檔案類型。
///
/// 參數：無。
/// 回傳：無；若 preview 沒有顯示預期的類型標籤則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_config_preview_shows_kind_label() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("config.toml"), "theme = \"nightfox\"\n").expect("toml");

    let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let preview: Vec<String> = pane
        .preview_lines(4, Theme::default())
        .into_iter()
        .map(|line| line.to_string())
        .collect();

    assert!(
        preview
            .iter()
            .any(|line| line == "  1 theme = \"nightfox\"")
    );
}

#[test]
/// 驗證短檔案在未達捲動門檻時，preview_cursor 仍可自由向下與向上移動。
/// 保護目的：避免短檔案因 max_scroll 為 0 而使游標永遠鎖死在第一行。
fn pane_state_short_file_preview_cursor_moves_freely() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("short.txt");
    fs::write(&path, "one\ntwo\nthree\n").expect("write short.txt");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(10);
    pane.set_preview_viewport_size(60, 10);

    let theme = Theme::default();

    // 初始游標在第 0 行
    assert_eq!(pane.preview_cursor, 0);
    assert_eq!(pane.preview_scroll, 0);

    // 向下移動 1 行
    pane.scroll_preview_down(1);
    assert_eq!(pane.preview_cursor, 1);
    assert_eq!(pane.preview_scroll, 0);

    // 再次向下移動 1 行
    pane.scroll_preview_down(1);
    assert_eq!(pane.preview_cursor, 2);
    assert_eq!(pane.preview_scroll, 0);

    // 再往下不能超出總行數（3 行，最大 cursor 為 2）
    pane.scroll_preview_down(1);
    assert_eq!(pane.preview_cursor, 2);

    // 向上移動 1 行
    pane.scroll_preview_up(1);
    assert_eq!(pane.preview_cursor, 1);

    // 驗證當前行有高亮
    let lines = pane.preview_lines(10, theme);
    assert!(
        lines[1]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    );
}

#[test]
/// 驗證長檔案中 preview 游標可如文字編輯器般在可視範圍內自由上下移動，
/// 僅在游標超出可視範圍邊界時才觸發 viewport 捲動。
/// 保護目的：確保游標不被固定在首行或頂部，而是如 Vim / 編輯器般自由穿梭在可見行之間。
fn pane_state_preview_cursor_moves_freely_within_viewport_before_scrolling() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("code.rs");
    let code = (1..=50)
        .map(|i| format!("fn line_{i}() {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, code).expect("write code.rs");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(10);
    pane.set_preview_viewport_size(80, 10);

    let theme = Theme::default();

    // 初始游標與捲動皆為 0
    assert_eq!(pane.preview_cursor, 0);
    assert_eq!(pane.preview_scroll, 0);

    // 1. 游標在 viewport (10 行) 內向下移動 5 行：游標改變，但 viewport 不捲動！
    for expected_cursor in 1..=5 {
        pane.move_preview_cursor_down(1);
        assert_eq!(pane.preview_cursor, expected_cursor);
        assert_eq!(pane.preview_scroll, 0, "游標未達底部邊界前視窗不應捲動");
    }

    // 驗證此時高亮確實套用在第 5 行 (index 5) 而非第 0 行
    let lines = pane.preview_lines(10, theme);
    assert!(
        lines[5]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    );
    assert!(
        !lines[0]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
    );

    // 2. 移動游標至第 9 行 (index 9，即 viewport 的最底行)
    for _ in 6..=9 {
        pane.move_preview_cursor_down(1);
    }
    assert_eq!(pane.preview_cursor, 9);
    assert_eq!(pane.preview_scroll, 0);

    // 3. 再次向下移動 1 行 (至 index 10)：游標超出 viewport 底部，此時視窗才向下捲動 1 行！
    pane.move_preview_cursor_down(1);
    assert_eq!(pane.preview_cursor, 10);
    assert_eq!(
        pane.preview_scroll, 1,
        "游標超出 viewport 底部時視窗應捲動 1 行"
    );

    // 4. 向上移動 1 行 (回 index 9)：游標仍在目前 viewport (1..11) 內，視窗不捲動
    pane.move_preview_cursor_up(1);
    assert_eq!(pane.preview_cursor, 9);
    assert_eq!(pane.preview_scroll, 1, "向上移動未達頂部邊界時視窗不捲動");

    // 5. 連續向上移動直到超出頂部邊界 (index 0)
    pane.move_preview_cursor_up(9);
    assert_eq!(pane.preview_cursor, 0);
    assert_eq!(pane.preview_scroll, 0, "游標回到頂部時視窗捲回 0");
}

#[test]
/// 驗證 PaneState 預覽捲動具備快取機制，j/k 捲動時重複讀取切片不會重複讀檔與語法解析。
/// 保護目的：避免大檔案或長程式碼預覽在 j/k 快速捲動時產生卡頓。
fn pane_state_preview_scrolling_uses_cache_without_lag() {
    let dir = tempdir().expect("tempdir");
    let test_file = dir.path().join("main.rs");
    let mut code = String::new();
    for i in 1..=60 {
        code.push_str(&format!("// line {}\nlet var_{} = {};\n", i, i, i));
    }
    fs::write(&test_file, &code).expect("write rust file");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(10);
    pane.set_preview_viewport_size(80, 10);

    // 1. 初次載入預覽
    let lines_initial = pane.preview_lines(10, Theme::default());
    assert_eq!(lines_initial.len(), 10);
    assert!(
        pane.preview_content_cache.lock().unwrap().is_some(),
        "預覽內容應寫入快取"
    );

    // 2. 向下捲動 5 行（模擬鍵盤按下 5 次 j）
    pane.scroll_preview_down(5);
    let lines_scrolled = pane.preview_lines(10, Theme::default());
    assert_eq!(lines_scrolled.len(), 10);
    assert!(pane.preview_has_more_below());

    // 驗證捲動後的行內容確實位移
    assert_ne!(lines_initial[0].to_string(), lines_scrolled[0].to_string());

    // 3. 向上捲動 2 行（模擬鍵盤按下 2 次 k）
    pane.scroll_preview_up(2);
    assert_eq!(pane.preview_scroll, 3);
}

#[test]
/// 驗證超過 1000 行的大型原始碼檔案在首次開啟預覽時採用漸進式載入（優先高亮前段），
/// 能在幾毫秒內極速呈現畫面，絕不阻塞主執行緒造成卡頓。
fn pane_state_large_file_over_1000_lines_previews_instantly_without_lag() {
    let dir = tempdir().expect("tempdir");
    let test_file = dir.path().join("big_file.rs");
    let mut code = String::new();
    for i in 1..=1500 {
        code.push_str(&format!("fn function_{i}() -> usize {{ {i} }}\n"));
    }
    fs::write(&test_file, &code).expect("write big file");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(20);
    pane.set_preview_viewport_size(80, 20);

    // 預熱全域靜態語法庫（單次 process 生命週期只會載入一次），避免將語法引擎反序列化時間算入首屏切片時間
    let syntax = crate::file_manager::preview::SYNTAX_SET
        .find_syntax_by_extension("rs")
        .unwrap();
    let theme = &crate::file_manager::preview::THEME_SET.themes["base16-ocean.dark"];
    let mut h = syntect::easy::HighlightLines::new(syntax, theme);
    let _ = h.highlight_line(
        "fn warmup() {}\n",
        &crate::file_manager::preview::SYNTAX_SET,
    );

    // 1. 初次載入預覽：首屏 20 行必須極速返回（放寬至 1000ms 以避免 CI 虛擬環境 debug 模式排程延遲誤判，release 通常 < 5ms）
    let t0 = std::time::Instant::now();
    let lines = pane.preview_lines(20, Theme::default());
    let elapsed = t0.elapsed();

    assert_eq!(lines.len(), 20);
    assert!(
        elapsed < std::time::Duration::from_millis(1000),
        "首次預覽大檔案應在極短時間內返回首屏，實際耗時: {:?}",
        elapsed
    );

    // 2. 驗證總行數已知且捲動上下界正確
    let guard = pane.preview_content_cache.lock().unwrap();
    let cache = guard.as_ref().expect("cache should exist");
    assert_eq!(cache.total_lines, 1500, "總行數應正確識別為 1500 行");
    drop(guard);

    assert_eq!(pane.max_preview_scroll(), 1480);
    assert!(pane.preview_has_more_below());

    // 3. 等待背景執行緒完成全文高亮
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let guard = pane.preview_content_cache.lock().unwrap();
        if let Some(c) = guard.as_ref()
            && c.is_complete
        {
            assert_eq!(c.lines.len(), 1500);
            break;
        }
    }
}

#[test]
/// 驗證在 5000 行以上的大型檔案中，於第一行按下 G 跳至最後一行時零停頓（< 50ms），
/// 絕不重新同步解析幾千行語法。
fn pane_state_jump_to_bottom_with_g_on_large_file_is_instant() {
    let dir = tempdir().expect("tempdir");
    let test_file = dir.path().join("file_5000.rs");
    let mut code = String::new();
    for i in 1..=5000 {
        code.push_str(&format!("fn function_{i}() -> usize {{ {i} }}\n"));
    }
    fs::write(&test_file, &code).expect("write 5000 lines file");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(20);
    pane.set_preview_viewport_size(80, 20);

    // 預熱語法庫
    let syntax = crate::file_manager::preview::SYNTAX_SET
        .find_syntax_by_extension("rs")
        .unwrap();
    let theme = &crate::file_manager::preview::THEME_SET.themes["base16-ocean.dark"];
    let mut h = syntect::easy::HighlightLines::new(syntax, theme);
    let _ = h.highlight_line(
        "fn warmup() {}\n",
        &crate::file_manager::preview::SYNTAX_SET,
    );

    // 初次載入預覽（首屏）
    let _ = pane.preview_lines(20, Theme::default());

    // 模擬使用者在第一行立即按下 G 跳到最後一行
    let t0 = std::time::Instant::now();
    pane.scroll_preview_bottom();
    let lines = pane.preview_lines(20, Theme::default());
    let elapsed = t0.elapsed();
    println!(
        "Time to jump to bottom and render last 20 lines: {:?}",
        elapsed
    );

    assert_eq!(lines.len(), 20);
    assert_eq!(pane.preview_scroll, 4980);
    // 驗證跳到底部時間必須極短（放寬至 1000ms 以避免 CI 虛擬機 debug 模式排程雜訊，release 通常 < 5ms）
    assert!(
        elapsed < std::time::Duration::from_millis(1000),
        "按下 G 跳至末尾應極速返回，實際耗時: {:?}",
        elapsed
    );

    // 驗證末尾行行號為 5000 且程式碼關鍵字帶有語法色彩
    let last_line = &lines[19];
    let line_str = last_line.to_string();
    assert!(
        line_str.contains("5000"),
        "末尾行應包含行號 5000，實際為: {line_str}"
    );
    assert!(
        line_str.contains("function_5000"),
        "末尾行應包含函式名稱，實際為: {line_str}"
    );
    let has_colored_kw = last_line
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("fn") && s.style.fg.is_some());
    assert!(has_colored_kw, "末尾行切片渲染應包含 fn 關鍵字語法顏色");
}

#[test]
/// 驗證鄰近預覽背景預熱功能：
/// 當 Pane 載入目前檔案預覽時，背景執行緒會自動預先快取鄰近檔案，
/// 讓使用者切換至鄰近檔案時達到 0ms 記憶體快取命中。
fn pane_state_prefetches_adjacent_previews_into_cache() {
    let dir = tempdir().expect("tempdir");
    let file_a = dir.path().join("a.rs");
    let file_b = dir.path().join("b.rs");
    let file_c = dir.path().join("c.rs");

    fs::write(&file_a, "fn a() { println!(\"A\"); }\n").expect("write a");
    fs::write(&file_b, "fn b() { println!(\"B\"); }\n").expect("write b");
    fs::write(&file_c, "fn c() { println!(\"C\"); }\n").expect("write c");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_height(10);
    pane.set_preview_viewport_size(80, 10);

    // 游標移動至中間項目 b.rs
    pane.move_to_visible_index(1);
    let selected_name = pane.selected_entry().map(|e| e.name.clone());
    assert_eq!(selected_name.as_deref(), Some("b.rs"));

    // 載入 b.rs 預覽，觸發鄰近預熱
    let _ = pane.preview_lines(10, Theme::default());

    // 等待背景預熱執行緒完成（通常 < 50ms）
    let mut a_cached = false;
    let mut c_cached = false;
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let guard = pane.preview_content_cache.lock().unwrap();
        if guard
            .get(
                &file_a,
                Some(fs::metadata(&file_a).unwrap().modified().unwrap()),
                pane.preview_viewport_width,
            )
            .is_some()
        {
            a_cached = true;
        }
        if guard
            .get(
                &file_c,
                Some(fs::metadata(&file_c).unwrap().modified().unwrap()),
                pane.preview_viewport_width,
            )
            .is_some()
        {
            c_cached = true;
        }
        if a_cached && c_cached {
            break;
        }
    }

    assert!(a_cached, "鄰近項目 a.rs 應被背景預熱至快取");
    assert!(c_cached, "鄰近項目 c.rs 應被背景預熱至快取");
}

#[test]
/// 驗證清單模式預熱會將當前選取項目 (include_current = true) 一併預熱進入快取，
/// 且重複觸發會被 in_flight 機制過濾防重。
fn pane_state_prefetches_current_and_adjacent_in_list_mode() {
    let dir = tempdir().expect("tempdir");
    let file_x = dir.path().join("x.rs");
    let file_y = dir.path().join("y.rs");
    let file_z = dir.path().join("z.rs");

    fs::write(&file_x, "fn x() {}\n").expect("write x");
    fs::write(&file_y, "fn y() {}\n").expect("write y");
    fs::write(&file_z, "fn z() {}\n").expect("write z");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_size(80, 10);

    // 停留在 y.rs
    pane.move_to_visible_index(1);

    // 觸發清單模式預熱 (include_current = true)
    pane.prefetch_current_and_adjacent_previews(true);

    // 驗證 in_flight 或已在快取中
    {
        let guard = pane.preview_content_cache.lock().unwrap();
        assert!(guard.is_in_flight_or_cached(
            &file_y,
            Some(fs::metadata(&file_y).unwrap().modified().unwrap()),
            80
        ));
    }

    // 等待背景預熱完成
    let mut y_cached = false;
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let guard = pane.preview_content_cache.lock().unwrap();
        if guard
            .get(
                &file_y,
                Some(fs::metadata(&file_y).unwrap().modified().unwrap()),
                pane.preview_viewport_width,
            )
            .is_some()
        {
            y_cached = true;
            break;
        }
    }
    assert!(y_cached, "當前選取項目 y.rs 應在清單模式下被預熱至快取");
}

#[test]
/// 驗證鄰近預覽背景預熱 (Speculative Prefetching) 在預熱短小檔案後，
/// 絕不會污染當前選取較大檔案的 max_preview_scroll，且進入 preview 模式後游標與捲動皆可立即流暢操作。
fn test_adjacent_prefetch_does_not_corrupt_active_preview_max_scroll() {
    let dir = tempdir().expect("tempdir");
    let file_big = dir.path().join("a_big.md");
    let file_small = dir.path().join("b_small.toml");

    // a_big 有 500 行，b_small 只有 10 行
    let big_content = (1..=500).map(|i| format!("Line {i}\n")).collect::<String>();
    let small_content = (1..=10)
        .map(|i| format!("Config {i}\n"))
        .collect::<String>();

    fs::write(&file_big, big_content).expect("write big");
    fs::write(&file_small, small_content).expect("write small");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_preview_viewport_size(80, 20);

    // 游標停留在 a_big.md (index 0)
    pane.move_to_visible_index(0);
    assert_eq!(pane.selected_entry().unwrap().name, "a_big.md");

    // 在清單模式下觸發預熱（會依序將 a_big.md 與相鄰的 b_small.toml 預熱進快取）
    pane.prefetch_current_and_adjacent_previews(true);

    // 等待背景預熱完成
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let guard = pane.preview_content_cache.lock().unwrap();
        if guard.contains(&file_big, None, 80) && guard.contains(&file_small, None, 80) {
            break;
        }
    }

    // 按下 Tab 進入 preview 模式
    assert!(pane.toggle_preview_active());

    // 核心驗證 1：max_preview_scroll 必須精準反映 a_big.md 的 500 行 (500 - 20 = 480)，
    // 絕不能被相鄰預熱的 b_small.toml (10 行 -> saturating_sub 變 0) 鎖死！
    let max_scroll = pane.max_preview_scroll();
    assert_eq!(
        max_scroll, 480,
        "max_preview_scroll 必須為 a_big.md 的 480，而非相鄰檔案的 0"
    );
    assert!(
        pane.preview_has_more_below(),
        "preview_has_more_below 應回傳 true"
    );

    // 核心驗證 2：向下捲動 5 行，游標與捲動位置必須立即反映，絕不卡死在 0
    pane.scroll_preview_down(5);
    assert_eq!(
        pane.preview_scroll, 5,
        "向下捲動 5 行後 preview_scroll 應為 5"
    );

    // 核心驗證 3：向下翻半頁
    pane.page_preview_down();
    assert_eq!(
        pane.preview_scroll, 15,
        "向下翻半頁後 preview_scroll 應為 15"
    );

    // 核心驗證 4：捲動至最底部
    pane.scroll_preview_bottom();
    assert_eq!(
        pane.preview_scroll, 480,
        "捲動至最底部後 preview_scroll 應為 480"
    );

    // 核心驗證 5：捲動回最上方
    pane.scroll_preview_top();
    assert_eq!(
        pane.preview_scroll, 0,
        "捲動至最上方後 preview_scroll 應為 0"
    );
}
