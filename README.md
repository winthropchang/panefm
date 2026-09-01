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
- **N-Way Diff 矩陣比對**：支援 2、3、4+ 個 Panel 的目錄與檔案即時比對，一鍵分類差異並呼叫外部比對工具。
- **Vim 操作**：使用 `h/j/k/l`、`gg/G`、`n/N` 等熟悉的方式移動。
- **快速搜尋**：使用 `fd` 搜尋檔名、`rg` 搜尋內容、`fzf` 模糊跳轉。
- **多檔案操作**：支援範圍選取、全選、複製、剪下、貼上、刪除及 Trash 還原。
- **跨平台**：以 macOS 與 Windows 為主要支援平台，包含 Windows 磁碟路徑與 SMB 位置。
- **可自訂擴充**：可切換成熟配色主題，透過 `plugins.toml` 自訂終端適配器（`[[terminals]]`）與 `Open with` 外部工具（VS Code、Xcode、Git 等）。

## 功能

- 多 Panel 分割、關閉與快速切換
- **智慧終端整合與外掛系統（`wt` / `:terminal` / `[[terminals]]`）**：
  - 支援 `[[terminals]]` 自訂外掛清單，透過環境變數或程序樹自動識別宿主終端
  - 內建 WezTerm、Alacritty、Windows Terminal、iTerm2、Kitty、Ghostty 適配
  - 內建 **OSC 7 目錄廣播**，終端原生快捷鍵（`Ctrl+Shift+T` / `Cmd+T`）秒開目前目錄 Tab
  - 100% 獨立程序群組（`CREATE_NEW_PROCESS_GROUP`）與 **TrustView 企業安全保護環境繼承**
- **N-Way 多 Panel 目錄與檔案矩陣比對（Diff Matrix）**：
  - 支援 2 ~ N 個 Panel 之間的目錄結構與檔案內容同時比對
  - 背景非阻塞掃描、`.gitignore` 過濾支援、Zero-I/O 智慧分類與首尾採樣 Hash
  - 整合 `delta`、`difft`、`nvim -d`、`vimdiff` 等外部專業比對工具
  - 支援狀態篩選（全部／僅相異／僅獨有／完全相同）、清單搜尋與快速重新掃描
- Finder、Explorer 或其他程式改動目前目錄時自動刷新相關 Panel
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

正式版本會在 GitHub Releases 提供 Windows x64、macOS Apple Silicon 與 macOS Intel
執行檔。未來預計再提供 Homebrew 與 Windows 套件管理器安裝；目前 README 不會提供
尚未設定完成的 `brew install panefm` 或 WinGet 命令。

下載 macOS 執行檔後，第一次執行前需要加上執行權限：

```bash
chmod +x panefm-macos-arm64 # Apple Silicon
# 或
chmod +x panefm-macos-x64   # Intel
```

查看目前版本：

```bash
panefm --version
# panefm 0.1.0
```

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
| `u` | 復原最近一次 copy 或 move 批次，可連續復原最多 20 筆 |
| `C` | 壓縮選取項目為 ZIP |
| `E` | 解壓縮選取項目（支援 `.zip`、`.tar.gz`、`.tar`、`.gz`） |
| `d` / `D` | 移到 Trash / 永久刪除 |
| `s` / `S` | 用 `fd` 搜尋檔名 / 用 `rg` 搜尋內容 |
| `z` / `Z` | 用 `fzf` 搜尋目前目錄樹 / 用 `zoxide` 跳轉常用目錄 |
| `f` | 模糊過濾目前列表 |
| `Tab` | 開啟或關閉 Preview |
| `w` | 開啟 Panel 操作選單 |
| `wd` / `Alt+d` | 開啟多 Panel 目錄 Diff 矩陣比對 |
| `wD` | 預填 `:diff ` 指定比對 Panel（例如 `:d 1 2`） |
| `wt` | 在 active Panel 目前目錄開啟新終端 |
| `1..9` / `0` | 直接切換到指定 Panel |
| `T` | 開啟任務管理面板（支援 `v` 範圍選取、`Space` 標記、`d`/`D` 刪除與清空、`x` 取消任務） |
| `~` / `F1` | 顯示全局完整說明字典（Global Help Dictionary），可搜尋並直接執行命令 |
| `?` | 顯示當前畫面／面板專屬快捷鍵指南（Context Cheatsheet，支援 `:cheatsheet` / `:cheat`） |
| `q` | 離開 PaneFM |

多按鍵操作在按下第一個按鍵後會顯示提示面板，不需要事先背下全部命令。

### 常用工作流程

多目錄／檔案 Diff 矩陣比對：

1. 按 `w` 接著按 `h/l` 開啟 2 個或多個 Panel，各自進入想比對的目錄。
2. 按 **`w` ➔ `d`**（或快捷鍵 **`Alt+d`**，或輸入 **`:d`** / **`:d 1 2`**）。
3. 畫面將立即進入全螢幕 **Diff Matrix** 比對矩陣：
   - 每個 Panel 狀態以顏色標記：`[#1]`（僅 Panel 1）、`[#2]`（僅 Panel 2）、`[Diff]`（內容不同）、`[Identical]`（完全相同）。
   - 按 `f` 循環切換篩選（全部 ➔ 僅差異 ➔ 僅各 Panel 獨有 ➔ 完全相同）。
   - 按 `i` 快速切換 **`.gitignore` 規則**（預設啟用以排除 `target/`、`node_modules/` 等巨量編譯目錄；切換後會即時包含 `target/` 並重新掃描）。
   - 按 `.` 快速切換 **隱藏檔**（包含或排除隱藏檔案）。
   - 按 `/` 在比對清單中快速搜尋檔名。
   - 選中相異檔案後按 `Enter`，自動啟動系統已安裝的最佳外部比對工具（`delta` / `difft` / `nvim -d` / `vimdiff`）檢視細部內容差異。
   - 按 `q` 或 `Esc` 隨時退出比對回到原本 Panel。

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

