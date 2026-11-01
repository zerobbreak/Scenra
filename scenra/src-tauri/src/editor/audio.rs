//! Local audio analysis on the 16-bit mic WAVs (no network, no consent needed):
//!
//! - **Speech map**: 20 ms RMS frames against an adaptive noise floor → where the presenter speaks.
//! - **Tidy up**: dead air before the first word or click and after the last is trimmed, and
//!   pauses longer than 1.5 s are shortened, never through a click.
//! - **Loudness**: integrated loudness (ITU-R BS.1770 K-weighting with gating), levelled to
//!   −16 LUFS with a peak limiter at −1 dBFS, written as a copy for export.

use super::model::{Cut, SceneEdit, Source};
use crate::take::ActionRecord;
use std::io::{Read, Write};
use std::path::Path;

const FRAME: f64 = 0.02;
/// Speech must be this far above the noise floor.
const SPEECH_OVER_FLOOR_DB: f32 = 12.0;
/// Quiet gaps shorter than this stay inside one phrase.
const BRIDGE: f64 = 0.3;
const MIN_SPEECH: f64 = 0.12;
const PAD: f64 = 0.15;
/// Kept before the first word / click and after the last.
const LEAD_IN: f64 = 0.3;
const TAIL: f64 = 0.4;
/// Trims shorter than this aren't worth a change.
const MIN_TRIM: f64 = 0.5;
pub const LONG_PAUSE: f64 = 1.5;
/// What's left of a long pause after shortening.
const KEEP_OF_PAUSE: f64 = 0.6;
pub const TARGET_LUFS: f64 = -16.0;
const CEILING_DB: f64 = -1.0;
/// A very quiet mic would otherwise have its hiss raised by 40 dB or more.
const MAX_GAIN_DB: f64 = 30.0;

pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    /// Interleaved, −1..1.
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.channels.max(1)) / f64::from(self.rate.max(1))
    }

    fn mono(&self) -> Vec<f32> {
        let c = usize::from(self.channels.max(1));
        self.samples.chunks(c).map(|f| f.iter().sum::<f32>() / c as f32).collect()
    }
}

/// Reads a 16-bit PCM WAV.
pub fn read_wav(path: &Path) -> Result<Pcm, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)).map_err(|e| format!("{}: {e}", path.display()))?;
    let bad = || format!("{} isn't a 16-bit PCM WAV", path.display());
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(bad());
    }
    let (mut at, mut fmt, mut data) = (12usize, None, None);
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().map_err(|_| bad())?) as usize;
        let body = at + 8..(at + 8 + size).min(bytes.len());
        match &bytes[at..at + 4] {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    let fmt = &bytes[fmt.ok_or_else(bad)?];
    let u16_at = |i: usize| fmt.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    if u16_at(0) != Some(1) || u16_at(14) != Some(16) {
        return Err(bad());
    }
    let channels = u16_at(2).ok_or_else(bad)?;
    let rate = fmt.get(4..8).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or_else(bad)?;
    let samples = bytes[data.ok_or_else(bad)?].as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0).collect();
    Ok(Pcm { rate, channels, samples })
}

pub fn write_wav(path: &Path, pcm: &Pcm) -> Result<(), String> {
    let data: Vec<u8> = pcm.samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()).collect();
    let header = crate::recorder::wav_header(pcm.rate, pcm.channels, data.len() as u32);
    let mut file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(&header).and_then(|()| file.write_all(&data)).map_err(|e| format!("{}: {e}", path.display()))
}

/// dBFS per 20 ms frame.
fn frame_levels(pcm: &Pcm) -> Vec<f32> {
    let mono = pcm.mono();
    let n = ((f64::from(pcm.rate) * FRAME) as usize).max(1);
    mono.chunks(n)
        .map(|f| {
            let ms = f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32;
            10.0 * ms.max(1e-12).log10()
        })
        .collect()
}

