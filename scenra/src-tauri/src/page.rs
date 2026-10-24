//! Reading the attached page over CDP: the outline the AI writes a script from.
//! Clicking is the InputEngine's job (`engine.rs`), with the real cursor.

use crate::cdp::{self, Session};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// What the AI is allowed to see of a page: visible headings and interactive controls by their
/// label. No input values, no hidden text, no URLs with query strings. This is the "redacted"
/// side of the AiProvider boundary (PROJECT_OUTLINE §4).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOutline {
    pub title: String,
    pub headings: Vec<String>,
    pub controls: Vec<Control>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Control {
    /// Stable within one outline, e.g. `c3`. The AI refers to controls by id.
    pub id: String,
    /// `link` or `button`.
    pub role: String,
    pub label: String,
}

const OUTLINE_JS: &str = r#"(() => {
  const clean = (s) => (s || '').replace(/\s+/g, ' ').trim();
  const visible = (e) => e.offsetParent !== null && getComputedStyle(e).visibility !== 'hidden';
  const headings = [...document.querySelectorAll('h1,h2,h3')].filter(visible)
    .map((e) => clean(e.innerText).slice(0, 120)).filter(Boolean).slice(0, 40);
  const seen = new Set();
  const controls = [];
  for (const e of document.querySelectorAll('a[href],button,[role=button]')) {
    if (!visible(e) || e.closest('form')) continue;
    const label = clean(e.innerText || e.getAttribute('aria-label')).slice(0, 80);
    if (!label || seen.has(label.toLowerCase())) continue;
    seen.add(label.toLowerCase());
    controls.push({ id: 'c' + controls.length, role: e.tagName === 'A' ? 'link' : 'button', label });
    if (controls.length >= 60) break;
  }
  return { title: clean(document.title), headings, controls };
})()"#;

pub fn page_session(port: u16) -> Result<Session, String> {
    let page = cdp::targets(port)?
        .into_iter()
        .find(|t| t.kind == "page" && t.url.starts_with("http"))
        .ok_or("no web page is open in the target")?;
    Session::connect(&page.web_socket_debugger_url)
}

pub fn outline(session: &mut Session) -> Result<PageOutline, String> {
    serde_json::from_value(session.evaluate(OUTLINE_JS)?).map_err(|e| e.to_string())
}

/// What the page looks like right now: whether it has loaded and painted, plus a fingerprint of
/// its size and layout. Two equal fingerprints a while apart mean nothing is still moving.
/// A document swap can briefly leave no root element, which counts as not ready rather than failing.
const SNAPSHOT_JS: &str = r#"(() => {
  const painted = performance.getEntriesByType('paint').some((e) => e.name === 'first-contentful-paint');
  const hidden = document.visibilityState !== 'visible';
  const root = document.documentElement;
  const ready = document.readyState === 'complete' && !hidden
    && document.fonts.status === 'loaded' && painted && !!root;
  const layout = [innerWidth, innerHeight, root?.scrollWidth ?? 0, root?.scrollHeight ?? 0,
    document.getElementsByTagName('*').length].join(',');
  return { ready, hidden, layout };
})()"#;

#[derive(Deserialize)]
struct Snapshot {
    ready: bool,
    hidden: bool,
    layout: String,
}

const LOADING: &str = "the target page hasn't finished loading and painting";
/// Chrome stops painting a minimised or fully covered window (T-7), so it never settles.
const HIDDEN: &str = "the target window is hidden (minimised, covered by another window, or the screen is locked); bring it to the front and retry";

/// How long the layout has to hold still before the page counts as settled.
const STILL_FOR: Duration = Duration::from_millis(600);
const POLL: Duration = Duration::from_millis(100);

