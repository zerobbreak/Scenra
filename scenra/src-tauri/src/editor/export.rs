//! Rendering the finished video with Windows' own editing engine (`Windows.Media.Editing`):
//! each timeline segment becomes a trimmed `MediaClip` of the take's `video.mp4`, and a
//! `BackgroundAudioTrack` of its `microphone.wav` trimmed the same way and delayed to the
//! segment's place in the output. The composition renders to H.264 + AAC MP4 with hardware
//! encoding where available. Nothing to install; the original takes are only read.

use super::model::Segment;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::Foundation::TimeSpan;
use windows::Media::Editing::{BackgroundAudioTrack, MediaClip, MediaComposition, MediaTrimmingPreference};
use windows::Media::Transcoding::TranscodeFailureReason;
use windows::Storage::{CreationCollisionOption, StorageFile, StorageFolder};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::core::HSTRING;
use windows_future::{AsyncOperationProgressHandler, AsyncStatus};

/// Bits per pixel per frame at 30 fps for the export (≈10 Mbit/s at 1080p), as the recorder uses.
const BITS_PER_PIXEL: f64 = 0.16;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn span(seconds: f64) -> TimeSpan {
    TimeSpan { Duration: (seconds.max(0.0) * 1e7).round() as i64 }
}

fn seconds(span: TimeSpan) -> f64 {
    span.Duration as f64 / 1e7
}

fn file(path: &Path) -> Result<StorageFile, String> {
    StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))
        .and_then(|op| op.join())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Maps a take folder to a replacement audio file, if any.
pub type AudioOverride = Box<dyn Fn(&str) -> Option<PathBuf> + Send>;

/// What one render needs: the session folder (takes are `<dir>/<segment.take>/…`), the
/// segments, where to write, and the file names inside each take.
pub struct Job {
    pub session_dir: PathBuf,
    pub segments: Vec<Segment>,
    pub out: PathBuf,
    pub video_name: &'static str,
    pub mic_name: &'static str,
    /// Per take, a replacement audio file (levelled copy) to use instead of `mic_name`.
    pub audio_override: AudioOverride,
}

/// Renders on its own multithreaded-apartment thread. `progress` gets 0–100.
pub fn render(job: Job, progress: impl Fn(f64) + Send + Sync + 'static, cancel: Arc<AtomicBool>) -> Result<PathBuf, String> {
    std::thread::Builder::new()
        .name("scenra-export".into())
        .spawn(move || {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(err)?;
            let result = render_here(&job, progress, &cancel);
            unsafe { RoUninitialize() };
            result.map(|()| job.out)
        })
        .map_err(err)?
        .join()
        .map_err(|_| "the export thread panicked".to_owned())?
}

