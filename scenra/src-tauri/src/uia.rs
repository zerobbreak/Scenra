//! Windows UI Automation: what apps without a debugging protocol show on screen (docs/adr/0004,
//! and slice 3 of docs/adr/0003).
//!
//! For Visual Studio this reads the code editor through UI Automation's TextPattern: one range
//! per non-empty visible line, as `"<line number> <code>"`, each with its screen rectangle.

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::core::BOOL;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextRange, TreeScope_Descendants,
    UIA_ClassNamePropertyId, UIA_TextPatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible};

fn err(e: windows::core::Error) -> String {
    e.message().to_string()
}

/// A screen rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One visible line of a code editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// 1-based.
    pub number: u32,
    pub code: String,
    pub rect: Rect,
}

/// The largest visible top-level window of process `pid`.
pub fn main_window(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        best: Option<(i64, HWND)>,
    }
    unsafe extern "system" fn top(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        let mut rect = RECT::default();
        if owner == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() && unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
            let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
            if area > search.best.map_or(0, |b| b.0) {
                search.best = Some((area, hwnd));
            }
        }
        true.into()
    }
    let mut search = Search { pid, best: None };
    let _ = unsafe { EnumWindows(Some(top), LPARAM(&mut search as *mut Search as isize)) };
    search.best.map(|b| b.1)
}

/// Every visible top-level window of process `pid`.
pub fn windows_of(pid: u32) -> Vec<HWND> {
    struct Search {
        pid: u32,
        found: Vec<HWND>,
    }
    unsafe extern "system" fn top(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            search.found.push(hwnd);
        }
        true.into()
    }
    let mut search = Search { pid, found: Vec::new() };
    let _ = unsafe { EnumWindows(Some(top), LPARAM(&mut search as *mut Search as isize)) };
    search.found
}

/// `"195 pub(crate) fn stop(…"` → line 195 and its code. Visual Studio prefixes each visible
/// line's accessible text with its number and one space.
pub fn split_numbered(text: &str) -> Option<(u32, &str)> {
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let number = text[..digits].parse().ok()?;
    let rest = &text[digits..];
    Some((number, rest.strip_prefix(' ').unwrap_or(rest).trim_end_matches(['\r', '\n'])))
}

/// The rectangles a text range covers: one per line.
fn rects(range: &IUIAutomationTextRange) -> Result<Vec<Rect>, String> {
    unsafe {
        let array: *mut SAFEARRAY = range.GetBoundingRectangles().map_err(err)?;
        if array.is_null() {
            return Ok(vec![]);
        }
        let upper = SafeArrayGetUBound(array, 1).unwrap_or(-1);
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut out = Vec::new();
        if upper >= 3 && SafeArrayAccessData(array, &mut data).is_ok() {
            let values = std::slice::from_raw_parts(data as *const f64, (upper + 1) as usize);
            out = values.as_chunks::<4>().0.iter().map(|&[x, y, width, height]| Rect { x, y, width, height }).collect();
            let _ = SafeArrayUnaccessData(array);
        }
        let _ = SafeArrayDestroy(array);
        Ok(out)
    }
}

/// An element's runtime id: unique while it exists, new when it's created again.
fn runtime_id(element: &IUIAutomationElement) -> Vec<i32> {
    unsafe {
        let Ok(array) = element.GetRuntimeId() else { return vec![] };
        if array.is_null() {
            return vec![];
        }
        let upper = SafeArrayGetUBound(array, 1).unwrap_or(-1);
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut out = Vec::new();
        if upper >= 0 && SafeArrayAccessData(array, &mut data).is_ok() {
            out = std::slice::from_raw_parts(data as *const i32, (upper + 1) as usize).to_vec();
            let _ = SafeArrayUnaccessData(array);
        }
        let _ = SafeArrayDestroy(array);
        out
    }
}

/// A UI Automation client. COM must be initialised on the calling thread.
pub struct Uia(IUIAutomation);

impl Uia {
    pub fn new() -> Result<Self, String> {
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.map(Uia).map_err(err)
    }

