//! Click ripple overlay (PROJECT_OUTLINE §4, Overlay windows): a ring that grows and fades where
//! Scenra clicks. Unlike the teleprompter it is NOT excluded from capture: the ripple is meant to
//! be in the video.
//!
//! It is a native layered window drawn with per-pixel alpha (`UpdateLayeredWindow`), not a
//! webview: a transparent WebView2 inside a click-through layered window draws nothing. The
//! window is click-through (`WS_EX_TRANSPARENT`) and never activates, so it can't swallow the
//! click or take the keyboard from the target. It runs on its own thread with its own message
//! loop, like the input hooks.

use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    MONITOR_DEFAULTTONEAREST, MonitorFromPoint, ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostThreadMessageW,
    RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
    WM_APP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

const WM_RIPPLE: u32 = WM_APP + 1;
/// Logical px of the square the ring animates in.
const BOX: f64 = 120.0;
const DURATION: Duration = Duration::from_millis(600);
const RING: (u8, u8, u8) = (0x63, 0x66, 0xF1);
const FILL: (u8, u8, u8) = (0x4F, 0x46, 0xE5);

static THREAD: OnceLock<u32> = OnceLock::new();

/// Starts the overlay thread. Safe to call more than once.
pub fn start() -> Result<(), String> {
    if THREAD.get().is_some() {
        return Ok(());
    }
    let (ready, started) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("scenra-ripple".into())
        .spawn(move || run(ready))
        .map_err(|e| e.to_string())?;
    let id = started.recv().map_err(|e| e.to_string())??;
    let _ = THREAD.set(id);
    Ok(())
}

/// Flashes the ring centred on `(x, y)`, physical screen px. Returns immediately.
pub fn show_at(x: f64, y: f64) -> Result<(), String> {
    let id = *THREAD.get().ok_or("ripple overlay not started")?;
    let (x, y) = (x.round() as i32, y.round() as i32);
    unsafe { PostThreadMessageW(id, WM_RIPPLE, WPARAM(x as u32 as usize), LPARAM(y as isize)) }
        .map_err(|e| e.to_string())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn run(ready: std::sync::mpsc::Sender<Result<u32, String>>) {
    let created = unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("ScenraRipple"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("ScenraRipple"),
            w!("Scenra Ripple"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )
    };
    let hwnd = match created {
        Ok(hwnd) => hwnd,
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        if msg.message == WM_RIPPLE {
            animate(hwnd, msg.wParam.0 as u32 as i32, msg.lParam.0 as i32);
        } else {
            unsafe { DispatchMessageW(&msg) };
        }
    }
}

/// Monitor scale at a point (1.0 = 96 dpi).
fn scale_at(x: i32, y: i32) -> f64 {
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
    dx as f64 / 96.0
}

fn animate(hwnd: HWND, cx: i32, cy: i32) {
    let scale = scale_at(cx, cy);
    let size = (BOX * scale).round() as i32;
    let origin = POINT { x: cx - size / 2, y: cy - size / 2 };
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            return;
        };
        let previous = SelectObject(mem, bitmap.into());
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
            ..Default::default()
        };
        let extent = SIZE { cx: size, cy: size };
        let source = POINT::default();

        let start = Instant::now();
        let mut shown = false;
        loop {
            let t = (start.elapsed().as_secs_f64() / DURATION.as_secs_f64()).min(1.0);
            draw(pixels, size, scale, t);
            let _ = UpdateLayeredWindow(
                hwnd,
                Some(screen),
                Some(&origin),
                Some(&extent),
                Some(mem),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            if !shown {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                shown = true;
            }
            if t >= 1.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(14));
        }
        let _ = ShowWindow(hwnd, SW_HIDE);
        SelectObject(mem, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

/// One frame: a faint disc with a ring at its edge, growing and fading. Premultiplied BGRA.
fn draw(pixels: &mut [u32], size: i32, scale: f64, t: f64) {
    let grow = 1.0 - (1.0 - t).powi(3); // ease-out
    let radius = (14.0 + 40.0 * grow) * scale;
    let ring = 3.0 * scale;
    let fade = 1.0 - t;
    let c = size as f64 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = (x as f64 + 0.5 - c).hypot(y as f64 + 0.5 - c);
            // Anti-aliased coverage of the ring band and of the disc inside it.
            let ring_cov = (ring / 2.0 + 0.5 - (d - radius).abs()).clamp(0.0, 1.0);
            let disc_cov = (radius - d + 0.5).clamp(0.0, 1.0);
            let ring_a = ring_cov * fade;
            let fill_a = disc_cov * 0.22 * fade * (1.0 - ring_cov);
            let a = ring_a + fill_a;
            pixels[(y * size + x) as usize] = if a <= 0.0 {
                0
            } else {
                let mix = |r: u8, f: u8| (r as f64 * ring_a + f as f64 * fill_a).round().min(255.0) as u32;
                let alpha = (a * 255.0).round().min(255.0) as u32;
                (alpha << 24) | (mix(RING.0, FILL.0) << 16) | (mix(RING.1, FILL.1) << 8) | mix(RING.2, FILL.2)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_opaque_at_its_radius_and_clear_far_outside() {
        let size = 120;
        let mut pixels = vec![0u32; (size * size) as usize];
        draw(&mut pixels, size, 1.0, 0.0);
        // t = 0: radius 14 px, so (60 + 14, 60) is on the ring and the corner is empty.
        assert!(pixels[(60 * size + 74) as usize] >> 24 > 200);
        assert_eq!(pixels[0], 0);
        // Fully faded at the end.
        draw(&mut pixels, size, 1.0, 1.0);
        assert!(pixels.iter().all(|&p| p == 0));
    }
}
