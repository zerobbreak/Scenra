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
use crate::teach::{TeachSession, TeachState};
use serde::{Deserialize, Serialize};
use crate::target::{self, LaunchedTarget};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct CurrentScript(pub(crate) Mutex<Option<Scenario>>);

#[derive(Default)]
pub struct ScriptInitialState(pub(crate) Mutex<Vec<page::InitialState>>);

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Rehearse,
    Record,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub(crate) mode: Mode,
    /// Timestamp; recorded sessions keep their scene takes in a folder of this name.
    pub(crate) id: String,
    /// A retake from the editor: record only this scene (0-based), then finish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scene: Option<usize>,
    /// Takes already in the session folder, per scene, so a retake never reuses a folder.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) attempts: std::collections::BTreeMap<usize, u32>,
}

#[derive(Default)]
pub struct CurrentSession(pub(crate) Mutex<Option<Session>>);

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
    setup: State<'_, crate::setup::SetupState>,
    teach: State<'_, TeachState>,
) -> Result<Scenario, String> {
    setup.ensure_idle()?;
    teach.ensure_idle()?;
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

/// A script narrating a taught walkthrough, and each scene's starting state: the page just before
/// its first taught click, or the page as teaching ended for a scene after the last click.
pub(crate) fn draft_from_session(
    provider: &impl AiProvider, session: &TeachSession, brief: &str,
) -> Result<(Scenario, Vec<page::InitialState>), String> {
    let aligned = provider.align_script(&session.steps, brief)?;
    if aligned.scenario.scenes.is_empty() {
        return Err("Gemini returned an empty script".into());
    }
    let states = aligned
        .scene_starts
        .iter()
        .map(|start| match start {
            Some(index) => session.steps.get(*index).map(|step| step.before.clone()).ok_or("a scene starts at an unknown step"),
            None => Ok(session.end.clone()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((aligned.scenario, states))
}

/// Asks Gemini to narrate the last taught walkthrough and shares the script with the pill. Each
/// scene's approved starting state comes from the walkthrough, so recorded scenes and Replay setup
/// check against the page as it really was at that point.
#[tauri::command(async)]
pub fn draft_from_teach(
    app: AppHandle,
    script: State<'_, CurrentScript>,
    initial: State<'_, ScriptInitialState>,
    setup: State<'_, crate::setup::SetupState>,
    teach: State<'_, TeachState>,
    brief: String,
) -> Result<Scenario, String> {
    setup.ensure_idle()?;
    teach.ensure_idle()?;
    let session = teach.last()?.ok_or("Teach a walkthrough first")?;
    let (scenario, states) = draft_from_session(&Gemini::from_store()?, &session, brief.trim())?;
    *initial.0.lock().map_err(err)? = states;
    *script.0.lock().map_err(err)? = Some(scenario.clone());
    app.emit("script-changed", &scenario).map_err(err)?;
    Ok(scenario)
}

/// Installs a script written or edited by hand. Every scene starts from the app as it is now;
/// use “Use current page” in the prompter to approve a different start for later scenes.
#[tauri::command(async)]
pub fn set_script(
    app: AppHandle, targets: State<'_, LaunchedTarget>, script: State<'_, CurrentScript>,
    initial: State<'_, ScriptInitialState>, setup: State<'_, crate::setup::SetupState>,
    teach: State<'_, TeachState>, mut scenario: Scenario,
) -> Result<Scenario, String> {
    setup.ensure_idle()?;
    teach.ensure_idle()?;
    validate(&scenario)?;
    // A code step without a label gets one the pill can show ("Highlight take.rs 195–200").
    for action in scenario.scenes.iter_mut().flat_map(|s| s.lines.iter_mut()).flat_map(|l| l.actions.iter_mut()) {
        if action.label.trim().is_empty() {
            action.label = match &action.step {
                Some(step) => step.describe(),
                None => format!("Click {}", action.target),
            };
        }
    }
    let (pid, port) = target::running(&targets)?;
    crate::locate::restore_target(pid)?;
    let state = page::initial_state(&mut page::page_session(port)?)?;
    *initial.0.lock().map_err(err)? = vec![state; scenario.scenes.len()];
    *script.0.lock().map_err(err)? = Some(scenario.clone());
    app.emit("script-changed", &scenario).map_err(err)?;
    Ok(scenario)
}

/// A hand-written script must have narration, and every action a word to fire on and something to do.
fn validate(scenario: &Scenario) -> Result<(), String> {
    if scenario.scenes.is_empty() {
        return Err("Add at least one scene".into());
    }
    for (s, scene) in scenario.scenes.iter().enumerate() {
        if scene.lines.iter().all(|l| l.text.trim().is_empty()) {
            return Err(format!("Scene {} has no narration", s + 1));
        }
        for line in &scene.lines {
            let words = line.text.split_whitespace().count();
            for action in &line.actions {
                if action.word >= words {
                    return Err(format!("Scene {}: “{}” fires on a word that isn't in its line", s + 1, action.label));
                }
                if action.step.is_none() && action.target.trim().is_empty() {
                    return Err(format!("Scene {}: “{}” has nothing to click", s + 1, action.label));
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn current_script(script: State<'_, CurrentScript>) -> Result<Option<Scenario>, String> {
    Ok(script.0.lock().map_err(err)?.clone())
}

/// Starts a pass through the script and tells the pill, which starts over in that mode.
#[tauri::command]
pub fn start_session(
    app: AppHandle, session: State<'_, CurrentSession>, setup: State<'_, crate::setup::SetupState>,
    teach: State<'_, TeachState>, script: State<'_, CurrentScript>, initial: State<'_, ScriptInitialState>, mode: Mode,
) -> Result<Session, String> {
    setup.ensure_idle()?;
    teach.ensure_idle()?;
    let started = Session { mode, id: take::stamp(), scene: None, attempts: Default::default() };
    if matches!(started.mode, Mode::Record) {
        // Saved up front so the editor can open the session even if Scenra closes mid-way.
        let scenario = script.0.lock().map_err(err)?.clone();
        let title = scenario.as_ref().map(|s| s.title.clone()).unwrap_or_default();
        let info = crate::session::SessionInfo { id: started.id.clone(), title };
        let dir = crate::session::root(&app).join(&started.id);
        crate::session::save(&dir, &info, scenario.as_ref())?;
        // Each scene's starting page, so a retake from the editor can check and restore it.
        crate::session::write_json(&dir.join(crate::session::INITIAL_FILE), &*initial.0.lock().map_err(err)?)?;
    }
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
pub fn approach_action(
    targets: State<'_, LaunchedTarget>, setup: State<'_, crate::setup::SetupState>, take: State<'_, take::CurrentTake>, action: crate::ai::Action,
) -> Result<(), String> {
    setup.ensure_idle()?;
    match &action.step {
        Some(step) => take::logged(&take, "approach", &action.label, || crate::steps::approach(&targets, step, Pace::Natural)),
        None => take::logged(&take, "approach", &action.target, || {
            engine::approach(&targets, &Selector::Label(action.target.clone()), Pace::Natural).map(|_| ())
        }),
    }
}

/// Runs the action on its trigger word: clicks the control labelled `target` (re-aiming first if
/// needed), or runs its code step.
#[tauri::command(async)]
pub fn press_action(
    targets: State<'_, LaunchedTarget>, setup: State<'_, crate::setup::SetupState>, take: State<'_, take::CurrentTake>,
    action: crate::ai::Action, expires_at: u64,
) -> Result<(), String> {
    setup.ensure_idle()?;
    let expiry = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_millis(expires_at))
        .ok_or("Invalid action cue expiry")?;
    let remaining = expiry.duration_since(std::time::SystemTime::now())
        .map_err(|_| "The action missed its cue; retry the scene")?;
    if let Some(step) = &action.step {
        if !crate::hooks::playback_active() || crate::hooks::foreign_seen() || crate::hooks::stopped() {
            return Err("Playback interrupted; retry the scene.".into());
        }
        // The press is the whole step (a scroll or highlight takes seconds); it's logged as one
        // span so the editor never cuts through it.
        return take::logged(&take, "press", &action.label, || crate::steps::run(&targets, step, Pace::Natural));
    }
    let remaining = remaining.min(std::time::Duration::from_millis(500));
    let target = action.target;
    take::logged(&take, "press", &target, || {
        engine::press_before(&targets, &Selector::Label(target.clone()), Some(std::time::Instant::now() + remaining))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teach::TeachStep;

    fn state(heading: &str) -> page::InitialState {
        page::InitialState { url: "http://localhost/".into(), headings: vec![heading.into()], dialogs: vec![], code: None }
    }

    fn session(labels: &[&str]) -> TeachSession {
        TeachSession {
            start: state("start"),
            end: state("end"),
            steps: labels.iter().enumerate().map(|(index, label)| TeachStep {
                index, at_ms: 0, label: (*label).into(), test_id: None, role: None,
                before: state(&format!("before {label}")), flags: vec![],
            }).collect(),
            warnings: vec![],
        }
    }

    #[test]
    fn scenes_start_where_their_first_taught_click_was() {
        let provider = ai::Scripted(vec![
            ("Open", vec![("Start a new recording.", vec![("new", "s0")])]),
            ("Settings", vec![("Open settings and confirm.", vec![("settings", "s1"), ("confirm", "s2")])]),
            ("Done", vec![("That's the tour.", vec![])]),
        ]);
        let (scenario, states) = draft_from_session(&provider, &session(&["New Recording", "Settings", "Confirm"]), "").unwrap();
        assert_eq!(scenario.scenes.len(), 3);
        let headings: Vec<&str> = states.iter().map(|s| s.headings[0].as_str()).collect();
        assert_eq!(headings, ["before New Recording", "before Settings", "end"]);
    }

    #[test]
    fn a_script_that_skips_a_taught_step_is_refused() {
        let provider = ai::Scripted(vec![("Open", vec![("Start a new recording.", vec![("new", "s0")])])]);
        let refused = draft_from_session(&provider, &session(&["New Recording", "Settings"]), "").unwrap_err();
        assert!(refused.contains("isn't narrated"), "{refused}");
    }

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

/// Fail closed before recording starts. Current scripts describe independent scenes on the draft page.
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
    if let Some(action) = scene.lines.iter().flat_map(|l| &l.actions).next().filter(|a| a.step.is_none()) {
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
    let (pid, port) = target::running(&targets)?;
    crate::locate::restore_target(pid)?;
    if let Some(code) = &expected.code {
        return crate::steps::restore_state(&targets, code);
    }
    page::restore_url(port, &expected.url, std::time::Duration::from_secs(15), &|| Ok(()))
}

/// Presenter explicitly approves a manually prepared page as this scene's starting state.
#[tauri::command(async)]
pub fn approve_scene_start(
    targets: State<'_, LaunchedTarget>, script: State<'_, CurrentScript>,
    initial: State<'_, ScriptInitialState>, scene: usize,
) -> Result<(), String> {
    let scenario = script.0.lock().map_err(err)?.clone().ok_or("Draft a script first")?;
    let selected = scenario.scenes.get(scene).ok_or("Unknown scene")?;
    let (pid, port) = target::running(&targets)?;
    crate::locate::restore_target(pid)?;
    page::wait_until_settled(port, std::time::Duration::from_secs(15))?;
    let mut page = page::page_session(port)?;
    if let Some(action) = selected.lines.iter().flat_map(|l| &l.actions).next().filter(|a| a.step.is_none()) {
        page::check_control(&mut page, &action.target)?;
    }
    let baseline = page::initial_state(&mut page)?;
    let mut states = initial.0.lock().map_err(err)?;
    *states.get_mut(scene).ok_or("Draft a script first")? = baseline;
    Ok(())
}
