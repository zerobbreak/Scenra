//! A take: OBS records one run into its own folder, then Scenra extracts `microphone.wav` from
//! audio track 2 (PROJECT_OUTLINE §9).
//!
//! Scenra records with its own OBS profile, "Scenra" (Advanced output, MKV, tracks 1+2, PCM audio),
//! and switches back to the presenter's profile afterwards, so their own OBS settings are never
//! changed. A recorded session keeps the Scenra profile between its scene takes: each switch
//! resets OBS's video pipeline, and a take started during the reset loses frames. The mic goes to
//! tracks 1 and 2, desktop audio to track 1 only: one recorder, so the mic is never recorded twice.
//! Video encoding is set up by `encoding`.

use crate::encoding;
use crate::mkv;
use crate::obs::Obs;
use crate::page;
use crate::target::{self, LaunchedTarget};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;

const PROFILE: &str = "Scenra";
/// How long preflight waits for the target page to load and settle before giving up.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(15);
/// Phase 0 keeps takes in the repo's git-ignored `recordings/` folder.
const TAKES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../recordings/takes");
const SETTINGS: [(&str, &str, &str); 5] = [
    ("Output", "Mode", "Advanced"),
    ("AdvOut", "RecType", "Standard"),
    ("AdvOut", "RecFormat2", "mkv"),
    ("AdvOut", "RecTracks", "3"), // bitmask: tracks 1 and 2
    ("AdvOut", "RecAudioEncoder", "ffmpeg_pcm_s16le"),
];

#[derive(Default)]
pub struct CurrentTake {
    running: Mutex<Option<Running>>,
    /// The presenter's own OBS profile while Scenra's is switched in; restored after the take,
    /// or after the last scene of a session.
    presenter_profile: Mutex<Option<String>>,
}

