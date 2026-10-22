//! The current script: drafted by the AI from the running target's page, read out by the pill,
//! and its actions run against the same page when narration reaches their word.

use crate::ai::{self, AiProvider, AiStatus, Gemini, Scenario};
use crate::engine;
use crate::input::Pace;
use crate::locate::Selector;
use crate::page;
use crate::target::{self, LaunchedTarget};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct CurrentScript(Mutex<Option<Scenario>>);

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

/// Clicks the control labelled `target` with the real cursor (move, hover-verify, click, ripple).
#[tauri::command(async)]
pub fn run_action(targets: State<'_, LaunchedTarget>, target: String) -> Result<(), String> {
    engine::click(&targets, &Selector::Label(target), Pace::Natural).map(|_| ())
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
