//! Scenra's built-in recorder (docs/adr/0001-built-in-recorder.md): one monitor to `video.mp4`
//! and the default mic to `microphone.wav`, with no OBS.
//!
//! Video and mic run on their own threads and share one clock (QPC). `start` returns only once
//! both have delivered their first frame and sample, so a scene never starts before its take
//! does. `stop` trims or pads the mic so both files begin at the same instant, and writes
//! `metadata.json` with the clock offsets.

mod mic;
mod video;
mod wav;

pub use video::{FPS, Snapshot};
pub(crate) use wav::header as wav_header;

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::GetCurrentProcess;

const START_TIMEOUT: Duration = Duration::from_secs(5);
const MIC_PART: &str = "microphone.pcm.part";

/// Now on the QPC clock, in 100 ns units (the unit WGC and WASAPI timestamps use).
pub(crate) fn qpc_now() -> i64 {
    let (mut now, mut freq) = (0i64, 1i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut now);
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    (i128::from(now) * 10_000_000 / i128::from(freq.max(1))) as i64
}

/// Scenra's own committed memory, in MB.
pub fn process_memory_mb() -> u64 {
    let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    let ok = unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut c).cast::<PROCESS_MEMORY_COUNTERS>(), c.cb)
    };
    if ok.is_err() { 0 } else { (c.PrivateUsage / 1_048_576) as u64 }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    mic: Option<String>,
    monitor: String,
    width: u32,
    height: u32,
    encoder: &'static str,
}

/// What a take would record right now, without recording.
pub fn status(monitor: Option<&str>) -> Result<RecorderStatus, String> {
    let m = video::describe(monitor)?;
    Ok(RecorderStatus {
        mic: mic::default_device_name(),
        monitor: m.device,
        width: m.width,
        height: m.height,
        encoder: "H.264 MP4 (Windows Media, hardware when available)",
    })
}

pub struct Recorder {
    dir: PathBuf,
    video: video::Video,
    mic: mic::Mic,
}

pub struct Recorded {
    pub video: PathBuf,
    pub microphone: PathBuf,
    pub microphone_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    /// Frames that reached Scenra more than 100 ms after Windows captured them, in percent.
    pub late_frames_percent: f64,
    /// QPC (100 ns) of the first video frame: time zero for both files.
    pub video_start_qpc: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata<'a> {
    monitor: &'a str,
    width: u32,
    height: u32,
    max_fps: u32,
    frames: u64,
    late_frames: u64,
    mic_device: &'a str,
    sample_rate: u32,
    channels: u16,
    /// QPC (100 ns) of the first video frame; both files start here.
    video_start_qpc: i64,
    /// QPC (100 ns) of the first mic sample as captured, before alignment.
    mic_start_qpc: i64,
    /// How far the mic started before (+) or after (−) the video, ms; trimmed or padded away.
    mic_lead_ms: f64,
}

fn late_percent(late: u64, frames: u64) -> f64 {
    if frames == 0 { 0.0 } else { (late as f64 * 1000.0 / frames as f64).round() / 10.0 }
}

