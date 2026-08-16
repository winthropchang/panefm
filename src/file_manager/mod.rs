mod app;
mod archive;
mod bookmark;
mod bundled;
mod copy;
mod entry;
mod fzf;
mod layout;
mod open;
mod pane;
mod platform;
mod rg;
mod search;
mod smb;
mod trash;
mod ui;

use std::io::{self, BufRead, BufReader, Stdout, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use crossterm::{
    cursor::SetCursorStyle,
    event::{
        self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ignore::WalkBuilder;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::config::load_config;

use self::app::{App, FzfJumpRequest, RenameMode};
use self::fzf::bundled_fzf_command;
use self::open::{LaunchMode, LaunchSpec};

/// 啟動檔案管理器模組的完整執行流程。
///
/// 參數：無。
/// 回傳：`Result<()>`。
/// - 成功時代表 TUI 已正常執行並完成關閉流程。
/// - 失敗時代表初始化、事件迴圈或還原 terminal 時出現錯誤。
pub(crate) fn run() -> Result<()> {
    if let Some(result) = maybe_run_internal_command() {
        return result;
    }

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
    enter_tui_mode(&mut stdout)?;
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
    leave_tui_mode(terminal.backend_mut())?;
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

        if let Some(queued) = app.take_pending_launch() {
            let result = run_launch_spec(terminal, queued.launch)
                .map_err(|error| io::Error::other(error.to_string()));
            app.finish_launch_task(queued.task_id, result);
            last_cursor_mode = None;
        }

        if let Some(request) = app.take_pending_fzf_jump() {
            match run_fzf_jump(terminal, &request) {
                Ok(selected_line) => {
                    app.apply_fzf_jump_selection(request, selected_line.as_deref())
                }
                Err(error) => app.status = format!("jump failed: {error}"),
            }
            last_cursor_mode = None;
        }
    }

    Ok(())
}

/// 暫時離開 TUI，交給 `fzf` 互動選擇目前 pane 的單目錄項目，再把結果帶回應用程式。
fn run_fzf_jump(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    request: &FzfJumpRequest,
) -> Result<Option<String>> {
    let jump_started_at = Instant::now();
    let fzf_command = bundled_fzf_command()?;

    disable_raw_mode()?;
    leave_tui_mode(terminal.backend_mut())?;
    terminal.show_cursor()?;

    let root_dir = request.root_dir.clone();
    let show_hidden = request.show_hidden;
    let follow_links = request.follow_links;
    let helper_command = build_fzf_helper_command(&root_dir, show_hidden, follow_links)
        .context("build jump helper command")?;
    let child = spawn_fzf_process(&fzf_command, &helper_command).context("spawn fzf")?;
    let fzf_wait_started_at = Instant::now();
    let output_result = child.wait_with_output().context("wait for fzf");
    debug_timing_log("fzf wait", fzf_wait_started_at);

    enter_tui_mode(terminal.backend_mut())?;
    enable_raw_mode()?;
    terminal.clear()?;
    terminal.show_cursor()?;

    let output = output_result?;
    if !output.status.success() {
        debug_timing_log("fzf jump total", jump_started_at);
        return Ok(None);
    }

    let selected = BufReader::new(output.stdout.as_slice())
        .lines()
        .collect::<io::Result<Vec<String>>>()
        .context("read fzf output")?;
    let selected = selected
        .into_iter()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string());
    if let Some(selected) = selected.as_deref() {
        debug_timing_message(&format!("fzf selected: {selected}"));
    }
    debug_timing_log("fzf jump total", jump_started_at);
    Ok(selected)
}

/// 若目前執行的是內部 helper 子命令，就直接執行後結束，不進入 TUI。
fn maybe_run_internal_command() -> Option<Result<()>> {
    let mut args = std::env::args_os();
    let _ = args.next();

    match args.next().as_deref() {
        Some(command) if command == std::ffi::OsStr::new("__stream-fzf-jump") => {
            Some(run_fzf_stream_helper(args))
        }
        _ => None,
    }
}

/// 執行 `fzf` 候選串流 helper，把掃描結果直接寫到 stdout。
///
/// 這樣主程式可以在 `fzf` 結束時立刻 kill 掉 helper process，
/// 避免同一個行程內的背景 thread 還在遞迴掃描，拖慢回到 TUI 的速度。
fn run_fzf_stream_helper<I>(mut args: I) -> Result<()>
where
    I: Iterator<Item = std::ffi::OsString>,
{
    let root_dir = std::path::PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("missing jump helper root dir"))?,
    );
    let show_hidden = parse_helper_bool_flag(args.next(), "show_hidden")?;
    let follow_links = parse_helper_bool_flag(args.next(), "follow_links")?;
    let cancel = AtomicBool::new(false);
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    stream_fzf_candidates(&root_dir, show_hidden, follow_links, &cancel, &mut handle)
        .context("stream jump candidates")?;
    Ok(())
}

