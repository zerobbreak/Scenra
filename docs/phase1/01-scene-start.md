# Scene starting states

Draft a script with the target on its intended starting page. Each scene initially uses that page as its baseline. Before recording, Scenra checks the URL, headings, dialogs and the first action's availability. A mismatch leaves OBS stopped.

If the page has changed, prepare it manually and press **Record scene** again. **Reset page** navigates to the scene's saved URL (unsaved page changes may be lost). It does not restore application data or scroll position. For a scene that intentionally starts somewhere different, prepare the page and press **Use current page** to approve that scene's baseline. Then press **Record scene**. These baselines live with the loaded script in memory.

## Verification

- Draft on the Lumina fixture; record a scene that opens Settings.
- Try recording the next scene with Settings still open. Expect a dialog mismatch before OBS starts.
- Close Settings and retry. Expect the page check, OBS start, then countdown.
- Repeat with Reset page; recording should require another press after setup.
- Navigate elsewhere, prepare the next scene, and approve Use current page. Check that its first target is available before approval succeeds.
- Retake an earlier scene and confirm it checks that scene's baseline.

Automated regression:

```sh
cargo test -p scenra --lib
cargo test -p scenra --lib scene_preflight_rejects_overlay_and_recovers_after_reset -- --ignored --nocapture
cargo clippy -p scenra --all-targets -- -D warnings
pnpm --filter scenra build
```

The live test launches the built demo target and needs WebView2 loopback debugging access. It validates dialog blocking, manual recovery and navigation reset; it does not record through OBS. Whole-script rehearsal is still continuous. Fast-forward, setup scripts and persistent starting states are not implemented here.

## Playback and retake regressions

`pnpm --filter scenra test` runs the real reader hook with deterministic timers and mocked IPC. It covers retaking the final scene in the same session, retaining its dropped-frame data, halting after a narration stall, and pausing action dispatch.

The completion pill shows the final take's frame warning and a Retake button. Retry on the control bar also retakes that scene. Hover over a truncated start-check error to read the full message.

Playback guards mouse and keyboard input between actions, preserves interference across nested input operations, and rejects clicks that miss their cue by more than 500 ms. A narration timer delayed by over a second halts playback. The presenter should retry a halted scene; OBS remains under the presenter's Retry/Stop controls. Labels must match exactly and uniquely.

For the minimised-window regression, run:

```sh
cargo test -p scenra --lib locator_restores_minimised_target_before_measuring -- --ignored --nocapture
```

The script panel now offers **Launch Lumina demo**, using the bundled local fixture. Enter a full URL only when testing your own web app.
