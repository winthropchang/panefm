# terminal-file-manager

Minimal terminal file manager prototype built with Rust, `ratatui`, and `crossterm`.

## Run

```bash
cargo run
```

`z` / `:jump` 使用系統 `PATH` 中已安裝的 `fzf`；找不到時會顯示缺少依賴提示。
目前內建 fallback 已包含：

- macOS `arm64`
- macOS `amd64`
- Windows `arm64`
- Windows `amd64`

只有找不到系統命令時，才會把對應平台的 binary 解包到本機快取目錄後使用，因此已安裝工具時不會重複解包。

`rg` 與 `zoxide` 也使用系統 `PATH` 中的命令。程式不再攜帶或解包這三個第三方 binary，啟動時會列出缺少的依賴。

之後接手開發、開新對話續做、查看目前功能與開發紀錄，請優先閱讀 [PROJECT_HANDOFF.md](/Users/otto/Documents/terminal-file-manager/PROJECT_HANDOFF.md)。

The app loads config from the first existing path in this order:

1. `TFM_CONFIG=/absolute/path/to/config.toml`
2. `./config.toml`
3. `$XDG_CONFIG_HOME/terminal-file-manager/config.toml`
4. `~/.config/terminal-file-manager/config.toml`

## Controls

- `h j k l`: move / parent / up / enter
- `gg` / `G`: jump to top or bottom
- `yy`: copy the selected file or directory into the internal clipboard
- `Space`: toggle mark on the selected file or directory
- `Ctrl-r`: invert marks for all visible entries in the current pane
- `Y` / `X`: clear copied / cut clipboard state
- `x`: cut the selected file or directory into the internal clipboard
- `p`: paste the clipboard item into the current directory
- `P`: paste with overwrite if the destination name already exists
- `a`: create a new entry in the current directory
  - end the path with `/` to create a directory
  - omit the trailing `/` to create a file
  - nested paths like `test/gg.txt` create missing parent directories automatically
- `f`: open the filter box on the right side and filter results while typing
  - first `Esc`: hide the input box but keep the filtered result list
  - second `Esc`: clear the filter and return to the normal list
- `z`: use system `fzf` to recursively jump to a file or directory under the current pane root
- `.`: toggle hidden files and directories
- `,`: open the sort panel
  - `,m` / `,M`: sort by modified time (forward / reverse)
  - `,b` / `,B`: sort by birth time (forward / reverse)
  - `,e` / `,E`: sort by extension (forward / reverse)
  - `,a` / `,A`: sort alphabetically (forward / reverse)
  - `,n` / `,N`: sort naturally (forward / reverse)
  - `,s` / `,S`: sort by size (forward / reverse)
  - `,r`: sort randomly
- `Ctrl-w s`: horizontal split
- `Ctrl-w v`: vertical split
- `Ctrl-w h j k l`: switch pane focus
- `Ctrl-w c`: close current pane
- `Ctrl-w o`: keep only current pane
- `d`: move the selected file or directory to the internal trash
- `D`: permanently delete the selected or marked entries after confirmation
- `v`: enter visual selection mode; use `j/k` to extend the range, then press `v` or `Esc` to finish
- `r`: open rename dialog for the selected item
- `R`: open regex batch rename command input; after entering the pattern and replacement, press `Enter` again in the preview to apply
- `:rename-regex <pattern> <replace>` (short alias: `:reg <pattern> <replace>`), for example `:reg '^(.*)\\.txt$' '$1.md'`
- `:rename`: open rename dialog for the selected item
- `:create`, `:create <path>`
- `:copy`, `:cut`, `:paste`
- `:restore`: restore the most recently trashed item
- `:split`, `:vsplit`, `:close`, `:only`
- `t`, then `l`: open the `Theme List` picker
- `t`, then `n`: switch to the next theme
- `t`, then `t`: open the trash actions
- `t`, then `u`: restore the most recently trashed item
- `:theme list`, `:theme next`, `:theme <name>`
- `q`: quit

## Test

```bash
cargo test
```

## Theme

Colors are centralized in `src/theme.rs`.
Change `Theme::default_theme()` to update the active palette, or use the built-in presets like `Theme::forest_theme()` and `Theme::ocean_theme()` as a starting point.

Each preset is selected through `ThemePreset` and converted into a `Theme`.
If you want to tune the current look:

- Edit the color values inside `Theme::default_theme()`
- Keep the field meanings stable: `accent`, `focus_border`, `muted`, `selection_bg`, `danger`
- Add a new preset by extending `ThemePreset`, then add its palette in `impl From<ThemePreset> for Theme`
- Inside the theme picker, use `j/k` or arrow keys to move, `Enter` to apply, and `Esc` to cancel

## Config

Start from [config.toml.example](/Users/otto/Documents/terminal-file-manager/config.toml.example) and [plugins.toml.example](/Users/otto/Documents/terminal-file-manager/plugins.toml.example).

Available keys:

```toml
[ui]
theme = "catppuccin-mocha" # also: dracula, nord, tokyo-night, gruvbox-dark, everforest...
poll_rate_ms = 150

[ui.preview]
height = 8
focus_list_height = 6

[ui.dialog.confirm]
width_percent = 60
height = 5

[ui.dialog.theme_picker]
width_percent = 42
height = 20

[pane]
show_hidden = false
default_sort = "natural" # alphabetical / natural / size / modified / created / extension / random
default_sort_reverse = false

[search]
global_search_limit = 200
global_search_chunk_size = 24
show_loading = true
fzf_follow_links = true

[navigation]
fast_move_step = 5
panel_page_step = 10

[behavior]
cancel_search_on_leave = true
```

字體與字體大小由 Windows Terminal、iTerm2 或其他外部 Terminal 控制，TUI 不會
嘗試修改這些設定；請直接在外部 Terminal 的設定中調整。

Custom `Open with` actions belong in `plugins.toml` next to `config.toml`:

```toml
[actions]

[[actions.open_with]]
name = "Xcode"
scope = "dir" # file / dir / both
mode = "detached" # detached / terminal
mac_command = "open -a Xcode {path}"

[[actions.open_with]]
name = "Git log"
scope = "both"
mode = "terminal"
command = "git -C {parent} log --oneline"
windows_command = "git -C {parent} log --oneline"
```

`plugins.toml` 裡的 `actions.open_with` 目前會直接出現在 `Open with` 面板。
`plugins.toml` 可以直接寫註解，TOML 的註解語法是 `#`。

欄位定義：

- `name`：面板中顯示給使用者看的名稱。
- `scope`：這個動作要套用在什麼目標上，可用 `file`、`dir`、`both`。
- `mode`：啟動方式，可用 `detached`、`terminal`。
- `command`：所有平台共用的預設指令。
- `mac_command`：只在 macOS 使用，優先於 `command`。
- `windows_command`：只在 Windows 使用，優先於 `command`。

模式建議：

- `detached`：適合 VS Code、Xcode、Finder、Explorer 這類交給外部 App 處理的動作。
- `terminal`：適合 Vim、git、svn 這類要直接佔用終端並顯示輸出的指令。

可用佔位字串：

- `{path}`：目前選取項目的完整路徑
- `{parent}`：目前選取項目的父目錄
- `{name}`：目前顯示名稱
- `{stem}`：去掉副檔名後的檔名
