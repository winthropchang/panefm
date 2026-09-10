use super::{
    FileCategory, IconStyle, SearchListState, TaskPanelLine, entry_icon, file_category,
    format_diff_path_column, format_pane_title, format_permissions_detail, format_size_short,
    format_sort_detail, regex_rename_status_style, render_entry_line, render_pane_title_line,
    search_empty_message, search_list_selected_index, task_panel_display_lines,
    top_right_input_rect, truncate_text_to_display_width, visible_list_window_range,
};
use ratatui::layout::Rect;
use std::path::Path;
use std::time::SystemTime;
use unicode_width::UnicodeWidthStr;

use crate::file_manager::entry::FileEntry;
use crate::file_manager::pane::SortDetailKind;
use crate::file_manager::search::GlobalSearchEntry;
use crate::theme::Theme;

/// 建立 UI 格式測試使用的最小 FileEntry，避免測試依賴實際檔案系統 metadata。
fn test_entry(name: &str, is_dir: bool) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path: Path::new(name).to_path_buf(),
        is_dir,
        size: 0,
        directory_size: None,
        directory_size_complete: false,
        modified: SystemTime::UNIX_EPOCH,
        created: SystemTime::UNIX_EPOCH,
        readonly: false,
        unix_mode: None,
    }
}

#[test]
/// 驗證當項目處於背景傳輸或處理中時，列表列會直接顯示工作標籤與百分比。
fn render_entry_line_displays_active_job_badge() {
    let entry = test_entry("terminal-file-manager", true);
    let line = render_entry_line(
        &entry,
        false,
        false,
        false,
        SortDetailKind::None,
        60,
        Theme::from(crate::theme::ThemePreset::Dracula),
        true,
        IconStyle::NerdFont,
        None,
        None,
        Some("[copying 99%]"),
    );
    let text = line
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert!(
        text.contains("[copying 99%]"),
        "列表列必須包含工作進度標籤: {text}"
    );
}

#[test]
/// 驗證中文目錄名稱會依終端顯示寬度截短，而不是把 linemode 資訊推到 panel 外。
///
/// 保護目的：一個中文字通常佔兩個終端欄位。若排版誤用 `chars().count()`，size
/// 與 permissions 看似沒有資料，實際上是被超寬名稱裁掉。測試同時覆蓋兩種右側
/// 資訊，確保所有 `m` 選項共用的列表排版都保留完整 detail。
fn render_entry_line_keeps_details_visible_after_wide_chinese_name() {
    let theme = Theme::from(crate::theme::ThemePreset::Dracula);
    let mut entry = test_entry("Ch02 單層感知器的數學原理與實作入門", true);
    entry.directory_size = Some(6 * 1_024);
    entry.directory_size_complete = true;
    entry.unix_mode = Some(0o755);

    for (detail_kind, expected_suffix) in [
        (SortDetailKind::Size, "6K"),
        (SortDetailKind::Permissions, "drwxr-xr-x"),
    ] {
        let line = render_entry_line(
            &entry,
            false,
            false,
            false,
            detail_kind,
            32,
            theme,
            false,
            IconStyle::Ascii,
            None,
            None,
            None,
        );
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(UnicodeWidthStr::width(text.as_str()), 32);
        assert!(
            text.ends_with(expected_suffix),
            "右側 linemode 資訊不可被中文名稱裁掉: {text}"
        );
    }
}

#[test]
/// 驗證顯示寬度截短函數不會把中文誤當成單欄字元。
///
/// 保護目的：此函數是列表名稱與右側資訊正確對齊的基礎；未來調整圖示或樣式時，
/// 仍須確保輸出寬度不超過限制，且被截短時有明確省略號。
fn truncate_text_uses_terminal_width_for_chinese() {
    let truncated = truncate_text_to_display_width("中文目錄abcdef", 8);

    assert_eq!(truncated, "中文目…");
    assert_eq!(UnicodeWidthStr::width(truncated.as_str()), 7);
}

#[test]
/// 驗證大型目錄只會渲染 viewport 內的少量項目，而不是替完整列表建立 widget。
///
/// 保護目的：`target/debug/deps` 類型的目錄可能包含上萬個檔案。若一次 j/k 仍建立
/// 10,000 個 ListItem，游標操作會明顯停頓；此測試固定要求 10,000 筆資料只選出
/// 20 列，並確保游標離開舊畫面時 viewport 會跟隨但仍包含目前項目。
fn large_list_window_only_contains_visible_rows() {
    let (start, end) = visible_list_window_range(10_000, 5_432, 20, 0);

    assert_eq!(end - start, 20);
    assert!(start <= 5_432 && 5_432 < end);
    assert_eq!((start, end), (5_413, 5_433));
}

