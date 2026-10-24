# Fast-forward scene setup

When a recorded scene's initial-state check fails, **Replay setup** restores the first scene's URL and replays earlier approved script clicks until the requested scene's defined starting state is reached. The control appears for scene 2 onward alongside Reset page and Use current page. It does not record a take or advance narration. If the restored page already satisfies the target scene, no clicks are replayed.

Each prior scene must match its approved starting state. Actions follow narration order, and every click must resolve to one exact, visible, enabled, uncovered label. Setup uses trusted CDP clicks without moving the OS cursor. Controls inside forms are excluded. Setup refuses extra target tabs and stops when an approved click opens another tab. It never invents an action or asks an AI to recover.

The prompter shows how many clicks ran and stays Ready. Press Record separately; normal scene and memory checks still apply. If setup cannot reach the scene, prepare it manually or use Reset page and approve the prepared state as appropriate.

## Limits and cancellation

- At most 50 earlier clicks and a 60-second overall budget; each navigation/settlement wait is bounded.
- Stop cancels current and queued setup requests. Closing the prompter, emergency stop, or outside physical input also interrupts setup before subsequent actions; an already dispatched click may finish and its mouse button is always released.
- The script and starting states must still match the plan shown in the prompter. A changed plan stops setup. Drafting or starting playback while setup is active is refused.
- Scenra holds its take lock during setup and verifies OBS is idle, including recordings started outside Scenra. OBS must be reachable for that check.
- Reset activates the target tab and waits for a new document before accepting a settled page, so a delayed navigation cannot accidentally validate the old document.
- Current scripts contain clicks only. Setup does not type, run shell scripts, restore application storage, or recreate arbitrary data. It checks the URL, visible headings, dialogs and first action target already defined by the approved scene state. It does not infer unrecorded dependencies.

## Verification

```sh
pnpm --filter scenra test
pnpm --filter scenra build
cargo clippy -p scenra --all-targets -- -D warnings
cargo test -p scenra --lib
cargo test -p scenra --lib fast_forward_restores_dependent_scene_and_stops_on_cancel_or_covered_targets -- --ignored --nocapture
cargo test -p scenra --lib chrome_preflight_then_first_click -- --ignored --nocapture
```

The local fixture regression recreates the recording panel with one approved click, reaches a Settings-dependent scene with two clicks, verifies trusted events, refuses covered/duplicate targets, and cancels between actions and immediately before a press. It never clicks the target scene's own action. The Chrome regression deliberately delays page responses and verifies that reset observes a different document before the first click. Unit tests cover exclusive setup, ordered actions, queued cancellation and the action limit. Reader tests ensure success/failure return to Ready without starting OBS.

Chrome validation remains pending on this run: the debug port answered, but the fixture document reported hidden with no paint entries; a native-window diagnostic could not find its page content area. Strict visibility/paint requirements and the regression are retained. This does not establish Chrome reset/fast-forward success; the dependent-scene live coverage uses the bundled WebView2 target.

Final checks (2026-10-23): 34 native tests passed, including five live WebView2 regressions; seven reader tests, the frontend production build and strict Clippy passed. The native run excluded Gemini, real OBS recording, and the separately tracked Chrome fixture check (T-7).
