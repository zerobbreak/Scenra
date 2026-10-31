//! Phase 0 · Step 1, automated: the capture-exclusion matrix in docs/phase0/01-capture-exclusion.md,
//! run against Scenra's built-in recorder.
//!
//! For each row Scenra opens the pill over the demo target (excluded or visible), records a take
//! while clicking New Recording → Settings → Confirm, and grabs the recorder's own frames at each
//! step. Each frame is checked for the pill's magenta marker inside the pill's rect, and saved as a
//! PNG next to the take so a person can confirm the cursor, ripple and dialog are there.
//!
//! The recorder captures one monitor at its physical size, so the pill's screen rect maps onto the
//! frame by subtracting the monitor's origin.

use crate::capture::{self, TELEPROMPTER};
use crate::engine;
use crate::input::{self, Pace};
use crate::locate::{self, Selector};
use crate::page;
use crate::recorder::Snapshot;
use crate::take;
use crate::target::{self, LaunchedTarget};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, State};

/// Gap between the pill and the button below it, physical px.
const PILL_GAP: i32 = 16;
/// WGC only delivers a frame when the screen changes; the shot nudges the cursor to make one.
const SHOT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
enum Pill {
    Visible,
    Excluded,
    /// Excluded at first, made visible mid-recording.
    Toggled,
}

struct Row {
    name: &'static str,
    pill: Pill,
}

