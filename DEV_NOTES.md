# 開發接續筆記

這份筆記用來記錄目前 `terminal-file-manager` 的互動規則、已完成功能與後續建議，讓之後繼續開發時可以快速接上。

## 目前已完成的重點

- 支援 `config.toml` 外部設定檔讀取。
- `config.toml` 已改成分區式結構，可控制 UI、pane、search、behavior 設定。
- 支援主題切換與 theme 架構整理。
- 重新命名採用接近 mature-reference 的 inline 視窗。
- rename 已支援較完整的 Vim 操作：
  `Esc` 切到 normal mode、`i`/`a`/`A` 進入 insert mode、`w`/`b`/`e` 移動游標。
- 複製、剪下、貼上已支援批次操作。
- 同目錄複製時會自動處理重名。
- `a` 可建立檔案或資料夾。
- 若輸入像 `test/gg.txt`，會自動建立中間目錄與檔案。
- `f` 可開啟 filter，輸入時即時過濾。
- filter 的 `Esc` 流程是兩段式：
  第一次離開輸入，第二次離開 filter 畫面。
- `,` 可開啟排序面板，並以單鍵切換排序方式。
- 右側排序資訊會依排序模式顯示對應內容，例如時間、附檔名、大小。
- preview 已支援 focus mode，且可與列表區塊交換比例，讓預覽放大。
- 預覽中已支援搜尋。
- pane 已支援分割與關閉。
- preview 現在已改成 pane 各自獨立的狀態，不再是全域共享。
- `d` 現在會先把項目移到 internal trash，而不是直接永久刪除。
- `:restore` 可還原最近一次移到 trash 的項目。
- trash 面板已支援 pane-local 顯示與搜尋，不會再用全域 popup 蓋住畫面。
- F1 help 面板已改成 pane-local 列表模式，支援搜尋、`Ctrl-d` / `Ctrl-u` 捲動、`Enter` 直接執行對應功能。
- 已加入第一版 global search：
  `s` 先打開輸入框，按下 `Enter` 後背景搜尋，主列表會切成 loading / 搜尋結果。

## 目前的 `V` 視覺標記規則

- 按下 `V` 會進入視覺標記模式。
- 在視覺標記模式中，用 `j/k`、`gg/G` 移動游標來選取一段範圍。
- 按下 `V` 或 `Esc`，都會結束這次範圍選取，並把這段加入已標記項目。
- 離開本次選取後，可以移動到別處，再按一次 `V` 繼續補選新的範圍。
- 如果目前不在視覺標記模式，但畫面上還保留已標記項目，按下 `Esc` 會一次清掉全部標記。
- 列表左側的標記欄位目前規則：
  沒有任何標記時不顯示額外縮排；只要有任一項被標記，欄位寬度就固定保留，避免列表跳動。

## 這次最後補上的行為

- 進入子目錄後再回上一層，游標會回到剛剛離開的那個目錄，不會跳回第一列。
- 在 preview 中可用 `Ctrl-w` 切換 pane。
- preview 狀態只屬於原本那個 pane；切到其他 pane 不會被強制進入 preview。
- global search 一開始只顯示輸入框，主列表維持原樣。
- global search 真正開始搜尋後，主列表才切成 `Loading search results...` 與搜尋結果。
- global search 已支援背景分批回傳結果，且可在離開搜尋時取消背景工作。
- help 面板中的命令定義已重新對齊實際行為：
  `d` 對應 `:delete`，`:trash` 沒有快捷鍵時就不顯示假的快捷鍵。
- help 面板 `Enter` 已驗證會真的切到對應模式，不再只是顯示項目名稱。

## 2026-08-15 最新進度

### Command / 鍵盤輸入

- command palette 已支援命令提示與自動補齊。
- 原本 `Ctrl-n` / `Ctrl-p` 在部分 terminal 會被吃成大寫字母，已改成更穩定的按鍵正規化處理。
- 目前 command palette 可使用：
  `Shift-n` / `Shift-p`、`Ctrl-n` / `Ctrl-p`、`Tab` / `BackTab`、`Up` / `Down` 來切換提示項目。
- `Enter` 在 command palette 中已修正：
  如果使用者已經輸入命令參數，不會再把提示範例覆蓋回輸入框。
- 像 `connect` 這種需要參數的命令，提示顯示與實際補齊內容已拆開：
  畫面上仍可顯示範例，但補齊時只會插入命令前綴，不會硬塞測試用參數。

### 壓縮 / 解壓縮