/// 建立 `fzf` 子行程，並套用本程式要求的固定互動鍵位。
///
/// 這裡不依賴使用者自己的 `FZF_DEFAULT_OPTS` 去決定核心行為，
/// 避免不同 terminal / shell 環境導致 `Esc` 無法穩定返回 TUI。
fn spawn_fzf_process(
    fzf_command: &std::ffi::OsString,
    helper_command: &str,
) -> io::Result<std::process::Child> {
    Command::new(fzf_command)
        .arg("--bind")
        .arg(fzf_bindings())
        .env("FZF_DEFAULT_COMMAND", helper_command)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
}

/// 組出要交給 `fzf` 執行的候選資料 helper 命令列。
fn build_fzf_helper_command(
    root_dir: &std::path::Path,
    show_hidden: bool,
    follow_links: bool,
) -> Result<String> {
    let current_exe = std::env::current_exe().context("resolve current executable")?;
    let args = vec![
        current_exe.into_os_string(),
        "__stream-fzf-jump".into(),
        root_dir.as_os_str().to_owned(),
        format_helper_bool_flag(show_hidden).into(),
        format_helper_bool_flag(follow_links).into(),
    ];
    Ok(shell_join_os_args(&args))
}

/// 回傳本程式固定使用的 `fzf` 綁定字串。
fn fzf_bindings() -> &'static str {
    "esc:abort,enter:accept"
}

/// 把 helper process 需要的布林值轉成穩定的 CLI 旗標字串。
fn format_helper_bool_flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// 解析 helper process CLI 傳入的布林旗標。
fn parse_helper_bool_flag(value: Option<std::ffi::OsString>, field_name: &str) -> Result<bool> {
    match value.as_deref() {
        Some(raw) if raw == std::ffi::OsStr::new("1") => Ok(true),
        Some(raw) if raw == std::ffi::OsStr::new("0") => Ok(false),
        Some(raw) => Err(anyhow::anyhow!(
            "invalid {field_name} flag: {}",
            raw.to_string_lossy()
        )),
        None => Err(anyhow::anyhow!("missing {field_name} flag")),
    }
}

