//! Visual Studio as a target (docs/adr/0004): launched on the codebase folder, driven through
//! its DTE (`dte.rs`) and read through UI Automation (`uia.rs`).
//!
//! Unlike VS Code it runs with the presenter's own settings: Visual Studio has no throwaway
//! profile that starts quickly, and its experimental instance (`/RootSuffix`) starts with every
//! setting reset.

use crate::dte::{self, Dispatch};
use crate::page::{CodeState, InitialState};
use crate::uia::{self, Uia};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

/// The URL Visual Studio states carry; scene checks compare its code state, not this.
pub const URL: &str = "visualstudio://editor";
/// UI Automation class of Visual Studio's code editor.
pub const EDITOR_CLASS: &str = "WpfTextView";
/// Visual Studio takes a while to start, longer on its first run after an update.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct VisualStudio {
    pub exe: PathBuf,
    /// The codebase folder Visual Studio opened.
    pub folder: PathBuf,
}

/// `devenv.exe` of the newest Visual Studio, from the installer's `vswhere`.
pub fn find_exe() -> Result<PathBuf, String> {
    let vswhere = PathBuf::from(std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into()))
        .join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    let missing = || "Visual Studio isn't installed (no vswhere.exe from the Visual Studio Installer)".to_owned();
    let out = Command::new(&vswhere).args(["-latest", "-property", "productPath"]).output().map_err(|_| missing())?;
    let exe = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    if exe.is_file() { Ok(exe) } else { Err(missing()) }
}

/// Starts Visual Studio on the codebase folder.
pub fn spawn(vs: &VisualStudio) -> Result<Child, String> {
    Command::new(&vs.exe).arg(&vs.folder).spawn().map_err(|e| format!("Couldn't start Visual Studio: {e}"))
}

/// COM for the calling thread. Tauri runs commands on pool threads, so every entry point calls
/// this first; a thread that already has COM keeps it.
pub fn com() {
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
}

/// The DTE of Visual Studio process `pid`.
pub fn connect(pid: u32) -> Result<Dispatch, String> {
    com();
    dte::find(pid)
}

