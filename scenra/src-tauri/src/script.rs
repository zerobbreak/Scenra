//! The current script: drafted by the AI from the running target's page, read out by the pill,
//! and its actions run against the same page when narration reaches their word.
//!
//! A session is one pass through the script in the pill: a rehearsal (nothing recorded) or a
//! recording, where each scene is its own take in `<session id>/scene-NN-take-N`.

use crate::ai::{self, AiProvider, AiStatus, Gemini, Scenario};
use crate::engine;
use crate::input::Pace;
use crate::locate::Selector;
use crate::page;
use crate::take;
use serde::{Deserialize, Serialize};
use crate::target::{self, LaunchedTarget};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct CurrentScript(Mutex<Option<Scenario>>);

#[derive(Default)]
pub struct ScriptInitialState(Mutex<Vec<page::InitialState>>);

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Rehearse,
    Record,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    mode: Mode,
    /// Timestamp; recorded sessions keep their scene takes in a folder of this name.
    id: String,
}

#[derive(Default)]
pub struct CurrentSession(Mutex<Option<Session>>);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub fn ai_status() -> AiStatus {
    ai::status()
}

/// Outlines the running target's page, asks Gemini for a script, and shares it with the pill.
#[tauri::command(async)]
pub fn draft_script(
    app: AppHandle,
    targets: State<'_, LaunchedTarget>,
    script: State<'_, CurrentScript>,
    initial: State<'_, ScriptInitialState>,
    brief: String,
) -> Result<Scenario, String> {
    let provider = Gemini::from_store()?;
    let port = target::running_port(&targets)?;
    let mut page = page::page_session(port)?;
    let baseline = page::initial_state(&mut page)?;
    let outline = page::outline(&mut page)?;
    if outline.controls.is_empty() {
        return Err("the page has no visible links or buttons to script".into());
    }
    let scenario = provider.draft_script(&outline, brief.trim())?;
    if scenario.scenes.is_empty() {
        return Err("Gemini returned an empty script".into());
    }
    page::check_initial(&baseline, &page::initial_state(&mut page)?)?;
    *initial.0.lock().map_err(err)? = vec![baseline; scenario.scenes.len()];
    *script.0.lock().map_err(err)? = Some(scenario.clone());
    app.emit("script-changed", &scenario).map_err(err)?;
    Ok(scenario)
}

#[tauri::command]
pub fn current_script(script: State<'_, CurrentScript>) -> Result<Option<Scenario>, String> {
    Ok(script.0.lock().map_err(err)?.clone())
}

/// Starts a pass through the script and tells the pill, which starts over in that mode.
#[tauri::command]
pub fn start_session(app: AppHandle, session: State<'_, CurrentSession>, mode: Mode) -> Result<Session, String> {
    let started = Session { mode, id: take::stamp() };
    *session.0.lock().map_err(err)? = Some(started.clone());
    app.emit("session-changed", &started).map_err(err)?;
    Ok(started)
}

#[tauri::command]
pub fn current_session(session: State<'_, CurrentSession>) -> Result<Option<Session>, String> {
    Ok(session.0.lock().map_err(err)?.clone())
}

/// Glides the real cursor onto the control labelled `target` and hover-verifies it, ahead of
/// its trigger word.
#[tauri::command(async)]
pub fn approach_action(targets: State<'_, LaunchedTarget>, target: String) -> Result<(), String> {
    engine::approach(&targets, &Selector::Label(target), Pace::Natural).map(|_| ())
}

