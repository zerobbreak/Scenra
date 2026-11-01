# ADR 0001: Scenra records with its own built-in recorder, not OBS

**Status:** Accepted, 2026-10-31

## Context

Phase 0 and the start of Phase 1 recorded every take through OBS Studio, driven over obs-websocket with Scenra's own OBS profile (MKV, PCM, mic on track 2, demuxed by `mkv.rs`). That caused two problems:

- **Install barrier.** Without OBS installed, running and with its websocket enabled, Scenra couldn't record at all.
- **Memory.** OBS runs a compositor, preview, scene system and plugins that Scenra never uses. Presenters run Scenra next to an IDE, a browser and the target app, often on laptops with integrated graphics, where GPU memory comes out of system RAM.

Scenra also needs to record across several apps (editor, terminal, browser), not only one Chromium window.

## Decision

Scenra records takes itself (`scenra/src-tauri/src/recorder/`):

- **Video:** Windows.Graphics.Capture of one whole monitor, with the cursor and no capture border, through the [`windows-capture`](https://docs.rs/windows-capture) crate (MIT). Frames stay on the GPU and go to Windows' MediaTranscoder (hardware H.264 when available) → `video.mp4`, with no audio track. WGC only delivers changed frames, so the MP4 has a variable frame rate of up to 30 fps.
- **Mic:** the Windows default recording device via WASAPI ([`wasapi`](https://docs.rs/wasapi) crate), 48 kHz mono 16-bit, written straight to `microphone.wav`.
- **Alignment:** both streams are timestamped on QPC. At stop, the mic is trimmed (or padded with silence) so both files start at the video's first frame. `metadata.json` records both start times and the offset.
- **Exclusion:** the teleprompter and control bar keep `WDA_EXCLUDEFROMCAPTURE`, which Windows' compositor applies to every WGC capture.
- **OBS is removed entirely:** `obs.rs`, `mkv.rs`, `encoding.rs`, the OBS profile switching and the OBS password secret are gone.

## Spike results (Intel Iris Xe, 1920×1080 at 150 %, Windows build 26300)

| | Built-in recorder | OBS 32.2.2 |
|---|---:|---:|
| Private memory while recording 1080p | **≈218 MB** (whole process, flat over 60 s) | **≈633 MB** (170 MB idle) |
| CPU | ≈13 % of one core | not measured |
| 60 s take | 59 s MP4, 9.8 Mbit/s, 60.0 s mic | — |

The OBS figure used its current "Untitled" profile, not Scenra's old profile. Integrated graphics count GPU surfaces as process memory, which is most of the 218 MB.

Capture matrix on the built-in recorder (2026-10-31, 150 %): `wgc-visible`, `wgc-excluded` and `wgc-toggled` all pass. Every excluded frame has 0 marker pixels; the cursor, ripple and Settings dialog are present; no capture border appears. Takes: `recordings/takes/20261031-1902*`.

## Consequences

- No OBS install, websocket setup or profile juggling. **Check recorder** shows the monitor, mic and encoder.
- Scene output is `video.mp4` + `microphone.wav` + `metadata.json`.
- `droppedFramesPercent` (from OBS's stats) is replaced by `lateFramesPercent`: frames that reached Scenra more than 100 ms after capture.
- The crate's encoder takes a target bitrate (≈0.16 bits per pixel per frame, 10 Mbit/s at 1080p30), not constant quality, and can't crop on the GPU.
- **Not yet:** region capture (a crop rectangle). That needs Scenra's own Media Foundation encode path, with a GPU video processor for crop and BGRA→NV12 conversion; it's the next slice. Monitors above 4096×2304 are refused (H.264 level 5.1). System/desktop audio isn't recorded.

## Options considered

- **Bundled ffmpeg sidecar** (`ddagrab` + mic): proven, but adds about 80 MB, involves licence care, and is a second process to manage.
- **WebView `getDisplayMedia` + MediaRecorder:** shows a picker on every take, writes WebM, and gives little control.
- **Embedding libobs:** GPL, a heavy build, and most of OBS's memory cost.
- **Keeping OBS as an optional backend:** rejected to keep one recording path.
