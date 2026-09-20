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
| **Phase 6** | `src/file_manager/app/file_ops.rs`<br>**(2,197 行)** | 拆分為 `app/file_ops/` 目錄下 **7 個領域子模組**，門面僅 **9 行** | `e83fc3b` | ✅ 單檔最大僅 647 行，開啟、建立、壓縮、刪除、剪貼、貼上、傳輸完全解耦 |
| **Phase 7** | `src/file_manager/app/help.rs`<br>**(2,064 行)** | 拆分為 `app/help/` 目錄下 **8 個領域子模組**，門面僅 **14 行** | `bf63dfb` | ✅ 單檔最大僅 566 行，F1 字典、搜尋、編輯模式與面板速查表完全解耦 |
| **Phase 8** | `src/file_manager/app/navigation.rs`<br>**(1,970 行)** | 拆分為 `app/navigation/` 目錄下 **6 個領域子模組**，門面僅 **7 行** | `95a258b` | ✅ 單檔最大僅 428 行，視窗佈局、路徑追蹤、書籤、說明狀態、選擇器與搜尋啟動完全解耦 |
| **Phase 9** | `src/file_manager/app/polling.rs`<br>**(1,783 行)** | 拆分為 `app/polling/` 目錄下 **6 個領域子模組**，門面僅 **141 行** | `7f5439b` | ✅ 單檔最大僅 412 行，任務管理、搜尋緩衝、目錄載入與大小掃描、監聽與檔案操作完全解耦 |
| **Phase 10** | `src/config.rs`<br>**(1,362 行)** | 拆分為 `src/config/` 目錄下 **7 個領域子模組**，門面僅 **15 行** | `32029f4` | ✅ 單檔最大僅 362 行，資料結構、檔案型別、路徑、範本、持久化、外掛與驗證套用完全解耦 |
| **Phase 11** | `src/file_manager/app/status.rs`<br>**(1,258 行)** | 拆分為 `app/status/` 目錄下 **4 個領域子模組**，門面僅 **23 行** | `c4cdfa1` | ✅ 單檔最大僅 401 行，操作提示、快捷選單提示、全域快捷鍵列與文字格式化完全解耦 |
| **Phase 12** | `src/file_manager/app/keys/mod.rs`<br>**(1,100 行)** | 拆分為 `keys/` 目錄下 **5 個專用子模組**，門面僅 **167 行** | `e260303` | ✅ 單檔最大僅 362 行，計數前綴、貼上事件、待處理分派、導航按鍵與檔案操作按鍵徹底解耦 |
| **Phase 13** | `src/file_manager/pane/preview.rs`<br>**(1,017 行)** | 拆分為 `pane/preview/` 目錄下 **6 個專用子模組**，門面僅 **99 行** | `d28004a` | ✅ 單檔最大僅 341 行，預覽渲染、標題格式化、滾動邊界、搜尋跳轉、背景預熱與狀態控制徹底解耦 |
| **Phase 14** | `src/file_manager/app/mod.rs`<br>**(1,647 行)** | 拆分為 `app/` 目錄下 **5 個專用子模組**，門面僅 **204 行** | `394b33f` | ✅ 單檔最大僅 439 行，狀態定義、初始化生命週期、單一視窗渲染、彈窗對話框與頂層渲染徹底解耦 |
| **Phase 15** | `src/file_manager/platform.rs`<br>**(933 行)** | 拆分為 `platform/` 目錄下 **5 個領域子模組**，門面僅 **48 行** | `8a49bf1` | ✅ 單檔最大僅 302 行，剪貼簿、終端規格、祖先行程追蹤與系統開啟/Reveal 徹底解耦 |
| **Phase 16** | `src/file_manager/app/tests.rs`<br>**(10,836 行)** | 拆分為 `app/tests/` 目錄下 **20 個領域測試子模組** + `helpers.rs`，門面僅 **65 行** | `d20e57b` | ✅ 單檔最大僅 917 行（多數 < 800 行），329 個測試全數遷移、零遺漏零回歸 |

---

## 2. 目前剩餘大型檔案規模統計 (> 1,000 行)

截至 Phase 16 完成後，**專案中所有核心業務邏輯原始碼已全數低於 850 行，且 10,836 行的 `app/tests.rs` 巨石已徹底消除！**
專案中僅剩單一整合測試檔案超過 1,000 行：

```text
優先級   檔案路徑                              目前行數    主要職責
─────────────────────────────────────────────────────────────────────────────
[P1]    src/file_manager/tests/pane_test.rs  2,362 行   Pane 模組整合測試套件（可拆分為操作、導航、渲染子測試檔）
```

---

## 3. 接續處理建議清單與拆分方案 (Action Plan)

### 推薦第一順位：`src/file_manager/tests/pane_test.rs` (2,362 行)
**目標**：依測試領域拆分為獨立測試檔案。
- `tests/pane/ops_test.rs`：分割、關閉、等寬/平衡調整測試
- `tests/pane/navigation_test.rs`：目錄移動、歷史歷程、聚焦邊界測試
- `tests/pane/render_test.rs`：視口計算、寬高縮放、標籤格式化測試

### 推薦第二順位：`src/file_manager/app/keys/pickers.rs` (815 行)
**目標**：拆解選單快捷鍵交互
- 快捷鍵選擇器按鍵處理
- 搜尋與過濾對話框按鍵
- 主題與配置即時切換按鍵


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
  - `04fec9d` (`refactor(preview): modularize preview.rs into 8 domain submodules`)
  - `e83fc3b` (`refactor(file_ops): modularize file_ops.rs into 7 domain submodules`)
  - `bf63dfb` (`refactor(help): modularize help.rs into 8 domain submodules`)
  - `95a258b` (`refactor(navigation): modularize navigation.rs into 6 domain submodules`)
  - `7f5439b` (`refactor(polling): modularize polling.rs into 6 domain submodules`)