/// Waits until the target has a web page that has loaded, painted its first content, and stopped
/// changing size or layout (e.g. after Chrome is maximised). Preflight runs this before
/// StartRecord, so a take never opens on a blank page and the first click lands on what the
/// viewer sees. Right after launch DevTools can answer before the page exists, so a missing page
/// or a dropped session (the page navigating) is retried until the deadline.
pub fn wait_until_settled(port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut session: Option<Session> = None;
    let mut still_since: Option<(Instant, String)> = None;
    let mut last_error = String::from(LOADING);
    loop {
        let snapshot = match session.as_mut() {
            Some(open) => open.evaluate(SNAPSHOT_JS).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())),
            None => page_session(port).and_then(|mut open| {
                let snapshot = open.evaluate(SNAPSHOT_JS)?;
                session = Some(open);
                serde_json::from_value(snapshot).map_err(|e| e.to_string())
            }),
        };
        let now = Instant::now();
        still_since = match snapshot {
            Ok(Snapshot { ready: true, layout, .. }) => match still_since {
                Some((since, previous)) if previous == layout => {
                    if now - since >= STILL_FOR {
                        return Ok(());
                    }
                    Some((since, layout))
                }
                Some(_) => {
                    last_error = "the target page is still changing layout".into();
                    Some((now, layout))
                }
                None => Some((now, layout)),
            },
            Ok(Snapshot { ready: false, hidden, .. }) => {
                last_error = if hidden { HIDDEN } else { LOADING }.into();
                None
            }
            Err(e) => {
                last_error = e;
                session = None;
                None
            }
        };
        if now >= deadline {
            return Err(last_error);
        }
        std::thread::sleep(POLL);
    }
}

/// Local-only starting state. URLs never enter the AI outline or error messages.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct InitialState {
    pub url: String,
    pub headings: Vec<String>,
    pub dialogs: Vec<String>,
}

/// A JS function returning the page's [`InitialState`]. Teach mode calls the same function in the
/// page just before each click, so taught states compare equal to ones read here.
pub(crate) const INITIAL_STATE_FN: &str = r#"() => {
      const visible = e => e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden';
      const clean = e => (e.innerText || e.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim().slice(0,120);
      return { url: location.href,
        headings: [...document.querySelectorAll('h1,h2,h3')].filter(visible).map(clean),
        dialogs: [...document.querySelectorAll('dialog[open],[role=dialog],[aria-modal=true]')].filter(visible).map(clean) };
    }"#;

pub fn initial_state(session: &mut Session) -> Result<InitialState, String> {
    serde_json::from_value(session.evaluate(&format!("({INITIAL_STATE_FN})()"))?).map_err(|e| e.to_string())
}

pub fn check_initial(expected: &InitialState, actual: &InitialState) -> Result<(), String> {
    if actual.url != expected.url {
        return Err("The page has navigated away from the script starting page.".into());
    }
    if actual.dialogs != expected.dialogs {
        return Err("The open dialogs differ from the script starting page. Close or restore the dialog.".into());
    }
    if actual.headings != expected.headings {
        return Err("The page headings differ from the script starting page.".into());
    }
    Ok(())
}

/// Check the first action without moving the pointer or changing scroll position.
pub fn check_control(session: &mut Session, label: &str) -> Result<(), String> {
    let expression = format!(r#"(() => {{
      const clean = s => (s || '').replace(/\s+/g,' ').trim().toLowerCase();
      const label = clean({});
      const matches = [...document.querySelectorAll('a[href],button,[role=button]')]
        .filter(e => e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden'
          && clean(e.innerText || e.getAttribute('aria-label')) === label);
      if (matches.length !== 1) return false;
      const e = matches[0], r = e.getBoundingClientRect();
      if (e.disabled || e.getAttribute('aria-disabled') === 'true') return false;
      const x = r.left+r.width/2, y = r.top+r.height/2;
      if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) return false;
      const hit = document.elementFromPoint(x,y);
      return !!hit && (hit === e || e.contains(hit));
    }})()"#, serde_json::to_string(label).map_err(|e| e.to_string())?);
    if session.evaluate(&expression)?.as_bool() != Some(true) {
        return Err(format!("'{label}' must be visible, enabled and uncovered. Set up the page and retry."));
    }
    Ok(())
}

