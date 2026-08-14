mod app;
mod entry;
mod layout;
mod pane;
mod ui;

use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::config::load_config;

use self::app::App;

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
    execute!(stdout, EnterAlternateScreen)?;
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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
    let poll_rate = loaded_config.config.poll_rate;
    let mut app = App::new(cwd, loaded_config)?;

    loop {
        terminal.draw(|frame| app.render(frame))?;

        if event::poll(poll_rate)?
            && let Event::Key(key) = event::read()?
            && !app.handle_key(key)?
        {
            break;
        }
    }

    Ok(())
}
