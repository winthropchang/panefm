# PaneFM

<img src="assets/panefm-icon.png" alt="PaneFM icon" width="160">

PaneFM 是一個以 **多 Panel**、**Vim 操作**與**快速搜尋**為核心的終端檔案管理器。
它適合希望留在鍵盤上完成檔案瀏覽、搜尋、複製、移動及批次操作的使用者。

> PaneFM 目前仍在開發階段，已可在 macOS 與 Windows 編譯執行。

## Vibe Coding 專案

PaneFM 是一個以 **Vibe Coding** 方式開發的軟體，也是我嘗試使用 AI 建立自己真正會使用之工具的專案。
功能方向、操作流程與使用體驗來自我的實際需求；程式碼則透過與 AI 持續討論、實作、測試及修正逐步完成。

這個專案不只是展示 AI 產生的程式碼，而是記錄一個使用者如何把自己的想法轉化成可在 macOS 與 Windows
實際使用的終端檔案管理器。所有功能仍會經過人工操作、測試與調整，並以好用、快速及容易維護為主要目標。

## 為什麼使用 PaneFM？

- **Panel 優先**：每個 Panel 都有獨立的目錄、游標、Preview 與操作狀態。
- **Vim 操作**：使用 `h/j/k/l`、`gg/G`、`n/N` 等熟悉的方式移動。
- **快速搜尋**：使用 `fd` 搜尋檔名、`rg` 搜尋內容、`fzf` 模糊跳轉。
- **多檔案操作**：支援範圍選取、全選、複製、剪下、貼上、刪除及 Trash 還原。
- **跨平台**：以 macOS 與 Windows 為主要支援平台，包含 Windows 磁碟路徑與 SMB 位置。
- **可自訂**：可切換成熟配色主題，並透過 `plugins.toml` 加入 VS Code、Xcode、Git、SVN 等外部動作。

## 功能

- 多 Panel 分割、關閉與快速切換
- 檔案與資料夾建立、改名、Regex 批次改名
- 複製、剪下、貼上、同名覆蓋與跨 Panel 移動
- Trash 列表、批次選取、還原與永久刪除
- `fd` 全域檔名搜尋與即時結果
- `rg` 全域內容搜尋、命中 Preview 與結果內模糊過濾
- `fzf` 遞迴模糊跳轉檔案或目錄
- `zoxide` 常用目錄學習與快速跳轉
- Preview 內容搜尋與循環切換命中結果
- ZIP、TAR、TAR.GZ、GZ 壓縮與解壓縮
- 書籤、排序、linemode、主題與背景 Task 管理
- 本機路徑、Windows 磁碟路徑與 `smb://` 路徑跳轉
- 可設定的外部程式與自訂 Open With 動作

## 安裝

### 必要環境

目前 PaneFM 需要從原始碼編譯，請先安裝：

- Rust 1.85 或更新版本
- Git
- `fd`
- `fzf`
- `ripgrep`（執行命令為 `rg`）
- `zoxide`

macOS 可使用 Homebrew 安裝外部工具：

```bash
brew install fd fzf ripgrep zoxide
```

Windows 可使用 WinGet、Scoop 或 Chocolatey 安裝上述工具。安裝後請確認
`fd`、`fzf`、`rg`、`zoxide` 都能從 Terminal 的 `PATH` 執行。

PaneFM 內可輸入 `:status` 查看目前缺少哪些工具。

### 從原始碼安裝

```bash
git clone https://github.com/winthropchang/panefm.git
cd panefm
cargo build --release
```

編譯結果：

- macOS：`target/release/panefm`
- Windows：`target\release\panefm.exe`

可以直接執行該檔案，或把它放進系統 `PATH` 後，在任何目錄輸入：

```bash
panefm
```

未來建立正式 GitHub Release 後，預計提供 Homebrew 與 Windows 套件管理器安裝；
目前尚未發布，因此 README 不會提供無法使用的 `brew install panefm` 或 WinGet 命令。

## 快速開始

啟動後先記住這幾個操作即可：

