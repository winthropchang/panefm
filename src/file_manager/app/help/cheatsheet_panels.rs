//! 特殊覆蓋面板（全域搜尋、檔名跳轉、任務佇列、垃圾桶、Diff 比對、視窗管理、工具面板）的 Cheatsheet 速查表。

use super::types::{ContextHelpKind, HelpAction, HelpEntry, help_entry};

/// 取得特殊覆蓋面板的快捷鍵指南。
pub(crate) fn cheatsheet_for_panel(kind: ContextHelpKind) -> Option<(String, Vec<HelpEntry>)> {
    let result = match kind {
        ContextHelpKind::GlobalSearch => (
            String::from("Cheatsheet: Global Search (全域搜尋)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "在搜尋結果清單中上下移動",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump-large",
                    "J / K",
                    "大步快速移動游標",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "跳至搜尋結果頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "open",
                    "Enter / l / Right",
                    "跳轉並定位至所選檔案位置",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filter",
                    "f",
                    "在目前搜尋結果中進行即時模糊過濾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "re-edit",
                    "i / s",
                    "重新回到搜尋關鍵字輸入框重新搜尋",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "preview",
                    "Tab",
                    "（內容搜尋模式）切換進入 / 離開右側檔案內容預覽",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "preview-match",
                    "n / p / N",
                    "在預覽中跳至下一個 / 上一個內容比對匹配行",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "preview-scroll",
                    "j / k",
                    "在預覽中上下捲動檔案內容",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "exit",
                    "Esc / q / h",
                    "退出搜尋結果面板，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ListFind => (
            String::from("Cheatsheet: List Find (檔名尋找)"),
            vec![
                help_entry(
                    "type",
                    "Characters",
                    "輸入要尋找的檔名關鍵字",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "confirm",
                    "Enter",
                    "確認尋找並鎖定目標項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "next/prev",
                    "n / N",
                    "在檔案列表中跳至下一個 / 上一個匹配項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消檔名尋找並退出",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::TaskPanel => (
            String::from("Cheatsheet: Task Panel (任務管理)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選取任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "跳至任務清單頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "visual",
                    "v / V",
                    "開啟視覺連續多選模式（連續標記多個任務）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark",
                    "Space",
                    "標記 / 取消標記目前任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark-all",
                    "a",
                    "全選所有任務 / 清除所有標記",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d",
                    "直接刪除所選或所有已標記的任務記錄（不彈窗）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "clear-all",
                    "D",
                    "直接清空面板中所有任務記錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "x / c",
                    "取消目前正在執行的背景任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel-all",
                    "X / C",
                    "取消所有正在執行的背景任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "f",
                    "開啟搜尋列，即時過濾任務名稱或路徑",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "detail",
                    "Enter / l / Right",
                    "檢視該任務完整執行細節、路徑與錯誤訊息",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "close",
                    "Esc / q / t / h",
                    "關閉任務面板，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::TrashPanel => (
            String::from("Cheatsheet: Trash Panel (垃圾桶)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選取垃圾桶項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "跳至清單頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "visual",
                    "v / V",
                    "開啟視覺多選模式（連續標記多個項目）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark",
                    "Space",
                    "標記 / 取消標記目前項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark-all",
                    "a",
                    "全選所有項目 / 清除所有標記",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "restore",
                    "u",
                    "還原所選或已標記項目回原本目錄（需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "restore-all",
                    "U",
                    "還原垃圾桶內所有項目（需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d",
                    "永久刪除所選或已標記項目（需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "empty-trash",
                    "D",
                    "清空整個垃圾桶（永久刪除所有檔案，需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "f",
                    "開啟搜尋列，即時過濾垃圾桶項目名稱",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "detail",
                    "Enter / l",
                    "檢視原始路徑與刪除時間",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "close",
                    "Esc / q / h",
                    "關閉垃圾桶面板，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::DiffMatrix => (
            String::from("Cheatsheet: Diff Matrix (檔案差異比對)"),
            vec![
                help_entry(
                    "move",
                    "j / k",
                    "在差異項目清單中上下移動",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "switch-col",
                    "h / l",
                    "在左 / 中 / 右各 Panel 欄位間切換焦點",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "toggle",
                    "Space",
                    "勾選 / 切換選取要同步的差異項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "all",
                    "a",
                    "全選 / 取消全選所有差異項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "diff-detail",
                    "d",
                    "開啟雙欄檔案內容詳細 Diff 比對檢視視窗",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filter-cycle",
                    "f",
                    "循環切換篩選（全部 ➔ 僅差異 ➔ 僅獨有 ➔ 相同）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "gitignore",
                    "i",
                    "切換 .gitignore 規則（包含/排除 build 與忽略檔）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "hidden",
                    ".",
                    "切換顯示 / 隱藏以點開頭之隱藏檔",
                    HelpAction::QuitHint,
                ),
                help_entry("search", "/", "檔名關鍵字搜尋比對", HelpAction::QuitHint),
                help_entry(
                    "rescan",
                    "r",
                    "重新掃描所有比對 Panel 目錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "apply",
                    "Enter",
                    "套用同步動作（將選取項目從來源複製至目標）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "exit",
                    "Esc / q",
                    "退出差異比對矩陣，返回多面板模式",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::WindowPicker => (
            String::from("Cheatsheet: Window Layout (視窗分割與管理)"),
            vec![
                help_entry(
                    "split-v",
                    "v",
                    "垂直新增分割視窗 (Vertical Split)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "split-s",
                    "s",
                    "水平新增分割視窗 (Horizontal Split)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "focus",
                    "h / j / k / l",
                    "切換焦點至 左 / 下 / 上 / 右 視窗",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "close",
                    "c / q",
                    "關閉目前焦點視窗 (Close Pane)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "only",
                    "o",
                    "僅保留目前視窗，關閉其他所有視窗 (Only)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "diff",
                    "d",
                    "開啟多 Panel 目錄 Diff 矩陣比對",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "terminal",
                    "t",
                    "在目前目錄開啟外部終端機 (Terminal)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "resize-mode",
                    "r",
                    "進入視窗連續尺寸調整模式 (Sticky Resize Mode)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "equalize",
                    "=",
                    "均等重設所有分割視窗大小 (Equalize)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "width",
                    "W",
                    "調整目前視窗寬度 (預填 :width 指令)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "height",
                    "H",
                    "調整目前視窗高度 (預填 :height 指令)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "select-pane",
                    "1..9",
                    "直接切換焦點至指定編號視窗",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / w",
                    "取消退出視窗管理選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::WindowResize => (
            String::from("Cheatsheet: Window Resize Mode (視窗尺寸調整)"),
            vec![
                help_entry(
                    "shrink-width",
                    "h / Left",
                    "減少目前視窗寬度 (4 欄)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "expand-width",
                    "l / Right",
                    "增加目前視窗寬度 (4 欄)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "expand-height",
                    "k / Up",
                    "增加目前視窗高度 (2 列)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "shrink-height",
                    "j / Down",
                    "減少目前視窗高度 (2 列)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "equalize",
                    "=",
                    "均等平衡所有視窗尺寸 (Reset Equal)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "done",
                    "Esc / Enter / q",
                    "結束調整模式並返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ToolPanel => (
            String::from("Cheatsheet: Tool Dependencies (相依工具)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動檢視相依工具狀態",
                    HelpAction::QuitHint,
                ),
                help_entry("close", "Esc / q", "關閉相依工具面板", HelpAction::QuitHint),
            ],
        ),
        _ => return None,
    };
    Some(result)
}