const ROWS: [Row; 3] = [
    Row { name: "wgc-visible", pill: Pill::Visible },
    Row { name: "wgc-excluded", pill: Pill::Excluded },
    Row { name: "wgc-toggled", pill: Pill::Toggled },
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    label: String,
    /// Whether the pill should show up in this frame.
    expect_pill: bool,
    marker_pixels: usize,
    pill_pixels: usize,
    pill_seen: bool,
    png: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowResult {
    name: &'static str,
    method: &'static str,
    scale: f64,
    take: String,
    shots: Vec<Shot>,
    passed: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command(async)]
pub fn run_capture_matrix(app: AppHandle, monitor_index: Option<usize>) -> Result<Vec<RowResult>, String> {
    if app.state::<take::CurrentTake>().is_recording() {
        return Err("Stop the current recording before running display validation".into());
    }
    let outcome = (|| -> Result<Vec<RowResult>, String> {
        let _placement = if let Some(index) = monitor_index {
            let monitors = app.available_monitors().map_err(err)?;
            let monitor = monitors.get(index).ok_or("Selected monitor is no longer available")?;
            let (pid, _) = target::running(&app.state::<LaunchedTarget>())?;
            let area = monitor.work_area();
            Some(locate::place_on_monitor(pid, (area.position.x, area.position.y, area.size.width, area.size.height))?)
        } else { None };
        ROWS.iter().map(|row| run_row(&app, row)).collect()
    })();
    let _ = capture::close_teleprompter(app.clone());
    let _ = take::stop(&app.state());
    outcome
}

fn run_row(app: &AppHandle, row: &Row) -> Result<RowResult, String> {
    let targets: State<'_, LaunchedTarget> = app.state();
    let excluded = !matches!(row.pill, Pill::Visible);
    capture::open_teleprompter(app.clone(), excluded)?;
    let pill = app.get_webview_window(TELEPROMPTER).ok_or("the teleprompter didn't open")?;
    place_over_target(&pill, &targets)?;
    let scale = pill.scale_factor().map_err(err)?;
    let monitor = pill.current_monitor().map_err(err)?.ok_or("the pill isn't on a monitor")?;
    let device = monitor.name().ok_or("Monitor name unavailable")?.clone();
    let origin = (monitor.position().x, monitor.position().y);
    std::thread::sleep(Duration::from_millis(800));

    // Record the monitor the pill and target are on, whatever Setup › Display says.
    let take_dir = take::start_take_on(&app.state(), &targets, &crate::session::root(app), None, Some(&device))?;
    let dir = Path::new(&take_dir);
    let mut shots = Vec::new();
    let mut shoot = |label: &str, expect_pill: bool| -> Result<(), String> {
        shots.push(shot(app, &pill, origin, dir, label, expect_pill)?);
        Ok(())
    };

    shoot("1-pill-over-target", !excluded)?;
    if let Pill::Toggled = row.pill {
        capture::set_capture_excluded(app.clone(), false)?;
        std::thread::sleep(Duration::from_millis(500));
        shoot("2-toggled-visible", true)?;
    }
    let visible_now = !matches!(row.pill, Pill::Excluded);
    engine::click(&targets, &Selector::TestId("create-recording".into()), Pace::Natural)?;
    std::thread::sleep(Duration::from_millis(400));
    engine::click(&targets, &Selector::TestId("open-settings".into()), Pace::Natural)?;
    std::thread::sleep(Duration::from_millis(500));
    shoot("3-settings-dialog", visible_now)?;
    engine::click(&targets, &Selector::TestId("settings-confirm".into()), Pace::Natural)?;
    std::thread::sleep(Duration::from_millis(400));
    // Cross the pill with the cursor: it must be captured even where the pill is hidden.
    let position = pill.outer_position().map_err(err)?;
    let size = pill.outer_size().map_err(err)?;
    input::move_to(
        f64::from(position.x) + f64::from(size.width) * 0.6,
        f64::from(position.y) + f64::from(size.height) / 2.0,
        Pace::Natural,
    )
    .map_err(err)?;
    std::thread::sleep(Duration::from_millis(300));
    shoot("4-cursor-over-pill", visible_now)?;

    take::stop(&app.state())?;
    close_overlays(app)?;

    let passed = shots.iter().all(|s| s.pill_seen == s.expect_pill);
    Ok(RowResult { name: row.name, method: "WGC", scale, take: take_dir, shots, passed })
}

/// Closes the pill and bar and waits until they are gone: `destroy` is asynchronous, and opening
/// the next row's pill before then would reuse the dying window.
fn close_overlays(app: &AppHandle) -> Result<(), String> {
    capture::close_teleprompter(app.clone())?;
    for _ in 0..50 {
        if app.get_webview_window(TELEPROMPTER).is_none() && app.get_webview_window(capture::CONTROLS).is_none() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("the teleprompter didn't close".into())
}

/// Puts the pill just above the demo target's New Recording button, so it covers the target's
/// heading but none of the controls the run clicks.
fn place_over_target(pill: &tauri::WebviewWindow, targets: &LaunchedTarget) -> Result<(), String> {
    let (pid, port) = target::running(targets)?;
    let located = locate::locate(&mut page::page_session(port)?, pid, &Selector::TestId("create-recording".into()))?;
    let size = pill.outer_size().map_err(err)?;
    let button = located.screen;
    let (cx, cy) = button.center();
    let monitor = pill.app_handle().available_monitors().map_err(err)?.into_iter().find(|m| {
        cx >= f64::from(m.position().x) && cy >= f64::from(m.position().y)
            && cx < f64::from(m.position().x) + f64::from(m.size().width)
            && cy < f64::from(m.position().y) + f64::from(m.size().height)
    }).ok_or("Target control is not on an available monitor")?;
    let area = monitor.work_area();
    let x = (button.x as i32 + button.width as i32 / 2 - size.width as i32 / 3)
        .clamp(area.position.x, (area.position.x + area.size.width as i32 - size.width as i32).max(area.position.x));
    let y = (button.y as i32 - size.height as i32 - PILL_GAP).max(area.position.y);
    pill.set_position(PhysicalPosition::new(x, y)).map_err(err)
}

/// A screen rect (physical px) → the frame's pixel range, clamped to the frame.
/// Returns (x0, x1, y0, y1).
fn to_frame(rect: (i32, i32, u32, u32), origin: (i32, i32), frame: (u32, u32)) -> (u32, u32, u32, u32) {
    let clamp = |v: i64, max: u32| v.clamp(0, i64::from(max)) as u32;
    let (left, top) = (i64::from(rect.0 - origin.0), i64::from(rect.1 - origin.1));
    (
        clamp(left, frame.0),
        clamp(left + i64::from(rect.2), frame.0),
        clamp(top, frame.1),
        clamp(top + i64::from(rect.3), frame.1),
    )
}

fn count_marker(frame: &Snapshot, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> usize {
    let mut marker = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (r, g, b) = frame.pixel(x, y);
            marker += usize::from(capture::is_marker(r, g, b));
        }
    }
    marker
}

/// Grabs the recorder's next frame, saves it as PNG, and counts marker pixels inside the pill's rect.
fn shot(
    app: &AppHandle,
    pill: &tauri::WebviewWindow,
    origin: (i32, i32),
    dir: &Path,
    label: &str,
    expect_pill: bool,
) -> Result<Shot, String> {
    // Nudge the cursor so Windows has a changed screen to deliver.
    let nudge = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(100));
        let (x, y) = input::cursor();
        let _ = input::move_to(f64::from(x) + 3.0, f64::from(y), Pace::Fast);
        let _ = input::move_to(f64::from(x), f64::from(y), Pace::Fast);
    });
    let grabbed = take::snapshot(&app.state(), SHOT_TIMEOUT);
    let _ = nudge.join();
    let (frame, _) = grabbed?;
    let png_path = dir.join(format!("{label}.png"));
    std::fs::write(&png_path, frame.png()?).map_err(err)?;

    let position = pill.outer_position().map_err(err)?;
    let size = pill.outer_size().map_err(err)?;
    let range = to_frame((position.x, position.y, size.width, size.height), origin, (frame.width, frame.height));
    let pill_pixels = ((range.1 - range.0) * (range.3 - range.2)) as usize;
    if pill_pixels == 0 {
        return Err("the pill is outside the recorded monitor".into());
    }
    let marker_pixels = count_marker(&frame, range);
    Ok(Shot {
        label: label.into(),
        expect_pill,
        marker_pixels,
        pill_pixels,
        // Same threshold as the GDI probe: the strip is ~3.5 % of the pill.
        pill_seen: marker_pixels * 100 > pill_pixels,
        png: png_path.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pill_rect_maps_onto_a_secondary_monitor_frame() {
        // Monitor at x = -1920 (left of primary); pill at screen (-1000, 100).
        assert_eq!(to_frame((-1000, 100, 840, 112), (-1920, 0), (1920, 1080)), (920, 1760, 100, 212));
    }

    #[test]
    fn pill_rect_is_clamped_to_the_frame() {
        assert_eq!(to_frame((1500, -20, 840, 112), (0, 0), (1920, 1080)), (1500, 1920, 0, 92));
        assert_eq!(to_frame((3000, 0, 100, 100), (0, 0), (1920, 1080)), (1920, 1920, 0, 100));
    }

    #[test]
    fn counts_magenta_inside_the_rect_only() {
        // 3×1: magenta, magenta, grey (BGRA).
        let frame = Snapshot { width: 3, height: 1, bgra: vec![255, 0, 255, 255, 255, 0, 255, 255, 80, 80, 80, 255] };
        assert_eq!(count_marker(&frame, (0, 3, 0, 1)), 2);
        assert_eq!(count_marker(&frame, (1, 3, 0, 1)), 1);
    }
}
