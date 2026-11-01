//! What the presenter said in each take, word by word, with times: the basis for picking the best
//! take, cutting stumbles, editing by text, and captions.
//!
//! Gemini transcribes the take's mic (sent only after the presenter agreed; see `Consent`). The
//! audio goes as 16 kHz mono WAV, about 1.9 MB a minute. The result is cached as
//! `<take>/transcript.json`, keyed by the WAV's size and modified time, so it's sent once.

use super::audio::{self, Pcm};
use crate::ai::Gemini;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const FILE: &str = "transcript.json";
/// Inline audio has to fit one request (about 20 MB with base64); 10 minutes is about 19 MB raw.
const MAX_SECONDS: f64 = 9.0 * 60.0;

/// Whether the presenter allows narration audio to be sent to Gemini.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Consent {
    #[default]
    Ask,
    Allowed,
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
    /// "um", "uh", a repeated word or a false start: something to cut.
    #[serde(default)]
    pub filler: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub words: Vec<Word>,
    wav_bytes: u64,
    wav_modified: u64,
}

pub trait Transcriber {
    /// `wav` is 16 kHz mono 16-bit WAV of `seconds` length.
    fn transcribe(&self, wav: &[u8], seconds: f64) -> Result<Vec<Word>, String>;
}

const SYSTEM: &str = "You transcribe narration recorded for a software demo video. \
Transcribe exactly what is said, verbatim, in the spoken language, including filler words \
(um, uh, er, like used as filler), repeated words and false starts. Never correct, summarise or \
skip anything. Give every word its start and end time in seconds from the start of the audio, \
as precisely as you can. Mark `filler` true for words an editor would cut: filler sounds, a \
word repeated by mistake, and every word of a false start that the speaker then restarts. \
If nothing is said, return no words.";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "words": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "start": { "type": "number" },
                        "end": { "type": "number" },
                        "filler": { "type": "boolean" }
                    },
                    "required": ["text", "start", "end", "filler"]
                }
            }
        },
        "required": ["words"]
    })
}

impl Transcriber for Gemini {
    fn transcribe(&self, wav: &[u8], seconds: f64) -> Result<Vec<Word>, String> {
        #[derive(Deserialize)]
        struct Reply {
            words: Vec<Word>,
        }
        let prompt = format!("The audio is {seconds:.2} seconds long. Return the words in order.");
        let text = Gemini::transcribe(self, wav, SYSTEM, &prompt, schema())?;
        let reply: Reply = serde_json::from_str(&text).map_err(|e| format!("Gemini transcript: {e}"))?;
        Ok(reply.words)
    }
}

