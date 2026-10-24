//! AiProvider (PROJECT_OUTLINE §4): Gemini drafts a narrated script from a page outline.
//!
//! Actions are pinned to a word in the script, never to a time. The model names the trigger word
//! and a control id from the outline; Scenra resolves and checks both, and drops what doesn't
//! match the real page rather than guessing.

use crate::page::{Control, PageOutline};
use crate::secrets::{self, KeySource};
use crate::teach::TeachStep;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

/// Tried in order. A model that's overloaded (429, 5xx) or doesn't answer in time hands over to the
/// next; the `-latest` aliases have hung for minutes under load, so the primary is pinned.
pub const GEMINI_MODELS: [&str; 2] = ["gemini-2.5-flash", "gemini-flash-lite-latest"];
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    pub title: String,
    pub scenes: Vec<Scene>,
    /// Actions Scenra dropped because they didn't match the script or the page.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub title: String,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub text: String,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// Index into `text.split_whitespace()`: the action fires when narration reaches this word.
    pub word: usize,
    /// The control's label on the page, used to find it at click time.
    pub target: String,
    /// What the pill shows, e.g. "Click Recall".
    pub label: String,
}

pub trait AiProvider {
    fn draft_script(&self, page: &PageOutline, brief: &str) -> Result<Scenario, String>;
    /// Writes narration around a taught walkthrough. Every usable step is clicked exactly once,
    /// in the order it was taught.
    fn align_script(&self, steps: &[TeachStep], brief: &str) -> Result<Aligned, String>;
}

/// A script aligned to a taught walkthrough.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aligned {
    pub scenario: Scenario,
    /// For each scene, the index (into the taught steps) of the first step at or after the scene's
    /// start: its starting state is that step's `before`. `None` for a scene after the last step,
    /// which starts on the page as teaching ended.
    pub scene_starts: Vec<Option<usize>>,
}

pub struct Gemini {
    key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    configured: bool,
    source: Option<KeySource>,
    model: &'static str,
}

pub fn status() -> AiStatus {
    let source = secrets::gemini_key().map(|(_, source)| source);
    AiStatus { configured: source.is_some(), source, model: GEMINI_MODELS[0] }
}

impl Gemini {
    pub fn from_store() -> Result<Self, String> {
        let (key, _) = secrets::gemini_key().ok_or(format!(
            "no Gemini key: run `cmdkey /generic:{} /user:gemini /pass:<key>` or set {}",
            secrets::GEMINI_TARGET,
            secrets::GEMINI_ENV
        ))?;
        Ok(Self { key })
    }
}

const SYSTEM: &str = "You write narration scripts for screen-recorded software demos. \
The presenter reads your script aloud from a teleprompter while Scenra clicks through the app. \
Write in a warm, plain, spoken voice, first person plural (\"let's\"). \
Every line is one short spoken sentence of 6 to 14 words. \
Pin each click to the word in the line where the viewer should see it happen: \
`trigger` must be one word copied exactly from that line, and `control` must be an id from the \
provided controls. Only use controls that navigate or reveal content; never sign in, sign up, \
join a waitlist, buy or submit anything. Use only facts visible in the outline.";

const SYSTEM_ALIGN: &str = "You write narration scripts for screen-recorded software demos. \
The presenter already clicked through the workflow once; you get those clicks as ordered steps, \
each with the control's label and the headings and dialogs visible just before it. \
The presenter reads your script aloud from a teleprompter while Scenra repeats the clicks. \
Write in a warm, plain, spoken voice, first person plural (\"let's\"). \
Every line is one short spoken sentence of 6 to 14 words. \
Group the steps into scenes at natural boundaries, such as a new page, panel or dialog. \
Use every step exactly once, in the given order, and no others: `step` must be a step id. \
Pin each step to the word in the line where the viewer should see that click: \
`trigger` must be one word copied exactly from that line. \
Use only facts visible in the steps.";

