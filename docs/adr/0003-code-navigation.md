# ADR 0003: Driving VS Code and other apps

**Status:** Accepted, 2026-11-01. Slices 1 (VS Code engine) and 2 (Teach in VS Code) are done; slice 3 (UIA layer) is next.

## Context

Scenra could only drive Chromium pages (Chrome, WebView2) over CDP. Moving through a page was an instant `scrollIntoView`; there was no wheel scrolling, no keyboard shortcuts and no UI Automation. Presenters want to demo their codebase: open files, scroll to code, highlight it, and move between VS Code, the browser and the terminal.

## Research (2026-11-01, this machine)

| Route | Test | Verdict |
|---|---|---|
| CDP into VS Code (Electron, `--remote-debugging-port`) | Works with VS Code 1.144. Explorer rows readable by `aria-label`; tabs, editors and visible line numbers readable | First-class route for VS Code |
| `code -r -g file:line` with the same `--user-data-dir` | Opened `take.rs` centred on line 120 in the same window | Instant open-at-line |
| Windows UI Automation | VS Code by default: 13 elements. With `--force-renderer-accessibility`: 149 (editor, tabs, 3 scrollable areas) | Generic any-app layer; shallow for editors |
| Playwright `_electron.launch` | [Broken for modern VS Code](https://github.com/stevez/playwright-repl/issues/555) | Not used; Scenra launches VS Code and attaches |
| Vision (Gemini Computer Use) | Google: "not yet optimized for desktop OS-level control" | Later, as a validated fallback |
| JetBrains | Needs Java Access Bridge | Later |

## Decisions (grilling Q1–Q16)

- **Targets:** VS Code is first-class (CDP plus the `code` CLI). Native apps go through UIA, proven on Windows Terminal, File Explorer and Notepad. Vision and JetBrains come later.
- **Actions:**
  - open a file at a line (visible via Ctrl+P, or instant);
  - scroll to lines;
  - highlight lines;
  - click in the explorer;
  - run a command-palette command;
  - search;
  - switch window;
  - type a scripted terminal command.
- **On camera:** human-like by default, with instant available per action. Setup between scenes is always instant.
- **Scrolling:** real wheel events in sub-notch steps, eased, about 15 lines/s, ending with the target range centred. A long jump (over about 150 lines) jumps first, then scrolls the last stretch.
- **Highlighting:** a real selection: click the first line number, Shift+click the last.
- **Authoring:** Teach mode is the main path, with an editable action list. Teach records:
  - clicks by element;
  - keyboard shortcuts;
  - typing only in Quick Open, Search and the palette;
  - scroll and selection as end states;
  - window switches.
- **Anchors:** code steps store the first line's text and the nearest symbol as well as a line-number hint. Playback follows the anchor and refuses if it's gone.
- **Start state for code scenes:** the open file, the top visible line and the side panel. Replay setup restores them instantly.
- **Recording profile:** a separate VS Code `--user-data-dir`, seeded with the presenter's theme and font family. Defaults: font size 16, no minimap or breadcrumbs, no welcome page, smooth scrolling and auto-save on. The presenter's own VS Code is never touched. Each project has a codebase folder setting.
- **Gemini:** sees only the highlighted line ranges, behind a separate "send code" consent.
- **Typing:** scripted terminal commands only; editing code comes later.
- **Build order:**
  1. VS Code engine, with hand-written steps;
  2. Teach in VS Code;
  3. UIA layer;
  4. terminal commands.

## Slice 2: Teach in VS Code (2026-11-02)

Decisions made while building it:

- **Explorer and tab clicks** are recorded as the file that opened and replay as Open through Quick Open, which works whatever folders are expanded. The video shows Quick Open instead of the explorer click.
- **End states, not events:** Scenra polls the workbench. A new file is an Open at once; a scroll or selection becomes a step after the editor holds still for 0.9 s. VS Code's own scroll restore after opening a file, and a palette command's effects, aren't separate steps.
- **Commits without an end state** come from an in-page key listener: Enter on a `>` palette entry (its visible name, not its aria-label, which carries the key binding) and Enter in the Search view.
- **Shortcuts** are kept as flagged, skipped steps, unless the editor shows their effect within 2.5 s (Ctrl+End → the Scroll stands for it). Key combinations in text boxes are text editing and ignored. Typing in the editor isn't recorded and warns once.
- **Gemini** sees only step names ("Highlight take.rs 195–197"), never code.

