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
const SNAPSHOT_JS: &str = r#"(() => {
  const painted = performance.getEntriesByType('paint').some((e) => e.name === 'first-contentful-paint');
  const ready = document.readyState === 'complete' && document.visibilityState === 'visible'
    && document.fonts.status === 'loaded' && painted;
  const root = document.documentElement;
  const layout = [innerWidth, innerHeight, root.scrollWidth, root.scrollHeight,
    document.getElementsByTagName('*').length].join(',');
  return { ready, layout };
})()"#;

#[derive(Deserialize)]
struct Snapshot {
    ready: bool,
    layout: String,
}

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
    let mut last_error = String::from("the target page hasn't finished loading and painting");
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
            Ok(Snapshot { ready: true, layout }) => match still_since {
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
            Ok(Snapshot { ready: false, .. }) => None,
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