/// Gemini's response schema (OpenAPI subset). Mirrors the draft types below. `target` names the
/// action's id field: `control` for a page outline, `step` for a taught walkthrough.
fn schema(target: &str) -> Value {
    json!({
      "type": "OBJECT",
      "required": ["title", "scenes"],
      "properties": {
        "title": { "type": "STRING" },
        "scenes": { "type": "ARRAY", "items": {
          "type": "OBJECT",
          "required": ["title", "lines"],
          "properties": {
            "title": { "type": "STRING" },
            "lines": { "type": "ARRAY", "items": {
              "type": "OBJECT",
              "required": ["text"],
              "properties": {
                "text": { "type": "STRING" },
                "actions": { "type": "ARRAY", "items": {
                  "type": "OBJECT",
                  "required": ["trigger", target],
                  "properties": {
                    "trigger": { "type": "STRING" },
                    target: { "type": "STRING" }
                  }
                }}
              }
            }}
          }
        }}
      }
    })
}

#[derive(Deserialize)]
struct Draft {
    title: String,
    scenes: Vec<DraftScene>,
}

#[derive(Deserialize)]
struct DraftScene {
    title: String,
    lines: Vec<DraftLine>,
}

#[derive(Deserialize)]
struct DraftLine {
    text: String,
    #[serde(default)]
    actions: Vec<DraftAction>,
}

#[derive(Deserialize)]
struct DraftAction {
    trigger: String,
    /// A control id from the outline, or a step id when aligning to a walkthrough.
    #[serde(alias = "step")]
    control: String,
}

/// What the AI sees of a taught step: no URL, nothing from fields (those steps aren't sent).
#[derive(Serialize)]
struct AlignStep<'a> {
    id: String,
    label: &'a str,
    headings: &'a [String],
    dialogs: &'a [String],
}

fn step_id(step: &TeachStep) -> String {
    format!("s{}", step.index)
}

fn align_input(steps: &[TeachStep]) -> Vec<AlignStep<'_>> {
    steps
        .iter()
        .filter(|s| s.usable())
        .map(|s| AlignStep { id: step_id(s), label: &s.label, headings: &s.before.headings, dialogs: &s.before.dialogs })
        .collect()
}

impl AiProvider for Gemini {
    fn draft_script(&self, page: &PageOutline, brief: &str) -> Result<Scenario, String> {
        let prompt = format!(
            "Brief: {brief}\n\nPage outline (JSON):\n{}\n\n\
             Write 3 to 5 scenes, 2 to 4 lines each, about 60 to 90 seconds read aloud in total. \
             Use 4 to 8 clicks across the whole script.",
            serde_json::to_string_pretty(page).map_err(|e| e.to_string())?
        );
        let draft: Draft = serde_json::from_str(&self.generate(SYSTEM, &prompt, schema("control"))?)
            .map_err(|e| format!("Gemini script: {e}"))?;
        Ok(resolve(draft, &page.controls))
    }

    fn align_script(&self, steps: &[TeachStep], brief: &str) -> Result<Aligned, String> {
        let input = align_input(steps);
        if input.is_empty() {
            return Err("the walkthrough has no clicks Scenra can replay".into());
        }
        let prompt = format!(
            "Brief: {brief}\n\nSteps, in the order they were taught (JSON):\n{}\n\n\
             Write 2 to 5 scenes, 2 to 4 lines each. Use each step exactly once, in order.",
            serde_json::to_string_pretty(&input).map_err(|e| e.to_string())?
        );
        let draft: Draft = serde_json::from_str(&self.generate(SYSTEM_ALIGN, &prompt, schema("step"))?)
            .map_err(|e| format!("Gemini script: {e}"))?;
        resolve_aligned(draft, steps)
    }
}

