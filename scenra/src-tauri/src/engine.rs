//! Commands that drive the target with the real cursor: locate → move → hover-verify → click,
//! plus the Phase 0 test hooks for the emergency stop and InputGuard.

use crate::cdp::Session;
use crate::hooks::{self, GuardCounts};
use crate::input::{self, Pace};
use crate::locate::{self, Probe, ProbeError, Selector};
use crate::page;
use crate::ripple;
use crate::target::{self, LaunchedTarget};
use std::time::Duration;
use tauri::State;

fn session(targets: &LaunchedTarget) -> Result<(u32, Session), String> {
    let (pid, port) = target::running(targets)?;
    Ok((pid, page::page_session(port)?))
}

/// Moves to the element and checks the page saw the cursor there. Re-measures once on a miss
/// (the page may have scrolled or re-laid out), then gives up rather than click blind.
fn verified(session: &mut Session, pid: u32, selector: &Selector, pace: Pace) -> Result<Probe, String> {
    let first = locate::probe(session, pid, selector, pace).map_err(|e| e.to_string())?;
    if first.passed {
        return Ok(first);
    }
    let second = locate::probe(session, pid, selector, Pace::Fast).map_err(|e| e.to_string())?;
    if second.passed {
        return Ok(second);
    }
    Err(match second.error_css {
        Some(e) => format!("hover probe missed {selector} by {e:.1} CSS px (max {})", locate::MAX_PROBE_ERROR_CSS),
        None => format!("the page never saw the cursor over {selector}: is something covering it?"),
    })
}

/// Clicks with the real cursor and flashes the ripple. Shared by `click_element` and scripts.
pub fn click(targets: &LaunchedTarget, selector: &Selector, pace: Pace) -> Result<Probe, String> {
    let (pid, mut session) = session(targets)?;
    let probe = verified(&mut session, pid, selector, pace)?;
    input::click().map_err(|e| e.to_string())?;
    let (x, y) = probe.located.screen.center();
    ripple::show_at(x, y)?;
    Ok(probe)
}

#[tauri::command(async)]
pub fn probe_element(targets: State<'_, LaunchedTarget>, selector: Selector) -> Result<Probe, String> {
    let (pid, mut session) = session(&targets)?;
    locate::probe(&mut session, pid, &selector, Pace::Natural).map_err(|e: ProbeError| e.to_string())
}

#[tauri::command(async)]
pub fn click_element(targets: State<'_, LaunchedTarget>, selector: Selector) -> Result<Probe, String> {
    click(&targets, &selector, Pace::Natural)
}

/// Clicks the field, then types into it like a person.
#[tauri::command(async)]
pub fn type_into(
    targets: State<'_, LaunchedTarget>,
    selector: Selector,
    text: String,
) -> Result<(), String> {
    click(&targets, &selector, Pace::Natural)?;
    std::thread::sleep(Duration::from_millis(250));
    input::type_text(&text, 11.0).map_err(|e| e.to_string())
}

/// A slow ~8 s loop around the screen centre, for testing the emergency stop and InputGuard
/// mid-motion.
#[tauri::command(async)]
pub fn slow_move_test() -> Result<(), String> {
    let (cx, cy) = input::cursor();
    let (cx, cy) = (cx as f64, cy as f64);
    for i in 1..=8 {
        let angle = i as f64 * std::f64::consts::FRAC_PI_4;
        input::move_to(cx + 260.0 * angle.cos(), cy + 160.0 * angle.sin(), Pace::Slow).map_err(|e| e.to_string())?;
    }
    input::move_to(cx, cy, Pace::Slow).map_err(|e| e.to_string())
}

/// A slow typing run into the focused field, for testing the stop mid-typing.
#[tauri::command(async)]
pub fn slow_type_test(text: String) -> Result<(), String> {
    input::type_text(&text, 4.0).map_err(|e| e.to_string())
}

/// Freezes the UI thread: a sync command runs on the main thread. The emergency stop must still
/// work, because its hook has its own thread and message loop.
#[tauri::command]
pub fn simulate_hang(seconds: u64) {
    std::thread::sleep(Duration::from_secs(seconds.min(30)));
}

#[tauri::command]
pub fn guard_status() -> GuardCounts {
    hooks::counts()
}

/// A key press in one of Scenra's own windows (see `hooks::own_window_key`). Async, so a busy
/// main thread doesn't delay it.
#[tauri::command(async)]
pub fn guard_key() {
    hooks::own_window_key();
}

#[tauri::command]
pub fn clear_stop() {
    hooks::clear_stop();
}