#[test]
/// 驗證游標仍在原 viewport 內時保持畫面起點，避免每按一次 j/k 整頁就跟著跳動。
///
/// 保護目的：虛擬化不能只追求速度，也必須保留一般檔案管理器穩定的捲動體驗。
fn list_window_keeps_previous_start_while_selection_is_visible() {
    assert_eq!(visible_list_window_range(10_000, 510, 30, 500), (500, 530));
    assert_eq!(visible_list_window_range(10_000, 499, 30, 500), (499, 529));
}

#[test]
/// 驗證右上角輸入框會使用傳入 Panel 的座標，而不是退回整個 terminal 的右上角。
/// 保護目的：避免多 Panel 重構後，Find／Filter UI 再次脫離 active Panel。
fn top_right_input_rect_is_relative_to_owning_panel() {
    let panel = Rect::new(40, 2, 38, 18);

    let popup = top_right_input_rect(panel);

    assert_eq!(popup, Rect::new(45, 3, 32, 3));
    assert!(popup.x >= panel.x);
    assert!(popup.right() <= panel.right());
    assert!(popup.bottom() <= panel.bottom());
}

#[test]
/// 驗證 Panel 小於一般輸入框寬度時，輸入框會縮小並完整留在 Panel 內。
/// 保護目的：避免四分割或更小版面中，Filter 邊框與文字覆蓋相鄰 Panel。
fn top_right_input_rect_shrinks_inside_narrow_panel() {
    let panel = Rect::new(12, 4, 10, 2);

    let popup = top_right_input_rect(panel);

    assert_eq!(popup, Rect::new(12, 5, 9, 1));
    assert!(popup.right() <= panel.right());
    assert!(popup.bottom() <= panel.bottom());
}

#[test]
/// 驗證 task 的完整目的路徑會移到摘要下方，並依窄 panel 寬度換成多行。
///
/// 參數：無。
/// 回傳：無；若目的地尾端被裁掉，或任一行超出 panel 寬度，測試失敗。
/// 保護目的：大型 copy 最重要的診斷資訊位於 detail 尾端；過去與固定欄位塞在
/// 同一行，只看得到 `destination: /`，無法判斷工作實際寫到哪個目錄。
fn task_detail_wraps_without_losing_the_destination_tail() {
    let task = TaskPanelLine {
        state: String::from("RUNNING"),
        started_at: String::from("14:30:47"),
        finished_at: String::from("--:--:--"),
        progress: String::from("24.4G / 77.2G"),
        title: String::from("copy 1 item(s)"),
        source_locations: vec![String::from(
            "/Users/otto/Documents/source/large-archive.zip",
        )],
        destination_location: Some(String::from(
            "/Users/otto/Documents/AB_Demo/very-long-target-directory",
        )),
        detail: String::from("pasted copy: 1 item"),
        marked: false,
    };

    let rendered = task_panel_display_lines(&task, 32);
    let rendered_text = rendered.iter().map(ToString::to_string).collect::<Vec<_>>();

    assert!(rendered_text.len() >= 4);
    let source_start = rendered_text
        .iter()
        .position(|line| line.starts_with("  source:"))
        .expect("source must start on its own indented line");
    let destination_start = rendered_text
        .iter()
        .position(|line| line.starts_with("  destination:"))
        .expect("destination must start on its own indented line");
    let result_start = rendered_text
        .iter()
        .position(|line| line.starts_with("  result:"))
        .expect("result must start on its own indented line");
    let reconstructed_source = rendered_text[source_start..destination_start]
        .iter()
        .map(|line| line.trim_start())
        .collect::<String>();
    let reconstructed_destination = rendered_text[destination_start..result_start]
        .iter()
        .map(|line| line.trim_start())
        .collect::<String>();
    assert!(reconstructed_source.contains("large-archive.zip"));
    assert!(reconstructed_destination.contains("very-long-target-directory"));
    assert!(
        rendered_text
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 32)
    );
}

