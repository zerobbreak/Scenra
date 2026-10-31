//! A take: Scenra's built-in recorder (`recorder`) records one run into its own folder:
//! `video.mp4` (the chosen monitor), `microphone.wav` (the default mic, aligned to the video's
//! first frame), `metadata.json`, and `actions.json` (when and what Scenra clicked, so the editor
//! never cuts through a click). No OBS (docs/adr/0001-built-in-recorder.md). Folders live under
//! the sessions dir (`session.rs`).

use crate::page;
use crate::recorder::{self, Recorder, RecorderStatus, Snapshot, qpc_now};
use crate::session;
use crate::settings;
use crate::target::{self, LaunchedTarget};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};

/// How long preflight waits for the target page to load and settle before giving up.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct CurrentTake {
    running: Mutex<Option<Running>>,
}

struct Running {
    dir: PathBuf,
    recorder: Recorder,
    actions: Vec<LoggedAction>,
}

/// One thing Scenra did to the target during a take, on the QPC clock.
struct LoggedAction {
    kind: &'static str,
    label: String,
    start_qpc: i64,
    end_qpc: i64,
    ok: bool,
}

/// `actions.json`: times in seconds from the video's first frame.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActionRecord {
    /// `approach` (the cursor gliding to a control) or `press` (the click).
    pub kind: String,
    pub label: String,
    pub start: f64,
    pub end: f64,
    pub ok: bool,
}

pub const ACTIONS_FILE: &str = "actions.json";

fn records(actions: &[LoggedAction], video_start_qpc: i64) -> Vec<ActionRecord> {
    let seconds = |qpc: i64| ((qpc - video_start_qpc) as f64 / 1e7 * 1000.0).round() / 1000.0;
    actions
        .iter()
        .map(|a| ActionRecord { kind: a.kind.into(), label: a.label.clone(), start: seconds(a.start_qpc), end: seconds(a.end_qpc), ok: a.ok })
        .collect()
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
    /// Frames that reached the recorder more than 100 ms late (the machine fell behind), in percent.
    late_frames_percent: f64,
    /// Physical memory free when the take stopped, in MB.
    free_memory_mb: u64,
    /// Scenra's own committed memory when the take stopped, in MB.
    scenra_memory_mb: u64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Physical memory free right now, in MB.
pub fn free_memory_mb() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    match unsafe { GlobalMemoryStatusEx(&mut status) } {
        Ok(()) => status.ullAvailPhys / (1024 * 1024),
        Err(_) => 0,
    }
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
    memory_preflight(free_memory_mb())
}

