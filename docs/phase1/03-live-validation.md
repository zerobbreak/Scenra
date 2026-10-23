# Live validation - 2026-10-23

The updated app was built and run on the actual recording laptop. Only one monitor was available: 1920x1080 at 150% scaling. The earlier 100% secondary display was not attached during this verification.

## Results

- 25 Rust unit tests and 5 reader-flow tests passed; frontend production build passed.
- The dedicated live Chrome preflight/first-click regression passed against a deliberately delayed, locally served Lumina fixture.
- Live memory preflight reported 685 MB free and returned the new close-apps/check-again/record-anyway warning. The reader test confirms that waiting for a memory decision does not start a take or consume an attempt.
- At 75% prompter zoom, physical size stayed 945x126 for compact, 945x358 for peek, then 945x126 on repeated compact resizing. Original 90% zoom restored.
- All four OBS capture-matrix rows passed on Chrome at 150%: WGC visible control, WGC excluded, DXGI excluded, WGC toggled mid-take. All 13 marker checks matched expectations. Saved excluded screenshots were inspected: the recording panel, Settings dialog, pointer and ripple are present, with no prompter or black replacement box. The visible-control shot includes the prompter and its magenta marker.
- OBS finished idle on its original Untitled profile. The matrix restored the Display Capture settings and target window placement. Capture windows were closed.

## Recording evidence

| Row | Take folder | Result |
|---|---|---|
| WGC visible control | `recordings/takes/20261023-134138` | Pass |
| WGC excluded | `recordings/takes/20261023-134159` | Pass |
| DXGI excluded | `recordings/takes/20261023-134218` | Pass |
| WGC toggled | `recordings/takes/20261023-134237` | Pass |

Each folder contains an MKV, isolated microphone WAV, and PNG samples. `recordings/live-check/matrix-150.json` contains the marker measurements and `verified-sizing.json` contains the native sizes. The recordings are local test artifacts and are intentionally ignored by Git.

## Remaining coverage

100% scaling and mixed-DPI transitions cannot be claimed as verified without that display connected. The matrix now supports selecting it, temporarily pointing OBS at it, and restoring the setup afterward. The original intermittent B-9 outliers were not reproduced; the guard and late-cue safeguards remain separately documented in the tracker.
