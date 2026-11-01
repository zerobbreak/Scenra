//! Teach mode in VS Code (docs/adr/0003-code-navigation.md, slice 2): the presenter works in
//! VS Code once and Scenra turns what changed into code steps.
//!
//! Editor state is recorded as end states, by polling the workbench: a different file becomes
//! Open (replayed through Quick Open, whichever way it was opened), scrolling that settles becomes
//! Scroll, and a settled selection becomes Highlight. Commits that leave no state behind are caught
//! by an in-page key listener: Enter on a command-palette command (Palette) and in the Search view
//! (Search). Other keyboard shortcuts are kept as flagged steps, shown but not replayed.

use crate::page::InitialState;
use crate::steps::{self, Anchor, Step};
use crate::teach::StepFlag;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// How long the editor must hold still before a scroll or selection counts as finished.
const SETTLE_MS: f64 = 900.0;
/// After a file opens, VS Code restores its scroll position: those moves aren't the presenter's.
const OPEN_SETTLE_MS: f64 = 1200.0;
/// A palette command's own effects (opening a file, scrolling) aren't separate steps.
const COMMAND_SETTLE_MS: f64 = 1500.0;
/// Lines the top may move without counting as a scroll (matches `page::TOP_TOLERANCE`).
const SCROLL_TOLERANCE: u32 = 2;

/// One poll of the workbench: the scene-start state plus what the editor shows.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Snapshot {
    pub state: InitialState,
    /// Lines with selected text, as rendered.
    #[serde(default)]
    pub selected: Vec<u32>,
    /// The first and last fully visible lines.
    #[serde(default)]
    pub visible: Option<(u32, u32)>,
    /// The first and last rendered lines (partly visible rows included).
    #[serde(default)]
    pub shown: Option<(u32, u32)>,
}

impl Snapshot {
    fn uri(&self) -> Option<&str> {
        self.state.code.as_ref()?.uri.as_deref()
    }

    fn top(&self) -> Option<u32> {
        self.state.code.as_ref()?.top
    }
}

/// A step worked out from the editor or the key listener, before it is numbered.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeChange {
    /// Milliseconds since the Unix epoch.
    pub at: f64,
    pub before: InitialState,
    pub label: String,
    pub step: Option<Step>,
    pub flags: Vec<StepFlag>,
}

/// The JS that takes one [`Snapshot`]. `{state}` is [`page::INITIAL_STATE_FN`], so taught states
/// compare equal to scene-start checks.
pub fn snapshot_js() -> String {
    format!(
        r#"(() => {{
  const state = ({state})();
  const ed = document.querySelector('.editor-group-container.active .editor-instance .monaco-editor')
    || document.querySelector('.editor-instance .monaco-editor');
  if (!ed) return {{ state, selected: [], visible: null, shown: null }};
  const g = (ed.querySelector('.overflow-guard') || ed).getBoundingClientRect();
  const tops = new Map(), full = [];
  for (const d of ed.querySelectorAll('.margin-view-overlays > div')) {{
    const n = d.querySelector('.line-numbers');
    if (!n || !+n.textContent) continue;
    tops.set(d.style.top, +n.textContent);
    const r = n.getBoundingClientRect();
    if (r.height > 0 && r.top >= g.top - 1 && r.bottom <= g.bottom + 1) full.push(+n.textContent);
  }}
  const lineOf = (d) => tops.get(d.style.top);
  // A whole-line selection ends at the start of the next line, which gets an empty (zero-width) mark.
  const marked = (d) => [...d.querySelectorAll('.selected-text')].some((s) => s.getBoundingClientRect().width > 0.5);
  const selected = [...ed.querySelectorAll('.view-overlays > div')].filter(marked).map(lineOf).filter(Boolean);
  const shown = [...tops.values()];
  return {{ state, selected: [...new Set(selected)].sort((a, b) => a - b),
    visible: full.length ? [Math.min(...full), Math.max(...full)] : null,
    shown: shown.length ? [Math.min(...shown), Math.max(...shown)] : null }};
}})()"#,
        state = crate::page::INITIAL_STATE_FN
    )
}

