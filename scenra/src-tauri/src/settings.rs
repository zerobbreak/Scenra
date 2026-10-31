//! The presenter's own preferences, kept in `settings.json` in Scenra's config folder.
//! Only things a person adjusts by hand go here; nothing secret (that's `secrets.rs`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Size of the pill and control bar relative to their designed size, on top of Windows
    /// display scaling. 1.0 = as designed.
    pub overlay_zoom: f64,
    /// The monitor takes record, as a Windows device name (`\\.\DISPLAY2`); `None` = primary.
    pub capture_monitor: Option<String>,
    /// Where recorded sessions are kept; `None` = `Videos\Scenra` (see `session.rs`).
    pub sessions_dir: Option<std::path::PathBuf>,
    /// Whether narration may be sent to Gemini to transcribe it (editor AI features).
    pub transcription: crate::editor::transcript::Consent,
}

impl Default for Settings {
    fn default() -> Self {
        Self { overlay_zoom: 1.0, capture_monitor: None, sessions_dir: None, transcription: Default::default() }
    }
}

pub const MIN_ZOOM: f64 = 0.75;
pub const MAX_ZOOM: f64 = 1.5;

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("settings.json"))
}

/// The saved settings, or the defaults when there are none yet (or the file is unreadable).
pub fn load(app: &AppHandle) -> Settings {
    path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_files_record_the_primary_monitor() {
        let s: Settings = serde_json::from_str(r#"{ "overlayZoom": 1.25 }"#).unwrap();
        assert_eq!(s.overlay_zoom, 1.25);
        assert_eq!(s.capture_monitor, None);
    }
}
