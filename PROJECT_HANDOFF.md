# Project Handoff

這份文件是 `PaneFM` 之後的單一交接文件。

之後如果開新對話，要先讀這份文件，再開始開發。
它的用途是把以下幾種資訊集中在同一個地方：

- 專案目標
- 不可破壞的設計規則
- 目前已完成功能
- 重要快捷鍵與操作模型
- 設定檔 / 使用者資料檔
- 雙平台狀態
- 最近開發紀錄
- 已知問題與下一步建議

如果這份文件和其他舊文件有衝突，以這份文件為主。

## 1. 專案定位

`PaneFM` 是一個以個人高效率操作為核心的 terminal 檔案管理器。

### 1.1 專案名稱、發佈與圖示決策

- 正式專案名稱與 Cargo 套件名稱為 `PaneFM` / `panefm`。
- PaneFM 定位為透過 Terminal 執行的 CLI/TUI 工具，未來預計提供 Homebrew 與 Windows 套件管理器安裝。
- CLI 套件不把圖示嵌入 macOS 或 Windows 執行檔，避免增加沒有實際用途的建置流程與相依套件。
- `assets/panefm-icon.svg` 與 `assets/panefm-icon.png` 保留給 GitHub、README 與 Release 品牌顯示。
- 只有未來真的製作 macOS `.app`、Windows 桌面捷徑或圖形化安裝程式時，才另外產生並使用 `.icns` / `.ico`。
- Homebrew 與 Windows 套件管理器尚未發布；必須先建立公開 GitHub 倉庫與 Release，再加入對應 formula / manifest。

主要設計方向：

- 參考 `mature-reference`
- 操作核心以 `Vim` 習慣為主
- 正式支援 `macOS` 與 `Windows`
- `Linux / Unix` 目前不是正式支援目標，但架構要保留未來擴充空間

這個專案不是玩具原型，而是要逐步變成日常可用工具，所以：

- 操作一致性比炫技重要
- 體感速度比做出一堆功能重要
- panel 邏輯、terminal 相容性、跨平台設計都必須從一開始就顧好

## 2. 新對話接手方式

之後如果使用新對話，應先做這些事：

1. 先讀這份文件。
2. 再看 `git status`，確認工作樹狀態。
3. 若要開發功能，先確認它有沒有違反這份文件裡的規則。
4. 若要修 bug，一定要補對應測試。
5. 完成功能後，要更新這份文件的「開發紀錄」與「目前狀態」。

建議新對話第一句直接說：

```text
先讀 PROJECT_HANDOFF.md，再接著目前進度繼續開發
```

## 3. 不可破壞的核心規則

### 3.1 Panel 是第一級物件

- `panel` 是第一級物件，不是附屬概念。
- 每個 panel 都要有自己的狀態。
- 一個 panel 的暫時 UI 或模式，不可污染其他 panel。
- 除非明確指定為全域功能，否則所有 UI 一律畫在目前 focus 的 panel 內。

### 3.2 預設是 panel-local，不是 global

以下介面預設都應該是 panel-local：

- help
- bookmark
- command UI
- filter
- rename
- open-with picker
- trash
- task
- search
- find
- preview search
- zoxide list
- linemode picker

### 3.3 Vim 風格是核心

只要本質上是「可移動游標的列表 / 面板 / 選單」，預設要支援：

- `j / k`：上下移動
- `h`：離開、返回、上一層、取消
- `l`：進入、確認、下一層、執行
- `gg / G`：跳到頂部 / 底部
- `Ctrl-u / Ctrl-d`：半頁上下移動
- `Ctrl-b / Ctrl-f`：整頁上下移動
- `J / K`：固定大步長移動

如果某個介面不能支援這套規則，必須有明確理由，不可以只是為了省事。

### 3.4 `Esc` 規則要一致

- `Esc` 優先代表取消、退出輸入、離開暫時模式、返回上一層。
- 若某模式是兩段式離開，狀態列要明確說明。
- 若某個下方面板是「按同一鍵再按一次可關閉」，就要一致支援。

