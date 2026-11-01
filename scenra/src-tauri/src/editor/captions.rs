//! Captions for the finished video: transcript words mapped through the timeline into output
//! time (cut words drop out), grouped into cues of at most two 42-character lines and 6 s, and
//! written as SubRip (`.srt`) next to the export.

use super::assemble::is_filler_sound;
use super::model::Segment;
use super::transcript::Word;

const LINE: usize = 42;
const MAX_CUE: f64 = 6.0;
/// A silence this long starts a new cue.
const GAP: f64 = 0.7;

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Cues for the finished video. `words_of(take)` is the take's transcript, if it has one.
pub fn cues<'a>(segments: &[Segment], words_of: impl Fn(&str) -> Option<&'a [Word]>) -> Vec<Cue> {
    let placed: Vec<(f64, f64, &str)> = segments
        .iter()
        .flat_map(|s| {
            words_of(&s.take)
                .unwrap_or_default()
                .iter()
                // A word belongs to the segment its midpoint falls in. Every word left in the video
                // is captioned, flagged or not; only hesitation sounds (um, uh…) are left out.
                .filter(move |w| !is_filler_sound(&w.text) && (s.src_in..s.src_out).contains(&((w.start + w.end) / 2.0)))
                .map(move |w| {
                    let ms = |t: f64| (t * 1000.0).round() / 1000.0;
                    (ms(s.out_at + (w.start.max(s.src_in) - s.src_in)), ms(s.out_at + (w.end.min(s.src_out) - s.src_in)), w.text.as_str())
                })
        })
        .collect();

    let mut cues: Vec<Cue> = Vec::new();
    let mut prev_end = f64::NEG_INFINITY;
    for (start, end, text) in placed {
        let joinable = cues.last().is_some_and(|cue| {
            let sentence_end = cue.text.ends_with(['.', '?', '!']);
            fits(&format!("{} {text}", cue.text)) && end - cue.start <= MAX_CUE && start - prev_end <= GAP && !sentence_end
        });
        if joinable {
            let cue = cues.last_mut().unwrap();
            cue.text.push(' ');
            cue.text.push_str(text);
            cue.end = end;
        } else {
            cues.push(Cue { start, end, text: text.to_owned() });
        }
        prev_end = end;
    }
    cues
}

/// Where to break `text` into two lines of at most `LINE` characters, as evenly as possible.
fn split_at(text: &str) -> Option<usize> {
    let middle = text.len() / 2;
    text.match_indices(' ')
        .map(|(i, _)| i)
        .filter(|i| *i <= LINE && text.len() - i - 1 <= LINE)
        .min_by_key(|i| i.abs_diff(middle))
}

/// Whether `text` fits on one line or two.
fn fits(text: &str) -> bool {
    text.len() <= LINE || split_at(text).is_some()
}

/// Splits a cue into at most two lines, as evenly as the words allow.
fn lines(text: &str) -> String {
    if text.len() <= LINE {
        return text.to_owned();
    }
    match split_at(text) {
        Some(i) => format!("{}\n{}", &text[..i], &text[i + 1..]),
        None => text.to_owned(),
    }
}

fn stamp(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

pub fn srt(cues: &[Cue]) -> String {
    cues.iter()
        .enumerate()
        .map(|(i, c)| format!("{}\n{} --> {}\n{}\n", i + 1, stamp(c.start), stamp(c.end), lines(&c.text)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::transcript::tests::w;

    fn seg(take: &str, src_in: f64, src_out: f64, out_at: f64) -> Segment {
        Segment { scene: 0, take: take.into(), src_in, src_out, out_at }
    }

    #[test]
    fn words_follow_the_edit_and_cut_words_drop_out() {
        let words = [w("Let's", 1.0, 1.3), w("um", 1.4, 1.6), w("open", 2.0, 2.3), w("Settings.", 2.4, 2.9), w("Done.", 5.0, 5.4)];
        // Scene trimmed to 0.8 s, with 1.35–1.9 cut; a second take follows.
        let segments = [seg("a", 0.8, 1.35, 0.0), seg("a", 1.9, 3.0, 0.55), seg("b", 4.8, 5.6, 1.65)];
        let cues = cues(&segments, |t| (t == "a" || t == "b").then_some(&words[..]));
        assert_eq!(cues[0].text, "Let's open Settings.");
        assert_eq!((cues[0].start, cues[0].end), (0.2, 1.55));
        assert_eq!(cues[1].text, "Done.");
        assert!((cues[1].start - 1.85).abs() < 1e-9);
    }

    #[test]
    fn flagged_words_left_in_the_video_are_captioned() {
        let words = [w("start", 1.0, 1.3), Word { filler: true, ..w("a,", 1.4, 1.5) }, Word { filler: true, ..w("Um", 1.6, 1.7) }, w("new", 1.9, 2.1)];
        let cues = cues(&[seg("a", 0.0, 3.0, 0.0)], |_| Some(&words[..]));
        assert_eq!(cues[0].text, "start a, new");
    }

    #[test]
    fn long_runs_split_into_two_line_cues() {
        let words: Vec<Word> = (0..30).map(|i| w("word", f64::from(i) * 0.3, f64::from(i) * 0.3 + 0.25)).collect();
        let cues = cues(&[seg("a", 0.0, 10.0, 0.0)], |_| Some(&words[..]));
        assert!(cues.len() >= 2);
        let file = srt(&cues);
        assert!(file.starts_with("1\n00:00:00,000 --> "));
        for cue in file.split("\n\n") {
            assert!(cue.lines().skip(2).count() <= 2, "{cue}");
            assert!(cue.lines().skip(2).all(|l| l.len() <= LINE), "{cue}");
        }
    }

    #[test]
    fn timestamps_are_srt_format() {
        assert_eq!(stamp(3725.5), "01:02:05,500");
    }
}
