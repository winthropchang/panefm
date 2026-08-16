# terminal-file-manager

Minimal terminal file manager prototype built with Rust, `ratatui`, and `crossterm`.

## Run

```bash
cargo run
```

`z` / `:jump` 使用內建的 `fzf` binary。
目前專案已內含：

- macOS `arm64`
- macOS `amd64`
- Windows `arm64`
- Windows `amd64`

執行時會自動把對應平台的 `fzf` 解包到本機快取目錄後使用，不需要另外安裝。

開發規範請參考 [DEVELOPMENT_GUIDELINES.md](/Users/otto/Documents/terminal-file-manager/DEVELOPMENT_GUIDELINES.md)。

The app loads config from the first existing path in this order:

1. `TFM_CONFIG=/absolute/path/to/config.toml`
2. `./config.toml`
3. `$XDG_CONFIG_HOME/terminal-file-manager/config.toml`
4. `~/.config/terminal-file-manager/config.toml`

## Controls

- `h j k l`: move / parent / up / enter
- `gg` / `G`: jump to top or bottom
- `yy`: copy the selected file or directory into the internal clipboard
- `x`: cut the selected file or directory into the internal clipboard
- `p`: paste the clipboard item into the current directory
- `a`: create a new entry in the current directory
  - end the path with `/` to create a directory
  - omit the trailing `/` to create a file
  - nested paths like `test/gg.txt` create missing parent directories automatically
- `f`: open the filter box on the right side and filter results while typing
  - first `Esc`: hide the input box but keep the filtered result list
  - second `Esc`: clear the filter and return to the normal list
- `z`: use bundled `fzf` to recursively jump to a file or directory under the current pane root
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
- `r`: open rename dialog for the selected item
- `:rename`: open rename dialog for the selected item
- `:create`, `:create <path>`
- `:copy`, `:cut`, `:paste`
- `:restore`: restore the most recently trashed item
- `:split`, `:vsplit`, `:close`, `:only`
- `:theme`: open the theme picker
- `:theme next`, `:theme default`, `:theme forest`, `:theme ocean`
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

Start from [config.toml.example](/Users/otto/Documents/terminal-file-manager/config.toml.example).

Available keys:

```toml
[ui]
theme = "default" # or forest / ocean
poll_rate_ms = 150

[ui.preview]
height = 8
focus_list_height = 6

[ui.dialog.confirm]
width_percent = 60
height = 5

[ui.dialog.theme_picker]
width_percent = 42
height = 8

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