### 3.5 Terminal 相容性不能寫死

這個專案不能只對單一 terminal 正常。

規則：

- 所有快捷鍵判斷都應透過統一的 key normalization 流程
- 不可假設每個 terminal 對 `Shift` / `Ctrl` / 符號鍵的回報完全相同
- 一般文字輸入和快捷鍵判斷要分開
- 新功能若新增快捷鍵，必須檢查是否有納入正規化路徑

### 3.6 外部互動程式要正確進出 TUI

任何會暫時離開 TUI 的功能，例如：

- `fzf`
- `vim`
- `$EDITOR`
- 外部 open / reveal
- 未來 terminal 型 plugin

都必須遵守：

1. 關閉 raw mode
2. 還原 cursor style
3. 關閉 keyboard enhancement
4. 離開 alternate screen
5. 執行外部程式
6. 回到 alternate screen
7. 恢復 keyboard enhancement
8. 恢復 raw mode
9. 清畫面並恢復可互動狀態

### 3.7 雙平台是正式要求

- `macOS` 和 `Windows` 都要可用
- 平台差異要集中在抽象層，不要散落業務邏輯
- 外部命令、路徑、open / reveal / SMB / archive 都要先考慮兩平台
- 沒有 Windows 真機時，也要把架構先做成可分流、可替換、可補測

### 3.8 可調整的東西不要寫死

容易常改的東西必須走設定或主題：

- 顏色
- 面板尺寸
- 快速移動步長
- page step
- 預設排序
- preview 尺寸
- 未來更多 UI 行為

### 3.9 每個功能都要有測試

硬性規則：

- 每個新功能都要有測試
- 每個修過的 bug 都要補回歸測試
- 不能只修眼前問題而不建立保護

### 3.10 註解要用繁體中文，而且要能看懂

重要函式、型別、流程都要有繁體中文註解，至少要說明：

- 這個函式 / 類型做什麼
- 參數是什麼
- 回傳值是什麼
- 特殊模式或副作用是什麼

## 4. 目前專案的重要檔案

- `/Users/otto/Documents/terminal-file-manager/src`
  核心程式碼
- `/Users/otto/Documents/terminal-file-manager/config.toml.example`
  設定檔範例
- `/Users/otto/Documents/terminal-file-manager/plugins.toml.example`
  plugin / open-with 設定範例
- `/Users/otto/Documents/terminal-file-manager/plugins.toml`
  本機 plugin 設定
- `/Users/otto/Documents/terminal-file-manager/bookmark.toml`
  使用者書籤資料
- `fzf`、`rg`、`zoxide` 必須由使用者安裝並放在系統 `PATH` 中；程式不再攜帶第三方 binary。

## 5. 目前設定檔與資料檔規則

### 5.1 `config.toml`

只放設定，不放使用者資料。

目前重點用途：

- theme
- UI 尺寸
- search 行為
- navigation step
- pane 預設排序

### 5.2 `bookmark.toml`

放使用者書籤資料。

規則：

- 書籤有變動就同步寫檔
- 不應混進 `config.toml`
- 可保存本機路徑與 `smb://...` 目標

### 5.3 `plugins.toml`

放自訂 `open with` 動作，不要混進 `config.toml`。

目前是設定檔驅動，不是動態程式碼插件。

用途範例：

- VS Code 開目錄
- Xcode 開目錄
- Git / SVN 指令
- 自訂 reveal / external tool

## 6. 目前已完成的功能總表

以下是到目前為止，已經做過且應視為存在中的功能。

### 6.1 基本列表 / pane

- 多 panel 分割
- panel 關閉
- panel only
- panel focus 切換
- panel 有固定編號
- 可直接用數字跳 panel
- `:pane <id>` 指令切換 panel
- 離開子目錄回上一層時，游標會回到原本離開的目錄

### 6.2 Vim 式列表移動

