//! Phase 0 · Step 1, automated: the capture-exclusion matrix in docs/phase0/01-capture-exclusion.md.
//!
//! For each row Scenra switches OBS's Display Capture method, opens the pill over the demo target
//! (excluded or visible), records a take while clicking New Recording → Settings → Confirm, and
//! grabs OBS's program output at each step. Each grab is checked for the pill's magenta marker
//! inside the pill's rect, and saved as a PNG next to the take so a person can confirm the cursor,
//! ripple and dialog are there.
//!
//! It runs at whatever display scaling Windows is set to, and with any OBS canvas: the pill's
//! screen rect is mapped onto the canvas through the Display Capture source's transform.

use crate::capture::{self, TELEPROMPTER};
use crate::engine;
use crate::input::Pace;
use crate::locate::{self, Selector};
use crate::obs::Obs;
use crate::page;
use crate::take::{self, CurrentTake};
use crate::target::{self, LaunchedTarget};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, State};

const DISPLAY_CAPTURE: &str = "monitor_capture";
/// OBS's `method` values for Display Capture.
const DXGI: u64 = 1;
const WGC: u64 = 2;
/// OBS restarts the capture when the method changes; give it time to deliver frames again.
const METHOD_SETTLE: Duration = Duration::from_millis(2500);
/// Gap between the pill and the button below it, physical px.
const PILL_GAP: i32 = 16;

#[derive(Clone, Copy)]
enum Pill {
    Visible,
    Excluded,
    /// Excluded at first, made visible mid-recording.
    Toggled,
}

struct Row {
    name: &'static str,
    method: u64,
    pill: Pill,
}

const ROWS: [Row; 4] = [
    Row { name: "wgc-visible", method: WGC, pill: Pill::Visible },
    Row { name: "wgc-excluded", method: WGC, pill: Pill::Excluded },
    Row { name: "dxgi-excluded", method: DXGI, pill: Pill::Excluded },
    Row { name: "wgc-toggled", method: WGC, pill: Pill::Toggled },
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
pub fn run_capture_matrix(app: AppHandle) -> Result<Vec<RowResult>, String> {
    let mut obs = Obs::connect()?;
    let capture = display_capture(&mut obs)?;
    let source = capture.name.clone();
    let original = obs.request("GetInputSettings", json!({ "inputName": source }))?["inputSettings"]["method"].clone();

    let mut results = Vec::new();
    let outcome = ROWS.iter().try_for_each(|row| {
        results.push(run_row(&app, &mut obs, &capture, row)?);
        Ok::<_, String>(())
    });

    // Leave things as they were, even after a failed row.
    let _ = capture::close_teleprompter(app.clone());
    let take: State<'_, CurrentTake> = app.state();
    let _ = take::stop_take(take, None);
    let _ = obs.request("SetInputSettings", json!({ "inputName": source, "inputSettings": { "method": original } }));
    outcome.map(|()| results)
}

/// The Display Capture source in the program scene, and where it sits on the OBS canvas.
struct DisplayCapture {
    name: String,
    /// Canvas px of the captured monitor's top-left corner (after crop).
    position: (f64, f64),
    scale: (f64, f64),
    crop: (f64, f64),
    /// The captured monitor's size in physical px.
    source_size: (f64, f64),
}

impl DisplayCapture {
    /// Physical px on the captured monitor (relative to its top-left) → canvas px.
    fn to_canvas(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.position.0 + (x - self.crop.0) * self.scale.0,
            self.position.1 + (y - self.crop.1) * self.scale.1,
        )
    }
}

/// OBS's alignment value for "top left", the default for a new source.
const ALIGN_TOP_LEFT: u64 = 5;

fn display_capture(obs: &mut Obs) -> Result<DisplayCapture, String> {
    let scene = obs.request("GetCurrentProgramScene", json!({}))?["currentProgramSceneName"].clone();
    let items = obs.request("GetSceneItemList", json!({ "sceneName": scene }))?;
    let item = items["sceneItems"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["inputKind"] == DISPLAY_CAPTURE)
        .ok_or("the OBS program scene has no Display Capture source")?;
    let name = item["sourceName"].as_str().unwrap_or_default().to_owned();
    let t = obs.request("GetSceneItemTransform", json!({ "sceneName": scene, "sceneItemId": item["sceneItemId"] }))?;
    let t = &t["sceneItemTransform"];
    let f = |key: &str| t[key].as_f64().unwrap_or(0.0);
    if t["alignment"].as_u64() != Some(ALIGN_TOP_LEFT) || f("rotation") != 0.0 || t["boundsType"] != "OBS_BOUNDS_NONE" {
        return Err(format!(
            "set “{name}” in OBS to top-left alignment, no rotation and no bounding box \
             (right-click › Transform › Reset Transform), so Scenra can find the pill in the frame"
        ));
    }
    Ok(DisplayCapture {
        name,
        position: (f("positionX"), f("positionY")),
        scale: (f("scaleX"), f("scaleY")),
        crop: (f("cropLeft"), f("cropTop")),
        source_size: (f("sourceWidth"), f("sourceHeight")),
    })
}

