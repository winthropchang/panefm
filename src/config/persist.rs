//! 主題設定就地更新與持久化。

use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::theme::ThemePreset;

/// 將目前選取的主題名稱同步寫入設定檔。
///
/// 參數：
/// - `path: &Path`，要更新的 `config.toml` 路徑。
/// - `preset: ThemePreset`，要保存的主題預設值。
///
/// 回傳：`Result<()>`，成功寫入或建立設定檔時回傳 `Ok(())`。
///
/// 這個函數只修改 `[ui]` 區塊中的 `theme` 欄位，其他設定、註解與格式都會保留。
pub fn persist_theme(path: &Path, preset: ThemePreset) -> Result<()> {
    let theme_line = format!("theme = \"{}\"", preset.name());
    let contents = if path.exists() {
        fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?
    } else {
        String::new()
    };

    let mut output = String::new();
    let mut in_ui = false;
    let mut replaced = false;
    let mut has_ui = false;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_ui = trimmed == "[ui]";
            has_ui |= in_ui;
        }

        if in_ui && trimmed.starts_with("theme") && trimmed[5..].trim_start().starts_with('=') {
            let indentation = &line[..line.len() - line.trim_start().len()];
            output.push_str(indentation);
            output.push_str(&theme_line);
            output.push('\n');
            replaced = true;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    if !replaced {
        if has_ui {
            let mut lines = output.lines().map(str::to_owned).collect::<Vec<_>>();
            let insert_at = lines
                .iter()
                .position(|line| line.trim() == "[ui]")
                .map(|index| index + 1)
                .unwrap_or(0);
            lines.insert(insert_at, theme_line);
            output = lines.join("\n");
            output.push('\n');
        } else {
            output = format!("[ui]\n{theme_line}\n{output}");
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    fs::write(path, output)
        .with_context(|| format!("failed to write config file {}", path.display()))?;
    Ok(())
}