- `h j k l`
- `gg / G`
- `J / K`
- `Ctrl-u / Ctrl-d`
- `Ctrl-b / Ctrl-f`
- 方向鍵已映射進核心移動模型

### 6.3 排序 / 顯示

- `,` 打開 sort 面板
- 支援多種排序模式
  - modified
  - birth time
  - extension
  - alphabetical
  - natural
  - size
  - random
- 排序面板是底部面板式 UI
- 右側欄位會依排序模式顯示對應資訊
- `m` 打開 linemode 面板
- linemode 目前支援：
  - size
  - permissions
  - btime
  - mtime
  - none
- linemode 不會改排序，只會改右側顯示內容

### 6.4 主題 / 配色

- 有 theme 架構，現在使用 `ratatui-themes` 提供的成熟色盤
- 預設主題是 `catppuccin-mocha`
- 主題選擇器目前支援 Dracula、Nord、Catppuccin、Gruvbox、Tokyo Night、Solarized、Everforest 等 16 種色盤
- `default / forest / ocean` 舊設定名稱仍保留相容映射，分別對應 Catppuccin Mocha、Everforest、Nord
- 所有色盤都會先轉成專案自己的語意化 `Theme`，UI 不應直接依賴外部色盤欄位
- 已支援切換主題
- 和 search / preview highlight 有關的顏色，也應定義在 theme
- `Theme` 已提供 directory、executable、image、archive、source 等檔案類型語意色
- 列表會依目錄、常見副檔名與跨平台可取得的 executable 資訊套用類型色
- `config.toml` 的 `[ui.icons].enabled` 控制列表圖示，預設為 `true`
- `[ui.icons].style = "nerd-font"` 使用接近 mature-reference 的文字圖示；`"ascii"` 是不依賴特殊字型的 fallback
- Windows 或 macOS 若 Terminal 沒有 Nerd Font，應改用 `style = "ascii"`，避免圖示變成方框
- 字體實際由 Windows Terminal、iTerm2 或其他外部 Terminal 控制，TUI 不送出不跨平台的改字體控制碼

### 6.5 檔案操作

- copy
- cut
- paste
- paste overwrite
- 同目錄複製時自動處理重名
- delete to trash
- permanent delete
- create file / directory
- 支援巢狀路徑建立，例如 `test/gg.txt`
- rename inline UI
- rename 保留 `:rename`
- `r` 直接 rename
- rename normal / insert mode
- rename 已支援部分 Vim 操作：
  - `i`
  - `a`
  - `A`
  - `w`
  - `b`
  - `e`
  - `Esc`
- Regex 批次改名使用 `R` 或 `:rename-regex <pattern> <replace>`；短別名為 `:reg <pattern> <replace>`。
- Regex 預覽中只有標示 `ready` 的項目會套用；`unchanged` 代表規則沒有改變名稱，`conflict` 或 `invalid` 存在時，Enter 會拒絕整批套用。
- 預覽列表按 `Enter` 或 `l` 才會執行實際改名，`Esc` / `h` / `q` 取消。

### 6.6 視覺標記 / 批次操作

- `V` 進入視覺標記
- 可分段標記多個範圍
- 離開本次選取後可繼續補選
- 若畫面上已有標記，`Esc` 可一次清空全部標記
- 多數批次操作已能吃視覺標記結果
- trash panel 中 `u / d` 已修正為：
  如果存在標記，會處理所有標記項目，不只游標所在項目

### 6.7 Trash

- `d`：移到 internal trash
- `D`：永久刪除
- trash 為 panel-local 列表
- trash 支援搜尋
- trash 支援 `V` 視覺標記
- trash 列表中支援：
  - `d`：刪除單個或標記項目
  - `D`：刪除全部或整批刪除
  - `u`：還原單個或標記項目
  - `U`：全部還原
- trash 操作會顯示確認視窗
- trash undo 已有命令：
  - `:trash undo`

### 6.8 Bookmark

