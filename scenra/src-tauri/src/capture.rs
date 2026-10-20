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
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
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
const MIN_BUILD: u32 = 19041;

/// Marker colour drawn by the teleprompter (#FF00FF). Keep in sync with `Teleprompter.tsx`.
const MARKER_RGB: (u8, u8, u8) = (0xFF, 0x00, 0xFF);
const MARKER_TOLERANCE: u8 = 12;

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

#[tauri::command]
pub fn open_teleprompter(app: AppHandle, excluded: bool) -> Result<CaptureStatus, String> {
    let window = match app.get_webview_window(TELEPROMPTER) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(&app, TELEPROMPTER, WebviewUrl::default())
            .title("Scenra Teleprompter")
            .inner_size(560.0, 380.0)
            .always_on_top(true)
            .build()
            .map_err(err)?,
    };
    set_excluded(&window, excluded)?;
    window.set_focus().map_err(err)?;
    status(&app)
}

#[tauri::command]
pub fn close_teleprompter(app: AppHandle) -> Result<CaptureStatus, String> {
    if let Some(window) = app.get_webview_window(TELEPROMPTER) {
        window.destroy().map_err(err)?;
    }
    Ok(CaptureStatus { open: false, affinity: None, excluded: false, ..status(&app)? })
}

#[tauri::command]
pub fn set_capture_excluded(app: AppHandle, excluded: bool) -> Result<CaptureStatus, String> {
    let window = app.get_webview_window(TELEPROMPTER).ok_or("teleprompter is not open")?;
    set_excluded(&window, excluded)?;
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
    let near = |a: u8, b: u8| a.abs_diff(b) <= MARKER_TOLERANCE;
    let marker_pixels = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| near(p[2], MARKER_RGB.0) && near(p[1], MARKER_RGB.1) && near(p[0], MARKER_RGB.2))
        .count();
    let total_pixels = (w * h) as usize;

    Ok(ProbeResult {
        rect: [rect.left, rect.top, w, h],
        marker_pixels,
        total_pixels,
        // The band covers ~10% of the window; 1% avoids false hits from stray magenta.
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
