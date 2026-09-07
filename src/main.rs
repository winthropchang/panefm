//! PaneFM 執行檔入口。
//!
//! 這裡刻意保持輕量，只把控制權交給 library crate。終端初始化、事件迴圈與清理
//! 都集中在 `file_manager`，讓測試可以直接呼叫 library，而不必啟動另一個程序。

use anyhow::Result;
use panefm::updater::{CliCommand, parse_cli_command, run_cli_update};

/// 啟動 PaneFM 終端檔案管理器或處理命令列指令。
///
/// 參數：無。
/// 回傳：`Result<()>`，正常離開時回傳 `Ok(())`，初始化或執行失敗時回傳錯誤。
fn main() -> Result<()> {
    let first_arg = std::env::args_os().nth(1);
    match parse_cli_command(first_arg.as_deref()) {
        CliCommand::Version => {
            println!("panefm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        CliCommand::Help => {
            println!(
                "PaneFM - A fast, pane-first terminal file manager with Vim-style controls\n\n\
                 USAGE:\n    \
                     panefm [OPTIONS]\n    \
                     panefm <COMMAND>\n\n\
                 COMMANDS:\n    \
                     update          檢查 GitHub 最新版本並自動更新當前執行檔\n\n\
                 OPTIONS:\n    \
                     -h, --help      顯示此說明訊息\n    \
                     -V, --version   顯示目前版本號"
            );
            Ok(())
        }
        CliCommand::Update => {
            if let Err(err) = run_cli_update() {
                eprintln!("\n錯誤: {err}");
                std::process::exit(1);
            }
            Ok(())
        }
        CliCommand::RunApp => panefm::run(),
    }
}
