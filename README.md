# PaneFM (Pane File Manager)

<p align="center">
  <img src="https://raw.githubusercontent.com/winthropchang/panefm/main/assets/panefm-icon.png" alt="PaneFM icon" width="160"><br>
  <b>具備 Vim 操控手感、多視窗分割與 N-Way 目錄比對的現代化終端檔案管理器</b><br>
  <sub>針對多目錄並行管理、鍵盤流操作與終端工作流程整合設計</sub>
</p>

<p align="center">
  <a href="https://github.com/winthropchang/panefm/actions/workflows/ci.yml"><img src="https://github.com/winthropchang/panefm/actions/workflows/ci.yml/badge.svg" alt="CI Status"></a>
  <a href="https://github.com/winthropchang/panefm/releases"><img src="https://img.shields.io/github/v/release/winthropchang/panefm?color=blue" alt="Latest Release"></a>
  <img src="https://img.shields.io/badge/Language-Rust_1.85+-orange.svg" alt="Rust 1.85+">
  <img src="https://img.shields.io/badge/Platform-macOS_%7C_Windows-blue.svg" alt="Platform">
  <img src="https://img.shields.io/badge/License-MIT-green.svg" alt="License">
  <img src="https://img.shields.io/badge/Style-Vibe_Coding-purple.svg" alt="Vibe Coding">
</p>

---

## 核心特性 (Features)

- **多視窗佈局 (Multi-Pane Layout)**：支援水平與垂直自由分割（最多 9 個獨立視窗），具備巢狀層級尺寸鎖定、一鍵均等重設 (`w=`) 與連續尺寸微調模式 (`wr`)。
- **Vim 導航思維 (Vim-Style Navigation)**：支援 `h/j/k/l`、計數前綴（如 `5j`）、`gg/G`、`Ctrl+d/u` 翻頁，以及兩鍵直達畫面任意檔案的 `EasyMotion` 快速跳轉 (`e`)。
- **左右雙欄即時預覽 (Side-by-Side Live Preview)**：在清單中移動游標即時預覽檔案內容；支援文字程式碼語法高亮、ANSI 彩色縮圖、目錄結構統計與壓縮包檢視，並提供非同步背景預熱機制。
- **N-Way 目錄差異比對 (Diff Matrix)**：同時對齊與比對 2 至 N 個目錄樹，支援狀態過濾（差異、獨有、相同）與外部比對工具（如 `delta`、`difftastic`）呼叫。
- **安全檔案操作與復原 (File Operations & Undo)**：支援 macOS APFS / Reflink 檔案系統層級的零空間複製 (Copy-on-Write)、多線程背景傳輸、視覺多選 (`v`)、Regex 批次改名與交易式復原 (`u`)。
- **終端環境整合 (Terminal Integration)**：內建 OSC 7 目錄廣播，原生終端開新分頁自動同步當前目錄；整合 `fd`（檔名搜尋）、`ripgrep`（內容檢索）、`fzf`（模糊跳轉）與 `zoxide`（歷史目錄）。

---

## 系統架構 (Architecture)

<p align="center">
  <img src="https://raw.githubusercontent.com/winthropchang/panefm/main/assets/architecture.svg" alt="PaneFM 系統架構圖" width="100%">
</p>

| 🪟 多視窗佈局 | ⚡ Vim 導航跳轉 | 📦 安全檔案操作 | 🔍 全域搜尋預覽 |
|:---|:---|:---|:---|
| • 自由水平/垂直分割<br>• 1~9 視窗快速切換<br>• 原生終端分頁同步 | • `h/j/k/l` & 數字前綴<br>• `g` 快速跳轉選單<br>• `fzf` / `zoxide` / `List Find` | • 視覺多選 (`v`) / 安全貼上<br>• CoW 零空間快速複製<br>• 交易式 Undo / 垃圾桶 | • `fd` 檔名串流搜尋<br>• `rg` 內容全文檢索<br>• `Tab` 即時預覽（語法/圖片/目錄） |

