# Teach mode — manual run (P1-4, step 8)

Everything in Teach slice 1 has passed automated and scripted checks. This run is the presenter doing it for real: teach a workflow by hand, draft it, rehearse it, record scenes, and retake one. Note what happens at each step in the **Result** column, and anything odd under **Findings**.

Teach records **clicks only**. Clicking into a field is shown as skipped, and anything typed is never recorded. Keep the workflow to links and buttons.

## Before you start

- [ ] **Check recorder** (in the "Recorder take" box) shows the monitor you'll record and your mic. Pick the monitor in **Setup › Display › Record**.
- [ ] Gemini is configured (Step 3 shows the model and key source).
- [ ] At least 1 GB of free memory, or expect the low-memory prompt before each take.
- [ ] Decide the target: **Launch Lumina demo**, or your own app with **Launch Chrome** (full `http(s)://` URL).

## Checklist

| # | Do this | Expect | Result |
|---|---|---|---|
| 1 | Step 3 › **Launch Lumina demo** (or **Launch Chrome**). | Green note: target attached. | |
| 2 | Edit the **Brief** to describe the demo you want. | Brief is used for the narration's tone and content. | |
| 3 | **Start teaching**. | "Teaching: click through the app now…" appears. | |
| 4 | In the app, click through 4–8 steps across at least two views (e.g. open a panel, then a dialog, then confirm). Include one click into a text field and type something. | Each click appears in the list as you make it. The field click is greyed: *Skipped: a click into a field…*. | |
| 5 | Click something that isn't a button or link (blank page, a heading). | It appears as skipped: *not on a link or button*. | |
| 6 | **Stop teaching**. | × buttons appear. Any warnings are listed. | |
| 7 | Delete the stray clicks from steps 4–5 with ×. | Steps renumber; only clean clicks remain. | |
| 8 | Close Scenra and reopen it. | Step 3 still shows the same taught steps. | |
| 9 | **Draft from walkthrough**. | In about 10–30 s a script appears: every taught click is narrated once, in order, on a word in its line. If Gemini doesn't follow the walkthrough you get "draft again" with reasons. | |
| 10 | Read the script. | Note any click pinned to an odd word, or narration that doesn't match the app. | |
| 11 | **Rehearse**. Read along at Natural speed. | Each click lands on its word; nothing is recorded. | |
| 12 | Put the app back on its first page, then **Record scenes** › **Record scene 1**. | Start check passes, Scenra records, the take stops after the last word. | |
| 13 | Record the remaining scenes in order. | Each scene's start check passes because the page is where the walkthrough left it. | |
| 14 | Retake **scene 2 or later** from the wrong page (e.g. reload the app first). | Start check fails and says why (URL, headings or dialogs differ). | |
| 15 | Press **Replay setup**. | Scenra replays the earlier scenes' clicks without recording, then stays Ready. Record again succeeds. | |
| 16 | Check the take folders in `recordings/takes/<session>/`. | One `scene-NN-take-N` per take, each with `video.mp4`, `microphone.wav` and `metadata.json`. | |

## Findings

| Step | What happened | Expected | Severity (blocker / annoying / cosmetic) |
|---|---|---|---|
| | | | |

## Things worth watching

- A click that wasn't recorded, or was recorded against the wrong control.
- A scene whose start check fails even though the page looks right — note the reason shown.
- Replay setup that stops early, or reaches the scene but the first click then misses.
- Narration that pins a click to an unrelated word (seen once in testing: *Confirm* on "notes").
- Any typed text appearing anywhere in Scenra — it must not.

## Known limits (not bugs)

- Typing, dropdowns and other field input aren't taught or replayed yet.
- Your own narration text isn't supported yet; Gemini writes it from the brief.
- A covered or minimised target is refused by the start check; bring it to the front.
- If the page is replaced mid-teach (e.g. a tab closed and reopened), Scenra re-attaches and warns that a click at that moment may be missing.
