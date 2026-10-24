//! Teach mode (PROJECT_OUTLINE §2): the presenter clicks through the workflow once by hand and
//! Scenra records each click as a step.
//!
//! A capture-phase `pointerdown` listener inside the page reads the element and the page's
//! starting state before the app's own handlers run, so a click that opens a dialog or navigates
//! still records what was clicked and what the page looked like just before. The listener is
//! re-injected into every new document and reports through a CDP binding.
//!
//! Slice 1 records clicks only. Clicks into fields are flagged and never read: no value, no
//! placeholder, nothing typed. Flagged steps are kept so the presenter sees them, but they are
//! never offered to the AI or replayed.

use crate::cdp::Session;
use crate::page::{self, InitialState};
use crate::{hooks, locate, setup, take, target};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const BINDING: &str = "__scenraTeach";
const POLL: Duration = Duration::from_millis(200);
/// How often the recorder checks that the walkthrough is still in one tab.
const TAB_CHECK_EVERY: u32 = 5;

/// The in-page listener. `{state}` is [`page::INITIAL_STATE_FN`]; the label and uniqueness rules
/// match `page::check_control` and the label locator, so a clean step can be replayed as is.
fn listener_js() -> String {
    format!(
        r#"(() => {{
  if (window.__scenraTeachOff) return true;
  const state = {state};
  const clean = s => (s || '').replace(/\s+/g, ' ').trim();
  const visible = e => e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden';
  const CONTROLS = 'a[href],button,[role=button]';
  const FIELDS = 'input,textarea,select,[contenteditable=""],[contenteditable=true]';
  const onDown = ev => {{
    if (ev.button !== 0 || !ev.isTrusted || typeof window.{BINDING} !== 'function') return;
    const hit = ev.target instanceof Element ? ev.target : null;
    const field = hit && hit.closest(FIELDS);
    const control = !field && hit && hit.closest(CONTROLS);
    let label = '', testId = null, role = null, unique = false, disabled = false;
    if (control) {{
      label = clean(control.innerText || control.getAttribute('aria-label')).slice(0, 120);
      testId = control.getAttribute('data-testid');
      role = control.tagName === 'A' ? 'link' : 'button';
      disabled = !!control.disabled || control.getAttribute('aria-disabled') === 'true';
      const want = label.toLowerCase();
      const text = e => clean(e.innerText || e.getAttribute('aria-label') || e.placeholder).toLowerCase();
      unique = !!want && [...document.querySelectorAll(CONTROLS + ',' + FIELDS)]
        .filter(e => visible(e) && text(e) === want).length === 1;
    }}
    window.{BINDING}(JSON.stringify({{
      at: performance.timeOrigin + ev.timeStamp,
      kind: field ? 'field' : control ? 'control' : 'other',
      label, testId, role, unique, disabled, before: state(),
    }}));
  }};
  addEventListener('pointerdown', onDown, true);
  window.__scenraTeachOff = () => {{ removeEventListener('pointerdown', onDown, true); delete window.__scenraTeachOff; }};
  return true;
}})()"#,
        state = page::INITIAL_STATE_FN
    )
}

/// What the listener sends for one click.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawClick {
    /// Milliseconds since the Unix epoch, from the page's clock.
    at: f64,
    kind: String,
    label: String,
    test_id: Option<String>,
    role: Option<String>,
    unique: bool,
    disabled: bool,
    before: InitialState,
}

/// Why a step can't be replayed as recorded. A flagged step is shown but never replayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepFlag {
    /// A click into a text field; typing isn't supported yet.
    Field,
    /// Not on a link or button.
    NotControl,
    /// The control has no visible label or aria-label.
    Unlabelled,
    /// Another visible control has the same label.
    Ambiguous,
    Disabled,
}