- 書籤資料寫入 `bookmark.toml`
- `b` 會打開書籤功能面板
- 書籤目前操作模型：
  - `s`：save bookmark，自動分配代號
  - `g`：打開書籤列表並跳轉
  - `d`：刪除單一書籤
  - `D`：刪除全部書籤
- 書籤列表可：
  - `j / k` 移動
  - `Enter` 跳轉
  - `Esc` 離開
  - `f` filter
- 書籤支援 `smb://...` 目標

### 6.9 Help / Task / 下方功能面板

- `F1` 開 help
- `~` 也可直接開 help
- help 是 panel-local
- help 會取代 panel 內容，不再額外開全域面板
- help 支援：
  - `j / k`
  - `Ctrl-d / Ctrl-u`
  - `Ctrl-b / Ctrl-f`
  - `f` 搜尋
  - `Enter` 執行目前命令
- `t` / `T` task 相關快捷鍵已存在過，實際顯示要以 help 為準
- 多鍵 leader 類功能面板目前包含：
  - `,`
  - `m`
  - `b`
  - `g`
  - `w`
- 這些底部面板現在只顯示第二鍵，不重複顯示第一鍵
- 同一個快捷鍵再次按下，可關閉對應面板的功能已套用到多個面板

### 6.10 Filter / Find / Preview Search

- `f`：列表 filter
- filter 是即時過濾
- `Esc` 是兩段式離開
- `/`：find next 類型搜尋
- 列表可即時反白符合字串
- `Enter` 後保留搜尋結果
- `n / N` 可在 find 結果中前後移動
- preview 內也支援 find / search navigation
- preview highlight 規則已做過多輪調整：
  - 符合字串要清楚可見
  - 目前 focus 的 match 要更明顯
  - 目前 focus 行要有整行背景
- preview 搜尋應只針對真正檔案內容，不應把 preview metadata 當成搜尋內容

### 6.11 Preview

- preview 現在是各 panel 自己的狀態，不是全域共用
- preview 可放大顯示
- search 結果與一般 preview 都能共用 preview 顯示能力
- 曾做過「列表 / preview 高度交換」方向調整
- 使用者目前偏好是：preview 不要預設佔很高空間

### 6.12 Open / Open With

- `Enter` / `o`：依類型開啟
- `Shift+O` / `Shift+Enter`：開啟 open-with 小型選單
- 檔案與目錄的 open-with 項目不同
- 目前支援的概念包含：
  - `$EDITOR`
  - `Vim`
  - `Open`
  - `Reveal`
- 自訂 open-with 動作可由 `plugins.toml` 提供

### 6.13 Search

- `S`：全域內容搜尋
- 目前已整合 `rg`
- 搜尋結果會顯示在主列表，不額外在下方再塞一份列表
- 搜尋啟動後有 loading 狀態
- 搜尋支援背景分批回傳結果
- 離開搜尋時，背景 task 應可取消
- 搜尋結果列表目前維持簡潔，只顯示檔案路徑
- 游標停在某個結果時，下方 preview 會顯示該檔案第一個命中內容
- `P` 可打開較大的 preview 繼續查看命中內容

### 6.14 FZF Jump

- `z` 會打開內建 `fzf`
- 搜尋範圍是目前 panel root 之下的遞迴檔案與目錄
- 已修正過 `Esc` 無法返回、第一次快第二次慢、TTY 互動異常等問題
- 目前整合方式改成讓 `fzf` 自己透過命令產生候選，而不是直接把候選資料硬 pipe 到 `stdin`
- `fzf` 使用系統 `PATH`；未安裝時會顯示缺少依賴提示

### 6.15 Zoxide

- 已改用 `zoxide` 取代舊的 recent 方向
- `Z` / `:zoxide` 可打開 zoxide 列表
- zoxide 資料庫會學習 pane 導航後的目錄
- zoxide 列表支援 `f` filter
- `zoxide` 使用系統 `PATH`；未安裝時會顯示缺少依賴提示