fn run_row(app: &AppHandle, obs: &mut Obs, capture: &DisplayCapture, row: &Row) -> Result<RowResult, String> {
    let targets: State<'_, LaunchedTarget> = app.state();
    obs.request("SetInputSettings", json!({ "inputName": capture.name, "inputSettings": { "method": row.method } }))?;
    std::thread::sleep(METHOD_SETTLE);

    let excluded = !matches!(row.pill, Pill::Visible);
    capture::open_teleprompter(app.clone(), excluded)?;
    let pill = app.get_webview_window(TELEPROMPTER).ok_or("the teleprompter didn't open")?;
    place_over_target(&pill, &targets)?;
    let scale = pill.scale_factor().map_err(err)?;
    let monitor = pill.current_monitor().map_err(err)?.ok_or("the pill isn't on a monitor")?;
    let size = monitor.size();
    if (f64::from(size.width), f64::from(size.height)) != capture.source_size {
        return Err(format!(
            "OBS's “{}” captures a {}×{} display, but the pill and target are on a {}×{} one: \
             point Display Capture at the monitor the target is on",
            capture.name, capture.source_size.0, capture.source_size.1, size.width, size.height
        ));
    }
    let monitor_origin = (f64::from(monitor.position().x), f64::from(monitor.position().y));
    std::thread::sleep(Duration::from_millis(800));

    let take_dir = take::start_take(app.state(), app.state(), None)?;
    let dir = Path::new(&take_dir);
    let mut shots = Vec::new();
    let mut shoot = |obs: &mut Obs, label: &str, expect_pill: bool| -> Result<(), String> {
        shots.push(shot(obs, &pill, capture, monitor_origin, dir, label, expect_pill)?);
        Ok(())
    };

    shoot(obs, "1-pill-over-target", !excluded)?;
    if let Pill::Toggled = row.pill {
        capture::set_capture_excluded(app.clone(), false)?;
        std::thread::sleep(Duration::from_millis(500));
        shoot(obs, "2-toggled-visible", true)?;
    }
    let visible_now = !matches!(row.pill, Pill::Excluded);
    engine::click(&targets, &Selector::TestId("create-recording".into()), Pace::Natural)?;
    std::thread::sleep(Duration::from_millis(400));
    engine::click(&targets, &Selector::TestId("open-settings".into()), Pace::Natural)?;
    std::thread::sleep(Duration::from_millis(500));
    shoot(obs, "3-settings-dialog", visible_now)?;
    engine::click(&targets, &Selector::TestId("settings-confirm".into()), Pace::Natural)?;
    std::thread::sleep(Duration::from_millis(400));
    // Cross the pill with the cursor: it must be captured even where the pill is hidden.
    let position = pill.outer_position().map_err(err)?;
    let size = pill.outer_size().map_err(err)?;
    crate::input::move_to(
        f64::from(position.x) + f64::from(size.width) * 0.6,
        f64::from(position.y) + f64::from(size.height) / 2.0,
        Pace::Natural,
    )
    .map_err(err)?;
    std::thread::sleep(Duration::from_millis(300));
    shoot(obs, "4-cursor-over-pill", visible_now)?;

    take::stop_take(app.state(), None)?;
    close_overlays(app)?;

    let passed = shots.iter().all(|s| s.pill_seen == s.expect_pill);
    Ok(RowResult {
        name: row.name,
        method: if row.method == DXGI { "DXGI" } else { "WGC" },
        scale,
        take: take_dir,
        shots,
        passed,
    })
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
    let x = (button.x as i32 + button.width as i32 / 2 - size.width as i32 / 3).max(0);
    let y = (button.y as i32 - size.height as i32 - PILL_GAP).max(0);
    pill.set_position(PhysicalPosition::new(x, y)).map_err(err)
}