impl Gemini {
    /// One structured-output request, falling back through [`GEMINI_MODELS`]. Returns the JSON text.
    fn generate(&self, system: &str, prompt: &str, schema: Value) -> Result<String, String> {
        let body = json!({
            "systemInstruction": { "parts": [{ "text": system }] },
            "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema,
                "temperature": 0.7
            }
        });

        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut busy = Vec::new();
        let reply = 'models: {
            for model in GEMINI_MODELS {
                let mut response = match agent
                    .post(format!("{ENDPOINT}/{model}:generateContent"))
                    .header("x-goog-api-key", &self.key)
                    .send_json(&body)
                {
                    Ok(response) => response,
                    Err(e) => {
                        busy.push(format!("{model}: {e}"));
                        continue;
                    }
                };
                let status = response.status();
                let reply: Value = response.body_mut().read_json().map_err(|e| format!("Gemini reply: {e}"))?;
                if status.is_success() {
                    break 'models reply;
                }
                let message = reply["error"]["message"].as_str().unwrap_or("unknown error");
                if status.as_u16() == 429 || status.is_server_error() {
                    busy.push(format!("{model}: {status} {message}"));
                    continue;
                }
                return Err(format!("Gemini {status}: {message}"));
            }
            return Err(format!("Gemini unavailable ({})", busy.join("; ")));
        };
        reply["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "Gemini returned no script".to_owned())
    }
}

