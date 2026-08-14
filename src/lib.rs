use std::{
    collections::BTreeMap,
    fs,
    io::{self, Stdout},
    path::{Path, PathBuf},
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
pub mod config;
pub mod theme;

use crate::config::{AppConfig, load_config};
use crate::theme::{Theme, ThemePreset};

/// 啟動整個 terminal file manager。
///
/// 參數：無。
/// 回傳：`Result<()>`。
/// - 成功時代表 TUI 已正常啟動並在結束後完成清理。
/// - 失敗時代表初始化、事件迴圈或終端還原流程中出現錯誤。
pub fn run() -> Result<()> {
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
        // 每一輪都先完整重畫，再依照設定好的 poll rate 等待輸入。
        // 這種事件迴圈結構很常見於 TUI 程式：畫面與輸入共用同一個主循環。
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

/// 表示目錄列表中的單一檔案或資料夾項目。
///
/// 這個結構是檔案瀏覽清單與預覽系統共用的最小單位資料。
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

impl FileEntry {
    /// 產生適合顯示在列表中的名稱。
    ///
    /// 參數：
    /// - `self: &FileEntry`，目前的檔案項目。
    ///
    /// 回傳：`String`。
    /// - 若是資料夾，名稱尾端會補上 `/`。
    /// - 若是一般檔案，直接回傳原名稱。
    fn display_name(&self) -> String {
        if self.is_dir {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// 表示單一 pane 的完整瀏覽狀態。
///
/// 每個 pane 都獨立維護自己的目錄、游標與列表狀態，
/// 這樣分割視窗後每個區塊才可以各自操作。
#[derive(Debug)]
struct PaneState {
    // 每個 pane 都有自己的 cwd 與選取狀態，
    // 這樣分割視窗後，每個窗格才能獨立瀏覽不同目錄。
    cwd: PathBuf,
    entries: Vec<FileEntry>,
    selected: usize,
    list_state: ListState,
}

impl PaneState {
    /// 建立一個新的 pane 狀態，並立即載入指定目錄內容。
    ///
    /// 參數：
    /// - `cwd: PathBuf`，這個 pane 啟動時要顯示的目錄。
    ///
    /// 回傳：`io::Result<PaneState>`。
    /// - 成功時回傳已載入目錄內容的 pane。
    /// - 失敗時回傳讀取目錄時發生的 I/O 錯誤。
    fn new(cwd: PathBuf) -> io::Result<Self> {
        let mut pane = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
        };
        pane.reload()?;
        Ok(pane)
    }

    /// 重新掃描目前目錄，並同步更新列表與游標位置。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane 狀態。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表目錄內容已重新載入。
    /// - 失敗時代表讀目錄過程發生 I/O 錯誤。
    fn reload(&mut self) -> io::Result<()> {
        self.entries = read_dir_entries(&self.cwd)?;
        // reload 後要重新修正 selected，
        // 否則刪除檔案或切換目錄後，舊索引可能會超出範圍。
        if self.entries.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
        } else {
            self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            self.list_state.select(Some(self.selected));
        }
        Ok(())
    }

    /// 將列表選取游標向上移動一格。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被移動游標的 pane。
    ///
    /// 回傳：`()`
    fn move_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        self.list_state.select(Some(self.selected));
    }

    /// 將列表選取游標向下移動一格。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被移動游標的 pane。
    ///
    /// 回傳：`()`
    fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
        self.list_state.select(Some(self.selected));
    }

    /// 將列表選取游標跳到最上方。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane。
    ///
    /// 回傳：`()`
    fn move_top(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = 0;
        self.list_state.select(Some(self.selected));
    }

    /// 將列表選取游標跳到最下方。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane。
    ///
    /// 回傳：`()`
    fn move_bottom(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = self.entries.len() - 1;
        self.list_state.select(Some(self.selected));
    }

    /// 取得目前游標指向的檔案項目。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Option<&FileEntry>`。
    /// - `Some(...)` 代表有選取項目。
    /// - `None` 代表目前目錄為空。
    fn selected_entry(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected)
    }

    /// 若目前選到的是資料夾，則進入該資料夾。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要切換目錄的 pane。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已完成目錄切換或目前選項不是資料夾。
    /// - 失敗時代表進入目錄或重新載入內容時發生錯誤。
    fn enter_selected(&mut self) -> io::Result<()> {
        if let Some(entry) = self.selected_entry()
            && entry.is_dir
        {
            self.cwd = entry.path.clone();
            self.selected = 0;
            self.reload()?;
        }
        Ok(())
    }

    /// 回到目前目錄的上一層。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要切換到父目錄的 pane。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已回到父目錄或目前已無父目錄。
    /// - 失敗時代表重新載入父目錄內容時發生錯誤。
    fn go_parent(&mut self) -> io::Result<()> {
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.selected = 0;
            self.reload()?;
        }
        Ok(())
    }

    /// 依照目前選取項目產生預覽區要顯示的文字行。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    /// - `max_lines: usize`，預覽區最多顯示的行數。
    ///
    /// 回傳：`Vec<Line<'static>>`，可直接交給 `ratatui` 的 Paragraph 渲染。
    fn preview_lines(&self, max_lines: usize) -> Vec<Line<'static>> {
        // preview 的策略很保守：
        // 資料夾只顯示摘要，文字檔顯示內容，其他檔案則顯示基本資訊。
        // 這樣可以先保證穩定，再慢慢擴充更完整的預覽系統。
        match self.selected_entry() {
            Some(entry) if entry.is_dir => vec![
                Line::from(format!("dir: {}", entry.path.display())),
                Line::from(format!("items: {}", count_items(&entry.path))),
            ],
            Some(entry) => preview_file(&entry.path, max_lines),
            None => vec![Line::from("empty directory")],
        }
    }

    /// 刪除目前選取的檔案或資料夾。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，執行刪除的 pane。
    ///
    /// 回傳：`io::Result<Option<String>>`。
    /// - `Ok(Some(name))` 代表成功刪除，並回傳顯示名稱。
    /// - `Ok(None)` 代表目前沒有可刪除的選取項目。
    /// - `Err(...)` 代表檔案系統操作失敗。
    fn delete_selected(&mut self) -> io::Result<Option<String>> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(None);
        };

        // 這裡故意直接區分檔案與資料夾刪除，
        // 因為兩者呼叫的 filesystem API 不同，錯誤型態也可能不同。
        if entry.is_dir {
            fs::remove_dir_all(&entry.path)?;
        } else {
            fs::remove_file(&entry.path)?;
        }

        let removed_name = entry.display_name();
        self.reload()?;
        Ok(Some(removed_name))
    }
}

