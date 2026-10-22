# Scenra — Project Outline

> A personal recording director for software demos, focused on apps you build yourself. You teach Scenra a workflow once, it pairs your actions with your voice-over script, and then it replays the workflow with natural cursor movement while you narrate and OBS records each scene.

Status: **Phase 0 complete (2026-10-21) → Phase 1 MVP**. Remaining uncertainties are to be resolved by implementation and measurement, not more planning.

---

## 1. Decisions

| Area | Decision |
|---|---|
| Audience | Personal tool. One user, one machine |
| Platform | Windows 10 (2004+) / 11 first |
| Stack | **Tauri**: Rust owns the core engine; TypeScript + React + Tailwind own the UI. No Python, no Node sidecar for v1 |
| Target apps | **My own** Chromium-based apps first: browser apps, Electron, Tauri (WebView2). Third-party apps later |
| Locators | **CDP-first**; UIA as fallback and for native windows/dialogs; vision as a validated last resort |
| Locator choice | Automatic, through **capability detection** per app |
| AI planning | Rehearsal-based: **Teach → Understand → Rehearse → Record** |
| Teach mode | One continuous walkthrough, split into independently editable, re-teachable scenes |
| AI provider | Gemini first, behind a provider-independent interface |
| Narration sync | Separate narration and automation state machines; configurable per step; **manual checkpoints by default** |
| Take control | Global hotkeys + independent emergency stop |
| Unexpected input | Pause automation, mark take `needs_review`. Scenra tags its own injected input |
| Capture | OBS display/region capture; teleprompter excluded via `WDA_EXCLUDEFROMCAPTURE` (to be proven in Phase 0) |
| Cursor & input | Real OS cursor: smooth movement, human-like typing, scrolling, click ripples. Zoom later |
| Scene output | `video.mkv` + `microphone.wav` (OBS track 2) + `actions.json` + `metadata.json` |
| Retakes | Per-scene recordings; each scene has a defined initial state. Stitching later |
| Secrets | Windows Credential Manager via Rust; SQLite stores only reference IDs |
| Recovery | Pause on failure; AI only *proposes*, I approve |
| Narration | Live human voice first; speech recognition and TTS later |
| Deferred | Playwright over CDP (would add a Node runtime) — only if direct CDP proves too costly |

---

## 2. Workflow

| Stage | What happens | Output |
|---|---|---|
| **Teach** | I perform the whole workflow once by hand. Scenra records windows, element properties, input events, and checkpoint screenshots, with secrets redacted as they're captured. | Teach session |
| **Split** | Scenra proposes scene boundaries (navigation, new window, long waits); I adjust them. | Scenes |
| **Understand** | AI aligns script ↔ scenes/steps, assigns narration, timing, and expectations. I edit and approve. | Approved scenario |
| **Rehearse** | Replay without recording: every target resolves, every expectation passes. | Rehearsal report |
| **Record** | Scenra acts, I narrate, OBS records scene by scene. | Scene folders |

### Scene initial state

Every scene records the state it starts from, so it can be retaken on its own:

```yaml
scene: generate-notes
initial_state:
  window: { title: "Lumina", engine: cdp }
  url: "http://localhost:1420/recordings/demo-1"   # CDP targets
  requires: [ recording-exists ]                    # named facts from earlier scenes
  reach_by: fast_forward   # fast_forward | setup_script | manual
  setup_script: scripts/reset-to-generate-notes.ps1  # optional
```

- **fast_forward** (default): replay earlier scenes' actions quickly, with no recording and no cursor animation, to reach the start state.
- **setup_script**: run a project script (seed data, set route) for fast, deterministic retakes.
- Before recording, Scenra checks the initial state (URL, key elements visible) and refuses to start if it doesn't match.

### Teach-mode rules
- Each mouse event is resolved to the element under the cursor: DOM node via CDP, else UIA `ElementFromPoint`, plus the owning window.
- Typed text becomes step data, so it can be edited without re-teaching.
- **Redaction happens as input is captured:** DOM `type=password`, UIA `IsPassword`, and fields I mark as sensitive are stored as `{{secret:<ref-id>}}`. Scenra doesn't take screenshots while a sensitive field is focused.

---

## 3. Core Concepts