### 6.16 Goto

- `goto <path>` 取代舊的 `cd <path>`
- `g` 目前是 leader 面板
- 已存在：
  - `:goto <path>`
  - `:goto document`
  - `:goto desktop`
- `g` 面板中目前應能進入這類跳轉命令

### 6.17 Archive

- 已加入跨平台方向的 `:compress`
- 已加入跨平台方向的 `:extract`
- 目前支援：
  - `zip`
  - `tar`
  - `tar.gz`
  - `gz`
- 壓縮 / 解壓縮已考慮重名衝突處理

### 6.18 SMB

- 已做 `smb://host/share[/path]` 方向支援
- 書籤可保存 SMB 目標
- 跳 SMB 書籤時會自動走 SMB 連線流程
- Windows 方向以 UNC 路徑為設計基礎

## 7. 目前重要快捷鍵模型

這一段只記「操作模型」，不是完整 help 清單。
完整命令應以程式內 F1 help 為準。

### 7.1 單鍵核心

- `h j k l`：核心導航
- `J / K`：大步長移動
- `gg / G`：頂部 / 底部
- `f`：filter 或列表內搜尋入口
- `/`：find
- `n / N`：find 或 preview 搜尋前後跳轉
- `r`：rename
- `a`：create
- `d`：trash
- `D`：permanent delete
- `v`：進入 visual mark，使用 `j / k` 擴大選取範圍，再按 `v` 或 `Esc` 結束；目前仍相容 `V`
- `p`：paste
- `P`：較強的 paste / overwrite，或在某些搜尋情境下打開大 preview
- `z`：fzf jump
- `Z`：zoxide
- `b`：bookmark leader
- `m`：linemode leader
- `,`：sort leader
- `g`：goto leader
- `w`：window / panel leader
- `F1` / `~`：help

### 7.2 多鍵 leader 面板

- `,`：sort
- `m`：linemode
- `b`：bookmark
- `g`：goto
- `w`：panel / window

規則：

- 第一鍵打開底部面板
- 第二鍵執行功能
- 同一鍵再次按下可關閉
- 面板說明要同時寫出離開鍵，例如 `Esc` 與對應 leader 本身

## 8. 雙平台現況

### 8.1 目前設計目標

- 正式支援 `macOS`
- 正式支援 `Windows`

### 8.2 已完成的跨平台整理

- open / reveal 走平台分流
- SMB 方向已考慮 Windows UNC
- archive 功能採跨平台設計方向
- `fzf`、`rg`、`zoxide` 完全使用系統 `PATH` 中的命令，未安裝時啟動狀態會列出缺少項目
- 缺少任一工具時，使用相關功能會在目前 focus panel 顯示依賴列表；`j/k` 移動、`Esc` 關閉
- `vendor` 已不再是編譯或執行必要條件，刪除後可直接縮小原始碼與執行檔體積

### 8.3 還需要的確認

最缺的不是設計，而是 Windows 真機驗證。

到 Windows 真機時，優先驗證：

- 基本 TUI 啟動
- `hjkl` / `Esc` / `Enter`
- open / reveal
- `Shift+O` / `Shift+Enter`
- `fzf` 跳轉
- `zoxide` 學習與跳轉
- `rg` 搜尋
- SMB
- 壓縮 / 解壓縮

## 9. 目前工作樹狀態

整理這份文件時，工作樹預期狀態是：

- 程式碼應該乾淨
- `bookmark.toml` 可能是唯一保留的使用者資料變更

`bookmark.toml` 是使用者本機資料，不應隨便當成功能改動 commit。

目前若正在進行 theme 整合，還可能有 `Cargo.toml`、`Cargo.lock`、`src/theme.rs`、`src/config.rs`、`src/file_manager/app.rs`、`config.toml.example` 與本文件的未提交變更；提交時不要包含 `bookmark.toml`。

## 10. 最近開發紀錄

以下只記目前接手最重要的幾段。

