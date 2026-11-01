//! Screen video: Windows.Graphics.Capture of one monitor, encoded to H.264 MP4 by Windows'
//! MediaTranscoder (hardware when the GPU has an encoder), through the `windows-capture` crate.
//!
//! Frames stay on the GPU from capture to encoder, so recording costs little CPU and no
//! per-frame copies in system RAM. WGC only delivers a frame when something on screen changed,
//! so the MP4 has a variable frame rate (up to 30 fps); players and editors handle that.
//! Capture-excluded windows (the teleprompter, control bar) are left out by Windows itself.
//!
//! Because frames only come on change, a screen that holds still at the end of a take would end
//! the video early. The handler keeps a GPU copy of the latest frame and, at stop, writes it once
//! more stamped with the stop time, so the video always covers the whole take.

use super::qpc_now;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::path::{Path, PathBuf};
use std::time::Duration;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder, VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub const FPS: u32 = 30;
/// H.264 level 5.1's largest frame; hardware encoders refuse anything bigger.
const MAX_SIZE: (u32, u32) = (4096, 2304);
/// A frame handed to Scenra this long after Windows captured it means the machine is behind.
const LATE_TICKS: i64 = 1_000_000; // 100 ms

/// One captured frame in system memory, top-down BGRA.
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Snapshot {
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        (self.bgra[i + 2], self.bgra[i + 1], self.bgra[i])
    }

    pub fn png(&self) -> Result<Vec<u8>, String> {
        use windows_capture::encoder::{ImageEncoder, ImageEncoderPixelFormat, ImageFormat};
        ImageEncoder::new(ImageFormat::Png, ImageEncoderPixelFormat::Bgra8)
            .and_then(|e| e.encode(&self.bgra, self.width, self.height))
            .map_err(|e| e.to_string())
    }
}

type SnapshotReply = mpsc::Sender<Result<Snapshot, String>>;

#[derive(Default)]
struct Shared {
    first_qpc: AtomicI64,
    frames: AtomicU64,
    late: AtomicU64,
    snapshot: Mutex<Option<SnapshotReply>>,
}

struct Flags {
    path: PathBuf,
    width: u32,
    height: u32,
    shared: Arc<Shared>,
}

struct Handler {
    encoder: Option<VideoEncoder>,
    shared: Arc<Shared>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// GPU copy of the latest frame, and its QPC time.
    last: Option<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC, i64)>,
}

impl Handler {
    /// Keeps a copy of `frame` on the GPU (no system-memory copy).
    fn keep(&mut self, frame: &Frame, ticks: i64) -> windows::core::Result<()> {
        let desc = *frame.desc();
        let reusable = matches!(&self.last, Some((_, d, _)) if d.Width == desc.Width && d.Height == desc.Height && d.Format == desc.Format);
        if !reusable {
            let own = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_DEFAULT, BindFlags: 0, CPUAccessFlags: 0, MiscFlags: 0, ..desc };
            let mut texture = None;
            unsafe { self.device.CreateTexture2D(&own, None, Some(&mut texture))? };
            self.last = texture.map(|t| (t, desc, ticks));
        }
        if let Some((texture, _, at)) = &mut self.last {
            unsafe { self.context.CopyResource(&*texture, frame.as_raw_texture()) };
            *at = ticks;
        }
        Ok(())
    }

    /// Writes the latest frame again at `now`, if the screen has held still since it.
    fn close_at(&mut self, now: i64) -> Result<(), BoxError> {
        let (Some(encoder), Some((texture, desc, at))) = (self.encoder.as_mut(), &self.last) else { return Ok(()) };
        if now - at < 10_000_000 / i64::from(FPS) {
            return Ok(());
        }
        let staging_desc = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0, ..*desc };
        let mut staging = None;
        unsafe { self.device.CreateTexture2D(&staging_desc, None, Some(&mut staging))? };
        let staging = staging.ok_or("no staging texture")?;
        unsafe { self.context.CopyResource(&staging, texture) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
        let (width, height, pitch) = (desc.Width as usize * 4, desc.Height as usize, mapped.RowPitch as usize);
        // The encoder's buffer path wants BGRA rows bottom to top.
        let mut bottom_up = Vec::with_capacity(width * height);
        for row in (0..height).rev() {
            let line = unsafe { std::slice::from_raw_parts((mapped.pData as *const u8).add(row * pitch), width) };
            bottom_up.extend_from_slice(line);
        }
        unsafe { self.context.Unmap(&staging, 0) };
        encoder.send_frame_buffer(&bottom_up, now)?;
        Ok(())
    }
}

/// Bits per second for crisp UI text: about 0.16 bits per pixel per frame (10 Mbps at 1080p30).
pub fn bitrate(width: u32, height: u32) -> u32 {
    (f64::from(width) * f64::from(height) * f64::from(FPS) * 0.16).clamp(4_000_000.0, 50_000_000.0) as u32
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = BoxError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = ctx.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(f.width, f.height)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(FPS)
                .bitrate(bitrate(f.width, f.height)),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &f.path,
        )?;
        Ok(Self { encoder: Some(encoder), shared: f.shared, device: ctx.device, context: ctx.device_context, last: None })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        let ticks = frame.timestamp()?.Duration; // QPC, 100 ns units
        let s = self.shared.clone();
        let _ = s.first_qpc.compare_exchange(0, ticks, Ordering::SeqCst, Ordering::SeqCst);
        s.frames.fetch_add(1, Ordering::Relaxed);
        if qpc_now() - ticks > LATE_TICKS {
            s.late.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(encoder) = self.encoder.as_mut() {
            encoder.send_frame(frame)?;
        }
        self.keep(frame, ticks)?;
        let wanted = s.snapshot.lock().ok().and_then(|mut slot| slot.take());
        if let Some(reply) = wanted {
            let _ = reply.send(grab(frame));
        }
        Ok(())
    }
}

