# PaneFM 待優化與接續開發清單 (Pending Refactoring Guide)

> 本文件專為接續重構與後續開發編寫。您在公司電腦或晚上回家時，可直接依循本文件的模組規劃與指令繼續進行。

---

## 1. 專案當前狀態快照 (Current Snapshot)

- **最新發布版本**: `v0.1.21`
- **當前分支**: `main`
- **測試狀態**: **629 / 629 全數通過**（560 單元 + 19 config + 4 diff + 14 layout + 9 preview + 4 theme + 3 undo + 16 updater）
- **品質標準**: `cargo clippy --all-targets -- -D warnings` 零警告，`cargo fmt --check` 排版通過。
- **已完成重構里程碑（累計 10 大模組完成，全部 < 450 行）**:
  1. `Phase 1`: `src/file_manager/app/mod.rs` (6,908 -> 1,643 行，抽出 10 個領域子模組)
  2. `Phase 2`: `src/file_manager/app/keys/` (5,469 行 -> 14 個子模組，主調度 1,100 行)
  3. `Phase 3`: `src/file_manager/pane/` (5,206 行 -> 13 個子模組，門面 154 行)
  4. `Phase 4`: `src/file_manager/ui/` (3,364 行 -> 8 個子模組，門面 76 行)
  5. `Phase 5`: `src/file_manager/preview/` (2,009 行 -> 8 個子模組，門面 42 行)
  6. `Phase 6`: `src/file_manager/app/file_ops/` (2,197 行 -> 7 個子模組，門面 9 行)
  7. `Phase 7`: `src/file_manager/app/help/` (2,064 行 -> 8 個子模組，門面 14 行)
  8. `Phase 8`: `src/file_manager/app/navigation/` (1,970 行 -> 6 個子模組，門面 7 行)
  9. `Phase 9`: `src/file_manager/app/polling/` (1,783 行 -> 6 個子模組，門面 141 行)
  10. `Phase 10`: `src/config.rs` (1,362 行 -> 7 個子模組，門面 15 行，單檔最大 362 行)

---

## 2. 剩餘待優化大型檔案清單 (> 1,000 行)

目前代碼庫中剩餘 > 1,000 行的檔案如下，建議依此順序處理：

```text
優先級   檔案路徑                              目前行數    建議目標
─────────────────────────────────────────────────────────────────────────────
[P1]    src/file_manager/app/status.rs       1,258 行   拆分為 app/status/ (3 個子模組)
[P2]    src/file_manager/app/keys/mod.rs     1,100 行   細拆 normal 模式鍵位至 keys/
[P3]    src/file_manager/pane/preview.rs     1,017 行   拆分為 pane/preview/ (3 個子模組)
[P4]    src/file_manager/app/mod.rs          1,643 行   抽出 state.rs 與 lifecycle.rs
[P5]    src/file_manager/app/tests.rs       10,835 行   拆分為 tests/ 領域專用測試檔案
```

---

## 3. 接續各階段詳細拆分規劃 (Step-by-Step Plans)

### 🚀 第一順位：`src/file_manager/app/status.rs` (1,258 行)
**目標目錄**: `src/file_manager/app/status/`
- **`shortcuts.rs`** (~450 行)：
  - 底部動態快捷鍵模型：`StatusShortcutHint`, `active_status_shortcut_hints`, `status_shortcut_line`
- **`formatters.rs`** (~450 行)：
  - 各模式狀態文字格式化：`global_search_status`, `format_filter_status`, `preview_search_status`, `list_find_status`, `list_find_locked_status`, `missing_search_tool_status`, `paste_success_status`, `paste_failure_status`, `trash_confirm_status` 等
- **`wrap.rs`** (~250 行)：
  - 狀態列多行文字折疊與寬度排版：`wrap_status_text`, `format_status_detail`
- **`mod.rs`** (~50 行)：
  - 子模組宣告與 re-export

---

### 🚀 第二順位：`src/file_manager/app/status.rs` (1,258 行)
**目標目錄**: `src/file_manager/app/status/`
- **`shortcuts.rs`** (~450 行)：
  - 底部動態快捷鍵模型：`StatusShortcutHint`, `active_status_shortcut_hints`, `status_shortcut_line`
- **`formatters.rs`** (~500 行)：
  - 各模式狀態文字格式化：`global_search_status`, `format_filter_status`, `preview_search_status`, `list_find_status`, `list_find_locked_status`, `missing_search_tool_status`, `paste_success_status`, `paste_failure_status`, `trash_confirm_status` 等
- **`wrap.rs`** (~250 行)：
  - 狀態列多行文字折疊與寬度排版：`wrap_status_text`, `format_status_detail`
- **`mod.rs`** (~50 行)：
  - 子模組宣告與 re-export

---

### 🚀 第三順位：`src/file_manager/app/keys/mod.rs` (1,100 行)
**目標**: 瘦身主按鍵分發器
- **`keys/normal_ops.rs`** (~400 行)：
  - 剪下/複製/貼上/刪除/改名等 Normal 模式檔案操作按鍵分發
- **`keys/normal_nav.rs`** (~350 行)：
  - 上下移動、分頁、書籤觸發、跳轉觸發等 Normal 導航按鍵分發
- **`keys/mod.rs`** (~350 行)：
  - 頂層事件分發、Bracketed Paste 事件處理、數字前綴 (Count) 解析

---

### 🚀 第四順位：`src/file_manager/pane/preview.rs` (1,017 行)
**目標目錄**: `src/file_manager/pane/preview/`
- **`content.rs`** (~400 行)：預覽內容產生與雙欄文字呈現
- **`scroll.rs`** (~300 行)：預覽內部滾動與搜尋結果跳轉
- **`preheat.rs`** (~300 行)：非同步預覽預熱與快取命中管理

---

### 🚀 第五順位：`src/file_manager/app/mod.rs` (1,643 行)
**目標**: 抽取 App 生命週期與初始化
- **`app/state.rs`** (~400 行)：`App` struct 完整欄位定義與預設值
- **`app/lifecycle.rs`** (~500 行)：`App::new` 初始化與終端退出演奏
- **`app/mod.rs`** (~700 行)：主事件迴圈、渲染總調度

---

### 🚀 第六順位：`src/file_manager/app/tests.rs` (10,835 行)
**目標**: 依測試領域拆分為獨立檔案
- `tests/app_keys_test.rs`
- `tests/app_navigation_test.rs`
- `tests/app_search_test.rs`
- `tests/app_ops_test.rs`
- `tests/app_ui_test.rs`

---

## 4. 驗證與品質把關指令 (Quality Assurance Checklist)

每次修改或完成一個模組後，請依序執行下列指令確認零回歸：

```bash
# 1. 快速型別與編譯檢查
cargo check

# 2. 靜態分析（嚴格 0 警告）
cargo clippy --all-targets -- -D warnings

# 3. 程式碼排版
cargo fmt --check

# 4. 全套單元與整合測試（629 個測試必須 100% 通過）
cargo test -- --test-threads=1
```

---

## 5. Git 操作與提交指引

```bash
# 提交單一重構模組
git add <files>
git commit -m "refactor(<scope>): modularize <file> into domain submodules"

# 推送所有成果至 GitHub
git push origin main
```
