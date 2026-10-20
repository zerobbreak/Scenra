# Scenra — Project Context & Feature Reference

**Version:** 0.1  
**Date:** October 2026  
**Status:** Living project specification  
**Tagline:** *From script to scene.*

## 1. Project Overview

**Category:** AI-assisted desktop recording and content creation  
**Platform:** Windows first  
**Architecture:** Local-first desktop application

### Vision

Scenra is an AI-powered recording director that helps developers and content creators produce polished application demonstrations, tutorials, product showcases, and development videos.

Instead of manually navigating an application while narrating, users teach Scenra how their application works, generate scripts from project context, and let Scenra execute approved actions while they provide the voice-over.

The goal is to make software demonstrations **structured, repeatable, professional, and less mentally demanding**.

### Objectives

- Understand projects through source code, documentation, and AI development sessions.
- Generate scripts grounded in project goals and implemented features.
- Learn application workflows from user demonstrations.
- Automate approved interactions with visible, natural mouse and keyboard actions.
- Synchronize actions with narration.
- Control OBS Studio to record individual scenes.
- Support scene retakes without restarting a whole recording.
- Store projects, scripts, history, and preferences locally.
- Support multiple AI providers through a modular interface.

## 2. Core Workflow

1. **Understand:** Import repository, documentation, and AI coding session history.
2. **Script:** Generate narration, scene breakdowns, action plans, and timing suggestions.
3. **Teach:** User demonstrates the desired application workflow once.
4. **Rehearse:** Scenra repeats and validates approved actions without recording.
5. **Record:** User narrates while Scenra executes actions and OBS records.
6. **Export:** Save clips, separate narration audio, and recording metadata.

## 3. Technology Stack

| Layer | Technology | Responsibility |
|---|---|---|
| Desktop framework | Tauri | Native application shell |
| Core engine | Rust | Orchestration, automation, IPC |
| User interface | React + TypeScript | Editor, teleprompter, dashboard |
| Local database | SQLite | Project and recording metadata |
| AI | Gemini API initially | Project understanding and script generation |
| Element discovery | Chrome DevTools Protocol (CDP) | Inspect Chromium-based applications |
| Native automation | Windows UI Automation (UIA) | Inspect native Windows controls |
| Input simulation | Windows APIs via Rust | Visible mouse and keyboard input |
| Recording | OBS + obs-websocket | Video and audio capture |
| Credential storage | Windows Credential Manager | API keys and sensitive demo values |

Python is not required initially. Playwright is a possible future browser automation option, if direct CDP proves insufficient.

## 4. Feature Modules

| Module | Features | Priority |
|---|---|---|
| Project Context Engine | Repository analysis, Claude Code history import, documentation, project profiles | P1 |
| AI Script Studio | Script generation, scene breakdown, narration, content formats | P1 |
| Teach Engine | Capture interactions, identify elements, record workflows | P1 |
| Automation Engine | CDP, UIA, visible cursor, human-like typing, scrolling | P1 |
| Recording Director | OBS integration, narration checkpoints, hotkeys, scene recording | P1 |
| Scene Manager | Rehearsals, retries, scene boundaries, recording history | P1 |
| Content Export | Video clips, separate narration tracks, metadata | P1 |
| Advanced Production | TTS, subtitles, zoom effects, clip stitching | Later |
| Additional Platforms | Third-party apps, other operating systems | Later |

## 5. Project Context Engine

Scenra should understand why an application was built, not just inspect its current UI.

Potential sources:

- Claude Code sessions and `CLAUDE.md`
- `AGENTS.md` and other project instructions
- Gemini CLI histories where available
- Repository files and README documents
- Architecture plans and product documentation

Extracted knowledge includes:

- Project purpose and target audience
- Architecture and technology decisions
- Implemented versus planned features
- Development milestones and challenges
- Demonstrable workflows
- Potential video topics

**Verification:** AI session history is evidence of intent and decisions, not proof that features work. Source inspection and rehearsal should validate functionality.

**Privacy:** Keep imported context local by default. Exclude or redact sensitive content before sending approved excerpts to a cloud AI provider.

