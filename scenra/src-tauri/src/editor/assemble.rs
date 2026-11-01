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
    /// Words that aren't in the script and aren't cut as stumbles (ad-libs, wrong words).
    pub extra_words: usize,
    pub failed_clicks: usize,
}

impl Score {
    fn value(&self) -> f64 {
        self.coverage * 100.0 - self.stumbles as f64 * 3.0 - self.extra_words as f64 * 2.0 - self.failed_clicks as f64 * 40.0
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
        if self.extra_words > 0 {
            parts.push(format!("{} extra word{}", self.extra_words, if self.extra_words == 1 { "" } else { "s" }));
        }
        if self.failed_clicks > 0 {
            parts.push(format!("{} failed click{}", self.failed_clicks, if self.failed_clicks == 1 { "" } else { "s" }));
        }
        format!("Take {attempt}: {}", parts.join(", "))
    }
}

/// Sounds that are never part of a script.
const FILLER_SOUNDS: [&str; 8] = ["um", "uh", "er", "erm", "uhm", "hmm", "mm", "ah"];

/// Whether `word` is a hesitation sound (um, uh…) rather than a word.
pub fn is_filler_sound(word: &str) -> bool {
    FILLER_SOUNDS.contains(&bare(word).as_str())
}

/// Returns, per word, whether it's part of a stumble to cut. No one signal is trusted alone:
/// - filler sounds (um, uh…) always;
/// - words Gemini flags, unless the script needs them ("start a, a new" keeps one "a");
/// - false starts found locally: a word repeated within three words, with only off-script words
///   before the repeat ("we are, we turn" → cut "we are,"), or a direct repeat ("the the").
fn analyze(script: &[String], words: &[Word]) -> Vec<bool> {
    let said: Vec<String> = words.iter().map(|w| bare(&w.text)).collect();
    let has_script = !script.is_empty();
    let matched = if has_script { on_script(script, &said) } else { vec![false; words.len()] };
    let mut stumble: Vec<bool> = (0..words.len())
        .map(|i| FILLER_SOUNDS.contains(&said[i].as_str()) || (words[i].filler && !matched[i]))
        .collect();
    for i in 0..words.len() {
        if said[i].is_empty() || stumble[i] {
            continue;
        }
        for j in i + 1..(i + 4).min(words.len()) {
            if said[j] != said[i] {
                continue;
            }
            if stumble[j] {
                break; // the repeat is already being cut, so this one is the keeper
            }
            let direct = j == i + 1;
            let between_off_script = (i + 1..j).all(|k| !matched[k] || stumble[k]);
            let script_repeats = matched[i] && matched[j];
            if (direct && !script_repeats) || (has_script && between_off_script && !script_repeats) {
                for flag in &mut stumble[i..j] {
                    *flag = true;
                }
            }
            break;
        }
    }
    stumble
}

/// Runs of consecutive stumble words, as index ranges (one stumble each).
fn stumble_runs(stumble: &[bool]) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < stumble.len() {
        if stumble[i] {
            let start = i;
            while i < stumble.len() && stumble[i] {
                i += 1;
            }
            runs.push(start..i);
        } else {
            i += 1;
        }
    }
    runs
}