| Term | Meaning |
|---|---|
| **Project** | Target app + launch/attach config, capture region, OBS scene, setup/reset scripts, capability profile. |
| **Capability profile** | What automation methods this app supports right now (see §5). |
| **Script** | My free-form voice-over text. |
| **Teach session** | Raw continuous walkthrough. |
| **Scenario** | Approved scenes built from Script + Teach session. |
| **Scene** | Steps recorded as one clip; has an initial state; unit of retake and re-teach. |
| **Step** | Narration + actions + sync mode + expectations. |
| **Target** | Ordered locator list (CDP → UIA → vision). |
| **Take** | One recording attempt of a scene. |

### Step format

```yaml
- id: open-new-recording
  narration: >
    Now let's create a new recording. Lumina lets you record meetings
    and automatically generate structured notes.
  sync: during              # before | during | after
  narration_done: manual    # manual (default) | after: 6s | speech (later)
  action_cue: hotkey        # hotkey (default) | narration_start | immediate
  actions:
    - click:
        target:
          - cdp:    { testId: create-recording }
          - uia:    { name: "New Recording", controlType: Button }
          - vision: { image: targets/new-recording.png, minConfidence: 0.9 }
  expect:
    - visible: { cdp: { testId: recording-panel } }
```

---

## 4. Architecture

```
┌──────────────────────────── Scenra (Tauri) ───────────────────────────────┐
│ UI (TS/React): Projects · Teach · Scene splitter · Scenario editor ·       │
│               Teleprompter · Preflight · Takes                              │
├─────────────────────────── Rust core ──────────────────────────────────────┤
│  Orchestrator ── Narration SM ⇄ Automation SM ── Run log                    │
│     ├── Capability detector                                                 │
│     ├── LocatorEngine                                                       │
│     │    ├── CdpLocator     ── Chrome/Edge · Electron · WebView2            │
│     │    ├── UiaLocator     ── native Windows controls, dialogs, windows    │
│     │    └── VisionLocator  ── screenshot fallback (validated)              │
│     ├── Coordinate mapper   CSS px → physical screen px                     │
│     ├── InputEngine         SendInput (tagged), eased motion, typing        │
│     ├── InputGuard          LL hooks: tag check → expected vs. foreign      │
│     ├── EmergencyStop       own thread + own hook                           │
│     ├── Overlay / Teleprompter windows                                      │
│     ├── ObsClient           obs-websocket v5                                │
│     ├── AiProvider          Gemini → others                                 │
│     ├── SecretStore         Windows Credential Manager                      │
│     └── Storage             SQLite + scene folders                          │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Interfaces

```rust
trait Locator {
    fn kind(&self) -> LocatorKind;
    fn inspect(&self) -> Result<UiState>;
    fn locate(&self, target: &TargetSpec) -> Result<ScreenRect>;   // physical px
    fn element_at(&self, p: ScreenPoint) -> Result<ElementInfo>;   // teach mode
    fn verify(&self, expected: &ExpectedState) -> Result<bool>;
}

trait InputEngine {
    fn move_to(&self, rect: ScreenRect, style: &MotionStyle) -> Result<()>;
    fn click(&self, kind: ClickKind) -> Result<()>;
    fn type_text(&self, text: &SecretOrText, style: &TypingStyle) -> Result<()>;
    fn scroll(&self, delta: i32, style: &MotionStyle) -> Result<()>;
    fn hotkey(&self, keys: &[Key]) -> Result<()>;
    fn release_all(&self);                                          // used by emergency stop
}