| ⚖️ 目錄差異矩陣 | 🔖 智慧書籤歷史 | 🎨 主題外觀視圖 | 📊 任務與說明 |
|:---|:---|:---|:---|
| • 2~N 目錄同時對齊<br>• 4 種狀態差異過濾<br>• 呼叫外部比對器 (`delta`) | • 單鍵快捷跳轉書籤<br>• 檔案系統自動監聽<br>• 網路磁碟防卡死保護 | • 10+ 款精選主題<br>• 動態即時預覽切換<br>• 5 種欄位顯示模式 | • 背景非同步傳輸不阻斷 UI<br>• `?` 當前面板情境速查<br>• 統一 `q`/`Esc` 退出 |

---

## 安裝方式 (Installation)

### 1. 一鍵快速安裝（macOS 推薦）

透過官方安裝腳本直接下載最新版本並完成系統整合（自動設定可執行權限、解除 macOS Gatekeeper 阻擋並進行程式碼簽名）：

```bash
curl -fsSL https://raw.githubusercontent.com/winthropchang/panefm/main/install.sh | bash
```

> 安裝腳本會自動偵測硬體架構（Apple Silicon / Intel），並優先安裝至 `/usr/local/bin/panefm`（若無權限則自動安裝至 `~/.local/bin/panefm`）。

---

### 2. 外部工具相依

PaneFM 依賴系統 `PATH` 中的外部工具以發揮完整搜尋與跳轉效能：

- **`fd`**（全域檔名搜尋）
- **`ripgrep`**（執行檔為 `rg`，全域內容檢索）
- **`fzf`**（互動式模糊跳轉）
- **`zoxide`**（目錄歷史學習）

**macOS (Homebrew)**：
```bash
brew install fd fzf ripgrep zoxide
```

**Windows (WinGet)**：
```powershell
winget install sharkdp.fd BurntSushi.ripgrep.MSVC junegunn.fzf ajeetdsouza.zoxide
```

> 啟動 PaneFM 後可輸入 `:status` 檢查外部工具的安裝狀態。

---

### 3. 下載預先編譯版本 (GitHub Releases)

