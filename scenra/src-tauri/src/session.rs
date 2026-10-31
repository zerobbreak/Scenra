//! Where recorded sessions live, and what each one keeps on disk so it can be edited later
//! (docs/adr/0002-editor.md).
//!
//! ```text
//! <sessions dir>/<session id>/
//! ├── session.json            id, created, title
//! ├── script.json             the Scenario the scenes were recorded from
//! ├── initial.json            each scene's starting page, for retakes
//! ├── scene-01-take-1/        video.mp4, microphone.wav, metadata.json, actions.json
//! ├── edit.json               the editor's non-destructive edit
//! └── export/                 finished videos
//! ```
//!
//! The sessions dir is `Videos\Scenra` unless the presenter picked another folder, or
//! `SCENRA_SESSIONS_DIR` is set (tests, dev runs). Capture-matrix takes go to `checks/`.

use crate::ai::Scenario;
use crate::settings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub const ENV: &str = "SCENRA_SESSIONS_DIR";
/// Takes that aren't part of a session (capture matrix, debug takes).
pub const CHECKS: &str = "checks";
pub const SESSION_FILE: &str = "session.json";
pub const SCRIPT_FILE: &str = "script.json";
/// Each scene's starting page (`page::InitialState`), for retakes.
pub const INITIAL_FILE: &str = "initial.json";
/// Before sessions moved to Videos\Scenra; development builds still list sessions found here.
pub const LEGACY_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../recordings/takes");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
}

/// The Windows Videos folder (follows OneDrive or other redirection).
fn videos_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None).ok()?;
        let result = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0.cast()));
        result
    }
}

/// Where new sessions are recorded.
pub fn root(app: &AppHandle) -> PathBuf {
    root_from(std::env::var_os(ENV).map(PathBuf::from), settings::load(app).sessions_dir, videos_dir())
}

fn root_from(env: Option<PathBuf>, chosen: Option<PathBuf>, videos: Option<PathBuf>) -> PathBuf {
    env.or(chosen)
        .or_else(|| videos.map(|v| v.join("Scenra")))
        .unwrap_or_else(|| PathBuf::from(LEGACY_DIR))
}

/// Every folder sessions may be found in: the current root, plus the legacy repo folder in dev builds.
pub fn search_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = vec![root(app)];
    let legacy = PathBuf::from(LEGACY_DIR);
    if cfg!(debug_assertions) && legacy.is_dir() && !dirs.contains(&legacy) {
        dirs.push(legacy);
    }
    dirs
}

/// Writes `session.json` and `script.json` for a recorded session (idempotent).
pub fn save(dir: &Path, info: &SessionInfo, script: Option<&Scenario>) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    write_json(&dir.join(SESSION_FILE), info)?;
    if let Some(script) = script {
        write_json(&dir.join(SCRIPT_FILE), script)?;
    }
    Ok(())
}

/// Writes JSON via a temp file and a rename, so a crash never leaves half a file.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, json).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_beats_settings_beats_videos() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(root_from(p("E"), p("S"), p("V")), PathBuf::from("E"));
        assert_eq!(root_from(None, p("S"), p("V")), PathBuf::from("S"));
        assert_eq!(root_from(None, None, p("V")), PathBuf::from("V").join("Scenra"));
        assert_eq!(root_from(None, None, None), PathBuf::from(LEGACY_DIR));
    }

    #[test]
    fn finds_the_videos_folder() {
        assert!(videos_dir().is_some_and(|v| v.is_absolute()));
    }

    #[test]
    fn json_round_trips_without_leaving_a_partial_file() {
        let dir = std::env::temp_dir().join(format!("scenra-session-{}", std::process::id()));
        let info = SessionInfo { id: "20261031-120000".into(), title: "Demo".into() };
        save(&dir, &info, None).unwrap();
        assert_eq!(read_json::<SessionInfo>(&dir.join(SESSION_FILE)), Some(info));
        assert!(!dir.join("session.json.partial").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
