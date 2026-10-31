//! Assemble with AI: from the takes' transcripts, a first-draft edit the presenter then adjusts.
//!
//! For each scene: pick the take that says the most of the script with the fewest stumbles and no
//! failed clicks, trim dead air, shorten long pauses, and cut filler words and false starts. Every
//! change is tagged with its reason; nothing ever cuts through a click.

use super::audio;
use super::library::TakeInfo;
use super::model::{Cut, SceneEdit, Source};
use super::transcript::Word;

/// A take Assemble can choose for a scene, with what's known about it.
pub struct Candidate<'a> {
    pub take: &'a TakeInfo,
    pub words: Option<&'a [Word]>,
}

/// Lower-cased word without punctuation.
pub fn bare(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>().to_lowercase()
}

pub fn script_words(lines: &[String]) -> Vec<String> {
    lines.iter().flat_map(|l| l.split_whitespace().map(bare)).filter(|w| !w.is_empty()).collect()
}

/// For each said word, whether it lines up with the script (longest common subsequence).
pub fn on_script(script: &[String], said: &[String]) -> Vec<bool> {
    let (n, m) = (script.len(), said.len());
    let mut table = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if script[i] == said[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let mut matched = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if script[i] == said[j] {
            matched[j] = true;
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matched
}

#[derive(Debug, PartialEq)]
pub struct Score {
    /// Share of the script's words this take says, 0–1 (1 without a script).
    pub coverage: f64,
    pub stumbles: usize,
    pub failed_clicks: usize,
}

impl Score {
    fn value(&self) -> f64 {
        self.coverage * 100.0 - self.stumbles as f64 * 3.0 - self.failed_clicks as f64 * 40.0
    }

    fn describe(&self, attempt: u32, has_script: bool) -> String {
        let mut parts = Vec::new();
        if has_script {
            parts.push(format!("says {:.0}% of the script", self.coverage * 100.0));
        }
        parts.push(match self.stumbles {
            0 => "no stumbles".to_owned(),
            1 => "1 stumble".to_owned(),
            n => format!("{n} stumbles"),
        });
        if self.failed_clicks > 0 {
            parts.push(format!("{} failed click{}", self.failed_clicks, if self.failed_clicks == 1 { "" } else { "s" }));
        }
        format!("Take {attempt}: {}", parts.join(", "))
    }
}

/// Groups of consecutive filler words (one stumble each).
fn filler_runs(words: &[Word]) -> Vec<&[Word]> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if words[i].filler {
            let start = i;
            while i < words.len() && words[i].filler {
                i += 1;
            }
            runs.push(&words[start..i]);
        } else {
            i += 1;
        }
    }
    runs
}

pub fn score(script: &[String], words: &[Word], take: &TakeInfo) -> Score {
    let said: Vec<String> = words.iter().filter(|w| !w.filler).map(|w| bare(&w.text)).collect();
    let coverage = if script.is_empty() {
        1.0
    } else {
        on_script(script, &said).iter().filter(|m| **m).count() as f64 / script.len() as f64
    };
    Score {
        coverage,
        stumbles: filler_runs(words).len(),
        failed_clicks: take.actions.iter().filter(|a| a.kind == "press" && !a.ok).count(),
    }
}

/// Speech regions from the words (more precise than the mic level when there's room noise).
fn speech_from_words(words: &[Word], duration: f64) -> Vec<(f64, f64)> {
    let mut regions: Vec<(f64, f64)> = Vec::new();
    for w in words.iter().filter(|w| !w.filler) {
        let (a, b) = ((w.start - 0.15).max(0.0), (w.end + 0.15).min(duration));
        match regions.last_mut() {
            Some(last) if a <= last.1 + 0.3 => last.1 = last.1.max(b),
            _ => regions.push((a, b)),
        }
    }
    regions
}

