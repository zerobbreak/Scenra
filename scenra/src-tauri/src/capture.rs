//! Phase 0 · Step 1 — keep the teleprompter out of screen capture.
//!
//! The teleprompter is its own top-level window. `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`
//! on its HWND asks DWM to leave it out of captured frames while it stays visible on the monitor.
//! The flag needs Windows 10 2004 (build 19041); older builds treat it like `WDA_MONITOR`
//! and capture a black box instead.
//!
//! OBS is the real gate (see docs/phase0/01-capture-exclusion.md). `probe_capture` is a
//! quick self-check: it grabs the teleprompter's screen rect with GDI `BitBlt` and counts
//! pixels of the magenta marker band the teleprompter draws. Marker found = window leaks.

use serde::Serialize;
use crate::settings;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, GetWindowRect, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    WDA_NONE,
};

pub const TELEPROMPTER: &str = "teleprompter";
pub const CONTROLS: &str = "controls";
const MIN_BUILD: u32 = 19041;
/// Logical px. Keep in sync with the layouts in `Teleprompter.tsx` and `Controls.tsx`.
const PILL_SIZE: (f64, f64) = (840.0, 112.0);
const PILL_MARGIN: f64 = 30.0;
/// Four 44 px buttons, 6 px gaps and padding, 1 px border.
const CONTROLS_SIZE: (f64, f64) = (208.0, 58.0);
const CONTROLS_MARGIN: f64 = 40.0;

/// The teleprompter draws a #FF00FF strip (see `Teleprompter.tsx`). WebView2 colour-manages
/// for the display profile, so on screen it can land well off that value (#EA33F7 was seen).
/// Match "strong magenta" instead: red and blue high, green low.
pub(crate) fn is_marker(r: u8, g: u8, b: u8) -> bool {
    r >= 0xC0 && b >= 0xC0 && g <= 0x70
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    open: bool,
    /// Raw value read back from `GetWindowDisplayAffinity` (0 = none, 0x11 = exclude).
    affinity: Option<u32>,
    excluded: bool,
    os_build: u32,
    supported: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    /// Physical-pixel rect that was captured.
    rect: [i32; 4],
    marker_pixels: usize,
    total_pixels: usize,
    /// True when the teleprompter shows up in a GDI screen grab.
    leaks: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn hwnd(window: &WebviewWindow) -> Result<HWND, String> {
    // Tauri may link a different `windows` version, so convert through the raw handle.
    Ok(HWND(window.hwnd().map_err(err)?.0 as _))
}

fn os_build() -> u32 {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // RtlGetVersion isn't subject to the manifest-based version lie of GetVersionExW.
    let _ = unsafe { RtlGetVersion(&mut info) };
    info.dwBuildNumber
}

/// Frameless windows have square corners; ask DWM for the Windows 11 rounded ones instead of a
/// transparent window, so capture exclusion is tested on a plain, non-layered HWND.
/// Windows 10 ignores the attribute and keeps square corners.
fn round_corners(window: &WebviewWindow) -> Result<(), String> {
    let preference = DWMWCP_ROUND;
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd(window)?,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            size_of_val(&preference) as u32,
        )
    };
    Ok(())
}

