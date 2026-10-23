//! InputEngine (PROJECT_OUTLINE §4): moves the real cursor, clicks and types with `SendInput`.
//!
//! Every event carries [`TAG`] in `dwExtraInfo`, so the InputGuard can tell Scenra's input from
//! the user's. Each step checks the emergency stop and the guard; on either, the engine releases
//! anything it holds and returns [`Abort`].

use crate::hooks;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

/// "SCNR" — marks Scenra's own input events.
pub const TAG: usize = 0x5343_4E52;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Abort {
    /// Ctrl+Alt+Esc.
    EmergencyStop,
    /// The user moved the mouse or pressed a key while Scenra was driving.
    ForeignInput,
}

impl std::fmt::Display for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Abort::EmergencyStop => "emergency stop (Ctrl+Alt+Esc)",
            Abort::ForeignInput => "you moved the mouse or typed, so Scenra stopped",
        })
    }
}

#[derive(Clone, Copy, serde::Deserialize, Default)]
pub enum Pace {
    Slow,
    #[default]
    Natural,
    Fast,
}

impl Pace {
    fn factor(self) -> f64 {
        match self {
            Pace::Slow => 1.6,
            Pace::Natural => 1.0,
            Pace::Fast => 0.6,
        }
    }
}

const STEP: Duration = Duration::from_millis(8);

fn check() -> Result<(), Abort> {
    if hooks::stopped() {
        return Err(Abort::EmergencyStop);
    }
    if hooks::foreign_seen() {
        // The guard sees the Esc of Ctrl+Alt+Esc a moment before the stop hotkey fires. Give the
        // stop a beat so the presenter is told the real reason.
        std::thread::sleep(Duration::from_millis(30));
        return Err(if hooks::stopped() { Abort::EmergencyStop } else { Abort::ForeignInput });
    }
    Ok(())
}

/// Sleeps in small slices so a stop lands within a few ms.
fn wait(duration: Duration) -> Result<(), Abort> {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        check()?;
        std::thread::sleep(STEP.min(end - Instant::now()));
    }
    check()
}

fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
}

fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: TAG } },
    }
}

fn key(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: TAG } },
    }
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

/// Absolute move in physical screen px, normalised over the whole virtual desktop.
fn move_abs(x: f64, y: f64) {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN) as f64,
            GetSystemMetrics(SM_YVIRTUALSCREEN) as f64,
            GetSystemMetrics(SM_CXVIRTUALSCREEN) as f64,
            GetSystemMetrics(SM_CYVIRTUALSCREEN) as f64,
        )
    };
    let nx = ((x - vx) * 65535.0 / (vw - 1.0)).round() as i32;
    let ny = ((y - vy) * 65535.0 / (vh - 1.0)).round() as i32;
    send(&[mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK, nx, ny)]);
}

/// Minimum-jerk easing: starts and ends at rest, like a hand.
fn ease(t: f64) -> f64 {
    t * t * t * (10.0 - 15.0 * t + 6.0 * t * t)
}

/// Movement time grows with the log of distance (Fitts), 250–1100 ms at natural pace.
fn duration_for(distance: f64, pace: Pace) -> Duration {
    let ms = (250.0 + 140.0 * (1.0 + distance / 12.0).log2()).min(1100.0) * pace.factor();
    Duration::from_millis(ms as u64)
}

/// A small pseudo-random number in [-1, 1] from the clock, so arcs vary between moves.
fn jitter() -> f64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    (nanos % 2001) as f64 / 1000.0 - 1.0
}

/// Moves the real cursor to `(x, y)` (physical px) along a gently curved, eased path.
pub fn move_to(x: f64, y: f64, pace: Pace) -> Result<(), Abort> {
    let _armed = hooks::arm();
    check()?;
    let (sx, sy) = cursor();
    let (sx, sy) = (sx as f64, sy as f64);
    let (dx, dy) = (x - sx, y - sy);
    let distance = dx.hypot(dy);
    if distance < 1.0 {
        return Ok(());
    }
    // Bow the path sideways by up to 6% of its length, like a wrist pivoting.
    let bow = 0.06 * distance * jitter();
    let (nx, ny) = (-dy / distance, dx / distance);

    let total = duration_for(distance, pace);
    let start = Instant::now();
    loop {
        let t = (start.elapsed().as_secs_f64() / total.as_secs_f64()).min(1.0);
        let s = ease(t);
        let arc = bow * (std::f64::consts::PI * s).sin();
        move_abs(sx + dx * s + nx * arc, sy + dy * s + ny * arc);
        if t >= 1.0 {
            break;
        }
        if let Err(abort) = wait(STEP) {
            release_all();
            return Err(abort);
        }
    }
    // Keep the final event tagged too: SetCursorPos looks like foreign input to the guard.
    move_abs(x, y);
    Ok(())
}

/// Left click where the cursor is, with a short human hold.
pub fn click() -> Result<(), Abort> {
    let _armed = hooks::arm();
    check()?;
    send(&[mouse(MOUSEEVENTF_LEFTDOWN, 0, 0)]);
    let held = wait(Duration::from_millis(75));
    send(&[mouse(MOUSEEVENTF_LEFTUP, 0, 0)]);
    held
}

/// Types `text` as Unicode key events at roughly `cps` characters per second.
pub fn type_text(text: &str, cps: f64) -> Result<(), Abort> {
    let _armed = hooks::arm();
    let gap = 1.0 / cps.max(1.0);
    for unit in text.encode_utf16() {
        check().inspect_err(|_| release_all())?;
        send(&[key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE)]);
        send(&[key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]);
        // ±35% per-key variation; a little longer after spaces.
        let mut secs = gap * (1.0 + 0.35 * jitter());
        if unit == u16::from(b' ') {
            secs *= 1.4;
        }
        if let Err(abort) = wait(Duration::from_secs_f64(secs)) {
            release_all();
            return Err(abort);
        }
    }
    Ok(())
}

/// Lets go of every button and modifier Scenra might be holding. Used by the emergency stop.
pub fn release_all() {
    let mut inputs = vec![mouse(MOUSEEVENTF_LEFTUP, 0, 0), mouse(MOUSEEVENTF_RIGHTUP, 0, 0)];
    for vk in [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN] {
        inputs.push(key(vk, 0, KEYEVENTF_KEYUP));
    }
    send(&inputs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_starts_and_ends_at_rest() {
        assert_eq!(ease(0.0), 0.0);
        assert!((ease(1.0) - 1.0).abs() < 1e-12);
        assert!((ease(0.5) - 0.5).abs() < 1e-12);
        // Velocity near the ends is close to zero.
        assert!(ease(0.02) < 0.001 && 1.0 - ease(0.98) < 0.001);
    }

    #[test]
    fn longer_moves_take_longer_but_stay_capped() {
        let short = duration_for(20.0, Pace::Natural);
        let long = duration_for(1500.0, Pace::Natural);
        assert!(short < long);
        assert!(long <= Duration::from_millis(1100));
        assert!(duration_for(1500.0, Pace::Slow) > long);
    }
}
