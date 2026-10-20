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
| 4 · Record | `MergedRecord.dc.html` | Full-screen focus prompter, next chip glows inside the sentence |
| 4b · Paused | `MergedPaused.dc.html` | Failure mid-take: Scenra stops and asks, never improvises |
| 5 · Takes | `MergedTakes.dc.html` | Player with action markers on the scrubber, takes, saved files |

### Recording screen alternatives (decision pending)

| Alternative | File | Origin |
|---|---|---|
| Record with side script | `CDRecordSide.dc.html` | Flow v1 Record |
| Record compact | `CDRecordCompact.dc.html` | Flow v1 compact mode |
| Studio layout | `CDStudio.dc.html` | Concept A |
| Cue cards | `CDCards.dc.html` | Concept B |

## Archive · Flow v1 + concepts

The earlier light, dock-based version, plus concepts A–D restyled to match it. Kept for reference.

`Setup`, `Main`, `Teach`, `Record`, `RecordCompact`, `Takes`, `ConceptStudio`, `ConceptCards`, `ConceptTimeline`, `ConceptPrompter` (`.dc.html`)

## Related

- UX research behind the designs: [`context/research/ux-patterns-demo-tools.md`](../../context/research/ux-patterns-demo-tools.md)
- Project outline: [`context/PROJECT_OUTLINE.md`](../../context/PROJECT_OUTLINE.md)