/// What a take would record: monitor, mic and encoder.
#[tauri::command(async)]
pub fn recorder_status(app: AppHandle) -> Result<RecorderStatus, String> {
    recorder::status(settings::load(&app).capture_monitor.as_deref())
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

/// `folder` is relative to the takes folder, e.g. `20261022-101500/scene-02-take-1` for a scene
/// take; without it the take gets a timestamped folder of its own.
#[tauri::command(async)]
pub fn start_take(
    app: AppHandle,
    take: State<'_, CurrentTake>,
    targets: State<'_, LaunchedTarget>,
    folder: Option<String>,
) -> Result<String, String> {
    let monitor = settings::load(&app).capture_monitor;
    start_take_on(&take, &targets, &session::root(&app), folder, monitor.as_deref())
}

/// `start_take` on a given monitor (Windows device name), whatever the settings say.
pub(crate) fn start_take_on(
    take: &CurrentTake,
    targets: &LaunchedTarget,
    root: &Path,
    folder: Option<String>,
    monitor: Option<&str>,
) -> Result<String, String> {
    let mut slot = take.running.lock().map_err(err)?;
    if slot.is_some() {
        return Err("a take is already recording".into());
    }
    // Preflight: don't roll on a page that is still loading or resizing (B-3). With no target
    // running there is nothing to wait for, and the recorder records the desktop as it is.
    if let Ok(port) = target::running_port(targets) {
        page::wait_until_settled(port, SETTLE_TIMEOUT)?;
    }
    let folder = match folder {
        Some(folder) if is_safe_folder(&folder) => folder,
        Some(folder) => return Err(format!("not a take folder name: {folder}")),
        None => format!("{}/{}", session::CHECKS, stamp()),
    };
    let dir = root.join(folder);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dir = PathBuf::from(dir.canonicalize().map_err(err)?.to_string_lossy().trim_start_matches(r"\\?\"));
    let recorder = Recorder::start(&dir, monitor)?;
    let dir_str = dir.to_string_lossy().into_owned();
    *slot = Some(Running { dir, recorder, actions: Vec::new() });
    Ok(dir_str)
}

/// Letters, digits, `-`, `_` and `/` between non-empty parts: never escapes the takes folder.
fn is_safe_folder(folder: &str) -> bool {
    folder
        .split('/')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// Stops the current take. The take is unregistered whatever happens: stopping always ends the
/// recorder's threads, so nothing is left recording.
#[tauri::command(async)]
pub fn stop_take(take: State<'_, CurrentTake>) -> Result<TakeResult, String> {
    stop(&take)
}

pub(crate) fn stop(take: &CurrentTake) -> Result<TakeResult, String> {
    let Running { dir, recorder, actions } = take.running.lock().map_err(err)?.take().ok_or("no take is recording")?;
    let done = recorder.stop()?;
    session::write_json(&dir.join(ACTIONS_FILE), &records(&actions, done.video_start_qpc))?;
    Ok(TakeResult {
        dir: dir.to_string_lossy().into_owned(),
        video: done.video.to_string_lossy().into_owned(),
        microphone: done.microphone.to_string_lossy().into_owned(),
        microphone_seconds: done.microphone_seconds,
        sample_rate: done.sample_rate,
        channels: done.channels,
        late_frames_percent: done.late_frames_percent,
        free_memory_mb: free_memory_mb(),
        scenra_memory_mb: recorder::process_memory_mb(),
    })
}

/// The next frame the current take captures, and the monitor it's recording.
pub(crate) fn snapshot(take: &CurrentTake, timeout: Duration) -> Result<(Snapshot, String), String> {
    let running = take.running.lock().map_err(err)?;
    let running = running.as_ref().ok_or("no take is recording")?;
    Ok((running.recorder.snapshot(timeout)?, running.recorder.monitor().to_owned()))
}

/// Runs `action` and, if a take is recording, logs it with its start and end time.
pub(crate) fn logged<T>(take: &CurrentTake, kind: &'static str, label: &str, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let start_qpc = qpc_now();
    let result = action();
    if let Ok(mut running) = take.running.lock()
        && let Some(running) = running.as_mut()
    {
        running.actions.push(LoggedAction { kind, label: label.into(), start_qpc, end_qpc: qpc_now(), ok: result.is_ok() });
    }
    result
}

impl CurrentTake {
    /// Whether a take is recording right now.
    pub(crate) fn is_recording(&self) -> bool {
        self.running.lock().map(|running| running.is_some()).unwrap_or(true)
    }

    /// Hold the take lock for setup, so another window cannot start a take halfway through it.
    pub(crate) fn while_idle<T>(&self, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let running = self.running.lock().map_err(err)?;
        if running.is_some() { return Err("Stop the current take before preparing a scene".into()); }
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
        for bad in ["", "../x", "a/../b", "/abs", "a//b", r"a\b", "C:x", "a/"] {
            assert!(!is_safe_folder(bad), "{bad}");
        }
    }

    #[test]
    fn warns_below_one_gib_without_blocking_the_recorder() {
        assert!(memory_preflight(1023).warning.is_some());
        assert!(memory_preflight(1024).warning.is_none());
    }

    #[test]
    fn action_times_are_seconds_from_the_first_frame() {
        let logged = [LoggedAction { kind: "press", label: "Settings".into(), start_qpc: 12_500_000, end_qpc: 14_000_000, ok: true }];
        assert_eq!(
            records(&logged, 10_000_000),
            vec![ActionRecord { kind: "press".into(), label: "Settings".into(), start: 0.25, end: 0.4, ok: true }]
        );
    }

    #[test]
    fn reads_free_memory() {
        assert!(free_memory_mb() > 0);
    }
}
