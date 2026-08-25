# Development Guidelines

這份文件是 `PaneFM` 的開發規範。

之後每次新增、修改或重構功能時，都必須先對照這份文件，確認設計沒有違反規則，再開始實作。

## 1. 專案目標

這個專案是以個人高效率使用為核心的 terminal 檔案管理器，操作風格以 Vim / mature-reference 為主要參考。

目前正式支援的平台是：

- `macOS`
- `Windows`

`Linux / Unix` 目前不是正式支援目標，但架構必須保留擴充空間，不能把平台邏輯寫死。

## 2. 核心設計原則

每次開發功能時，必須優先遵守以下原則：

- `panel` 是第一級物件，所有 UI 與狀態都應先以 pane/panel 為單位思考。
- 除非明確指定為全域功能，否則所有 UI 都應只畫在目前 focus 的 panel 內。
- 操作邏輯優先維持 Vim 風格一致性，避免每個介面都有不同操作規則。
- 功能若會頻繁調整，必須預留設定檔或可抽換的設計，不可寫死。
- 平台差異、terminal 差異、外部程式差異，必須在設計時就先考慮，不可等 bug 出現才補救。

## 3. UI / Panel 規則

### 3.1 Panel 優先

- 每個 panel 都必須有自己的狀態。
- 不可讓某個 panel 的暫時 UI 直接污染其他 panel。
- 切換 focus 後，其他 panel 不應被強制帶進同樣模式，除非那是全域模式。

### 3.2 預設不是全域 UI

以下功能若沒有特別說明，一律做成 panel-local：

- help
- bookmark list
- command UI
- filter
- rename
- open picker
- trash view
- search / find / preview search

### 3.3 畫面切換

- 優先使用「取代目前 panel 內容」而不是再開一層多餘視窗，除非彈窗真的更清楚。
- 小型選擇器或輸入框可以疊在 panel 內，但必須維持可讀性。
- 任何暫時 UI 關閉後，必須能乾淨回到原本列表或模式。

## 4. 操作一致性規則

### 4.1 Vim 風格是核心

只要介面本質上是「可移動游標的列表或面板」，預設必須支援：

- `j/k`：上下移動
- `h`：離開、上一層、取消、返回
- `l`：進入、確認、下一層、執行目前項目
- `gg / G`：跳到頂部 / 底部
- `Ctrl-d / Ctrl-u`：往下 / 往上翻半頁
- `J / K`：固定大步長移動

如果某個介面不能支援這些鍵，必須有非常明確的原因，不能只是因為實作方便。

### 4.2 模式切換

- 若功能有 normal / insert 兩種行為，游標外觀必須明確反映模式。
- `Esc` 的行為必須一致且可預期，優先代表離開輸入、取消暫時模式、回到上一層。
- 如果某個模式是兩段式離開，必須在狀態列清楚說明目前狀態。

## 5. Terminal 相容性規則

這個專案不是只跑在單一 terminal，因此不能依賴某一個 terminal 的特殊行為。

### 5.1 鍵盤輸入正規化

- 所有關鍵快捷鍵判斷都應透過統一的 key normalization helper。
- 不能假設每個 terminal 都會把 `Shift`、`Ctrl`、符號鍵回報成完全相同格式。
- 一般文字輸入與快捷鍵判斷要分開思考。
- 新功能若新增快捷鍵，必須檢查是否需要納入既有的正規化流程。

### 5.2 Keyboard Enhancement Flags

- TUI 模式只啟用「最小必要」的 keyboard enhancement flags。
- 除非有非常清楚的需求與測試保護，不能隨意新增更激進的 terminal keyboard flags。
- 外部程式互動前，必須先還原 terminal 狀態，避免把 TUI 的鍵盤協定洩漏給外部程式。

目前規則：

- 只保留 `DISAMBIGUATE_ESCAPE_CODES`
- 不預設啟用 `REPORT_ALL_KEYS_AS_ESCAPE_CODES`
- 不預設啟用 `REPORT_EVENT_TYPES`

原因：

- 太激進的 keyboard flags 可能會讓 `fzf`、`vim` 或其他外部程式收到不可預期的 escape sequence。
- 這類 bug 在 `macOS` 上特別容易出現，但設計上必須同時考慮 `Windows`。

### 5.3 TUI 與外部程式的邊界

任何要離開 TUI 去執行外部互動程式的功能，都必須遵守：

1. 關閉 raw mode
2. 還原 cursor style
3. 關閉 TUI 的 keyboard enhancement 狀態
4. 離開 alternate screen
5. 執行外部程式
6. 回來後重新進入 alternate screen
7. 恢復 TUI keyboard enhancement 狀態
8. 恢復 raw mode
9. 清畫面並回到可互動狀態

適用功能包含：

- `fzf`
- `vim`
- `$EDITOR`
- 未來任何 terminal-blocking 外部互動工具

### 5.4 `fzf` 整合規則

- `fzf` 必須保有自己的 TTY 互動輸入，不可把候選串流直接 pipe 到 `fzf` 的 `stdin` 後又期待 `Esc`、`Enter` 等互動完全正常。
- 若需要動態產生候選資料，優先讓 `fzf` 透過 `FZF_DEFAULT_COMMAND` 或等效機制自行啟動 helper command。
- 不可依賴使用者自己的 `FZF_DEFAULT_OPTS` 決定核心行為；像 `Esc`、`Enter` 這種關鍵互動必須由程式明確指定。
- 若 `fzf` 體感變慢，應先區分：
  - `fzf` 本身等待/退出耗時
  - 跳回 TUI 後的 `go_to_path` / `reload` / preview 耗時