/// The draft edit of one scene. `fallback_speech` is the mic-level speech map of each candidate,
/// used when a take has no transcript.
pub fn assemble_scene(scene: &SceneEdit, candidates: &[Candidate], script: &[String], fallback_speech: impl Fn(&TakeInfo) -> Vec<(f64, f64)>) -> SceneEdit {
    let mut out = scene.clone();
    if candidates.is_empty() {
        return out;
    }
    // 1. The take: best score if every candidate has a transcript, else keep the current one.
    let chosen = if candidates.iter().all(|c| c.words.is_some()) {
        let scores: Vec<Score> = candidates.iter().map(|c| score(script, c.words.unwrap_or_default(), c.take)).collect();
        let best = (0..candidates.len())
            .max_by(|&a, &b| scores[a].value().total_cmp(&scores[b].value()).then(candidates[a].take.attempt.cmp(&candidates[b].take.attempt)))
            .unwrap_or(0);
        out.take_note = Some(scores[best].describe(candidates[best].take.attempt, !script.is_empty()));
        &candidates[best]
    } else {
        out.take_note = None;
        candidates.iter().find(|c| Some(&c.take.folder) == scene.take.as_ref()).unwrap_or(&candidates[candidates.len() - 1])
    };
    if out.take.as_ref() != Some(&chosen.take.folder) {
        out.cuts.clear(); // the presenter's cuts were times in another take
        out.take = Some(chosen.take.folder.clone());
    }
    let take = chosen.take;

    // 2. Trims and long pauses.
    let speech = match chosen.words {
        Some(words) if words.iter().any(|w| !w.filler) => speech_from_words(words, take.duration),
        _ => fallback_speech(take),
    };
    out.trim_in = 0.0;
    out.trim_out = None;
    out = audio::tidy(&out, take.duration, &speech, &take.actions);

    // 3. Filler words and false starts.
    if let Some(words) = chosen.words {
        for run in filler_runs(words) {
            let (from, to) = ((run[0].start - 0.02).max(0.0), (run[run.len() - 1].end + 0.02).min(take.duration));
            let inside = from >= out.trim_in && out.trim_out.is_none_or(|end| to <= end);
            let through_click = take.actions.iter().any(|a| from < a.end && to > a.start);
            if !inside || through_click {
                continue;
            }
            let said: Vec<&str> = run.iter().map(|w| w.text.as_str()).collect();
            out.cuts.push(Cut {
                from: (from * 1000.0).round() / 1000.0,
                to: (to * 1000.0).round() / 1000.0,
                source: Source::Ai { reason: format!("stumble: “{}”", said.join(" ")) },
                enabled: true,
            });
        }
        out.cuts.sort_by(|a, b| a.from.total_cmp(&b.from));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::transcript::tests::w;
    use crate::take::ActionRecord;

    fn take(folder: &str, attempt: u32, duration: f64) -> TakeInfo {
        TakeInfo { folder: folder.into(), scene: 0, attempt, duration, has_actions: true, actions: vec![] }
    }

    fn filler(text: &str, start: f64, end: f64) -> Word {
        Word { filler: true, ..w(text, start, end) }
    }

    fn line(words: &[&str]) -> Vec<String> {
        script_words(&[words.join(" ")])
    }

    #[test]
    fn script_matching_ignores_case_punctuation_and_extra_words() {
        let script = line(&["Let's", "open", "Settings."]);
        let said: Vec<String> = ["so", "let's", "open", "the", "settings"].iter().map(|s| bare(s)).collect();
        assert_eq!(on_script(&script, &said), vec![false, true, true, false, true]);
    }

    #[test]
    fn the_take_that_says_the_script_cleanly_wins() {
        let script = line(&["open", "the", "settings", "now"]);
        let a = take("scene-01-take-1", 1, 5.0);
        let b = take("scene-01-take-2", 2, 5.0);
        let c = take("scene-01-take-3", 3, 5.0);
        let a_words = vec![w("open", 1.0, 1.3), w("settings", 1.4, 1.8)]; // skipped words
        let b_words = vec![w("open", 1.0, 1.3), w("the", 1.3, 1.4), w("settings", 1.4, 1.8), w("now", 1.9, 2.1)];
        let c_words = vec![filler("um", 0.5, 0.7), w("open", 1.0, 1.3), w("the", 1.3, 1.4), w("settings", 1.4, 1.8), w("now", 1.9, 2.1)];
        let candidates = [
            Candidate { take: &a, words: Some(&a_words) },
            Candidate { take: &b, words: Some(&b_words) },
            Candidate { take: &c, words: Some(&c_words) },
        ];
        let out = assemble_scene(&SceneEdit::default(), &candidates, &script, |_| vec![]);
        assert_eq!(out.take.as_deref(), Some("scene-01-take-2"));
        assert_eq!(out.take_note.as_deref(), Some("Take 2: says 100% of the script, no stumbles"));
    }

    #[test]
    fn a_failed_click_loses_to_a_clean_take() {
        let mut bad = take("scene-01-take-2", 2, 5.0);
        bad.actions = vec![ActionRecord { kind: "press".into(), label: "Settings".into(), start: 1.0, end: 1.2, ok: false }];
        let good = take("scene-01-take-1", 1, 5.0);
        let words = vec![w("open", 1.0, 1.3)];
        let candidates = [Candidate { take: &good, words: Some(&words) }, Candidate { take: &bad, words: Some(&words) }];
        let out = assemble_scene(&SceneEdit::default(), &candidates, &line(&["open"]), |_| vec![]);
        assert_eq!(out.take.as_deref(), Some("scene-01-take-1"));
    }

    #[test]
    fn stumbles_are_cut_but_never_through_a_click() {
        let mut t = take("scene-01-take-1", 1, 8.0);
        t.actions = vec![ActionRecord { kind: "press".into(), label: "Settings".into(), start: 4.0, end: 4.3, ok: true }];
        let words = vec![
            w("let's", 1.0, 1.2),
            filler("um", 1.3, 1.6),
            w("open", 1.7, 2.0),
            filler("the", 3.9, 4.1), // over the click: kept
            w("settings", 4.4, 4.9),
        ];
        let candidates = [Candidate { take: &t, words: Some(&words) }];
        let out = assemble_scene(&SceneEdit { take: Some(t.folder.clone()), ..Default::default() }, &candidates, &[], |_| vec![]);
        let stumbles: Vec<&Cut> = out.cuts.iter().filter(|c| matches!(&c.source, Source::Ai { reason } if reason.starts_with("stumble"))).collect();
        assert_eq!(stumbles.len(), 1);
        assert_eq!((stumbles[0].from, stumbles[0].to), (1.28, 1.62));
        assert!(out.trim_in >= 0.5, "dead air before 1.0 s is trimmed: {}", out.trim_in);
    }

    #[test]
    fn without_transcripts_the_current_take_is_kept_and_tidied() {
        let a = take("scene-01-take-1", 1, 6.0);
        let b = take("scene-01-take-2", 2, 6.0);
        let candidates = [Candidate { take: &a, words: None }, Candidate { take: &b, words: None }];
        let scene = SceneEdit { take: Some("scene-01-take-1".into()), ..Default::default() };
        let out = assemble_scene(&scene, &candidates, &[], |_| vec![(2.0, 3.0)]);
        assert_eq!(out.take.as_deref(), Some("scene-01-take-1"));
        assert_eq!(out.take_note, None);
        assert_eq!(out.trim_in, 1.7);
    }
}
