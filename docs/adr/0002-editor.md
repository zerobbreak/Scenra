# ADR 0002: A built-in editor turns a session's takes into one finished video

**Status:** Accepted, 2026-10-31

## Context

With the built-in recorder (ADR 0001), every scene take is its own folder (`video.mp4` with no audio, `microphone.wav`, `metadata.json`), and a session holds several scenes plus retakes. Nothing joined them, so the presenter had no finished video. The script and the take list lived only in memory.

The decisions below were settled in a grilling session (Q1–Q18) before building.

## Decision

**Persistence.** Sessions live in `Videos\Scenra\<session>\`. The folder can be changed with the `sessionsDir` setting or the `SCENRA_SESSIONS_DIR` environment variable; development builds also list the old repo `recordings/takes/`. A recorded session saves `session.json`, `script.json` and `initial.json` (each scene's starting page), and each take saves `actions.json`: what Scenra clicked and when, in seconds from the video's first frame. Capture-matrix takes go to `checks/`.

**Non-destructive edit.** `edit.json` stores, per scene: the chosen take, trims, cuts (manual or automatic, each with a reason, switch-off-able), and excluded scenes. `editor::model::timeline` turns that into the ordered source segments of the finished video. Preview, export and captions all use it. A cut that would go through a click or the cursor's approach to it is skipped with a warning.

**Editor window** (`src/editor/`). Its home screen is the session list. Inside a session there is a scene board, a preview that plays the original files in sync (whole video, or one raw take for placing trims and cuts), a strip per take showing trims, cuts and clicks, undo/redo, and the transcript view.

**Export: Windows' editing engine** (`Windows.Media.Editing`). Each segment becomes a trimmed `MediaClip` plus a `BackgroundAudioTrack` delayed to its place in the output. The render uses `CreateDefaultEncodingProfile()` with the bitrate raised to about 10 Mbit/s at 1080p (a hand-built `CreateMp4` profile fails with `MF_E_ATTRIBUTENOTFOUND` on these takes). Output is H.264 + AAC MP4 in `<session>/export/`. Nothing to install.

**Local audio, with no consent needed.** A speech map comes from 20 ms RMS frames over an adaptive noise floor. **Tidy up** trims dead air before the first word or click and after the last, and shortens pauses over 1.5 s to 0.6 s, never through a click. Export levels each take to −16 LUFS (BS.1770 K-weighting with gating, 30 dB maximum gain, a −1 dBFS limiter), using copies in `.edit-cache/`.

**AI, after one-time consent** (`transcription` setting: ask, allowed or off). Gemini transcribes each take's mic as 16 kHz mono WAV with word times and filler flags. Word times are snapped into the speech map, because Gemini drifts by up to about 0.4 s. The result is cached as `transcript.json`. **Assemble with AI** then does the following for each scene:
- picks the best take: script coverage by LCS alignment, minus stumbles, minus failed clicks, with the latest take winning ties;
- tidies it;
- cuts stumbles, unless they overlap a click.

Every change shows its reason. **Remove automatic changes** reverts all of them, and the presenter's own edits stay.

**Captions.** Transcript words are mapped through the timeline: a word is kept if its midpoint survives the edit. They're grouped into cues of at most two 42-character lines and 6 s, and written as `.srt` next to the MP4.

**Title, description and chapters.** Gemini writes the title and description from the script (or from the transcripts if there's no script). Chapters come from the timeline (scene starts, first at 00:00), with a note when YouTube's rules (at least 3 chapters, each at least 10 s) aren't met. Everything is also saved to `export/description.txt`.

**Retake from the editor.** The session's script and starting pages are loaded into the prompter, the session resumes in record mode at that scene, and new takes are numbered after the existing ones. Back in the editor, a scene's new take is selected automatically.

## Recorder fix found on the way

WGC only delivers frames on change, so a screen that held still at the end of a take ended the MP4 early, and the editor clamped the take to that shorter length, cutting off narration. The recorder now keeps a GPU copy of the latest frame and, at stop, writes it again stamped with a single shared stop time. The mic is cut at that same instant, so video and mic lengths match (verified: they agree within 0.15 s).

## Not yet

Burned-in captions, noise removal, title cards, fades, music, zoom-and-follow, and a crop-region export. Assemble was tested with synthetic speech (Windows TTS); a real narrated session hasn't been through it yet.