- 若要排查效能，先使用 `TFM_DEBUG_TIMING=1` 觀察 timing log，再決定是否改搜尋器、reload 或 preview 流程。

## 6. 雙平台規則

### 6.1 平台分流

- 所有平台差異必須集中在明確的抽象層，例如 `platform.rs`。
- 業務邏輯不應直接散落 `if macOS / if Windows`。
- 若某功能需要不同平台命令，必須包成可測試的 builder 或 helper。

### 6.2 支援等級

- `macOS` 與 `Windows` 都必須是正式可用等級。
- 若某功能目前只有其中一個平台實測過，必須在文件或註記中明確標出。
- 若當下沒有真機可測，程式結構至少要先做到「可分流、可替換、可補測」。

### 6.3 外部命令

- 打開檔案、Reveal、SMB、壓縮/解壓縮、搜尋器等功能，都要先思考平台差異。
- 外部命令字串不可散落在各處，應盡量集中管理。
- 任何新增的外部程式依賴，都要先想好：
  - `macOS` 怎麼跑
  - `Windows` 怎麼跑
  - 失敗時如何提示
  - 缺少依賴時如何提示

## 7. 搜尋與效能規則

- 使用者感知到的流暢度很重要，不能只看功能有沒有做出來。
- 大目錄下的操作若可能卡頓，必須優先考慮：
  - lazy loading
  - chunked result
  - loading 狀態
  - 背景執行
  - 可取消流程
- 若使用外部成熟工具可以大幅提升體驗，優先考慮整合，而不是重做一套較差的版本。

例如：

- 單目錄/全目錄快速跳轉：可優先考慮 `fzf`
- 大量檔案搜尋：優先考慮成熟搜尋工具或分批回傳架構

## 8. 設定檔規則

- 可調整的行為、步長、主題、UI 尺寸、預設模式，不應寫死在程式中。
- `config.toml` 只放設定。
- 與使用者資料有關的內容，應放在獨立檔案，例如 `bookmark.toml`。
- 新增設定時必須同步更新：
  - `config.toml.example`
  - 設定解析程式
  - 驗證邏輯
  - README 或相關文件
  - 測試

## 9. 測試規則

這個專案的硬性規則是：

- 每一個新功能都必須有測試
- 每一個修過的 bug 都必須補測試
- 不能只修目前看到的問題而不建立保護

### 9.1 測試註解格式

每一個 `#[test]` 都必須附上繁體中文註解，而且至少清楚回答兩件事：

- `驗證內容`：測試建立了什麼條件、執行什麼操作，以及預期哪個結果。
- `保護目的`：這個案例是為了避免哪一種 bug、平台差異或重構回歸。

建議格式：

```rust
#[test]
/// 驗證 Windows Reveal 會使用 Explorer `/select,` 聚焦指定檔案。
/// 保護目的：避免平台命令重構後只打開父目錄，卻沒有選中使用者原本操作的檔案。
fn windows_reveal_uses_explorer_select() {
    // ...
}
```

不可只寫「測試 rename」或直接重述函數名稱，因為這種註解無法協助不熟悉專案的
開發者判斷失敗代表哪一條產品規格被破壞。

至少要覆蓋：

- 正常路徑
- 取消 / 失敗路徑
- 重要邊界條件
- 平台分流 helper 的輸出
- terminal 事件正規化

如果功能和 terminal 差異有關，應優先增加：

- 不同 key event 變體的測試
- 外部命令 builder 測試
- 狀態切換前後的測試

## 10. 程式結構規則

- 功能要依責任切分，不可再把大量邏輯塞回單一巨大檔案。
- 新功能應先找現有模組責任是否合適，再決定新增檔案或子模組。
- 類似功能要集中在同一個目錄或模組下管理。
- 若某功能已明顯長大，應主動拆出 helper、builder、state 或 renderer。

## 11. 註解與文件規則

- 所有公開或重要函式、結構、流程，都應有繁體中文註解。
- 註解要說明：
  - 這個類別或函式做什麼
  - 參數是什麼
  - 回傳值代表什麼
  - 為什麼這樣設計
- 遇到平台相容性、terminal 協定、外部程式邊界這類容易踩雷的地方，要留下設計原因。

## 12. 開發前檢查清單

每次開發功能前，先確認：

- 這個功能是 panel-local 還是 global？
- 是否符合 Vim 核心操作？
- 是否有平台差異？
- 是否會碰到 terminal keyboard / raw mode / alternate screen？
- 是否會呼叫外部程式？
- 是否需要設定檔？
- 是否需要 loading / cancel / background？
- 要補哪些測試？

## 13. 開發後檢查清單

每次完工前，至少確認：

- `cargo fmt`
- `cargo test`
- 新功能有測試
- 修 bug 有回歸測試
- 沒有把 panel 狀態做成不必要的全域
- 沒有把平台差異散落在業務邏輯裡
- 沒有讓外部程式吃到 TUI 的 terminal 狀態
- 設定或文件有同步更新

## 14. 文件維護規則

這份文件不是一次性文件。

如果未來在開發中發現新的共通規則，或踩到新的跨平台 / terminal / UI 問題，必須把經驗補進這份文件，而不是只修一次 code 就結束。
