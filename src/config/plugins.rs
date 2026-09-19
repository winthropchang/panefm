//! 外掛與自訂動作（actions, terminal, terminals）設定解析與驗證。

use anyhow::{Context, Result, bail};

use super::file_types::{ActionsConfigFile, TerminalLauncherFile, TerminalPluginFile};
use super::schema::{
    ActionLaunchMode, ActionTargetScope, AppConfig, CustomOpenActionConfig, TerminalLauncherConfig,
    TerminalPluginConfig,
};

/// 套用並驗證 `actions` 區塊設定。
pub(crate) fn apply_actions_config(
    config: &mut AppConfig,
    actions: ActionsConfigFile,
) -> Result<()> {
    let Some(raw_actions) = actions.open_with else {
        return Ok(());
    };

    let mut parsed = Vec::with_capacity(raw_actions.len());
    for (index, raw) in raw_actions.into_iter().enumerate() {
        let name = raw
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .with_context(|| format!("actions.open_with[{index}].name is required"))?;

        if raw.command.as_deref().is_none()
            && raw.mac_command.as_deref().is_none()
            && raw.windows_command.as_deref().is_none()
        {
            bail!(
                "actions.open_with[{index}] must define at least one of command / mac_command / windows_command"
            );
        }

        let scope = match raw
            .scope
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
        {
            None => ActionTargetScope::Both,
            Some(value) if value == "both" => ActionTargetScope::Both,
            Some(value) if value == "file" => ActionTargetScope::File,
            Some(value) if value == "dir" || value == "directory" => ActionTargetScope::Directory,
            Some(value) => {
                bail!(
                    "unknown actions.open_with[{index}].scope: {value}. available: file, dir, both"
                );
            }
        };

        let mode = match raw
            .mode
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
        {
            None => ActionLaunchMode::Detached,
            Some(value) if value == "detached" => ActionLaunchMode::Detached,
            Some(value) if value == "terminal" || value == "terminal_blocking" => {
                ActionLaunchMode::TerminalBlocking
            }
            Some(value) => {
                bail!(
                    "unknown actions.open_with[{index}].mode: {value}. available: detached, terminal"
                );
            }
        };

        parsed.push(CustomOpenActionConfig {
            name,
            scope,
            mode,
            command: raw.command.map(|value| value.trim().to_string()),
            mac_command: raw.mac_command.map(|value| value.trim().to_string()),
            windows_command: raw.windows_command.map(|value| value.trim().to_string()),
        });
    }

    config.actions.open_with = parsed;
    Ok(())
}

/// 驗證並套用 `[terminal]` 自訂啟動器。
///
/// 參數：`config: &mut AppConfig`，最終設定；`terminal: TerminalLauncherFile`，原始欄位。
/// 回傳：`Result<()>`；完全沒有命令時回傳設定錯誤，避免 `wt` 靜默失效。
pub(crate) fn apply_terminal_launcher_config(
    config: &mut AppConfig,
    terminal: TerminalLauncherFile,
) -> Result<()> {
    let normalize = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let launcher = TerminalLauncherConfig {
        command: normalize(terminal.command),
        mac_command: normalize(terminal.mac_command),
        windows_command: normalize(terminal.windows_command),
    };
    if launcher.command.is_none()
        && launcher.mac_command.is_none()
        && launcher.windows_command.is_none()
    {
        bail!("terminal must define at least one of command / mac_command / windows_command");
    }
    config.actions.terminal = Some(launcher);
    Ok(())
}

/// 套用並驗證 `terminals` 自訂終端外掛清單設定。
pub(crate) fn apply_terminal_plugins_config(
    config: &mut AppConfig,
    terminals: Vec<TerminalPluginFile>,
) -> Result<()> {
    let normalize = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    for item in terminals {
        let name = match item
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(name) => name.to_string(),
            None => bail!("terminal plugin must define a non-empty name"),
        };
        let match_env = item
            .match_env
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let match_process = item
            .match_process
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let command = normalize(item.command);
        let mac_command = normalize(item.mac_command);
        let windows_command = normalize(item.windows_command);

        if command.is_none() && mac_command.is_none() && windows_command.is_none() {
            bail!(
                "terminal plugin '{}' must define at least one of command / mac_command / windows_command",
                name
            );
        }

        config.actions.terminals.push(TerminalPluginConfig {
            name,
            match_env,
            match_process,
            command,
            mac_command,
            windows_command,
        });
    }
    Ok(())
}