- 已加入跨平台方向的 `:compress` / `:extract` 第一版。
- 目前支援：
  `zip`、`tar`、`tar.gz`、`gz`。
- 壓縮與解壓縮都有處理同名衝突，避免直接覆蓋原檔。
- 這一塊核心邏輯主要走 Rust crate，本身比直接依賴平台 shell 指令更容易維持跨平台。

### SMB / 書籤

- 已加入 `:connect smb://host/share[/path]`。
- macOS / Linux 目前的設計是：
  先檢查本機掛載點，再視情況請求系統掛載。
- Windows 目前已加入 UNC 路徑方向的解析邏輯：
  `smb://host/share/path` 會轉成 `\\\\host\\share\\path`。
- `connect` 的錯誤訊息已改好，不再只是生硬的 `usage`。
  如果只輸入 IP、少了 share 名稱，現在會顯示明確的格式錯誤說明。
- 已支援把 SMB 位置存進 `bookmark.toml`。
- 書籤現在不只支援本機路徑，也支援 `smb://...` 目標。
- 在 SMB 位置下存書籤時，會寫入原始 `smb://...`，不是只存 `/Volumes/...` 這種掛載後路徑。
- 之後使用：
  `bookmark jump <key>` 或書籤列表按 `Enter`，
  都會自動判斷是本機還是 SMB，必要時自動走 SMB 連線流程。
- pane 現在會記住自己的書籤來源目標；
  如果目前是在 SMB share 內繼續進出子目錄，書籤目標也會同步更新，不會很快退回成本機掛載路徑。

### 測試狀態

- 今天整理後，本機測試結果為：
  `cargo test` => `134 passed`
- 另外有針對今天新增的 SMB / 書籤流程補測試：
  - SMB 書籤會正確寫進 `bookmark.toml`
  - SMB 連線成功後存書籤不會誤存成本機掛載路徑
  - 跳 SMB 書籤會自動進入對應目標
- 外部開啟平台抽象也已補測試：
  - Windows `Open`
  - Windows `Reveal`
  - macOS `Reveal`
  - command line 解析

### 平台抽象

- 目前正式支援目標以 `macOS + Windows` 為主。
- 平台相關命令已抽到：
  [src/file_manager/platform.rs](/Users/otto/Documents/terminal-file-manager/src/file_manager/platform.rs)
- [src/file_manager/open.rs](/Users/otto/Documents/terminal-file-manager/src/file_manager/open.rs)
  現在只負責使用者意圖與命令組裝，不再自己混寫平台細節。
- 目前結構設計已保留 `LinuxLike` 擴充點，但不把 Unix / Linux 視為當前正式支援目標。

## Windows 目前的狀態

- 核心檔案操作、pane、filter、search、bookmark、rename、壓縮解壓縮都已朝跨平台方向整理。
- 外部開啟流程已改成平台分流：
  - Windows `Open`：`cmd.exe /C start`
  - Windows `Reveal`：`explorer.exe /select,...`
  - macOS `Open`：`open`
  - macOS `Reveal`：`open -R`
- `$EDITOR` 與 `Vim` 已不再綁死 `/bin/sh`，改成先解析命令，再直接執行。
- 目前真正還缺的是「Windows 真機驗證」，不是結構設計。

## 2026-08-16 `fzf` 效能與互動修正

- `z` 目前已改成使用外部 `fzf` 做遞迴 jump，且搜尋範圍會展開目前目錄下的所有子目錄與檔案。
- `fzf` 整合方式已改成：
  不是把候選資料直接 pipe 進 `fzf stdin`，而是讓 `fzf` 自己透過 `FZF_DEFAULT_COMMAND` 啟動內部 helper command 產生候選。
- 這樣做的原因：
  - `fzf` 可以保有自己的 TTY 輸入，`Esc` / `Enter` 互動比較穩定。
  - 不容易再出現第一次快、第二次慢，或 `Esc` 無法返回 TUI 的問題。
- `fzf` 的核心互動已固定指定：
  - `Esc` => abort
  - `Enter` => accept
- 目前已知體感明顯比前一版快，使用者實測回報「現在快很多了」。

### `fzf` 除錯方式

- 若之後又出現某些路徑特別慢，可以先開啟：
  - `TFM_DEBUG_TIMING=1 cargo run`
- 目前會輸出到 `stderr` 的 timing 重點：
  - `fzf wait`
  - `fzf jump total`
  - `jump go_to_path`
- 先看哪一段慢，再決定要優化：
  - `fzf` 候選生成
  - pane `reload`
  - preview

