//! Title, description and chapters for publishing the finished video (YouTube and similar).
//!
//! Chapters come from the timeline: where each scene starts in the finished video, titled from
//! the script. Gemini writes the title and description from the script's narration (text it
//! wrote in the first place), or from the transcripts when there's no script (those exist only
//! if the presenter allowed transcription).

use super::model::Segment;
use crate::ai::Gemini;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub title: String,
    pub description: String,
    /// `00:00 Intro` lines.
    pub chapters: String,
    /// Why YouTube might not show the chapters, if so.
    pub chapter_note: Option<String>,
}

fn clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// One chapter per scene that's in the video, at the scene's first segment.
pub fn chapters(segments: &[Segment], duration: f64, title_of: impl Fn(usize) -> String) -> (String, Option<String>) {
    let mut starts: Vec<(usize, f64)> = Vec::new();
    for s in segments {
        if starts.last().is_none_or(|(scene, _)| *scene != s.scene) {
            starts.push((s.scene, s.out_at));
        }
    }
    if let Some(first) = starts.first_mut() {
        first.1 = 0.0; // YouTube requires the first chapter at 00:00
    }
    let lines: Vec<String> = starts.iter().map(|(scene, at)| format!("{} {}", clock(*at), title_of(*scene))).collect();
    let lengths: Vec<f64> = starts.iter().enumerate().map(|(i, (_, at))| starts.get(i + 1).map_or(duration, |n| n.1) - at).collect();
    let note = if starts.len() < 3 {
        Some("YouTube shows chapters only when there are at least 3".to_owned())
    } else if lengths.iter().any(|l| *l < 10.0) {
        Some("YouTube shows chapters only when each is at least 10 seconds long".to_owned())
    } else {
        None
    };
    (lines.join("\n"), note)
}

/// Asks Gemini for a title and description from `material` (narration per scene).
pub fn write(gemini: &Gemini, material: &str) -> Result<(String, String), String> {
    #[derive(Deserialize)]
    struct Reply {
        title: String,
        description: String,
    }
    let system = "You write the title and description for a software demo video. The title is at \
        most 70 characters, plain and specific, no clickbait, no emoji. The description is 2 to 4 \
        short paragraphs in plain language: what the video shows and who it's for. Use only what \
        the narration says; don't invent features, links or prices.";
    let schema = json!({
        "type": "object",
        "properties": { "title": { "type": "string" }, "description": { "type": "string" } },
        "required": ["title", "description"]
    });
    let text = gemini.describe(system, &format!("The video's narration, scene by scene:\n\n{material}"), schema)?;
    let reply: Reply = serde_json::from_str(&text).map_err(|e| format!("Gemini description: {e}"))?;
    Ok((reply.title.trim().to_owned(), spaced(reply.description.trim())))
}

/// Puts back the space Gemini sometimes drops between sentences ("panel.It" → "panel. It").
fn spaced(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev = None;
    for c in text.chars() {
        if matches!(prev, Some('.' | '!' | '?')) && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(scene: usize, out_at: f64) -> Segment {
        Segment { scene, take: "t".into(), src_in: 0.0, src_out: 1.0, out_at }
    }

    #[test]
    fn one_chapter_per_scene_starting_at_zero() {
        let segments = [seg(0, 0.4), seg(0, 5.0), seg(1, 14.0), seg(2, 75.5)];
        let (text, note) = chapters(&segments, 100.0, |s| format!("Scene {}", s + 1));
        assert_eq!(text, "00:00 Scene 1\n00:14 Scene 2\n01:16 Scene 3");
        assert_eq!(note, None);
    }

    #[test]
    fn short_or_few_chapters_get_a_note() {
        let (_, few) = chapters(&[seg(0, 0.0), seg(1, 20.0)], 60.0, |_| "x".into());
        assert!(few.unwrap().contains("at least 3"));
        let (_, short) = chapters(&[seg(0, 0.0), seg(1, 5.0), seg(2, 30.0)], 60.0, |_| "x".into());
        assert!(short.unwrap().contains("10 seconds"));
    }

    #[test]
    fn run_together_sentences_get_their_space_back() {
        assert_eq!(spaced("Open the panel.It works! Version 2.5 ships."), "Open the panel. It works! Version 2.5 ships.");
    }

    #[test]
    fn long_videos_use_hours() {
        assert_eq!(clock(3725.0), "1:02:05");
    }
}
