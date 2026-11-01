//! The Editor (docs/adr/0002-editor.md): turns a recorded session into one finished video.
//!
//! A session's takes are never changed. The presenter's choices (which take per scene, trims,
//! cuts) live in `edit.json`; `model::timeline` turns them into the segments that preview plays
//! and `export` renders.

pub mod assemble;
pub mod audio;
pub mod captions;
pub mod describe;
pub mod export;
pub mod library;
pub mod model;
pub mod transcript;

use library::{SessionData, SessionSummary};
use model::Edit;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const EDITOR: &str = "editor";

#[derive(Default)]
pub struct ExportState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Lets the editor's webview play files from the session folders (asset protocol).
fn allow_media(app: &AppHandle, dir: &Path) -> Result<(), String> {
    app.asset_protocol_scope().allow_directory(dir, true).map_err(err)
}

/// `canonicalize` gives `\\?\C:\…`, which Windows' storage APIs reject; drop the prefix.
fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// Only folders inside a sessions dir can be opened, edited or exported.
fn session_dir(app: &AppHandle, dir: &str) -> Result<PathBuf, String> {
    let dir = plain(&PathBuf::from(dir).canonicalize().map_err(|e| format!("{dir}: {e}"))?);
    let inside = crate::session::search_dirs(app)
        .iter()
        .filter_map(|root| root.canonicalize().ok().map(|r| plain(&r)))
        .any(|root| dir.parent() == Some(root.as_path()));
    if !inside {
        return Err("That folder isn't a Scenra session".into());
    }
    allow_media(app, &dir)?;
    Ok(dir)
}

/// Opens (or focuses) the Editor window, optionally on one session.
#[tauri::command(async)]
pub fn open_editor(app: AppHandle, session: Option<String>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(EDITOR) {
        let _ = window.unminimize();
        window.set_focus().map_err(err)?;
        if let Some(session) = session {
            app.emit_to(EDITOR, "editor-open-session", session).map_err(err)?;
        }
        return Ok(());
    }
    // A new window isn't listening yet, so the session goes in its URL.
    let url = match session.filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
        Some(session) => WebviewUrl::App(format!("index.html?session={session}").into()),
        None => WebviewUrl::default(),
    };
    WebviewWindowBuilder::new(&app, EDITOR, url)
        .title("Scenra Editor")
        .inner_size(1180.0, 760.0)
        .min_inner_size(820.0, 560.0)
        .build()
        .map_err(err)?;
    Ok(())
}

#[tauri::command(async)]
pub fn list_sessions(app: AppHandle) -> Vec<SessionSummary> {
    library::list(&crate::session::search_dirs(&app))
}

/// A session by id (newest match across sessions dirs) or by folder path.
#[tauri::command(async)]
pub fn load_session(app: AppHandle, session: String) -> Result<SessionData, String> {
    let dir = if Path::new(&session).is_absolute() {
        session
    } else {
        crate::session::search_dirs(&app)
            .into_iter()
            .map(|root| root.join(&session))
            .find(|dir| dir.is_dir())
            .ok_or(format!("Session {session} wasn't found"))?
            .to_string_lossy()
            .into_owned()
    };
    library::load(&session_dir(&app, &dir)?)
}

/// Saves the edit and returns the session with its new timeline.
#[tauri::command(async)]
pub fn save_edit(app: AppHandle, dir: String, edit: Edit) -> Result<SessionData, String> {
    let dir = session_dir(&app, &dir)?;
    library::save_edit(&dir, &edit)?;
    library::load(&dir)
}

/// Tidy up: trims dead air and shortens long pauses in every scene, from the mic alone.
/// Replaces earlier automatic changes; the presenter's own cuts stay.
#[tauri::command(async)]
pub fn tidy_session(app: AppHandle, dir: String) -> Result<SessionData, String> {
    let dir = session_dir(&app, &dir)?;
    let data = library::load(&dir)?;
    let mut edit = data.edit.clone();
    for scene in &mut edit.scenes {
        let Some(take) = data.takes.iter().find(|t| Some(&t.folder) == scene.take.as_ref()) else { continue };
        let pcm = audio::read_wav(&dir.join(&take.folder).join(library::MIC))?;
        *scene = audio::tidy(scene, take.duration, &audio::speech(&pcm), &take.actions);
    }
    library::save_edit(&dir, &edit)?;
    library::load(&dir)
}

