//! Code steps in VS Code (docs/adr/0003-code-navigation.md): open a file, scroll to lines,
//! highlight lines, run a palette command, search. Each runs with the real cursor and keyboard so
//! it looks like a person doing it, or instantly where the step says so.
//!
//! Code positions are text anchors: the first line's text, the nearest symbol and a line-number
//! hint. If the code moved since the script was written, the step follows the anchor; if the
//! anchor is gone, it refuses rather than scroll to the wrong place.

use crate::cdp::Session;
use crate::input::{self, Pace};
use crate::locate::{self, ScreenMap};
use crate::page;
use crate::target::{self, LaunchedTarget};
use crate::vscode::{self, VsCode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_A, VK_B, VK_CONTROL, VK_D, VK_E, VK_F, VK_G, VK_K, VK_P, VK_RETURN, VK_SHIFT, VK_W, VK_X};

/// Lines per second while scrolling at cruising speed.
const SCROLL_SPEED: f64 = 15.0;
/// Further than this, jump most of the way instantly, then scroll the rest.
const LONG_SCROLL: f64 = 150.0;
/// How many lines of the long jump are left to scroll visibly.
const LONG_SCROLL_TAIL: f64 = 40.0;
const TYPE_CPS: f64 = 12.0;

/// Where a step's code is: `line` is a 1-based hint; `text` is that line, trimmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub line: u32,
    #[serde(default)]
    pub text: String,
    /// The enclosing function or type name, to choose between identical lines.
    #[serde(default)]
    pub symbol: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Step {
    /// Open `path` (relative to the codebase): Ctrl+P and the name, or instantly.
    Open {
        path: String,
        #[serde(default)]
        line: Option<u32>,
        #[serde(default)]
        instant: bool,
    },
    /// Scroll until `lines` lines from the anchor are centred.
    Scroll { path: String, at: Anchor, lines: u32 },
    /// Select `lines` lines from the anchor (scrolling to them first if needed).
    Highlight { path: String, at: Anchor, lines: u32 },
    /// Run a command-palette command by its name.
    Palette { command: String },
    /// Search the codebase.
    Search { query: String },
}

impl Step {
    pub fn describe(&self) -> String {
        // The pill has room for a few words: the file's name, not its whole path.
        let name = |path: &str| path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned();
        match self {
            Step::Open { path, line: Some(l), .. } => format!("Open {}:{l}", name(path)),
            Step::Open { path, .. } => format!("Open {}", name(path)),
            Step::Scroll { path, at, lines } => format!("Scroll {} to {}–{}", name(path), at.line, at.line + lines.saturating_sub(1)),
            Step::Highlight { path, at, lines } => format!("Highlight {} {}–{}", name(path), at.line, at.line + lines.saturating_sub(1)),
            Step::Palette { command } => format!("Run “{command}”"),
            Step::Search { query } => format!("Search “{query}”"),
        }
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---- Anchors ----------------------------------------------------------------------------------

/// The 1-based line the anchor points at in `text` now.
pub fn find_anchor(text: &str, at: &Anchor) -> Result<u32, String> {
    let lines: Vec<&str> = text.lines().collect();
    if at.text.trim().is_empty() {
        return Ok(at.line.clamp(1, lines.len().max(1) as u32));
    }
    let wanted = at.text.trim();
    let candidates: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.trim() == wanted).map(|(i, _)| i).collect();
    if candidates.is_empty() {
        return Err(format!("“{wanted}” is no longer in the file; update the step"));
    }
    // Inside the named symbol: the declaration enclosing the candidate is exactly that one, found
    // the same way the anchor's symbol was captured.
    let in_symbol = |i: usize| at.symbol.is_some() && enclosing_symbol(text, i as u32 + 1) == at.symbol;
    let hint = at.line.saturating_sub(1) as i64;
    let best = candidates
        .iter()
        .copied()
        .min_by_key(|&i| (!in_symbol(i), (i as i64 - hint).abs()))
        .unwrap_or(candidates[0]);
    Ok(best as u32 + 1)
}

// ---- What the editor shows --------------------------------------------------------------------

#[derive(Deserialize, Debug)]
struct NumberBox {
    line: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Deserialize, Debug)]
struct Area {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize, Debug)]
struct View {
    uri: Option<String>,
    area: Area,
    lines: Vec<NumberBox>,
}

