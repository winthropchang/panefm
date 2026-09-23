# PaneFM 待優化與接續開發清單 (Pending Refactoring Guide)

> 本文件專為接續重構與後續開發編寫。您在公司電腦或晚上回家時，可直接依循本文件的模組規劃與指令繼續進行。

---

## 1. 專案當前狀態快照 (Current Snapshot)

- **最新發布版本**: `v0.1.25`
- **當前分支**: `main`
- **測試狀態**: **669 / 669 全數通過**（600 單元 + 19 config + 4 diff + 14 layout + 9 preview + 4 theme + 3 undo + 16 updater）
- **品質標準**: `cargo clippy --all-targets -- -D warnings` 零警告，`cargo fmt --check` 排版通過。
- **已完成重構里程碑（累計 17 大模組完成，全專案 100% 檔案 < 1,000 行，多數 < 800 行）**:
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
  15. `Phase 15`: `src/file_manager/platform.rs` (933 行 -> 5 個子模組，門面 48 行，單檔最大 302 行)
  16. `Phase 16`: `src/file_manager/app/tests.rs` (10,836 行 -> 20 個領域測試子模組 + `helpers.rs`，門面 65 行，單檔最大 917 行)
  17. `Phase 17`: `src/file_manager/tests/pane_test.rs` (2,362 行 -> 5 個領域測試子模組，門面 24 行，單檔最大 774 行)

---

## 2. 剩餘待優化檔案清單

🎉 **重大里程碑：專案中所有檔案（生產核心原始碼與所有測試套件）已 100% 全數在 1,000 行以內！**
目前全專案單檔最高僅 917 行。若後續想進一步精簡生產程式碼（現皆 < 850 行），剩餘可關注的候選檔案如下：

```text
優先級   檔案路徑                              目前行數    性質與建議目標
─────────────────────────────────────────────────────────────────────────────
[P1]    src/file_manager/app/keys/pickers.rs   815 行   生產程式碼：選單快捷鍵派發與過濾處理
[P2]    src/file_manager/ui/pane.rs            756 行   生產程式碼：單一面板與樹狀檢視渲染
[P3]    src/updater.rs                         725 行   生產程式碼：更新檢查、下載與安裝驗證
[P4]    src/file_manager/vcs.rs                719 行   生產程式碼：Git / SVN 狀態追蹤與查詢
```

---

## 3. 接續各階段詳細拆分規劃 (Step-by-Step Plans)

### 🚀 第一順位：`src/file_manager/app/keys/pickers.rs` (815 行)
**目標**: 拆解選單快捷鍵交互
- 快捷鍵選擇器按鍵處理
- 搜尋與過濾對話框按鍵
- 主題與配置即時切換按鍵
- 快捷鍵選擇器按鍵處理
- 搜尋與過濾對話框按鍵
- 主題與配置即時切換按鍵

---

### 🚀 第二順位：`src/file_manager/app/keys/pickers.rs` (815 行)
**目標**: 拆解選單快捷鍵交互
- 快捷鍵選擇器按鍵處理
- 搜尋與過濾對話框按鍵
- 主題與配置即時切換按鍵

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
