# PaneFM 代碼架構重構與後續優化路線圖 (Refactoring Roadmap)

> 本文件記錄了 PaneFM 核心程式碼模組化重構的已完成進度、目前程式碼庫規模統計，以及晚上接續處理的詳細優化路線與拆分指引。

---

## 1. 已完成重構里程碑 (Completed Milestones)

本次連續重構以**高內聚、單檔 < 800 行、零行為破壞、100% 測試保護**為核心原則，徹底瓦解了專案原先的四大巨石檔案：

| 階段 | 原檔案與行數 | 重構後架構 | Commit | 成果與狀態 |
|:---|:---|:---|:---|:---|
| **Phase 1** | `app/mod.rs`<br>**(6,908 行)** | 拆分為 **10 個領域子模組**，主控制器縮減至 **1,651 行** | `1647f6e` | ✅ 629 測試全過、Clippy 0 警告 |
| **Phase 2** | `app/keys.rs`<br>**(5,469 行)** | 拆分為 `keys/` 目錄下 **14 個按鍵子模組**，主調度器 **1,100 行** | `845f808` | ✅ 徹底避免 >2,400 行集中檔案 |
| **Phase 3** | `src/file_manager/pane.rs`<br>**(5,206 行)** | 拆分為 `pane/` 目錄下 **13 個領域子模組**，門面僅 **154 行** | `3707766` | ✅ 單檔最大僅 1,017 行，自然排序與 CoW 獨立 |
| **Phase 4** | `src/file_manager/ui.rs`<br>**(3,364 行)** | 拆分為 `ui/` 目錄下 **8 個渲染子模組**，門面僅 **76 行** | `2627f35` | ✅ 單檔最大僅 756 行，渲染與格式化完全解耦 |
| **Phase 5** | `src/file_manager/preview.rs`<br>**(2,009 行)** | 拆分為 `preview/` 目錄下 **8 個領域子模組**，門面僅 **42 行** | `04fec9d` | ✅ 單檔最大僅 435 行，Halfblock、語法、壓縮檔樹全面解耦 |
| **Phase 6** | `src/file_manager/app/file_ops.rs`<br>**(2,197 行)** | 拆分為 `app/file_ops/` 目錄下 **7 個領域子模組**，門面僅 **9 行** | 待提交 | ✅ 單檔最大僅 647 行，開啟、建立、壓縮、刪除、剪貼、貼上、傳輸完全解耦 |

---

## 2. 目前剩餘大型檔案規模統計 (> 1,000 行)

截至版本 `v0.1.21`（Phase 6 完成後），程式碼庫中仍超過 1,000 行的檔案如下（已依優先級排序）：

```text
優先級   檔案路徑                              目前行數    主要職責
─────────────────────────────────────────────────────────────────────────────
[P1]    src/file_manager/app/help.rs         2,064 行   F1 完整功能字典、情境速查表、說明搜尋與動作關聯
[P2]    src/file_manager/app/navigation.rs   1,970 行   目錄切換、跨視窗焦點、歷史路徑跳轉、多選同步
[P3]    src/file_manager/app/polling.rs      1,783 行   檔案監聽事件處理、目錄大小更新、非同步背景傳輸輪詢
[P4]    src/file_manager/app/mod.rs          1,649 行   App 結構體定義、初始化、全域生命週期與事件調度
[P5]    src/config.rs                        1,362 行   TOML 設定檔載入、驗證、持久化、預設範本與快捷鍵配置
[P6]    src/file_manager/app/status.rs       1,258 行   底部狀態列動態快捷鍵提示、狀態字串折疊與格式化
[P7]    src/file_manager/app/keys/mod.rs     1,100 行   Normal 模式按鍵調度、計數前綴 (Count)、貼上事件
[P8]    src/file_manager/pane/preview.rs     1,017 行   PaneState 專屬預覽視窗滾動、搜尋跳轉、非同步預熱
[P9]    src/file_manager/app/tests.rs       10,835 行   整合測試套件（可按領域拆分為多個測試檔案）
```

---

## 3. 接續處理建議清單與拆分方案 (Action Plan)

### 推薦第一順位：`src/file_manager/app/help.rs` (2,064 行)
**目標**：將靜態功能說明字典與動態搜尋分離。
- **`help/entries.rs`** (~1,400 行)：純粹靜態的 `help_entries()` 大字典資料庫。
- **`help/cheatsheet.rs`** (~400 行)：各操作模式的情境速查表 (`context_cheatsheet_entries`)。
- **`help/search.rs`** (~250 行)：說明項目的前綴搜尋、模糊搜尋與快捷鍵過濾。

---

### 推薦第四順位：`src/file_manager/app/navigation.rs` (1,970 行)
**目標**：拆分目錄與游標導航。
- **`navigation/cursor.rs`** (~500 行)：列表游標上下移動、換頁、開頭/結尾跳轉、計數跳轉。
- **`navigation/panes.rs`** (~600 行)：視窗間焦點切換、分割視窗同步、寬度與權重調整。
- **`navigation/path.rs`** (~500 行)：目錄切換 (`cd`)、父目錄 (`..`)、歷史目錄後退與前進。
- **`navigation/marks.rs`** (~350 行)：多選標記連動、反選、全選操作。

---

## 4. 驗證與品質把關指令 (Quality Assurance Checklist)

每次模組化完成後，請依序執行下列指令確保零破壞：

```bash
# 1. 快速型別與編譯檢查
cargo check

# 2. 靜態程式碼分析 (必須 0 警告)
cargo clippy --all-targets -- -D warnings

# 3. 程式碼排版格式化
cargo fmt --check

# 4. 全套單元與整合測試 (全專案 629 個測試必須 100% 通過)
cargo test -- --test-threads=1
```

---

## 5. 本次發布版本資訊 (Release Info)
- **最新版本**: `v0.1.21`
- **當前分支**: `main`
- **累計重構提交**:
  - `1647f6e` (`refactor(app): modularize app/mod.rs into domain submodules`)
  - `845f808` (`refactor(keys): modularize keys.rs into 14 domain submodules`)
  - `3707766` (`refactor(pane): modularize pane.rs into 13 domain submodules`)
  - `2627f35` (`refactor(ui): modularize ui.rs into 8 domain submodules`)