/// 判斷目前是否啟用開發用的 timing debug 輸出。
pub(super) fn debug_timing_enabled() -> bool {
    std::env::var("TFM_DEBUG_TIMING")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

/// 若已啟用 debug timing，就把某段流程耗時寫到 stderr。
pub(super) fn debug_timing_log(label: &str, started_at: Instant) {
    if debug_timing_enabled() {
        eprintln!(
            "[tfm-timing] {label}: {} ms",
            started_at.elapsed().as_millis()
        );
    }
}

/// 若已啟用 debug timing，就輸出一般訊息。
pub(super) fn debug_timing_message(message: &str) {
    if debug_timing_enabled() {
        eprintln!("[tfm-timing] {message}");
    }
}

/// 把 OS 參數安全地組成 shell 可執行的命令列字串。
fn shell_join_os_args(args: &[std::ffi::OsString]) -> String {
    #[cfg(windows)]
    {
        args.iter()
            .map(|arg| quote_windows_cmd_arg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[cfg(not(windows))]
    {
        args.iter()
            .map(|arg| quote_posix_shell_arg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(not(windows))]
fn quote_posix_shell_arg(arg: &std::ffi::OsString) -> String {
    let text = arg.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}

#[cfg(windows)]
fn quote_windows_cmd_arg(arg: &std::ffi::OsString) -> String {
    let text = arg.to_string_lossy();
    if text.is_empty() {
        return "\"\"".to_string();
    }

    let needs_quotes = text
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '^' | '&' | '|' | '<' | '>'));
    if !needs_quotes {
        return text.into_owned();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in text.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                if backslashes > 0 {
                    quoted.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                quoted.push(ch);
            }
        }
    }
    if backslashes > 0 {
        quoted.push_str(&"\\".repeat(backslashes * 2));
    }
    quoted.push('"');
    quoted
}

/// 回傳目前 TUI 真正需要的 keyboard enhancement flag。
///
/// 目前刻意只保留 `DISAMBIGUATE_ESCAPE_CODES`，避免把一般字元鍵編成
/// `CSI u` 這類外部互動程式不一定能處理的序列。
fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
}

/// 進入 TUI 所需的 terminal 模式，並在支援時啟用最小必要的 keyboard enhancement。
fn enter_tui_mode<W: Write>(writer: &mut W) -> Result<()> {
    execute!(writer, EnterAlternateScreen)?;
    if matches!(supports_keyboard_enhancement(), Ok(true)) {
        execute!(
            writer,
            PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
        )?;
    }
    Ok(())
}

/// 離開 TUI 前還原 terminal 狀態，避免把進階鍵盤協定留給外部程式。
fn leave_tui_mode<W: Write>(writer: &mut W) -> Result<()> {
    execute!(writer, SetCursorStyle::DefaultUserShape)?;
    if matches!(supports_keyboard_enhancement(), Ok(true)) {
        execute!(writer, PopKeyboardEnhancementFlags)?;
    }
    execute!(writer, LeaveAlternateScreen)?;
    Ok(())
}

/// 一邊遞迴掃描目錄，一邊把候選項目持續寫進 `fzf` 的 stdin。
fn stream_fzf_candidates<W: Write>(
    root_dir: &std::path::Path,
    show_hidden: bool,
    follow_links: bool,
    cancel_scan: &AtomicBool,
    writer: &mut W,
) -> io::Result<()> {
    let mut walker = WalkBuilder::new(root_dir);
    walker
        .hidden(!show_hidden)
        .git_ignore(false)
        .git_exclude(false)
        .parents(false)
        .ignore(false)
        .follow_links(follow_links);

    let mut written = 0usize;
    for entry in walker.build() {
        if cancel_scan.load(Ordering::Relaxed) {
            return Ok(());
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path == root_dir {
            continue;
        }

        let Some(relative_path) = path.strip_prefix(root_dir).ok() else {
            continue;
        };
        let label = format_jump_candidate_label(
            relative_path,
            entry.file_type().is_some_and(|ty| ty.is_dir()),
        );
        write_fzf_candidate(writer, &label)?;
        written += 1;
        if written.is_multiple_of(64) {
            flush_fzf_candidates(writer)?;
        }
    }

    flush_fzf_candidates(writer)
}

/// 把實際路徑轉成適合顯示在 `fzf` 中的相對路徑文字。
fn format_jump_candidate_label(relative_path: &std::path::Path, is_dir: bool) -> String {
    let mut label = relative_path.to_string_lossy().replace('\\', "/");
    if is_dir && !label.ends_with('/') {
        label.push('/');
    }
    label
}

/// 寫入單一 `fzf` 候選；若 `fzf` 已提前退出，BrokenPipe 視為正常結束。
fn write_fzf_candidate<W: Write>(writer: &mut W, label: &str) -> io::Result<()> {
    match writeln!(writer, "{label}") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error),
    }
}

