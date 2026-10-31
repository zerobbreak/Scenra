//! `microphone.wav`: 16-bit PCM, written once the take stops so its first sample lines up with
//! the video's first frame.
//!
//! While recording, the mic thread appends raw PCM to a `.part` file. At stop, `finish` writes the
//! WAV header, then the PCM, skipping the samples captured before the first video frame (or
//! padding silence if the mic started after it). Both clocks are QPC in 100 ns units.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const BITS: u16 = 16;

/// How to line the mic up with the video: frames are sample frames (one sample per channel).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Align {
    /// The mic started first: drop this many frames from its start.
    Skip(u64),
    /// The mic started late: put this many frames of silence before it.
    Pad(u64),
}

/// `mic_first` and `video_first` are QPC times in 100 ns units of each stream's first sample/frame.
pub fn alignment(mic_first: i64, video_first: i64, sample_rate: u32) -> Align {
    let frames = |ticks: i64| (ticks as i128 * i128::from(sample_rate) / 10_000_000) as u64;
    if video_first >= mic_first {
        Align::Skip(frames(video_first - mic_first))
    } else {
        Align::Pad(frames(mic_first - video_first))
    }
}

pub fn header(sample_rate: u32, channels: u16, data_bytes: u32) -> [u8; 44] {
    let block_align = channels * BITS / 8;
    let mut h = [0u8; 44];
    let mut put = |at: usize, bytes: &[u8]| h[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, b"RIFF");
    put(4, &(36 + data_bytes).to_le_bytes());
    put(8, b"WAVEfmt ");
    put(16, &16u32.to_le_bytes());
    put(20, &1u16.to_le_bytes()); // PCM
    put(22, &channels.to_le_bytes());
    put(24, &sample_rate.to_le_bytes());
    put(28, &(sample_rate * u32::from(block_align)).to_le_bytes());
    put(32, &block_align.to_le_bytes());
    put(34, &BITS.to_le_bytes());
    put(36, b"data");
    put(40, &data_bytes.to_le_bytes());
    h
}

/// Writes `wav` from the raw PCM in `part`, aligned by `align` and cut to at most `max_frames`
/// (so it ends when the video does). Returns the frames written.
pub fn finish(part: &Path, wav: &Path, sample_rate: u32, channels: u16, align: Align, max_frames: Option<u64>) -> Result<u64, String> {
    let io = |e: std::io::Error| format!("{}: {e}", wav.display());
    let block = u64::from(channels * BITS / 8);
    let mut input = File::open(part).map_err(io)?;
    let raw_frames = input.metadata().map_err(io)?.len() / block;
    let (skip, pad) = match align {
        Align::Skip(n) => (n.min(raw_frames), 0),
        Align::Pad(n) => (0, n),
    };
    let frames = (raw_frames - skip + pad).min(max_frames.unwrap_or(u64::MAX));
    let pad = pad.min(frames);
    let data_bytes = u32::try_from(frames * block).map_err(|_| "the microphone recording is over 4 GB".to_owned())?;
    input.seek(SeekFrom::Start(skip * block)).map_err(io)?;

    let mut out = BufWriter::new(File::create(wav).map_err(io)?);
    out.write_all(&header(sample_rate, channels, data_bytes)).map_err(io)?;
    std::io::copy(&mut std::io::repeat(0).take(pad * block), &mut out).map_err(io)?;
    std::io::copy(&mut BufReader::new(input).take((frames - pad) * block), &mut out).map_err(io)?;
    out.flush().map_err(io)?;
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mic_that_started_first_is_trimmed() {
        // Mic 380 ms before the first frame at 48 kHz.
        assert_eq!(alignment(1_000_000, 4_800_000, 48_000), Align::Skip(18_240));
    }

    #[test]
    fn mic_that_started_late_is_padded() {
        assert_eq!(alignment(2_000_000, 1_000_000, 48_000), Align::Pad(4_800));
    }

    #[test]
    fn header_describes_16_bit_pcm() {
        let h = header(48_000, 1, 960);
        assert_eq!(&h[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 996);
        assert_eq!(u16::from_le_bytes(h[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(h[24..28].try_into().unwrap()), 48_000);
        assert_eq!(u32::from_le_bytes(h[28..32].try_into().unwrap()), 96_000);
        assert_eq!(u16::from_le_bytes(h[32..34].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(h[40..44].try_into().unwrap()), 960);
    }

    fn run(pcm: &[i16], align: Align) -> Vec<i16> {
        run_max(pcm, align, None)
    }

    fn run_max(pcm: &[i16], align: Align, max: Option<u64>) -> Vec<i16> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("scenra-wav-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (part, wav) = (dir.join("mic.part"), dir.join("mic.wav"));
        std::fs::write(&part, pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()).unwrap();
        let frames = finish(&part, &wav, 48_000, 1, align, max).unwrap();
        let bytes = std::fs::read(&wav).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as u64, frames * 2);
        bytes[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
    }

    #[test]
    fn finish_skips_the_lead_in() {
        assert_eq!(run(&[1, 2, 3, 4], Align::Skip(2)), vec![3, 4]);
    }

    #[test]
    fn finish_pads_silence_before_a_late_mic() {
        assert_eq!(run(&[5, 6], Align::Pad(3)), vec![0, 0, 0, 5, 6]);
    }

    #[test]
    fn finish_ends_with_the_video() {
        assert_eq!(run_max(&[1, 2, 3, 4, 5], Align::Skip(1), Some(2)), vec![2, 3]);
        assert_eq!(run_max(&[1, 2], Align::Pad(3), Some(4)), vec![0, 0, 0, 1]);
    }

    #[test]
    fn finish_never_skips_past_the_end() {
        assert_eq!(run(&[7], Align::Skip(10)), Vec::<i16>::new());
    }
}
