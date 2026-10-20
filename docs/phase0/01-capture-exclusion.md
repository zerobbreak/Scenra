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
2. Drag the teleprompter so it overlaps the demo target's **New Recording** button.
3. Click **Run GDI probe**. Note the result. It's a pre-check only; OBS is the gate.
4. In OBS, start recording. In the demo target, click **New Recording**, then **Settings**
   (opens the modal dialog) and **Confirm**. Move the mouse across the teleprompter.
5. Stop recording and scrub the video.

| # | OBS capture method | Teleprompter | Scaling | Expected in video |
|---|---|---|---|---|
| 1 | Windows 10 (1903 and up) [WGC] | control (visible) | 100 % | Magenta band visible (proves the test can fail) |
| 2 | Windows 10 (1903 and up) [WGC] | excluded | 100 % | No teleprompter; target behind it visible |
| 3 | DXGI Desktop Duplication | excluded | 100 % | Same as #2 |
| 4 | WGC | excluded | 150 % | Same as #2 |
| 5 | DXGI | excluded | 150 % | Same as #2 |
| 6 | WGC | excluded → toggled visible mid-recording | 100 % | Teleprompter appears only after the toggle |

In every row, check that these are present: the cursor, the settings dialog, and the status text changing.

Also note: single-monitor vs. multi-monitor, and whether the teleprompter leaves a black box
instead of disappearing (that means `WDA_MONITOR` behaviour, i.e. an unsupported build).

## Recording results

Save the videos under `recordings/phase0-step1/` (git-ignored). Record the outcome of each row here:

| # | Date | Result | Notes |
|---|---|---|---|
| 1 | | | |
| 2 | | | |
| 3 | | | |
| 4 | | | |
| 5 | | | |
| 6 | | | |

**Fallback if exclusion fails:** use region capture that never covers the teleprompter's area
(PROJECT_OUTLINE §12).
