//! Display setup: what Scenra sees of the presenter's monitors, and the overlay size they chose.
//! Scenra works in physical px and reads each monitor's scaling from Windows, so any display
//! scaling is supported; this panel shows what was detected and lets the presenter size the pill.

use crate::capture;
use crate::settings::{self, MAX_ZOOM, MIN_ZOOM};
use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    name: Option<String>,
    /// Physical px: x, y, width, height.
    rect: [i32; 4],
    /// Windows display scaling, e.g. 1.5 for 150 %.
    scale: f64,
    primary: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    monitors: Vec<MonitorInfo>,
    overlay_zoom: f64,
    min_zoom: f64,
    max_zoom: f64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub fn display_info(app: AppHandle) -> Result<DisplayInfo, String> {
    let primary = app.primary_monitor().map_err(err)?.map(|m| *m.position());
    let monitors = app
        .available_monitors()
        .map_err(err)?
        .into_iter()
        .map(|m| MonitorInfo {
            name: m.name().cloned(),
            rect: [m.position().x, m.position().y, m.size().width as i32, m.size().height as i32],
            scale: m.scale_factor(),
            primary: Some(*m.position()) == primary,
        })
        .collect();
    Ok(DisplayInfo {
        monitors,
        overlay_zoom: settings::load(&app).overlay_zoom,
        min_zoom: MIN_ZOOM,
        max_zoom: MAX_ZOOM,
    })
}

/// Saves the pill and control bar size and resizes them if they're open.
#[tauri::command(async)]
pub fn set_overlay_zoom(app: AppHandle, zoom: f64) -> Result<DisplayInfo, String> {
    if !zoom.is_finite() {
        return Err("zoom must be a number".into());
    }
    let mut current = settings::load(&app);
    current.overlay_zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    settings::save(&app, &current)?;
    capture::refit(&app)?;
    display_info(app)
}

/// Match Windows' monitor interface ID to OBS's monitor_id (resolution alone is ambiguous).
pub(crate) fn monitor_capture_id(name: &str) -> Result<String, String> {
    use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};
    use windows::core::PCWSTR;
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    if !unsafe { EnumDisplayDevicesW(PCWSTR(name.as_ptr()), 0, &mut device, 1) }.as_bool() {
        return Err("Could not identify the monitor for OBS capture".into());
    }
    let len = device.DeviceID.iter().position(|c| *c == 0).unwrap_or(device.DeviceID.len());
    Ok(String::from_utf16_lossy(&device.DeviceID[..len]))
}