/// flush 候選串流；若 `fzf` 已退出，BrokenPipe 不應阻塞回到 TUI。
fn flush_fzf_candidates<W: Write>(writer: &mut W) -> io::Result<()> {
    match writer.flush() {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error),
    }
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
            leave_tui_mode(terminal.backend_mut())?;
            terminal.show_cursor()?;

            let status = Command::new(&launch.program).args(&launch.args).status()?;

            enter_tui_mode(terminal.backend_mut())?;
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
    use std::fs;
    use std::sync::atomic::AtomicBool;

    use crossterm::event::KeyEventKind;
    use tempfile::tempdir;

    use super::{
        format_helper_bool_flag, fzf_bindings, parse_helper_bool_flag, should_handle_key_event,
        stream_fzf_candidates,
    };

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

    #[test]
    fn stream_fzf_candidates_writes_nested_entries() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

        let cancel = AtomicBool::new(false);
        let mut output = Vec::new();
        stream_fzf_candidates(dir.path(), false, true, &cancel, &mut output).expect("stream");
        let text = String::from_utf8(output).expect("utf8");

        assert!(text.contains("alpha.txt\n"));
        assert!(text.contains("docs/\n"));
        assert!(text.contains("docs/guide.md\n"));
    }

    #[test]
    fn stream_fzf_candidates_includes_hidden_entries_when_enabled() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".secret.txt"), "secret").expect("secret");
        fs::create_dir(dir.path().join(".cache")).expect("cache");
        fs::write(dir.path().join(".cache").join("item.txt"), "item").expect("item");

        let cancel = AtomicBool::new(false);
        let mut output = Vec::new();
        stream_fzf_candidates(dir.path(), true, true, &cancel, &mut output).expect("stream");
        let text = String::from_utf8(output).expect("utf8");

        assert!(text.contains(".secret.txt\n"));
        assert!(text.contains(".cache/\n"));
        assert!(text.contains(".cache/item.txt\n"));
    }

    #[cfg(unix)]
    #[test]
    fn stream_fzf_candidates_can_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().expect("tempdir");
        let real_dir = dir.path().join("real");
        fs::create_dir(&real_dir).expect("real");
        fs::write(real_dir.join("nested.txt"), "nested").expect("nested");
        symlink(&real_dir, dir.path().join("linked")).expect("symlink");

        let cancel = AtomicBool::new(false);
        let mut without_follow = Vec::new();
        stream_fzf_candidates(dir.path(), true, false, &cancel, &mut without_follow)
            .expect("stream");
        let without_follow = String::from_utf8(without_follow).expect("utf8");
        assert!(without_follow.contains("linked\n") || without_follow.contains("linked/\n"));
        assert!(!without_follow.contains("linked/nested.txt\n"));

        let cancel = AtomicBool::new(false);
        let mut with_follow = Vec::new();
        stream_fzf_candidates(dir.path(), true, true, &cancel, &mut with_follow).expect("stream");
        let with_follow = String::from_utf8(with_follow).expect("utf8");
        assert!(with_follow.contains("linked/nested.txt\n"));
    }

    #[test]
    fn fzf_bindings_include_escape_abort() {
        assert!(fzf_bindings().contains("esc:abort"));
        assert!(fzf_bindings().contains("enter:accept"));
    }

    #[test]
    fn helper_bool_flags_round_trip() {
        assert_eq!(format_helper_bool_flag(true), "1");
        assert_eq!(format_helper_bool_flag(false), "0");
        assert!(parse_helper_bool_flag(Some("1".into()), "flag").expect("true"));
        assert!(!parse_helper_bool_flag(Some("0".into()), "flag").expect("false"));
    }

    #[test]
    fn helper_bool_flags_reject_invalid_values() {
        assert!(parse_helper_bool_flag(Some("yes".into()), "flag").is_err());
        assert!(parse_helper_bool_flag(None, "flag").is_err());
    }
}
