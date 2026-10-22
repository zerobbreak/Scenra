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
    brief: String,
) -> Result<Scenario, String> {
    let provider = Gemini::from_store()?;
    let port = target::running_port(&targets)?;
    let outline = page::outline(&mut page::page_session(port)?)?;
    if outline.controls.is_empty() {
        return Err("the page has no visible links or buttons to script".into());
    }
    let scenario = provider.draft_script(&outline, brief.trim())?;
    if scenario.scenes.is_empty() {
        return Err("Gemini returned an empty script".into());
    }
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
pub fn press_action(targets: State<'_, LaunchedTarget>, target: String) -> Result<(), String> {
    engine::press(&targets, &Selector::Label(target))
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