## 6. Automation and Recording

### Locator and Input Engines

`LocatorEngine` inspects, locates, and verifies UI elements. Implementations:

- **CDP locator:** Chrome, Edge, and compatible Electron/WebView2 applications when debugging access is enabled.
- **UIA locator:** Native Windows controls, file dialogs, and applications without suitable CDP access.
- **Vision fallback:** Screenshot or text-based recognition when structured inspection is insufficient; requires validation.

A shared Rust `InputEngine` performs OS-level mouse and keyboard actions, making cursor movement visible in the recording regardless of which locator found the element.

### Teach Mode

Users complete a continuous walkthrough. Scenra captures actions, correlates clicks with UI elements, and allows the walkthrough to be split into independently editable and re-teachable scenes. Sensitive fields should be represented by placeholders at capture time.

### Narration Synchronization

Each step can execute **before**, **during**, or **after** narration. Manual hotkey checkpoints are the default in v1. The scene advances when its required narration and automation states are complete. Speech-based cue detection is a later enhancement.

### Recording and Retakes

OBS records application interactions and microphone audio. The teleprompter must stay out of the captured image. Each scene is saved separately, enabling retries without redoing the whole demo.

Suggested output structure:

```text
recordings/
└── scene-03/
    ├── video.mkv
    ├── microphone.wav
    ├── actions.json
    └── metadata.json
```

OBS may record multitrack MKV with the microphone isolated on a dedicated track, which Scenra can extract afterward. Preflight must validate the required OBS configuration.

### Safety

- No unapproved AI improvisation during a live recording.
- Unexpected physical input pauses automation, excluding recognized control hotkeys.
- An independent emergency-stop mechanism restores user control.
- Authentication state and secrets must not be sent to AI prompts or exposed in logs.
- Debugging endpoints should be enabled only in trusted local development sessions.

## 7. Initial Scope and Future Expansion

### Initial Release

- Windows desktop application
- Primarily developer-owned applications
- AI-assisted project understanding and script generation
- Teach → Rehearse → Record workflow
- Approved automation with natural cursor movement
- OBS recording and individual scene retakes
- Local project storage and recording history

### Later

- Automatic speech synchronization
- Optional text-to-speech narration
- Advanced cursor effects and zooms
- Video stitching and subtitle generation
- Additional AI providers
- Broader third-party application support
- Cross-platform automation

## 8. Phase 0 — Technical Proof of Concept

The prototype should demonstrate:

1. Connect to a compatible Chromium-based application through CDP.
2. Locate a UI element and translate its position to physical screen coordinates.
3. Move the real cursor and interact with the element.
4. Connect to OBS and record the interaction.
5. Show a teleprompter that is excluded from the recording.
6. Safely handle unexpected physical input and emergency-stop commands.

**Key uncertainty:** Test capture exclusion across OBS source types, display scaling, dialogs, overlays, and single-monitor setups. `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` is a candidate mechanism, not a guarantee for every capture pipeline.

## 9. Product Principles

1. **Local-first:** Keep project metadata, scripts, histories, and recordings on the user's machine by default.
2. **AI-assisted, not uncontrolled:** AI plans; users approve; deterministic engines execute.
3. **Recording quality matters:** Natural timing, visible interactions, reliable capture, and easy retakes are essential.
4. **Modular architecture:** Replace AI providers, locator engines, and recording backends without redesigning Scenra.
5. **Reusable understanding:** A project profile should support many videos without repeated explanation.

## 10. Open Questions and Validation Tasks

- Confirm CDP connectivity and coordinate mapping for Chrome/Edge, Electron, and WebView2 individually.
- Compare approaches for native Windows UIA access from Rust.
- Verify teleprompter exclusion and cursor/ripple capture in OBS.
- Determine reliable scene reset/setup hooks for retakes.
- Validate OS-level input tagging, hotkey handling, and emergency stop.
- Design provider-neutral AI interfaces and per-project credential references.
- Benchmark memory/CPU usage with OBS and the target application running.

---

*This is the baseline project context for Scenra. Update it as features, architectural decisions, and implementation findings evolve.*