| 按鍵 | 功能 |
| --- | --- |
| `h/j/k/l` | 上一層、向下、向上、進入目錄 |
| `Enter` / `o` | 使用預設方式開啟檔案 |
| `a` | 建立檔案或資料夾；名稱以 `/` 結尾會建立資料夾 |
| `r` | 重新命名 |
| `v` | 進入或離開範圍選取模式 |
| `y` / `x` | 複製 / 剪下 |
| `p` / `P` | 貼上並詢問覆蓋 / 直接覆蓋 |
| `d` / `D` | 移到 Trash / 永久刪除 |
| `s` / `S` | 用 `fd` 搜尋檔名 / 用 `rg` 搜尋內容 |
| `z` / `Z` | 用 `fzf` 搜尋目前目錄樹 / 用 `zoxide` 跳轉常用目錄 |
| `f` | 模糊過濾目前列表 |
| `Tab` | 開啟或關閉 Preview |
| `w` | 開啟 Panel 操作選單 |
| `1..9` / `0` | 直接切換到指定 Panel |
| `:` | 開啟命令輸入介面 |
| `~` / `F1` | 顯示完整 Help，可搜尋並直接執行命令 |
| `q` | 離開 PaneFM |

多按鍵操作在按下第一個按鍵後會顯示提示面板，不需要事先背下全部命令。

### 常用工作流程

搜尋檔名並快速過濾：

1. 按 `s`，輸入關鍵字後按 `Enter`。
2. 結果會分批顯示，可立即使用 `j/k` 移動。
3. 結果太多時按 `f`，使用模糊過濾縮小範圍。
4. 按 `Enter` 或 `l` 跳到選取項目。

選取多個項目並複製：

1. 按 `v` 進入選取模式。
2. 使用 `j/k` 選取範圍，再按 `v` 或 `Esc` 結束選取。
3. 按 `y` 複製，移動到目的目錄後按 `p` 貼上。

建立多個 Panel：

1. 按 `w` 開啟 Panel 選單。
2. 使用 `h/j/k/l` 在左、下、上、右建立新 Panel。
3. 直接按 `1..9` 或 `0` 切換 Panel。

## 設定

請參考 [config.toml.example](config.toml.example)。PaneFM 依序讀取第一個存在的設定檔：

1. `PANE_FM_CONFIG` 指定的檔案
2. 目前目錄的 `config.toml`
3. macOS：`$XDG_CONFIG_HOME/panefm/config.toml`
4. macOS：`~/.config/panefm/config.toml`
5. Windows：`%APPDATA%\panefm\config.toml`

舊版的 `TFM_CONFIG` 與 `terminal-file-manager` 設定目錄仍可讀取，方便既有使用者遷移。

設定檔可調整主題、隱藏檔、預設排序、搜尋上限、移動步長與 UI 尺寸。
字體和字體大小由 Windows Terminal、iTerm2 或其他 Terminal 控制，不由 PaneFM 設定。

## 自訂動作

PaneFM 可透過 [plugins.toml.example](plugins.toml.example) 加入公司或個人環境專用的動作，
不需要把 VS Code、Xcode、Git、SVN 或其他工具寫死在程式裡：

```toml
[[actions.open_with]]
name = "VS Code"
scope = "dir"
mode = "detached"
mac_command = "open -a 'Visual Studio Code' {path}"
windows_command = "code {path}"
```

把 `plugins.toml` 放在 `config.toml` 旁邊，使用 `O` 或 `Shift-Enter` 開啟
Open With 面板即可選擇自訂動作。

## 外觀與圖示

PaneFM 是在 Terminal 內執行的 CLI/TUI 工具，因此不需要把桌面圖示嵌入 macOS 或
Windows 執行檔。`assets/panefm-icon.svg` 與 `assets/panefm-icon.png` 用於 GitHub、
README 與 Release 品牌顯示。未來若製作真正的 `.app` 或桌面安裝程式，再另外加入
`.icns` 或 `.ico`。

## 開發

```bash
cargo fmt --all
cargo test
cargo run
```

開發前請閱讀 [DEVELOPMENT_GUIDELINES.md](DEVELOPMENT_GUIDELINES.md)。需要接續目前進度時，
請先閱讀 [PROJECT_HANDOFF.md](PROJECT_HANDOFF.md) 與 [DEV_NOTES.md](DEV_NOTES.md)。

## 專案狀態

PaneFM 已完成主要檔案管理與搜尋流程，但仍在持續進行 Windows 真機驗證、效能優化及
操作一致性整理。若準備在重要資料上使用，建議先備份並從測試目錄開始。
