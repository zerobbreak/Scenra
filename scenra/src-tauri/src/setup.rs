//! Deterministic setup from the approved script, not a new AI recovery plan.
use crate::{ai::{Action, Scenario}, capture, cdp, hooks, locate, page, script, take, target};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

const MAX_ACTIONS: usize = 50;
const SETUP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct SetupState { running: AtomicBool, cancelled_through: AtomicU64 }
impl SetupState {
    pub(crate) fn ensure_idle(&self) -> Result<(), String> {
        if self.running.load(Ordering::SeqCst) { Err("Scene setup is running; wait or press Stop".into()) } else { Ok(()) }
    }
    fn cancel(&self, through: u64) {
        self.cancelled_through.fetch_max(through, Ordering::SeqCst);
    }
    fn is_cancelled(&self, request_id: u64) -> bool {
        request_id <= self.cancelled_through.load(Ordering::SeqCst)
    }
    fn claim(&self) -> Result<SetupRun<'_>, String> {
        self.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "Another scene setup is already running")?;
        Ok(SetupRun(self))
    }
}
struct SetupRun<'a>(&'a SetupState);
impl Drop for SetupRun<'_> {
    fn drop(&mut self) { self.0.running.store(false, Ordering::SeqCst); }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest { scene: usize, scenario: Scenario, request_id: u64 }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupResult { pub actions_run: usize }

#[tauri::command]
pub fn cancel_scene_setup(setup: State<'_, SetupState>) {
    setup.cancel(epoch_ms());
}

#[tauri::command(async)]
pub fn prepare_scene(
    app: AppHandle, targets: State<'_, target::LaunchedTarget>,
    script: State<'_, script::CurrentScript>, initial: State<'_, script::ScriptInitialState>,
    take: State<'_, take::CurrentTake>, setup: State<'_, SetupState>, request: SetupRequest,
) -> Result<SetupResult, String> {
    let _run = setup.claim()?;
    if request.request_id == 0 || request.request_id > epoch_ms().saturating_add(1000) {
        return Err("Invalid setup request; retry from the prompter".into());
    }
    if hooks::playback_active() { return Err("Pause playback before preparing a scene".into()); }
    let scenario = request.scenario;
    if script.0.lock().map_err(|e| e.to_string())?.as_ref() != Some(&scenario) {
        return Err("The script changed; reopen the prompter before preparing this scene".into());
    }
    let scene = request.scene;
    let states = initial.0.lock().map_err(|e| e.to_string())?.clone();
    let (pid, port) = target::running(&targets)?;
    let _input = hooks::arm();
    let checkpoint = || -> Result<(), String> {
        if setup.is_cancelled(request.request_id) || app.get_webview_window(capture::TELEPROMPTER).is_none() {
            return Err("Scene setup was cancelled".into());
        }
        if hooks::stopped() || hooks::foreign_seen() || hooks::playback_active() {
            return Err("Scene setup was interrupted; prepare the page manually or retry".into());
        }
        if script.0.lock().map_err(|e| e.to_string())?.as_ref() != Some(&scenario)
            || initial.0.lock().map_err(|e| e.to_string())?.as_slice() != states.as_slice()
        { return Err("The approved script or starting states changed; setup stopped".into()); }
        Ok(())
    };
    checkpoint()?;
    take.while_idle(|| {
        checkpoint()?;
        locate::restore_target(pid)?;
        replay_to_start(port, &scenario, &states, scene, checkpoint)
    })
}


fn epoch_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn single_page(port: u16) -> Result<(), String> {
    if cdp::targets(port)?.iter().filter(|t| t.kind == "page").count() != 1 {
        return Err("Replay setup needs one target tab. Close extra tabs and prepare the page manually".into());
    }
    Ok(())
}

fn ordered_actions(scene: &crate::ai::Scene) -> Vec<&Action> {
    scene.lines.iter().flat_map(|line| {
        let mut actions: Vec<_> = line.actions.iter().collect();
        actions.sort_by_key(|a| a.word);
        actions
    }).collect()
}

fn check_scene(port: u16, scenario: &Scenario, states: &[page::InitialState], scene: usize) -> Result<(), String> {
    let selected = scenario.scenes.get(scene).ok_or("Unknown scene")?;
    let expected = states.get(scene).ok_or("This scene has no approved starting state")?;
    let mut page = page::page_session(port)?;
    page::check_initial(expected, &page::initial_state(&mut page)?)?;
    if let Some(action) = ordered_actions(selected).first() { page::check_control(&mut page, &action.target)?; }
    Ok(())
}