/// The in-page key listener for VS Code. It reports, through `binding`, a palette command or a
/// search when Enter commits it, other shortcuts, and the first typing in the editor. Keys that
/// open Quick Open, the palette, Search or Go to Line aren't reported: what they lead to is.
pub fn listener_js(binding: &str) -> String {
    format!(
        r#"(() => {{
  if (window.__scenraTeachOff) return true;
  const state = {state};
  const SKIP = new Set(['Ctrl+P', 'Ctrl+E', 'Ctrl+Shift+P', 'Ctrl+Shift+F', 'Ctrl+G', 'Ctrl+C', 'Ctrl+Tab', 'Ctrl+Shift+Tab']);
  let typed = false;
  const send = (ev, data) => window.{binding}(JSON.stringify({{ at: performance.timeOrigin + ev.timeStamp, before: state(), ...data }}));
  const onKey = ev => {{
    if (!ev.isTrusted || typeof window.{binding} !== 'function' || ev.repeat) return;
    const a = document.activeElement;
    const quick = a && a.closest('.quick-input-widget');
    if (ev.key === 'Enter' && quick) {{
      const value = (quick.querySelector('input') || {{}}).value || '';
      const row = quick.querySelector('.monaco-list-row.focused');
      // The visible name: the row's aria-label also carries its key binding ("…, Alt+Z").
      const name = row && (row.querySelector('.label-name') || {{}}).textContent;
      if (value.startsWith('>') && name) send(ev, {{ kind: 'palette', text: name }});
      return;
    }}
    if (ev.key === 'Enter' && a && a.closest('.search-view') && a.tagName === 'TEXTAREA') {{
      send(ev, {{ kind: 'search', text: a.value }});
      return;
    }}
    if (['Control', 'Shift', 'Alt', 'Meta'].includes(ev.key)) return;
    // Ctrl+A, Ctrl+Backspace… in a text box (Search, Find, a rename box) edit its text: not commands.
    const textBox = a && (a.tagName === 'INPUT' || a.tagName === 'TEXTAREA') && !a.closest('.monaco-editor');
    if (textBox) return;
    if (ev.ctrlKey || ev.altKey || ev.metaKey) {{
      const key = ev.key.length === 1 ? ev.key.toUpperCase() : ev.key;
      const combo = [ev.ctrlKey && 'Ctrl', ev.altKey && 'Alt', ev.shiftKey && 'Shift', ev.metaKey && 'Win', key].filter(Boolean).join('+');
      if (!SKIP.has(combo) && !quick) send(ev, {{ kind: 'shortcut', text: combo }});
      return;
    }}
    if (!typed && !quick && ev.key.length === 1 && a && a.closest('.monaco-editor')) {{
      typed = true;
      send(ev, {{ kind: 'typing', text: '' }});
    }}
  }};
  addEventListener('keydown', onKey, true);
  window.__scenraTeachOff = () => {{ removeEventListener('keydown', onKey, true); delete window.__scenraTeachOff; }};
  return true;
}})()"#,
        state = crate::page::INITIAL_STATE_FN
    )
}

/// What the key listener sends.
#[derive(Deserialize)]
pub struct RawKey {
    /// Milliseconds since the Unix epoch, from the page's clock.
    pub at: f64,
    pub kind: String,
    pub text: String,
    pub before: InitialState,
}

pub const TYPING_WARNING: &str = "typing in the editor isn't recorded; Scenra only replays opening, scrolling, highlighting, palette commands and searches";

/// A shortcut is kept as a flagged step unless the editor shows its effect (a new file, a
/// scroll or a selection) within this long: then that step stands for it.
const SHORTCUT_EXPLAINED_MS: f64 = 2500.0;

/// Turns workbench snapshots and key events into steps. `read` gives a file's text, for anchors.
pub struct Watcher<R: Fn(&Path) -> Option<String>> {
    folder: PathBuf,
    read: R,
    /// The editor as of the last step (or the start): what the next step is measured against.
    stable: Option<Snapshot>,
    /// The latest snapshot and when the editor last changed.
    latest: Option<(Snapshot, f64)>,
    /// Until then, changes update `stable` without becoming steps.
    absorb_until: f64,
    /// Shortcuts waiting to see whether the editor shows what they did.
    shortcuts: Vec<CodeChange>,
}

impl<R: Fn(&Path) -> Option<String>> Watcher<R> {
    pub fn new(folder: PathBuf, read: R) -> Self {
        Self { folder, read, stable: None, latest: None, absorb_until: 0.0, shortcuts: Vec::new() }
    }