#[test]
/// 驗證多選工作只在面板展開前五個來源，並清楚顯示尚有多少來源未展開。
///
/// 保護目的：完整來源必須寫進歷史供診斷，但一次操作數百個檔案時不能讓單一 task
/// 佔滿整個 panel；此測試固定 UI 摘要與持久化資料必須彼此獨立。
fn task_panel_summarizes_large_source_batches_without_losing_the_count() {
    let task = TaskPanelLine {
        state: String::from("RUNNING"),
        started_at: String::from("10:00:00"),
        finished_at: String::from("--:--:--"),
        progress: String::from("1M / 8M"),
        title: String::from("move 8 item(s)"),
        source_locations: (1..=8)
            .map(|index| format!("/source/file-{index}.txt"))
            .collect(),
        destination_location: Some(String::from("/destination")),
        detail: String::from("running"),
        marked: false,
    };

    let rendered = task_panel_display_lines(&task, 80)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("source 5: /source/file-5.txt"));
    assert!(rendered.contains("source: ... and 3 more"));
    assert!(!rendered.contains("file-6.txt"));
    assert!(rendered.contains("destination: /destination"));
}

#[test]
/// 驗證搜尋仍在背景載入時，只要已有結果就不會再用 Loading 訊息蓋住列表。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn search_results_are_visible_while_loading() {
    let entry = GlobalSearchEntry {
        path: Path::new("result.txt").to_path_buf(),
        relative_path: String::from("result.txt"),
        is_dir: false,
        match_line_number: Some(1),
        match_column: Some(1),
        match_preview: Some(String::from("887")),
    };
    let results = vec![entry];
    let state = SearchListState {
        results: &results,
        selected: 0,
        loading: true,
        preview_query: None,
        preview_scroll: None,
        preview_current_match: None,
    };

    assert_eq!(search_empty_message(&state), None);
    assert_eq!(search_list_selected_index(&state), Some(0));
}

#[test]
/// 驗證背景搜尋仍在回傳資料時，列表會立即顯示目前游標，而不是等待 Done 事件。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn search_cursor_is_visible_and_clamped_while_loading() {
    let results = vec![
        GlobalSearchEntry {
            path: Path::new("first.txt").to_path_buf(),
            relative_path: String::from("first.txt"),
            is_dir: false,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        },
        GlobalSearchEntry {
            path: Path::new("second.txt").to_path_buf(),
            relative_path: String::from("second.txt"),
            is_dir: false,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        },
    ];
    let state = SearchListState {
        results: &results,
        selected: 99,
        loading: true,
        preview_query: None,
        preview_scroll: None,
        preview_current_match: None,
    };

    assert_eq!(search_list_selected_index(&state), Some(1));
}

#[test]
/// 驗證 regex 預覽右側狀態會依 ready、unchanged 與錯誤類型套用主題顏色。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn regex_rename_status_uses_theme_semantic_colors() {
    let theme = Theme::default_theme();
    assert_eq!(
        regex_rename_status_style(theme, "ready").fg,
        Some(theme.executable)
    );
    assert_eq!(
        regex_rename_status_style(theme, "unchanged").fg,
        Some(theme.muted)
    );
    assert_eq!(
        regex_rename_status_style(theme, "conflict").fg,
        Some(theme.danger)
    );
    assert_eq!(
        regex_rename_status_style(theme, "invalid").fg,
        Some(theme.danger)
    );
}

#[test]
/// 驗證列表會依照目錄與常見副檔名分辨檔案類別與圖示。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn entry_kind_uses_cross_platform_categories() {
    assert_eq!(
        entry_icon(&test_entry("src", true), IconStyle::Ascii),
        "[D]"
    );
    assert_eq!(
        file_category(&test_entry("main.rs", false)),
        FileCategory::Source
    );
    assert_eq!(
        file_category(&test_entry("photo.png", false)),
        FileCategory::Image
    );
    assert_eq!(
        file_category(&test_entry("backup.zip", false)),
        FileCategory::Archive
    );
    assert_eq!(
        file_category(&test_entry("tool.exe", false)),
        FileCategory::Executable
    );
}

#[test]
/// 驗證大小格式會轉成人類較容易閱讀的單位顯示。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn format_size_short_uses_compact_units() {
    assert_eq!(format_size_short(512), "512B");
    assert_eq!(format_size_short(2_048), "2K");
    assert_eq!(format_size_short(1_572_864), "1.5M");
    assert_eq!(format_size_short(6_270_192_614), "5.84G");
}

