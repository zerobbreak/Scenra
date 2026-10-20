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
| `docs/phase0/` | Test procedures and results for each Phase 0 exit criterion |
| `context/` | Project spec, outline, and research notes |
| `design/scenra-canvas/` | Snapshot of the UI design canvas |

## Prerequisites

- Windows 10 2004+ / 11, with WebView2 (preinstalled on Windows 11)
- Rust (stable, edition 2024) with the MSVC toolchain
- Node 22+ and pnpm
- OBS Studio 30+ (obs-websocket v5 is built in)

## Develop

```sh
pnpm install
pnpm scenra    # run Scenra (Vite on :1420 + Tauri)
pnpm target    # run the Lumina demo target
cargo check --workspace
```

## Phase 0 progress

- [x] Scenra launches as a Tauri desktop app
- [ ] Teleprompter excluded from OBS capture: [procedure](docs/phase0/01-capture-exclusion.md)
- [ ] Launch the demo target with a WebView2 debug port and connect over CDP
- [ ] Locate a button by `data-testid` and map it to physical screen coordinates (hover probe at 100 % / 150 %)
- [ ] InputEngine: smooth real-cursor movement and click, with an overlay ripple
- [ ] OBS recording via obs-websocket, `microphone.wav` extracted from track 2
- [ ] Emergency stop interrupts motion and typing, even with the UI frozen
- [ ] InputGuard separates tagged Scenra input from real input