### 2026-08-21

- theme 改用 `ratatui-themes` 的成熟色盤，並將 Ratatui 升級到 `0.30` 以統一 Color 型別
- 預設主題改為 `catppuccin-mocha`
- 主題選擇器擴充為 16 種外部色盤，仍透過專案自己的語意化 `Theme` 提供給 UI
- 保留 `default / forest / ocean` 設定名稱的相容映射
- 主題整合與相容性測試完成，完整測試目前為 283 個通過
- 建立根目錄 `config.toml`，作為目前實際使用的本機設定檔
- 列表加入目錄、程式碼、圖片、壓縮檔、可執行檔與一般檔案的語意色與文字圖示
- 圖示改為 `nerd-font` / `ascii` 兩種可設定風格，不使用圖片或 Emoji
- 新增 `[ui.icons].enabled` 與 `[ui.icons].style` 設定及跨平台檔案分類回歸測試，完整測試目前為 284 個通過
- 主題選擇器預設高度調整為 20，支援 `j/k` 與 `Ctrl-d/u` 滾動，開啟時會定位目前使用中的主題
- 主題選擇器滾動與目前主題定位回歸測試完成，完整測試目前為 287 個通過
- 主題套用後會同步寫回目前載入的 `config.toml`，只更新 `[ui].theme`，保留其他設定與註解；沒有設定檔時會建立最小設定檔
- `t` 改為 `t` 系列快捷鍵面板：`tl` 開啟標題為 `Theme List` 的主題列表、`tn` 切換下一個主題、`tt` 開啟 Trash 操作
- `theme list` 與 `theme next` 已加入命令與 Help，主題保存與 `t` 系列操作回歸測試完成，完整測試目前為 292 個通過
- `tt` 已改為兩次按鍵直接進入 Trash 列表，`tu` 直接還原最近一次刪除；`t` 面板依序顯示 theme 與 trash 的全部快捷鍵
- Trash 不是新增 split panel，而是目前焦點 panel 的 `TrashPanel` 模式：它暫時替換該 panel 的檔案列表區域，離開後恢復原本列表

### 10.1 UI 類型判斷

- **替換目前列表的 panel mode**：Trash、Tasks、Help、Regex Rename、Global Search 結果；它們保留目前 panel 的位置與焦點，只替換列表內容。Trash 目前就是這一類，不是中央 popup。
- **列表內容過濾或標記模式**：Filter、List Find、Visual Selection、Search/Preview Search；它們仍在原本列表或 preview 上工作，不建立新的 panel。
- **panel 內的中央 popup**：Bookmark List 與 Zoxide List。它們只在目前 focus 的 panel 矩形範圍內繪製外框、清除 popup 區域，底下的原列表仍保留，關閉後即可回到原列表。
- **目前 panel 內的底部快捷鍵面板**：`t`、`g`、`w`、`,`、`m`、`b`；這些是選擇下一個按鍵的暫時提示，不是 split panel。
- **popup / dialog**：Theme List、Command UI、Open Picker、Copy Picker、確認視窗、Rename/Create 輸入框；它們是暫時覆蓋畫面的互動視窗，完成或取消後回到原模式。Trash 的刪除/還原確認視窗屬於這一類，但 Trash 列表本身仍是替換列表模式。
- **真正新增或調整 split panel**：只有 `w` 系列與 `:split`、`:vsplit` 等 layout 操作會改變 panel 數量或位置。

除非未來明確指定為全域功能，以上 UI 都必須綁定目前 focus 的 panel；「替換列表」不代表建立新的 panel。

### 2026-08-15

- command palette 補上提示與補齊
- 修正不同 terminal 下 `Ctrl-n / Ctrl-p` 被吃成大寫字母的問題
- 開始整理跨平台 open / reveal / SMB / archive 結構
- `bookmark.toml` 開始承載書籤資料，不再塞進 config

### 2026-08-16