/// Grabs OBS's program output, saves it as PNG, and counts marker pixels inside the pill's rect,
/// mapped from the screen onto the canvas.
fn shot(
    obs: &mut Obs,
    pill: &tauri::WebviewWindow,
    capture: &DisplayCapture,
    monitor_origin: (f64, f64),
    dir: &Path,
    label: &str,
    expect_pill: bool,
) -> Result<Shot, String> {
    let scene = obs.request("GetCurrentProgramScene", json!({}))?["currentProgramSceneName"].clone();
    let png = screenshot(obs, &scene, "png")?;
    let png_path = dir.join(format!("{label}.png"));
    std::fs::write(&png_path, png).map_err(err)?;

    let raw = screenshot(obs, &scene, "bmp")?;
    let bmp = Bmp::parse(&raw)?;
    let position = pill.outer_position().map_err(err)?;
    let size = pill.outer_size().map_err(err)?;
    let left = f64::from(position.x) - monitor_origin.0;
    let top = f64::from(position.y) - monitor_origin.1;
    let (x0, y0) = capture.to_canvas(left, top);
    let (x1, y1) = capture.to_canvas(left + f64::from(size.width), top + f64::from(size.height));
    let clamp_x = |v: f64| (v.round().max(0.0) as usize).min(bmp.width);
    let clamp_y = |v: f64| (v.round().max(0.0) as usize).min(bmp.height);
    let (x0, x1, y0, y1) = (clamp_x(x0), clamp_x(x1), clamp_y(y0), clamp_y(y1));
    let mut marker_pixels = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (r, g, b) = bmp.pixel(x, y);
            marker_pixels += usize::from(capture::is_marker(r, g, b));
        }
    }
    let pill_pixels = (x1 - x0) * (y1 - y0);
    if pill_pixels == 0 {
        return Err("the pill is outside OBS's canvas: is Display Capture cropped or moved off-screen?".into());
    }
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

fn screenshot(obs: &mut Obs, scene: &Value, format: &str) -> Result<Vec<u8>, String> {
    let reply = obs.request("GetSourceScreenshot", json!({ "sourceName": scene, "imageFormat": format }))?;
    let data = reply["imageData"].as_str().ok_or("OBS returned no screenshot")?;
    let base64 = data.split_once(',').map_or(data, |(_, b)| b);
    B64.decode(base64).map_err(err)
}

/// An uncompressed 24- or 32-bit BMP, as OBS (Qt) writes it.
struct Bmp<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    top_down: bool,
    bytes_per_pixel: usize,
    stride: usize,
}

impl<'a> Bmp<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
        if data.get(..2) != Some(b"BM") {
            return Err("OBS screenshot isn't a BMP".into());
        }
        let offset = u32_at(10).ok_or("truncated BMP")? as usize;
        let width = u32_at(18).ok_or("truncated BMP")? as i32;
        let height = u32_at(22).ok_or("truncated BMP")? as i32;
        let bits = data.get(28).copied().ok_or("truncated BMP")?;
        if bits != 24 && bits != 32 {
            return Err(format!("unsupported BMP depth: {bits} bits"));
        }
        let bytes_per_pixel = usize::from(bits / 8);
        let width = width.unsigned_abs() as usize;
        let stride = (width * bytes_per_pixel).div_ceil(4) * 4;
        let bmp = Self {
            data: data.get(offset..).ok_or("truncated BMP")?,
            width,
            height: height.unsigned_abs() as usize,
            top_down: height < 0,
            bytes_per_pixel,
            stride,
        };
        if bmp.data.len() < bmp.stride * bmp.height {
            return Err("truncated BMP".into());
        }
        Ok(bmp)
    }

    fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let row = if self.top_down { y } else { self.height - 1 - y };
        let i = row * self.stride + x * self.bytes_per_pixel;
        (self.data[i + 2], self.data[i + 1], self.data[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(position: (f64, f64), scale: (f64, f64), crop: (f64, f64)) -> DisplayCapture {
        DisplayCapture { name: "Display Capture".into(), position, scale, crop, source_size: (3840.0, 2160.0) }
    }

    #[test]
    fn screen_px_map_onto_a_downscaled_canvas() {
        // A 4K monitor shown at half size on a 1080p canvas.
        let c = capture((0.0, 0.0), (0.5, 0.5), (0.0, 0.0));
        assert_eq!(c.to_canvas(1000.0, 400.0), (500.0, 200.0));
    }

    #[test]
    fn screen_px_map_through_position_and_crop() {
        let c = capture((100.0, 50.0), (1.0, 1.0), (20.0, 10.0));
        assert_eq!(c.to_canvas(20.0, 10.0), (100.0, 50.0));
        assert_eq!(c.to_canvas(220.0, 110.0), (300.0, 150.0));
    }

    #[test]
    fn bmp_reads_bottom_up_24_bit_rows() {
        // 2×2, bottom-up: the first stored row is the bottom one. Rows pad to 8 bytes.
        let mut data = vec![0u8; 54];
        data[..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[18..22].copy_from_slice(&2i32.to_le_bytes());
        data[22..26].copy_from_slice(&2i32.to_le_bytes());
        data[28] = 24;
        data.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]); // bottom: red, green
        data.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 0]); // top: blue, magenta
        let bmp = Bmp::parse(&data).unwrap();
        assert_eq!(bmp.pixel(0, 0), (0, 0, 255));
        assert_eq!(bmp.pixel(1, 0), (255, 0, 255));
        assert_eq!(bmp.pixel(0, 1), (255, 0, 0));
        assert_eq!(bmp.pixel(1, 1), (0, 255, 0));
    }
}