#[derive(Clone)]
struct Running {
    dir: PathBuf,
    frames_at_start: encoding::FrameCounts,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsStatus {
    version: String,
    profile: String,
    recording: bool,
    mic: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeResult {
    dir: String,
    video: String,
    microphone: String,
    microphone_seconds: f64,
    sample_rate: u32,
    channels: u16,
    /// Frames OBS dropped to rendering or encoding lag during the take, in percent.
    dropped_frames_percent: f64,
    /// Physical memory free when the take stopped, in MB.
    free_memory_mb: u64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TakePreflight {
    free_memory_mb: u64,
    warning: Option<String>,
}

fn memory_preflight(free_memory_mb: u64) -> TakePreflight {
    let warning = (free_memory_mb < 1024).then(|| format!(
        "Only {free_memory_mb} MB of RAM is free. Close other apps and check again, or record anyway."
    ));
    TakePreflight { free_memory_mb, warning }
}

/// Advisory only: the presenter decides whether to proceed on a busy machine.
#[tauri::command]
pub fn take_preflight() -> TakePreflight {
    memory_preflight(encoding::free_memory_mb())
}

#[tauri::command(async)]
pub fn obs_status() -> Result<ObsStatus, String> {
    let mut obs = Obs::connect()?;
    let version = obs.request("GetVersion", json!({}))?;
    let profiles = obs.request("GetProfileList", json!({}))?;
    let recording = obs.request("GetRecordStatus", json!({}))?;
    let special = obs.request("GetSpecialInputs", json!({}))?;
    Ok(ObsStatus {
        version: version["obsVersion"].as_str().unwrap_or_default().to_owned(),
        profile: profiles["currentProfileName"].as_str().unwrap_or_default().to_owned(),
        recording: recording["outputActive"] == true,
        mic: special["mic1"].as_str().map(str::to_owned),
    })
}

/// "20261021-001523", UTC, for folder names.
pub fn stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Switches OBS to the Scenra profile, creating and configuring it on first use.
/// Returns the profile that was current before.
fn use_scenra_profile(obs: &mut Obs) -> Result<String, String> {
    let profiles = obs.request("GetProfileList", json!({}))?;
    let current = profiles["currentProfileName"].as_str().unwrap_or_default().to_owned();
    let exists = profiles["profiles"].as_array().is_some_and(|p| p.iter().any(|n| n == PROFILE));
    if !exists {
        obs.request("CreateProfile", json!({ "profileName": PROFILE }))?; // also switches to it
    } else if current != PROFILE {
        obs.request("SetCurrentProfile", json!({ "profileName": PROFILE }))?;
    }

    let encoder = choose_encoder(obs)?;
    let mut changed = write_encoder_settings(encoder)?;
    let video = [("AdvOut", "RecEncoder", encoder), ("AdvOut", "RecUseRescale", "false")];
    for (category, name, value) in SETTINGS.into_iter().chain(video) {
        let now = obs.request("GetProfileParameter", json!({ "parameterCategory": category, "parameterName": name }))?;
        if now["parameterValue"] != value {
            obs.request(
                "SetProfileParameter",
                json!({ "parameterCategory": category, "parameterName": name, "parameterValue": value }),
            )?;
            changed = true;
        }
    }
    // Output settings are read when the profile loads, so reload it after a change.
    if changed {
        let other = profiles["profiles"]
            .as_array()
            .and_then(|p| p.iter().filter_map(Value::as_str).find(|n| *n != PROFILE))
            .map(str::to_owned);
        if let Some(other) = other {
            obs.request("SetCurrentProfile", json!({ "profileName": other }))?;
            obs.request("SetCurrentProfile", json!({ "profileName": PROFILE }))?;
        }
    }
    encoding::apply_video(obs, encoder)?;
    Ok(if current == PROFILE || current.is_empty() { "Untitled".into() } else { current })
}

/// The GPU's hardware encoder, unless it has already failed to start on this machine.
fn choose_encoder(obs: &mut Obs) -> Result<&'static str, String> {
    let (category, name) = encoding::FAILED;
    let failed = obs.request("GetProfileParameter", json!({ "parameterCategory": category, "parameterName": name }))?;
    Ok(match encoding::hardware() {
        Some(hardware) if failed["parameterValue"] != hardware => hardware,
        _ => encoding::X264,
    })
}

/// Writes the profile's `recordEncoder.json` if it doesn't already hold these settings (OBS adds
/// its own keys when it saves, so only Scenra's are compared). Returns whether it wrote.
fn write_encoder_settings(encoder: &str) -> Result<bool, String> {
    let Some(dir) = encoding::profile_dir(PROFILE) else { return Ok(false) };
    let path = dir.join("recordEncoder.json");
    let want = encoding::settings(encoder);
    let have: Value = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if want.as_object().is_some_and(|w| w.iter().all(|(k, v)| have.get(k) == Some(v))) {
        return Ok(false);
    }
    std::fs::write(&path, want.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// Starts recording into `dir` and waits until the output is live.
fn record(obs: &mut Obs, dir: &str) -> Result<(), String> {
    obs.request("SetRecordDirectory", json!({ "recordDirectory": dir }))?;
    obs.request("StartRecord", json!({}))?;
    // StartRecord returns before the output is live; a scene must not start before its take does.
    wait_for_recording(obs, true)
}

/// Mic on tracks 1+2, desktop audio on track 1 only.
fn route_audio(obs: &mut Obs) -> Result<(), String> {
    let special = obs.request("GetSpecialInputs", json!({}))?;
    let tracks = |on: &[&str]| -> Value {
        (1..=6).map(|t| (t.to_string(), json!(on.contains(&t.to_string().as_str())))).collect::<serde_json::Map<_, _>>().into()
    };
    let mic = special["mic1"].as_str().ok_or("OBS has no microphone set (Settings › Audio › Mic/Auxiliary Audio)")?;
    obs.request("SetInputAudioTracks", json!({ "inputName": mic, "inputAudioTracks": tracks(&["1", "2"]) }))?;
    for desktop in ["desktop1", "desktop2"] {
        if let Some(name) = special[desktop].as_str() {
            obs.request("SetInputAudioTracks", json!({ "inputName": name, "inputAudioTracks": tracks(&["1"]) }))?;
        }
    }
    Ok(())
}

/// `folder` is relative to the takes folder, e.g. `20261022-101500/scene-02-take-1` for a scene
/// take; without it the take gets a timestamped folder of its own.
#[tauri::command(async)]
pub fn start_take(
    take: State<'_, CurrentTake>,
    targets: State<'_, LaunchedTarget>,
    folder: Option<String>,
) -> Result<String, String> {
    let mut slot = take.running.lock().map_err(err)?;
    if slot.is_some() {
        return Err("a take is already recording".into());
    }
    // Preflight: don't roll on a page that is still loading or resizing (B-3). With no target
    // running there is nothing to wait for, and OBS records the desktop as it is.
    if let Ok(port) = target::running_port(&targets) {
        page::wait_until_settled(port, SETTLE_TIMEOUT)?;
    }
    let folder = match folder {
        Some(folder) if is_safe_folder(&folder) => folder,
        Some(folder) => return Err(format!("not a take folder name: {folder}")),
        None => stamp(),
    };
    let dir = PathBuf::from(TAKES_DIR).join(folder);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dir = dir.canonicalize().map_err(err)?;
    let dir_str = dir.to_string_lossy().trim_start_matches(r"\\?\").to_owned();

    let mut obs = Obs::connect()?;
    if obs.request("GetRecordStatus", json!({}))?["outputActive"] == true {
        return Err("OBS is already recording".into());
    }
    let profile = use_scenra_profile(&mut obs)?;
    // Between a session's scenes OBS is already on the Scenra profile; keep the presenter's.
    take.presenter_profile.lock().map_err(err)?.get_or_insert(profile);
    route_audio(&mut obs)?;
    let frames_at_start = encoding::FrameCounts::read(&mut obs)?;
    if let Err(e) = record(&mut obs, &dir_str) {
        let encoder = obs.request("GetProfileParameter", json!({ "parameterCategory": "AdvOut", "parameterName": "RecEncoder" }))?;
        let encoder = encoder["parameterValue"].as_str().unwrap_or_default().to_owned();
        let _ = obs.request("StopRecord", json!({}));
        if encoder == encoding::X264 {
            return Err(e);
        }
        // The hardware encoder wouldn't start: remember that, and record with x264 instead.
        let (category, name) = encoding::FAILED;
        obs.request(
            "SetProfileParameter",
            json!({ "parameterCategory": category, "parameterName": name, "parameterValue": encoder }),
        )?;
        use_scenra_profile(&mut obs)?;
        record(&mut obs, &dir_str).map_err(|retry| format!("{encoder} failed ({e}); x264 too: {retry}"))?;
    }
    *slot = Some(Running { dir: PathBuf::from(&dir_str), frames_at_start });
    Ok(dir_str)
}

/// Waits until OBS reports the record output active (or inactive).
fn wait_for_recording(obs: &mut Obs, active: bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if (obs.request("GetRecordStatus", json!({}))?["outputActive"] == true) == active {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("OBS didn't {} recording within 15 s", if active { "start" } else { "stop" }))
}

/// Letters, digits, `-`, `_` and `/` between non-empty parts: never escapes the takes folder.
fn is_safe_folder(folder: &str) -> bool {
    folder
        .split('/')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// Waits until OBS has finished writing the file (its size stops changing).
fn settle(path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last = None;
    while Instant::now() < deadline {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size > 0 && last == Some(size) {
            return Ok(());
        }
        last = Some(size);
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("{} was still being written after 30 s", path.display()))
}

/// Switches OBS back to the presenter's profile, if Scenra's is still switched in.
fn restore_profile(take: &CurrentTake, obs: &mut Obs) -> Result<(), String> {
    if let Some(profile) = take.presenter_profile.lock().map_err(err)?.take() {
        obs.request("SetCurrentProfile", json!({ "profileName": profile }))?;
    }
    Ok(())
}

/// Ends a recorded session: puts the presenter's OBS profile back (no-op if it already is).
#[tauri::command(async)]
pub fn restore_obs_profile(take: State<'_, CurrentTake>) -> Result<(), String> {
    if take.presenter_profile.lock().map_err(err)?.is_none() {
        return Ok(());
    }
    restore_profile(&take, &mut Obs::connect()?)
}

/// The take stays registered until OBS has actually stopped, so a failed stop (OBS busy or
/// unreachable) can be retried rather than leaving OBS recording with nothing to stop it.
/// `keep_profile` leaves OBS on the Scenra profile for the session's next scene.
#[tauri::command(async)]
pub fn stop_take(take: State<'_, CurrentTake>, keep_profile: Option<bool>) -> Result<TakeResult, String> {
    let running = take.running.lock().map_err(err)?.clone().ok_or("no take is recording")?;
    let mut obs = Obs::connect()?;
    let stopped = obs.request("StopRecord", json!({}));
    // StopRecord returns while the output is still flushing (seconds when the encoder is behind).
    // Switching profile or reading the file before it has stopped cuts the take short.
    let stopped_at = wait_for_recording(&mut obs, false);
    let dropped_frames_percent = encoding::FrameCounts::read(&mut obs)
        .map(|end| (end.dropped_percent_since(&running.frames_at_start) * 10.0).round() / 10.0)
        .unwrap_or_default();
    if stopped.is_ok() || stopped_at.is_ok() {
        *take.running.lock().map_err(err)? = None;
    }
    let restore = if keep_profile == Some(true) { Ok(()) } else { restore_profile(&take, &mut obs) };
    let video = PathBuf::from(stopped?["outputPath"].as_str().ok_or("OBS didn't say where it saved the take")?);
    stopped_at?;
    restore?;
    settle(&video)?;

    let microphone = running.dir.join("microphone.wav");
    let extracted = mkv::extract_audio(&video, 1, &microphone)?;
    Ok(TakeResult {
        dir: running.dir.to_string_lossy().into_owned(),
        video: video.to_string_lossy().into_owned(),
        microphone: microphone.to_string_lossy().into_owned(),
        microphone_seconds: extracted.seconds,
        sample_rate: extracted.track.sample_rate,
        channels: extracted.track.channels,
        dropped_frames_percent,
        free_memory_mb: encoding::free_memory_mb(),
    })
}

impl CurrentTake {
    /// Hold the take lock for setup, so another window cannot start OBS halfway through it.
    pub(crate) fn while_idle<T>(&self, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let running = self.running.lock().map_err(err)?;
        if running.is_some() { return Err("Stop the current take before preparing a scene".into()); }
        let mut obs = Obs::connect().map_err(|e| format!("Cannot confirm OBS is idle: {e}"))?;
        if obs.request("GetRecordStatus", json!({}))?["outputActive"] == true {
            return Err("OBS is recording; stop it before preparing a scene".into());
        }
        let result = work();
        drop(running);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_is_a_sortable_utc_timestamp() {
        let s = stamp();
        assert_eq!(s.len(), 15);
        assert_eq!(&s[8..9], "-");
        assert!(s.starts_with("20"));
    }

    #[test]
    fn take_folders_stay_inside_the_takes_folder() {
        assert!(is_safe_folder("20261022-101500/scene-02-take-1"));
        for bad in ["", "../x", "a/../b", "/abs", "a//b", r"a", "C:x", "a/"] {
            assert!(!is_safe_folder(bad), "{bad}");
        }
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    #[test]
    fn warns_below_one_gib_without_blocking_the_recorder() {
        assert!(memory_preflight(1023).warning.is_some());
        assert!(memory_preflight(1024).warning.is_none());
    }
}