fn set_excluded(window: &WebviewWindow, excluded: bool) -> Result<(), String> {
    let affinity = if excluded { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe { SetWindowDisplayAffinity(hwnd(window)?, affinity) }.map_err(err)
}

fn status(app: &AppHandle) -> Result<CaptureStatus, String> {
    let build = os_build();
    let affinity = match app.get_webview_window(TELEPROMPTER) {
        Some(window) => {
            let mut value = 0u32;
            unsafe { GetWindowDisplayAffinity(hwnd(&window)?, &mut value) }.map_err(err)?;
            Some(value)
        }
        None => None,
    };
    Ok(CaptureStatus {
        open: affinity.is_some(),
        affinity,
        excluded: affinity == Some(WDA_EXCLUDEFROMCAPTURE.0),
        os_build: build,
        supported: build >= MIN_BUILD,
    })
}

#[derive(Clone, Copy)]
enum Edge {
    Top,
    Bottom,
}

/// Each overlay's designed size (logical px) and where it sits.
fn layout(label: &str) -> ((f64, f64), Edge, f64) {
    if label == CONTROLS {
        (CONTROLS_SIZE, Edge::Bottom, CONTROLS_MARGIN)
    } else {
        (PILL_SIZE, Edge::Top, PILL_MARGIN)
    }
}

/// Keep at least this much of the work area free on each side, logical px.
const SIDE_GAP: f64 = 16.0;

/// Opens (or reuses) a frameless, always-on-top overlay window.
fn overlay(app: &AppHandle, label: &str, title: &str) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(label) {
        return Ok(window);
    }
    let (size, _, _) = layout(label);
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::default())
        .title(title)
        .inner_size(size.0, size.1)
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .build()
        .map_err(err)?;
    fit(app, &window, label)?;
    round_corners(&window)?;
    Ok(window)
}

/// Sizes the overlay by the presenter's zoom (shrunk if it wouldn't fit the monitor's work area)
/// and centres it horizontally, `margin` logical px from its edge. Windows display scaling is
/// applied on top by the logical → physical conversion, so this works at any scaling.
fn fit(app: &AppHandle, window: &WebviewWindow, label: &str) -> Result<(), String> {
    let (size, edge, margin) = layout(label);
    let monitor = window.current_monitor().map_err(err)?.or(app.primary_monitor().map_err(err)?);
    let Some(monitor) = monitor else { return Ok(()) };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let origin = area.position.to_logical::<f64>(scale);
    let area = area.size.to_logical::<f64>(scale);
    let wanted = settings::load(app).overlay_zoom.clamp(settings::MIN_ZOOM, settings::MAX_ZOOM);
    let zoom = fitted_zoom(wanted, area.width, area.height, size);
    let (w, h) = (size.0 * zoom, size.1 * zoom);
    window.set_size(LogicalSize::new(w, h)).map_err(err)?;
    window.set_zoom(zoom).map_err(err)?;
    let x = origin.x + (area.width - w) / 2.0;
    let y = match edge {
        Edge::Top => origin.y + margin,
        Edge::Bottom => origin.y + area.height - h - margin,
    };
    window.set_position(LogicalPosition::new(x, y)).map_err(err)
}

/// Re-fits the open overlays after the zoom setting changed.
pub fn refit(app: &AppHandle) -> Result<(), String> {
    for window in overlays(app) {
        fit(app, &window, window.label())?;
    }
    Ok(())
}

/// The pill and its control bar are one unit: opened, excluded and closed together.
fn overlays(app: &AppHandle) -> impl Iterator<Item = WebviewWindow> {
    [TELEPROMPTER, CONTROLS].into_iter().filter_map(|label| app.get_webview_window(label))
}

// Async: on Windows, building a webview window inside a sync command (main thread) leaves the
// new webview stuck on about:blank.
#[tauri::command(async)]
pub fn open_teleprompter(app: AppHandle, excluded: bool) -> Result<CaptureStatus, String> {
    // Design C2: the script pill near the top, the minimised control bar near the bottom.
    let pill = overlay(&app, TELEPROMPTER, "Scenra Teleprompter")?;
    overlay(&app, CONTROLS, "Scenra Controls")?;
    for window in overlays(&app) {
        set_excluded(&window, excluded)?;
    }
    pill.set_focus().map_err(err)?;
    status(&app)
}

#[tauri::command]
pub fn close_teleprompter(app: AppHandle) -> Result<CaptureStatus, String> {
    for window in overlays(&app) {
        window.destroy().map_err(err)?;
    }
    // The control bar's Stop button closes the overlays too; tell the debug panel.
    app.emit("teleprompter-closed", ()).map_err(err)?;
    // Off the main thread `destroy` is asynchronous and the dying window has no HWND, so don't
    // ask it for its affinity.
    let build = os_build();
    Ok(CaptureStatus { open: false, affinity: None, excluded: false, os_build: build, supported: build >= MIN_BUILD })
}