### 明天到公司要驗證的項目

1. 基本啟動
   - Windows 上能正常啟動 TUI
   - `hjkl`、`Enter`、`Esc`、`;`、`F1` 可正常使用
2. 本機開啟流程
   - 文字檔按 `Enter`
   - 文字檔 `Shift+O` 選 `$EDITOR`
   - 文字檔 `Shift+O` 選 `Vim`
   - 任意檔案 `Shift+O` 選 `Reveal`
   - 資料夾按 `Enter`
   - 資料夾 `Shift+O` 選 `Open`
   - 資料夾 `Shift+O` 選 `Reveal`
3. 外部程式返回 TUI
   - 離開 editor / vim 後畫面是否正常回來
   - 是否會黑畫面或卡死
4. SMB
   - `connect smb://host/share`
   - `connect smb://host/share/path`
   - 成功後可否瀏覽
   - 成功後可否存書籤
   - `bookmark jump <key>` 是否會自動回到 SMB
5. 錯誤訊息
   - share 不存在
   - 權限不足
   - 網路不通
   - editor / vim 未安裝

## 明天接手時的建議順序

1. 先跑一次 `cargo test`，確認今天的基線還是乾淨的。
2. 到公司先做 Windows 真機測試，不要先急著改碼。
3. 把真機測到的 bug 依序記錄成：
   操作步驟、實際結果、預期結果。
4. 若 SMB 在 Windows 實測有問題，再針對 UNC 路徑與 pane 導航補保護。
5. Windows 基本可用後，下一個大方向就是列表導航與快速定位效能優化。

## 已知待優化問題

- global search 在超大目錄中的體感速度仍明顯落後 mature-reference。
- 目前已改善成：
  按下 `s` 不會立刻掃描、搜尋改成背景執行、主列表有 loading 提示、結果會分批灌入。
- 但仍未完成：
  真正接近 mature-reference 的大型目錄掃描效率、搜尋 task 調度優化、可能的索引/快取策略。
- 這一塊之後要優先朝 mature-reference 類似方向優化，這不是小修，是需要花時間處理的效能工程。
- 目前在單一目錄中移動游標、快速找到想要的檔案或資料夾，體感仍偏慢。
- 這個問題不只是 rendering，也包含操作模式本身不夠直接。
- 下一階段要優先思考：
  - 更快的列表跳轉方式
  - 更快的單目錄內搜尋 / jump 模式
  - 是否加入像 mature-reference / Vim 那種更高效率的定位互動

## 建議下一步優先做的功能

- 列表導航 / 快速定位優化
  你已明確覺得目前在目錄內移動與找檔案太慢，這會直接影響日常使用體驗，優先度很高。
- plugin / action 擴充系統
  之後要支援使用者自行定義對檔案或目錄可執行的動作，例如：
  Xcode 開目錄、`git log`、`git pull`、`svn update`、`svn log` 等。
  第一版建議先做成設定檔驅動的 action plugin，而不是動態載入程式碼。
- global search
  第一版已完成，下一步要把重點轉成效能優化與更像 mature-reference 的體感。
- 更完整的 visual mode
  目前 `V` 已可做範圍標記，但之後可以補更多 Vim 式移動與操作整合。
- 多 pane 專屬操作
  之後可以思考不同 pane 的獨立標記、獨立 clipboard 呈現、或 pane 間的操作提示。
- 操作提示面板優化
  目前 help 已可搜尋與執行，但之後仍可補更多分類、分頁與 mode 提示。
- 設定檔擴充
  可以把主題、預覽比例、是否顯示 hidden、預設排序方式等都開放到 `config.toml`。
- trash 工作流補強
  例如批次 restore、永久刪除、依日期或路徑排序、清空 trash。

## 目前最近的 commit

- `de4873b` `Add visual selection workflow`
- `9d78ace` `Preserve selection when leaving directories`
- `2d9e0e4` `Add development notes`
- `8532293` `Scope preview mode to each pane`
- `1eada83` `Add global search workflow`
- `117b37a` `Add cross-platform open integration and SMB bookmarks`

## 接續開發時建議先做的事

1. 先執行 `cargo test`，確認基線狀態正常。
2. 打開這份筆記確認上次討論到哪裡。
3. 明天先做 Windows 真機測試，把實際問題帶回來。
4. Windows 若沒有大問題，下一步直接做列表導航 / 快速定位優化。
5. global search 的深層效能優化依然重要，但可以排在單目錄導航體驗之後。
