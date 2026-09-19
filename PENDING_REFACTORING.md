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
  11. `Phase 11`: `src/file_manager/app/status.rs` (1,258 行 -> 4 個子模組，門面 23 行，單檔最大 401 行)
  12. `Phase 12`: `src/file_manager/app/keys/mod.rs` (1,100 行 -> 5 個子模組，門面 167 行，單檔最大 362 行)
  13. `Phase 13`: `src/file_manager/pane/preview.rs` (1,017 行 -> 6 個子模組，門面 99 行，單檔最大 341 行)
  14. `Phase 14`: `src/file_manager/app/mod.rs` (1,647 行 -> 5 個子模組，門面 204 行，單檔最大 439 行)

---

## 2. 剩餘待優化大型檔案清單 (> 1,000 行)

**專案中所有核心業務邏輯原始碼已全數低於 1,000 行！**
僅剩單一整合測試檔案超過 1,000 行：

```text
優先級   檔案路徑                              目前行數    建議目標
─────────────────────────────────────────────────────────────────────────────
[P1]    src/file_manager/app/tests.rs       10,835 行   拆分為 tests/ 領域專用測試檔案
```

---

## 3. 接續各階段詳細拆分規劃 (Step-by-Step Plans)

### 🚀 第一順位：`src/file_manager/app/tests.rs` (10,835 行)
**目標**: 依測試領域拆分為獨立測試檔案
- `tests/app_keys_test.rs`：鍵盤事件、Vim 模式、前綴計數測試
- `tests/app_navigation_test.rs`：目錄導航、歷史回退、Zoxide、書籤跳轉測試
- `tests/app_search_test.rs`：即時搜尋、全域搜尋、過濾比對測試
- `tests/app_ops_test.rs`：檔案改名、建立、刪除、垃圾桶與非同步任務測試
- `tests/app_ui_test.rs`：面板渲染、狀態列、對話框與樣式測試

---

### 🚀 第二順位：`src/file_manager/app/mod.rs` (1,643 行)
**目標**: 抽取 App 生命週期與初始化
- **`app/state.rs`** (~400 行)：`App` struct 完整欄位定義與預設值
- **`app/lifecycle.rs`** (~500 行)：`App::new` 初始化與終端退出演奏
- **`app/mod.rs`** (~700 行)：主事件迴圈、渲染總調度

---

### 🚀 第三順位：`src/file_manager/app/tests.rs` (10,835 行)
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