const VIEW_JS: &str = r#"(() => {
  const ed = document.querySelector('.editor-group-container.active .editor-instance .monaco-editor')
    || document.querySelector('.editor-instance .monaco-editor');
  if (!ed) return null;
  const guard = ed.querySelector('.overflow-guard') || ed;
  const g = guard.getBoundingClientRect();
  const lines = [...ed.querySelectorAll('.margin-view-overlays .line-numbers')]
    .map((n) => { const r = n.getBoundingClientRect(); return { line: +n.textContent, x: r.left, y: r.top, w: r.width, h: r.height }; })
    .filter((n) => n.line > 0 && n.h > 0);
  return { uri: ed.getAttribute('data-uri'), area: { x: g.left, y: g.top, width: g.width, height: g.height }, lines };
})()"#;

impl View {
    /// The fractional line at the top edge of the editor (e.g. 104.5 = half of line 104 hidden).
    fn top(&self) -> Option<f64> {
        let first = self.lines.iter().min_by(|a, b| a.y.total_cmp(&b.y))?;
        Some(f64::from(first.line) + (self.area.y - first.y) / first.h)
    }

    fn line_height(&self) -> f64 {
        self.lines.first().map_or(19.0, |l| l.h)
    }

    /// Lines that fit in the editor.
    fn rows(&self) -> f64 {
        self.area.height / self.line_height()
    }

    fn fully_visible(&self, line: u32) -> Option<&NumberBox> {
        self.lines.iter().find(|n| n.line == line && n.y >= self.area.y - 1.0 && n.y + n.h <= self.area.y + self.area.height + 1.0)
    }

    fn shows(&self, file: &Path) -> bool {
        self.uri.as_deref().and_then(uri_to_path).is_some_and(|p| same_path(&p, file))
    }
}

fn read_view(session: &mut Session) -> Result<View, String> {
    let v = session.evaluate(VIEW_JS)?;
    if v.is_null() {
        return Err("No file is open in VS Code".into());
    }
    serde_json::from_value(v).map_err(err)
}

/// `file:///c%3A/dev/x.rs` → `c:\dev\x.rs`.
pub(crate) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file:///")?;
    let mut out = Vec::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            out.push(u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(out).ok()?.replace('/', "\\")))
}

/// The same file, ignoring case and which way the slashes lean.
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\").trim_end_matches('\\').to_owned();
    norm(a) == norm(b)
}

// ---- Running steps ----------------------------------------------------------------------------

struct Ctx {
    pid: u32,
    session: Session,
    vs: VsCode,
}

fn ctx(targets: &LaunchedTarget) -> Result<Ctx, String> {
    let (pid, port, vs) = target::vscode(targets)?;
    Ok(Ctx { pid, session: page::page_session(port)?, vs })
}

fn abort(e: input::Abort) -> String {
    e.to_string()
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> Result<bool, String>, what: &str) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if done()? {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("VS Code didn't {what} within {} s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(60));
    }
}

fn quick_input_open(session: &mut Session) -> Result<bool, String> {
    let v = session.evaluate("(() => { const w = document.querySelector('.quick-input-widget'); return !!w && w.style.display !== 'none' && w.offsetParent !== null; })()")?;
    Ok(v.as_bool().unwrap_or(false))
}

/// Moves the cursor over the middle of the editor (for the wheel), unless it's already there.
fn hover_editor(c: &mut Ctx, pace: Pace) -> Result<(ScreenMap, View), String> {
    let map = locate::screen_map(&mut c.session, c.pid)?;
    let view = read_view(&mut c.session)?;
    let (x0, y0) = map.point(view.area.x, view.area.y);
    let (x1, y1) = map.point(view.area.x + view.area.width, view.area.y + view.area.height);
    let (cx, cy) = input::cursor();
    let inside = (cx as f64) > x0 + 20.0 && (cx as f64) < x1 - 20.0 && (cy as f64) > y0 + 20.0 && (cy as f64) < y1 - 20.0;
    if !inside {
        input::move_to(x0 + (x1 - x0) * 0.55, y0 + (y1 - y0) * 0.5, pace).map_err(abort)?;
    }
    Ok((map, view))
}

