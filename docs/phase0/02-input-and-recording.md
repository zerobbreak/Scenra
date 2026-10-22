# Phase 0 · Steps 2–7 — CDP, coordinates, input, stop, guard, OBS

Results of the automated and semi-automated checks run on 2026-10-20/21, on a single 1920×1080
display at **150 %** scaling (Windows build 26300), OBS 32.2.2. Steps 2–7 are driven from
Scenra's Phase 0 panel (sections "Step 2" and "Steps 4–7") against the demo target.

## Results

| Step | Exit criterion (PROJECT_OUTLINE §11) | Result | Evidence |
|---|---|---|---|
| P0-2 | Launch the demo target with a WebView2 debug port, connect over CDP, profile shows `cdp: webview2` | ✅ | `cargo test -p scenra -- --include-ignored` (live launch) |
| P0-3 | Find a button by `data-testid`, map it to physical px, hover probe passes at 150 % | ✅ | Probe off by **0.48 CSS px** (limit 2), 3/3 runs, dpr 1.5 |
| P0-4 | Real cursor moves smoothly and clicks, with an overlay ripple | ✅ | OBS output frames: cursor glides to *New Recording*, ripple ring grows and fades, then to the title field |
| P0-5 | OBS records via obs-websocket; `microphone.wav` extracted from track 2 | ✅ | `recordings/takes/<utc-stamp>/`: MKV + `microphone.wav` (9.2 s for a 10 s take, 48 kHz, mic live at about −60 dBFS) |
| P0-1 | Teleprompter absent from the recording; cursor, ripple, dialog present | ✅ WGC and DXGI (matrix in `01-capture-exclusion.md`) | Pill + control bar on screen during the demo take, absent from every OBS frame |
| P0-6 | Emergency stop interrupts mid-motion and mid-typing, even with the UI frozen | ✅ | Ctrl+Alt+Esc: motion stopped 3/3; typing stopped instantly while the UI thread was blocked (57 → 57 → 57 chars) |
| P0-7 | InputGuard separates tagged Scenra input from real input | ✅ | Untagged mouse nudge mid-motion stops the run 3/3; Scenra's own events counted as expected, 0 foreign |

Phase 0 targets 150 % display scaling, the presenter's setup. Other scalings are Phase 2 coverage.
The capture-exclusion matrix is in [`01-capture-exclusion.md`](01-capture-exclusion.md).

## How it works, and where it differs from the outline

- **Viewport origin** (§6): taken from the screen position of Chromium's render widget
  (`Chrome_RenderWidgetHostHWND`) rather than UIA. Same rectangle, and it works the same for Chrome
  and WebView2. Scenra checks the widget is as wide as the page's `innerWidth × dpr`.
- **Before every click**, Scenra brings the target to the front and checks the window under the
  target point is the page itself. Anything else (another app, Scenra's own panel, an autofill
  popup) is named and the click is refused. Then the hover probe has to pass.
- **InputEngine**: `SendInput` with every event tagged `0x5343_4E52` ("SCNR"). Minimum-jerk easing
  with a slight arc, 250–1100 ms by distance (Fitts), ±35 % typing jitter.
- **Ripple**: a native layered window drawn with per-pixel alpha. A transparent WebView2 inside a
  click-through window drew nothing, so the ripple doesn't use a webview. It is captured on purpose.
- **Emergency stop**: a registered hotkey (`RegisterHotKey`, Ctrl+Alt+Esc) on its own thread,
  falling back to a low-level hook if another app owns the combo. Finding: while a Scenra window
  has focus, Windows doesn't call Scenra's own `WH_KEYBOARD_LL` hooks at all (neither key-down nor
  key-up; the callback never runs), but `WM_HOTKEY` still arrives.
- **InputGuard**: low-level mouse and keyboard hooks, each on its own thread. Modifiers and Esc
  don't count as interference, so the stop is reported as the stop. Because the keyboard hook is
  blind to keys headed to Scenra's own windows, each Scenra webview also reports its key presses
  (`guard_key`). Scenra only sends input to the target, so a key in a Scenra window is the user's.
  The mouse hook isn't affected.
- **OBS** (§9): Scenra records with its own OBS profile, *Scenra*: Advanced output, MKV, tracks 1+2,
  **PCM** audio. The mic goes to tracks 1+2 and desktop audio to track 1. After the take OBS is
  switched back to the presenter's profile. Because the audio is PCM, `microphone.wav` is a
  Matroska demux with no decode, so **ffmpeg isn't needed**.

## Testing notes

- `CopyFromScreen`/`BitBlt` without `CAPTUREBLT` doesn't see layered windows (the ripple). Use OBS's
  `GetSourceScreenshot` or `BitBlt` with `CAPTUREBLT` to check what a recording shows.
- UI Automation's `InvokePattern` on a Scenra button sends no OS input, so the guard doesn't see
  it (3/3 with **Slow move**, 0 foreign events). Tools that "click" by moving the real mouse do
  trip the guard, correctly: that is untagged input. Drive long-running tests over Scenra's
  command interface (IPC) or with UIA Invoke, not with mouse-driving helpers.
- To test the guard against keys aimed at Scenra, click into the Scenra window *before* the
  armed span starts, as a presenter would. Bringing a window forward with `SetForegroundWindow`
  alone doesn't give its webview keyboard focus, so the key lands on the bare host window.