/// Clicks the control labelled `target` on its trigger word (re-aiming first if needed).
#[tauri::command(async)]
pub fn press_action(targets: State<'_, LaunchedTarget>, target: String, expires_at: u64) -> Result<(), String> {
    let expiry = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_millis(expires_at))
        .ok_or("Invalid action cue expiry")?;
    let remaining = expiry.duration_since(std::time::SystemTime::now())
        .map_err(|_| "The action missed its cue; retry the scene")?
        .min(std::time::Duration::from_millis(500));
    engine::press_before(&targets, &Selector::Label(target), Some(std::time::Instant::now() + remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live: needs a stored Gemini key and a CDP page on `SCENRA_TEST_CDP_PORT`.
    /// Run with `cargo test -p scenra gemini -- --ignored --nocapture`.
    #[test]
    #[ignore = "calls Gemini against a live page"]
    fn gemini_drafts_a_script_for_the_open_page() {
        let port: u16 = std::env::var("SCENRA_TEST_CDP_PORT").unwrap().parse().unwrap();
        let outline = page::outline(&mut page::page_session(port).unwrap()).unwrap();
        let scenario = Gemini::from_store().unwrap().draft_script(&outline, "A short product tour").unwrap();
        println!("{}", serde_json::to_string_pretty(&scenario).unwrap());
        assert!(!scenario.scenes.is_empty());
        assert!(scenario.scenes.iter().flat_map(|s| &s.lines).any(|l| !l.actions.is_empty()));
    }
}

/// Fail closed before OBS starts. Current scripts describe independent scenes on the draft page.
#[tauri::command(async)]
pub fn check_scene_start(
    targets: State<'_, LaunchedTarget>, script: State<'_, CurrentScript>,
    initial: State<'_, ScriptInitialState>, scene: usize,
) -> Result<(), String> {
    let scenario = script.0.lock().map_err(err)?.clone().ok_or("Draft a script first")?;
    let scene_index = scene;
    let scene = scenario.scenes.get(scene_index).ok_or("Unknown scene")?;
    let expected = initial.0.lock().map_err(err)?.get(scene_index).cloned().ok_or("Draft a script to capture its starting page")?;
    let (pid, port) = target::running(&targets)?;
    crate::locate::restore_target(pid)?;
    page::wait_until_settled(port, std::time::Duration::from_secs(15))?;
    let mut page = page::page_session(port)?;
    page::check_initial(&expected, &page::initial_state(&mut page)?)?;
    if let Some(action) = scene.lines.iter().flat_map(|l| &l.actions).next() {
        page::check_control(&mut page, &action.target)?;
    }
    Ok(())
}

/// Explicit presenter setup: navigate back to the page used to draft this script and reload it.
/// Does not replay actions, alter storage, or begin a recording.
#[tauri::command(async)]
pub fn reset_scene_page(
    targets: State<'_, LaunchedTarget>, initial: State<'_, ScriptInitialState>, scene: usize,
) -> Result<(), String> {
    let expected = initial.0.lock().map_err(err)?.get(scene).cloned().ok_or("Draft a script first")?;
    let port = target::running_port(&targets)?;
    let mut page = page::page_session(port)?;
    let result = page.call("Page.navigate", serde_json::json!({ "url": expected.url }))?;
    if result.get("errorText").is_some() { return Err("Could not restore the starting page".into()); }

    page::wait_until_settled(port, std::time::Duration::from_secs(15))
}

/// Presenter explicitly approves a manually prepared page as this scene's starting state.
#[tauri::command(async)]
pub fn approve_scene_start(
    targets: State<'_, LaunchedTarget>, script: State<'_, CurrentScript>,
    initial: State<'_, ScriptInitialState>, scene: usize,
) -> Result<(), String> {
    let scenario = script.0.lock().map_err(err)?.clone().ok_or("Draft a script first")?;
    let selected = scenario.scenes.get(scene).ok_or("Unknown scene")?;
    let port = target::running_port(&targets)?;
    page::wait_until_settled(port, std::time::Duration::from_secs(15))?;
    let mut page = page::page_session(port)?;
    if let Some(action) = selected.lines.iter().flat_map(|l| &l.actions).next() {
        page::check_control(&mut page, &action.target)?;
    }
    let baseline = page::initial_state(&mut page)?;
    let mut states = initial.0.lock().map_err(err)?;
    *states.get_mut(scene).ok_or("Draft a script first")? = baseline;
    Ok(())
}