/// Gets ready ahead of the trigger word: for scroll and highlight, the cursor moves over the
/// editor so only the scroll or click is left for the word itself.
pub fn approach(targets: &LaunchedTarget, step: &Step, pace: Pace) -> Result<(), String> {
    match step {
        Step::Scroll { .. } | Step::Highlight { .. } => {
            let mut c = ctx(targets)?;
            if read_view(&mut c.session).is_ok() {
                hover_editor(&mut c, pace)?;
            }
            Ok(())
        }
        // Walking the codebase for a same-named file can take a moment: do it before the word.
        Step::Open { path, instant: false, .. } => {
            let (_, _, vs) = target::vscode(targets)?;
            let file = vscode::resolve(&vs.folder, path)?;
            let unique = unique_name(&vs.folder, &file_name(&file));
            *UNIQUE.lock().map_err(err)? = Some((file, unique));
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The last Open step's file and whether its name is unique, worked out in `approach`.
static UNIQUE: std::sync::Mutex<Option<(PathBuf, bool)>> = std::sync::Mutex::new(None);

fn file_name(file: &Path) -> String {
    file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn run(targets: &LaunchedTarget, step: &Step, pace: Pace) -> Result<(), String> {
    let mut c = ctx(targets)?;
    // Keys must reach VS Code, not whichever window happens to be in front.
    locate::focus_target(c.pid)?;
    match step {
        Step::Open { path, line, instant } => open(&mut c, path, *line, *instant),
        Step::Scroll { path, at, lines } => {
            let file = vscode::resolve(&c.vs.folder, path)?;
            let target = find_anchor(&std::fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?, at)?;
            scroll_to(&mut c, &file, target, *lines, pace)
        }
        Step::Highlight { path, at, lines } => {
            let file = vscode::resolve(&c.vs.folder, path)?;
            let first = find_anchor(&std::fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?, at)?;
            highlight(&mut c, &file, first, *lines, pace)
        }
        Step::Palette { command } => palette(&mut c, command),
        Step::Search { query } => search(&mut c, query),
    }
}

/// Puts VS Code back where a scene starts, instantly: the file at its top line, and the side panel.
pub fn restore_state(targets: &LaunchedTarget, want: &page::CodeState) -> Result<(), String> {
    let mut c = ctx(targets)?;
    locate::focus_target(c.pid)?;
    if let Some(file) = want.uri.as_deref().and_then(uri_to_path) {
        let top = want.top.unwrap_or(1);
        // `code -g` centres the line; aim half a screen below the wanted top line.
        let rows = read_view(&mut c.session).map(|v| v.rows()).unwrap_or(30.0);
        vscode::goto(&c.vs, &file, top + (rows / 2.0) as u32)?;
        let session = &mut c.session;
        wait_until(Duration::from_secs(8), || Ok(read_view(session).is_ok_and(|v| v.shows(&file))), "open the file")?;
        std::thread::sleep(Duration::from_millis(400));
        // Correct for the real height once the file is showing. VS Code only scrolls to a line
        // that's off screen, so first move well away from it, then come back.
        if let Some(have) = read_view(&mut c.session)?.top()
            && (have.round() as i64 - i64::from(top)).abs() > 1
        {
            let rows = read_view(&mut c.session)?.rows();
            let away = if f64::from(top) > rows * 2.0 { 1 } else { top + (rows * 3.0) as u32 };
            vscode::goto(&c.vs, &file, away)?;
            std::thread::sleep(Duration::from_millis(400));
            vscode::goto(&c.vs, &file, (f64::from(top) + rows / 2.0) as u32)?;
            std::thread::sleep(Duration::from_millis(400));
        }
    } else if want.uri.is_none() && read_view(&mut c.session).is_ok() {
        // The scene starts with no file open: Close All Editors (Ctrl+K Ctrl+W).
        input::chord(&[VK_CONTROL, VK_K]).map_err(abort)?;
        input::chord(&[VK_CONTROL, VK_W]).map_err(abort)?;
        let session = &mut c.session;
        wait_until(Duration::from_secs(5), || Ok(read_view(session).is_err()), "close the open files")?;
    }
    let panel = |s: &mut Session| -> Result<String, String> {
        Ok(s.evaluate("(() => { const p = document.querySelector('.part.sidebar'); return p && p.offsetParent !== null ? ((p.querySelector('.composite.title h2') || {}).textContent || '').trim() : ''; })()")?
            .as_str().unwrap_or_default().to_owned())
    };
    let have = panel(&mut c.session)?;
    if have != want.panel {
        let keys: &[_] = match want.panel.to_lowercase().as_str() {
            "" => &[VK_CONTROL, VK_B],
            "explorer" => &[VK_CONTROL, VK_SHIFT, VK_E],
            "search" => &[VK_CONTROL, VK_SHIFT, VK_F],
            "source control" => &[VK_CONTROL, VK_SHIFT, VK_G],
            "run and debug" => &[VK_CONTROL, VK_SHIFT, VK_D],
            "extensions" => &[VK_CONTROL, VK_SHIFT, VK_X],
            other => return Err(format!("Open the “{other}” side panel by hand: Scenra can't restore it")),
        };
        input::chord(keys).map_err(abort)?;
        std::thread::sleep(Duration::from_millis(400));
    }
    Ok(())
}

fn open(c: &mut Ctx, path: &str, line: Option<u32>, instant: bool) -> Result<(), String> {
    let file = vscode::resolve(&c.vs.folder, path)?;
    if !file.is_file() {
        return Err(format!("{path} isn't in the codebase"));
    }
    if instant {
        vscode::goto(&c.vs, &file, line.unwrap_or(1))?;
    } else {
        input::chord(&[VK_CONTROL, VK_P]).map_err(abort)?;
        wait_until(Duration::from_secs(3), || quick_input_open(&mut c.session), "open Quick Open")?;
        let name = file_name(&file);
        let cached = UNIQUE.lock().map_err(err)?.take().filter(|(f, _)| *f == file).map(|(_, u)| u);
        let unique = cached.unwrap_or_else(|| unique_name(&c.vs.folder, &name));
        // The file name alone reads best; the relative path when the name isn't unique.
        let typed = if unique { name.clone() } else { path.replace('\\', "/") };
        input::type_text(&typed, TYPE_CPS).map_err(abort)?;
        // Quick Open searches as you type; on a big or freshly opened codebase the results lag.
        // Only press Enter once the top result is the file: its name, and its folder too when
        // another file has the same name.
        let folder = relative_to(&c.vs.folder, &file)
            .and_then(|r| r.rsplit_once('/').map(|(dir, _)| dir.to_lowercase()))
            .unwrap_or_default();
        let top_js = "(() => { const r = document.querySelector('.quick-input-widget .monaco-list-row'); if (!r) return null; \
            const t = (sel) => ((r.querySelector(sel) || {}).textContent || '').trim(); \
            return { name: t('.label-name'), dir: t('.label-description') }; })()";
        let session = &mut c.session;
        wait_until(
            Duration::from_secs(12),
            || {
                let top = session.evaluate(top_js)?;
                let (got_name, got_dir) = (top["name"].as_str().unwrap_or_default(), top["dir"].as_str().unwrap_or_default());
                Ok(got_name.eq_ignore_ascii_case(&name) && (unique || got_dir.replace('\\', "/").to_lowercase() == folder))
            },
            "find the file in Quick Open",
        )?;
        std::thread::sleep(Duration::from_millis(200));
        input::chord(&[VK_RETURN]).map_err(abort)?;
    }
    let session = &mut c.session;
    wait_until(Duration::from_secs(15), || Ok(read_view(session).is_ok_and(|v| v.shows(&file))), "open the file")?;
    if let (false, Some(line)) = (instant, line) {
        input::chord(&[VK_CONTROL, VK_G]).map_err(abort)?;
        wait_until(Duration::from_secs(3), || quick_input_open(&mut c.session), "open Go to Line")?;
        input::type_text(&line.to_string(), TYPE_CPS).map_err(abort)?;
        std::thread::sleep(Duration::from_millis(250));
        input::chord(&[VK_RETURN]).map_err(abort)?;
    }
    Ok(())
}

/// Whether `name` occurs once in the codebase (skipping dependency and build folders).
fn unique_name(folder: &Path, name: &str) -> bool {
    const SKIP: [&str; 6] = ["node_modules", "target", ".git", "dist", "build", "recordings"];
    let (mut found, mut seen) = (0, 0);
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            seen += 1;
            if seen > 50_000 {
                return false;
            }
            let path = entry.path();
            let file_name = entry.file_name();
            // The entry's own type: symlinks and junctions aren't followed, so a loop can't trap the walk.
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if !SKIP.iter().any(|s| file_name == *s) {
                    stack.push(path);
                }
            } else if file_name.to_string_lossy().eq_ignore_ascii_case(name) {
                found += 1;
                if found > 1 {
                    return false;
                }
            }
        }
    }
    found == 1
}

/// Smoothly scrolls so lines `first..first+count` are centred, with the real wheel.
fn scroll_to(c: &mut Ctx, file: &Path, first: u32, count: u32, pace: Pace) -> Result<(), String> {
    let view = read_view(&mut c.session)?;
    if !view.shows(file) {
        return Err(format!("{} isn't the open file: add an Open step before this one", file.display()));
    }
    let desired = |v: &View| (f64::from(first) + f64::from(count.max(1)) / 2.0 - v.rows() / 2.0).max(1.0);
    let mut top = view.top().ok_or("can't read the editor's position")?;
    let target = desired(&view);

    // A long way: jump most of it instantly (centred on a line short of the target), scroll the rest.
    if (target - top).abs() > LONG_SCROLL {
        let short = if target > top { target - LONG_SCROLL_TAIL } else { target + LONG_SCROLL_TAIL };
        vscode::goto(&c.vs, file, (short + view.rows() / 2.0).max(1.0) as u32)?;
        std::thread::sleep(Duration::from_millis(400));
        top = read_view(&mut c.session)?.top().ok_or("can't read the editor's position")?;
    }
    hover_editor(c, pace)?;

    // Calibrate: lines moved per wheel unit, from a gentle first nudge in the right direction.
    let dir = if target > top { -1.0 } else { 1.0 }; // wheel down (negative) moves the view down
    if (target - top).abs() < 0.5 {
        return Ok(());
    }
    let probe = 40.0;
    input::wheel((dir * probe) as i32).map_err(abort)?;
    std::thread::sleep(Duration::from_millis(260));
    let after = read_view(&mut c.session)?.top().ok_or("can't read the editor's position")?;
    let per_unit = ((after - top).abs() / probe).max(0.002);
    if (after - top).abs() < 0.05 {
        return Err("The editor didn't scroll: is the cursor over it?".into());
    }
    top = after;

    for _ in 0..3 {
        let remaining = desired(&read_view(&mut c.session)?) - top;
        if remaining.abs() < 0.6 {
            break;
        }
        ease_wheel(remaining, per_unit)?;
        std::thread::sleep(Duration::from_millis(300)); // VS Code's own smooth-scroll animation
        top = read_view(&mut c.session)?.top().ok_or("can't read the editor's position")?;
    }
    Ok(())
}

/// Sends wheel units for `lines` (positive = down) over an eased duration at about SCROLL_SPEED.
fn ease_wheel(lines: f64, per_unit: f64) -> Result<(), String> {
    let total_units = -lines / per_unit;
    let seconds = (lines.abs() / SCROLL_SPEED).clamp(0.35, 12.0);
    let start = Instant::now();
    let (mut sent, mut owed) = (0.0f64, 0.0f64);
    loop {
        let t = (start.elapsed().as_secs_f64() / seconds).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t); // smoothstep: speeds up, cruises, slows down
        owed += total_units * eased - sent - owed;
        let step = owed.trunc();
        if step != 0.0 {
            input::wheel(step as i32).map_err(abort)?;
            sent += step;
            owed -= step;
        }
        if t >= 1.0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

fn highlight(c: &mut Ctx, file: &Path, first: u32, count: u32, pace: Pace) -> Result<(), String> {
    let last = first + count.max(1) - 1;
    let view = read_view(&mut c.session)?;
    if view.fully_visible(first).is_none() || view.fully_visible(last).is_none() {
        scroll_to(c, file, first, count, pace)?;
    }
    let map = locate::screen_map(&mut c.session, c.pid)?;
    let view = read_view(&mut c.session)?;
    let centre = |n: &NumberBox| map.point(n.x + n.w / 2.0, n.y + n.h / 2.0);
    let a = view.fully_visible(first).map(centre).ok_or(format!("line {first} isn't on screen"))?;
    let b = view.fully_visible(last).map(centre).ok_or(format!("line {last} isn't on screen (the range may be taller than the editor)"))?;
    input::move_to(a.0, a.1, pace).map_err(abort)?;
    input::click().map_err(abort)?;
    if last != first {
        input::move_to(b.0, b.1, pace).map_err(abort)?;
        input::shift_click().map_err(abort)?;
    }
    // Off the gutter: resting there shows VS Code's faint breakpoint hint in the recording.
    let rest = view.fully_visible(last).map(|n| map.point(n.x + n.w + 60.0, n.y + n.h / 2.0)).unwrap_or(b);
    input::move_to(rest.0, rest.1, pace).map_err(abort)?;
    Ok(())
}

fn palette(c: &mut Ctx, command: &str) -> Result<(), String> {
    input::chord(&[VK_CONTROL, VK_SHIFT, VK_P]).map_err(abort)?;
    wait_until(Duration::from_secs(3), || quick_input_open(&mut c.session), "open the command palette")?;
    input::type_text(command, TYPE_CPS).map_err(abort)?;
    std::thread::sleep(Duration::from_millis(500));
    input::chord(&[VK_RETURN]).map_err(abort)?;
    let session = &mut c.session;
    wait_until(Duration::from_secs(3), || quick_input_open(session).map(|open| !open), "run the command")
}

fn search(c: &mut Ctx, query: &str) -> Result<(), String> {
    input::chord(&[VK_CONTROL, VK_SHIFT, VK_F]).map_err(abort)?;
    let focused = "(() => { const a = document.activeElement; return !!a && !!a.closest('.search-view'); })()";
    let session = &mut c.session;
    wait_until(Duration::from_secs(3), || Ok(session.evaluate(focused)?.as_bool().unwrap_or(false)), "open Search")?;
    input::chord(&[VK_CONTROL, VK_A]).map_err(abort)?;
    input::type_text(query, TYPE_CPS).map_err(abort)?;
    input::chord(&[VK_RETURN]).map_err(abort)
}

// ---- Authoring: capture where VS Code is ------------------------------------------------------

/// Where the presenter is in VS Code: the file, the selected lines (or the cursor's line), as a
/// ready-made anchor.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub path: String,
    pub at: Anchor,
    pub lines: u32,
}

const SELECTION_JS: &str = r#"(() => {
  const ed = document.querySelector('.editor-group-container.active .editor-instance .monaco-editor')
    || document.querySelector('.editor-instance .monaco-editor');
  if (!ed) return null;
  const tops = new Map();
  for (const d of ed.querySelectorAll('.margin-view-overlays > div')) {
    const n = d.querySelector('.line-numbers');
    if (n && +n.textContent) tops.set(d.style.top, +n.textContent);
  }
  const rows = [...ed.querySelectorAll('.view-overlays > div')];
  const lineOf = (d) => tops.get(d.style.top);
  const selected = rows.filter((d) => d.querySelector('.selected-text')).map(lineOf).filter(Boolean);
  const current = rows.filter((d) => d.querySelector('.current-line')).map(lineOf).filter(Boolean);
  const shown = [...tops.values()];
  return { uri: ed.getAttribute('data-uri'), selected, current: current[0] || null,
    shown: shown.length ? [Math.min(...shown), Math.max(...shown)] : null };
})()"#;