fn render_here(job: &Job, progress: impl Fn(f64) + Send + Sync + 'static, cancel: &AtomicBool) -> Result<(), String> {
    if job.segments.is_empty() {
        return Err("Nothing to export: every scene is excluded or has no take".into());
    }
    let composition = MediaComposition::new().map_err(err)?;
    let clips = composition.Clips().map_err(err)?;
    let tracks = composition.BackgroundAudioTracks().map_err(err)?;
    for segment in &job.segments {
        let take = job.session_dir.join(&segment.take);
        let clip = MediaClip::CreateFromFileAsync(&file(&take.join(job.video_name))?).and_then(|op| op.join()).map_err(err)?;
        let original = seconds(clip.OriginalDuration().map_err(err)?);
        let src_out = segment.src_out.min(original);
        clip.SetTrimTimeFromStart(span(segment.src_in)).map_err(err)?;
        clip.SetTrimTimeFromEnd(span(original - src_out)).map_err(err)?;
        clips.Append(&clip).map_err(err)?;

        let audio_path = (job.audio_override)(&segment.take).unwrap_or_else(|| take.join(job.mic_name));
        let track = BackgroundAudioTrack::CreateFromFileAsync(&file(&audio_path)?).and_then(|op| op.join()).map_err(err)?;
        let audio_length = seconds(track.OriginalDuration().map_err(err)?);
        track.SetTrimTimeFromStart(span(segment.src_in)).map_err(err)?;
        track.SetTrimTimeFromEnd(span(audio_length - src_out.min(audio_length))).map_err(err)?;
        track.SetDelay(span(segment.out_at)).map_err(err)?;
        tracks.Append(&track).map_err(err)?;
    }

    // The composition's own profile matches its clips (a hand-built `CreateMp4` profile fails
    // with MF_E_ATTRIBUTENOTFOUND on these takes); only the bitrate is raised so UI text stays sharp.
    let profile = composition.CreateDefaultEncodingProfile().map_err(err)?;
    let video = profile.Video().map_err(err)?;
    let (width, height) = (video.Width().map_err(err)?, video.Height().map_err(err)?);
    if width > 0 && height > 0 {
        let bitrate = (f64::from(width) * f64::from(height) * 30.0 * BITS_PER_PIXEL).clamp(4e6, 50e6) as u32;
        video.SetBitrate(bitrate.max(video.Bitrate().unwrap_or(0))).map_err(err)?;
    }

    let out_dir = job.out.parent().ok_or("export path has no folder")?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(out_dir.as_os_str()))
        .and_then(|op| op.join())
        .map_err(err)?;
    let name = job.out.file_name().ok_or("export path has no file name")?.to_string_lossy().into_owned();
    let destination = folder
        .CreateFileAsync(&HSTRING::from(name), CreationCollisionOption::ReplaceExisting)
        .and_then(|op| op.join())
        .map_err(err)?;

    let render = composition
        .RenderToFileWithProfileAsync(&destination, MediaTrimmingPreference::Precise, &profile)
        .map_err(err)?;
    render
        .SetProgress(&AsyncOperationProgressHandler::new(move |_, value: windows::core::Ref<f64>| {
            progress(*value);
            Ok(())
        }))
        .map_err(err)?;
    loop {
        match render.Status().map_err(err)? {
            AsyncStatus::Started => {
                if cancel.load(Ordering::SeqCst) {
                    let _ = render.Cancel();
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            AsyncStatus::Completed => break,
            AsyncStatus::Canceled => {
                let _ = std::fs::remove_file(&job.out);
                return Err("Export cancelled".into());
            }
            _ => {
                let e = render.GetResults().err().map(|e| e.to_string()).unwrap_or_default();
                let _ = std::fs::remove_file(&job.out);
                return Err(format!("Export failed: {e}"));
            }
        }
    }
    match render.GetResults().map_err(err)? {
        TranscodeFailureReason::None => Ok(()),
        reason => {
            let _ = std::fs::remove_file(&job.out);
            Err(format!("Export failed: Windows couldn't encode the video ({reason:?})"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live: records two short takes with the built-in recorder, then joins them with a cut.
    #[test]
    #[ignore = "records the screen and microphone, then renders a video"]
    fn exports_two_recorded_takes_as_one_video() {
        let root = std::env::temp_dir().join(format!("scenra-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for folder in ["scene-01-take-1", "scene-02-take-1"] {
            let dir = root.join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            let recorder = crate::recorder::Recorder::start(&dir, None).unwrap();
            std::thread::sleep(Duration::from_secs(3));
            recorder.stop().unwrap();
        }
        let data = super::super::library::load(&root).unwrap();
        let mut edit = data.edit.clone();
        edit.scenes[1].cuts.push(super::super::model::Cut { from: 1.0, to: 2.0, source: super::super::model::Source::Manual, enabled: true });
        let timeline = super::super::library::timeline_of(&edit, &data.takes);
        assert_eq!(timeline.segments.len(), 3);

        let out = root.join("export").join("joined.mp4");
        let job = Job {
            session_dir: root.clone(),
            segments: timeline.segments.clone(),
            out: out.clone(),
            video_name: super::super::library::VIDEO,
            mic_name: super::super::library::MIC,
            audio_override: Box::new(|_| None),
        };
        let last = Arc::new(std::sync::Mutex::new(0.0));
        let seen = last.clone();
        render(job, move |p| *seen.lock().unwrap() = p, Arc::new(AtomicBool::new(false))).unwrap();
        assert!(std::fs::metadata(&out).unwrap().len() > 10_000);
        assert!(*last.lock().unwrap() > 50.0);

        // The rendered file opens as a clip of the expected length.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.ok();
        let clip = MediaClip::CreateFromFileAsync(&file(&out).unwrap()).unwrap().join().unwrap();
        let length = seconds(clip.OriginalDuration().unwrap());
        assert!((length - timeline.duration).abs() < 0.25, "rendered {length} s, timeline {} s", timeline.duration);
        let _ = std::fs::remove_dir_all(&root);
    }
}
