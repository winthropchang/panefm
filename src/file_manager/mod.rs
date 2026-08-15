mod app;
mod bookmark;
mod entry;
mod layout;
mod open;
mod pane;
mod search;
mod trash;
mod ui;

use std::io::{self, Stdout};
use std::process::{Command, Stdio};

use anyhow::Result;
use crossterm::{
    cursor::SetCursorStyle,
    event::{
        self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::config::load_config;

use self::app::{App, RenameMode};
use self::open::{LaunchMode, LaunchSpec};

/// 啟動檔案管理器模組的完整執行流程。
///
/// 參數：無。
/// 回傳：`Result<()>`。
/// - 成功時代表 TUI 已正常執行並完成關閉流程。
/// - 失敗時代表初始化、事件迴圈或還原 terminal 時出現錯誤。
pub(crate) fn run() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

/// 建立並初始化 TUI 所需的 terminal 環境。
///
/// 參數：無。
/// 回傳：`Result<Terminal<CrosstermBackend<Stdout>>>`。
/// - 成功時回傳已進入 alternate screen 並啟用 raw mode 的 terminal。
/// - 失敗時回傳終端初始化相關錯誤。
fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        )
    )?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

/// 將 terminal 從 TUI 狀態恢復成一般命令列狀態。
///
/// 參數：
/// - `terminal: &mut Terminal<CrosstermBackend<Stdout>>`，目前使用中的 terminal 實例。
///
/// 回傳：`Result<()>`。
/// - 成功時代表 raw mode 與 alternate screen 都已正確還原。
/// - 失敗時代表終端清理過程發生錯誤。
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        SetCursorStyle::DefaultUserShape,
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// 執行主事件迴圈，負責持續重畫畫面並接收鍵盤輸入。
///
/// 參數：
/// - `terminal: &mut Terminal<CrosstermBackend<Stdout>>`，要被用來渲染畫面的 terminal。
///
/// 回傳：`Result<()>`。
/// - 成功時代表使用者正常離開應用程式。
/// - 失敗時代表設定檔載入、事件讀取或畫面更新過程出錯。
fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let loaded_config = load_config(&cwd)?;
    let poll_rate = loaded_config.config.ui.poll_rate;
    let mut app = App::new(cwd, loaded_config)?;
    let mut last_cursor_mode = None;

    loop {
        app.poll_background_tasks();
        terminal.draw(|frame| app.render(frame))?;
        sync_cursor_style(terminal, app.rename_cursor_mode(), &mut last_cursor_mode)?;

        if event::poll(poll_rate)?
            && let Event::Key(key) = event::read()?
            && should_handle_key_event(key.kind)
            && !app.handle_key(key)?
        {
            break;
        }

        if let Some(launch) = app.take_pending_launch() {
            run_launch_spec(terminal, launch)?;
            last_cursor_mode = None;
        }
    }

    Ok(())
}

/// 執行外部開啟命令；若需要佔用目前終端，會先暫時離開 TUI。
fn run_launch_spec(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    launch: LaunchSpec,
) -> Result<()> {
    match launch.mode {
        LaunchMode::Detached => {
            Command::new(&launch.program)
                .args(&launch.args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }
        LaunchMode::TerminalBlocking => {
            disable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                SetCursorStyle::DefaultUserShape,
                PopKeyboardEnhancementFlags,
                LeaveAlternateScreen
            )?;
            terminal.show_cursor()?;

            let status = Command::new(&launch.program).args(&launch.args).status()?;

            execute!(
                terminal.backend_mut(),
                EnterAlternateScreen,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                )
            )?;
            enable_raw_mode()?;
            terminal.clear()?;
            terminal.show_cursor()?;

            if !status.success() {
                return Err(anyhow::anyhow!(
                    "external command failed: {}",
                    launch.program
                ));
            }
        }
    }

    Ok(())
}

/// 依照目前 rename 模式同步 terminal 的游標外觀，避免每一輪重複送出相同指令。
///
/// 參數：
/// - `terminal: &mut Terminal<CrosstermBackend<Stdout>>`，目前使用中的 terminal。
/// - `desired_mode: Option<RenameMode>`，畫面目前需要的 rename 游標模式。
/// - `last_mode: &mut Option<RenameMode>`，上一輪已套用的游標模式快取。
///
/// 回傳：`Result<()>`。
/// - 成功時代表游標外觀已正確同步，或本輪不需要更新。
/// - 失敗時代表送出 terminal 控制碼時發生錯誤。
fn sync_cursor_style(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    desired_mode: Option<RenameMode>,
    last_mode: &mut Option<RenameMode>,
) -> Result<()> {
    if *last_mode == desired_mode {
        return Ok(());
    }

    let style = match desired_mode {
        Some(RenameMode::Insert) => SetCursorStyle::SteadyBar,
        Some(RenameMode::Normal) => SetCursorStyle::SteadyBlock,
        None => SetCursorStyle::DefaultUserShape,
    };

    execute!(terminal.backend_mut(), style)?;
    *last_mode = desired_mode;
    Ok(())
}

/// 判斷目前的鍵盤事件是否應該交給應用程式處理。
///
/// 參數：
/// - `kind: KeyEventKind`，crossterm 回報的鍵盤事件種類。
///
/// 回傳：`bool`。
/// - `true` 代表這是有效的按下或長按重複事件，應該交給應用程式。
/// - `false` 代表這是放開事件，應忽略以避免同一組按鍵被處理兩次。
fn should_handle_key_event(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEventKind;

    use super::should_handle_key_event;

    /// 驗證主事件迴圈只會處理按下或長按重複事件，避免放開事件造成快捷鍵重複觸發。
    ///
    /// 參數：無。
    /// 回傳：無。
    #[test]
    fn should_handle_only_press_and_repeat_key_events() {
        assert!(should_handle_key_event(KeyEventKind::Press));
        assert!(should_handle_key_event(KeyEventKind::Repeat));
        assert!(!should_handle_key_event(KeyEventKind::Release));
    }
}