    /// A key event from the listener. Returns a finished step, or a warning for typing.
    pub fn key(&mut self, raw: RawKey) -> Result<Option<CodeChange>, &'static str> {
        let step = match raw.kind.as_str() {
            "palette" if !raw.text.trim().is_empty() => {
                self.command_ran(raw.at);
                Step::Palette { command: raw.text.trim().to_owned() }
            }
            "search" if !raw.text.trim().is_empty() => Step::Search { query: raw.text.trim().to_owned() },
            "shortcut" => {
                let flagged = CodeChange { at: raw.at, before: raw.before, label: raw.text, step: None, flags: vec![StepFlag::Shortcut] };
                self.shortcuts.push(flagged);
                return Ok(None);
            }
            "typing" => return Err(TYPING_WARNING),
            _ => return Ok(None),
        };
        Ok(Some(CodeChange { at: raw.at, before: raw.before, label: step.describe(), step: Some(step), flags: vec![] }))
    }

    /// Steps that were waiting (shortcuts, an unsettled scroll or selection), at Stop.
    pub fn finish(&mut self, now: f64) -> Vec<CodeChange> {
        let mut out = match self.latest.clone() {
            Some((snap, changed_at)) => self.observe(snap, now.max(changed_at + SETTLE_MS)),
            None => vec![],
        };
        out.append(&mut self.shortcuts);
        out
    }

    /// Shortcuts the editor has had time to explain: kept unless a step came out of them.
    fn settle_shortcuts(&mut self, found: &[CodeChange], now: f64) -> Vec<CodeChange> {
        self.shortcuts.retain(|s| !found.iter().any(|f| f.at >= s.at - 50.0 && f.at <= s.at + SHORTCUT_EXPLAINED_MS));
        let (ready, waiting): (Vec<_>, Vec<_>) = self.shortcuts.drain(..).partition(|s| now - s.at > SHORTCUT_EXPLAINED_MS);
        self.shortcuts = waiting;
        ready
    }

    /// A palette command ran at `at`: what it does to the editor is part of that step.
    pub fn command_ran(&mut self, at: f64) {
        self.absorb_until = self.absorb_until.max(at + COMMAND_SETTLE_MS);
    }

    /// Feeds one poll taken at `now`; returns any finished steps.
    pub fn observe(&mut self, snap: Snapshot, now: f64) -> Vec<CodeChange> {
        let mut found = self.editor(snap, now);
        let shortcuts = self.settle_shortcuts(&found, now);
        found.extend(shortcuts);
        found
    }

    fn editor(&mut self, snap: Snapshot, now: f64) -> Vec<CodeChange> {
        let Some(stable) = self.stable.clone() else {
            self.latest = Some((snap.clone(), now));
            self.stable = Some(snap);
            return vec![];
        };
        let changed_at = match &self.latest {
            Some((last, at)) if *last == snap => *at,
            _ => now,
        };
        self.latest = Some((snap.clone(), changed_at));

        // A different file: a discrete change, recorded at once.
        if snap.uri() != stable.uri() && snap.uri().is_some() {
            let change = if now < self.absorb_until { None } else { self.open(&stable, &snap, now) };
            self.stable = Some(snap);
            self.absorb_until = self.absorb_until.max(now + OPEN_SETTLE_MS);
            return change.into_iter().collect();
        }
        if now - changed_at < SETTLE_MS {
            return vec![];
        }
        if now < self.absorb_until || snap.uri().is_none() {
            self.stable = Some(snap);
            return vec![];
        }
        let selection = !snap.selected.is_empty() && snap.selected != stable.selected;
        let scrolled = match (snap.top(), stable.top()) {
            (Some(a), Some(b)) => a.abs_diff(b) > SCROLL_TOLERANCE,
            _ => false,
        };
        let change = if selection {
            self.highlight(&stable, &snap, changed_at)
        } else if scrolled {
            self.scroll(&stable, &snap, changed_at)
        } else {
            None
        };
        if selection || scrolled || snap.selected != stable.selected {
            self.stable = Some(snap);
        }
        change.into_iter().collect()
    }

    /// The file's path inside the codebase and its text, or why it can't be a step.
    fn file(&self, uri: &str) -> Result<(String, String), StepFlag> {
        let path = steps::uri_to_path(uri).ok_or(StepFlag::NotInCodebase)?;
        let relative = steps::relative_to(&self.folder, &path).ok_or(StepFlag::NotInCodebase)?;
        let text = (self.read)(&path).ok_or(StepFlag::NotInCodebase)?;
        Ok((relative, text))
    }

    fn open(&self, before: &Snapshot, snap: &Snapshot, now: f64) -> Option<CodeChange> {
        let uri = snap.uri()?;
        let (label, step, flags) = match self.file(uri) {
            Ok((path, _)) => {
                let step = Step::Open { path, line: None, instant: false };
                (step.describe(), Some(step), vec![])
            }
            Err(flag) => (format!("Open {}", uri.rsplit('/').next().unwrap_or(uri)), None, vec![flag]),
        };
        Some(CodeChange { at: now, before: before.state.clone(), label, step, flags })
    }

    fn anchored(&self, snap: &Snapshot, first: u32) -> Option<(String, Anchor)> {
        let (path, text) = self.file(snap.uri()?).ok()?;
        let line_text = text.lines().nth(first.checked_sub(1)? as usize)?.trim().to_owned();
        Some((path, Anchor { line: first, text: line_text, symbol: steps::enclosing_symbol(&text, first) }))
    }

    fn highlight(&self, before: &Snapshot, snap: &Snapshot, at: f64) -> Option<CodeChange> {
        let (first, last) = (*snap.selected.iter().min()?, *snap.selected.iter().max()?);
        let lines = last - first + 1;
        let off_screen = snap.shown.is_some_and(|shown| {
            let total = snap.uri().and_then(|u| self.file(u).ok()).map_or(u32::MAX, |(_, t)| t.lines().count() as u32);
            steps::runs_off_screen(&snap.selected, shown, total)
        });
        Some(match self.anchored(snap, first) {
            Some((path, at_line)) if !off_screen => {
                let step = Step::Highlight { path, at: at_line, lines };
                CodeChange { at, before: before.state.clone(), label: step.describe(), step: Some(step), flags: vec![] }
            }
            Some(_) => CodeChange {
                at,
                before: before.state.clone(),
                label: format!("Highlight lines {first}–{last}"),
                step: None,
                flags: vec![StepFlag::OffScreen],
            },
            None => CodeChange {
                at,
                before: before.state.clone(),
                label: format!("Highlight lines {first}–{last}"),
                step: None,
                flags: vec![StepFlag::NotInCodebase],
            },
        })
    }

    fn scroll(&self, before: &Snapshot, snap: &Snapshot, at: f64) -> Option<CodeChange> {
        // Replay centres `lines` lines from the anchor: the whole visible range puts the top back.
        let (first, last) = snap.visible.or_else(|| snap.top().map(|t| (t, t)))?;
        let (path, anchor) = self.anchored(snap, first)?;
        let step = Step::Scroll { path, at: anchor, lines: last - first + 1 };
        Some(CodeChange { at, before: before.state.clone(), label: step.describe(), step: Some(step), flags: vec![] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::CodeState;

    const FILE: &str = "fn a() {\n    one();\n}\n\nfn b() {\n    two();\n    three();\n}\n";

    fn snap(file: Option<&str>, top: u32, selected: &[u32]) -> Snapshot {
        Snapshot {
            state: InitialState {
                url: "vscode://workbench".into(),
                headings: vec![],
                dialogs: vec![],
                code: Some(CodeState { uri: file.map(|f| format!("file:///c%3A/repo/{f}")), top: Some(top), panel: "Explorer".into() }),
            },
            selected: selected.to_vec(),
            visible: Some((top, top + 30)),
            shown: Some((top.saturating_sub(1).max(1), top + 31)),
        }
    }

    fn watcher() -> Watcher<impl Fn(&Path) -> Option<String>> {
        Watcher::new(PathBuf::from(r"C:\repo"), |p: &Path| p.to_string_lossy().ends_with("lib.rs").then(|| FILE.to_owned()))
    }

    /// Feeds the same snapshot for a while, like the poll loop does.
    fn hold<R: Fn(&Path) -> Option<String>>(w: &mut Watcher<R>, s: Snapshot, from: f64, ms: f64) -> Vec<CodeChange> {
        let mut out = vec![];
        let mut t = from;
        while t <= from + ms {
            out.extend(w.observe(s.clone(), t));
            t += 200.0;
        }
        out
    }

    #[test]
    fn a_new_file_is_an_open_step_and_its_restored_scroll_isnt_a_scroll() {
        let mut w = watcher();
        hold(&mut w, snap(Some("README.md"), 1, &[]), 0.0, 400.0);
        let steps = w.observe(snap(Some("src/lib.rs"), 1, &[]), 1000.0);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step, Some(Step::Open { path: "src/lib.rs".into(), line: None, instant: false }));
        assert_eq!(steps[0].before.code.as_ref().unwrap().uri.as_deref(), Some("file:///c%3A/repo/README.md"));
        // VS Code jumps to the file's remembered place right after opening it.
        assert!(hold(&mut w, snap(Some("src/lib.rs"), 40, &[]), 1200.0, 2000.0).is_empty());
    }

    #[test]
    fn a_scroll_counts_once_it_settles() {
        let mut w = watcher();
        hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 0.0, 400.0);
        // Still moving: nothing yet.
        assert!(w.observe(snap(Some("src/lib.rs"), 3, &[]), 600.0).is_empty());
        assert!(w.observe(snap(Some("src/lib.rs"), 5, &[]), 800.0).is_empty());
        let steps = hold(&mut w, snap(Some("src/lib.rs"), 5, &[]), 1000.0, 1200.0);
        assert_eq!(steps.len(), 1);
        let Some(Step::Scroll { at, lines, .. }) = &steps[0].step else { panic!("{:?}", steps[0]) };
        assert_eq!((at.line, at.text.as_str(), at.symbol.as_deref(), *lines), (5, "fn b() {", Some("fn b"), 31));
        assert_eq!(steps[0].before.code.as_ref().unwrap().top, Some(1));
        // A small wobble is not another scroll.
        assert!(hold(&mut w, snap(Some("src/lib.rs"), 6, &[]), 2400.0, 1200.0).is_empty());
    }

    #[test]
    fn a_settled_selection_is_a_highlight_and_clearing_it_is_nothing() {
        let mut w = watcher();
        hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 0.0, 400.0);
        let steps = hold(&mut w, snap(Some("src/lib.rs"), 1, &[6, 7]), 600.0, 1200.0);
        assert_eq!(steps.len(), 1);
        let Some(Step::Highlight { at, lines, .. }) = &steps[0].step else { panic!("{:?}", steps[0]) };
        assert_eq!((at.line, at.text.as_str(), *lines), (6, "two();", 2));
        assert!(hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 2000.0, 1200.0).is_empty());
        // The same selection again, after clearing it, is a new highlight.
        assert_eq!(hold(&mut w, snap(Some("src/lib.rs"), 1, &[6, 7]), 3400.0, 1200.0).len(), 1);
    }

    #[test]
    fn files_outside_the_codebase_and_off_screen_selections_are_flagged() {
        let mut w = watcher();
        hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 0.0, 400.0);
        let mut other = snap(None, 1, &[]);
        other.state.code.as_mut().unwrap().uri = Some("file:///c%3A/elsewhere/x.rs".into());
        let steps = w.observe(other, 600.0);
        assert_eq!((steps[0].step.as_ref(), steps[0].flags.as_slice()), (None, &[StepFlag::NotInCodebase][..]));

        let mut w = watcher();
        hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 0.0, 400.0);
        let mut tall = snap(Some("src/lib.rs"), 2, &[3, 4]);
        tall.shown = Some((3, 4));
        let steps = hold(&mut w, tall, 600.0, 1200.0);
        assert_eq!(steps[0].flags, vec![StepFlag::OffScreen]);
    }

    fn key(kind: &str, text: &str, at: f64) -> RawKey {
        RawKey { at, kind: kind.into(), text: text.into(), before: snap(Some("src/lib.rs"), 1, &[]).state }
    }

    #[test]
    fn shortcuts_are_flagged_unless_the_editor_shows_what_they_did() {
        let mut w = watcher();
        hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 0.0, 400.0);
        // Ctrl+End scrolls: the Scroll step stands for it.
        assert_eq!(w.key(key("shortcut", "Ctrl+End", 500.0)), Ok(None));
        let steps = hold(&mut w, snap(Some("src/lib.rs"), 5, &[]), 600.0, 4000.0);
        assert_eq!(steps.len(), 1, "{steps:?}");
        assert!(matches!(steps[0].step, Some(Step::Scroll { .. })));
        // Ctrl+B only hides the side bar: kept, flagged, once it's had time to show an effect.
        w.key(key("shortcut", "Ctrl+B", 5000.0)).unwrap();
        assert!(hold(&mut w, snap(Some("src/lib.rs"), 5, &[]), 5200.0, 1000.0).is_empty());
        let steps = hold(&mut w, snap(Some("src/lib.rs"), 5, &[]), 6400.0, 1400.0);
        assert_eq!((steps.len(), steps[0].label.as_str(), steps[0].flags.as_slice()), (1, "Ctrl+B", &[StepFlag::Shortcut][..]));
    }

    #[test]
    fn palette_and_search_commits_are_steps_and_typing_warns() {
        let mut w = watcher();
        let palette = w.key(key("palette", "View: Toggle Word Wrap", 100.0)).unwrap().unwrap();
        assert_eq!((palette.label.as_str(), palette.step), ("Run “View: Toggle Word Wrap”", Some(Step::Palette { command: "View: Toggle Word Wrap".into() })));
        let search = w.key(key("search", " fn stop ", 200.0)).unwrap().unwrap();
        assert_eq!(search.step, Some(Step::Search { query: "fn stop".into() }));
        assert_eq!(w.key(key("typing", "", 300.0)), Err(TYPING_WARNING));
        assert_eq!(w.key(key("search", "  ", 400.0)), Ok(None));
    }

    #[test]
    fn stop_keeps_an_unsettled_scroll_and_waiting_shortcuts() {
        let mut w = watcher();
        hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 0.0, 400.0);
        w.observe(snap(Some("src/lib.rs"), 5, &[]), 600.0);
        w.key(key("shortcut", "Ctrl+B", 2000.0)).unwrap();
        let steps = w.finish(2100.0);
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(matches!(steps[0].step, Some(Step::Scroll { .. })));
    }

    /// Live: teach a code walkthrough on this repo in Scenra's VS Code (Scenra's own input stands in
    /// for the presenter), draft it with a stand-in AI, then reset to the scene start and replay.
    #[test]
    #[ignore = "launches VS Code and moves the real mouse and keyboard"]
    fn teaches_and_replays_a_code_walkthrough() {
        use crate::input::{self, Pace};
        use std::sync::mpsc;
        use std::time::Duration;
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_B, VK_CONTROL};
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let repo = PathBuf::from(repo.canonicalize().unwrap().to_string_lossy().trim_start_matches(r"\\?\"));
        let profile = std::env::temp_dir().join(format!("scenra-teach-code-{}", std::process::id()));
        let targets = crate::target::launch_vscode_for_test(&repo, &profile).unwrap();
        let (pid, port, _) = crate::target::vscode(&targets).unwrap();
        std::thread::sleep(Duration::from_secs(6)); // workbench restores

        let take = "scenra/src-tauri/src/take.rs";
        let text = std::fs::read_to_string(repo.join(take)).unwrap();
        let stop = text.lines().position(|l| l.trim_start().starts_with("pub(crate) fn stop(")).unwrap() as u32 + 1;
        let at = Anchor { line: stop, text: text.lines().nth(stop as usize - 1).unwrap().trim().into(), symbol: None };
        let settle = || std::thread::sleep(Duration::from_millis(1800));
        let act = |step: Step| {
            crate::steps::approach(&targets, &step, Pace::Natural).unwrap();
            crate::steps::run(&targets, &step, Pace::Natural).unwrap_or_else(|e| panic!("{}: {e}", step.describe()));
            settle();
        };

        let (sent, seen) = mpsc::channel();
        let recorder = crate::teach::Recorder::start(port, Some(repo.clone()), move |s| {
            let _ = sent.send(s.label.clone());
        })
        .unwrap();
        settle();
        act(Step::Open { path: take.into(), line: None, instant: false });
        act(Step::Scroll { path: take.into(), at: at.clone(), lines: 6 });
        act(Step::Highlight { path: take.into(), at: at.clone(), lines: 3 });
        crate::locate::focus_target(pid).unwrap();
        input::chord(&[VK_CONTROL, VK_B]).unwrap(); // hides the side bar: a flagged shortcut
        std::thread::sleep(Duration::from_secs(3));
        act(Step::Search { query: "fn stop".into() });
        act(Step::Palette { command: "View: Toggle Word Wrap".into() });
        let session = recorder.stop().unwrap();
        let live: Vec<String> = seen.try_iter().collect();
        for s in &session.steps {
            eprintln!("{:>2}. {:<44} {:?} {:?}", s.index + 1, s.label, s.flags, s.before.code.as_ref().map(|c| (&c.uri, c.top, &c.panel)));
        }
        eprintln!("warnings: {:?}", session.warnings);

        let kinds: Vec<&str> = session
            .steps
            .iter()
            .map(|s| match (&s.step, s.flags.first()) {
                (Some(Step::Open { .. }), _) => "open",
                (Some(Step::Scroll { .. }), _) => "scroll",
                (Some(Step::Highlight { .. }), _) => "highlight",
                (Some(Step::Search { .. }), _) => "search",
                (Some(Step::Palette { .. }), _) => "palette",
                (None, Some(StepFlag::Shortcut)) => "shortcut",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["open", "scroll", "highlight", "shortcut", "search", "palette"], "{:?}", session.steps);
        assert!(live.len() >= 5, "steps should arrive live: {live:?}");
        let Some(Step::Highlight { at: taught, lines, .. }) = &session.steps[2].step else { unreachable!() };
        assert_eq!((taught.line, *lines), (stop, 3));
        assert_eq!(session.steps[4].step, Some(Step::Search { query: "fn stop".into() }));
        assert_eq!(session.steps[5].step, Some(Step::Palette { command: "View: Toggle Word Wrap".into() }));

        // Draft: one scene narrating the five replayable steps (the shortcut is skipped).
        let provider = crate::ai::Scripted(vec![(
            "Code tour",
            vec![
                ("Let's open the take module and scroll down to stop.", vec![("open", "s0"), ("scroll", "s1")]),
                ("Here are the lines that matter, so let's search and wrap.", vec![("lines", "s2"), ("search", "s4"), ("wrap", "s5")]),
            ],
        )]);
        let (script, states) = crate::script::draft_from_session(&provider, &session, "a code tour").unwrap();
        let actions: Vec<Step> = script.scenes[0].lines.iter().flat_map(|l| l.actions.iter().map(|a| a.step.clone().unwrap())).collect();
        assert_eq!(actions.len(), 5);
        assert!(script.warnings.iter().any(|w| w.contains("keyboard shortcuts")), "{:?}", script.warnings);

        // Replay: put VS Code back where the scene starts, check it, then run the steps.
        let start = states[0].code.clone().unwrap();
        crate::steps::restore_state(&targets, &start).unwrap();
        std::thread::sleep(Duration::from_millis(800));
        let now = crate::page::initial_state(&mut crate::page::page_session(port).unwrap()).unwrap();
        crate::page::check_initial(&states[0], &now).unwrap_or_else(|e| panic!("scene start not restored: {e} (want {start:?}, have {:?})", now.code));
        let mut view = crate::page::page_session(port).unwrap();
        for step in &actions {
            act(step.clone());
            if matches!(step, Step::Highlight { .. }) {
                let snap: Snapshot = serde_json::from_value(view.evaluate(&snapshot_js()).unwrap()).unwrap();
                assert_eq!(snap.selected, vec![stop, stop + 1, stop + 2], "replayed highlight");
            }
        }
        // An empty line inside a selection still counts as selected.
        let blank = text.lines().enumerate().skip(stop as usize).find(|(_, l)| l.trim().is_empty()).unwrap().0 as u32 + 1;
        let around = Anchor { line: blank - 1, text: text.lines().nth(blank as usize - 2).unwrap().trim().into(), symbol: None };
        act(Step::Highlight { path: take.into(), at: around, lines: 3 });
        let snap: Snapshot = serde_json::from_value(view.evaluate(&snapshot_js()).unwrap()).unwrap();
        assert_eq!(snap.selected, vec![blank - 1, blank, blank + 1], "selection across the empty line {blank}");
        drop(targets);
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn a_palette_commands_effects_are_absorbed() {
        let mut w = watcher();
        hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 0.0, 400.0);
        w.command_ran(500.0);
        assert!(w.observe(snap(Some("README.md"), 1, &[]), 700.0).is_empty());
        assert!(hold(&mut w, snap(Some("README.md"), 20, &[]), 900.0, 1400.0).is_empty());
        // Later, an ordinary scroll counts again.
        assert_eq!(hold(&mut w, snap(Some("src/lib.rs"), 1, &[]), 4000.0, 200.0).len(), 1);
    }
}
