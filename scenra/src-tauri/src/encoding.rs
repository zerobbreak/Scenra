//! How the Scenra OBS profile encodes video (PROJECT_OUTLINE §9).
//!
//! Screen recordings are mostly still text, so they're encoded at constant quality (CQP/CRF):
//! a fixed bitrate either blurs text or wastes space. The GPU's hardware encoder keeps the CPU
//! free for the target app and the presenter's machine; x264 at its lightest usable preset is the
//! fallback. Video is recorded at the canvas resolution (no downscale, so text stays sharp) at 30
//! fps, which is plenty for UI and half the work of 60.
//!
//! Hardware encoders are picked by the GPUs present, not by trial: OBS shows an error dialog when
//! a profile names an encoder it can't create. If the chosen one still won't start, `start_take`
//! falls back to x264 and the profile remembers it.

use crate::obs::Obs;
use serde_json::{Value, json};
use std::path::PathBuf;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

pub const X264: &str = "obs_x264";
pub const FPS: u64 = 30;
/// Software encoding above 1080p costs more CPU than a laptop can spare mid-demo.
const X264_MAX_HEIGHT: u64 = 1080;
const HARDWARE_MAX_HEIGHT: u64 = 2160;
/// Profile parameter recording that the hardware encoder failed to start on this machine.
pub const FAILED: (&str, &str) = ("Scenra", "HardwareEncoderFailed");

const NVIDIA: u32 = 0x10DE;
const AMD: u32 = 0x1002;
const INTEL: u32 = 0x8086;

/// The hardware H.264 encoder for the GPUs in this machine, best first.
pub fn hardware() -> Option<&'static str> {
    let vendors = gpu_vendors();
    [(NVIDIA, "obs_nvenc_h264_tex"), (AMD, "h264_texture_amf"), (INTEL, "obs_qsv11_v2")]
        .into_iter()
        .find(|(vendor, _)| vendors.contains(vendor))
        .map(|(_, id)| id)
}

fn gpu_vendors() -> Vec<u32> {
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new() };
    (0..)
        .map_while(|i| unsafe { factory.EnumAdapters1(i) }.ok())
        .filter_map(|adapter| unsafe { adapter.GetDesc1() }.ok())
        .map(|desc| desc.VendorId)
        .collect()
}

/// Encoder settings (`recordEncoder.json`) for screen recordings.
pub fn settings(encoder: &str) -> Value {
    let keyframes = 2; // seconds: lets an editor cut takes cleanly
    match encoder {
        // OBS 32's QuickSync reads the per-frame-type `qp*` keys; `cqp` is what its settings UI shows.
        "obs_qsv11_v2" => json!({ "rate_control": "CQP", "cqp": 22, "qpi": 22, "qpp": 22, "qpb": 22, "target_usage": "TU4", "profile": "high", "keyint_sec": keyframes }),
        "obs_nvenc_h264_tex" => json!({ "rate_control": "CQP", "cqp": 22, "preset2": "p4", "profile": "high", "keyint_sec": keyframes }),
        "h264_texture_amf" => json!({ "rate_control": "CQP", "cqp": 22, "preset": "balanced", "profile": "high", "keyint_sec": keyframes }),
        _ => json!({ "rate_control": "CRF", "crf": 22, "preset": "superfast", "profile": "high", "keyint_sec": keyframes }),
    }
}

/// Where OBS keeps a profile's files. OBS must run on this machine in its default (non-portable)
/// setup; otherwise Scenra can't write the encoder settings and OBS uses its own defaults.
pub fn profile_dir(profile: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("obs-studio/basic/profiles").join(profile);
    dir.is_dir().then_some(dir)
}

/// Output size for a canvas: the canvas itself, scaled down (keeping its shape) only past the
/// encoder's height limit. Even numbers, as encoders need.
pub fn output_size(base: (u64, u64), encoder: &str) -> (u64, u64) {
    let max = if encoder == X264 { X264_MAX_HEIGHT } else { HARDWARE_MAX_HEIGHT };
    let (w, h) = base;
    if h <= max {
        return (w & !1, h & !1);
    }
    ((w * max / h) & !1, max)
}

