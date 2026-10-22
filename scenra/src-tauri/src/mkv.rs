//! Pulls one PCM audio track out of an OBS Matroska recording into a WAV file.
//!
//! Scenra's OBS profile records PCM audio (`ffmpeg_pcm_s16le`) into MKV with track 1 = mix and
//! track 2 = mic only (PROJECT_OUTLINE §9), so extracting `microphone.wav` is a demux, not a
//! decode, and needs no ffmpeg. The reader streams: it seeks past video and other tracks' blocks,
//! so multi-GB takes don't have to fit in memory.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const CLUSTER: u32 = 0x1F43_B675;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const AUDIO: u32 = 0xE1;
const SAMPLING_FREQUENCY: u32 = 0xB5;
const CHANNELS: u32 = 0x9F;
const BIT_DEPTH: u32 = 0x6264;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const SIMPLE_BLOCK: u32 = 0xA3;
/// Elements whose children we read in place rather than skip.
const MASTERS: [u32; 7] = [SEGMENT, CLUSTER, TRACKS, TRACK_ENTRY, AUDIO, BLOCK_GROUP, EBML];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct AudioTrack {
    pub number: u64,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
}

#[derive(Debug, PartialEq)]
pub struct Extracted {
    pub track: AudioTrack,
    pub bytes: u64,
    pub seconds: f64,
}

type Result<T> = std::result::Result<T, String>;

fn io(e: std::io::Error) -> String {
    e.to_string()
}

/// Reads an EBML variable-length integer. `keep_marker` for element IDs.
fn vint<R: Read>(r: &mut R, keep_marker: bool) -> Result<Option<(u64, usize)>> {
    let mut first = [0u8; 1];
    if r.read(&mut first).map_err(io)? == 0 {
        return Ok(None);
    }
    let len = first[0].leading_zeros() as usize + 1;
    if len > 8 {
        return Err("invalid EBML length".into());
    }
    let mut value = if keep_marker { first[0] as u64 } else { (first[0] as u64) & (0xFF >> len) };
    let mut rest = [0u8; 7];
    r.read_exact(&mut rest[..len - 1]).map_err(io)?;
    for b in &rest[..len - 1] {
        value = (value << 8) | *b as u64;
    }
    Ok(Some((value, len)))
}

/// All size bits set means "unknown size" (live recordings write segments and clusters this way).
fn unknown(size: u64, len: usize) -> bool {
    size == (1u64 << (7 * len)) - 1
}

fn uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |v, b| (v << 8) | *b as u64)
}

fn float(bytes: &[u8]) -> f64 {
    match bytes.len() {
        4 => f32::from_be_bytes(bytes.try_into().unwrap_or_default()) as f64,
        8 => f64::from_be_bytes(bytes.try_into().unwrap_or_default()),
        _ => 0.0,
    }
}

/// Lists the audio tracks in file order.
pub fn audio_tracks(path: &Path) -> Result<Vec<AudioTrack>> {
    let mut tracks = Vec::new();
    walk(path, &mut |event| {
        if let Event::Track(t) = event {
            tracks.push(t);
        }
        Ok(true)
    })?;
    Ok(tracks)
}