trait AiProvider {
    async fn align_script(&self, script: &str, teach: &RedactedTeachSession) -> Result<DraftScenario>;
    async fn propose_recovery(&self, ctx: &RedactedFailureContext) -> Result<RecoveryProposal>;
}
```

`AiProvider` methods only accept **redacted** types, so the compiler prevents a raw secret from ever reaching Gemini.

---

## 5. Capability Detection

Runs when Scenra attaches to an app, and again in preflight.

```json
{
  "application": "Lumina",
  "window": { "hwnd": "0x2A04F2", "process": "lumina.exe", "dpi": 144 },
  "capabilities": {
    "cdp":   { "available": true, "flavor": "webview2", "endpoint": "127.0.0.1:9222" },
    "uia":   { "available": true, "tree_quality": "coarse" },
    "vision": true,
    "screen_coordinates": { "calibrated": true, "max_error_px": 1 }
  },
  "preferred_locator": "cdp",
  "warnings": []
}
```

Checks:
- **CDP:** does the endpoint answer `/json/version`, and does a target belong to this window's process? Is it bound to loopback only?
- **UIA:** element count and how many have a name or AutomationId → `rich` / `coarse` / `none`.
- **Coordinates:** passes the hover probe (§6).
- Shown in the UI as a single status ("Fully supported", "Limited: no CDP, using UIA"), with details available for debugging.

### Enabling CDP per flavor (development only)

| Flavor | How | Notes |
|---|---|---|
| Chrome / Edge | Launch with `--remote-debugging-port=<port> --user-data-dir=<dedicated demo profile>` | Recent Chrome requires a non-default profile for remote debugging; a demo profile also keeps logins clean |
| Electron | `app.commandLine.appendSwitch('remote-debugging-port', …)` in **dev builds only**, or the launch flag | Production builds may disable it with fuses; that's fine |
| Tauri (WebView2) | `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>` set when Scenra launches the app | Scenra sets the variable only for the process it launches, never globally |

Scenra will provide a short "Make your app Scenra-ready" guide: `data-testid` on interactive elements, plus a dev-only debug port. Production releases must never need an exposed port.

---

## 6. Coordinate Mapping

The goal: turn a DOM element's CSS-pixel rectangle into the **physical** screen pixel where the real cursor should go.

```
screen_px = viewport_origin_screen_px + css_px × devicePixelRatio
```

- `devicePixelRatio` already includes both display scaling and browser zoom.
- **viewport_origin**: the screen position of the content area, not the window rect, which includes borders, title bar, and toolbars. Phase 0 reads it from the `Chrome_RenderWidgetHostHWND` child window (same rect as UIA's render widget, and the same for Chrome and WebView2).
- Read `visualViewport` offset/scale to handle pinch-zoom.
- Scroll the element into view first, then re-measure right before acting.
- Scenra runs **per-monitor DPI aware (v2)**, so every Win32/UIA coordinate is in physical pixels.
- Multi-monitor: physical virtual-desktop coordinates; `SendInput` uses `MOUSEEVENTF_VIRTUALDESK` normalization.

**Hover probe (calibration and per-click verification):** inject a passive `mousemove` listener over CDP, move the real cursor to the computed point, and read back the page's `clientX/Y`. If the error is above 2 CSS px, don't click: re-measure once, then pause. This turns "coordinate accuracy is essential" into a check that runs before every click.

---

## 7. Narration ⇄ Automation Sync

Two state machines per step:

```
Narration:   Ready ──(step shown)──► Speaking ──(hotkey "done")──► Finished
Automation:  Pending ──(cue)──► Running ──(actions + expect ok)──► Completed
                                   └──(fail)──► Failed → take paused
```

| `sync` | Automation cue | Step completes when |
|---|---|---|
| `before` | Runs as the step starts; narration shown after it's Completed | Both done |
| `during` | `action_cue` (default: hotkey) while Speaking | Both done |
| `after` | When narration reaches Finished | Both done |

Default hotkeys (to be configured):
- `F9`: cue action
- `F10`: narration done / next
- `F8`: pause/resume
- `F7`: retry step
- `Ctrl+Alt+Pause`: **emergency stop**

v1 has no speech recognition. `narration_done: after: Ns` exists for Automatic mode only.

---

## 8. Input Safety

- **Tagged injection:** every `SendInput` event carries a Scenra-specific `dwExtraInfo` value (a random per-session token). The low-level hooks classify events as follows:

| Event | Classification |
|---|---|
| Injected + Scenra tag | Expected; ignored |
| Registered hotkey from hardware | Control input |
| Hardware input (not injected) | Foreign → pause, take `needs_review` |
| Injected without the tag (another tool, remote desktop) | Foreign → pause |

- **Emergency stop** runs on its own thread with its own hook and message loop. It calls `release_all()`, cancels the orchestrator, and stops OBS recording safely. It doesn't depend on the UI or WebView being responsive.
- Never hold a key or button across an await; every action has a timeout.
- Preflight warns about UIPI: an elevated target can't receive input from a non-elevated Scenra.
- Phase 0 tests the guard with **real hardware input** and with Scenra input.

---

## 9. Capture, Audio & Output

- Teleprompter: separate Tauri window, `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on its HWND. It takes the form of the compact pill (design C1–C5, `design/scenra-canvas/README.md`).
- OBS display or region capture; cursor capture on.
- Audio: Advanced output, MKV, track 1 = mix, track 2 = mic only, using Scenra's own OBS profile. Audio is recorded as PCM, so Scenra demuxes track 2 → `microphone.wav` itself with no ffmpeg (Phase 0, `mkv.rs`); optional remux to MP4 later. One recorder, so the mic is never recorded twice.