impl StepFlag {
    pub fn describe(self) -> &'static str {
        match self {
            StepFlag::Field => "a click into a field; typing isn't supported yet",
            StepFlag::NotControl => "not on a link or button",
            StepFlag::Unlabelled => "the control has no label",
            StepFlag::Ambiguous => "another control has the same label",
            StepFlag::Disabled => "the control was disabled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachStep {
    pub index: usize,
    /// Milliseconds after teaching started.
    pub at_ms: u64,
    pub label: String,
    pub test_id: Option<String>,
    pub role: Option<String>,
    /// The page just before this click, in the form scene-start checks compare against.
    pub before: InitialState,
    pub flags: Vec<StepFlag>,
}

impl TeachStep {
    pub fn usable(&self) -> bool {
        self.flags.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachSession {
    /// The page when teaching started.
    pub start: InitialState,
    /// The page when teaching stopped, for a scene that comes after the last click.
    pub end: InitialState,
    pub steps: Vec<TeachStep>,
    pub warnings: Vec<String>,
}

fn step_from(raw: RawClick, started_ms: f64) -> TeachStep {
    let mut flags = Vec::new();
    match raw.kind.as_str() {
        "field" => flags.push(StepFlag::Field),
        "control" => {
            if raw.label.is_empty() {
                flags.push(StepFlag::Unlabelled);
            } else if !raw.unique {
                flags.push(StepFlag::Ambiguous);
            }
            if raw.disabled {
                flags.push(StepFlag::Disabled);
            }
        }
        _ => flags.push(StepFlag::NotControl),
    }
    let field = flags.contains(&StepFlag::Field);
    TeachStep {
        index: 0,
        at_ms: (raw.at - started_ms).max(0.0) as u64,
        // Belt and braces: nothing describing a field is kept, even if the page sent it.
        label: if field { String::new() } else { raw.label },
        test_id: if field { None } else { raw.test_id },
        role: if field { None } else { raw.role },
        before: raw.before,
        flags,
    }
}

/// Orders steps by when they happened on the page and numbers them. Binding calls can overtake
/// each other in a quick double click.
fn finish(mut steps: Vec<TeachStep>) -> Vec<TeachStep> {
    steps.sort_by_key(|s| s.at_ms);
    for (index, step) in steps.iter_mut().enumerate() {
        step.index = index;
    }
    steps
}

fn epoch_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64() * 1000.0
}

/// How long to wait for a page after its debugging target closed (a cross-site navigation can
/// replace the target; a reload keeps it).
const REATTACH_TIMEOUT: Duration = Duration::from_secs(10);
const REATTACHED: &str = "the page was replaced and Scenra re-attached; a click made at that moment may be missing";

/// Connects to the target's page and installs the binding and listener, for this document and
/// every new one. Returns the session and the new-document script's id.
fn attach(port: u16) -> Result<(Session, String), String> {
    let mut session = page::page_session(port)?;
    // Binding calls are only reported to a session with the Runtime domain enabled.
    session.call("Runtime.enable", json!({}))?;
    // New-document scripts don't run in WebView2 until the Page domain is enabled.
    session.call("Page.enable", json!({}))?;
    session.call("Runtime.addBinding", json!({ "name": BINDING }))?;
    let script = listener_js();
    let added = session.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": script }))?;
    let script_id = added["identifier"].as_str().ok_or("couldn't add the teach listener")?.to_owned();
    if session.evaluate(&script)? != json!(true) {
        return Err("couldn't add the teach listener to the page".into());
    }
    Ok((session, script_id))
}

/// The click a binding event carries, if the event is one of ours.
fn click_in(event: &serde_json::Value) -> Option<Result<RawClick, String>> {
    (event["method"] == "Runtime.bindingCalled" && event["params"]["name"] == BINDING).then(|| {
        serde_json::from_str(event["params"]["payload"].as_str().unwrap_or_default())
            .map_err(|e| format!("ignored a click Scenra couldn't read: {e}"))
    })
}

/// The recorder thread's state, handed back on Stop.
struct Attached {
    session: Session,
    script_id: String,
    steps: Vec<TeachStep>,
    warnings: Vec<String>,
}

/// A running walkthrough capture against one CDP page.
pub struct Recorder {
    port: u16,
    stop: Arc<AtomicBool>,
    start: InitialState,
    started_ms: f64,
    thread: JoinHandle<Result<Attached, String>>,
}

impl Recorder {
    /// Injects the listener and starts collecting clicks. `on_step` runs on the recorder thread
    /// for each click as it arrives (provisional index: arrival order).
    pub fn start(port: u16, on_step: impl Fn(&TeachStep) + Send + 'static) -> Result<Self, String> {
        setup::single_page(port).map_err(|_| "Teach needs one target tab. Close extra tabs and retry".to_owned())?;
        page::wait_until_settled(port, Duration::from_secs(15))?;
        let start = page::initial_state(&mut page::page_session(port)?)?;
        let started_ms = epoch_ms();
        let (session, script_id) = attach(port)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("scenra-teach".into())
            .spawn(move || {
                let mut run = Attached { session, script_id, steps: Vec::new(), warnings: Vec::new() };
                let mut polls = 0u32;
                while !stopping.load(Ordering::SeqCst) {
                    match run.session.next_event(POLL)? {
                        Some(event) if event["method"] == "Inspector.detached" => {
                            let deadline = std::time::Instant::now() + REATTACH_TIMEOUT;
                            let (session, script_id) = loop {
                                match attach(port) {
                                    Ok(attached) => break attached,
                                    Err(_) if std::time::Instant::now() < deadline && !stopping.load(Ordering::SeqCst) => {
                                        std::thread::sleep(POLL);
                                    }
                                    Err(e) => return Err(format!("the target page closed during teaching: {e}")),
                                }
                            };
                            // Replacing the session drops the dead page's socket.
                            run.session = session;
                            run.script_id = script_id;
                            if !run.warnings.iter().any(|w| w == REATTACHED) {
                                run.warnings.push(REATTACHED.into());
                            }
                        }
                        Some(event) => match click_in(&event) {
                            Some(Ok(raw)) => {
                                let mut step = step_from(raw, started_ms);
                                step.index = run.steps.len();
                                on_step(&step);
                                run.steps.push(step);
                            }
                            Some(Err(e)) => run.warnings.push(e),
                            None => {}
                        },
                        None => {}
                    }
                    polls += 1;
                    if polls.is_multiple_of(TAB_CHECK_EVERY) && setup::single_page(port).is_err() {
                        const SECOND_TAB: &str = "a second tab opened; clicks in it weren't recorded";
                        if !run.warnings.iter().any(|w| w == SECOND_TAB) {
                            run.warnings.push(SECOND_TAB.into());
                        }
                    }
                }
                Ok(run)
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { port, stop, start, started_ms, thread })
    }

    /// Stops collecting, removes the listener and returns the walkthrough in page order.
    pub fn stop(self) -> Result<TeachSession, String> {
        self.stop.store(true, Ordering::SeqCst);
        let Attached { mut session, script_id, mut steps, mut warnings } =
            self.thread.join().map_err(|_| "the teach recorder crashed")??;
        // Clicks already queued when Stop was pressed still count.
        while let Some(event) = session.next_event(Duration::from_millis(50))? {
            match click_in(&event) {
                Some(Ok(raw)) => steps.push(step_from(raw, self.started_ms)),
                Some(Err(e)) => warnings.push(e),
                None => {}
            }
        }
        let _ = session.call("Page.removeScriptToEvaluateOnNewDocument", json!({ "identifier": script_id }));
        let _ = session.evaluate("window.__scenraTeachOff && window.__scenraTeachOff(); true");
        let _ = session.call("Runtime.removeBinding", json!({ "name": BINDING }));
        let end = page::page_session(self.port).and_then(|mut page| page::initial_state(&mut page)).unwrap_or_else(|e| {
            warnings.push(format!("couldn't read the final page: {e}"));
            self.start.clone()
        });
        Ok(TeachSession { start: self.start, end, steps: finish(steps), warnings })
    }
}

/// The walkthrough being taught, and the last one finished.
#[derive(Default)]
pub struct TeachState {
    active: Mutex<Option<Recorder>>,
    last: Mutex<Option<TeachSession>>,
}

impl TeachState {
    /// Playback, setup and drafting must not run while the presenter is teaching.
    pub(crate) fn ensure_idle(&self) -> Result<(), String> {
        if self.active.lock().map_err(|e| e.to_string())?.is_some() {
            return Err("Teaching is in progress; stop it first".into());
        }
        Ok(())
    }

