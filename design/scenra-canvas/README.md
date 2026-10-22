# Scenra design canvas

Source files of the **Scenra Design** canvas: https://claude.ai/artifact/VW5EVES3FjvGrG8TLAmXaR

These are Design Component files (`.dc.html`). They depend on the canvas runtime (`support.js`), so they won't render correctly if opened directly in a browser. Use the canvas link to view or present them. Treat this folder as a snapshot for version control and for reference while building the real UI.

`canvas.json` holds the layout: which page each screen belongs to, its position and size.

## Main board: Scenra (C + D)

Everything here uses one style: dark ground, Literata for script text, Plus Jakarta Sans for UI, indigo action chips. Actions are pinned to **words in the script**, not to seconds on a clock.

### Core flow

| Screen | File | What it shows |
|---|---|---|
| 0 · Ready check & style | `CDReady.dc.html` | Readiness score, checklist, cursor style, shortcuts |
| 0b · Home | `CDHome.dc.html` | Project overview: length, rehearsal progress, scenes with first lines |
| 1 · Teach | `MergedTeach.dc.html` | Click through your app once; each action becomes a chip |
| 2 · Prepare | `MergedPrepare.dc.html` | Chips placed on script words; drag to move; scene track |
| 3 · Rehearse | `MergedRehearse.dc.html` | Dry run: chips turn green / running / amber; fix a missing target |
| 4 · Record | `MergedRecord.dc.html` | Full-screen focus prompter, next chip glows inside the sentence. *Superseded by compact mode* |
| 4b · Paused | `MergedPaused.dc.html` | Failure mid-take: Scenra stops and asks, never improvises. *Superseded by C4* |
| 5 · Takes | `MergedTakes.dc.html` | Player with action markers on the scrubber, takes, saved files |

### Recording screen: compact mode (chosen, 2026-10-20)

The whole take lives in one pill at the top of the screen, under the webcam, above the recorded app. Only you see it: it's excluded from capture like the Phase 0 teleprompter. A small control bar (pause, retry, full prompter, stop) sits at the bottom. This replaces `4 · Record` (`MergedRecord`) and `4b · Paused` (`MergedPaused`) as the recording screen.

| Screen | File | What it shows |
|---|---|---|
| C1 · Get ready | `CompactReady.dc.html` | Countdown ring, first line, ready checks (OBS, mic, app window); dashed outline = recorded area |
| C2 · Action on a word | `CompactAction.dc.html` | Current line with the trigger word underlined, action chip counting down ("in 2 words"), scene progress with action dots |
| C3 · Peek ahead | `CompactPeek.dc.html` | Hold Ctrl: the pill expands to previous / current / next lines with inline action chips, reading-speed toggle |
| C4 · Paused on a problem | `CompactPaused.dc.html` | Target not found: Scenra stops instead of guessing, pauses OBS, suggests a match and shows where you'll pick up |
| C5 · Scene done | `CompactDone.dc.html` | Take summary (length, actions ran / approved), next scene's first line, scene strip |
| C6 · Final view | `CompactFinal.dc.html` | After recording: preview, timeline with action dots on words, scenes in the cut, export options |

**Cue, the mascot** (`Cue.dc.html`, `MascotSheet.dc.html`, `MascotPill.dc.html`): a small prompter character in the pill that reacts to the take (reading along, about to act, target not found, scene kept), so you can glance at it instead of reading.

### Recording screen alternatives (not chosen)

Kept for reference.

| Alternative | File | Origin |
|---|---|---|
| Record with side script | `CDRecordSide.dc.html` | Flow v1 Record |
| Record compact (first sketch of compact mode) | `CDRecordCompact.dc.html` | Flow v1 compact mode |
| Studio layout | `CDStudio.dc.html` | Concept A |
| Cue cards | `CDCards.dc.html` | Concept B |

## Archive · Flow v1 + concepts

The earlier light, dock-based version, plus concepts A–D restyled to match it. Kept for reference.

`Setup`, `Main`, `Teach`, `Record`, `RecordCompact`, `Takes`, `ConceptStudio`, `ConceptCards`, `ConceptTimeline`, `ConceptPrompter` (`.dc.html`)

## Related

- UX research behind the designs: [`context/research/ux-patterns-demo-tools.md`](../../context/research/ux-patterns-demo-tools.md)
- Project outline: [`context/PROJECT_OUTLINE.md`](../../context/PROJECT_OUTLINE.md)