/// 計算指定資料夾內的子項目數量。
///
/// 參數：
/// - `path: &Path`，要計算內容數量的資料夾路徑。
///
/// 回傳：`usize`，讀取成功時為項目數量，失敗時回傳 `0`。
fn count_items(path: &Path) -> usize {
    fs::read_dir(path).map(|iter| iter.count()).unwrap_or(0)
}

/// 讀取指定檔案並產生預覽內容。
///
/// 參數：
/// - `path: &Path`，要預覽的檔案路徑。
/// - `max_lines: usize`，最多要顯示的行數。
///
/// 回傳：`Vec<Line<'static>>`。
/// - 成功時回傳可直接渲染的預覽內容。
/// - 若檔案過大、非文字或無法讀取，則回傳說明訊息。
fn preview_file(path: &Path, max_lines: usize) -> Vec<Line<'static>> {
    let Ok(metadata) = fs::metadata(path) else {
        return vec![Line::from("unable to read metadata")];
    };

    if metadata.len() > 128 * 1024 {
        // 大檔案先跳過內容預覽，避免畫面更新時把整個 TUI 卡住。
        return vec![
            Line::from(format!("file: {}", path.display())),
            Line::from(format!("size: {} bytes", metadata.len())),
            Line::from("preview skipped for files larger than 128 KiB"),
        ];
    }

    match fs::read_to_string(path) {
        Ok(contents) => contents
            .lines()
            .take(max_lines.max(1))
            .map(|line| Line::from(line.to_owned()))
            .collect(),
        Err(_) => vec![
            Line::from(format!("file: {}", path.display())),
            Line::from(format!("size: {} bytes", metadata.len())),
            Line::from("binary or non-utf8 file"),
        ],
    }
}