- `fzf` 遞迴 jump 效能與 TTY 互動修正
- `Esc` 返回問題修正
- `z` 會搜尋目前 panel root 下全部檔案與目錄
- 整理 Windows 測試方向與外部開啟流程

### 2026-08-17 到 2026-08-19 期間的重要方向

- help / task / bookmark / sort / linemode / goto 等 leader 面板逐步統一
- `goto <path>` 取代 `cd <path>`
- 加入 linemode
- 書籤操作改成 `b` 功能面板
- direct pane switching、page navigation、preview / paste 行為持續調整
- help 說明文字與快捷鍵逐步對齊真實行為

### 2026-08-20 最近完成的重點

- 舊版曾有 bundled zoxide；目前已移除，改為只使用系統 `PATH`
- 缺少 `fzf`、`rg` 或 `zoxide` 時會開啟依賴狀態列表
- 修正 trash panel 中 `V` 多選後，`u / d` 只處理游標項目的 bug
- 目前 `u / d` 若存在標記，會批次處理標記項目

### 最近 commit

- `2bb69cd` `Add bundled zoxide support`
- `d1ecb05` `fix trash panel batch actions`
- `c8a47f3` `Refine go panel and command shortcuts`
- `bdf4b96` `Refine goto and regex rename shortcuts`
- `74a03ba` `Refine shortcut panels and toggles`
- `ed64004` `Add help and task shortcuts`
- `82b7579` `Redesign bookmark actions and management`
- `66ff0af` `Add linemode picker and update bookmark keys`
- `0869659` `Add direct pane switching and page navigation`

## 11. 目前已知問題 / 待優化

### 11.1 `S` 全域搜尋體感仍不夠快

雖然目前已做：

- `rg`
- loading
- 背景 task
- 分批回傳
- 可取消

但使用者體感仍認為和 `mature-reference` 有差距，這一塊仍屬高優先度效能工程。

### 11.2 列表內快速定位仍可更快

雖然已經有：

- filter
- find
- fzf jump
- zoxide
- `J / K`
- page move

但使用者仍在意「快速找到目錄內某個目標」的體感，這依然是核心優化方向。

### 11.3 Windows 真機驗證不足

架構雖然有先整理，但還需要真機驗證把不一致行為清掉。

### 11.4 help 要持續和真實快捷鍵同步

使用者很在意 help 必須完整、不能用猜的。

規則：

- 多鍵快捷鍵要完整展開
- 不要故意省略第二鍵
- 若命令改名，help 要同步更新
- 若 UI 行為改了，help 文字也要同步更新

## 12. 下一步建議優先順序

### 第一優先

- 繼續優化 `S` 搜尋體感
- 讓搜尋結果判讀與跳轉更快
- 持續確認 task 可取消、結果可快速進 preview

### 第二優先

- Windows 真機驗證
- 補齊跨平台 bug 與對應測試

### 第三優先

- `plugins.toml` 驅動的自訂動作系統繼續擴充
- 例如 VS Code、Git、SVN、公司內部工具等 workflow

### 第四優先

- 更完整的 visual mode 與批次操作一致性
- 更多列表 / 面板操作規則統一

## 13. 之後開發時一定要遵守的流程

1. 先讀這份文件。
2. 確認這次改動屬於哪個功能區。
3. 先檢查是否有既有模組可復用，不要重複造輪子。
4. 如果是 bug，先找可補的回歸測試。
5. 實作後跑相關測試。
6. 必要時更新 help。
7. 必要時更新這份文件。
8. 若功能完成一段，可整理 commit。

## 14. 建議之後以這份文件為唯一入口

舊文件目前仍存在：

- `/Users/otto/Documents/terminal-file-manager/README.md`
- `/Users/otto/Documents/terminal-file-manager/DEVELOPMENT_GUIDELINES.md`
- `/Users/otto/Documents/terminal-file-manager/DEV_NOTES.md`

但之後新對話接手、功能續做、開發記錄更新，都應優先以這份文件為主。