/// Waits for a freshly started Visual Studio to accept automation.
pub fn wait_until_ready(pid: u32) -> Result<(), String> {
    com();
    let dte = dte::wait_for(pid, STARTUP_TIMEOUT)
        .map_err(|_| "Visual Studio started but didn't accept automation within 2 minutes".to_owned())?;
    // The main window only exists once loading is done.
    let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
    while uia::main_window(pid).is_none() || dte.get("MainWindow").is_err() {
        if std::time::Instant::now() > deadline {
            return Err("Visual Studio's window didn't appear".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// `C:\dev\x.rs` → `file:///c%3A/dev/x.rs`, the form VS Code states use, so the same code paths
/// read both.
pub fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    for (i, c) in text.chars().enumerate() {
        match c {
            ':' if i == 1 => out.push_str("%3A"),
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            c if i == 0 => out.extend(c.to_lowercase()),
            c => out.push(c),
        }
    }
    out
}

/// The open file, if any.
pub fn active_file(dte: &Dispatch) -> Option<PathBuf> {
    let document = dte.object("ActiveDocument", &[]).ok()?;
    document.string("FullName").ok().filter(|f| !f.is_empty()).map(PathBuf::from)
}

/// The first line fully inside the editor, like VS Code's top line.
pub fn top_line(lines: &[uia::Line], editor: &uia::Rect) -> Option<u32> {
    lines.iter().find(|l| l.rect.y >= editor.y - 1.0).or(lines.first()).map(|l| l.number)
}

/// Visual Studio's scene-start state: the open file and the top visible line.
pub fn state(pid: u32) -> Result<InitialState, String> {
    let dte = connect(pid)?;
    let file = active_file(&dte);
    let top = match file {
        Some(_) => {
            let uia = Uia::new()?;
            let window = uia::main_window(pid).ok_or("Visual Studio's window isn't open")?;
            let editor = uia.find_class(window, EDITOR_CLASS)?;
            top_line(&uia.visible_lines(&editor)?, &uia.bounds(&editor)?)
        }
        None => None,
    };
    Ok(InitialState {
        url: URL.into(),
        headings: vec![],
        dialogs: vec![],
        code: Some(CodeState { uri: file.as_deref().map(path_to_uri), top, panel: String::new() }),
    })
}

// ---- Steps ------------------------------------------------------------------------------------

use crate::input::{self, Pace};
use crate::steps::{self, Step};
use windows::Win32::UI::Accessibility::IUIAutomationElement;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_A, VK_CONTROL, VK_F, VK_RETURN, VK_SHIFT, VK_T};

const TYPE_CPS: f64 = 12.0;
/// Longer than this many lines, a scroll jumps most of the way first (as in VS Code).
const LONG_SCROLL: f64 = 150.0;
const LONG_SCROLL_TAIL: f64 = 40.0;

/// What Visual Studio's editor shows: its visible non-empty lines and its rectangle.
struct View {
    lines: Vec<uia::Line>,
    area: uia::Rect,
}

impl View {
    fn line_height(&self) -> f64 {
        let mut heights: Vec<f64> = self.lines.iter().map(|l| l.rect.height).filter(|h| *h > 0.0).collect();
        heights.sort_by(f64::total_cmp);
        heights.get(heights.len() / 2).copied().unwrap_or(20.0)
    }

    fn rows(&self) -> f64 {
        self.area.height / self.line_height()
    }

    /// The fractional line at the editor's top edge. Empty lines aren't reported, so it's worked
    /// out from the first reported line and the line height.
    fn top(&self) -> Option<f64> {
        let first = self.lines.first()?;
        let h = self.line_height();
        Some(f64::from(first.number) - (first.rect.y + first.rect.height - h - self.area.y) / h)
    }

    /// Where line `n` is on screen: its own rectangle, or (for an empty line) one worked out
    /// from a neighbour. `None` unless fully inside the editor.
    fn line_rect(&self, n: u32) -> Option<uia::Rect> {
        let h = self.line_height();
        let near = self.lines.iter().min_by_key(|l| l.number.abs_diff(n))?;
        let y = near.rect.y + near.rect.height - h + (f64::from(n) - f64::from(near.number)) * h;
        let x = self.lines.iter().map(|l| l.rect.x).fold(f64::INFINITY, f64::min);
        (y >= self.area.y - 1.0 && y + h <= self.area.y + self.area.height + 1.0).then_some(uia::Rect { x, y, width: 0.0, height: h })
    }
}

struct Ctx {
    pid: u32,
    vs: VisualStudio,
    dte: Dispatch,
    uia: Uia,
    window: windows::Win32::Foundation::HWND,
}

impl Ctx {
    fn new(pid: u32, vs: &VisualStudio) -> Result<Self, String> {
        let dte = connect(pid)?;
        let uia = Uia::new()?;
        let window = uia::main_window(pid).ok_or("Visual Studio's window isn't open")?;
        Ok(Self { pid, vs: vs.clone(), dte, uia, window })
    }

    fn editor(&self) -> Result<IUIAutomationElement, String> {
        self.uia.find_class(self.window, EDITOR_CLASS).map_err(|_| "No file is open in Visual Studio".to_owned())
    }

    fn view(&self) -> Result<View, String> {
        let editor = self.editor()?;
        let lines = self.uia.visible_lines(&editor)?;
        if lines.is_empty() {
            return Err("can't read Visual Studio's editor".into());
        }
        Ok(View { lines, area: self.uia.bounds(&editor)? })
    }

    fn shows(&self, file: &Path) -> bool {
        active_file(&self.dte).is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(&file.to_string_lossy()))
    }

    fn command(&self, name: &str) -> Result<(), String> {
        self.dte.call("ExecuteCommand", &[windows::Win32::System::Variant::VARIANT::from(name)]).map(|_| ())
    }

    fn open_instantly(&self, file: &Path) -> Result<(), String> {
        let path = windows::Win32::System::Variant::VARIANT::from(file.to_string_lossy().as_ref());
        self.dte.object("ItemOperations", &[])?.call("OpenFile", &[path]).map(|_| ())
    }

    /// Moves the caret to `line` (scrolling only as far as needed).
    fn goto(&self, line: u32) -> Result<(), String> {
        use windows::Win32::System::Variant::VARIANT;
        let selection = self.dte.object("ActiveDocument", &[])?.object("Selection", &[])?;
        selection.call("GotoLine", &[VARIANT::from(line.max(1) as i32), VARIANT::from(false)]).map(|_| ())
    }

    /// Puts `line` at the top of the editor, instantly.
    fn line_to_top(&self, line: u32) -> Result<(), String> {
        self.goto(line)?;
        self.command("Edit.ScrollLineTop")
    }

    /// The selected lines, from DTE. A whole-line selection ends at the start of the next line;
    /// that line isn't counted.
    fn selected_lines(&self) -> Result<(u32, u32), String> {
        let selection = self.dte.object("ActiveDocument", &[])?.object("Selection", &[])?;
        let (top, mut bottom) = (selection.int("TopLine")? as u32, selection.int("BottomLine")? as u32);
        let at_start = selection.object("BottomPoint", &[])?.int("LineCharOffset")? == 1;
        if bottom > top && at_start {
            bottom -= 1;
        }
        Ok((top, bottom))
    }

    fn hover_editor(&self, pace: Pace) -> Result<View, String> {
        let view = self.view()?;
        let a = &view.area;
        let (cx, cy) = input::cursor();
        let inside = (cx as f64) > a.x + 20.0 && (cx as f64) < a.x + a.width - 20.0 && (cy as f64) > a.y + 20.0 && (cy as f64) < a.y + a.height - 20.0;
        if !inside {
            input::move_to(a.x + a.width * 0.55, a.y + a.height * 0.5, pace).map_err(abort)?;
        }
        Ok(view)
    }
}

fn abort(e: input::Abort) -> String {
    e.to_string()
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool, what: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    while !done() {
        if std::time::Instant::now() > deadline {
            return Err(format!("Visual Studio didn't {what}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Gets ready ahead of the trigger word: for scroll and highlight, the cursor moves over the editor.
pub fn approach(pid: u32, vs: &VisualStudio, step: &Step, pace: Pace) -> Result<(), String> {
    if matches!(step, Step::Scroll { .. } | Step::Highlight { .. }) {
        let c = Ctx::new(pid, vs)?;
        if c.view().is_ok() {
            c.hover_editor(pace)?;
        }
    }
    Ok(())
}

pub fn run(pid: u32, vs: &VisualStudio, step: &Step, pace: Pace) -> Result<(), String> {
    let c = Ctx::new(pid, vs)?;
    crate::locate::focus_window(pid, c.window)?;
    match step {
        Step::Open { path, line, instant } => open(&c, path, *line, *instant),
        Step::Scroll { path, at, lines } => {
            let file = crate::vscode::resolve(&c.vs.folder, path)?;
            let first = steps::find_anchor(&std::fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?, at)?;
            scroll_to(&c, &file, first, *lines, pace)
        }
        Step::Highlight { path, at, lines } => {
            let file = crate::vscode::resolve(&c.vs.folder, path)?;
            let first = steps::find_anchor(&std::fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?, at)?;
            highlight(&c, &file, first, *lines, pace)
        }
        // Feature Search (Ctrl+Q) mixes commands with settings and docs, so commands run by name.
        Step::Palette { command } => c.command(command),
        Step::Search { query } => {
            input::chord(&[VK_CONTROL, VK_SHIFT, VK_F]).map_err(abort)?;
            std::thread::sleep(Duration::from_millis(800));
            input::chord(&[VK_CONTROL, VK_A]).map_err(abort)?;
            input::type_text(query, TYPE_CPS).map_err(abort)?;
            input::chord(&[VK_RETURN]).map_err(abort)
        }
    }
}

fn open(c: &Ctx, path: &str, line: Option<u32>, instant: bool) -> Result<(), String> {
    let file = crate::vscode::resolve(&c.vs.folder, path)?;
    if !file.is_file() {
        return Err(format!("{path} isn't in the codebase"));
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if instant {
        c.open_instantly(&file)?;
    } else {
        // Search (Ctrl+T), filtered to files with "f:". Enter only once the selected result is
        // this file (its relative path is one of the result's texts); otherwise open it directly.
        input::chord(&[VK_CONTROL, VK_T]).map_err(abort)?;
        std::thread::sleep(Duration::from_millis(600));
        input::type_text(&format!("f:{name}"), TYPE_CPS).map_err(abort)?;
        let want = path.replace('/', "\\");
        let found = wait_until(
            Duration::from_secs(8),
            || c.uia.selected_item_texts(c.pid).is_some_and(|texts| texts.iter().any(|t| t.eq_ignore_ascii_case(&want))),
            "list the file in Search",
        );
        if found.is_ok() {
            input::chord(&[VK_RETURN]).map_err(abort)?;
        } else {
            input::chord(&[windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE]).map_err(abort)?;
            c.open_instantly(&file)?;
        }
    }
    wait_until(Duration::from_secs(10), || c.shows(&file), "open the file")?;
    if let Some(line) = line {
        c.goto(line)?;
    }
    Ok(())
}

fn scroll_to(c: &Ctx, file: &Path, first: u32, count: u32, pace: Pace) -> Result<(), String> {
    if !c.shows(file) {
        return Err(format!("{} isn't the open file: add an Open step before this one", file.display()));
    }
    let desired = |v: &View| (f64::from(first) + f64::from(count.max(1)) / 2.0 - v.rows() / 2.0).max(1.0);
    let view = c.view()?;
    let mut top = view.top().ok_or("can't read the editor's position")?;
    let target = desired(&view);
    if (target - top).abs() > LONG_SCROLL {
        let short = if target > top { target - LONG_SCROLL_TAIL } else { target + LONG_SCROLL_TAIL };
        c.line_to_top(short.max(1.0) as u32)?;
        std::thread::sleep(Duration::from_millis(300));
        top = c.view()?.top().ok_or("can't read the editor's position")?;
    }
    c.hover_editor(pace)?;
    if (target - top).abs() < 0.5 {
        return Ok(());
    }
    // Calibrate lines per wheel unit with a gentle first nudge, as in VS Code.
    let dir = if target > top { -1.0 } else { 1.0 };
    let probe = 40.0;
    input::wheel((dir * probe) as i32).map_err(abort)?;
    std::thread::sleep(Duration::from_millis(260));
    let after = c.view()?.top().ok_or("can't read the editor's position")?;
    if (after - top).abs() < 0.05 {
        return Err("The editor didn't scroll: is the cursor over it?".into());
    }
    let per_unit = ((after - top).abs() / probe).max(0.002);
    top = after;
    for _ in 0..3 {
        let remaining = desired(&c.view()?) - top;
        if remaining.abs() < 0.6 {
            break;
        }
        steps::ease_wheel(remaining, per_unit)?;
        std::thread::sleep(Duration::from_millis(300));
        top = c.view()?.top().ok_or("can't read the editor's position")?;
    }
    Ok(())
}

fn highlight(c: &Ctx, file: &Path, first: u32, count: u32, pace: Pace) -> Result<(), String> {
    let last = first + count.max(1) - 1;
    let view = c.view()?;
    if view.line_rect(first).is_none() || view.line_rect(last + 1).is_none() {
        scroll_to(c, file, first, count + 1, pace)?;
    }
    let view = c.view()?;
    let a = view.line_rect(first).ok_or(format!("line {first} isn't on screen"))?;
    let b = view.line_rect(last + 1).ok_or(format!("line {last} isn't on screen (the range may be taller than the editor)"))?;
    // Column 0 of the first line, then Shift+click at column 0 of the line after the last: whole lines.
    input::move_to(a.x + 1.0, a.y + a.height / 2.0, pace).map_err(abort)?;
    input::click().map_err(abort)?;
    input::move_to(b.x + 1.0, b.y + b.height / 2.0, pace).map_err(abort)?;
    input::shift_click().map_err(abort)?;
    input::move_to(b.x + 200.0, b.y - b.height / 2.0, pace).map_err(abort)?;
    // A click that landed a line off is corrected through DTE.
    if c.selected_lines()? != (first, last) {
        use windows::Win32::System::Variant::VARIANT;
        let selection = c.dte.object("ActiveDocument", &[])?.object("Selection", &[])?;
        selection.call("GotoLine", &[VARIANT::from(first as i32), VARIANT::from(false)])?;
        selection.call("LineDown", &[VARIANT::from(true), VARIANT::from(count.max(1) as i32)])?;
    }
    Ok(())
}

/// Puts Visual Studio back where a scene starts, instantly: the open file at its top line.
pub fn restore_state(pid: u32, vs: &VisualStudio, want: &CodeState) -> Result<(), String> {
    let c = Ctx::new(pid, vs)?;
    match want.uri.as_deref().and_then(steps::uri_to_path) {
        Some(file) => {
            c.open_instantly(&file)?;
            wait_until(Duration::from_secs(10), || c.shows(&file), "open the file")?;
            c.line_to_top(want.top.unwrap_or(1))?;
            std::thread::sleep(Duration::from_millis(300));
        }
        None => c.command("Window.CloseAllDocuments")?,
    }
    Ok(())
}

/// Visual Studio's open file and selection (or caret line), as a step position.
pub fn capture_position(pid: u32, vs: &VisualStudio) -> Result<steps::Position, String> {
    let c = Ctx::new(pid, vs)?;
    let file = active_file(&c.dte).ok_or("Open a file in Visual Studio first")?;
    let relative = steps::relative_to(&vs.folder, &file).ok_or("That file isn't inside the codebase folder")?;
    let (first, last) = c.selected_lines()?;
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let anchor_text = text.lines().nth(first as usize - 1).unwrap_or_default().trim().to_owned();
    Ok(steps::Position {
        path: relative,
        at: steps::Anchor { line: first, text: anchor_text, symbol: steps::enclosing_symbol(&text, first) },
        lines: last - first + 1,
    })
}

// ---- Teach ------------------------------------------------------------------------------------

/// Commands that edit text, move the caret, use the clipboard, save, or open search and go-to
/// boxes. They aren't steps: what they lead to (a file, a scroll, a selection, a search) is.
pub fn ignored_command(name: &str) -> bool {
    const PREFIXES: [&str; 14] = [
        "Edit.Char", "Edit.Word", "Edit.Line", "Edit.Page", "Edit.Document", "Edit.Scroll", "Edit.Select", "Edit.Delete",
        "Edit.Type", "Edit.Break", "Edit.Insert", "Edit.Tab", "Edit.GoTo", "Edit.Find",
    ];
    const EXACT: [&str; 13] = [
        "Edit.Undo", "Edit.Redo", "Edit.Copy", "Edit.Cut", "Edit.Paste", "Edit.Replace", "Edit.ReplaceinFiles",
        "File.OpenFile", "File.SaveSelectedItems", "File.SaveAll", "Window.ActivateDocumentWindow", "Edit.CollapseTag", "Edit.Escape",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p)) || EXACT.contains(&name)
}

/// Reads Visual Studio for Teach: snapshots of the editor, the commands it runs, and searches.
/// Lives on the recorder thread (its COM objects stay there).
pub struct TeachWatch {
    ctx: Ctx,
    commands: dte::CommandWatch,
    /// Queries of searches run, from the search thread.
    searches: std::sync::mpsc::Receiver<String>,
    stop_searches: std::sync::Arc<std::sync::atomic::AtomicBool>,
    search_thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for TeachWatch {
    fn drop(&mut self) {
        self.stop_searches.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// How often the search thread looks at the Find Results window (a look takes about 0.7 s).
const SEARCH_POLL: Duration = Duration::from_secs(1);

impl TeachWatch {
    pub fn start(pid: u32, vs: &VisualStudio) -> Result<Self, String> {
        let ctx = Ctx::new(pid, vs)?;
        let commands = dte::CommandWatch::start(&ctx.dte)?;
        let (found, searches) = std::sync::mpsc::channel();
        let stop_searches = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = stop_searches.clone();
        // Its own thread and UI Automation client: looking through the results window is slow,
        // and the editor is polled every 200 ms.
        let window = ctx.window.0 as isize;
        let search_thread = std::thread::Builder::new()
            .name("scenra-teach-vs-search".into())
            .spawn(move || {
                com();
                let Ok(uia) = Uia::new() else { return };
                let window = windows::Win32::Foundation::HWND(window as *mut _);
                // A search already in the results when teaching starts isn't one.
                let mut last = uia.text_starting(window, SEARCH_SUMMARY);
                loop {
                    // Checked after the look, so a search made just before Stop still counts.
                    let stop = stopping.load(std::sync::atomic::Ordering::SeqCst);
                    let now = uia.text_starting(window, SEARCH_SUMMARY);
                    if now.is_some() && now != last {
                        if let Some(query) = now.as_ref().and_then(|(text, _)| search_query(text)) {
                            let _ = found.send(query);
                        }
                        last = now;
                    }
                    if stop {
                        break;
                    }
                    std::thread::sleep(SEARCH_POLL);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { ctx, commands, searches, stop_searches, search_thread: Some(search_thread) })
    }

    /// The editor now, in the form the code-step watcher compares.
    pub fn snapshot(&self) -> Result<crate::teach_code::Snapshot, String> {
        let c = &self.ctx;
        let file = active_file(&c.dte);
        let (mut top, mut visible, mut selected) = (None, None, Vec::new());
        if file.is_some()
            && let Ok(view) = c.view()
        {
            let bottom = view.area.y + view.area.height + 1.0;
            let inside: Vec<u32> = view.lines.iter().filter(|l| l.rect.y >= view.area.y - 1.0 && l.rect.y + l.rect.height <= bottom).map(|l| l.number).collect();
            top = top_line(&view.lines, &view.area);
            visible = inside.first().zip(inside.last()).map(|(a, b)| (*a, *b));
            // DTE knows the whole selection, on screen or not.
            let selection = c.dte.object("ActiveDocument", &[])?.object("Selection", &[])?;
            if !bool::try_from(&selection.get("IsEmpty")?).unwrap_or(true) {
                let (first, last) = c.selected_lines()?;
                selected = (first..=last).collect();
            }
        }
        let state = InitialState {
            url: URL.into(),
            headings: vec![],
            dialogs: vec![],
            code: Some(CodeState { uri: file.as_deref().map(path_to_uri), top, panel: String::new() }),
        };
        Ok(crate::teach_code::Snapshot { state, selected, visible, shown: None })
    }

    /// Commands Visual Studio ran since the last call, by name, without the ignored ones.
    pub fn commands(&self) -> Vec<(f64, String)> {
        self.commands
            .ran
            .try_iter()
            .filter_map(|ran| dte::command_name(&self.ctx.dte, &ran).map(|name| (ran.at, name)))
            .filter(|(_, name)| !ignored_command(name))
            .collect()
    }

    /// The query of a search run since the last call. Visual Studio 2026's Find in Files
    /// doesn't update DTE's Find object and raises no named command; the Find Results window's
    /// summary ("Find all "fn stop", Entire Workspace, …") is drawn anew for each search, with a
    /// new runtime id even when the text is the same.
    pub fn new_search(&mut self) -> Option<String> {
        self.searches.try_recv().ok()
    }

    /// At Stop: one last look for a search, then the searches not yet collected.
    pub fn last_searches(&mut self) -> Vec<String> {
        self.stop_searches.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.search_thread.take() {
            let _ = thread.join();
        }
        self.searches.try_iter().collect()
    }
}

/// How the Find Results window's summary line starts.
const SEARCH_SUMMARY: &str = "Find all \"";

/// `Find all "fn stop", Entire Workspace, …` → `fn stop`.
pub fn search_query(summary: &str) -> Option<String> {
    let rest = summary.strip_prefix("Find all \"")?;
    let end = rest.rfind("\", ").unwrap_or(rest.rfind('"')?);
    Some(rest[..end].to_owned()).filter(|q| !q.trim().is_empty())
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_comes_from_the_results_summary() {
        assert_eq!(search_query("Find all \"fn stop\", Entire Workspace, Additional files might be excluded. Manage exclusions").as_deref(), Some("fn stop"));
        assert_eq!(search_query("Find all \"say \"hi\"\", Current document").as_deref(), Some("say \"hi\""));
        assert_eq!(search_query("Find all \"\", Entire Workspace"), None);
        assert_eq!(search_query("Replace all \"x\", Entire Workspace"), None);
    }

    #[test]
    fn editing_and_navigation_commands_arent_steps() {
        for name in ["Edit.LineDown", "Edit.CharLeft", "Edit.Paste", "Edit.GoToFindCombo", "Edit.FindinFiles", "File.SaveAll", "Edit.Undo"] {
            assert!(ignored_command(name), "{name}");
        }
        for name in ["Edit.ToggleWordWrap", "View.SolutionExplorer", "Edit.FormatDocument", "View.NavigateBackward"] {
            assert!(!ignored_command(name), "{name}");
        }
    }

    #[test]
    fn the_view_fills_in_empty_lines() {
        let line = |number, y| uia::Line { number, code: "x".into(), rect: uia::Rect { x: 144.0, y, width: 50.0, height: 22.0 } };
        // Line 10 is half hidden at the top; line 12 is empty, so not reported.
        let view = View { lines: vec![line(10, 89.0), line(11, 111.0), line(13, 155.0)], area: uia::Rect { x: 141.0, y: 100.0, width: 800.0, height: 220.0 } };
        assert!((view.top().unwrap() - 10.5).abs() < 1e-9);
        assert!((view.rows() - 10.0).abs() < 1e-9);
        assert_eq!(view.line_rect(12).map(|r| r.y), Some(133.0));
        assert_eq!(view.line_rect(10), None, "half hidden");
        assert_eq!(view.line_rect(18).map(|r| r.y), Some(265.0));
        assert_eq!(view.line_rect(19).map(|r| r.y), Some(287.0));
        assert_eq!(view.line_rect(20), None, "below the editor");
    }

    #[test]
    fn paths_become_vs_code_style_uris() {
        assert_eq!(path_to_uri(Path::new(r"C:\dev\Scenera\take.rs")), "file:///c%3A/dev/Scenera/take.rs");
        assert_eq!(path_to_uri(Path::new(r"D:\My Code\a#b.cs")), "file:///d%3A/My%20Code/a%23b.cs");
        assert_eq!(crate::steps::uri_to_path(&path_to_uri(Path::new(r"C:\My Code\x.rs"))), Some(PathBuf::from(r"c:\My Code\x.rs")));
    }

    #[test]
    fn the_top_line_is_the_first_one_fully_inside() {
        let editor = uia::Rect { x: 0.0, y: 100.0, width: 500.0, height: 400.0 };
        let line = |number, y| uia::Line { number, code: String::new(), rect: uia::Rect { x: 0.0, y, width: 10.0, height: 20.0 } };
        assert_eq!(top_line(&[line(40, 92.0), line(41, 112.0)], &editor), Some(41));
        assert_eq!(top_line(&[line(1, 100.0), line(2, 120.0)], &editor), Some(1));
    }

    /// Live: drives the Visual Studio running on this repo with real input: open (Go To All),
    /// scroll, highlight, palette, search, Use selection, and scene-start restore.
    #[test]
    #[ignore = "needs a running Visual Studio; moves the real mouse and keyboard"]
    fn drives_visual_studio_on_this_repo() {
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let pid = crate::target::process_ids("devenv.exe").into_iter().next().expect("start Visual Studio on this repo first");
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let repo = PathBuf::from(repo.canonicalize().unwrap().to_string_lossy().trim_start_matches(r"\\?\"));
        let vs = VisualStudio { exe: find_exe().unwrap(), folder: repo.clone() };
        let take = "scenra/src-tauri/src/take.rs";
        let text = std::fs::read_to_string(repo.join(take)).unwrap();
        let stop = text.lines().position(|l| l.trim_start().starts_with("pub(crate) fn stop(")).unwrap() as u32 + 1;
        let at = steps::Anchor { line: stop, text: text.lines().nth(stop as usize - 1).unwrap().trim().into(), symbol: None };
        let run_step = |step: Step| {
            let t0 = std::time::Instant::now();
            approach(pid, &vs, &step, Pace::Natural).unwrap();
            run(pid, &vs, &step, Pace::Natural).unwrap_or_else(|e| panic!("{}: {e}", step.describe()));
            eprintln!("{} ok in {:.1} s", step.describe(), t0.elapsed().as_secs_f64());
            std::thread::sleep(Duration::from_millis(600));
        };
        let code = || state(pid).unwrap().code.unwrap();

        // Start somewhere else, then open take.rs on camera.
        restore_state(pid, &vs, &CodeState { uri: Some(path_to_uri(&repo.join("README.md"))), top: Some(1), panel: String::new() }).unwrap();
        run_step(Step::Open { path: take.into(), line: None, instant: false });
        assert!(code().uri.unwrap().ends_with("/take.rs"));

        run_step(Step::Scroll { path: take.into(), at: at.clone(), lines: 6 });
        let c = Ctx::new(pid, &vs).unwrap();
        let view = c.view().unwrap();
        let centre = view.top().unwrap() + view.rows() / 2.0;
        assert!((centre - (f64::from(stop) + 3.0)).abs() < 2.5, "centre {centre:.1}, wanted ≈{}", stop + 3);

        run_step(Step::Highlight { path: take.into(), at: at.clone(), lines: 3 });
        assert_eq!(c.selected_lines().unwrap(), (stop, stop + 2));
        let position = capture_position(pid, &vs).unwrap();
        assert_eq!((position.path.as_str(), position.at.line, position.lines), (take, stop, 3));

        run_step(Step::Palette { command: "Edit.ToggleWordWrap".into() });
        run_step(Step::Palette { command: "Edit.ToggleWordWrap".into() });
        run_step(Step::Search { query: "fn stop".into() });
        input::chord(&[windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE]).unwrap();

        // Scene starts: a file at a top line, then no file at all.
        let want = CodeState { uri: Some(path_to_uri(&repo.join(take))), top: Some(50), panel: String::new() };
        restore_state(pid, &vs, &want).unwrap();
        let expected = InitialState { url: URL.into(), headings: vec![], dialogs: vec![], code: Some(want) };
        crate::page::check_initial(&expected, &state(pid).unwrap()).unwrap();
        restore_state(pid, &vs, &CodeState { uri: None, top: None, panel: String::new() }).unwrap();
        assert_eq!(code().uri, None);
    }

    /// Live: teach a code walkthrough in the Visual Studio running on this repo (Scenra's input
    /// stands in for the presenter), draft it with a stand-in AI, restore the scene start, replay.
    #[test]
    #[ignore = "needs a running Visual Studio; moves the real mouse and keyboard"]
    fn teaches_and_replays_in_visual_studio() {
        use crate::teach::StepFlag;
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        com();
        let pid = crate::target::process_ids("devenv.exe").into_iter().next().expect("start Visual Studio on this repo first");
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let repo = PathBuf::from(repo.canonicalize().unwrap().to_string_lossy().trim_start_matches(r"\\?\"));
        let vs = VisualStudio { exe: find_exe().unwrap(), folder: repo.clone() };
        let take = "scenra/src-tauri/src/take.rs";
        let text = std::fs::read_to_string(repo.join(take)).unwrap();
        let stop = text.lines().position(|l| l.trim_start().starts_with("pub(crate) fn stop(")).unwrap() as u32 + 1;
        let at = steps::Anchor { line: stop, text: text.lines().nth(stop as usize - 1).unwrap().trim().into(), symbol: None };
        let settle = || std::thread::sleep(Duration::from_millis(1800));
        let act = |step: Step| {
            approach(pid, &vs, &step, Pace::Natural).unwrap();
            run(pid, &vs, &step, Pace::Natural).unwrap_or_else(|e| panic!("{}: {e}", step.describe()));
            settle();
        };
        restore_state(pid, &vs, &CodeState { uri: Some(path_to_uri(&repo.join("README.md"))), top: Some(1), panel: String::new() }).unwrap();
        settle();

        let (sent, seen) = std::sync::mpsc::channel();
        let recorder = crate::teach::VsRecorder::start(pid, vs.clone(), move |s| {
            let _ = sent.send(s.label.clone());
        })
        .unwrap();
        settle();
        act(Step::Open { path: take.into(), line: None, instant: false });
        act(Step::Scroll { path: take.into(), at: at.clone(), lines: 6 });
        act(Step::Highlight { path: take.into(), at: at.clone(), lines: 3 });
        act(Step::Palette { command: "Edit.ToggleWordWrap".into() });
        std::thread::sleep(Duration::from_secs(2)); // a command waits to see whether the editor shows it
        act(Step::Search { query: "fn stop".into() });
        input::chord(&[windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE]).unwrap();
        let session = recorder.stop().unwrap();
        let live: Vec<String> = seen.try_iter().collect();
        for s in &session.steps {
            eprintln!("{:>2}. {:<44} {:?} {:?}", s.index + 1, s.label, s.flags, s.before.code.as_ref().map(|c| (&c.uri, c.top)));
        }
        eprintln!("warnings: {:?}", session.warnings);
        // Undo the taught word-wrap toggle.
        Ctx::new(pid, &vs).unwrap().command("Edit.ToggleWordWrap").unwrap();

        let kinds: Vec<&str> = session
            .steps
            .iter()
            .map(|s| match (&s.step, s.flags.first()) {
                (Some(Step::Open { .. }), _) => "open",
                (Some(Step::Scroll { .. }), _) => "scroll",
                (Some(Step::Highlight { .. }), _) => "highlight",
                (Some(Step::Search { .. }), _) => "search",
                (Some(Step::Palette { .. }), _) => "palette",
                (None, Some(StepFlag::Shortcut)) => "shortcut",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["open", "scroll", "highlight", "palette", "search"], "{:?}", session.steps);
        assert!(live.len() >= 4, "steps should arrive live: {live:?}");
        let Some(Step::Highlight { at: taught, lines, .. }) = &session.steps[2].step else { unreachable!() };
        assert_eq!((taught.line, *lines), (stop, 3));
        assert_eq!(session.steps[3].step, Some(Step::Palette { command: "Edit.ToggleWordWrap".into() }));
        assert_eq!(session.steps[4].step, Some(Step::Search { query: "fn stop".into() }));

        let provider = crate::ai::Scripted(vec![(
            "Code tour",
            vec![
                ("Let's open the take module and scroll down to stop.", vec![("open", "s0"), ("scroll", "s1")]),
                ("Here are the lines that matter, so let's wrap and search.", vec![("lines", "s2"), ("wrap", "s3"), ("search", "s4")]),
            ],
        )]);
        let (script, states) = crate::script::draft_from_session(&provider, &session, "a code tour").unwrap();
        let actions: Vec<Step> = script.scenes[0].lines.iter().flat_map(|l| l.actions.iter().map(|a| a.step.clone().unwrap())).collect();
        assert_eq!(actions.len(), 5);

        // Replay: back to the scene start, check it, run every step.
        let start = states[0].code.clone().unwrap();
        restore_state(pid, &vs, &start).unwrap();
        crate::page::check_initial(&states[0], &state(pid).unwrap()).unwrap_or_else(|e| panic!("scene start not restored: {e}"));
        let c = Ctx::new(pid, &vs).unwrap();
        for step in &actions {
            act(step.clone());
            if matches!(step, Step::Highlight { .. }) {
                assert_eq!(c.selected_lines().unwrap(), (stop, stop + 2), "replayed highlight");
            }
        }
        input::chord(&[windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE]).unwrap();
        c.command("Edit.ToggleWordWrap").unwrap();
    }

    /// Live: needs a Visual Studio running on this repo.
    #[test]
    #[ignore = "needs a running Visual Studio"]
    fn reads_visual_studios_state() {
        let pid = crate::target::process_ids("devenv.exe").into_iter().next().expect("start Visual Studio first");
        let state = state(pid).unwrap();
        eprintln!("{state:?}");
        let code = state.code.unwrap();
        assert!(code.uri.unwrap().starts_with("file:///c%3A/"));
        assert!(code.top.is_some());
        assert!(find_exe().unwrap().ends_with("devenv.exe"));
    }
}