/// The name of the function or type around `line` (1-based), from the nearest declaration above it.
pub fn enclosing_symbol(text: &str, line: u32) -> Option<String> {
    const KEYWORDS: [&str; 12] = ["fn", "function", "class", "def", "struct", "impl", "interface", "enum", "trait", "mod", "type", "func"];
    let lines: Vec<&str> = text.lines().collect();
    for l in lines[..(line as usize).min(lines.len())].iter().rev() {
        let words: Vec<&str> = l.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        if let Some(i) = words.iter().position(|w| KEYWORDS.contains(w))
            && let Some(name) = words.get(i + 1)
        {
            return Some(format!("{} {name}", words[i]));
        }
    }
    None
}

/// Whether a selection may go on past the rows VS Code renders (`shown`, first and last line):
/// it reaches an edge row and the file continues beyond it. VS Code only renders visible rows,
/// so the rest of such a selection can't be read.
pub(crate) fn runs_off_screen(selected: &[u32], shown: (u32, u32), file_lines: u32) -> bool {
    (shown.0 > 1 && selected.contains(&shown.0)) || (shown.1 < file_lines && selected.contains(&shown.1))
}

/// `file` as a `/`-separated path inside `folder`, matching whole path components (so
/// `C:\dev\app2\x.rs` isn't inside `C:\dev\app`), ignoring case and slash direction.
pub(crate) fn relative_to(folder: &Path, file: &Path) -> Option<String> {
    let parts = |p: &Path| -> Vec<String> {
        p.to_string_lossy().split(['\\', '/']).filter(|s| !s.is_empty()).map(str::to_owned).collect()
    };
    let (folder, file) = (parts(folder), parts(file));
    if file.len() <= folder.len() || !folder.iter().zip(&file).all(|(a, b)| a.to_lowercase() == b.to_lowercase()) {
        return None;
    }
    Some(file[folder.len()..].join("/"))
}

