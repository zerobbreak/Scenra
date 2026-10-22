//! Coordinate mapper (PROJECT_OUTLINE §6): CSS px in the page → physical screen px.
//!
//! `screen_px = viewport_origin_screen_px + (css_px − visualViewport.offset) × visualViewport.scale × devicePixelRatio`
//!
//! The viewport origin is the screen position of Chromium's render widget
//! (`Chrome_RenderWidgetHostHWND`), the child window that draws the page, so the title bar,
//! tabs and toolbars are excluded. That works the same for Chrome and WebView2. Physical px need
//! a per-monitor DPI-aware process, which Tauri apps are.
//!
//! The hover probe moves the real cursor to the computed point and reads back the `clientX/Y`
//! the page saw. More than 2 CSS px off means the mapping is wrong, so don't click.

use crate::cdp::Session;
use crate::input::{self, Abort, Pace};
use crate::target::process_tree;
use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumChildWindows, EnumWindows, GA_ROOT, GetAncestor, GetClassNameW,
    GetClientRect, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, HWND_TOP, IsIconic,
    IsWindowVisible, SW_RESTORE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowPos,
    ShowWindow, WindowFromPoint,
};
use windows::core::BOOL;

pub const MAX_PROBE_ERROR_CSS: f64 = 2.0;

/// How to find an element: by `data-testid` (preferred) or by its visible label.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Selector {
    TestId(String),
    Label(String),
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Selector::TestId(id) => write!(f, "[data-testid={id}]"),
            Selector::Label(label) => write!(f, "“{label}”"),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Located {
    /// In the page's layout viewport, CSS px.
    pub css: Rect,
    /// On screen, physical px.
    pub screen: Rect,
    pub device_pixel_ratio: f64,
    /// Content area (render widget) on screen, physical px.
    pub viewport: Rect,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub located: Located,
    /// Where the page saw the cursor, CSS px; `None` if no mousemove reached the page
    /// (something covers the point, e.g. the teleprompter).
    pub seen_css: Option<(f64, f64)>,
    pub error_css: Option<f64>,
    pub passed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageGeometry {
    rect: Option<CssRect>,
    dpr: f64,
    vv_left: f64,
    vv_top: f64,
    vv_scale: f64,
    inner_width: f64,
}

#[derive(Deserialize)]
struct CssRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn find_js(selector: &Selector) -> String {
    let find = match selector {
        Selector::TestId(id) => format!("document.querySelector({})", json_str(&format!("[data-testid=\"{id}\"]"))),
        Selector::Label(label) => format!(
            r#"(() => {{
              const clean = (s) => (s || '').replace(/\s+/g, ' ').trim().toLowerCase();
              const full = clean({label});
              const els = [...document.querySelectorAll('a[href],button,[role=button],input,textarea,select')]
                .filter((e) => e.offsetParent !== null);
              const text = (e) => clean(e.innerText || e.getAttribute('aria-label') || e.placeholder);
              // Labels can change after a click (a flashcard flips), so fall back to the first word.
              return els.find((e) => text(e).startsWith(full))
                || els.find((e) => text(e).split(' ')[0] === full.split(' ')[0]);
            }})()"#,
            label = json_str(label)
        ),
    };
    format!(
        r#"(() => {{
          const el = {find};
          let rect = null;
          if (el) {{
            el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }});
            const r = el.getBoundingClientRect();
            rect = {{ x: r.left, y: r.top, width: r.width, height: r.height }};
          }}
          const vv = window.visualViewport;
          return {{ rect, dpr: devicePixelRatio, vvLeft: vv ? vv.offsetLeft : 0, vvTop: vv ? vv.offsetTop : 0,
                   vvScale: vv ? vv.scale : 1, innerWidth }};
        }})()"#
    )
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Finds the element and maps it to physical screen px. `pid` is the target's process.
pub fn locate(session: &mut Session, pid: u32, selector: &Selector) -> Result<Located, String> {
    let geometry: PageGeometry =
        serde_json::from_value(session.evaluate(&find_js(selector))?).map_err(|e| e.to_string())?;
    let css = geometry.rect.ok_or_else(|| format!("no element {selector} on the page"))?;
    if css.width <= 0.0 || css.height <= 0.0 {
        return Err(format!("{selector} has no visible area"));
    }
    let (top, widget, viewport) = render_widget(pid)?;
    bring_to_front(top);
    // The widget must be as wide as the page says its viewport is, or we found the wrong one.
    let expected = geometry.inner_width * geometry.dpr;
    if (viewport.width - expected).abs() > expected * 0.03 + 2.0 {
        return Err(format!(
            "content area is {} px wide but the page reports {expected:.0} px: wrong window?",
            viewport.width
        ));
    }
    let scale = geometry.vv_scale * geometry.dpr;
    let to_screen = |x: f64, y: f64| {
        (viewport.x + (x - geometry.vv_left) * scale, viewport.y + (y - geometry.vv_top) * scale)
    };
    let (sx, sy) = to_screen(css.x, css.y);
    let (cx, cy) = (sx + css.width * scale / 2.0, sy + css.height * scale / 2.0);
    if let Some(what) = covering(cx, cy, widget) {
        return Err(format!("{what} is covering {selector} on screen"));
    }
    Ok(Located {
        css: Rect { x: css.x, y: css.y, width: css.width, height: css.height },
        screen: Rect { x: sx, y: sy, width: css.width * scale, height: css.height * scale },
        device_pixel_ratio: geometry.dpr,
        viewport,
    })
}

const PROBE_INSTALL: &str = r#"(() => {
  if (!window.__scenraProbeInstalled) {
    window.__scenraProbeInstalled = true;
    addEventListener('mousemove', (e) => { window.__scenraProbe = { x: e.clientX, y: e.clientY }; },
      { capture: true, passive: true });
  }
  window.__scenraProbe = null;
  return true;
})()"#;