impl Recorder {
    /// Starts recording `monitor` (a Windows device name, or the primary monitor) into `dir`.
    pub fn start(dir: &Path, monitor: Option<&str>) -> Result<Self, String> {
        let mic = mic::Mic::start(&dir.join(MIC_PART))?;
        let video = match video::Video::start(&dir.join("video.mp4"), monitor) {
            Ok(video) => video,
            Err(e) => {
                let _ = mic.stop();
                return Err(e);
            }
        };
        let recorder = Self { dir: dir.to_owned(), video, mic };
        let deadline = Instant::now() + START_TIMEOUT;
        while recorder.video.first_qpc() == 0 || recorder.mic.first_qpc() == 0 {
            if Instant::now() > deadline || recorder.video.has_failed() {
                let which = if recorder.video.first_qpc() == 0 { "screen capture" } else { "microphone" };
                let stopped = recorder.stop().err().map(|e| format!(" ({e})")).unwrap_or_default();
                return Err(format!("The {which} didn't start within {} s{stopped}", START_TIMEOUT.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(recorder)
    }

    /// The next captured frame, for checking what the recording shows.
    pub fn snapshot(&self, timeout: Duration) -> Result<Snapshot, String> {
        self.video.snapshot(timeout)
    }

    /// The monitor being recorded (Windows device name).
    pub fn monitor(&self) -> &str {
        &self.video.monitor.device
    }

    /// Stops both streams (even if one fails), then aligns and writes the files.
    pub fn stop(self) -> Result<Recorded, String> {
        let monitor = (self.video.monitor.device.clone(), self.video.monitor.width, self.video.monitor.height);
        let device = self.mic.device.clone();
        // One stop time for both: the video's last frame and the mic's last sample.
        let stop_qpc = qpc_now();
        let video = self.video.stop(stop_qpc);
        let mic = self.mic.stop();
        let (video, mic) = (video?, mic?);

        let microphone = self.dir.join("microphone.wav");
        let align = wav::alignment(mic.first_qpc, video.first_qpc, mic::SAMPLE_RATE);
        let length = u64::try_from(i128::from(stop_qpc - video.first_qpc) * i128::from(mic::SAMPLE_RATE) / 10_000_000).ok();
        let frames = wav::finish(&mic.part, &microphone, mic::SAMPLE_RATE, mic::CHANNELS, align, length)?;
        let _ = std::fs::remove_file(&mic.part);

        let metadata = Metadata {
            monitor: &monitor.0,
            width: monitor.1,
            height: monitor.2,
            max_fps: FPS,
            frames: video.frames,
            late_frames: video.late,
            mic_device: &device,
            sample_rate: mic::SAMPLE_RATE,
            channels: mic::CHANNELS,
            video_start_qpc: video.first_qpc,
            mic_start_qpc: mic.first_qpc,
            mic_lead_ms: (video.first_qpc - mic.first_qpc) as f64 / 10_000.0,
        };
        let json = serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("metadata.json"), json).map_err(|e| e.to_string())?;

        Ok(Recorded {
            video: video.path,
            microphone,
            microphone_seconds: frames as f64 / f64::from(mic::SAMPLE_RATE),
            sample_rate: mic::SAMPLE_RATE,
            channels: mic::CHANNELS,
            late_frames_percent: late_percent(video.late, video.frames),
            video_start_qpc: video.first_qpc,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_frames_are_a_rounded_percentage() {
        assert_eq!(late_percent(0, 0), 0.0);
        assert_eq!(late_percent(1, 3), 33.3);
        assert_eq!(late_percent(5, 100), 5.0);
    }

    #[test]
    fn qpc_moves_forward() {
        let a = qpc_now();
        std::thread::sleep(Duration::from_millis(5));
        assert!(qpc_now() - a >= 40_000);
    }

    #[test]
    fn reads_own_memory() {
        assert!(process_memory_mb() > 0);
    }

    /// Live: records 4 s of the primary monitor and the default mic.
    #[test]
    #[ignore = "records the screen and microphone"]
    fn records_a_take_without_obs() {
        let dir = std::env::temp_dir().join(format!("scenra-recorder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let recorder = Recorder::start(&dir, None).unwrap();
        std::thread::sleep(Duration::from_secs(4));
        let done = recorder.stop().unwrap();
        assert!(std::fs::metadata(&done.video).unwrap().len() > 10_000);
        assert!((done.microphone_seconds - 4.0).abs() < 0.5, "mic {} s", done.microphone_seconds);
        // Even if the screen held still, the video runs to the end of the take.
        let video = crate::editor::library::mp4_seconds(&done.video).unwrap();
        assert!((video - done.microphone_seconds).abs() < 0.15, "video {video} s, mic {} s", done.microphone_seconds);
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("metadata.json")).unwrap()).unwrap();
        assert!(meta["frames"].as_u64().unwrap() > 0);
        assert!(!dir.join(MIC_PART).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