/// Where the presenter is speaking, in seconds, padded and merged.
pub fn speech(pcm: &Pcm) -> Vec<(f64, f64)> {
    let levels = frame_levels(pcm);
    if levels.is_empty() {
        return Vec::new();
    }
    let mut sorted = levels.clone();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let loudest = sorted[sorted.len() - 1];
    // Never call the loudest part of the take silence, even if it's all quiet.
    let threshold = (floor + SPEECH_OVER_FLOOR_DB).min(loudest - 6.0);

    let mut regions: Vec<(f64, f64)> = Vec::new();
    for (i, level) in levels.iter().enumerate() {
        if *level < threshold {
            continue;
        }
        let (start, end) = (i as f64 * FRAME, (i + 1) as f64 * FRAME);
        match regions.last_mut() {
            Some(last) if start - last.1 <= BRIDGE => last.1 = end,
            _ => regions.push((start, end)),
        }
    }
    let total = pcm.seconds();
    regions.retain(|(a, b)| b - a >= MIN_SPEECH);
    let mut padded: Vec<(f64, f64)> = Vec::new();
    for (a, b) in regions {
        let (a, b) = ((a - PAD).max(0.0), (b + PAD).min(total));
        match padded.last_mut() {
            Some(last) if a <= last.1 => last.1 = b,
            _ => padded.push((a, b)),
        }
    }
    padded
}

fn r3(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

/// What Tidy up changes in one scene: trims at dead air, and shortened long pauses. Previous AI
/// changes are replaced; the presenter's own cuts stay.
pub fn tidy(scene: &SceneEdit, duration: f64, speech: &[(f64, f64)], actions: &[ActionRecord]) -> SceneEdit {
    let mut out = scene.clone();
    out.cuts.retain(|c| matches!(c.source, Source::Manual));
    let busy: Vec<(f64, f64)> = speech.iter().copied().chain(actions.iter().map(|a| (a.start, a.end))).collect();
    let (Some(first), Some(last)) = (
        busy.iter().map(|b| b.0).min_by(f64::total_cmp),
        busy.iter().map(|b| b.1).max_by(f64::total_cmp),
    ) else {
        return out;
    };

    let trim_in = (first - LEAD_IN).max(0.0);
    let trim_out = (last + TAIL).min(duration);
    let mut notes = Vec::new();
    if trim_in >= MIN_TRIM {
        out.trim_in = r3(trim_in);
        notes.push(format!("{trim_in:.1} s of silence at the start"));
    }
    if duration - trim_out >= MIN_TRIM {
        out.trim_out = Some(r3(trim_out));
        notes.push(format!("{:.1} s of silence at the end", duration - trim_out));
    }
    out.trim_note = (!notes.is_empty()).then(|| notes.join(", "));

    // Long gaps between speech, unless something happens on screen in them.
    for pair in speech.windows(2) {
        let (gap_from, gap_to) = (pair[0].1, pair[1].0);
        let gap = gap_to - gap_from;
        if gap <= LONG_PAUSE || gap_from < out.trim_in || out.trim_out.is_some_and(|end| gap_to > end) {
            continue;
        }
        let keep = KEEP_OF_PAUSE / 2.0;
        let (from, to) = (gap_from + keep, gap_to - keep);
        if actions.iter().any(|a| from < a.end && to > a.start) {
            continue;
        }
        out.cuts.push(Cut {
            from: r3(from),
            to: r3(to),
            source: Source::Ai { reason: format!("{gap:.1} s pause shortened") },
            enabled: true,
        });
    }
    out
}

// ---- Loudness (ITU-R BS.1770-4) ---------------------------------------------------------------

struct Biquad {
    b: [f64; 3],
    a: [f64; 3],
    z: [f64; 2],
}

impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[1] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[2] * y;
        y
    }
}

/// K-weighting filters for `rate`, from the BS.1770 analogue prototypes (exact at 48 kHz).
fn k_weighting(rate: f64) -> (Biquad, Biquad) {
    use std::f64::consts::PI;
    // Stage 1: high shelf.
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (PI * f0 / rate).tan();
    let (vh, vb) = (10f64.powf(g / 20.0), 10f64.powf(g / 20.0).powf(0.499_666_774_154_541_6));
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    // Stage 2: high pass.
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad { b: [1.0, -2.0, 1.0], a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], z: [0.0; 2] };
    (shelf, high_pass)
}