/// Restore a URL and observe a new document before accepting a settled layout. A slow network
/// response must not let the old, still-complete document pass the preflight.
pub fn restore_url(port: u16, url: &str, timeout: Duration, checkpoint: &impl Fn() -> Result<(), String>) -> Result<(), String> {
    checkpoint()?;
    let deadline = Instant::now() + timeout;
    let mut session = page_session(port)?;
    session.call("Page.bringToFront", serde_json::json!({}))?;
    let before = session.evaluate("performance.timeOrigin")?;
    checkpoint()?;
    let reply = session.call("Page.navigate", serde_json::json!({ "url": url }))?;
    if reply.get("errorText").is_some() { return Err("Could not restore the starting page".into()); }
    if reply.get("loaderId").is_none() {
        checkpoint()?;
        session.call("Page.reload", serde_json::json!({}))?;
    }
    loop {
        checkpoint()?;
        if let Ok(mut current) = page_session(port)
            && let Ok(state) = current.evaluate("({url:location.href,origin:performance.timeOrigin})")
            && state["url"].as_str() == Some(url) && state["origin"] != before
        {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() { break; }
            return wait_until_settled(port, left);
        }
        if Instant::now() >= deadline { break; }
        std::thread::sleep(POLL);
    }
    Err("The starting page did not finish navigating; set it up manually and retry".into())
}

/// Setup-only CDP click: exact, unique, enabled and hit-tested, without moving the OS cursor.
/// This never types into a field and never guesses another label.
pub fn setup_click(session: &mut Session, label: &str, checkpoint: &impl Fn() -> Result<(), String>) -> Result<(), String> {
    let expression = format!(r#"(() => {{
      const clean = s => (s || '').replace(/\s+/g,' ').trim().toLowerCase();
      const wanted = clean({});
      const matches = [...document.querySelectorAll('a[href],button,[role=button]')].filter(e =>
        e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden'
        && clean(e.innerText || e.getAttribute('aria-label')) === wanted);
      if (matches.length !== 1) return null;
      const e = matches[0];
      if (e.closest('form') || e.disabled || e.getAttribute('aria-disabled') === 'true') return null;
      e.scrollIntoView({{block:'center',inline:'center',behavior:'instant'}});
      const r = e.getBoundingClientRect(), x=r.left+r.width/2, y=r.top+r.height/2;
      const hit = document.elementFromPoint(x,y);
      return r.width>0 && r.height>0 && hit && (hit===e || e.contains(hit)) ? {{x,y}} : null;
    }})()"#, serde_json::to_string(label).map_err(|e| e.to_string())?);
    let point = session.evaluate(&expression)?;
    let x = point["x"].as_f64().ok_or_else(|| format!("Setup target '{label}' is missing, ambiguous, disabled or covered"))?;
    let y = point["y"].as_f64().ok_or("Setup target has no click position")?;
    checkpoint()?;
    let pressed = session.call("Input.dispatchMouseEvent", serde_json::json!({
        "type":"mousePressed", "x":x, "y":y, "button":"left", "clickCount":1
    }));
    // Always release, even if the press acknowledgement failed.
    let released = session.call("Input.dispatchMouseEvent", serde_json::json!({
        "type":"mouseReleased", "x":x, "y":y, "button":"left", "clickCount":1
    }));
    pressed?;
    released?;
    Ok(())
}

#[cfg(test)]
mod initial_tests {
    use super::*;
    fn baseline() -> InitialState {
        InitialState { url: "http://localhost/demo".into(), headings: vec!["Dashboard".into()], dialogs: vec![] }
    }
    #[test]
    fn rejects_navigation_dialog_and_heading_changes() {
        let expected = baseline();
        assert!(check_initial(&expected, &baseline()).is_ok());
        let mut actual = baseline(); actual.url.push_str("/other");
        assert!(check_initial(&expected, &actual).is_err());
        let mut actual = baseline(); actual.dialogs.push("Settings".into());
        assert!(check_initial(&expected, &actual).is_err());
        let mut actual = baseline(); actual.headings.clear();
        assert!(check_initial(&expected, &actual).is_err());
    }
}