若公司要求終端必須經由 TrustView 等保護軟體啟動，可在 `plugins.toml` 的
`[terminal]` 設定平台專用命令；`{path}` 會替換成 active Panel 目錄。未設定時，
Windows 會直接建立繼承 PaneFM 安全權杖的新 console；macOS 會優先延續目前的 iTerm2、
Terminal、WezTerm、Ghostty 或 Warp，無法辨識時才使用 Terminal.app。

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

## 外掛擴充與自訂動作 (`plugins.toml`)

PaneFM 支援透過 `plugins.toml` 自由擴充**終端適配器**與 **`Open with` 外部動作**，無需修改原始碼或重新編譯。

詳細設定與範例請參考 [plugins.toml.example](plugins.toml.example)。

### 1. 自訂終端適配器 (`[[terminals]]`)

當在 PaneFM 中按下 **`wt`** 或輸入 **`:terminal`** 時，系統會依序檢查 `[[terminals]]` 列表：
只要符合當前環境變數（`match_env`）或父程序名稱（`match_process`），就會自動以當前目錄 `{path}` 喚起該終端：

```toml
# 範例 1：macOS Kitty 終端（在現有視窗直接開新 Tab）
[[terminals]]
name = "kitty"
match_env = ["KITTY_WINDOW_ID", "KITTY_PID"]
match_process = ["kitty"]
mac_command = "kitty @ launch --type=tab --cwd={path}"

# 範例 2：跨平台 Ghostty 終端（開新視窗並定位至當前目錄）
[[terminals]]
name = "ghostty"
match_env = ["GHOSTTY_RESOURCES_DIR"]
match_process = ["ghostty.exe", "ghostty"]
mac_command = "open -a Ghostty {path}"
windows_command = "ghostty.exe --working-directory {path}"

# 範例 3：公司 TrustView 加密保護終端
[[terminals]]
name = "trustview-safe-terminal"
match_env = ["TRUSTVIEW_SESSION"]
windows_command = "TrustViewLauncher.exe --cwd {path}"
```

> 💡 **小撇步**：
> - **如何查詢終端的環境變數？** 在該終端執行 `Get-ChildItem env:`（PowerShell）或 `env`（macOS/Linux），即可看見該終端注入的專屬環境變數。
> - **不想查環境變數？** 直接寫 `match_process = ["軟體名稱"]`（如 `["kitty"]`、`["ghostty.exe"]`）即可，PaneFM 會自動追蹤祖先程序樹！
> - **開新 Tab（分頁）**：PaneFM 內建標準 **OSC 7 目錄廣播**。在支援的終端（WezTerm、Kitty、iTerm2 等）直接按終端原生快捷鍵（`Ctrl+Shift+T` / `Cmd+T`），也會直接在當前目錄開新 Tab！
> - **TrustView 安全性保證**：若 PaneFM 是在 TrustView 保護下啟動，不論是開新分頁還是開新視窗，都會 100% 繼承安全權杖，可正常讀取保護區檔案。

---

### 2. 自訂 `Open with` 動作 (`[[actions.open_with]]`)

按下 `O` 或 `Shift-Enter` 可開啟 Open With 彈出面板，自訂任何日常開發指令：

```toml
[[actions.open_with]]
name = "VS Code"
scope = "dir"
mode = "detached"
mac_command = "open -a 'Visual Studio Code' {path}"
windows_command = "code {path}"

[[actions.open_with]]
name = "Git log"
scope = "both"
mode = "terminal"
command = "git -C {parent} log --oneline"
```

把 `plugins.toml` 放在 `config.toml` 同級目錄即可自動生效。

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

### 自動建置與發布

每次 push 到任意 branch 時，GitHub Actions 會編譯以下三個測試用 artifact，保留 14 天：

- `panefm-windows-x64.exe`
- `panefm-macos-arm64`
- `panefm-macos-x64`

可從 GitHub 的 **Actions** 頁面開啟該次 workflow，在 **Artifacts** 區塊下載。一般 push
只會產生 CI artifact，不會建立 GitHub Release。

準備正式發布時，先把 `Cargo.toml` 的 `version` 更新並提交；再建立相同版本的 tag：

```bash
git tag v0.1.0
git push origin v0.1.0
```

tag 必須符合 `vX.Y.Z`，且版本必須和 `Cargo.toml` 完全一致。檢查通過後，GitHub Actions
才會建立正式 Release、自動產生 release notes，並上傳三個平台執行檔。

開發前請閱讀 [DEVELOPMENT_GUIDELINES.md](DEVELOPMENT_GUIDELINES.md)。需要接續目前進度時，
請先閱讀 [PROJECT_HANDOFF.md](PROJECT_HANDOFF.md) 與 [DEV_NOTES.md](DEV_NOTES.md)。

## 專案狀態

PaneFM 已完成主要檔案管理與搜尋流程，但仍在持續進行 Windows 真機驗證、效能優化及
操作一致性整理。若準備在重要資料上使用，建議先備份並從測試目錄開始。