/// Integrated loudness in LUFS, or `None` for silence.
pub fn loudness(pcm: &Pcm) -> Option<f64> {
    let c = usize::from(pcm.channels.max(1));
    let rate = f64::from(pcm.rate);
    let mut filters: Vec<(Biquad, Biquad)> = (0..c).map(|_| k_weighting(rate)).collect();
    let weighted: Vec<Vec<f64>> = {
        let mut per = vec![Vec::with_capacity(pcm.samples.len() / c); c];
        for frame in pcm.samples.chunks_exact(c) {
            for (ch, s) in frame.iter().enumerate() {
                let (shelf, hp) = &mut filters[ch];
                per[ch].push(hp.run(shelf.run(f64::from(*s))));
            }
        }
        per
    };
    // 400 ms blocks, 75 % overlap.
    let (block, step) = ((0.4 * rate) as usize, (0.1 * rate) as usize);
    let len = weighted[0].len();
    if len < block {
        return None;
    }
    let blocks: Vec<f64> = (0..=(len - block) / step)
        .map(|i| weighted.iter().map(|ch| ch[i * step..i * step + block].iter().map(|x| x * x).sum::<f64>() / block as f64).sum())
        .collect();
    let lufs = |power: f64| -0.691 + 10.0 * power.log10();
    let gated = |threshold: f64| -> Vec<f64> { blocks.iter().copied().filter(|p| lufs(*p) > threshold).collect() };
    let absolute = gated(-70.0);
    if absolute.is_empty() {
        return None;
    }
    let relative = lufs(absolute.iter().sum::<f64>() / absolute.len() as f64) - 10.0;
    let kept = gated(relative.max(-70.0));
    (!kept.is_empty()).then(|| lufs(kept.iter().sum::<f64>() / kept.len() as f64))
}

/// Gain (dB) to bring `measured` to the target, capped so a quiet mic's hiss isn't blown up.
pub fn gain_db(measured: f64) -> f64 {
    (TARGET_LUFS - measured).min(MAX_GAIN_DB)
}