/// 16 kHz mono WAV bytes from any 16-bit WAV (box-filtered decimation; fine for speech).
pub fn to_16k_mono(pcm: &Pcm) -> Vec<u8> {
    let c = usize::from(pcm.channels.max(1));
    let mono: Vec<f32> = pcm.samples.chunks(c).map(|f| f.iter().sum::<f32>() / c as f32).collect();
    let step = f64::from(pcm.rate) / 16_000.0;
    let n = (mono.len() as f64 / step) as usize;
    let data: Vec<u8> = (0..n)
        .flat_map(|i| {
            let (a, b) = ((i as f64 * step) as usize, (((i + 1) as f64 * step) as usize).min(mono.len()).max((i as f64 * step) as usize + 1));
            let avg = mono[a..b].iter().sum::<f32>() / (b - a) as f32;
            ((avg.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()
        })
        .collect();
    let mut wav = crate::recorder::wav_header(16_000, 1, data.len() as u32).to_vec();
    wav.extend(data);
    wav
}

/// Times in order, inside the take, each word at least 40 ms long and not before the previous one.
pub fn sanitize(mut words: Vec<Word>, duration: f64) -> Vec<Word> {
    words.retain(|w| !w.text.trim().is_empty() && w.start.is_finite() && w.end.is_finite());
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut last = 0.0f64;
    for w in &mut words {
        w.text = w.text.trim().to_owned();
        w.start = w.start.clamp(last, duration);
        w.end = w.end.clamp(w.start + 0.04, duration.max(w.start + 0.04));
        w.start = (w.start * 1000.0).round() / 1000.0;
        w.end = (w.end * 1000.0).round() / 1000.0;
        last = w.start;
    }
    words
}

/// Gemini's word times can drift by a few hundred ms. Pull each word inside the speech region
/// it overlaps most (from the mic level), so a word never seems to run into a pause or past the
/// end of the take. Words in no region at all are left as they are.
pub fn snap(mut words: Vec<Word>, speech: &[(f64, f64)]) -> Vec<Word> {
    for w in &mut words {
        let overlap = |r: &(f64, f64)| (w.end.min(r.1) - w.start.max(r.0)).max(0.0);
        let nearest = |r: &(f64, f64)| if w.end < r.0 { r.0 - w.end } else if w.start > r.1 { w.start - r.1 } else { 0.0 };
        let best = speech
            .iter()
            .max_by(|a, b| overlap(a).total_cmp(&overlap(b)).then(nearest(b).total_cmp(&nearest(a))));
        if let Some(&(a, b)) = best.filter(|r| overlap(r) > 0.0 || nearest(r) < 0.6) {
            let length = (w.end - w.start).min(b - a);
            w.start = w.start.clamp(a, b - length);
            w.end = (w.start + length).min(b);
            w.start = (w.start * 1000.0).round() / 1000.0;
            w.end = (w.end * 1000.0).round() / 1000.0;
        }
    }
    // Snapping moves words independently; keep them in spoken order.
    let mut last = 0.0f64;
    for w in &mut words {
        w.start = w.start.max(last);
        w.end = w.end.max(w.start + 0.01);
        last = w.start;
    }
    words
}

/// A recording's size and modified time (s): what caches made from it check before reuse.
pub fn key(wav: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(wav).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some((meta.len(), modified))
}

/// The cached transcript of a take, if it still matches its mic recording.
pub fn cached(take_dir: &Path, wav: &Path) -> Option<Transcript> {
    let t: Transcript = crate::session::read_json(&take_dir.join(FILE))?;
    (Some((t.wav_bytes, t.wav_modified)) == key(wav)).then_some(t)
}

/// Transcribes a take (unless cached) and caches the result.
pub fn transcribe_take(take_dir: &Path, wav: &Path, with: &dyn Transcriber) -> Result<Transcript, String> {
    if let Some(t) = cached(take_dir, wav) {
        return Ok(t);
    }
    let pcm = audio::read_wav(wav)?;
    let seconds = pcm.seconds();
    if seconds > MAX_SECONDS {
        return Err(format!("{} is over {} minutes, too long to transcribe in one go", take_dir.display(), MAX_SECONDS / 60.0));
    }
    let words = snap(sanitize(with.transcribe(&to_16k_mono(&pcm), seconds)?, seconds), &audio::speech(&pcm));
    let (wav_bytes, wav_modified) = key(wav).ok_or("the mic recording disappeared")?;
    let transcript = Transcript { words, wav_bytes, wav_modified };
    crate::session::write_json(&take_dir.join(FILE), &transcript)?;
    Ok(transcript)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn w(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end, filler: false }
    }

    pub(crate) struct Fake(pub Vec<Word>);
    impl Transcriber for Fake {
        fn transcribe(&self, wav: &[u8], _seconds: f64) -> Result<Vec<Word>, String> {
            assert_eq!(&wav[..4], b"RIFF");
            Ok(self.0.clone())
        }
    }

    /// Live: sends `SCENRA_TTS_WAV` (synthetic speech, never a real recording) to Gemini.
    #[test]
    #[ignore = "calls Gemini"]
    fn transcribes_speech_with_gemini() {
        let wav = std::path::PathBuf::from(std::env::var("SCENRA_TTS_WAV").expect("set SCENRA_TTS_WAV"));
        let pcm = audio::read_wav(&wav).unwrap();
        let gemini = Gemini::from_store().unwrap();
        let words = sanitize(Transcriber::transcribe(&gemini, &to_16k_mono(&pcm), pcm.seconds()).unwrap(), pcm.seconds());
        for w in &words {
            eprintln!("{:>6.2} {:>6.2} {}{}", w.start, w.end, w.text, if w.filler { "  [filler]" } else { "" });
        }
        let said: Vec<String> = words.iter().map(|w| crate::editor::assemble::bare(&w.text)).collect();
        assert!(said.contains(&"settings".to_owned()) && said.contains(&"confirm".to_owned()), "{said:?}");
        let settings = words.iter().find(|w| crate::editor::assemble::bare(&w.text) == "settings").unwrap();
        assert!(settings.start > 1.5 && settings.end < 6.0, "settings at {} s", settings.start);
        assert!(words.iter().any(|w| w.filler), "the “um” should be marked as filler");
        // Optionally cache it into a take folder whose microphone.wav is this same file.
        if let Some(take) = std::env::var_os("SCENRA_TTS_TAKE").map(std::path::PathBuf::from) {
            let cached = transcribe_take(&take, &take.join("microphone.wav"), &Fake(words.clone())).unwrap();
            assert_eq!(cached.words.len(), words.len());
        }
    }

    #[test]
    fn sanitize_orders_clamps_and_drops_empty_words() {
        let words = vec![w("b", 2.0, 1.5), w(" ", 0.0, 1.0), w("a", 0.5, 0.9), w("c", 9.0, 12.0)];
        let out = sanitize(words, 10.0);
        assert_eq!(out.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!((out[1].start, out[1].end), (2.0, 2.04));
        assert_eq!((out[2].start, out[2].end), (9.0, 10.0));
    }

    #[test]
    fn words_are_pulled_inside_the_speech_they_belong_to() {
        let speech = [(1.35, 5.25), (7.85, 11.4)];
        let words = snap(vec![w("panel.", 4.88, 5.42), w("Now", 8.03, 8.35), w("done.", 11.23, 11.62), w("hm", 20.0, 20.2)], &speech);
        assert_eq!((words[0].start, words[0].end), (4.71, 5.25));
        assert_eq!((words[1].start, words[1].end), (8.03, 8.35));
        assert_eq!((words[2].start, words[2].end), (11.01, 11.4));
        assert_eq!((words[3].start, words[3].end), (20.0, 20.2)); // nowhere near speech: unchanged
    }

    #[test]
    fn snapping_keeps_spoken_order() {
        let words = snap(vec![w("right", 10.80, 11.1), w("now.", 10.9, 11.4)], &[(9.0, 11.2)]);
        assert!(words[0].start <= words[1].start, "{words:?}");
    }

    #[test]
    fn downsamples_to_16k_mono() {
        let pcm = Pcm { rate: 48_000, channels: 2, samples: vec![0.5; 48_000 * 2] };
        let wav = to_16k_mono(&pcm);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(wav.len(), 44 + 16_000 * 2);
    }

    #[test]
    fn transcripts_are_cached_until_the_recording_changes() {
        let dir = std::env::temp_dir().join(format!("scenra-transcript-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("microphone.wav");
        audio::write_wav(&wav, &Pcm { rate: 48_000, channels: 1, samples: vec![0.0; 48_000] }).unwrap();
        let first = transcribe_take(&dir, &wav, &Fake(vec![w("hello", 0.1, 0.4)])).unwrap();
        assert_eq!(first.words.len(), 1);
        // A second call doesn't ask again.
        let again = transcribe_take(&dir, &wav, &Fake(vec![])).unwrap();
        assert_eq!(again.words, first.words);
        // A changed recording does.
        audio::write_wav(&wav, &Pcm { rate: 48_000, channels: 1, samples: vec![0.0; 24_000] }).unwrap();
        assert!(cached(&dir, &wav).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