/// Feeds a block's payload to a sink in chunks.
type Payload<'a> = &'a mut dyn FnMut(&mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()>;

enum Event<'a> {
    Track(AudioTrack),
    /// A block's track number and a reader positioned at its payload.
    Block { track: u64, payload: Payload<'a> },
}

/// Streams the file, reporting audio tracks and blocks. The callback returns `false` to stop.
fn walk(path: &Path, on: &mut dyn FnMut(Event) -> Result<bool>) -> Result<()> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut r = BufReader::with_capacity(1 << 20, file);
    let mut entry: Option<(u64, u64, String, f64, u64, u64)> = None; // number, type, codec, rate, channels, bits
    let flush = |entry: &mut Option<(u64, u64, String, f64, u64, u64)>, on: &mut dyn FnMut(Event) -> Result<bool>| {
        if let Some((number, kind, codec, rate, channels, bits)) = entry.take()
            && kind == 2
        {
            let track = AudioTrack { number, codec, sample_rate: rate as u32, channels: channels as u16, bits: bits as u16 };
            return on(Event::Track(track));
        }
        Ok(true)
    };

    while let Some((id, _)) = vint(&mut r, true)? {
        let Some((size, size_len)) = vint(&mut r, false)? else { break };
        let id = id as u32;
        if id == TRACK_ENTRY || id == CLUSTER {
            if !flush(&mut entry, on)? {
                return Ok(());
            }
            if id == TRACK_ENTRY {
                entry = Some((0, 0, String::new(), 8000.0, 1, 0));
            }
        }
        if MASTERS.contains(&id) || unknown(size, size_len) {
            continue; // read children in place
        }
        match id {
            TRACK_NUMBER | TRACK_TYPE | CHANNELS | BIT_DEPTH | CODEC_ID | SAMPLING_FREQUENCY if entry.is_some() => {
                let mut bytes = vec![0u8; size as usize];
                r.read_exact(&mut bytes).map_err(io)?;
                let e = entry.as_mut().unwrap();
                match id {
                    TRACK_NUMBER => e.0 = uint(&bytes),
                    TRACK_TYPE => e.1 = uint(&bytes),
                    CODEC_ID => e.2 = String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_owned(),
                    SAMPLING_FREQUENCY => e.3 = float(&bytes),
                    CHANNELS => e.4 = uint(&bytes),
                    _ => e.5 = uint(&bytes),
                }
            }
            SIMPLE_BLOCK | BLOCK => {
                let Some((track, track_len)) = vint(&mut r, false)? else { break };
                let mut header = [0u8; 3]; // timecode (i16) + flags
                r.read_exact(&mut header).map_err(io)?;
                let lacing = (header[2] >> 1) & 0x03;
                let payload_len = size - track_len as u64 - 3;
                if lacing != 0 {
                    return Err(format!("laced blocks aren't supported (track {track})"));
                }
                let mut consumed = false;
                let mut read_payload = |sink: &mut dyn FnMut(&[u8]) -> Result<()>| -> Result<()> {
                    let mut left = payload_len;
                    let mut buf = vec![0u8; 64 * 1024];
                    while left > 0 {
                        let n = left.min(buf.len() as u64) as usize;
                        r.read_exact(&mut buf[..n]).map_err(io)?;
                        sink(&buf[..n])?;
                        left -= n as u64;
                    }
                    consumed = true;
                    Ok(())
                };
                let keep_going = on(Event::Block { track, payload: &mut read_payload })?;
                if !consumed {
                    r.seek(SeekFrom::Current(payload_len as i64)).map_err(io)?;
                }
                if !keep_going {
                    return Ok(());
                }
            }
            _ => {
                r.seek(SeekFrom::Current(size as i64)).map_err(io)?;
            }
        }
    }
    flush(&mut entry, on)?;
    Ok(())
}