/// Records at the canvas resolution, 30 fps. Changing video settings resets OBS's video, so it
/// only happens when they differ (once per profile). Returns whether anything changed.
pub fn apply_video(obs: &mut Obs, encoder: &str) -> Result<bool, String> {
    let video = obs.request("GetVideoSettings", json!({}))?;
    let field = |name: &str| video[name].as_u64().unwrap_or_default();
    let base = (field("baseWidth"), field("baseHeight"));
    let (width, height) = output_size(base, encoder);
    if (field("outputWidth"), field("outputHeight")) == (width, height)
        && (field("fpsNumerator"), field("fpsDenominator")) == (FPS, 1)
    {
        return Ok(false);
    }
    obs.request(
        "SetVideoSettings",
        json!({ "fpsNumerator": FPS, "fpsDenominator": 1, "outputWidth": width, "outputHeight": height }),
    )?;
    Ok(true)
}

/// OBS's frame counters (`GetStats`), to tell how many frames a take lost.
#[derive(Clone, Copy, Default)]
pub struct FrameCounts {
    render_skipped: u64,
    render_total: u64,
    output_skipped: u64,
}

impl FrameCounts {
    pub fn read(obs: &mut Obs) -> Result<Self, String> {
        let stats = obs.request("GetStats", json!({}))?;
        let n = |name: &str| stats[name].as_u64().unwrap_or_default();
        Ok(Self {
            render_skipped: n("renderSkippedFrames"),
            render_total: n("renderTotalFrames"),
            output_skipped: n("outputSkippedFrames"),
        })
    }

    /// Share of frames since `start` that OBS couldn't render in time or encode, in percent.
    pub fn dropped_percent_since(&self, start: &FrameCounts) -> f64 {
        let total = self.render_total.saturating_sub(start.render_total);
        if total == 0 {
            return 0.0;
        }
        let dropped = self.render_skipped.saturating_sub(start.render_skipped)
            + self.output_skipped.saturating_sub(start.output_skipped);
        (dropped as f64 * 100.0 / total as f64).min(100.0)
    }
}

/// Physical memory free right now, in MB. OBS on integrated graphics shares system RAM, so a
/// machine that is swapping drops frames whatever the encoder.
pub fn free_memory_mb() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    match unsafe { GlobalMemoryStatusEx(&mut status) } {
        Ok(()) => status.ullAvailPhys / (1024 * 1024),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_at_canvas_size_up_to_the_encoders_limit() {
        assert_eq!(output_size((1920, 1080), X264), (1920, 1080));
        assert_eq!(output_size((2560, 1440), X264), (1920, 1080));
        assert_eq!(output_size((2560, 1440), "obs_qsv11_v2"), (2560, 1440));
        assert_eq!(output_size((3840, 2160), "obs_qsv11_v2"), (3840, 2160));
        assert_eq!(output_size((3440, 1440), X264), (2580, 1080));
        assert_eq!(output_size((1366, 767), X264), (1366, 766));
    }

    #[test]
    fn every_encoder_records_at_constant_quality() {
        for id in ["obs_qsv11_v2", "obs_nvenc_h264_tex", "h264_texture_amf", X264] {
            let s = settings(id);
            assert!(matches!(s["rate_control"].as_str(), Some("CQP" | "CRF")), "{id}");
            assert_eq!(s["keyint_sec"], 2);
        }
    }

    #[test]
    fn dropped_frames_count_both_render_and_encoding_lag() {
        let start = FrameCounts { render_skipped: 10, render_total: 1000, output_skipped: 5 };
        let end = FrameCounts { render_skipped: 40, render_total: 1600, output_skipped: 35 };
        assert_eq!(end.dropped_percent_since(&start), 10.0);
        assert_eq!(start.dropped_percent_since(&start), 0.0);
    }

    #[test]
    fn reads_free_memory() {
        assert!(free_memory_mb() > 0);
    }

    #[test]
    fn finds_a_gpu() {
        // Every Windows machine has at least the Basic Render Driver adapter.
        assert!(!gpu_vendors().is_empty());
    }
}