/// 讀取指定目錄，並整理成可顯示的檔案項目清單。
///
/// 參數：
/// - `path: &Path`，要掃描的目錄路徑。
///
/// 回傳：`io::Result<Vec<FileEntry>>`。
/// - 成功時回傳已排序的檔案與資料夾清單。
/// - 失敗時回傳讀取目錄或 metadata 時的 I/O 錯誤。
fn read_dir_entries(path: &Path) -> io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    for item in fs::read_dir(path)? {
        let item = item?;
        let file_type = item.file_type()?;
        let metadata = item.metadata()?;
        entries.push(FileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: item.path(),
            is_dir: file_type.is_dir(),
            size: metadata.len(),
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

/// 表示 pane 分割的方向。
///
/// `Horizontal` 代表上下分割，`Vertical` 代表左右分割。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitDirection {
    Horizontal,
    Vertical,
}

/// 表示整個多視窗布局的樹狀結構。
///
/// 葉節點代表單一 pane，中間節點代表一次分割行為，
/// 因此可以自然表達巢狀 split 的畫面配置。
#[derive(Clone, Debug, PartialEq, Eq)]
enum LayoutNode {
    Leaf {
        pane_id: usize,
    },
    Split {
        direction: SplitDirection,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    /// 將指定 pane 的葉節點替換成新的 split 節點。
    ///
    /// 參數：
    /// - `self: LayoutNode`，目前的布局樹。
    /// - `target: usize`，要被分割的 pane id。
    /// - `direction: SplitDirection`，新的分割方向。
    /// - `new_pane_id: usize`，新建立 pane 的 id。
    ///
    /// 回傳：`LayoutNode`，套用分割後的新布局樹。
    fn split_leaf(self, target: usize, direction: SplitDirection, new_pane_id: usize) -> Self {
        match self {
            LayoutNode::Leaf { pane_id } if pane_id == target => LayoutNode::Split {
                direction,
                first: Box::new(LayoutNode::Leaf { pane_id }),
                second: Box::new(LayoutNode::Leaf {
                    pane_id: new_pane_id,
                }),
            },
            LayoutNode::Leaf { pane_id } => LayoutNode::Leaf { pane_id },
            LayoutNode::Split {
                direction: split_direction,
                first,
                second,
            } => LayoutNode::Split {
                direction: split_direction,
                first: Box::new(first.split_leaf(target, direction, new_pane_id)),
                second: Box::new(second.split_leaf(target, direction, new_pane_id)),
            },
        }
    }

    /// 從布局樹中移除指定 pane。
    ///
    /// 參數：
    /// - `self: LayoutNode`，目前的布局樹。
    /// - `target: usize`，要關閉的 pane id。
    ///
    /// 回傳：`Option<LayoutNode>`。
    /// - `Some(...)` 代表移除後仍有可用布局。
    /// - `None` 代表移除後已沒有任何 pane。
    fn close_pane(self, target: usize) -> Option<Self> {
        match self {
            LayoutNode::Leaf { pane_id } => {
                if pane_id == target {
                    None
                } else {
                    Some(LayoutNode::Leaf { pane_id })
                }
            }
            LayoutNode::Split {
                direction,
                first,
                second,
            } => {
                let first = first.close_pane(target);
                let second = second.close_pane(target);
                match (first, second) {
                    (None, None) => None,
                    (Some(node), None) | (None, Some(node)) => Some(node),
                    (Some(first), Some(second)) => Some(LayoutNode::Split {
                        direction,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                }
            }
        }
    }

    /// 依照布局樹順序收集所有 pane id。
    ///
    /// 參數：
    /// - `self: &LayoutNode`，目前的布局樹。
    /// - `output: &mut Vec<usize>`，要寫入結果的容器。
    ///
    /// 回傳：`()`
    fn pane_ids(&self, output: &mut Vec<usize>) {
        match self {
            LayoutNode::Leaf { pane_id } => output.push(*pane_id),
            LayoutNode::Split { first, second, .. } => {
                first.pane_ids(output);
                second.pane_ids(output);
            }
        }
    }

    /// 計算每個 pane 在畫面上應該佔據的矩形區域。
    ///
    /// 參數：
    /// - `self: &LayoutNode`，目前的布局樹。
    /// - `area: Rect`，目前節點可使用的畫面範圍。
    /// - `map: &mut BTreeMap<usize, Rect>`，收集 pane id 與畫面區塊的對應表。
    ///
    /// 回傳：`()`
    fn render_rects(&self, area: Rect, map: &mut BTreeMap<usize, Rect>) {
        match self {
            LayoutNode::Leaf { pane_id } => {
                map.insert(*pane_id, area);
            }
            LayoutNode::Split {
                direction,
                first,
                second,
            } => {
                let chunks = Layout::default()
                    .direction(match direction {
                        SplitDirection::Horizontal => Direction::Vertical,
                        SplitDirection::Vertical => Direction::Horizontal,
                    })
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(area);
                first.render_rects(chunks[0], map);
                second.render_rects(chunks[1], map);
            }
        }
    }
}

/// 表示目前正在等待使用者完成的暫時互動。
///
/// 只要有 pending action，輸入會先被它攔截，
/// 而不會直接進到一般檔案瀏覽模式。
#[derive(Debug, PartialEq, Eq)]
enum PendingAction {
    // PendingAction 代表目前有一個「需要優先處理的暫時互動」。
    // 只要它存在，輸入就不會再直接進到一般檔案瀏覽模式。
    ConfirmDelete { pane_id: usize, target_name: String },
    ThemePicker { selected: usize },
}

/// 表示整個應用程式的核心狀態。
///
/// 這個結構整合了設定、主題、視窗布局、焦點與互動模式，
/// 是整個 TUI 運作時最主要的狀態容器。
#[derive(Debug)]
struct App {
    // App 是整個 TUI 的核心狀態容器。
    // 只要一個動作會影響整體畫面或操作模式，通常就會落在這裡管理。
    config: AppConfig,
    theme: Theme,
    theme_preset: ThemePreset,
    panes: BTreeMap<usize, PaneState>,
    layout: LayoutNode,
    focused_pane: usize,
    next_pane_id: usize,
    status: String,
    command_mode: bool,
    command_buffer: String,
    awaiting_ctrl_w: bool,
    pending_g: bool,
    pending_action: Option<PendingAction>,
}

impl App {
    /// 建立一個新的應用程式狀態。
    ///
    /// 參數：
    /// - `cwd: PathBuf`，啟動時第一個 pane 要打開的目錄。
    /// - `loaded_config: crate::config::LoadedConfig`，啟動時已載入的設定與來源資訊。
    ///
    /// 回傳：`io::Result<App>`。
    /// - 成功時回傳完整初始化的應用程式狀態。
    /// - 失敗時回傳建立第一個 pane 或載入目錄時的 I/O 錯誤。
    fn new(cwd: PathBuf, loaded_config: crate::config::LoadedConfig) -> io::Result<Self> {
        let pane = PaneState::new(cwd)?;
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);
        let theme_preset = loaded_config.config.theme_preset;
        let startup_status = match loaded_config.source {
            Some(path) => format!("loaded config: {}", path.display()),
            None => String::from("normal mode"),
        };

        Ok(Self {
            config: loaded_config.config,
            theme: theme_preset.into(),
            theme_preset,
            panes,
            layout: LayoutNode::Leaf { pane_id: 1 },
            focused_pane: 1,
            next_pane_id: 2,
            status: startup_status,
            command_mode: false,
            command_buffer: String::new(),
            awaiting_ctrl_w: false,
            pending_g: false,
            pending_action: None,
        })
    }

    /// 處理一般輸入事件的總入口。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `key: KeyEvent`，使用者按下的按鍵事件。
    ///
    /// 回傳：`Result<bool>`。
    /// - `Ok(true)` 代表繼續執行事件迴圈。
    /// - `Ok(false)` 代表應用程式應該結束。
    /// - `Err(...)` 代表處理過程發生錯誤。
    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // 輸入處理的優先順序很重要：
        // 1. 若有 popup / confirm 這種暫時互動，先讓它攔截按鍵
        // 2. 若在 command mode，就交給命令列編輯
        // 3. 最後才是正常瀏覽模式
        if self.pending_action.is_some() {
            return self.handle_pending_action_key(key);
        }

        if self.command_mode {
            return self.handle_command_key(key);
        }

        if self.awaiting_ctrl_w {
            self.awaiting_ctrl_w = false;
            return self.handle_ctrl_w(key);
        }

        let should_continue = match key.code {
            KeyCode::Char('q') => false,
            KeyCode::Char(':') => {
                self.command_mode = true;
                self.command_buffer.clear();
                self.status = String::from("command mode");
                true
            }
            KeyCode::Char('j') => {
                self.current_pane_mut()?.move_down();
                self.pending_g = false;
                true
            }
            KeyCode::Char('k') => {
                self.current_pane_mut()?.move_up();
                self.pending_g = false;
                true
            }
            KeyCode::Char('h') => {
                self.current_pane_mut()?.go_parent()?;
                self.status = String::from("moved to parent directory");
                self.pending_g = false;
                true
            }
            KeyCode::Char('l') | KeyCode::Enter => {
                self.current_pane_mut()?.enter_selected()?;
                self.status = String::from("opened directory");
                self.pending_g = false;
                true
            }
            KeyCode::Char('g') => {
                if self.pending_g {
                    self.current_pane_mut()?.move_top();
                    self.pending_g = false;
                    self.status = String::from("jumped to top");
                } else {
                    self.pending_g = true;
                    self.status = String::from("pending: g");
                }
                true
            }
            KeyCode::Char('G') => {
                self.current_pane_mut()?.move_bottom();
                self.pending_g = false;
                self.status = String::from("jumped to bottom");
                true
            }
            KeyCode::Char('d') => {
                self.start_delete_confirmation();
                self.pending_g = false;
                true
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.awaiting_ctrl_w = true;
                self.pending_g = false;
                self.status = String::from("Ctrl-w");
                true
            }
            KeyCode::Esc => {
                self.pending_g = false;
                self.status = String::from("normal mode");
                true
            }
            _ => {
                self.pending_g = false;
                true
            }
        };

        Ok(should_continue)
    }

    /// 處理暫時互動視窗的按鍵事件。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `key: KeyEvent`，使用者輸入的按鍵事件。
    ///
    /// 回傳：`Result<bool>`。
    /// - `Ok(true)` 代表互動完成後仍繼續執行應用程式。
    /// - `Err(...)` 代表執行刪除或其他互動時發生錯誤。
    fn handle_pending_action_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(action) = self.pending_action.take() else {
            return Ok(true);
        };

        // 先 take 再決定是否放回 pending_action，
        // 可以避免同時借用 self 與 self.pending_action 時出現借用衝突。
        match action {
            PendingAction::ConfirmDelete {
                pane_id,
                target_name,
            } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_delete(pane_id, &target_name)?;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.status = format!("delete cancelled: {target_name}");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmDelete {
                        pane_id,
                        target_name: target_name.clone(),
                    });
                    self.status = format!("confirm delete {target_name}: y/n");
                }
            },
            PendingAction::ThemePicker { mut selected } => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    selected = (selected + 1) % ThemePreset::ALL.len();
                    self.pending_action = Some(PendingAction::ThemePicker { selected });
                    self.status = format!("theme picker: {}", ThemePreset::ALL[selected].name());
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                    self.pending_action = Some(PendingAction::ThemePicker { selected });
                    self.status = format!("theme picker: {}", ThemePreset::ALL[selected].name());
                }
                KeyCode::Enter => {
                    self.apply_theme(ThemePreset::ALL[selected]);
                }
                KeyCode::Esc => {
                    self.status = String::from("theme picker cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ThemePicker { selected });
                    self.status = String::from("theme picker: use j/k and Enter");
                }
            },
        }

        Ok(true)
    }

    /// 處理 command mode 中的按鍵編輯與送出行為。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `key: KeyEvent`，使用者按下的按鍵。
    ///
    /// 回傳：`Result<bool>`。
    /// - `Ok(true)` 代表 command mode 處理完成後繼續執行。
    /// - `Err(...)` 代表命令執行時發生錯誤。
    fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.command_mode = false;
                self.command_buffer.clear();
                self.status = String::from("normal mode");
            }
            KeyCode::Backspace => {
                self.command_buffer.pop();
            }
            KeyCode::Enter => {
                let command = std::mem::take(&mut self.command_buffer);
                self.command_mode = false;
                self.execute_command(command.trim())?;
            }
            KeyCode::Char(c) => {
                self.command_buffer.push(c);
            }
            _ => {}
        }
        Ok(true)
    }

    /// 處理 `Ctrl-w` 前綴後的 pane 操作命令。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `key: KeyEvent`，`Ctrl-w` 之後接續輸入的按鍵。
    ///
    /// 回傳：`Result<bool>`。
    /// - `Ok(true)` 代表指令處理完成並繼續執行。
    /// - `Err(...)` 代表分割或切換過程中發生錯誤。
    fn handle_ctrl_w(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('h') | KeyCode::Char('k') => self.focus_previous_pane(),
            KeyCode::Char('l') | KeyCode::Char('j') => self.focus_next_pane(),
            KeyCode::Char('v') => self.split_current(SplitDirection::Vertical)?,
            KeyCode::Char('s') => self.split_current(SplitDirection::Horizontal)?,
            KeyCode::Char('c') => self.close_current_pane(),
            KeyCode::Char('o') => self.only_current_pane(),
            _ => {
                self.status = String::from("unknown Ctrl-w command");
            }
        }
        Ok(true)
    }

    /// 執行 command mode 送出的命令字串。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `command: &str`，使用者輸入的命令內容。
    ///
    /// 回傳：`Result<()>`。
    /// - 成功時代表命令已完成或已更新狀態訊息。
    /// - 失敗時代表命令觸發的底層操作失敗。
    fn execute_command(&mut self, command: &str) -> Result<()> {
        // command mode 目前維持簡單字串匹配。
        // 對現在的專案規模來說，這比過早引入完整 parser 更容易修改。
        match command {
            "q" => {
                self.status = String::from("use q in normal mode to quit");
            }
            "theme" => self.open_theme_picker(),
            "theme next" => self.cycle_theme(),
            "split" => self.split_current(SplitDirection::Horizontal)?,
            "vsplit" => self.split_current(SplitDirection::Vertical)?,
            "close" => self.close_current_pane(),
            "only" => self.only_current_pane(),
            "" => {
                self.status = String::from("normal mode");
            }
            other => {
                if let Some(name) = other.strip_prefix("theme ") {
                    self.set_theme_by_name(name.trim());
                } else {
                    self.status = format!("unknown command: {other}");
                }
            }
        }
        Ok(())
    }

    /// 取得目前有焦點的 pane 可變參考。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`io::Result<&mut PaneState>`。
    /// - 成功時回傳目前焦點 pane。
    /// - 失敗時代表焦點指向不存在的 pane。
    fn current_pane_mut(&mut self) -> io::Result<&mut PaneState> {
        self.panes
            .get_mut(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused pane"))
    }

    /// 將目前焦點 pane 依指定方向分割成兩個 pane。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `direction: SplitDirection`，要採用的分割方向。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表新 pane 已建立並加入布局。
    /// - 失敗時代表建立 pane 或複製目錄狀態時發生錯誤。
    fn split_current(&mut self, direction: SplitDirection) -> io::Result<()> {
        let cwd = self
            .panes
            .get(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused pane"))?
            .cwd
            .clone();

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        self.panes.insert(new_id, PaneState::new(cwd)?);
        self.layout = self
            .layout
            .clone()
            .split_leaf(self.focused_pane, direction, new_id);
        self.focused_pane = new_id;
        self.status = match direction {
            SplitDirection::Horizontal => String::from("horizontal split"),
            SplitDirection::Vertical => String::from("vertical split"),
        };
        Ok(())
    }

    /// 依照目前布局順序取得所有 pane id。
    ///
    /// 參數：
    /// - `self: &App`，目前的應用程式狀態。
    ///
    /// 回傳：`Vec<usize>`，依畫面遍歷順序排列的 pane id 清單。
    fn ordered_pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.layout.pane_ids(&mut ids);
        ids
    }

    /// 將焦點切換到下一個 pane。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`()`
    fn focus_next_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if let Some(index) = ids.iter().position(|id| *id == self.focused_pane) {
            self.focused_pane = ids[(index + 1) % ids.len()];
            self.status = format!("focused pane {}", self.focused_pane);
        }
    }

    /// 將焦點切換到上一個 pane。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`()`
    fn focus_previous_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if let Some(index) = ids.iter().position(|id| *id == self.focused_pane) {
            self.focused_pane = ids[(index + ids.len() - 1) % ids.len()];
            self.status = format!("focused pane {}", self.focused_pane);
        }
    }

    /// 關閉目前有焦點的 pane。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`()`
    fn close_current_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if ids.len() <= 1 {
            self.status = String::from("cannot close the last pane");
            return;
        }

        let old_focus = self.focused_pane;
        if let Some(index) = ids.iter().position(|id| *id == old_focus) {
            let fallback = if index > 0 {
                ids[index - 1]
            } else {
                ids[index + 1]
            };
            if let Some(layout) = self.layout.clone().close_pane(old_focus) {
                self.layout = layout;
                self.panes.remove(&old_focus);
                self.focused_pane = fallback;
                self.status = format!("closed pane {old_focus}");
            }
        }
    }

    /// 僅保留目前有焦點的 pane，其餘全部關閉。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`()`
    fn only_current_pane(&mut self) {
        let focused = self.focused_pane;
        self.panes.retain(|id, _| *id == focused);
        self.layout = LayoutNode::Leaf { pane_id: focused };
        self.status = String::from("kept only focused pane");
    }

    /// 將主題切換到下一個內建預設值。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`()`
    fn cycle_theme(&mut self) {
        let next = self.theme_preset.next();
        self.apply_theme(next);
    }

    /// 打開主題選擇視窗，並將選項焦點設在目前主題。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`()`
    fn open_theme_picker(&mut self) {
        // 打開主題視窗時，預設焦點會落在目前主題，
        // 這樣使用者一進來就能知道現在是哪一套。
        let selected = ThemePreset::ALL
            .iter()
            .position(|preset| *preset == self.theme_preset)
            .unwrap_or(0);
        self.pending_action = Some(PendingAction::ThemePicker { selected });
        self.status = String::from("theme picker: use j/k and Enter");
    }

    /// 依照主題名稱字串套用指定主題。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `name: &str`，要切換到的主題名稱。
    ///
    /// 回傳：`()`
    fn set_theme_by_name(&mut self, name: &str) {
        match ThemePreset::from_name(name) {
            Some(preset) => self.apply_theme(preset),
            None => {
                let available = ThemePreset::ALL
                    .iter()
                    .map(|preset| preset.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.status = format!("unknown theme: {name}. available: {available}");
            }
        }
    }

    /// 直接套用指定的主題預設值。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `preset: ThemePreset`，要套用的主題預設值。
    ///
    /// 回傳：`()`
    fn apply_theme(&mut self, preset: ThemePreset) {
        self.theme_preset = preset;
        self.theme = preset.into();
        self.status = format!("theme: {}", preset.name());
    }

    /// 開始刪除確認流程，建立一個待確認的刪除互動。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    ///
    /// 回傳：`()`
    fn start_delete_confirmation(&mut self) {
        let Some(entry) = self
            .panes
            .get(&self.focused_pane)
            .and_then(PaneState::selected_entry)
            .cloned()
        else {
            self.status = String::from("nothing selected to delete");
            return;
        };

        self.pending_action = Some(PendingAction::ConfirmDelete {
            pane_id: self.focused_pane,
            target_name: entry.display_name(),
        });
        self.status = format!("confirm delete {}: y/n", entry.display_name());
    }

    /// 真正執行刪除目前待確認項目的檔案系統操作。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `pane_id: usize`，來源 pane 的 id。
    /// - `target_name: &str`，用於狀態訊息顯示的目標名稱。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表刪除流程已完成並更新狀態訊息。
    /// - 失敗時代表刪除過程發生 I/O 錯誤。
    fn confirm_delete(&mut self, pane_id: usize, target_name: &str) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("pane no longer exists");
            return Ok(());
        };

        match pane.delete_selected() {
            Ok(Some(removed_name)) => {
                self.status = format!("deleted {removed_name}");
            }
            Ok(None) => {
                self.status = String::from("nothing selected to delete");
            }
            Err(error) => {
                self.status = format!("failed to delete {target_name}: {error}");
            }
        }

        Ok(())
    }

    /// 根據目前應用程式狀態繪製整個畫面。
    ///
    /// 參數：
    /// - `self: &mut App`，目前的應用程式狀態。
    /// - `frame: &mut ratatui::Frame<'_>`，本次重繪使用的畫面物件。
    ///
    /// 回傳：`()`
    fn render(&mut self, frame: &mut ratatui::Frame<'_>) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ])
            .split(frame.area());

        let mut pane_rects = BTreeMap::new();
        self.layout.render_rects(outer[0], &mut pane_rects);
        // layout tree 先把每個 pane 應該佔的矩形算出來，
        // 之後 render 階段只要根據 pane_id 把內容畫進對應區塊即可。
        for (pane_id, rect) in pane_rects {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                render_pane(
                    frame,
                    rect,
                    pane_id,
                    pane,
                    pane_id == self.focused_pane,
                    self.theme,
                );
            }
        }

        let help = Paragraph::new(Line::from(vec![
            Span::styled("hjkl", self.theme.accent_style()),
            Span::raw(" move  "),
            Span::styled("gg/G", self.theme.accent_style()),
            Span::raw(" jump  "),
            Span::styled("Ctrl-w s/v", self.theme.accent_style()),
            Span::raw(" split  "),
            Span::styled("Ctrl-w h/j/k/l", self.theme.accent_style()),
            Span::raw(" focus  "),
            Span::styled("d", self.theme.accent_style()),
            Span::raw(" delete  "),
            Span::styled(":split :vsplit :close :only", self.theme.accent_style()),
            Span::raw("  "),
            Span::styled(":theme", self.theme.accent_style()),
        ]))
        .block(Block::default().borders(Borders::TOP));
        frame.render_widget(help, outer[1]);

        let status_text = if self.command_mode {
            format!(":{}", self.command_buffer)
        } else {
            self.status.clone()
        };
        frame.render_widget(Paragraph::new(status_text), outer[2]);

        if self.command_mode {
            let area = centered_rect(frame.area(), 70, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!(":{}", self.command_buffer))
                    .block(Block::default().title("Command").borders(Borders::ALL)),
                area,
            );
        }

        match &self.pending_action {
            Some(PendingAction::ConfirmDelete { target_name, .. }) => {
                render_confirm_dialog(frame, frame.area(), target_name, self.theme, &self.config);
            }
            Some(PendingAction::ThemePicker { selected }) => {
                render_theme_picker(frame, frame.area(), self.theme, *selected, &self.config);
            }
            None => {}
        }
    }
}

