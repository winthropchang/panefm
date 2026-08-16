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
  `cargo test` => `130 passed`
- 另外有針對今天新增的 SMB / 書籤流程補測試：
  - SMB 書籤會正確寫進 `bookmark.toml`
  - SMB 連線成功後存書籤不會誤存成本機掛載路徑
  - 跳 SMB 書籤會自動進入對應目標

## Windows 目前還沒完成的地方

- 現在不能說已完整支援 Windows。
- 核心檔案操作、pane、filter、search、bookmark、rename、壓縮解壓縮這些大多是跨平台方向。
- 但外部開啟流程目前仍偏 Unix / mac 寫法，這是明天要優先處理的重點。

### 明確缺口

- [src/file_manager/open.rs](/Users/otto/Documents/terminal-file-manager/src/file_manager/open.rs)
  裡的 `$EDITOR` / `Vim` 目前仍寫死用 `/bin/sh` 啟動，Windows 一定不對。
- `system_open_spec()` 目前只有 macOS 用 `open`，其他平台都走 `xdg-open`；
  這代表 Windows 現在會錯用 `xdg-open`。
- `Reveal` 在 Windows 也還沒做成 `explorer.exe /select,...` 這種比較合理的行為。
- 雖然 SMB 已有 Windows UNC 路徑分支，但整體「連線成功後的外部整合」還沒有完成驗證。

## 明天接手時的建議順序

1. 先跑一次 `cargo test`，確認今天的基線還是乾淨的。
2. 先處理 Windows 的外部開啟抽象層：
   `Open`、`Reveal`、`$EDITOR`、`Vim`。
3. 把 [src/file_manager/open.rs](/Users/otto/Documents/terminal-file-manager/src/file_manager/open.rs) 做成明確的平台分流，不要再用「非 mac 一律 xdg-open」。
4. 補 Windows 專用測試：
   至少要覆蓋 `system_open_spec`、`reveal_in_system_spec`、編輯器與 Vim 啟動命令。
5. 如果還有時間，再檢查 SMB 在 Windows 實際使用時，是否需要把 UNC 路徑與 pane 導航再補一層保護。

## 已知待優化問題

- global search 在超大目錄中的體感速度仍明顯落後 mature-reference。
- 目前已改善成：
  按下 `s` 不會立刻掃描、搜尋改成背景執行、主列表有 loading 提示、結果會分批灌入。
- 但仍未完成：
  真正接近 mature-reference 的大型目錄掃描效率、搜尋 task 調度優化、可能的索引/快取策略。
- 這一塊之後要優先朝 mature-reference 類似方向優化，這不是小修，是需要花時間處理的效能工程。

## 建議下一步優先做的功能

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

## 接續開發時建議先做的事

1. 先執行 `cargo test`，確認基線狀態正常。
2. 打開這份筆記確認上次討論到哪裡。
3. 下一步先優先處理 Windows 相容層，特別是 `open.rs`。
4. Windows 相容補完後，再回頭做 global search 的深層效能優化。
5. visual mode / trash / open-edit 這些高頻操作仍可持續補強，但目前優先度低於 Windows。