/// Locates, moves the real cursor to the centre and checks where the page saw it.
pub fn probe(session: &mut Session, pid: u32, selector: &Selector, pace: Pace) -> Result<Probe, ProbeError> {
    let located = locate(session, pid, selector).map_err(ProbeError::Locate)?;
    session.evaluate(PROBE_INSTALL).map_err(ProbeError::Locate)?;
    let (x, y) = located.screen.center();
    // The page only reports where the cursor is when it moves. If it's already there (say, after
    // a previous probe), step off first so the arrival produces a mousemove.
    let (cx, cy) = input::cursor();
    if (cx as f64 - x).hypot(cy as f64 - y) < 4.0 {
        input::move_to(x + 14.0, y + 8.0, Pace::Fast).map_err(ProbeError::Aborted)?;
    }
    input::move_to(x, y, pace).map_err(ProbeError::Aborted)?;
    // Chromium dispatches mousemove aligned to frames, so the event can land a few frames after
    // the cursor does. Poll briefly, and wiggle by a pixel once to force a fresh event.
    let mut seen_css = None;
    for attempt in 0..10 {
        std::thread::sleep(Duration::from_millis(50));
        let seen = session.evaluate("window.__scenraProbe").map_err(ProbeError::Locate)?;
        seen_css = seen["x"].as_f64().zip(seen["y"].as_f64());
        if seen_css.is_some() {
            break;
        }
        if attempt == 4 {
            input::move_to(x + 1.0, y, Pace::Fast).map_err(ProbeError::Aborted)?;
            input::move_to(x, y, Pace::Fast).map_err(ProbeError::Aborted)?;
        }
    }
    let (cx, cy) = located.css.center();
    let error_css = seen_css.map(|(sx, sy)| (sx - cx).hypot(sy - cy));
    Ok(Probe {
        passed: error_css.is_some_and(|e| e <= MAX_PROBE_ERROR_CSS),
        located,
        seen_css,
        error_css,
    })
}

pub enum ProbeError {
    Locate(String),
    Aborted(Abort),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Locate(e) => f.write_str(e),
            ProbeError::Aborted(a) => write!(f, "{a}"),
        }
    }
}

/// Makes the target the foreground window, as a presenter would switch to it. Anything on top
/// of it (including Scenra's own panel) would otherwise get the cursor and the click.
fn bring_to_front(top: HWND) {
    unsafe {
        if IsIconic(top).as_bool() {
            let _ = ShowWindow(top, SW_RESTORE);
        }
        // Windows only lets the foreground thread raise other windows. Sharing its input state
        // for a moment lets Scenra do it even when its own window isn't in front.
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let me = GetCurrentThreadId();
        let attached = foreground != 0 && foreground != me && AttachThreadInput(me, foreground, true).as_bool();
        let _ = SetForegroundWindow(top);
        let _ = BringWindowToTop(top);
        let _ = SetWindowPos(top, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        if attached {
            let _ = AttachThreadInput(me, foreground, false);
        }
    }
    std::thread::sleep(Duration::from_millis(200));
}

/// What is at `(x, y)` if it isn't the page's own content area: another app's window, or one of
/// the target's own popups (an autofill list, a native menu).
fn covering(x: f64, y: f64, widget: HWND) -> Option<String> {
    unsafe {
        let hit = WindowFromPoint(POINT { x: x as i32, y: y as i32 });
        if hit == widget {
            return None;
        }
        let root = GetAncestor(hit, GA_ROOT);
        let mut title = [0u16; 200];
        let len = GetWindowTextW(root, &mut title) as usize;
        let title = String::from_utf16_lossy(&title[..len]);
        Some(if title.is_empty() { "a popup or untitled window".to_owned() } else { format!("“{title}”") })
    }
}

/// The largest visible `Chrome_RenderWidgetHostHWND` under the target's top-level windows: the
/// top-level window that holds it, the widget itself, and its screen rect.
fn render_widget(pid: u32) -> Result<(HWND, HWND, Rect), String> {
    struct Search {
        pids: HashSet<u32>,
        current_top: HWND,
        best: Option<(i64, HWND, HWND, Rect)>,
    }

    unsafe extern "system" fn child(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut class = [0u16; 64];
        let len = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
        if String::from_utf16_lossy(&class[..len]) == "Chrome_RenderWidgetHostHWND"
            && unsafe { IsWindowVisible(hwnd) }.as_bool()
        {
            let mut rect = RECT::default();
            let mut origin = POINT::default();
            if unsafe { GetClientRect(hwnd, &mut rect) }.is_ok() && unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
                let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
                let area = w as i64 * h as i64;
                if area > search.best.as_ref().map_or(0, |b| b.0) {
                    let r = Rect { x: origin.x as f64, y: origin.y as f64, width: w as f64, height: h as f64 };
                    search.best = Some((area, search.current_top, hwnd, r));
                }
            }
        }
        true.into()
    }

    unsafe extern "system" fn top(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if unsafe { IsWindowVisible(hwnd) }.as_bool() && search.pids.contains(&owner) {
            search.current_top = hwnd;
            let _ = unsafe { EnumChildWindows(Some(hwnd), Some(child), lparam) };
        }
        true.into()
    }

    let mut search = Search { pids: process_tree(pid), current_top: HWND::default(), best: None };
    let _ = unsafe { EnumWindows(Some(top), LPARAM(&mut search as *mut Search as isize)) };
    search.best.map(|b| (b.1, b.2, b.3)).ok_or_else(|| "couldn't find the page's content area on screen".into())
}