#[test]
/// 驗證 size linemode 會區分背景計算中的部分容量與已完成的真實容量。
///
/// 保護目的：使用者必須知道數字是否還會增加；掃描中以 `~` 標示，完成後同一列
/// 應只留下最終大小，不能退回舊版的直接子項目數量。
fn size_detail_marks_partial_directory_size_until_scan_completes() {
    let mut entry = test_entry("target", true);
    entry.directory_size = Some(1_572_864);

    assert_eq!(format_sort_detail(&entry, SortDetailKind::Size), "~1.5M");
    entry.directory_size_complete = true;
    assert_eq!(format_sort_detail(&entry, SortDetailKind::Size), "1.5M");
}

#[test]
/// 驗證 pane 標題會把固定 pane 編號以膠囊形式顯示在最前面，方便對照快捷鍵切換。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn format_pane_title_keeps_stable_pane_id_prefix() {
    let title = format_pane_title(
        3,
        Path::new("/tmp/demo"),
        "  [filter]",
        "  [mark: 2]",
        "  [help]",
        "sort: natural",
        80,
    );

    assert_eq!(
        title,
        " 3  /tmp/demo [filter] [mark: 2] [help] [sort: natural]"
    );
}

#[test]
/// 驗證 pane 寬度足夠時，標題會完整顯示，不會過早縮短路徑。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn format_pane_title_keeps_full_path_when_it_fits() {
    let title = format_pane_title(
        2,
        Path::new("/Users/otto/Documents/terminal-file-manager"),
        "",
        "",
        "",
        "sort: natural",
        120,
    );

    assert_eq!(
        title,
        " 2  /Users/otto/Documents/terminal-file-manager [sort: natural]"
    );
}

#[test]
/// 驗證 pane 很窄時，標題仍會自動縮短，不會超出可用寬度。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn format_pane_title_compacts_long_path_for_narrow_panes() {
    let title = format_pane_title(
        12,
        Path::new("/Users/otto/GitHub/cocos-tutorial-happy-path/dev/library/very/deep/path"),
        "",
        "",
        "",
        "sort: natural",
        32,
    );

    assert!(title.chars().count() <= 32);
    assert!(title.starts_with(" 12 "));
    assert!(title.contains("natural"));
    assert!(title.contains("path"));
    assert!(!title.contains("happy-path/dev"));
    assert!(!title.contains("/…/…"));
}

#[test]
/// 驗證路徑放不下時，會優先保留最後目錄名稱，而不是在前面留下多餘根路徑。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn format_pane_title_prefers_last_directory_tail() {
    let title = format_pane_title(
        4,
        Path::new("/Users/otto/Documents/terminal-file-manager"),
        "",
        "",
        "",
        "sort: natural",
        40,
    );

    assert!(title.contains("…"));
    assert!(title.contains("file-manager"));
    assert!(title.contains("[sort: natural]"));
    assert!(!title.contains("/…/…"));
}

#[test]
/// 驗證 linemode 開啟後，pane 標題尾端會顯示目前啟用的 linemode。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn format_pane_title_supports_linemode_status() {
    let title = format_pane_title(5, Path::new("/tmp/demo"), "", "", "", "linemode: size", 80);

    assert_eq!(title, " 5  /tmp/demo [linemode: size]");
}

#[test]
/// 驗證 render_pane_title_line 產生的標題 Line 包含高亮實心膠囊徽章與正確樣式。
/// 保護目的：避免主題或樣式重構後，造成視窗編號無法凸顯或文字對比不足。
fn render_pane_title_line_creates_styled_badge_spans() {
    let theme = Theme::default_theme();
    let focused_line = render_pane_title_line(" 1 ", "/tmp", "[mtime]", true, theme);
    assert_eq!(focused_line.spans[0].content, " 1 ");
    assert_eq!(focused_line.spans[0].style, theme.pane_badge_style(true));

    let unfocused_line = render_pane_title_line(" 2 ", "/tmp", "[mtime]", false, theme);
    assert_eq!(unfocused_line.spans[0].content, " 2 ");
    assert_eq!(unfocused_line.spans[0].style, theme.pane_badge_style(false));
}

#[test]
/// 驗證非 Unix 平台或缺少 mode bits 時，permissions 仍有可讀的 fallback 顯示。
/// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
fn format_permissions_detail_falls_back_to_cross_platform_text() {
    let entry = FileEntry {
        name: String::from("notes.txt"),
        path: Path::new("/tmp/notes.txt").to_path_buf(),
        is_dir: false,
        size: 12,
        directory_size: None,
        directory_size_complete: false,
        modified: SystemTime::UNIX_EPOCH,
        created: SystemTime::UNIX_EPOCH,
        readonly: true,
        unix_mode: None,
    };

    assert_eq!(format_permissions_detail(&entry), "file readonly");
}

