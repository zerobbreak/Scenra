# ADR 0004: Visual Studio, Docker and Android Studio

**Status:** Accepted, 2026-11-02. Visual Studio: launch, steps and scene starts done; Teach is next.

## Context

ADR 0003 made VS Code a first-class target. Presenters also need Visual Studio, Docker and Android Studio. The presenter chose (2026-11-02):

- **Visual Studio and Android Studio:** the same feature set as VS Code: open a file, scroll to code, highlight lines, run a command, search, and Teach mode for all of it. Building, debugging and the Android emulator are out of scope for now.
- **Docker:** both `docker` commands in a terminal and the Docker Desktop app.
- **Order:** Visual Studio, then Docker, then Android Studio (not installed on the development machine yet).

## Research (2026-11-02, this machine: Visual Studio Community 2026, 18.10)

| Route | Test | Verdict |
|---|---|---|
| DTE (Visual Studio's COM automation), found in the Running Object Table as `!VisualStudio.DTE.18.0:<pid>` | Attached in about 1 s to `devenv` opened on a folder (no solution). `ItemOperations.OpenFile` 38 ms; `Selection.GotoLine` + `LineDown(extend)` selected 195–197 and read back `TopLine`/`BottomLine` and the text; `ExecuteCommand("Edit.ToggleWordWrap")` worked | Actions and the open file and selection |
| DTE text window (`ActiveWindow.Object.ActivePane.StartPoint`) for the visible lines | Empty in both PowerShell 7 and 5.1: the document window exposes no `TextWindow` for a Rust file opened from a folder | Not usable |
| UI Automation on the editor (`WpfTextView`, name "Text Editor") | Supports TextPattern. `GetVisibleRanges` returns one range per non-empty visible line, text `"<line number> <code>"`, each with its screen rectangle (a partly hidden top line has a shorter one). `GetSelection` returns the selected text with one rectangle per line. The document text itself has no numbers | Visible lines, the fractional top line and screen positions |

## Decisions

- **Visual Studio engine:** DTE for instant actions and state (open file, go to line, select, run a command, active document, selection); UI Automation's TextPattern for the visible lines and where they are on screen. On-camera motion uses real input, as in VS Code:
  - **Open:** Ctrl+T opens the unified **Search Visual Studio** box in 2026 (not the old Go To All). Typing `f:<name>` filters to files; Scenra presses Enter only once the selected result's path is the file, otherwise it presses Escape and opens the file through DTE.
  - **Scroll:** eased, calibrated wheel scrolling; a long jump goes most of the way first with `Edit.ScrollLineTop`.
  - **Highlight:** click at column 0 of the first line, Shift+click at column 0 of the line after the last, then check the selection through DTE and correct it if a click landed a line off.
  - **Palette:** runs the command by name through DTE, instantly. Feature Search (Ctrl+Q) mixes commands with settings and docs, so it can't pick one reliably.
  - **Search:** Find in Files (Ctrl+Shift+F).
- **Scene starts:** the open file and top line, restored through DTE (`OpenFile`, `GotoLine`, `Edit.ScrollLineTop`; `Window.CloseAllDocuments` for none). Visual Studio has no side-panel state in a scene.
- **Settings:** Visual Studio runs with the presenter's own settings (the experimental instance resets everything). Replacing it as the target asks it to quit first, so it can prompt for unsaved files, and only kills it after 15 s.
- **Visible lines:** Visual Studio's TextPattern also reports a few laid-out lines below the editor's edge; only lines inside the editor's rectangle count.
- **Shared layers:** `dte.rs` (late-bound COM over `IDispatch`, retrying calls Visual Studio rejects while busy) and `uia.rs` (the UI Automation client). The UI Automation layer is also slice 3 of ADR 0003.
- **Steps and anchors** are the same as VS Code's (`steps::Step`, text and symbol anchors), so scripts and Teach work the same across editors.