    pub(crate) fn last(&self) -> Result<Option<TeachSession>, String> {
        Ok(self.last.lock().map_err(|e| e.to_string())?.clone())
    }
}

/// Starts recording the presenter's clicks in the running target. Each click is emitted as
/// `teach-step`.
#[tauri::command(async)]
pub fn start_teach(
    app: AppHandle, targets: State<'_, target::LaunchedTarget>, teach: State<'_, TeachState>,
    setup: State<'_, setup::SetupState>, take: State<'_, take::CurrentTake>,
) -> Result<(), String> {
    setup.ensure_idle()?;
    if hooks::playback_active() { return Err("Stop playback before teaching".into()); }
    if take.is_recording() { return Err("Stop the current take before teaching".into()); }
    let mut active = teach.active.lock().map_err(|e| e.to_string())?;
    if active.is_some() { return Err("Teaching is already in progress".into()); }
    let (pid, port) = target::running(&targets)?;
    locate::restore_target(pid)?;
    let recorder = Recorder::start(port, move |step| { let _ = app.emit("teach-step", step); })?;
    *active = Some(recorder);
    Ok(())
}

/// Stops teaching and returns the walkthrough, which becomes the last taught session.
#[tauri::command(async)]
pub fn stop_teach(teach: State<'_, TeachState>) -> Result<TeachSession, String> {
    let recorder = teach.active.lock().map_err(|e| e.to_string())?.take().ok_or("Teaching isn't running")?;
    let session = recorder.stop()?;
    *teach.last.lock().map_err(|e| e.to_string())? = Some(session.clone());
    Ok(session)
}

#[tauri::command]
pub fn last_teach(teach: State<'_, TeachState>) -> Result<Option<TeachSession>, String> {
    teach.last()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(url: &str) -> InitialState {
        InitialState { url: url.into(), headings: vec!["Recordings".into()], dialogs: vec![] }
    }

    fn raw(at: f64, kind: &str, label: &str, unique: bool) -> RawClick {
        RawClick {
            at, kind: kind.into(), label: label.into(), test_id: Some("t".into()), role: Some("button".into()),
            unique, disabled: false, before: state("http://localhost/"),
        }
    }

    #[test]
    fn clean_controls_are_usable_and_others_are_flagged() {
        let clean = step_from(raw(1_500.0, "control", "New Recording", true), 1_000.0);
        assert!(clean.usable());
        assert_eq!(clean.at_ms, 500);
        assert_eq!(step_from(raw(0.0, "control", "Save", false), 0.0).flags, [StepFlag::Ambiguous]);
        assert_eq!(step_from(raw(0.0, "control", "", false), 0.0).flags, [StepFlag::Unlabelled]);
        assert_eq!(step_from(raw(0.0, "other", "", false), 0.0).flags, [StepFlag::NotControl]);
        let mut disabled = raw(0.0, "control", "Save", true);
        disabled.disabled = true;
        assert_eq!(step_from(disabled, 0.0).flags, [StepFlag::Disabled]);
    }

    #[test]
    fn field_clicks_keep_nothing_that_describes_the_field() {
        let step = step_from(raw(0.0, "field", "hunter2", true), 0.0);
        assert_eq!(step.flags, [StepFlag::Field]);
        assert!(step.label.is_empty() && step.test_id.is_none() && step.role.is_none());
    }

    #[test]
    fn steps_are_ordered_by_page_time_and_renumbered() {
        let steps = finish(vec![
            step_from(raw(300.0, "control", "Settings", true), 0.0),
            step_from(raw(100.0, "control", "New Recording", true), 0.0),
        ]);
        assert_eq!(steps.iter().map(|s| (s.index, s.label.as_str())).collect::<Vec<_>>(), [(0, "New Recording"), (1, "Settings")]);
    }

    /// Trusted CDP click on a fixture control, like a presenter's mouse.
    fn press(port: u16, test_id: &str) -> Result<(), String> {
        let mut page = page::page_session(port)?;
        let at = page.evaluate(&format!(
            "(() => {{ const r = document.querySelector('[data-testid={test_id}]').getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }})()"
        ))?;
        for kind in ["mousePressed", "mouseReleased"] {
            page.call("Input.dispatchMouseEvent", json!({ "type": kind, "x": at[0], "y": at[1], "button": "left", "clickCount": 1 }))?;
        }
        Ok(())
    }

    #[test]
    #[ignore = "launches the demo target window"]
    fn records_clicks_across_dialogs_fields_and_navigation() {
        let user_data = std::env::temp_dir().join("scenra-test-teach");
        let (mut child, port) = crate::target::spawn_demo_target(&user_data).unwrap();
        let result = (|| -> Result<(), String> {
            page::wait_until_settled(port, Duration::from_secs(15))?;
            let (sent, received) = std::sync::mpsc::channel();
            let recorder = Recorder::start(port, move |step| { let _ = sent.send(step.label.clone()); })?;
            let next = |what: &str| received.recv_timeout(Duration::from_secs(5)).map_err(|_| format!("the click on {what} wasn't recorded"));
            press(port, "create-recording")?; next("create-recording")?;
            press(port, "title-input")?; next("title-input")?;
            page::page_session(port)?.call("Input.insertText", json!({ "text": "hunter2" }))?;
            press(port, "open-settings")?; next("open-settings")?;
            std::thread::sleep(Duration::from_millis(300));
            press(port, "settings-confirm")?; next("settings-confirm")?;
            // A new document gets the listener again.
            page::page_session(port)?.call("Page.reload", json!({}))?;
            std::thread::sleep(Duration::from_millis(500));
            page::wait_until_settled(port, Duration::from_secs(15))?;
            press(port, "create-recording")?; next("create-recording")?;
            let session = recorder.stop()?;

            let summary: Vec<_> = session.steps.iter().map(|s| (s.label.as_str(), s.flags.clone())).collect();
            let expected = [("New Recording", vec![]), ("", vec![StepFlag::Field]), ("Settings", vec![]), ("Confirm", vec![]), ("New Recording", vec![])];
            if summary != expected { return Err(format!("steps: {summary:?}")); }
            if session.steps[0].before != session.start { return Err("the first step's state isn't the starting page".into()); }
            if session.steps[3].before.dialogs.is_empty() { return Err("Confirm's state should include the open Settings dialog".into()); }
            if !session.steps[2].before.dialogs.is_empty() { return Err("Settings' state shouldn't have a dialog yet".into()); }
            if !session.steps.windows(2).all(|w| w[0].at_ms <= w[1].at_ms) { return Err("steps out of order".into()); }
            if serde_json::to_string(&session).map_err(|e| e.to_string())?.contains("hunter2") {
                return Err("typed text leaked into the teach session".into());
            }
            let mut page = page::page_session(port)?;
            if page.evaluate("typeof window.__scenraTeachOff")? != json!("undefined") {
                return Err("the listener is still installed after Stop".into());
            }
            Ok(())
        })();
        let _ = child.kill(); let _ = child.wait();
        result.unwrap();
    }

    #[test]
    fn listener_shares_the_initial_state_function() {
        assert!(listener_js().contains(page::INITIAL_STATE_FN));
        assert!(!listener_js().contains(".value"), "the listener must never read field values");
    }
}
