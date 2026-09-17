//! 核心互動模式（Normal、Visual、Command、Filter、Preview、Rename、Create、Dialog）的 Cheatsheet 速查表。

use super::types::{ContextHelpKind, HelpAction, HelpEntry, help_entry};

/// 取得編輯、預覽或導航互動模式的快捷鍵指南。
pub(crate) fn cheatsheet_for_mode(kind: ContextHelpKind) -> Option<(String, Vec<HelpEntry>)> {
    let result = match kind {
        ContextHelpKind::Normal => (
            String::from("Cheatsheet: Normal Mode (檔案列表)"),
            vec![
                help_entry(
                    "move",
                    "j / k",
                    "向下 / 向上移動檔案游標",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "navigate",
                    "h / l",
                    "返回上一層目錄 / 進入資料夾或開啟檔案",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "open",
                    "Enter / o",
                    "開啟所選檔案或進入資料夾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy",
                    "y / Y",
                    "複製目前檔案或所有標記項目 / 清除剪貼簿",
                    HelpAction::Command("copy"),
                ),
                help_entry(
                    "cut",
                    "x / X",
                    "剪下目前檔案或所有標記項目 / 清除剪貼簿",
                    HelpAction::Command("cut"),
                ),
                help_entry(
                    "paste",
                    "p / P",
                    "貼上檔案 / 強制覆蓋貼上",
                    HelpAction::Command("paste"),
                ),
                help_entry(
                    "rename",
                    "r",
                    "重新命名目前選取的檔案或資料夾",
                    HelpAction::Command("rename"),
                ),
                help_entry(
                    "regex-rename",
                    "R / :reg",
                    "開啟 Regex 批次改名預覽面板",
                    HelpAction::Command("rename-regex"),
                ),
                help_entry(
                    "create",
                    "a",
                    "建立新檔案或目錄（以 / 結尾為資料夾）",
                    HelpAction::Command("create"),
                ),
                help_entry(
                    "trash",
                    "d",
                    "將檔案移至垃圾桶 (需確認)",
                    HelpAction::Command("trash"),
                ),
                help_entry(
                    "delete!",
                    "D",
                    "永久直接刪除檔案或資料夾 (不進垃圾桶)",
                    HelpAction::Command("delete!"),
                ),
                help_entry(
                    "undo",
                    "u",
                    "復原上一步貼上或搬移操作 (Undo)",
                    HelpAction::Command("undo"),
                ),
                help_entry(
                    "visual",
                    "v / V",
                    "開啟視覺連續多選模式 (Visual Selection)",
                    HelpAction::Visual,
                ),
                help_entry(
                    "mark",
                    "Space",
                    "單檔切換標記 / 取消標記",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark-all",
                    "A",
                    "全選目前目錄所有檔案與資料夾",
                    HelpAction::Command("mark-all"),
                ),
                help_entry(
                    "unmark-all",
                    "U",
                    "清除目前目錄所有標記",
                    HelpAction::Command("unmark-all"),
                ),
                help_entry(
                    "invert-marks",
                    "Ctrl+r",
                    "反向切換目前目錄標記狀態",
                    HelpAction::Command("invert-marks"),
                ),
                help_entry(
                    "preview",
                    "Tab",
                    "切換右側檔案預覽 / 進入預覽模式",
                    HelpAction::Command("preview"),
                ),
                help_entry(
                    "hidden",
                    ".",
                    "切換顯示 / 隱藏以點開頭之隱藏檔",
                    HelpAction::Hidden,
                ),
                help_entry(
                    "compress",
                    "C",
                    "將所選項目壓縮為 ZIP 壓縮檔",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "extract",
                    "E",
                    "解壓縮所選壓縮檔（支援 zip, tar.gz, tar 等）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "sort",
                    ",",
                    "打開排序選單 (Name, Size, MTime, Ext, Reverse)",
                    HelpAction::Sort,
                ),
                help_entry(
                    "search-name",
                    "s",
                    "啟動 fd 檔名全域即時搜尋",
                    HelpAction::Command("search"),
                ),
                help_entry(
                    "search-content",
                    "S",
                    "啟動 rg 檔案內容全文即時搜尋",
                    HelpAction::Command("search"),
                ),
                help_entry(
                    "jump",
                    "z",
                    "啟動 fzf 目錄樹模糊搜尋跳轉",
                    HelpAction::Command("jump"),
                ),
                help_entry(
                    "zoxide",
                    "Z",
                    "打開 zoxide 常用歷史目錄跳轉清單",
                    HelpAction::Command("zoxide"),
                ),
                help_entry(
                    "list-find",
                    "/",
                    "快速檔名跳轉搜尋 (List Find)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filter",
                    "f / F",
                    "即時過濾目前目錄檔案 (Normal / Fuzzy Filter)",
                    HelpAction::Filter,
                ),
                help_entry(
                    "linemode",
                    "m",
                    "打開 Linemode 選單 (Size, Perms, BTime, MTime, None)",
                    HelpAction::Command("linemode "),
                ),
                help_entry(
                    "bookmark",
                    "b",
                    "打開書籤快捷選單 (ba 新增, bg 跳轉, bd 刪除, bD 清空)",
                    HelpAction::Command("bookmark"),
                ),
                help_entry(
                    "window",
                    "w",
                    "打開視窗分割與焦點選單 (wv 垂直, ws 水平, wh/j/k/l 切換, wc 關閉)",
                    HelpAction::Command("window"),
                ),
                help_entry(
                    "theme",
                    "t",
                    "打開佈景主題快速切換選單",
                    HelpAction::Command("theme list"),
                ),
                help_entry(
                    "tasks",
                    "T",
                    "打開任務管理面板 (檢視背景傳輸與執行進度)",
                    HelpAction::Command("tasks"),
                ),
                help_entry(
                    "trash-panel",
                    "gt",
                    "打開垃圾桶面板 (檢視與還原已刪除項目)",
                    HelpAction::Command("trash"),
                ),
                help_entry(
                    "diff",
                    "Alt+d / :diff",
                    "開啟全螢幕多 Panel 目錄矩陣與檔案內容比對",
                    HelpAction::Command("diff"),
                ),
                help_entry(
                    "command",
                    ":",
                    "開啟底端命令列模式 (Command Mode)",
                    HelpAction::Command(""),
                ),
                help_entry(
                    "help",
                    "~/F1",
                    "打開全局完整說明手冊 (Help Dictionary)",
                    HelpAction::Command("help"),
                ),
                help_entry(
                    "cheatsheet",
                    "?",
                    "開啟當前面板快捷鍵指南 (Cheatsheet)",
                    HelpAction::Command("cheatsheet"),
                ),
            ],
        ),
        ContextHelpKind::VisualSelection => (
            String::from("Cheatsheet: Visual Selection (視覺連續選取)"),
            vec![
                help_entry(
                    "expand",
                    "j / k / Down / Up",
                    "延伸 / 縮小視覺連續選取範圍",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "連續選取至檔案清單頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy",
                    "y",
                    "複製選取範圍內的所有檔案/資料夾 (Yank)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cut",
                    "x",
                    "剪下選取範圍內的所有檔案/資料夾 (Cut)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d",
                    "批次刪除選取範圍內的所有檔案/資料夾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "rename-regex",
                    "r / R",
                    "對目前選取範圍開啟 Regex 批次改名預覽",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "compress",
                    "C",
                    "將選取範圍內所有項目壓縮為 ZIP",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "commit",
                    "v",
                    "將選取範圍提交為常規標記並退出視覺模式",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消視覺選取模式",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::CommandMode => (
            String::from("Cheatsheet: Command Mode (命令列模式)"),
            vec![
                help_entry(
                    "execute",
                    "Enter",
                    "執行目前輸入之指令",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "complete",
                    "Tab",
                    "自動補全指令名稱或檔案路徑",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "history",
                    "Up / Down",
                    "瀏覽歷史輸入指令",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "start",
                    "Ctrl+a / Home",
                    "移動游標至指令開頭",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "end",
                    "Ctrl+e / End",
                    "移動游標至指令結尾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消並退出命令模式",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::Filter => (
            String::from("Cheatsheet: Filter (檔案即時過濾)"),
            vec![
                help_entry(
                    "confirm",
                    "Enter",
                    "確認鎖定目前過濾條件",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "toggle-fuzzy",
                    "Tab",
                    "切換模糊過濾 (Fuzzy) 與精確過濾 (Exact)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "清除過濾條件並返回完整清單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::Preview => (
            String::from("Cheatsheet: Preview (檔案預覽)"),
            vec![
                help_entry(
                    "scroll",
                    "j / k / Down / Up",
                    "向下 / 向上捲動預覽文字內容",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻滾預覽",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "/",
                    "在預覽內容中搜尋關鍵字",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "match",
                    "n / N",
                    "跳至下一個 / 上一個搜尋匹配項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "exit",
                    "Tab / Esc / q / h",
                    "退出預覽捲動，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::Rename => (
            String::from("Cheatsheet: Rename (重新命名)"),
            vec![
                help_entry(
                    "edit",
                    "Characters",
                    "編輯新檔案或資料夾名稱",
                    HelpAction::QuitHint,
                ),
                help_entry("apply", "Enter", "確認套用重新命名", HelpAction::QuitHint),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消重新命名並返回檔案列表 (Normal 模式)",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::CreateEntry => (
            String::from("Cheatsheet: Create Entry (新增檔案/目錄)"),
            vec![
                help_entry(
                    "name",
                    "Characters",
                    "輸入名稱（以 / 結尾為資料夾，否則為檔案）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "create",
                    "Enter",
                    "確認建立新檔案或目錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消建立並返回檔案列表 (Normal 模式)",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ConfirmAction => (
            String::from("Cheatsheet: Confirm Action (確認操作)"),
            vec![
                help_entry(
                    "confirm",
                    "y / Enter",
                    "確認執行此操作",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "n / Esc / q",
                    "取消操作並返回",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::RegexRename => (
            String::from("Cheatsheet: Regex Batch Rename (批次改名)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動檢視改名預覽項目",
                    HelpAction::QuitHint,
                ),
                help_entry("apply", "Enter", "確認套用批次改名", HelpAction::QuitHint),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消並退出批次改名",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        _ => return None,
    };
    Some(result)
}