/// 繪製單一 pane 的檔案列表與預覽區。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，這個 pane 在畫面上可使用的矩形範圍。
/// - `pane_id: usize`，目前 pane 的識別值。
/// - `pane: &mut PaneState`，要被渲染的 pane 狀態。
/// - `focused: bool`，這個 pane 是否具有焦點。
/// - `theme: Theme`，目前使用中的主題色盤。
///
/// 回傳：`()`
fn render_pane(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    pane_id: usize,
    pane: &mut PaneState,
    focused: bool,
    theme: Theme,
) {
    // 每個 pane 目前固定分成兩塊：
    // 上半部是檔案列表，下半部是預覽區。
    // 未來如果要加更完整的 metadata 或 preview pane，這裡會是主要延伸點。
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(6)])
        .split(area);

    let border_style = if focused {
        theme.focused_border_style()
    } else {
        theme.muted_style()
    };

    let title = format!(" pane {}  {}", pane_id, pane.cwd.display());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let items: Vec<ListItem<'static>> = if pane.entries.is_empty() {
        vec![ListItem::new(Line::from("empty directory"))]
    } else {
        pane.entries
            .iter()
            .map(|entry| {
                let detail = if entry.is_dir {
                    String::from("dir")
                } else {
                    format!("{}b", entry.size)
                };
                ListItem::new(Line::from(vec![
                    Span::raw(entry.display_name()),
                    Span::styled(format!("  [{detail}]"), theme.muted_style()),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, chunks[0], &mut pane.list_state);

    let preview = Paragraph::new(pane.preview_lines(4)).block(
        Block::default()
            .title("Preview")
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(border_style),
    );
    frame.render_widget(preview, chunks[1]);
}

/// 在指定區域中計算一個置中的 popup 矩形。
///
/// 參數：
/// - `area: Rect`，整體可用畫面範圍。
/// - `width_percent: u16`，popup 寬度占整體寬度的百分比。
/// - `height: u16`，popup 的固定高度列數。
///
/// 回傳：`Rect`，可直接拿來繪製 popup 的置中區域。
fn centered_rect(area: Rect, width_percent: u16, height: u16) -> Rect {
    // popup 全都共用這個 helper，把視窗放到畫面中央。
    // 寬度用百分比、高度用固定列數，對 terminal 來說比較直覺。
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(height),
            Constraint::Min(1),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

/// 繪製刪除確認視窗。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，整體可用畫面範圍。
/// - `target_name: &str`，要顯示的刪除目標名稱。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `config: &AppConfig`，控制 popup 尺寸的應用程式設定。
///
/// 回傳：`()`
fn render_confirm_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    target_name: &str,
    theme: Theme,
    config: &AppConfig,
) {
    // 刪除確認視窗刻意保持很小，讓焦點集中在「你是否真的要刪除」。
    let dialog_area = centered_rect(
        area,
        config.confirm_dialog_width_percent,
        config.confirm_dialog_height,
    );
    frame.render_widget(Clear, dialog_area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("Delete {target_name}?")),
            Line::from("Press y to confirm, n or Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Confirm Delete ",
                    theme.danger_title_style(),
                )))
                .borders(Borders::ALL),
        ),
        dialog_area,
    );
}

