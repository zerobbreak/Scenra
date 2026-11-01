# Scenra

*From script to scene.* A recording director for software demos: teach it a workflow once, and it
replays the workflow with a real, natural cursor while you narrate and Scenra records each scene.

Status: **Phase 0 complete; Phase 1 MVP in progress**. See [`context/PROJECT_OUTLINE.md`](context/PROJECT_OUTLINE.md)
for the architecture and exit criteria.

## Layout

| Path | What |
|---|---|
| `scenra/` | The Scenra Tauri app (Rust core in `src-tauri/`, React + Tailwind UI in `src/`) |
| `fixtures/demo-target/` | "Lumina", a tiny Tauri app with `data-testid` buttons and a dialog, used as the automation target |
| `docs/adr/` | Architecture decisions (0001: built-in recorder instead of OBS) |
| `docs/phase0/` | Test procedures and results for each Phase 0 exit criterion (`02-input-and-recording.md` has steps 2–7) |
| `context/` | Project spec, outline, and research notes |
| `design/scenra-canvas/` | Snapshot of the UI design canvas |

## Prerequisites

- Windows 10 2004+ / 11, with WebView2 (preinstalled on Windows 11)
- Rust (stable, edition 2024) with the MSVC toolchain
- Node 22+ and pnpm
- A microphone set as the Windows default recording device. No OBS needed: Scenra records the
  screen and mic itself ([ADR 0001](docs/adr/0001-built-in-recorder.md))

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

- **Record**: the monitor takes record (the primary one by default). A take records the whole
  monitor, so a scene can move between apps.

To check that the prompter stays out of your recordings, use **Run capture matrix** (Step 1). It
records takes of the monitor the target is on and checks each frame for the prompter.

## Editing

Recorded sessions are saved in `Videos\Scenra\` (set `SCENRA_SESSIONS_DIR` to use another folder,
e.g. for dev runs). **Open editor** in the main window, or **Open in editor** when a recording
finishes, opens the Editor: pick a take per scene, trim, cut (by time or by selecting words in the
transcript), and **Export video** to `<session>\export\`. **Tidy up** trims silence and shortens long
pauses on this computer. **Assemble with AI** (after you allow sending narration to Gemini) picks
the best take, cuts stumbles and adds captions. See [ADR 0002](docs/adr/0002-editor.md).

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