#[tauri::command]
pub fn set_capture_excluded(app: AppHandle, excluded: bool) -> Result<CaptureStatus, String> {
    app.get_webview_window(TELEPROMPTER).ok_or("teleprompter is not open")?;
    for window in overlays(&app) {
        set_excluded(&window, excluded)?;
    }
    status(&app)
}

#[tauri::command]
pub fn capture_status(app: AppHandle) -> Result<CaptureStatus, String> {
    status(&app)
}

#[tauri::command]
pub fn probe_capture(app: AppHandle) -> Result<ProbeResult, String> {
    let window = app.get_webview_window(TELEPROMPTER).ok_or("teleprompter is not open")?;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd(&window)?, &mut rect) }.map_err(err)?;
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    if w <= 0 || h <= 0 {
        return Err("teleprompter has no visible area".into());
    }

    let pixels = grab_screen(rect.left, rect.top, w, h)?;
    let marker_pixels = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| is_marker(p[2], p[1], p[0]))
        .count();
    let total_pixels = (w * h) as usize;

    Ok(ProbeResult {
        rect: [rect.left, rect.top, w, h],
        marker_pixels,
        total_pixels,
        // The strip covers ~3.5% of the window; 1% avoids false hits from stray magenta.
        leaks: marker_pixels * 100 > total_pixels,
    })
}

/// Copies a physical-pixel screen rect into a top-down BGRA buffer.
fn grab_screen(x: i32, y: i32, w: i32, h: i32) -> Result<Vec<u8>, String> {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let previous = SelectObject(mem, bitmap.into());

        let copied = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY);

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // negative = top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        SelectObject(mem, previous);
        let lines = GetDIBits(
            mem,
            bitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );

        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        copied.map_err(err)?;
        if lines == 0 {
            return Err("GetDIBits failed".into());
        }
        Ok(pixels)
    }
}

/// Reuse the same fitting rules when the reader opens/closes peek; preserve a dragged position.
#[tauri::command]
pub fn resize_teleprompter(app: AppHandle, expanded: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window(TELEPROMPTER) else { return Ok(()) };
    let Some(monitor) = window.current_monitor().map_err(err)?.or(app.primary_monitor().map_err(err)?) else { return Ok(()) };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let size = (PILL_SIZE.0, if expanded { 318.0 } else { PILL_SIZE.1 });
    let wanted = settings::load(&app).overlay_zoom.clamp(settings::MIN_ZOOM, settings::MAX_ZOOM);
    let zoom = fitted_zoom(wanted, f64::from(area.size.width) / scale, f64::from(area.size.height) / scale, size);
    window.set_zoom(zoom).map_err(err)?;
    window.set_size(LogicalSize::new(size.0 * zoom, size.1 * zoom)).map_err(err)?;
    let pos = window.outer_position().map_err(err)?;
    let max_x = area.position.x + area.size.width as i32 - (size.0 * zoom * scale).ceil() as i32;
    let max_y = area.position.y + area.size.height as i32 - (size.1 * zoom * scale).ceil() as i32;
    window.set_position(tauri::PhysicalPosition::new(
        pos.x.clamp(area.position.x, max_x.max(area.position.x)),
        pos.y.clamp(area.position.y, max_y.max(area.position.y)),
    )).map_err(err)
}

fn fitted_zoom(wanted: f64, width: f64, height: f64, size: (f64, f64)) -> f64 {
    wanted.min((width - 2.0 * SIDE_GAP).max(1.0) / size.0).min(height.max(1.0) / size.1)
}

#[cfg(test)]
mod fit_tests {
    use super::*;
    #[test]
    fn phase_resize_preserves_zoom_and_fits_peek_on_small_displays() {
        assert_eq!(fitted_zoom(0.75, 1280.0, 680.0, PILL_SIZE), 0.75);
        assert_eq!(fitted_zoom(0.9, 1280.0, 680.0, (840.0, 318.0)), 0.9);
        let zoom = fitted_zoom(1.5, 600.0, 300.0, (840.0, 318.0));
        assert!(840.0 * zoom <= 568.0 && 318.0 * zoom <= 300.0);
    }
}