/// Applies `gain_db` with a peak limiter at −1 dBFS (instant attack, 80 ms release).
pub fn level(pcm: &Pcm, gain_db: f64) -> Pcm {
    let gain = 10f32.powf(gain_db as f32 / 20.0);
    let ceiling = 10f32.powf(CEILING_DB as f32 / 20.0);
    let c = usize::from(pcm.channels.max(1));
    let release = (-1.0 / (0.08 * pcm.rate as f32)).exp();
    let mut reduction = 1.0f32;
    let mut out = Vec::with_capacity(pcm.samples.len());
    for frame in pcm.samples.chunks(c) {
        let peak = frame.iter().fold(0f32, |m, s| m.max((s * gain).abs()));
        let needed = if peak > ceiling { ceiling / peak } else { 1.0 };
        reduction = if needed < reduction { needed } else { needed + (reduction - needed) * release };
        out.extend(frame.iter().map(|s| (s * gain * reduction).clamp(-ceiling, ceiling)));
    }
    Pcm { rate: pcm.rate, channels: pcm.channels, samples: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, parts: &[(f64, f32)]) -> Pcm {
        // (seconds, amplitude) parts of a 300 Hz tone; amplitude 0 = room noise.
        let mut samples = Vec::new();
        let mut n = 0u64;
        for (secs, amp) in parts {
            for _ in 0..(secs * f64::from(rate)) as usize {
                let t = n as f32 / rate as f32;
                let noise = ((n.wrapping_mul(2_654_435_761) % 1000) as f32 / 1000.0 - 0.5) * 0.002;
                samples.push(amp * (2.0 * std::f32::consts::PI * 300.0 * t).sin() + noise);
                n += 1;
            }
        }
        Pcm { rate, channels: 1, samples }
    }

    #[test]
    fn finds_speech_between_silences() {
        let pcm = tone(48_000, &[(1.0, 0.0), (2.0, 0.3), (2.0, 0.0), (1.0, 0.3), (0.5, 0.0)]);
        let s = speech(&pcm);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!((s[0].0 - 0.85).abs() < 0.05 && (s[0].1 - 3.15).abs() < 0.05, "{s:?}");
        assert!((s[1].0 - 4.85).abs() < 0.05, "{s:?}");
    }

    #[test]
    fn tidy_trims_dead_air_and_shortens_long_pauses() {
        let pcm = tone(48_000, &[(2.0, 0.0), (2.0, 0.3), (3.0, 0.0), (1.0, 0.3), (2.0, 0.0)]);
        let scene = SceneEdit { take: Some("t".into()), ..Default::default() };
        let out = tidy(&scene, pcm.seconds(), &speech(&pcm), &[]);
        assert!((out.trim_in - 1.55).abs() < 0.05, "{}", out.trim_in);
        assert!((out.trim_out.unwrap() - 8.55).abs() < 0.05, "{:?}", out.trim_out);
        assert_eq!(out.cuts.len(), 1);
        assert!(matches!(&out.cuts[0].source, Source::Ai { reason } if reason.contains("pause")));
        assert!(out.trim_note.as_deref().unwrap().contains("start"));
    }

    #[test]
    fn tidy_keeps_clicks_and_the_presenters_cuts() {
        let pcm = tone(48_000, &[(2.0, 0.0), (1.0, 0.3), (3.0, 0.0), (1.0, 0.3)]);
        let manual = Cut { from: 0.1, to: 0.2, source: Source::Manual, enabled: true };
        let old_ai = Cut { from: 5.0, to: 5.5, source: Source::Ai { reason: "old".into() }, enabled: true };
        let scene = SceneEdit { take: Some("t".into()), cuts: vec![manual.clone(), old_ai], ..Default::default() };
        let click = ActionRecord { kind: "press".into(), label: "Settings".into(), start: 0.5, end: 0.7, ok: true };
        let quiet_click = ActionRecord { start: 4.0, end: 4.4, ..click.clone() };
        let out = tidy(&scene, pcm.seconds(), &speech(&pcm), &[click, quiet_click]);
        // The click at 0.5 s holds the start; the pause holds a click, so it stays.
        assert!(out.trim_in < 0.5);
        assert_eq!(out.cuts, vec![manual]);
    }

    #[test]
    fn measures_loudness_of_a_known_tone() {
        // A full-scale 1 kHz sine reads −3.01 LUFS in mono (BS.1770 reference).
        let rate = 48_000;
        let samples = (0..rate * 3).map(|n| (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / rate as f32).sin()).collect();
        let l = loudness(&Pcm { rate, channels: 1, samples }).unwrap();
        assert!((l + 3.01).abs() < 0.1, "{l}");
    }

    #[test]
    fn levelling_reaches_the_target_without_clipping() {
        let pcm = tone(48_000, &[(3.0, 0.02)]);
        let before = loudness(&pcm).unwrap();
        let after = level(&pcm, gain_db(before));
        let l = loudness(&after).unwrap();
        assert!((l - TARGET_LUFS).abs() < 1.0, "{before} → {l}");
        let ceiling = 10f32.powf(CEILING_DB as f32 / 20.0);
        assert!(after.samples.iter().all(|s| s.abs() <= ceiling + 1e-6));
    }

    #[test]
    fn wav_round_trips() {
        let path = std::env::temp_dir().join(format!("scenra-audio-{}.wav", std::process::id()));
        let pcm = tone(48_000, &[(0.1, 0.5)]);
        write_wav(&path, &pcm).unwrap();
        let back = read_wav(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!((back.rate, back.channels, back.samples.len()), (48_000, 1, pcm.samples.len()));
        assert!((back.samples[100] - pcm.samples[100]).abs() < 1e-4);
    }
}