/// Writes the `index`-th audio track (0-based, in file order) of `mkv` to `wav`.
pub fn extract_audio(mkv: &Path, index: usize, wav: &Path) -> Result<Extracted> {
    let tracks = audio_tracks(mkv)?;
    let track = tracks
        .get(index)
        .cloned()
        .ok_or_else(|| format!("{} has {} audio track(s); wanted track {}", mkv.display(), tracks.len(), index + 1))?;
    let format = match track.codec.as_str() {
        "A_PCM/INT/LIT" => 1u16,
        "A_PCM/FLOAT/IEEE" => 3,
        other => return Err(format!("audio track {} is {other}, not PCM: set Scenra's OBS profile to PCM", index + 1)),
    };

    let mut out = BufWriter::new(File::create(wav).map_err(io)?);
    out.write_all(&[0u8; 44]).map_err(io)?; // header written once the size is known
    let mut bytes = 0u64;
    walk(mkv, &mut |event| {
        if let Event::Block { track: number, payload } = event
            && number == track.number
        {
            payload(&mut |chunk| {
                bytes += chunk.len() as u64;
                out.write_all(chunk).map_err(io)
            })?;
        }
        Ok(true)
    })?;

    let block_align = track.channels * track.bits / 8;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&((36 + bytes).min(u32::MAX as u64) as u32).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&format.to_le_bytes());
    header.extend_from_slice(&track.channels.to_le_bytes());
    header.extend_from_slice(&track.sample_rate.to_le_bytes());
    header.extend_from_slice(&(track.sample_rate * block_align as u32).to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&track.bits.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(bytes.min(u32::MAX as u64) as u32).to_le_bytes());
    out.seek(SeekFrom::Start(0)).map_err(io)?;
    out.write_all(&header).map_err(io)?;
    out.flush().map_err(io)?;

    let seconds = bytes as f64 / (track.sample_rate as f64 * block_align.max(1) as f64);
    Ok(Extracted { track, bytes, seconds })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        // 8-byte size vint: 0x01 then 7 bytes.
        v.push(0x01);
        v.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        v.extend_from_slice(body);
        v
    }

    fn audio_entry(number: u8) -> Vec<u8> {
        let audio = [
            element(&[0xB5], &48000f64.to_be_bytes()),
            element(&[0x9F], &[2]),
            element(&[0x62, 0x64], &[16]),
        ]
        .concat();
        element(
            &[0xAE],
            &[element(&[0xD7], &[number]), element(&[0x83], &[2]), element(&[0x86], b"A_PCM/INT/LIT"), element(&[0xE1], &audio)].concat(),
        )
    }

    fn block(track: u8, payload: &[u8]) -> Vec<u8> {
        element(&[0xA3], &[&[0x80 | track, 0, 0, 0x80][..], payload].concat())
    }

    #[test]
    fn extracts_the_second_audio_track_as_wav() {
        let video = element(&[0xAE], &[element(&[0xD7], &[1]), element(&[0x83], &[1]), element(&[0x86], b"V_MPEG4/ISO/AVC")].concat());
        let tracks = element(&[0x16, 0x54, 0xAE, 0x6B], &[video, audio_entry(2), audio_entry(3)].concat());
        // An unknown-size cluster, as OBS writes while recording.
        let mut cluster = vec![0x1F, 0x43, 0xB6, 0x75, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        cluster.extend(element(&[0xE7], &[0]));
        cluster.extend(block(1, &[9; 32]));
        cluster.extend(block(2, &[1, 1, 1, 1]));
        cluster.extend(block(3, &[2, 2, 2, 2]));
        cluster.extend(block(3, &[3, 3, 3, 3]));
        let segment = [vec![0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], tracks, cluster].concat();
        let file = [element(&[0x1A, 0x45, 0xDF, 0xA3], &element(&[0x42, 0x82], b"matroska")), segment].concat();

        let dir = std::env::temp_dir().join(format!("scenra-mkv-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mkv = dir.join("take.mkv");
        let wav = dir.join("microphone.wav");
        std::fs::write(&mkv, file).unwrap();

        let found = audio_tracks(&mkv).unwrap();
        assert_eq!(found.iter().map(|t| t.number).collect::<Vec<_>>(), vec![2, 3]);
        let out = extract_audio(&mkv, 1, &wav).unwrap();
        assert_eq!(out.bytes, 8);
        assert_eq!(out.track.sample_rate, 48000);
        let data = std::fs::read(&wav).unwrap();
        assert_eq!(&data[..4], b"RIFF");
        assert_eq!(&data[44..], &[2, 2, 2, 2, 3, 3, 3, 3]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_non_pcm_audio() {
        let entry = element(&[0xAE], &[element(&[0xD7], &[2]), element(&[0x83], &[2]), element(&[0x86], b"A_AAC")].concat());
        let file = element(&[0x16, 0x54, 0xAE, 0x6B], &entry);
        let dir = std::env::temp_dir().join(format!("scenra-mkv-aac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mkv = dir.join("take.mkv");
        std::fs::write(&mkv, file).unwrap();
        let err = extract_audio(&mkv, 0, &dir.join("x.wav")).unwrap_err();
        assert!(err.contains("not PCM"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Run with `SCENRA_TEST_MKV=<obs take.mkv> cargo test -p scenra real_obs -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a real OBS recording"]
    fn real_obs_recording() {
        let mkv = std::path::PathBuf::from(std::env::var("SCENRA_TEST_MKV").unwrap());
        println!("{:?}", audio_tracks(&mkv).unwrap());
        let out = extract_audio(&mkv, 1, &mkv.with_file_name("microphone.wav")).unwrap();
        println!("{out:?}");
        assert!(out.seconds > 0.5);
    }
}