```
recordings/<project>/<take-timestamp>/scene-03/
├── video.mkv
├── microphone.wav
├── actions.json     # executed actions, OBS-timecode aligned, secrets as refs only
└── metadata.json    # scenario version, capability profile, DPI, take status
```

---

## 10. Secrets

| SQLite | Windows Credential Manager (`scenra/<ref-id>`) |
|---|---|
| Projects, scripts, scenarios | Gemini API key |
| Action history, run logs | Demo account credentials |
| Credential **reference IDs** only | Sensitive input values |

- Placeholders are substituted **only inside `InputEngine::type_text`**, at the moment of execution.
- Secret values never appear in logs, `actions.json`, screenshots, the AI context, or the UI (shown as `••••`).
- Scenra takes no screenshots while a sensitive field is focused, in Teach or Record.

---

## 11. Roadmap

### Phase 0 — Technical prototype
Repo layout: `scenra/` (Tauri app) + `fixtures/demo-target/` (a tiny Tauri app with `data-testid` buttons and a dialog, used as the target).

Exit criteria:
- [x] Scenra launches as a Tauri desktop app.
- [x] It launches the demo target with a WebView2 debug port and connects over CDP (capability profile shows `cdp: webview2`).
- [x] It finds a button by `data-testid`.
- [x] Rust maps it to physical screen coordinates; the hover probe passes at 150 % scaling (the presenter's display; 100 % is deferred to Phase 2).
- [x] The InputEngine moves the real cursor smoothly and clicks, with an overlay ripple.
- [x] OBS records the interaction via obs-websocket; `microphone.wav` is extracted from track 2.
- [x] The teleprompter overlapping the target is absent from the recording; cursor, ripple, and the target's dialog are present. Tested with both OBS display-capture methods.
- [x] The emergency stop interrupts mid-motion and mid-typing, even with the UI frozen (simulated hang).
- [x] The InputGuard correctly separates tagged Scenra input from real mouse/keyboard input.

Stretch: the same test against Chrome and against an Electron fixture.

### Phase 1 — MVP
Teach (continuous) → scene split → Gemini alignment → Rehearse → Record per scene, with initial-state checks, fast-forward/setup scripts, manual checkpoints, pause-and-propose recovery, and secrets via Credential Manager.

### Phase 2 — Coverage
UIA locator for native windows/dialogs; hover probe and capture matrix at other display scalings (100 %, mixed-DPI multi-monitor); vision fallback with review; speech-based narration completion; retake from the Takes list.

### Phase 3 — Polish
Zoom/highlight effects, take stitching, chapter markers, Claude provider, TTS, third-party apps, Playwright-over-CDP if needed, other OSes.

---

## 12. Risks

| Risk | Mitigation |
|---|---|
| Teleprompter leaks into capture | Phase 0 gate; region capture that never includes the teleprompter's area as backup |
| Coordinate errors → mis-click | Hover probe before every click; pause on mismatch |
| CDP not available for an app | Capability profile shows it clearly; UIA/vision fallback; setup guide |
| Debug port exposed | Dev-only, loopback-only, set per launched process |
| Scene retake depends on earlier scenes | Initial-state definition + fast-forward/setup scripts + preflight check |
| Losing control of the machine | Independent emergency stop, `release_all`, timeouts |
| Secrets leaking to logs/AI | Redaction at capture; redacted-only types in `AiProvider` |
| AI misalignment | Human approval + rehearsal before any recording |