pub fn score(script: &[String], words: &[Word], take: &TakeInfo) -> Score {
    let stumble = analyze(script, words);
    // Judged on the words that stay after the stumbles are cut.
    let kept: Vec<String> = words.iter().zip(&stumble).filter(|(_, s)| !**s).map(|(w, _)| bare(&w.text)).filter(|w| !w.is_empty()).collect();
    let (coverage, extra_words) = if script.is_empty() {
        (1.0, 0)
    } else {
        let matched = on_script(script, &kept);
        let hits = matched.iter().filter(|m| **m).count();
        (hits as f64 / script.len() as f64, kept.len() - hits)
    };
    Score {
        coverage,
        stumbles: stumble_runs(&stumble).len(),
        extra_words,
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

    // 3. Filler words and false starts: from the stumble's first word up to the next word kept.
    if let Some(words) = chosen.words {
        for range in stumble_runs(&analyze(script, words)) {
            let run = &words[range.clone()];
            let next = words.get(range.end).map(|w| w.start - 0.02);
            let to = next.unwrap_or(run[run.len() - 1].end + 0.02).max(run[run.len() - 1].end).min(take.duration);
            let (from, to) = ((run[0].start - 0.02).max(0.0), to);
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

    /// Live (offline): runs Assemble on `SCENRA_ASSEMBLE_DIR` using its cached transcripts only.
    /// Prints the decisions; writes nothing and sends nothing.
    #[test]
    #[ignore = "reads a local session"]
    fn assembles_a_recorded_session_from_cached_transcripts() {
        use crate::editor::library;
        let dir = std::path::PathBuf::from(std::env::var("SCENRA_ASSEMBLE_DIR").expect("set SCENRA_ASSEMBLE_DIR"));
        let data = library::load(&dir).unwrap();
        for scene in &data.edit.scenes {
            let candidates: Vec<Candidate> = data
                .takes
                .iter()
                .filter(|t| t.scene == scene.scene)
                .map(|t| Candidate { take: t, words: data.transcripts.get(&t.folder).map(Vec::as_slice) })
                .collect();
            assert!(candidates.iter().all(|c| c.words.is_some()), "scene {} has an untranscribed take", scene.scene + 1);
            let lines: Vec<String> = data.script.as_ref().unwrap().scenes[scene.scene].lines.iter().map(|l| l.text.clone()).collect();
            let script = script_words(&lines);
            for c in &candidates {
                let s = score(&script, c.words.unwrap(), c.take);
                eprintln!("scene {} {}: {}", scene.scene + 1, c.take.folder, s.describe(c.take.attempt, true));
            }
            let out = assemble_scene(scene, &candidates, &script, |_| vec![]);
            let words = data.transcripts[out.take.as_ref().unwrap()].clone();
            eprintln!("  → {} | trims {:.2}–{:?}", out.take_note.as_deref().unwrap_or("-"), out.trim_in, out.trim_out);
            for cut in &out.cuts {
                let reason = match &cut.source { Source::Ai { reason } => reason.as_str(), Source::Manual => "manual" };
                eprintln!("  cut {:.2}–{:.2} {reason}", cut.from, cut.to);
            }
            let kept: Vec<&str> = words
                .iter()
                .filter(|w| {
                    let mid = (w.start + w.end) / 2.0;
                    mid >= out.trim_in && out.trim_out.is_none_or(|e| mid <= e) && !out.cuts.iter().any(|c| mid >= c.from && mid <= c.to)
                })
                .map(|w| w.text.as_str())
                .collect();
            eprintln!("  kept: {}", kept.join(" "));
        }
    }

    #[test]
    fn a_false_start_is_cut_even_when_gemini_misses_it() {
        // Script: "here we turn on notes". Said: "Here we are, we turn on notes", nothing flagged.
        let script = line(&["Here", "we", "turn", "on", "notes"]);
        let words = vec![w("Here", 1.0, 1.3), w("we", 1.5, 1.7), w("are,", 1.8, 2.0), w("we", 2.4, 2.6), w("turn", 2.7, 3.0), w("on", 3.0, 3.2), w("notes", 3.3, 3.8)];
        let t = take("scene-02-take-1", 1, 5.0);
        let s = score(&script, &words, &t);
        assert_eq!((s.coverage, s.stumbles, s.extra_words), (1.0, 1, 0));
        let out = assemble_scene(&SceneEdit::default(), &[Candidate { take: &t, words: Some(&words) }], &script, |_| vec![]);
        let cut = out.cuts.iter().find(|c| matches!(&c.source, Source::Ai { reason } if reason.contains("stumble"))).expect("a stumble cut");
        assert_eq!((cut.from, cut.to), (1.48, 2.38), "cut “we are,” up to the restart");
    }

    #[test]
    fn a_flagged_word_the_script_needs_is_kept() {
        // "start a, a new": Gemini flags both a's; one is needed.
        let script = line(&["start", "a", "new", "recording"]);
        let words = vec![w("start", 1.0, 1.3), filler("a", 1.4, 1.5), filler("a", 1.7, 1.8), w("new", 1.9, 2.1), w("recording", 2.2, 2.7)];
        assert_eq!(analyze(&script, &words).iter().filter(|s| **s).count(), 1);
        assert_eq!(score(&script, &words, &take("t", 1, 3.0)).coverage, 1.0);
    }

    #[test]
    fn script_repeats_and_plain_speech_are_not_stumbles() {
        let script = line(&["that", "that", "works"]);
        let words = vec![w("that", 0.0, 0.2), w("that", 0.3, 0.5), w("works", 0.6, 0.9)];
        assert!(analyze(&script, &words).iter().all(|s| !s));
        // Without a script, only direct repeats count.
        let words = vec![w("we", 0.0, 0.2), w("open", 0.3, 0.5), w("we", 0.6, 0.9)];
        assert!(analyze(&[], &words).iter().all(|s| !s));
        let words = vec![w("the", 0.0, 0.2), w("the", 0.3, 0.5), w("menu", 0.6, 0.9)];
        assert_eq!(analyze(&[], &words), vec![true, false, false]);
    }

    #[test]
    fn extra_words_lower_the_score() {
        let script = line(&["open", "settings"]);
        let t = take("t", 1, 3.0);
        let clean = score(&script, &[w("open", 0.0, 0.2), w("settings", 0.3, 0.6)], &t);
        let chatty = score(&script, &[w("so", 0.0, 0.1), w("open", 0.2, 0.4), w("those", 0.5, 0.7), w("settings", 0.8, 1.0)], &t);
        assert_eq!(chatty.extra_words, 2);
        assert!(clean.value() > chatty.value());
        assert!(chatty.describe(1, true).contains("2 extra words"));
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
        assert_eq!((stumbles[0].from, stumbles[0].to), (1.28, 1.68), "up to the next word");
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
