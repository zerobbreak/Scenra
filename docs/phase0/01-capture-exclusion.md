# Phase 0 · Step 1 — Teleprompter capture exclusion

**Question:** Does a window with `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` stay out of
an OBS recording, while everything else on screen (target app, its dialog, the cursor) is captured?

Exit criterion (PROJECT_OUTLINE §11): *the teleprompter overlapping the target is absent from the
recording; cursor and the target's dialog are present. Tested with both OBS display-capture methods.*

## Setup

1. Windows 10 2004 (build 19041) or newer. The Scenra panel shows the build and whether it's supported.
2. OBS Studio 30+ with a scene containing a **Display Capture** source, cursor capture on.
3. Start both apps from the repo root (two terminals):
   ```sh
   pnpm target   # Lumina demo target
   pnpm scenra   # Scenra Phase 0 panel
   ```
4. Put the demo target where the display capture sees it.

## Procedure

Run the matrix below. For each row:

1. In Scenra, click **Open (excluded)** (or **Open (control, visible)** for control rows).
2. Drag the teleprompter (the compact pill at the top of the screen; drag anywhere except its button) so it overlaps the demo target's **New Recording** button.
3. Click **Run GDI probe**. Note the result. It's a pre-check only; OBS is the gate.
4. In OBS, start recording. In the demo target, click **New Recording**, then **Settings**
   (opens the modal dialog) and **Confirm**. Move the mouse across the teleprompter.
5. Stop recording and scrub the video.

| # | OBS capture method | Teleprompter | Scaling | Expected in video |
|---|---|---|---|---|
| 1 | Windows 10 (1903 and up) [WGC] | control (visible) | 150 % | Pill with its magenta bottom strip visible (proves the test can fail) |
| 2 | Windows 10 (1903 and up) [WGC] | excluded | 150 % | No teleprompter; target behind it visible |
| 3 | DXGI Desktop Duplication | excluded | 150 % | Same as #2 |
| 4 | WGC | excluded → toggled visible mid-recording | 150 % | Teleprompter appears only after the toggle |

Phase 0 targets 150 %, the presenter's display scaling. Other scalings (100 %, mixed-DPI
multi-monitor) are Phase 2 coverage; the automated run below works at any scaling.

In every row, check that these are present: the cursor, the settings dialog, and the status text changing.

Also note: single-monitor vs. multi-monitor, and whether the teleprompter leaves a black box
instead of disappearing (that means `WDA_MONITOR` behaviour, i.e. an unsupported build).

## Automated run

Scenra can run the rows for the current scaling by itself: **Run OBS matrix** in the Step 1
panel (or the `run_capture_matrix` command over IPC). It needs OBS running with a Display Capture
source in the program scene, and the demo target launched from Scenra (Step 2). The source may be
scaled, moved or cropped on any canvas size, but needs top-left alignment, no rotation and no
bounding box, and must capture the monitor the target is on; Scenra says so if not. Keep hands off
the mouse while it runs: it moves the real cursor, and the InputGuard stops on foreign input.

For each row it sets the Display Capture method (1 = DXGI, 2 = WGC), opens the pill just above
the target's New Recording button, records a take while clicking New Recording → Settings →
Confirm and crossing the pill with the cursor, and grabs OBS's program output at each step
(`GetSourceScreenshot`). Each grab is checked for the magenta marker inside the pill's rect and
saved as a PNG in the take folder, so you can confirm the cursor, ripple and dialog by eye.
Afterwards it restores the source's original method. It runs at whatever scaling Windows is set to.

## Recording results

Save the videos under `recordings/phase0-step1/` (git-ignored). Record the outcome of each row here:

| # | Date | Result | Notes |
|---|---|---|---|
| 1 | 2026-10-21 | Pass | Automated (`wgc-visible`). Marker seen in all 3 frames (7,510 px in the pill rect), so the check can fail. Take `recordings/takes/20261021-200838`. |
| 2 | 2026-10-21 | Pass | Automated (`wgc-excluded`). 0 marker px in all 3 frames; no black box. Dialog, cursor, ripple and status text present. Take `recordings/takes/20261021-200851`. |
| 3 | 2026-10-21 | Pass | Automated (`dxgi-excluded`). Same as #2. Take `recordings/takes/20261021-200904`. |
| 4 | 2026-10-21 | Pass | Automated (`wgc-toggled`). No marker before the toggle; marker in every frame after it. Take `recordings/takes/20261021-200917`. |

All rows: single monitor, 1920×1080 at 150 %, OBS 32.2.2, Windows build 26300.

**Extra run, 2026-10-20 (real app):** Chrome on the Lumina production site, recorded via obs-websocket. OBS 32.2.2, Display Capture with method *Automatic* (0), 1920×1080 at 60 fps, 150 % scaling. The compact pill (top) and control bar (bottom), both excluded, were absent from all 7 output frames, with the page, the cursor and the click results present. This doesn't fill a matrix row, because the method was Automatic rather than WGC or DXGI. Evidence is in `recordings/lumina-first-run/`.

**Fallback if exclusion fails:** use region capture that never covers the teleprompter's area
(PROJECT_OUTLINE §12).