/// 繪製主題選擇視窗。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，整體可用畫面範圍。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `selected: usize`，主題選單目前選取的索引位置。
/// - `config: &AppConfig`，控制 popup 尺寸的應用程式設定。
///
/// 回傳：`()`
fn render_theme_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    selected: usize,
    config: &AppConfig,
) {
    // 主題選單用 ListState 來管理游標位置，
    // 好處是和左側檔案列表的操作模型一致，未來也比較容易抽成共用元件。
    let dialog_area = centered_rect(
        area,
        config.theme_picker_width_percent,
        config.theme_picker_height,
    );
    frame.render_widget(Clear, dialog_area);

    let items: Vec<ListItem<'static>> = ThemePreset::ALL
        .iter()
        .map(|preset| ListItem::new(Line::from(preset.name().to_string())))
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let list = List::new(items)
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Theme Picker ",
                    theme.accent_style().add_modifier(Modifier::BOLD),
                )))
                .borders(Borders::ALL),
        )
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, dialog_area, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    /// 驗證 split 操作會將目標葉節點替換成新的分割節點。
    ///
    /// 參數：無。
    /// 回傳：無；若布局結果不正確則測試失敗。
    fn split_leaf_replaces_target_with_split_node() {
        let layout = LayoutNode::Leaf { pane_id: 1 };
        let updated = layout.split_leaf(1, SplitDirection::Vertical, 2);

        assert_eq!(
            updated,
            LayoutNode::Split {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Leaf { pane_id: 1 }),
                second: Box::new(LayoutNode::Leaf { pane_id: 2 }),
            }
        );
    }

    #[test]
    /// 驗證關閉其中一個 pane 後，父 split 會正確收斂。
    ///
    /// 參數：無。
    /// 回傳：無；若布局未正確收斂則測試失敗。
    fn close_pane_collapses_parent_split() {
        let layout = LayoutNode::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Leaf { pane_id: 1 }),
            second: Box::new(LayoutNode::Leaf { pane_id: 2 }),
        };

        assert_eq!(layout.close_pane(2), Some(LayoutNode::Leaf { pane_id: 1 }));
    }

    #[test]
    /// 驗證 pane 重新載入目錄時，資料夾會排在檔案前面。
    ///
    /// 參數：無。
    /// 回傳：無；若排序規則錯誤則測試失敗。
    fn pane_state_lists_directories_before_files() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("nested")).expect("nested dir");
        fs::write(dir.path().join("alpha.txt"), "hello").expect("file");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let names: Vec<String> = pane.entries.iter().map(FileEntry::display_name).collect();

        assert_eq!(
            names,
            vec![String::from("nested/"), String::from("alpha.txt")]
        );
    }

    #[test]
    /// 驗證 pane 可以正確進入子目錄並返回父目錄。
    ///
    /// 參數：無。
    /// 回傳：無；若目錄切換行為錯誤則測試失敗。
    fn pane_state_enters_and_leaves_directories() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        fs::create_dir(&child).expect("child dir");
        fs::write(child.join("note.txt"), "hello").expect("note");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.enter_selected().expect("enter child");
        assert_eq!(pane.cwd, child);

        pane.go_parent().expect("back parent");
        assert_eq!(pane.cwd, dir.path());
    }

    #[test]
    /// 驗證 `only_current_pane` 會只保留目前焦點窗格。
    ///
    /// 參數：無。
    /// 回傳：無；若多餘 pane 未被移除則測試失敗。
    fn app_only_keeps_focused_pane() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(
            dir.path().to_path_buf(),
            crate::config::LoadedConfig {
                config: AppConfig::default(),
                source: None,
            },
        )
        .expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.only_current_pane();

        assert_eq!(app.ordered_pane_ids().len(), 1);
        assert_eq!(
            app.layout,
            LayoutNode::Leaf {
                pane_id: app.focused_pane
            }
        );
    }

    #[test]
    /// 驗證 `PaneState` 可以正確刪除目前選取的檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若檔案未被刪除或狀態未更新則測試失敗。
    fn pane_state_delete_selected_file_removes_it() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let removed = pane.delete_selected().expect("delete");

        assert_eq!(removed, Some(String::from("alpha.txt")));
        assert!(!file_path.exists());
        assert!(pane.entries.is_empty());
    }

    #[test]
    /// 驗證刪除確認流程在確認後會真正刪除選取項目。
    ///
    /// 參數：無。
    /// 回傳：無；若刪除流程未成功套用則測試失敗。
    fn app_delete_confirmation_removes_selected_entry() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(
            dir.path().to_path_buf(),
            crate::config::LoadedConfig {
                config: AppConfig::default(),
                source: None,
            },
        )
        .expect("app");
        app.start_delete_confirmation();
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm delete");

        assert!(!file_path.exists());
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "deleted delete-me.txt");
    }

    #[test]
    /// 驗證輪替主題時會切換到下一個預設值。
    ///
    /// 參數：無。
    /// 回傳：無；若主題未正確輪替則測試失敗。
    fn app_cycle_theme_switches_to_next_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(
            dir.path().to_path_buf(),
            crate::config::LoadedConfig {
                config: AppConfig::default(),
                source: None,
            },
        )
        .expect("app");

        app.cycle_theme();

        assert_eq!(app.theme_preset, ThemePreset::Forest);
        assert_eq!(app.theme, ThemePreset::Forest.into());
        assert_eq!(app.status, "theme: forest");
    }

    #[test]
    /// 驗證打開主題選擇視窗時，游標會落在目前主題。
    ///
    /// 參數：無。
    /// 回傳：無；若預設選取位置錯誤則測試失敗。
    fn app_open_theme_picker_tracks_current_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(
            dir.path().to_path_buf(),
            crate::config::LoadedConfig {
                config: AppConfig::default(),
                source: None,
            },
        )
        .expect("app");

        app.open_theme_picker();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker { selected: 0 })
        );
    }

    #[test]
    /// 驗證依主題名稱字串指定主題時會正確更新狀態。
    ///
    /// 參數：無。
    /// 回傳：無；若主題未正確切換則測試失敗。
    fn app_set_theme_by_name_updates_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(
            dir.path().to_path_buf(),
            crate::config::LoadedConfig {
                config: AppConfig::default(),
                source: None,
            },
        )
        .expect("app");

        app.set_theme_by_name("ocean");

        assert_eq!(app.theme_preset, ThemePreset::Ocean);
        assert_eq!(app.theme, ThemePreset::Ocean.into());
        assert_eq!(app.status, "theme: ocean");
    }

    #[test]
    /// 驗證在主題選擇視窗按下 Enter 後會套用目前選取的主題。
    ///
    /// 參數：無。
    /// 回傳：無；若主題未依選單選取值套用則測試失敗。
    fn app_theme_picker_confirm_applies_selected_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(
            dir.path().to_path_buf(),
            crate::config::LoadedConfig {
                config: AppConfig::default(),
                source: None,
            },
        )
        .expect("app");
        app.pending_action = Some(PendingAction::ThemePicker { selected: 2 });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("apply theme");

        assert_eq!(app.theme_preset, ThemePreset::Ocean);
        assert_eq!(app.theme, ThemePreset::Ocean.into());
        assert_eq!(app.status, "theme: ocean");
    }
}