/// Levelled copies of the mic for every take in the timeline, in `<session>/.edit-cache/`.
fn level_takes(dir: &Path, takes: &[&str]) -> Result<(), String> {
    let cache = dir.join(CACHE_DIR);
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    for take in takes {
        let pcm = audio::read_wav(&dir.join(take).join(library::MIC))?;
        let levelled = match audio::loudness(&pcm) {
            Some(measured) => audio::level(&pcm, audio::gain_db(measured)),
            None => pcm, // silent take: nothing to level
        };
        audio::write_wav(&cache.join(format!("{take}.wav")), &levelled)?;
    }
    Ok(())
}

pub const CACHE_DIR: &str = ".edit-cache";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    consent: transcript::Consent,
    configured: bool,
}

#[tauri::command]
pub fn editor_ai_status(app: AppHandle) -> AiStatus {
    AiStatus { consent: crate::settings::load(&app).transcription, configured: crate::ai::status_configured() }
}

#[tauri::command(async)]
pub fn set_transcription(app: AppHandle, consent: transcript::Consent) -> Result<AiStatus, String> {
    let mut current = crate::settings::load(&app);
    current.transcription = consent;
    crate::settings::save(&app, &current)?;
    Ok(editor_ai_status(app))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssembleProgress {
    dir: String,
    done: usize,
    total: usize,
}

/// Assemble with AI: transcribes every take (Gemini, once per take, at most 3 at a time), then
/// drafts the edit: best take per scene, trims, shortened pauses, stumbles cut.
#[tauri::command(async)]
pub fn assemble_session(app: AppHandle, dir: String) -> Result<SessionData, String> {
    if crate::settings::load(&app).transcription != transcript::Consent::Allowed {
        return Err("Allow sending narration to Gemini first".into());
    }
    let gemini = crate::ai::Gemini::from_store()?;
    let dir = session_dir(&app, &dir)?;
    let data = library::load(&dir)?;
    let key = dir.to_string_lossy().into_owned();
    let total = data.takes.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let failures = Mutex::new(Vec::new());
    let queue = Mutex::new(data.takes.iter());
    std::thread::scope(|scope| {
        for _ in 0..3 {
            scope.spawn(|| {
                while let Some(take) = queue.lock().ok().and_then(|mut q| q.next()) {
                    let take_dir = dir.join(&take.folder);
                    if let Err(e) = transcript::transcribe_take(&take_dir, &take_dir.join(library::MIC), &gemini)
                        && let Ok(mut f) = failures.lock()
                    {
                        f.push(format!("{}: {e}", take.folder));
                    }
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    let _ = app.emit_to(EDITOR, "assemble-progress", AssembleProgress { dir: key.clone(), done: n, total });
                }
            });
        }
    });
    let failures = failures.into_inner().map_err(err)?;
    if !failures.is_empty() && failures.len() == total {
        return Err(format!("Couldn't transcribe the takes: {}", failures.join("; ")));
    }

    let data = library::load(&dir)?; // with the new transcripts
    let mut edit = data.edit.clone();
    for scene in &mut edit.scenes {
        let candidates: Vec<assemble::Candidate> = data
            .takes
            .iter()
            .filter(|t| t.scene == scene.scene)
            .map(|t| assemble::Candidate { take: t, words: data.transcripts.get(&t.folder).map(Vec::as_slice) })
            .collect();
        let lines: Vec<String> = data
            .script
            .as_ref()
            .and_then(|s| s.scenes.get(scene.scene))
            .map(|s| s.lines.iter().map(|l| l.text.clone()).collect())
            .unwrap_or_default();
        let fallback = |t: &library::TakeInfo| {
            audio::read_wav(&dir.join(&t.folder).join(library::MIC)).map(|pcm| audio::speech(&pcm)).unwrap_or_default()
        };
        *scene = assemble::assemble_scene(scene, &candidates, &assemble::script_words(&lines), fallback);
    }
    library::save_edit(&dir, &edit)?;
    let mut data = library::load(&dir)?;
    data.timeline.warnings.extend(failures.into_iter().map(|f| format!("Not transcribed: {f}")));
    Ok(data)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportProgress {
    dir: String,
    percent: f64,
}

/// File-name-safe version of the session title.
fn file_stem(title: &str) -> String {
    let stem: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if stem.is_empty() { "Scenra video".into() } else { stem.chars().take(60).collect() }
}

/// Renders the session's current edit to `<session>/export/<title> <stamp>.mp4`. Emits
/// `export-progress` as it goes; `cancel_export` stops it.
#[tauri::command(async)]
pub fn export_session(app: AppHandle, state: tauri::State<'_, ExportState>, dir: String) -> Result<String, String> {
    let dir = session_dir(&app, &dir)?;
    let data = library::load(&dir)?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.cancel.lock().map_err(err)?;
        if slot.is_some() {
            return Err("An export is already running".into());
        }
        *slot = Some(cancel.clone());
    }
    let out = dir
        .join(library::EXPORT_DIR)
        .join(format!("{} {}.mp4", file_stem(&data.title), crate::take::stamp()));
    let mut used: Vec<&str> = data.timeline.segments.iter().map(|s| s.take.as_str()).collect();
    used.dedup();
    if let Err(e) = level_takes(&dir, &used) {
        *state.cancel.lock().map_err(err)? = None;
        return Err(format!("Couldn't level the voice: {e}"));
    }
    let cache = dir.join(CACHE_DIR);
    let job = export::Job {
        session_dir: dir.clone(),
        segments: data.timeline.segments.clone(),
        out,
        video_name: library::VIDEO,
        mic_name: library::MIC,
        audio_override: Box::new(move |take| Some(cache.join(format!("{take}.wav"))).filter(|p| p.is_file())),
    };
    let (emitter, key) = (app.clone(), dir.to_string_lossy().into_owned());
    let result = export::render(
        job,
        move |percent| {
            let _ = emitter.emit_to(EDITOR, "export-progress", ExportProgress { dir: key.clone(), percent });
        },
        cancel,
    );
    *state.cancel.lock().map_err(err)? = None;
    let path = result?;
    // Captions, when the takes in the video have transcripts.
    let cues = captions::cues(&data.timeline.segments, |take| data.transcripts.get(take).map(Vec::as_slice));
    if !cues.is_empty() {
        std::fs::write(path.with_extension("srt"), captions::srt(&cues)).map_err(|e| format!("Captions: {e}"))?;
    }
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn cancel_export(state: tauri::State<'_, ExportState>) -> Result<(), String> {
    if let Some(cancel) = state.cancel.lock().map_err(err)?.as_ref() {
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// Retake a scene from the editor: loads the session's script into the prompter, resumes the
/// session in record mode at `scene` (numbering new takes after the existing ones), and opens
/// the prompter. The prompter's start check and Replay setup get the app to the scene's page.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn retake_scene(
    app: AppHandle,
    script: tauri::State<'_, crate::script::CurrentScript>,
    initial: tauri::State<'_, crate::script::ScriptInitialState>,
    current: tauri::State<'_, crate::script::CurrentSession>,
    setup: tauri::State<'_, crate::setup::SetupState>,
    teach: tauri::State<'_, crate::teach::TeachState>,
    take: tauri::State<'_, crate::take::CurrentTake>,
    dir: String,
    scene: usize,
) -> Result<(), String> {
    let dir = session_dir(&app, &dir)?;
    if dir.parent() != Some(plain(&crate::session::root(&app).canonicalize().map_err(err)?).as_path()) {
        return Err("Only sessions in the current sessions folder can be retaken".into());
    }
    let scenario: crate::ai::Scenario = crate::session::read_json(&dir.join(crate::session::SCRIPT_FILE))
        .ok_or("This session has no saved script (recorded before scripts were saved), so it can't be retaken")?;
    if scene >= scenario.scenes.len() {
        return Err(format!("The script has no scene {}", scene + 1));
    }
    setup.ensure_idle()?;
    teach.ensure_idle()?;
    if take.is_recording() {
        return Err("Stop the current take first".into());
    }
    let states: Vec<crate::page::InitialState> = crate::session::read_json(&dir.join(crate::session::INITIAL_FILE)).unwrap_or_default();
    let data = library::load(&dir)?;
    let attempts = library::attempts_in(&dir);
    *script.0.lock().map_err(err)? = Some(scenario.clone());
    *initial.0.lock().map_err(err)? = states;
    app.emit("script-changed", &scenario).map_err(err)?;
    let session = crate::script::Session { mode: crate::script::Mode::Record, id: data.id, scene: Some(scene), attempts };
    *current.0.lock().map_err(err)? = Some(session.clone());
    app.emit("session-changed", &session).map_err(err)?;
    crate::capture::open_teleprompter(app, true).map(|_| ())
}

/// Title, description and chapters for publishing, saved as `export/description.txt`.
#[tauri::command(async)]
pub fn describe_session(app: AppHandle, dir: String) -> Result<describe::Description, String> {
    let dir = session_dir(&app, &dir)?;
    let data = library::load(&dir)?;
    if data.timeline.segments.is_empty() {
        return Err("Nothing is in the video yet".into());
    }
    let scene_title = |scene: usize| {
        data.script.as_ref().and_then(|s| s.scenes.get(scene)).map(|s| s.title.clone()).unwrap_or_else(|| format!("Scene {}", scene + 1))
    };
    let in_video: std::collections::BTreeSet<usize> = data.timeline.segments.iter().map(|s| s.scene).collect();
    let material: Vec<String> = in_video
        .iter()
        .filter_map(|&scene| {
            let text = match data.script.as_ref().and_then(|s| s.scenes.get(scene)) {
                Some(s) => s.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" "),
                None => {
                    let take = data.edit.scenes.iter().find(|s| s.scene == scene)?.take.as_ref()?;
                    data.transcripts.get(take)?.iter().filter(|w| !w.filler).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
                }
            };
            (!text.trim().is_empty()).then(|| format!("{}: {text}", scene_title(scene)))
        })
        .collect();
    if material.is_empty() {
        return Err("There's no script or transcript to describe the video from".into());
    }
    let (title, description) = describe::write(&crate::ai::Gemini::from_store()?, &material.join("\n\n"))?;
    let (chapters, chapter_note) = describe::chapters(&data.timeline.segments, data.timeline.duration, scene_title);
    let out = describe::Description { title, description, chapters, chapter_note };
    let export = dir.join(library::EXPORT_DIR);
    std::fs::create_dir_all(&export).map_err(err)?;
    let text = format!("{}\n\n{}\n\nChapters\n{}\n", out.title, out.description, out.chapters);
    std::fs::write(export.join("description.txt"), text).map_err(err)?;
    Ok(out)
}

/// Shows a file (or folder) in Explorer.
#[tauri::command(async)]
pub fn reveal_in_explorer(app: AppHandle, path: String) -> Result<(), String> {
    let path = PathBuf::from(&path);
    let dir = if path.is_dir() { path.clone() } else { path.parent().ok_or("no folder")?.to_owned() };
    let session = if dir.file_name().is_some_and(|n| n == library::EXPORT_DIR) { dir.parent().unwrap_or(&dir) } else { &dir };
    session_dir(&app, &session.to_string_lossy())?;
    let arg = if path.is_dir() { path.as_os_str().to_owned() } else { format!("/select,{}", path.display()).into() };
    std::process::Command::new("explorer.exe").arg(arg).spawn().map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_safe_file_names() {
        assert_eq!(file_stem("Lumina: a tour / v2?"), "Lumina a tour v2");
        assert_eq!(file_stem("  "), "Scenra video");
    }
}