/// Shared with the live regression; checkpoint is called between every navigation/action.
pub(crate) fn replay_to_start(
    port: u16, scenario: &Scenario, states: &[page::InitialState], scene: usize,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<SetupResult, String> {
    if scene >= scenario.scenes.len() || states.len() != scenario.scenes.len() {
        return Err("Unknown scene or missing approved starting state".into());
    }
    let count: usize = scenario.scenes[..scene].iter().map(|s| ordered_actions(s).len()).sum();
    if count > MAX_ACTIONS { return Err("Setup exceeds 50 approved clicks; prepare this scene manually".into()); }
    let deadline = Instant::now() + SETUP_TIMEOUT;
    let check = || -> Result<(), String> {
        checkpoint()?;
        if Instant::now() >= deadline { return Err("Scene setup exceeded 60 seconds".into()); }
        Ok(())
    };
    let remaining = || deadline.saturating_duration_since(Instant::now()).min(Duration::from_secs(15));
    check()?;
    single_page(port)?;
    page::restore_url(port, &states[0].url, remaining(), &check)?;
    check()?;
    if check_scene(port, scenario, states, scene).is_ok() { return Ok(SetupResult { actions_run: 0 }); }
    let mut actions_run = 0;
    for previous in 0..scene {
        check()?;
        check_scene(port, scenario, states, previous)
            .map_err(|e| format!("Cannot replay scene {}: {e}", previous + 1))?;
        for action in ordered_actions(&scenario.scenes[previous]) {
            check()?;
            single_page(port)?;
            page::setup_click(&mut page::page_session(port)?, &action.target, &check)
                .map_err(|e| format!("Setup stopped in scene {}: {e}", previous + 1))?;
            actions_run += 1;
            page::wait_until_settled(port, remaining())?;
            check()?;
            single_page(port)?;
        }
        if check_scene(port, scenario, states, scene).is_ok() { return Ok(SetupResult { actions_run }); }
    }
    Err("Earlier approved scenes did not reach this starting state. Prepare the page manually and retry".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scenario_with_actions(count: usize) -> (Scenario, Vec<page::InitialState>) {
        let action = Action { word: 0, target: "Open".into(), label: "Open".into() };
        let scene = crate::ai::Scene { title: "Scene".into(), lines: vec![crate::ai::Line { text: "Open".into(), actions: vec![action; count] }] };
        let state = page::InitialState { url: "http://localhost/".into(), headings: vec![], dialogs: vec![] };
        (Scenario { title: "Test".into(), scenes: vec![scene.clone(), scene], warnings: vec![] }, vec![state.clone(), state])
    }
    #[test]
    fn cancellation_and_action_limit_are_checked_before_touching_a_page() {
        let (scenario, states) = scenario_with_actions(1);
        let cancelled = replay_to_start(0, &scenario, &states, 1, || Err("Cancelled by test".into()));
        assert!(cancelled.err().unwrap().contains("Cancelled by test"));
        let (scenario, states) = scenario_with_actions(51);
        let oversized = replay_to_start(0, &scenario, &states, 1, || Ok(()));
        assert!(oversized.err().unwrap().contains("50 approved clicks"));
        assert!(replay_to_start(0, &scenario, &states, 99, || Ok(())).is_err());
    }

    #[test]
    fn stop_also_cancels_requests_that_have_not_started_yet() {
        let state = SetupState::default();
        state.cancel(20);
        let run = state.claim().unwrap();
        assert!(state.is_cancelled(10));
        assert!(state.is_cancelled(20));
        assert!(!state.is_cancelled(21));
        drop(run);
    }

    #[test]
    fn setup_is_exclusive_and_releases_its_busy_state() {
        let state = SetupState::default();
        let run = state.claim().unwrap();
        assert!(state.claim().is_err());
        assert!(state.ensure_idle().is_err());
        drop(run);
        assert!(state.ensure_idle().is_ok());
    }
    #[test]
    fn clicks_follow_narration_order_inside_each_line() {
        let scene = crate::ai::Scene { title: "Setup".into(), lines: vec![crate::ai::Line {
            text: "a b c".into(), actions: vec![
                Action { word: 2, target: "Second".into(), label: "".into() },
                Action { word: 0, target: "First".into(), label: "".into() },
            ],
        }] };
        assert_eq!(ordered_actions(&scene).iter().map(|a| a.target.as_str()).collect::<Vec<_>>(), vec!["First", "Second"]);
    }
}