/// Lower-cased word without surrounding punctuation, for matching triggers.
fn bare(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn resolve(draft: Draft, controls: &[Control]) -> Scenario {
    let mut warnings = Vec::new();
    let scenes = draft
        .scenes
        .into_iter()
        .map(|scene| Scene {
            title: scene.title,
            lines: scene
                .lines
                .into_iter()
                .map(|line| {
                    let words: Vec<String> = line.text.split_whitespace().map(bare).collect();
                    let mut actions = Vec::new();
                    for action in line.actions {
                        let Some(control) = controls.iter().find(|c| c.id == action.control) else {
                            warnings.push(format!("dropped a click on unknown control {}", action.control));
                            continue;
                        };
                        let want = bare(&action.trigger);
                        let taken = |i: &usize| actions.iter().any(|a: &Action| a.word == *i);
                        let Some(word) = (0..words.len()).find(|i| words[*i] == want && !taken(i)) else {
                            warnings.push(format!(
                                "dropped “{}”: trigger “{}” isn't in “{}”",
                                control.label, action.trigger, line.text
                            ));
                            continue;
                        };
                        actions.push(Action {
                            word,
                            target: control.label.clone(),
                            label: format!("Click {}", title_case(&short(&control.label))),
                        });
                    }
                    actions.sort_by_key(|a| a.word);
                    Line { text: line.text, actions }
                })
                .filter(|line| !line.text.trim().is_empty())
                .collect(),
        })
        .filter(|scene: &Scene| !scene.lines.is_empty())
        .collect();
    Scenario { title: draft.title, scenes, warnings }
}

/// Checks a script against the walkthrough it narrates. Unlike [`resolve`], nothing is dropped:
/// a skipped, repeated or reordered step would replay a different workflow than the one taught,
/// so any mismatch fails the draft. Flagged steps were never offered and are reported as skipped.
fn resolve_aligned(draft: Draft, steps: &[TeachStep]) -> Result<Aligned, String> {
    let usable: Vec<&TeachStep> = steps.iter().filter(|s| s.usable()).collect();
    let mut problems = Vec::new();
    let mut next = 0;
    let mut scenes = Vec::new();
    let mut scene_starts = Vec::new();
    for scene in draft.scenes {
        let lines: Vec<DraftLine> = scene.lines.into_iter().filter(|l| !l.text.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        scene_starts.push(usable.get(next).map(|s| s.index));
        let mut resolved = Vec::new();
        for line in lines {
            let words: Vec<String> = line.text.split_whitespace().map(bare).collect();
            let mut placed: Vec<(usize, &TeachStep)> = Vec::new();
            for action in line.actions {
                let Some(step) = usable.iter().find(|s| step_id(s) == action.control) else {
                    problems.push(format!("unknown step {}", action.control));
                    continue;
                };
                let want = bare(&action.trigger);
                let Some(word) = (0..words.len()).find(|i| words[*i] == want && !placed.iter().any(|(w, _)| w == i)) else {
                    problems.push(format!("trigger “{}” for “{}” isn't in “{}”", action.trigger, step.label, line.text));
                    continue;
                };
                placed.push((word, step));
            }
            // Narration order within a line is word order, whatever order the actions came in.
            placed.sort_by_key(|(word, _)| *word);
            let mut actions = Vec::new();
            for (word, step) in placed {
                match usable.get(next) {
                    Some(expected) if expected.index == step.index => next += 1,
                    Some(expected) => problems.push(format!(
                        "“{}” is narrated where step {} (“{}”) was taught",
                        step.label, expected.index + 1, expected.label
                    )),
                    None => problems.push(format!("“{}” is narrated twice", step.label)),
                }
                actions.push(Action {
                    word,
                    target: step.label.clone(),
                    label: format!("Click {}", title_case(&short(&step.label))),
                });
            }
            resolved.push(Line { text: line.text, actions });
        }
        scenes.push(Scene { title: scene.title, lines: resolved });
    }
    for missing in &usable[next.min(usable.len())..] {
        problems.push(format!("step {} (“{}”) isn't narrated", missing.index + 1, missing.label));
    }
    if !problems.is_empty() {
        return Err(format!("Gemini's script doesn't follow the walkthrough; draft again ({})", problems.join("; ")));
    }
    let warnings = steps
        .iter()
        .filter(|s| !s.usable())
        .map(|s| {
            let why: Vec<&str> = s.flags.iter().map(|f| f.describe()).collect();
            format!("skipped step {} ({}); if it changed the page, later scenes may not start as taught", s.index + 1, why.join(", "))
        })
        .collect();
    Ok(Aligned { scenario: Scenario { title: draft.title, scenes, warnings }, scene_starts })
}

/// A stand-in provider for tests: replies with a fixed script, checked like Gemini's.
/// Each scene is `(title, lines)`; each line is `(text, [(trigger, step id)])`.
#[cfg(test)]
pub(crate) struct Scripted(pub Vec<(&'static str, Vec<ScriptedLine>)>);

/// `(text, [(trigger, step or control id)])`.
#[cfg(test)]
pub(crate) type ScriptedLine = (&'static str, Vec<(&'static str, &'static str)>);

#[cfg(test)]
impl Scripted {
    fn draft(&self) -> Draft {
        Draft {
            title: "Scripted".into(),
            scenes: self.0.iter().map(|(title, lines)| DraftScene {
                title: (*title).into(),
                lines: lines.iter().map(|(text, actions)| DraftLine {
                    text: (*text).into(),
                    actions: actions.iter().map(|(t, s)| DraftAction { trigger: (*t).into(), control: (*s).into() }).collect(),
                }).collect(),
            }).collect(),
        }
    }
}

#[cfg(test)]
impl AiProvider for Scripted {
    fn draft_script(&self, page: &PageOutline, _brief: &str) -> Result<Scenario, String> {
        Ok(resolve(self.draft(), &page.controls))
    }

    fn align_script(&self, steps: &[TeachStep], _brief: &str) -> Result<Aligned, String> {
        resolve_aligned(self.draft(), steps)
    }
}

/// The first few words of a control label, short enough for the pill's action chip.
fn short(label: &str) -> String {
    let mut out = String::new();
    for word in label.split_whitespace() {
        if !out.is_empty() && out.len() + word.len() > 22 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// "RECALL" → "Recall", "NEXT CARD →" → "Next Card →". Leaves mixed-case labels alone.
fn title_case(label: &str) -> String {
    if label.chars().any(|c| c.is_lowercase()) {
        return label.to_owned();
    }
    label
        .split(' ')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls() -> Vec<Control> {
        vec![
            Control { id: "c0".into(), role: "link".into(), label: "RECALL".into() },
            Control { id: "c1".into(), role: "button".into(), label: "Next card →".into() },
        ]
    }

    fn draft(text: &str, actions: &[(&str, &str)]) -> Draft {
        Draft {
            title: "Tour".into(),
            scenes: vec![DraftScene {
                title: "One".into(),
                lines: vec![DraftLine {
                    text: text.into(),
                    actions: actions
                        .iter()
                        .map(|(t, c)| DraftAction { trigger: (*t).into(), control: (*c).into() })
                        .collect(),
                }],
            }],
        }
    }

    #[test]
    fn resolves_trigger_word_ignoring_case_and_punctuation() {
        let s = resolve(draft("Now let's open Recall, our flashcards.", &[("recall", "c0")]), &controls());
        let action = &s.scenes[0].lines[0].actions[0];
        assert_eq!(action.word, 3);
        assert_eq!(action.target, "RECALL");
        assert_eq!(action.label, "Click Recall");
        assert!(s.warnings.is_empty());
    }

    #[test]
    fn long_labels_are_shortened_for_the_chip() {
        assert_eq!(short("QUESTION State the second law of thermodynamics."), "QUESTION State the");
        assert_eq!(short("RECALL"), "RECALL");
    }

    #[test]
    fn drops_unknown_controls_and_missing_triggers() {
        let s = resolve(draft("Let's flip to the next card.", &[("next", "c9"), ("banana", "c1")]), &controls());
        assert!(s.scenes[0].lines[0].actions.is_empty());
        assert_eq!(s.warnings.len(), 2);
    }

    fn taught(labels: &[&str]) -> Vec<TeachStep> {
        labels
            .iter()
            .enumerate()
            .map(|(index, label)| TeachStep {
                index,
                at_ms: index as u64 * 1000,
                label: (*label).into(),
                test_id: None,
                role: Some("button".into()),
                before: crate::page::InitialState { url: "http://localhost/secret-path".into(), headings: vec![format!("H{index}")], dialogs: vec![] },
                // An empty label stands for a click into a field.
                flags: if label.is_empty() { vec![crate::teach::StepFlag::Field] } else { vec![] },
            })
            .collect()
    }

    fn scene(title: &str, lines: &[(&str, &[(&str, &str)])]) -> DraftScene {
        DraftScene {
            title: title.into(),
            lines: lines
                .iter()
                .map(|(text, actions)| DraftLine {
                    text: (*text).into(),
                    actions: actions.iter().map(|(t, s)| DraftAction { trigger: (*t).into(), control: (*s).into() }).collect(),
                })
                .collect(),
        }
    }

    fn aligned(scenes: Vec<DraftScene>, steps: &[TeachStep]) -> Result<Aligned, String> {
        resolve_aligned(Draft { title: "Tour".into(), scenes }, steps)
    }

    #[test]
    fn aligns_every_step_in_order_and_records_where_scenes_start() {
        let steps = taught(&["New Recording", "", "Settings", "Confirm"]);
        let result = aligned(
            vec![
                scene("Start", &[("Let's start a new recording.", &[("new", "s0")])]),
                // Actions listed out of word order still read in word order.
                scene("Settings", &[("Open settings, then confirm the change.", &[("confirm", "s3"), ("settings", "s2")])]),
                scene("Wrap up", &[("That's all it takes.", &[])]),
            ],
            &steps,
        )
        .unwrap();
        let actions: Vec<(usize, &str)> = result.scenario.scenes.iter()
            .flat_map(|s| &s.lines).flat_map(|l| &l.actions).map(|a| (a.word, a.target.as_str())).collect();
        assert_eq!(actions, [(3, "New Recording"), (1, "Settings"), (3, "Confirm")]);
        assert_eq!(result.scene_starts, [Some(0), Some(2), None]);
        assert_eq!(result.scenario.warnings.len(), 1, "the field click is reported as skipped");
        assert!(result.scenario.warnings[0].contains("step 2"));
    }

    #[test]
    fn a_scene_without_clicks_starts_at_the_next_step() {
        let steps = taught(&["New Recording", "Settings"]);
        let result = aligned(
            vec![
                scene("Intro", &[("Welcome to Lumina.", &[])]),
                scene("Go", &[("Start a new recording, then open settings.", &[("new", "s0"), ("settings", "s1")])]),
            ],
            &steps,
        )
        .unwrap();
        assert_eq!(result.scene_starts, [Some(0), Some(0)]);
    }

    #[test]
    fn reordered_skipped_or_repeated_steps_fail_the_draft() {
        let steps = taught(&["New Recording", "Settings"]);
        let reordered = aligned(vec![scene("A", &[("Open settings, then start new.", &[("settings", "s1"), ("new", "s0")])])], &steps);
        assert!(reordered.unwrap_err().contains("where step 1"));
        let skipped = aligned(vec![scene("A", &[("Start a new one.", &[("new", "s0")])])], &steps);
        assert!(skipped.unwrap_err().contains("step 2 (“Settings”) isn't narrated"));
        let repeated = aligned(
            vec![scene("A", &[("New, then settings, then settings again.", &[("new", "s0"), ("settings", "s1"), ("settings", "s1")])])],
            &steps,
        );
        assert!(repeated.unwrap_err().contains("narrated twice"));
    }

    #[test]
    fn unknown_steps_and_missing_triggers_fail_the_draft() {
        let steps = taught(&["New Recording"]);
        let unknown = aligned(vec![scene("A", &[("Start a new recording.", &[("new", "s7")])])], &steps).unwrap_err();
        assert!(unknown.contains("unknown step s7"));
        let trigger = aligned(vec![scene("A", &[("Start a recording.", &[("banana", "s0")])])], &steps).unwrap_err();
        assert!(trigger.contains("trigger “banana”"));
        // A flagged step was never offered, so naming it is as wrong as naming a made-up one.
        let flagged = aligned(vec![scene("A", &[("Type here.", &[("type", "s1")])])], &taught(&["New Recording", ""])).unwrap_err();
        assert!(flagged.contains("unknown step s1"));
    }

    #[test]
    fn the_ai_sees_labels_and_headings_but_no_urls_or_flagged_steps() {
        let steps = taught(&["New Recording", "", "Settings"]);
        let sent = serde_json::to_string(&align_input(&steps)).unwrap();
        assert!(!sent.contains("http") && !sent.contains("secret-path"));
        assert!(sent.contains("\"s0\"") && sent.contains("\"s2\"") && !sent.contains("\"s1\""));
        assert!(sent.contains("H2"));
    }

    /// Live: needs a stored Gemini key. Run with `cargo test -p scenra gemini_aligns -- --ignored --nocapture`.
    #[test]
    #[ignore = "calls Gemini"]
    fn gemini_aligns_a_script_to_a_walkthrough() {
        let steps = taught(&["New Recording", "", "Start", "Settings", "Confirm"]);
        let aligned = Gemini::from_store().unwrap().align_script(&steps, "Show how to record a meeting and adjust settings").unwrap();
        println!("{}", serde_json::to_string_pretty(&aligned.scenario).unwrap());
        println!("scene starts: {:?}", aligned.scene_starts);
        let clicked: Vec<&str> = aligned.scenario.scenes.iter().flat_map(|s| &s.lines).flat_map(|l| &l.actions).map(|a| a.target.as_str()).collect();
        assert_eq!(clicked, ["New Recording", "Start", "Settings", "Confirm"]);
    }

    #[test]
    fn repeated_trigger_words_map_to_separate_occurrences() {
        let s = resolve(draft("Click next, then next again.", &[("next", "c1"), ("next", "c1")]), &controls());
        let words: Vec<usize> = s.scenes[0].lines[0].actions.iter().map(|a| a.word).collect();
        assert_eq!(words, vec![1, 3]);
    }
}