/// Reads VS Code's open file and selection (or cursor line) into a step position.
#[tauri::command(async)]
pub fn capture_code_position(targets: tauri::State<'_, LaunchedTarget>) -> Result<Position, String> {
    let mut c = ctx(&targets)?;
    let v = c.session.evaluate(SELECTION_JS)?;
    if v.is_null() {
        return Err("Open a file in VS Code first".into());
    }
    let file = v["uri"].as_str().and_then(uri_to_path).ok_or("VS Code's open file isn't on disk")?;
    let relative = relative_to(&c.vs.folder, &file).ok_or("That file isn't inside the codebase folder")?;
    let selected: Vec<u32> = v["selected"].as_array().into_iter().flatten().filter_map(|n| n.as_u64()).map(|n| n as u32).collect();
    let text = std::fs::read_to_string(&file).map_err(err)?;
    let shown = v["shown"].as_array().and_then(|r| Some((r.first()?.as_u64()? as u32, r.get(1)?.as_u64()? as u32)));
    if let Some(shown) = shown
        && runs_off_screen(&selected, shown, text.lines().count() as u32)
    {
        return Err("The selection runs past the edge of the editor. Scroll so all of it shows, then try again".into());
    }
    let (first, lines) = match (selected.iter().min(), selected.iter().max()) {
        (Some(a), Some(b)) => (*a, b - a + 1),
        _ => (v["current"].as_u64().ok_or("Put the cursor on a line or select some lines in VS Code")? as u32, 1),
    };
    let anchor_text = text.lines().nth(first as usize - 1).unwrap_or_default().trim().to_owned();
    Ok(Position { path: relative, at: Anchor { line: first, text: anchor_text, symbol: enclosing_symbol(&text, first) }, lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_reaching_a_hidden_edge_are_refused() {
        // Rows 86–120 of a 500-line file are rendered.
        assert!(!runs_off_screen(&[90, 91, 92], (86, 120), 500));
        assert!(runs_off_screen(&[86, 87], (86, 120), 500));
        assert!(runs_off_screen(&[119, 120], (86, 120), 500));
        // The file's own first and last lines have nothing beyond them.
        assert!(!runs_off_screen(&[1, 2], (1, 35), 500));
        assert!(!runs_off_screen(&[29, 30], (1, 30), 30));
    }

    #[test]
    fn relative_paths_need_a_whole_folder_match() {
        let f = Path::new(r"C:\dev\app");
        assert_eq!(relative_to(f, Path::new(r"c:\Dev\App\src\x.rs")).as_deref(), Some("src/x.rs"));
        assert_eq!(relative_to(Path::new("C:/dev/app/"), Path::new(r"C:\dev\app\x.rs")).as_deref(), Some("x.rs"));
        assert_eq!(relative_to(f, Path::new(r"C:\dev\app2\src\x.rs")), None);
        assert_eq!(relative_to(f, Path::new(r"C:\dev\app")), None);
    }

    const CODE: &str = "fn a() {\n    let x = 1;\n}\n\nfn b() {\n    let x = 1;\n    done();\n}\n";

    fn at(line: u32, text: &str, symbol: Option<&str>) -> Anchor {
        Anchor { line, text: text.into(), symbol: symbol.map(str::to_owned) }
    }

    /// Live: drives a throwaway VS Code on this repo with the real mouse and keyboard.
    #[test]
    #[ignore = "launches VS Code and moves the real cursor"]
    fn drives_vs_code_on_this_repo() {
        // Physical px, like the app (the test process isn't DPI-aware by default).
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let repo = PathBuf::from(repo.canonicalize().unwrap().to_string_lossy().trim_start_matches(r"\\?\"));
        let profile = std::env::temp_dir().join(format!("scenra-vscode-test-{}", std::process::id()));
        let targets = crate::target::launch_vscode_for_test(&repo, &profile).unwrap();
        let (_, port, _) = crate::target::vscode(&targets).unwrap();
        std::thread::sleep(Duration::from_secs(6)); // workbench restores
        let mut session = page::page_session(port).unwrap();
        let take = repo.join(r"scenra\src-tauri\src\take.rs");
        let text = std::fs::read_to_string(&take).unwrap();
        let stop_line = text.lines().position(|l| l.trim_start().starts_with("pub(crate) fn stop(")).unwrap() as u32 + 1;
        let anchor_text = text.lines().nth(stop_line as usize - 1).unwrap().trim().to_owned();
        let shot = |_: &str| {
            if let Ok(mut s) = page::page_session(port) {
                let dump = s.evaluate("(() => { const w = document.querySelector('.quick-input-widget'); return { open: !!w && w.style.display !== 'none', value: w?.querySelector('input')?.value, rows: [...(w?.querySelectorAll('.monaco-list-row') || [])].slice(0, 5).map(r => r.getAttribute('aria-label')), active: document.activeElement?.className, uri: document.querySelector('.editor-instance .monaco-editor')?.getAttribute('data-uri'), title: document.title }; })()");
                eprintln!("  state at failure: {dump:?}");
            }
        };
        let run_step = |step: Step| {
            let t0 = Instant::now();
            approach(&targets, &step, Pace::Natural).unwrap();
            if let Err(e) = run(&targets, &step, Pace::Natural) {
                shot("failure");
                panic!("{}: {e}", step.describe());
            }
            eprintln!("{} ok in {:.1} s", step.describe(), t0.elapsed().as_secs_f64());
        };

        run_step(Step::Open { path: "scenra/src-tauri/src/take.rs".into(), line: None, instant: false });
        assert!(read_view(&mut session).unwrap().shows(&take));

        let at = Anchor { line: stop_line + 7, text: anchor_text, symbol: None }; // a stale hint on purpose
        run_step(Step::Scroll { path: "scenra/src-tauri/src/take.rs".into(), at: at.clone(), lines: 6 });
        let view = read_view(&mut session).unwrap();
        let centre = view.top().unwrap() + view.rows() / 2.0;
        eprintln!("  target {stop_line}–{} centred at {centre:.1}", stop_line + 5);
        assert!((centre - (f64::from(stop_line) + 3.0)).abs() < 2.5, "centre {centre:.1}, wanted ≈{}", stop_line + 3);

        run_step(Step::Highlight { path: "scenra/src-tauri/src/take.rs".into(), at, lines: 6 });
        let selected = session.evaluate("document.querySelectorAll('.editor-group-container.active .selected-text').length").unwrap();
        assert!(selected.as_u64().unwrap_or(0) > 0, "no selection drawn");

        run_step(Step::Palette { command: "View: Show Explorer".into() });
        run_step(Step::Search { query: "fn stop_take".into() });

        // A long file and a long way: jump, then scroll the tail.
        run_step(Step::Open { path: "Cargo.lock".into(), line: None, instant: true });
        run_step(Step::Scroll { path: "Cargo.lock".into(), at: Anchor { line: 3000, text: String::new(), symbol: None }, lines: 4 });
        let view = read_view(&mut session).unwrap();
        let centre = view.top().unwrap() + view.rows() / 2.0;
        assert!((centre - 3002.0).abs() < 3.0, "Cargo.lock centre {centre:.1}");

        crate::target::stop_for_test(&targets);
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn finds_the_enclosing_symbol() {
        let code = "use x;

pub(crate) fn stop(take: &CurrentTake) {
    let a = 1;
}
class Foo:
    x = 1
";
        assert_eq!(enclosing_symbol(code, 4).as_deref(), Some("fn stop"));
        assert_eq!(enclosing_symbol(code, 7).as_deref(), Some("class Foo"));
        assert_eq!(enclosing_symbol(code, 1), None);
    }

    #[test]
    fn anchors_follow_moved_code() {
        // The script said line 3; the code moved down by 4.
        assert_eq!(find_anchor(CODE, &at(3, "done();", None)), Ok(7));
    }

    #[test]
    fn identical_lines_are_told_apart_by_symbol_then_hint() {
        assert_eq!(find_anchor(CODE, &at(1, "let x = 1;", Some("fn b"))), Ok(6));
        assert_eq!(find_anchor(CODE, &at(2, "let x = 1;", None)), Ok(2));
        assert_eq!(find_anchor(CODE, &at(9, "let x = 1;", None)), Ok(6));
        // `fn stop` is not `fn stop_take`, even though one name contains the other.
        let code = "fn stop() {\n    let x = 1;\n}\nfn stop_take() {\n    let x = 1;\n}\n";
        assert_eq!(find_anchor(code, &at(5, "let x = 1;", Some("fn stop"))), Ok(2));
    }

    #[test]
    fn a_vanished_anchor_is_refused() {
        assert!(find_anchor(CODE, &at(3, "gone();", None)).unwrap_err().contains("no longer"));
    }

    #[test]
    fn steps_round_trip_as_json() {
        let step = Step::Highlight { path: "src/take.rs".into(), at: at(120, "pub fn stop", Some("stop_take")), lines: 6 };
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains(r#""kind":"highlight""#), "{json}");
        assert_eq!(serde_json::from_str::<Step>(&json).unwrap(), step);
        let open: Step = serde_json::from_str(r#"{ "kind": "open", "path": "README.md" }"#).unwrap();
        assert_eq!(open, Step::Open { path: "README.md".into(), line: None, instant: false });
    }

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(uri_to_path("file:///c%3A/dev/Scenera/a%20b.rs"), Some(PathBuf::from(r"c:\dev\Scenera\a b.rs")));
        assert!(same_path(Path::new(r"C:\Dev\X.rs"), Path::new(r"c:\dev\x.rs")));
        assert!(same_path(Path::new(r"C:/dev/Scenera\a.rs"), Path::new(r"c:\dev\scenera\a.rs")));
    }

    #[test]
    fn the_top_line_is_fractional() {
        let view = View {
            uri: None,
            area: Area { x: 0.0, y: 90.0, width: 500.0, height: 570.0 },
            lines: vec![NumberBox { line: 105, x: 0.0, y: 80.0, w: 20.0, h: 19.0 }, NumberBox { line: 106, x: 0.0, y: 99.0, w: 20.0, h: 19.0 }],
        };
        assert!((view.top().unwrap() - (105.0 + 10.0 / 19.0)).abs() < 1e-9);
        assert_eq!(view.rows(), 30.0);
        assert!(view.fully_visible(105).is_none());
        assert!(view.fully_visible(106).is_some());
    }
}