#[test]
fn format_diff_path_column_truncates_and_pads_to_exact_width() {
    let short = "src/main.rs";
    let formatted = format_diff_path_column(short, 20);
    assert_eq!(UnicodeWidthStr::width(formatted.as_str()), 20);
    assert!(formatted.starts_with("src/main.rs"));

    let long = ".creator/asset-template/typescript/Custom Script Template Help Documentation.url";
    let formatted_long = format_diff_path_column(long, 35);
    assert_eq!(UnicodeWidthStr::width(formatted_long.as_str()), 35);
    assert!(formatted_long.contains('…'));
    assert!(formatted_long.starts_with(".cre"));
    assert!(formatted_long.ends_with(".url"));
}

#[test]
/// 驗證 `cursor_display_width` 會依終端機全形欄寬正確累加中英數混和字串。
/// 保護目的：避免中文或寬字元檔名被當成 1 欄位算，造成 rename 輸入框的游標錯位。
fn cursor_display_width_handles_ascii_and_wide_cjk() {
    use super::cursor_display_width;

    // ASCII: 1 字元 = 1 欄寬
    assert_eq!(cursor_display_width("alpha.txt", 0), 0);
    assert_eq!(cursor_display_width("alpha.txt", 5), 5);
    assert_eq!(cursor_display_width("alpha.txt", 9), 9);
    assert_eq!(cursor_display_width("alpha.txt", 99), 9);

    // 中文: 1 字元 = 2 欄寬
    assert_eq!(cursor_display_width("中文.txt", 0), 0);
    assert_eq!(cursor_display_width("中文.txt", 1), 2); // 停在 '文' 前
    assert_eq!(cursor_display_width("中文.txt", 2), 4); // 停在 '.' 前（主檔名末端）
    assert_eq!(cursor_display_width("中文.txt", 6), 8); // 停在檔名最後

    // 中英混和: 2 * 4 + 4 = 12 欄寬
    assert_eq!(cursor_display_width("專案_v2_測試.rs", 8), 12);
}

#[test]
/// 驗證長文字輸入框在空間不足時會採用 Scheme A 水平滑動視窗，並附帶 `<` / `>` 溢位提示。
fn scrolled_input_handles_overflow_and_cursor_tracking() {
    use super::compute_scrolled_input;
    let theme = Theme::default();

    // 1. 完全裝得下：無任何溢位符號
    let view = compute_scrolled_input("goto docs", 4, 20, Some(":"), theme);
    let text: String = view.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, ":goto docs");
    assert_eq!(view.cursor_col, 5); // 1 (for :) + 4

    // 2. 游標靠左，右側溢位：顯示 `>`
    let long_path = "goto D:\\otto-documents\\github-panefm\\panefm\\src";
    let view_start = compute_scrolled_input(long_path, 5, 25, Some(":"), theme);
    let text_start: String = view_start
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text_start.starts_with(":goto "));
    assert!(text_start.ends_with('>'));
    assert!(!text_start.contains('<'));
    assert_eq!(view_start.cursor_col, 6); // 1 (for :) + 5

    // 3. 游標靠右，左側溢位：顯示 `<`
    let view_end =
        compute_scrolled_input(long_path, long_path.chars().count(), 25, Some(":"), theme);
    let text_end: String = view_end.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text_end.starts_with(":<"));
    assert!(text_end.ends_with("src"));
    assert!(!text_end.ends_with('>'));
    assert!(view_end.cursor_col < 25);
    assert_eq!(
        view_end.cursor_col as usize,
        unicode_width::UnicodeWidthStr::width(text_end.as_str())
    );

    // 游標在字尾與最後一個字元之間移動時，可見文字內容完全固定不晃動，且游標正確左右位移
    let total_chars = long_path.chars().count();
    let view_last_char = compute_scrolled_input(long_path, total_chars - 1, 25, Some(":"), theme);
    let text_last_char: String = view_last_char
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(
        text_last_char, text_end,
        "visible text must remain identical when moving near end"
    );
    assert_eq!(view_last_char.cursor_col + 1, view_end.cursor_col);

    // 4. 游標在中間，雙向溢位：同時顯示 `<` 與 `>`
    let view_mid = compute_scrolled_input(long_path, 28, 25, Some(":"), theme);
    let text_mid: String = view_mid.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text_mid.starts_with(":<"));
    assert!(text_mid.ends_with('>'));
    assert!(view_mid.cursor_col < 25);
}
