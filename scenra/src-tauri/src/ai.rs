//! AiProvider (PROJECT_OUTLINE §4): Gemini drafts a narrated script from a page outline.
//!
//! Actions are pinned to a word in the script, never to a time. The model names the trigger word
//! and a control id from the outline; Scenra resolves and checks both, and drops what doesn't
//! match the real page rather than guessing.

use crate::page::{Control, PageOutline};
use crate::secrets::{self, KeySource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

pub const GEMINI_MODEL: &str = "gemini-flash-latest";
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    pub title: String,
    pub scenes: Vec<Scene>,
    /// Actions Scenra dropped because they didn't match the script or the page.
    pub warnings: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub title: String,
    pub lines: Vec<Line>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub text: String,
    pub actions: Vec<Action>,
}

#[derive(Clone, Serialize, Deserialize)]
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
}

pub struct Gemini {
    key: String,
    model: &'static str,
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
    AiStatus { configured: source.is_some(), source, model: GEMINI_MODEL }
}

impl Gemini {
    pub fn from_store() -> Result<Self, String> {
        let (key, _) = secrets::gemini_key().ok_or(format!(
            "no Gemini key: run `cmdkey /generic:{} /user:gemini /pass:<key>` or set {}",
            secrets::GEMINI_TARGET,
            secrets::GEMINI_ENV
        ))?;
        Ok(Self { key, model: GEMINI_MODEL })
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

/// Gemini's response schema (OpenAPI subset). Mirrors the draft types below.
fn schema() -> Value {
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
                  "required": ["trigger", "control"],
                  "properties": {
                    "trigger": { "type": "STRING" },
                    "control": { "type": "STRING" }
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
    control: String,
}

impl AiProvider for Gemini {
    fn draft_script(&self, page: &PageOutline, brief: &str) -> Result<Scenario, String> {
        let prompt = format!(
            "Brief: {brief}\n\nPage outline (JSON):\n{}\n\n\
             Write 3 to 5 scenes, 2 to 4 lines each, about 60 to 90 seconds read aloud in total. \
             Use 4 to 8 clicks across the whole script.",
            serde_json::to_string_pretty(page).map_err(|e| e.to_string())?
        );
        let body = json!({
            "systemInstruction": { "parts": [{ "text": SYSTEM }] },
            "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema(),
                "temperature": 0.7
            }
        });

        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(90)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut response = agent
            .post(format!("{ENDPOINT}/{}:generateContent", self.model))
            .header("x-goog-api-key", &self.key)
            .send_json(&body)
            .map_err(|e| format!("Gemini request failed: {e}"))?;
        let status = response.status();
        let reply: Value = response.body_mut().read_json().map_err(|e| format!("Gemini reply: {e}"))?;
        if !status.is_success() {
            let message = reply["error"]["message"].as_str().unwrap_or("unknown error");
            return Err(format!("Gemini {status}: {message}"));
        }
        let text = reply["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or("Gemini returned no script")?;
        let draft: Draft = serde_json::from_str(text).map_err(|e| format!("Gemini script: {e}"))?;
        Ok(resolve(draft, &page.controls))
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

    #[test]
    fn repeated_trigger_words_map_to_separate_occurrences() {
        let s = resolve(draft("Click next, then next again.", &[("next", "c1"), ("next", "c1")]), &controls());
        let words: Vec<usize> = s.scenes[0].lines[0].actions.iter().map(|a| a.word).collect();
        assert_eq!(words, vec![1, 3]);
    }
}
