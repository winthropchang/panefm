# terminal-file-manager

Minimal terminal file manager prototype built with Rust, `ratatui`, and `crossterm`.

## Run

```bash
cargo run
```

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
- `.`: toggle hidden files and directories
- `Ctrl-w s`: horizontal split
- `Ctrl-w v`: vertical split
- `Ctrl-w h j k l`: switch pane focus
- `Ctrl-w c`: close current pane
- `Ctrl-w o`: keep only current pane
- `d`: delete selected file or directory with confirmation
- `r`: open rename dialog for the selected item
- `:rename`: open rename dialog for the selected item
- `:create`, `:create <path>`
- `:copy`, `:cut`, `:paste`
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

- `theme = "default" | "forest" | "ocean"`
- `poll_rate_ms = 150`
- `[confirm_dialog].width_percent`
- `[confirm_dialog].height`
- `[theme_picker].width_percent`
- `[theme_picker].height`
