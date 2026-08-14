# terminal-file-manager

Minimal terminal file manager prototype built with Rust, `ratatui`, and `crossterm`.

## Run

```bash
cargo run
```

## Controls

- `h j k l`: move / parent / up / enter
- `gg` / `G`: jump to top or bottom
- `Ctrl-w s`: horizontal split
- `Ctrl-w v`: vertical split
- `Ctrl-w h j k l`: switch pane focus
- `Ctrl-w c`: close current pane
- `Ctrl-w o`: keep only current pane
- `:split`, `:vsplit`, `:close`, `:only`
- `q`: quit

## Test

```bash
cargo test
```