    /// The first element of class `class` inside window `hwnd`.
    pub fn find_class(&self, hwnd: HWND, class: &str) -> Result<IUIAutomationElement, String> {
        unsafe {
            let root = self.0.ElementFromHandle(hwnd).map_err(err)?;
            let condition = self.0.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(class)).map_err(err)?;
            root.FindFirst(TreeScope_Descendants, &condition).map_err(|_| format!("no {class} in the window"))
        }
    }

    fn text(element: &IUIAutomationElement) -> Result<IUIAutomationTextPattern, String> {
        unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }.map_err(|_| "the editor doesn't expose its text".into())
    }

    /// The editor's visible, non-empty lines, top to bottom. Visual Studio also reports a few
    /// laid-out lines below the editor's edge; only lines at least partly inside it count.
    pub fn visible_lines(&self, editor: &IUIAutomationElement) -> Result<Vec<Line>, String> {
        let pattern = Self::text(editor)?;
        let bounds = self.bounds(editor)?;
        let mut lines = Vec::new();
        unsafe {
            let ranges = pattern.GetVisibleRanges().map_err(err)?;
            for i in 0..ranges.Length().map_err(err)? {
                let range = ranges.GetElement(i).map_err(err)?;
                let text = range.GetText(-1).map_err(err)?.to_string();
                let (Some((number, code)), Some(rect)) = (split_numbered(&text), rects(&range)?.first().copied()) else { continue };
                if rect.y + rect.height <= bounds.y || rect.y >= bounds.y + bounds.height {
                    continue;
                }
                lines.push(Line { number, code: code.to_owned(), rect });
            }
        }
        Ok(lines)
    }

    /// The texts inside the selected list item in any of process `pid`'s windows (a search
    /// popup is a window of its own), or `None` while there isn't one.
    pub fn selected_item_texts(&self, pid: u32) -> Option<Vec<String>> {
        use windows::Win32::UI::Accessibility::{IUIAutomationSelectionItemPattern, TreeScope_Subtree, UIA_ControlTypePropertyId, UIA_ListItemControlTypeId, UIA_SelectionItemPatternId};
        unsafe {
            let items = self.0.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ListItemControlTypeId.0)).ok()?;
            let any = self.0.CreateTrueCondition().ok()?;
            for hwnd in windows_of(pid) {
                let Ok(root) = self.0.ElementFromHandle(hwnd) else { continue };
                let Ok(found) = root.FindAll(TreeScope_Descendants, &items) else { continue };
                for i in 0..found.Length().unwrap_or(0) {
                    let Ok(item) = found.GetElement(i) else { continue };
                    let selected = item
                        .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                        .and_then(|p| p.CurrentIsSelected())
                        .is_ok_and(|s| s.as_bool());
                    if !selected {
                        continue;
                    }
                    let mut texts = Vec::new();
                    if let Ok(inside) = item.FindAll(TreeScope_Subtree, &any) {
                        for j in 0..inside.Length().unwrap_or(0) {
                            if let Ok(name) = inside.GetElement(j).and_then(|e| e.CurrentName()) {
                                texts.push(name.to_string());
                            }
                        }
                    }
                    return Some(texts);
                }
            }
        }
        None
    }

    /// The first text block in window `hwnd` whose text starts with `prefix`, with the element's
    /// runtime id (it changes when the element is created again, e.g. results redrawn).
    pub fn text_starting(&self, hwnd: HWND, prefix: &str) -> Option<(String, Vec<i32>)> {
        unsafe {
            let root = self.0.ElementFromHandle(hwnd).ok()?;
            let condition = self.0.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from("TextBlock")).ok()?;
            let found = root.FindAll(TreeScope_Descendants, &condition).ok()?;
            for i in 0..found.Length().unwrap_or(0) {
                let Ok(element) = found.GetElement(i) else { continue };
                let Ok(name) = element.CurrentName() else { continue };
                let name = name.to_string();
                if name.starts_with(prefix) {
                    return Some((name, runtime_id(&element)));
                }
            }
        }
        None
    }

    /// The element's screen rectangle.
    pub fn bounds(&self, element: &IUIAutomationElement) -> Result<Rect, String> {
        let r = unsafe { element.CurrentBoundingRectangle() }.map_err(err)?;
        Ok(Rect { x: r.left as f64, y: r.top as f64, width: (r.right - r.left) as f64, height: (r.bottom - r.top) as f64 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_lines_carry_their_number() {
        assert_eq!(split_numbered("195 pub(crate) fn stop("), Some((195, "pub(crate) fn stop(")));
        assert_eq!(split_numbered("178     Ok(dir_str)"), Some((178, "    Ok(dir_str)")));
        assert_eq!(split_numbered("179 }\r\n"), Some((179, "}")));
        assert_eq!(split_numbered("fn main()"), None);
    }

    /// Live: needs a Visual Studio running on this repo with `take.rs` open.
    #[test]
    #[ignore = "needs a running Visual Studio"]
    fn reads_visual_studios_editor() {
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED).ok().unwrap() };
        let pid = crate::target::process_ids("devenv.exe").into_iter().next().expect("start Visual Studio first");
        let uia = Uia::new().unwrap();
        let editor = uia.find_class(main_window(pid).unwrap(), "WpfTextView").unwrap();
        let bounds = uia.bounds(&editor).unwrap();
        let lines = uia.visible_lines(&editor).unwrap();
        eprintln!("editor {bounds:?}; {} lines, {}–{}", lines.len(), lines[0].number, lines.last().unwrap().number);
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), r"\src\take.rs")).unwrap();
        for line in &lines {
            assert_eq!(line.code.trim_end(), text.lines().nth(line.number as usize - 1).unwrap().trim_end(), "line {}", line.number);
            assert!(line.rect.y >= bounds.y - 1.0 && line.rect.y < bounds.y + bounds.height, "{line:?} outside {bounds:?}");
        }
        assert!(lines.windows(2).all(|w| w[0].number < w[1].number && w[0].rect.y < w[1].rect.y));
    }
}