fn grab(frame: &mut Frame) -> Result<Snapshot, String> {
    let buffer = frame.buffer().map_err(|e| e.to_string())?;
    let (width, height) = (buffer.width(), buffer.height());
    let mut scratch = Vec::new();
    let bgra = buffer.as_nopadding_buffer(&mut scratch).to_vec();
    Ok(Snapshot { width, height, bgra })
}

pub struct MonitorChoice {
    pub device: String,
    pub width: u32,
    pub height: u32,
}

/// The monitor with this Windows device name (`\\.\DISPLAY1`), or the primary one.
fn pick(device: Option<&str>) -> Result<(Monitor, MonitorChoice), String> {
    let err = |e: windows_capture::monitor::Error| e.to_string();
    let monitor = match device {
        Some(name) => Monitor::enumerate()
            .map_err(err)?
            .into_iter()
            .find(|m| m.device_name().is_ok_and(|n| n.eq_ignore_ascii_case(name)))
            .ok_or_else(|| format!("Monitor {name} isn't connected; choose another in Setup › Display"))?,
        None => Monitor::primary().map_err(err)?,
    };
    let choice = MonitorChoice { device: monitor.device_name().map_err(err)?, width: monitor.width().map_err(err)?, height: monitor.height().map_err(err)? };
    Ok((monitor, choice))
}

pub fn describe(device: Option<&str>) -> Result<MonitorChoice, String> {
    pick(device).map(|(_, choice)| choice)
}

pub struct Video {
    pub monitor: MonitorChoice,
    path: PathBuf,
    shared: Arc<Shared>,
    control: CaptureControl<Handler, BoxError>,
}

pub struct VideoStats {
    pub path: PathBuf,
    pub first_qpc: i64,
    pub frames: u64,
    pub late: u64,
}

impl Video {
    pub fn start(path: &Path, device: Option<&str>) -> Result<Self, String> {
        let (monitor, choice) = pick(device)?;
        if choice.width > MAX_SIZE.0 || choice.height > MAX_SIZE.1 {
            return Err(format!(
                "{} is {}×{}; Scenra records up to {}×{}. Lower its resolution or record another monitor",
                choice.device, choice.width, choice.height, MAX_SIZE.0, MAX_SIZE.1
            ));
        }
        let shared = Arc::new(Shared::default());
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(1000 / u64::from(FPS))),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            Flags { path: path.to_owned(), width: choice.width, height: choice.height, shared: shared.clone() },
        );
        let control = Handler::start_free_threaded(settings).map_err(|e| format!("Screen capture didn't start: {e}"))?;
        Ok(Self { monitor: choice, path: path.to_owned(), shared, control })
    }

    pub fn first_qpc(&self) -> i64 {
        self.shared.first_qpc.load(Ordering::SeqCst)
    }

    /// Whether the capture thread has ended on its own (it failed).
    pub fn has_failed(&self) -> bool {
        self.control.is_finished()
    }

    /// The next frame Windows delivers, copied to system memory. WGC only delivers frames when
    /// the screen changes, so this waits up to `timeout`.
    pub fn snapshot(&self, timeout: Duration) -> Result<Snapshot, String> {
        let (tx, rx) = mpsc::channel();
        *self.shared.snapshot.lock().map_err(|e| e.to_string())? = Some(tx);
        rx.recv_timeout(timeout).map_err(|_| {
            let _ = self.shared.snapshot.lock().map(|mut slot| slot.take());
            format!("the screen didn't change for {} s, so there was no frame to check", timeout.as_secs())
        })?
    }

    /// Stops at `stop_qpc`: the video ends there even if the screen held still before it.
    pub fn stop(self, stop_qpc: i64) -> Result<VideoStats, String> {
        // A failed last frame still stops capture and finishes the MP4, so the take stays playable.
        let (closed, encoder) = {
            let callback = self.control.callback();
            let mut handler = callback.lock();
            let closed = handler.close_at(stop_qpc).map_err(|e| format!("Couldn't write the last frame: {e}"));
            (closed, handler.encoder.take())
        };
        let stopped = self.control.stop().map_err(|e| format!("Screen capture failed: {e}"));
        let finished = match encoder {
            Some(encoder) => encoder.finish().map_err(|e| format!("Couldn't finish {}: {e}", self.path.display())),
            None => Ok(()),
        };
        stopped?;
        finished?;
        closed?;
        Ok(VideoStats {
            path: self.path,
            first_qpc: self.shared.first_qpc.load(Ordering::SeqCst),
            frames: self.shared.frames.load(Ordering::SeqCst),
            late: self.shared.late.load(Ordering::SeqCst),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_scales_with_pixels_within_bounds() {
        assert_eq!(bitrate(1920, 1080), 9_953_280);
        assert_eq!(bitrate(640, 480), 4_000_000);
        assert_eq!(bitrate(4096, 2304), 45_298_483);
    }

    #[test]
    fn snapshot_pixels_are_rgb_from_bgra() {
        let s = Snapshot { width: 2, height: 1, bgra: vec![1, 2, 3, 255, 10, 20, 30, 255] };
        assert_eq!(s.pixel(0, 0), (3, 2, 1));
        assert_eq!(s.pixel(1, 0), (30, 20, 10));
    }
}