至 [Releases 頁面](https://github.com/winthropchang/panefm/releases) 下載對應作業系統的執行檔：

- **macOS (Apple Silicon / ARM64)**：`panefm-macos-arm64`
- **macOS (Intel / x86_64)**：`panefm-macos-x64`
- **Windows (x64)**：`panefm-windows-x64.exe`

#### macOS 隔離標記解除 (Gatekeeper 阻擋排除)
若透過瀏覽器下載執行時遇到系統提示「無法打開，因為 Apple 無法檢查其是否包含惡意軟體」或「檔案已損毀」，請在終端機執行以下指令解除 Gatekeeper 隔離並賦予執行權限：
```bash
# 解除隔離屬性、賦予執行權限與本機簽名
xattr -c ./panefm-macos-arm64
chmod +x ./panefm-macos-arm64
codesign -s - --force ./panefm-macos-arm64

# 移動至全域 PATH 目錄（選用）
sudo mv ./panefm-macos-arm64 /usr/local/bin/panefm
```

---

### 4. 從原始碼編譯

```bash
git clone https://github.com/winthropchang/panefm.git
cd panefm
cargo build --release
```
編譯產生的執行檔位於 `target/release/panefm`（Windows 為 `target\release\panefm.exe`）。

---

## 快速上手 (Quick Start)

### 常用核心按鍵

| 按鍵 | 功能說明 |
| :--- | :--- |
| `h` / `j` / `k` / `l` | 上一層目錄 / 向下移動 / 向上移動 / 進入目錄（支援數字前綴，如 `5j`） |
| `Tab` | 開啟 / 關閉左右雙欄即時預覽 |
| `v` / `Space` | 連續選取模式 (Visual Mode) / 單項標記 |
| `y` / `x` / `p` | 複製選單（`yy` 複製、`y 1..9` 跨視窗複製） / 剪下 / 貼上 |
| `r` / `d` / `u` | 重新命名 / 移至垃圾桶 / 交易式復原 (Undo) |
| `Ctrl+v` / `Ctrl+s` | 垂直分割新視窗 / 水平分割新視窗 |
| `1` ~ `9` | 直接切換至指定編號的視窗 |
| `s` / `S` | 全域檔名搜尋 (`fd`) / 全文內容檢索 (`rg`) |
| `/` | 當前目錄即時搜尋（按 `n`/`N` 跳轉） |
| `e` | EasyMotion 標籤直達（螢幕可見檔案兩鍵瞬移） |
| `?` / `F1` | **情境感知速查表 (Cheatsheet)** / 全局完整說明手冊 |
| `q` / `Esc` | 關閉當前面板 / 退出程式 |

> **提示**：在任何畫面按下 **`?`** 即可即時查看當下操作模式可用的完整快捷鍵清單。

---

### 左右即時預覽操作 (Live Preview)

```text
┌─── 左側檔案清單 (38%) ──┬────────────── 右側即時預覽 (62%) ──────────────┐
│ [1] ~/project          │ Preview: main.rs                       ^ v    │
│ > 01_guide.md          │ 1  use std::path::Path;                       │
│   02_config.toml       │ 2  use anyhow::Result;                        │
│   src/                 │ 3                                             │
│   Cargo.toml           │ 4  fn main() -> Result<()> {                  │
│                        │ 5      println!("Hello panefm!");             │
│                        │ 6      Ok(())                                 │
│                        │ 7  }                                          │
└────────────────────────┴───────────────────────────────────────────────┘
```

1. **開關預覽**：在清單中按 `Tab` 開啟左右雙欄，焦點維持在左側清單。
2. **即時切換**：在清單中移動 `j`/`k`，右側預覽即時自動更新內容。
3. **聚焦預覽**：在檔案上按 `l` 進入預覽焦點，此時可使用 `j`/`k` 捲動內容、`Ctrl+d/u` 翻頁、`/` 預覽搜尋、`[`/`]` 切換檔案。
4. **切回清單**：在預覽焦點中按 `h`、`q` 或 `Esc` 返回左側清單，預覽視窗保持開啟。
5. **關閉雙欄**：任何時候按 `Tab` 即可徹底關閉預覽，返回全寬清單。

---

<details>
<summary><b>完整快捷鍵參考表 (Full Keybindings Reference - 點擊展開)</b></summary>

<br>

| 分類 | 按鍵 | 功能說明 |
| :--- | :--- | :--- |
| **游標導航** | `h` / `j` / `k` / `l` | 上一層 / 向下 / 向上 / 進入目錄（支援數字前綴如 `10j`） |
| | `gg` / `G` | 跳至清單頂端 / 跳至清單底端 |
| | `Ctrl+d` / `Ctrl+u` | 向下半頁 / 向上半頁 |
| | `e` / `:easymotion` | 啟用 EasyMotion 畫面跳轉模式 |
| | `g` | 快速跳轉選單（`gd` 文件、`gk` 桌面、`gl` 下載、`gh` 家目錄、`gt` 輸入路徑） |
| **搜尋過濾** | `s` / `S` | 全域檔名搜尋 (`fd`) / 全域內容檢索 (`rg`) |
| | `/` | 當前目錄即時搜尋（`n`/`N` 下一個/上一個） |
| | `f` | 即時過濾清單（`Tab` 切換一般/模糊模式） |
| | `z` / `Z` | `fzf` 目錄模糊跳轉 / `zoxide` 歷史目錄跳轉 |
| **檔案操作** | `Space` / `v` | 單檔選取標記 / 連續選取模式 (Visual Mode) |
| | `A` / `U` | 全選目前目錄所有項目 / 清空所有選取標記 |
| | `y` / `x` / `p` | 複製選單（`yy` 複製、`y 1..9` 跨視窗複製） / 剪下 / 貼上（`P` 強制覆蓋） |
| | `m` | 搬移選單（`m 1..9` 跨視窗搬移） / 欄位顯示模式切換 |
| | `r` / `R` | 原地改名 / Regex 批次改名預覽面板 |
| | `a` | 建立檔案或目錄（名稱結尾為 `/` 自動建為目錄） |
| | `d` / `D` | 移至垃圾桶 / 永久刪除 |
| | `u` | 交易式復原 (Undo 上一步複製或搬移) |
| | `C` / `E` | 壓縮為 ZIP / 解壓縮檔案 |
| **視窗佈局** | `Ctrl+v` / `Ctrl+s` | 垂直分割視窗 / 水平分割視窗 |
| | `1` ~ `9` | 切換至指定編號的視窗 |
| | `w` | 視窗操作選單（分割、關閉、全螢幕切換等） |
| | `wr` / `:resize-mode` | 進入連續尺寸微調模式（`h/l` 增減寬度、`j/k` 增減高度、`=` 均等、`Esc` 退出） |
| | `w=` / `:equal` | 將所有視窗重設為平衡均等尺寸 |
| | `wW` / `wH` | 精準增減指定視窗欄數與列數 |
| | `Alt+d` / `wd` | 開啟多視窗 N-Way 目錄差異比對矩陣 (Diff Matrix) |
| **預覽外觀** | `Tab` | 開啟 / 關閉左右雙欄預覽 |
| | `[` / `]` | 預覽模式中切換上一個 / 下一個檔案 |
| | `t` | 主題色彩切換（選單支援即時預覽） |
| | `,` | 檔案排序選單（名稱、大小、修改時間、建立時間、副檔名） |
| | `T` | 背景任務管理面板 |
| | `?` / `F1` | 當前面板情境速查 / 全局完整手冊 |
| | `q` | 離開當前面板 / 退出程式 |

</details>

---

## 設定與擴充 (Configuration & Plugins)

### 設定檔路徑

PaneFM 在啟動時會依序搜尋以下位置：
1. `PANE_FM_CONFIG` 環境變數指定之路徑
2. 執行檔同目錄下的 `config.toml`
3. 使用者設定目錄：
   - **macOS / Linux**：`~/.config/panefm/config.toml`
   - **Windows**：`%APPDATA%\panefm\config.toml`

若設定檔不存在，首次啟動時系統會自動建立附帶繁體中文註解的預設範本。

### 核心設定範例 (`config.toml`)

```toml
[ui]
# 介面主題：catppuccin-mocha, tokyo-night, gruvbox, nord, dracula 等 10 款
theme = "catppuccin-mocha"

[pane]
show_hidden = false          # 預設是否顯示隱藏檔案
default_sort = "natural"     # 預設排序：natural, modified, created, size, extension
default_linemode = "mtime"   # 右側欄位：mtime, btime, size, permissions, none

[navigation]
fast_move_step = 5           # Shift+J / Shift+K 跳躍列數
panel_page_step = 10         # Ctrl+D / Ctrl+U 翻頁列數
```

詳細設定與進階參數請參閱 [config.toml.example](config.toml.example)。

### 終端與外部動作擴充 (`plugins.toml`)

透過 `plugins.toml` 可自訂終端適配器（`wt` 喚起的終端程式）與 `Open with` 外部自訂動作（`O` 喚起），無須修改原始碼或重新編譯。設定細節請參閱 [plugins.toml.example](plugins.toml.example)。

---

## 更新與指令 (Self-Update & CLI)

```bash
# 檢查 GitHub 最新版本並原地自動置換更新
panefm update

# 檢視目前版本
panefm --version

# 檢視命令列說明
panefm --help
```

---

## Vibe Coding 專案故事

PaneFM 採用 **Vibe Coding** 理念與現代 AI 協作開發，源於對每日命令列工作流程效率與終端操作痛點的真實需求。

透過持續架構迭代、功能打磨與嚴謹的自動化測試防護（**500+ 單元與整合測試**），打造專注、順手且具備高度擴充彈性的終端生產力工具。

---

## 授權條款 (License)

本專案採用 [MIT License](LICENSE) 授權發布。歡迎提交 Issue 與 Pull Request。
