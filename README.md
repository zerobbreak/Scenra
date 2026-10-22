# Scenra

*From script to scene.* A recording director for software demos: teach it a workflow once, and it
replays the workflow with a real, natural cursor while you narrate and OBS records each scene.

Status: **Phase 0, technical prototype**. See [`context/PROJECT_OUTLINE.md`](context/PROJECT_OUTLINE.md)
for the architecture and exit criteria.

## Layout

| Path | What |
|---|---|
| `scenra/` | The Scenra Tauri app (Rust core in `src-tauri/`, React + Tailwind UI in `src/`) |
| `fixtures/demo-target/` | "Lumina", a tiny Tauri app with `data-testid` buttons and a dialog, used as the automation target |
| `docs/phase0/` | Test procedures and results for each Phase 0 exit criterion (`02-input-and-recording.md` has steps 2–7) |
| `context/` | Project spec, outline, and research notes |
| `design/scenra-canvas/` | Snapshot of the UI design canvas |

## Prerequisites

- Windows 10 2004+ / 11, with WebView2 (preinstalled on Windows 11)
- Rust (stable, edition 2024) with the MSVC toolchain
- Node 22+ and pnpm
- OBS Studio 30+ (obs-websocket v5 is built in)

## Display setup

Scenra works at any Windows display scaling (100 %, 125 %, 150 %, 200 % …): it reads each
monitor's scaling from Windows and maps page coordinates to physical pixels itself. The
**Setup › Display** panel at the top of Scenra:

- **shows what was detected**: each monitor's resolution, scaling and position;
- **Prompter size** slider (75–150 %): sizes the pill and control bar on top of Windows scaling.
  It's saved in `settings.json` in Scenra's config folder. A pill that wouldn't fit the screen is
  shrunk to fit;
- **Check pointer accuracy**: with the demo target open (Step 2), moves the cursor to its New
  Recording button and checks the page saw it there (within 2 CSS px).

To check that OBS hides the prompter on your setup, use **Run OBS matrix** (Step 1). It works with
any OBS canvas size: Display Capture can be scaled or cropped, as long as it uses top-left
alignment, no rotation and no bounding box (right-click › Transform › Reset Transform gives you
that), and captures the monitor the target is on.

## Develop

```sh
pnpm install
pnpm scenra    # run Scenra (Vite on :1420 + Tauri)
pnpm target    # run the Lumina demo target
cargo check --workspace
cargo build -p demo-target              # Scenra launches this exe for the CDP step
cargo test -p scenra                        # unit tests
cargo test -p scenra -- --include-ignored   # plus a live demo-target launch + CDP attach
```

## Progress

Tracked in [`context/PROGRESS-TRACKER.md`](context/PROGRESS-TRACKER.md) (Started / Next / Done / Bugs).
