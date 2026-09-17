//! 快速選擇選單（書籤、Zoxide、排序、Linemode、Yank、跳轉、主題、路徑複製、外部開啟）的 Cheatsheet 速查表。

use super::types::{ContextHelpKind, HelpAction, HelpEntry, help_entry};

/// 取得快速彈出選單的快捷鍵指南。
pub(crate) fn cheatsheet_for_picker(kind: ContextHelpKind) -> Option<(String, Vec<HelpEntry>)> {
    let result = match kind {
        ContextHelpKind::BookmarkPicker => (
            String::from("Cheatsheet: Bookmark (書籤管理)"),
            vec![
                help_entry(
                    "add",
                    "a",
                    "自動挑選下一個可用代號，將目前目錄存入書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump-list",
                    "g",
                    "打開書籤清單進行跳轉",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete-list",
                    "d",
                    "打開書籤清單進行刪除",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "clear-all",
                    "D",
                    "直接清空所有已儲存書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "quick-jump",
                    "'{key}",
                    "按單一按鍵直接跳轉至對應代號書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / b",
                    "取消退出書籤選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::BookmarkList => (
            String::from("Cheatsheet: Bookmark List (書籤清單)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選擇書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "f",
                    "即時過濾書籤名稱或路徑",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump",
                    "Enter / l",
                    "跳轉至所選書籤目錄（刪除模式下為刪除）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d / {key}",
                    "刪除對應代號書籤（在刪除模式下）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消退出書籤清單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ZoxideList => (
            String::from("Cheatsheet: Zoxide (歷史目錄跳轉)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選擇常用目錄",
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
                    "filter",
                    "f",
                    "即時過濾歷史目錄路徑關鍵字",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump",
                    "Enter / l",
                    "跳轉進入所選常用目錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消並返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::SortPicker => (
            String::from("Cheatsheet: Sort (檔案排序)"),
            vec![
                help_entry(
                    "by-name",
                    "n",
                    "依檔案名稱排序 (Name)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "by-size",
                    "s",
                    "依檔案大小排序 (Size)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "by-mtime",
                    "m",
                    "依修改時間排序 (Modified Time)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "by-ext",
                    "e",
                    "依副檔名排序 (Extension)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "reverse",
                    "r",
                    "反轉目前排序順序 (Reverse)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / ,",
                    "取消退出排序選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::LineModePicker => (
            String::from("Cheatsheet: Move / Linemode (搬移與欄位顯示)"),
            vec![
                help_entry(
                    "move to path",
                    "m",
                    "開啟 :move 目錄搬移命令列",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "move to panel",
                    "p",
                    "開啟 :move-panel 編號搬移命令列",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "move to pane",
                    "1..9",
                    "直接將選取項目搬移到指定 Panel",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "size",
                    "s",
                    "右側欄位顯示檔案容量 (Size)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "perms",
                    "r",
                    "右側欄位顯示檔案權限 (Permissions)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "btime",
                    "b",
                    "右側欄位顯示建立時間 (Birth Time)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mtime",
                    "t",
                    "右側欄位顯示修改時間 (Modified Time)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "none",
                    "n",
                    "簡潔模式，不顯示右側額外欄位 (None)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消退出選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::YankPicker => (
            String::from("Cheatsheet: Yank / Copy (複製到剪貼簿或視窗)"),
            vec![
                help_entry(
                    "copy to clipboard",
                    "y",
                    "複製選取或標記檔案到內部剪貼簿 (yy)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy to panel",
                    "p",
                    "開啟 :copy-panel 編號複製命令列",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy to pane",
                    "1..9",
                    "直接將選取項目複製到指定 Panel",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消退出複製選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::GoPicker => (
            String::from("Cheatsheet: Quick Jump (快速跳轉)"),
            vec![
                help_entry(
                    "documents",
                    "d",
                    "快速跳轉至 ~/Documents 目錄",
                    HelpAction::Command("goto ~/Documents"),
                ),
                help_entry(
                    "desktop",
                    "k",
                    "快速跳轉至 ~/Desktop 目錄 (desKtop)",
                    HelpAction::Command("goto ~/Desktop"),
                ),
                help_entry(
                    "downloads",
                    "l",
                    "快速跳轉至 ~/Downloads 目錄 (downLoad)",
                    HelpAction::Command("goto ~/Downloads"),
                ),
                help_entry(
                    "home",
                    "h",
                    "快速跳轉至使用者家目錄 ~",
                    HelpAction::Command("goto ~"),
                ),
                help_entry(
                    "goto-path",
                    "t",
                    "開啟路徑跳轉輸入框 (Goto path)",
                    HelpAction::Command("goto "),
                ),
                help_entry(
                    "cancel",
                    "Esc / q / g",
                    "取消退出跳轉選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ThemePicker => (
            String::from("Cheatsheet: Theme (佈景主題)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動即時預覽佈景主題",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "apply",
                    "Enter / l",
                    "套用所選主題並儲存為預設",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消並恢復原主題",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::CopyPicker => (
            String::from("Cheatsheet: Copy Path (複製路徑)"),
            vec![
                help_entry(
                    "full-path",
                    "1",
                    "複製完整絕對路徑到剪貼簿",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filename",
                    "2",
                    "僅複製檔案名稱到剪貼簿",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "parent-dir",
                    "3",
                    "複製所在目錄路徑到剪貼簿",
                    HelpAction::QuitHint,
                ),
                help_entry("cancel", "Esc / c", "取消複製選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::OpenPicker => (
            String::from("Cheatsheet: Open With (開啟應用程式)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選擇應用程式",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "open",
                    "Enter",
                    "使用所選應用程式開啟檔案",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消並返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        _ => return None,
    };
    Some(result)
}
