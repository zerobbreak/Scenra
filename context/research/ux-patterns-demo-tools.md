# UX patterns from demo, screen-recording and teleprompter tools: notes for Scenra

Researched 2026-10-20. Sources are official product pages, help centres and docs. Where only vendor marketing or app-store copy was available, that is noted. "(unverified)" means I could not confirm the claim at a primary source.

---

## 1. Tool-by-tool findings

### Screen Studio (macOS screen recorder)
- **Approachability:** The flow is three verbs: "Pick what to show, bring your camera and voice along, and press record." It can capture a display, a window or an area. A countdown runs first, pause/resume works within one take, and the recording is saved to disk every 2 s for crash recovery ([screen.studio](https://screen.studio/)).
- **Cursor polish:** Raw cursor paths are smoothed ("Shaky movement becomes a steady glide"), with motion blur on fast moves. Clicks show "A quiet circle or ripple" and can play an optional click sound. Cursor size and style can be changed after recording ([screen.studio](https://screen.studio/)). Cursor options include size, hide-when-idle, loop to start position, "Remove cursor shakes", "Rotate cursor while moving" and "Stop cursor movement at the end of the video" ([guide/cursor](https://www.screen.studio/guide/cursor)).
- **Auto zoom:** It "looks at what you did while recording, like where you clicked, and zooms in on the moments that matter". Manual zooms are dropped onto the timeline ([screen.studio](https://screen.studio/)). Each zoom is either Auto (follows clicks) or Manual ([guide/auto-zoom](https://screen.studio/guide/auto-zoom)). Automatic zoom can be turned off globally in Settings > Recording > "Create zooms automatically" ([guide](https://screen.studio/guide/disable-automatic-zooms-)). Users have asked for control over zoom lead and lag timing, which suggests none exists today ([hub request](https://hub.screen.studio/p/custom-settings-for-default-automatic-zoom-range); user request, not official).
- **In-take controls:** A small floating widget shows elapsed time with Finish / Pause / Restart / Delete. It can be hidden and is not captured in the recording ([guide](https://www.screen.studio/guide/managing-recording-in-progress)). One global shortcut starts and stops recording. ⌘K opens a command menu for any setting ([screen.studio](https://screen.studio/)). ⌘/ shows all shortcuts, and they can be rebound ([shortcuts](https://screen.studio/guide/screen-studio-shortcuts)).
- **Defaults:** It can reuse the last project's settings for new recordings ([guide](https://screen.studio/guide/managing-project-settings-for-new-recordings)).
- **Takeaway:** Polish is on by default and adjusted after recording. The user never has to set it up beforehand.

### Loom
- **Approachability:** There are three modes (screen + cam, screen, cam). You check the mic by watching its icon react, and a countdown "gives you time to collect your thoughts". Stopping goes straight to a share page ([loom.com/learn](https://www.loom.com/learn/how-to-record-your-first-video)).
- **Control bar:** Restart, pause and cancel sit together on the left. Cancel discards the video. Preferences include a "highlight mouse clicks" toggle ([loom.com/learn](https://www.loom.com/learn/how-to-record-your-first-video)).
- **Restart:** Quick restart starts over and keeps your recording preferences ([Atlassian](https://support.atlassian.com/loom/docs/restart-a-recording)).
- **Windows shortcuts (rebindable):** Ctrl+Shift+L start/stop, Alt+Shift+P pause/resume, Alt+Shift+C cancel, Ctrl+Shift+R quick restart, Ctrl+Shift+D draw ([Atlassian](https://support.atlassian.com/loom/docs/use-looms-keyboard-shortcuts/)).
- **Takeaway:** Every shortcut uses two modifiers, so it never collides with typing inside the app being recorded. That matters for Scenra, whose replay is driving the real app while hotkeys are live.

### Tella
- **Record by clip:** Each recording is a clip and lands in a **clip bar** at the bottom of the editor. Preview plays all clips in sequence, and each clip can use a different layout (camera, screen, or both). To retake, delete the clip (with a brief Undo, or "Restore removed clip" later) and record again ([Tella docs](https://www.tella.com/docs/article/how-to-use-clips)). Tella's own reasoning is that short segments mean you only re-record the part that went wrong ([Tella docs](https://www.tella.com/docs/article/how-to-use-clips)).
- **Teleprompter ("Speaker notes"):** Play/Pause auto-scroll. If you scroll by hand, playback continues from the new position. Speeds are expressed in words per minute: 120/160/200 on web, and 100/150/200/300 in the desktop apps. On desktop the notes float above other windows, can be made transparent, and never appear in the recording. The web docs advise keeping the notes near the camera to hold eye contact ([Tella docs](https://docs.tella.com/help/recording/use-speaker-notes-while-recording)).
- **Online teleprompter keys:** Space pause/resume, ↑/↓ ±10 wpm, ←/→ jump a few lines, M mirror, R restart, Esc close ([Tella docs](https://docs.tella.com/help/recording/use-speaker-notes-while-recording)).
- **Editing:** Mistakes are fixed by deleting words in the transcript, and layouts, backgrounds, zooms and subtitles are added by default ([tella.com](https://www.tella.com/)).

### Descript
- **Teleprompter:** Controls cover font size, Variable or Constant scroll speed (time-based, not voice-tracked), Restart, Play, and auto-scroll when recording begins (on by default, and reset for each take). There is also mirror, a light/dark theme, and a "Focus mode" gradient above and below the reading line. You can import text from the script. The desktop teleprompter is never captured. On web it is visible in screen shares ([help.descript.com](https://help.descript.com/record/record-with-descript-s-built-in-teleprompter)).
- **Script-to-scene mapping:** Typing `/` in the script inserts a scene boundary. The script shows a thumbnail where each scene starts, the timeline shows scenes as segments, and storyboard view shows them as tiles ([Scenes overview](https://help.descript.com/hc/en-us/articles/10248939749517-Scenes-overview)). Boundaries can be dragged in the script or the timeline, and the script highlights where a boundary will land ([Scene boundaries](https://help.descript.com/hc/en-us/articles/10248940392461-Scene-boundaries)).
- **Takeaway:** Descript is the closest analogue to Scenra's "align script to steps" model. The script is the primary surface and scenes are cuts within it.

### Arcade (interactive demos)
- **Teach UX:** You pick Interactive or Video, then a display/window/area, then Start recording. Each click creates a screenshot plus a hotspot, and typing or scrolling is captured as video. Esc cancels at any time ([Arcade docs](https://docs.arcade.software/kb/interactive-demo/build/interactive-demo/record.md)).
- **Step editing:** Steps are listed in a left-hand tab, one per recorded click. Each step has a ⋯ menu to rename it. You can duplicate steps, multi-select them with Shift+click, and set per-step autoplay. Hotspot text is edited in a right-hand panel, and styling can be applied to all hotspots at once ([hotspots docs](https://docs.arcade.software/kb/interactive-demo/build/interactive-demo/edit/hotspots-callouts-and-spotlights.md)). Ctrl+Z undoes edits ([edit docs](https://docs.arcade.software/kb/interactive-demo/build/interactive-demo/edit.md)).
- **Auto pan and zoom:** It is added automatically where a hotspot or large area is detected, and you can adjust it. Easing presets are Gentle, Quick and Slow, plus a delay ([pan & zoom](https://docs.arcade.software/kb/interactive-demo/build/interactive-demo/edit/pan-and-zoom.md)).

### Supademo
- **Capture:** You click "Record Supademo" and use the product as normal. Each click becomes a step, and AI writes the step text automatically. Windows and Mac desktop apps exist. The editor adds hotspots, tooltips and branching ([supademo.com](https://supademo.com/)).
- **Bulk AI edits:** AI Command edits text across steps, and every change needs your approval ([supademo.com](https://supademo.com/)).
- Its "most user-friendly" G2 claim is vendor-stated (unverified).

### Scribe
- **Capture:** The flow is Start capture, walk through the process, Complete capture. Steps appear live in a side panel while you record. Each step gets a screenshot and auto-written text ([Scribe support search result](https://support.scribehow.com/hc/en-us/articles/8951146003741-New-User-Guide-Scribe-101); page returned 403 to direct fetch, so the details are from the search snippet and partly unverified).
- **Magic Edit:** After capture it rewrites step text, removes accidental steps, merges similar ones and adds headers. You can undo everything or revert single steps ([Magic Edit](https://support.scribehow.com/hc/en-us/articles/35437040779421-Magic-Edit); search snippet, partly unverified).
- **Takeaway:** Show steps live while the user is teaching, then offer an AI cleanup pass that can be reverted.

### Guidde
- The recording panel offers pause, restart and delete. A countdown runs before recording, and Done generates the guide ([help.guidde.com](https://help.guidde.com/en/articles/11045453-screen-recording)). "Capture New Step" in the editor records just one missing action, which is then rearranged into place ([help.guidde.com](https://help.guidde.com/en/articles/9113291-add-steps-in-the-video-editor)).

### Camtasia
- **Cursor effects:** Highlight, Ripple, Scaling ("scale the cursor down and up again to simulate the mouse click"), Scope and click Sound. Cursor Smoothing has Duration, Delay and Easing settings, plus "Detect Cursor Pauses" for stops of 1 s or more ([cursor effects](https://www.techsmith.com/learn/tutorials/camtasia/cursor-effects/), [cursor smoothing](https://www.techsmith.com/learn/tutorials/camtasia/cursor-smoothing/)). Newer releases add Motion Blur, Kinetic Cursor and Cursor Elevation ([what's new](https://www.techsmith.com/camtasia/whats-new/?current_page=3)).
- **Takeaway:** Camtasia exposes many separate effects, each tuned on the timeline. That is powerful but expert-oriented. Scenra needs only a handful of presets.

### Playwright codegen
- **Inspector:** Two windows open, the browser and the Inspector. The record button toggles recording. An assertion toolbar offers three choices: visibility, text and value. "Pick Locator" highlights elements on hover and puts the locator in an **editable field**. Copy and Clear are also there ([playwright.dev](https://playwright.dev/docs/codegen)).
- **VS Code:** "Record new" and "Record at cursor" insert actions into an existing test ([playwright.dev](https://playwright.dev/docs/codegen)).
- **Takeaway:** Steps appear only as code. That is fine for developers but opaque for Scenra's users. The useful pattern is the hover-to-highlight target picker for re-pointing a step.

### Cypress Studio
- **Recording:** Actions appear in real time as commands. Selectors follow a priority order: `data-cy`, `data-test`, `data-testid`, `data-qa`, then `name`, `id` and others. Recording can be paused. Undo, redo and reset are available. Right-clicking an element offers assertions based on its current state ([docs.cypress.io](https://docs.cypress.io/app/guides/cypress-studio)).
- **AI suggestions:** Each one is Accepted or Rejected individually or all at once, with before/after DOM snapshots shown on hover. When a test fails, the failing line is highlighted ([docs.cypress.io](https://docs.cypress.io/app/guides/cypress-studio)).
- **Takeaway:** A ranked fallback strategy for the "selector", accept/reject for AI changes, and a highlighted failing step.

### OBS Studio
- **Layout:** The main window holds the Scenes list, the Sources list (layered, reorderable, eye toggles), a Preview canvas, an Audio Mixer, Transitions and Controls. Studio Mode adds side-by-side live and edit views ([OBS overview](https://obsproject.com/kb/obs-studio-overview)). An Auto-Configuration Wizard runs on first launch. The quick-start guide also advises a few minutes of test recording before real use ([quick start](https://obsproject.com/kb/quick-start-guide)).
- **Hotkeys:** These are set in Settings > Hotkeys ([OBS overview](https://obsproject.com/kb/obs-studio-overview)). Pause only works in some output modes ([forum](https://obsproject.com/forum/threads/what-happened-to-the-pause-recording-button.140031/); community source, unverified). Community advice is to record to MKV so interrupted files survive (unverified, forum).
- **Takeaway:** This is the anti-pattern for Scenra's audience. Scenra should drive OBS headlessly and never surface scenes and sources.

### Teleprompter apps
- **PromptSmart VoiceTrack:** Scrolls as you speak, holds your place when you go off-script, and runs speech recognition on the device. Fixed-speed and remote modes are also available ([App Store](https://apps.apple.com/app/id894811756); vendor listing).
- **BIGVU:** Advises a narrow text column close to the lens. The default speed is reportedly 160 wpm, and a "per line" pacing option exists ([bigvu.tv teleprompter](https://bigvu.tv/teleprompter/teleprompter-app); partly unverified).
- **Teleprompter Premium +:** Voice-controlled scrolling, adjustable speed, font, line spacing and margins, a cue marker, and elapsed/remaining clocks ([listing](https://mwm.ai/apps/teleprompter-premium/6779357842); vendor listing).

---

## 2. Cross-cutting patterns

| Theme | Pattern seen | Sources |
|---|---|---|
| First recording | Pick target, then one Record button, then a countdown. Polish is applied by default | Screen Studio, Loom, Guidde |
| Scenes | A strip or bar of clips, each clip retaken on its own | Tella, Descript storyboard |
| Script ↔ scenes | The script is primary and scene cuts are marked inside it | Descript `/` boundaries |
| Taught steps | A live side panel of step cards with screenshots and auto-written text, renamable | Scribe, Arcade, Supademo |
| Target repair | Hover-to-highlight picker with an editable locator and ranked fallbacks | Playwright, Cypress |
| AI edits | Proposed changes that are accepted or rejected per item, and revertible | Cypress, Supademo, Scribe |
| In-take control | A small floating widget (timer, pause, restart, discard) kept out of the capture | Screen Studio, Loom |
| Hotkeys | Two-modifier global shortcuts, rebindable | Loom, Tella Windows |
| Teleprompter | wpm presets, Space to pause, a reading-line focus gradient, a floating transparent window, voice tracking that holds place when off-script | Tella, Descript, PromptSmart |
| Safety | Autosave every few seconds, Undo on delete, restore removed clip | Screen Studio, Tella |

---

## 3. Recommendations for Scenra

1. **Three-stage top bar: Teach → Rehearse → Record.** It works like a wizard stepper, with a single primary button on each screen. This mirrors Screen Studio's three-verb flow ([screen.studio](https://screen.studio/)) and Loom's mode pick, countdown, record sequence ([loom.com/learn](https://www.loom.com/learn/how-to-record-your-first-video)).

2. **Live step cards while teaching.** As the user clicks, a side panel adds a card for each click, showing a cropped screenshot with a ripple at the click point and plain-English text such as "Click **Save**". This follows Scribe's live side panel ([Scribe](https://support.scribehow.com/hc/en-us/articles/8951146003741-New-User-Guide-Scribe-101)) and Arcade's "each click = screenshot + hotspot" ([Arcade](https://docs.arcade.software/kb/interactive-demo/build/interactive-demo/record.md)).

3. **A "Fix target" button on each card.** It opens a hover-to-highlight picker over the real app, and a re-click re-points the step. The selector stays under a collapsed "Advanced" section. Scenra should store ranked fallbacks (accessibility name, then automation ID, then relative position) and say "Found by: button name". This draws on Playwright's Pick Locator ([playwright.dev](https://playwright.dev/docs/codegen)) and Cypress's selector priority ([cypress](https://docs.cypress.io/app/guides/cypress-studio)).

4. **The script is the storyboard.** Show one script document where each paragraph sits beside its step card's thumbnail. AI alignment proposes boundaries, and the user drags a boundary or types `/` to split. This is Descript's script-scene model ([Descript](https://help.descript.com/hc/en-us/articles/10248939749517-Scenes-overview), [boundaries](https://help.descript.com/hc/en-us/articles/10248940392461-Scene-boundaries)).

5. **AI suggestions as accept/reject chips.** Alignment and step-text rewrites appear as pending suggestions, each with Accept or Reject plus Accept all, and all of them can be undone. This follows Cypress Studio AI ([cypress](https://docs.cypress.io/app/guides/cypress-studio)), Supademo AI Command approval ([supademo.com](https://supademo.com/)) and Scribe Magic Edit revert ([Scribe](https://support.scribehow.com/hc/en-us/articles/35437040779421-Magic-Edit)).

6. **A scene strip with take badges.** A Tella-style bar at the bottom shows one tile per scene with a state: Not recorded, Recorded (✓ plus take count), or Needs retake. Clicking a tile re-records only that scene, and older takes are kept and can be restored ([Tella clips](https://www.tella.com/docs/article/how-to-use-clips)).

7. **A floating teleprompter "pill" near the webcam.** It is a narrow column docked to the top-centre by default, near the camera, and it can be dragged, made transparent and excluded from capture. It shows a reading-line focus gradient and the current scene's text only. This combines Tella's floating, transparent, never-captured window ([Tella](https://docs.tella.com/help/recording/use-speaker-notes-while-recording)), Descript's Focus mode ([Descript](https://help.descript.com/record/record-with-descript-s-built-in-teleprompter)) and BIGVU's narrow column near the lens ([BIGVU](https://bigvu.tv/teleprompter/teleprompter-app)).

8. **Speed in plain words.** Offer Slow / Natural / Fast presets (about 120/150/180 wpm, following Tella's wpm presets), with ↑/↓ for fine adjustment. Add an optional **Follow my voice** toggle that holds place when the user goes off-script, as PromptSmart VoiceTrack does ([Tella](https://docs.tella.com/help/recording/use-speaker-notes-while-recording), [PromptSmart](https://apps.apple.com/app/id894811756)).

9. **Countdown, then cue.** Use a large 3-2-1 overlay before each scene. The replay of the first step should wait for the user's "cue" hotkey or the end of the countdown, whichever setting they pick. This follows the Loom and Guidde countdowns ([loom](https://www.loom.com/learn/how-to-record-your-first-video), [guidde](https://help.guidde.com/en/articles/11045453-screen-recording)).

10. **A tiny take HUD.** A floating, non-captured widget shows a red ● REC dot with a timer, the scene name ("Scene 3 of 7"), the next step ("Next: Click Save"), and four buttons: Cue ▶, Pause ⏸, Retry ↺ and Stop ■. This copies Screen Studio's widget ([guide](https://www.screen.studio/guide/managing-recording-in-progress)).

11. **Collision-proof, rebindable hotkeys with an on-screen cheat card.** Use two-modifier defaults (for example Ctrl+Shift+Space for cue, Ctrl+Shift+D for done, Ctrl+Shift+P for pause, Ctrl+Shift+R for retry). Emergency stop should be a deliberately distinct key such as Pause/Break or Ctrl+Alt+End. Avoid Ctrl+Shift+Esc, which opens Task Manager on Windows; the exact choice needs testing. It always releases mouse control first, then stops OBS, then keeps the partial take. Show the bindings on a cheat card, the way Screen Studio's ⌘/ does. Sources: Loom's two-modifier shortcuts ([Atlassian](https://support.atlassian.com/loom/docs/use-looms-keyboard-shortcuts/)), Tella Windows' "at least two modifier keys" rule ([Tella](https://docs.tella.com/help/recording/use-speaker-notes-while-recording)) and Screen Studio's ⌘/ ([shortcuts](https://screen.studio/guide/screen-studio-shortcuts)).

12. **Retry keeps settings and never asks twice.** Retry discards only the current take, resets the app state to the scene start, and re-runs the countdown. It does not ask for confirmation, but an Undo toast appears. This follows Loom's quick restart, which keeps preferences ([Atlassian](https://support.atlassian.com/loom/docs/restart-a-recording)), and Tella's delete-with-Undo ([Tella](https://www.tella.com/docs/article/how-to-use-clips)).

13. **Cursor polish as three presets, not a panel.** Offer Subtle / Clear / Bold. Each preset bundles smoothing with easing, ripple size, optional click sound and an optional auto-zoom on click. A single "Preview cursor" button replays one step. Camtasia shows the full list of knobs Scenra would be collapsing (ripple, scaling, sound, easing, pause detection); see its [cursor effects](https://www.techsmith.com/learn/tutorials/camtasia/cursor-effects/) and [cursor smoothing](https://www.techsmith.com/learn/tutorials/camtasia/cursor-smoothing/) pages. Screen Studio shows that defaults-on with a global off switch is enough ([guide](https://screen.studio/guide/disable-automatic-zooms-)).

14. **Rehearse mode is a dry run with no OBS.** It runs the same HUD and teleprompter with a grey "REHEARSAL" badge, and replays clicks with an option to run at reduced speed. If a target can't be found, the step's card turns amber with a "Fix target" button. This follows Cypress's highlighted failing line ([cypress](https://docs.cypress.io/app/guides/cypress-studio)) and OBS's own advice to test-record before the real session ([OBS quick start](https://obsproject.com/kb/quick-start-guide)).

15. **OBS as a hidden engine with one status light.** A pre-flight check shows "OBS connected · Mic OK (level meter) · Disk OK". Scenra creates and switches OBS scenes itself and saves takes as MKV, then remuxes them. Autosave project state every few seconds. These ideas come from the OBS Auto-Config wizard ([quick start](https://obsproject.com/kb/quick-start-guide)), Loom's "watch the mic icon move" check ([loom](https://www.loom.com/learn/how-to-record-your-first-video)) and Screen Studio's 2 s autosave ([screen.studio](https://screen.studio/)). The MKV advice is community-sourced (unverified).

### Visual language
- **Theme:** Use light and dark themes that follow Windows. Descript offers a teleprompter theme toggle ([Descript](https://help.descript.com/record/record-with-descript-s-built-in-teleprompter)).
- **Status colours:** Use one consistent colour per state. Grey means draft or rehearsal, green means ready, red with a pulse means recording, amber means needs attention, and solid red means stopped or error. Always pair the colour with a text label.
- **Density:** Keep density low, with a single primary action per screen. Put advanced options behind "More". Screen Studio's ⌘K command menu ([screen.studio](https://screen.studio/)) is a good escape hatch for power users without cluttering the main UI.

### Anti-patterns to avoid
- **OBS-style panel soup:** Scenes plus sources plus mixer plus transitions plus Studio Mode ([OBS overview](https://obsproject.com/kb/obs-studio-overview)). Never expose these to the user.
- **Code as the step list:** Playwright and Cypress show steps as code ([playwright.dev](https://playwright.dev/docs/codegen), [cypress](https://docs.cypress.io/app/guides/cypress-studio)). Scenra should show cards instead and keep code or selectors in an Advanced section.
- **One effect, one timeline track:** Camtasia tunes each cursor effect on its own track ([Camtasia](https://www.techsmith.com/learn/tutorials/camtasia/cursor-effects/)). Use presets instead.
- **Single-key hotkeys:** These collide with typing in the app being recorded. Loom and Tella both use multi-modifier combinations.
- **A teleprompter that gets captured:** Descript's web teleprompter is visible in screen shares ([Descript](https://help.descript.com/record/record-with-descript-s-built-in-teleprompter)). Scenra must exclude its own windows from OBS capture.
- **Hidden timing controls:** Screen Studio users ask for zoom lead and lag control ([hub](https://hub.screen.studio/p/custom-settings-for-default-automatic-zoom-range)). Scenra should expose one "zoom timing" slider per preset rather than none.
- **Confirmation dialogs mid-take:** Use Undo toasts instead (Tella).
