# ADR 0004: Visual Studio, Docker and Android Studio

**Status:** Accepted, 2026-11-02. Visual Studio is in progress.

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

- **Visual Studio engine:** DTE for instant actions and state (open file, go to line, select, run a command, active document, selection); UI Automation's TextPattern for the visible lines and where they are on screen. On-camera motion uses real input, as in VS Code: eased wheel scrolling, clicks in the line-number margin for a highlight, **Go To All** (Ctrl+T) to open a file, **Feature Search** (Ctrl+Q) for commands, **Find in Files** (Ctrl+Shift+F) for search.
- **Shared layers:** `dte.rs` (late-bound COM over `IDispatch`, retrying calls Visual Studio rejects while busy) and `uia.rs` (the UI Automation client). The UI Automation layer is also slice 3 of ADR 0003.
- **Steps and anchors** are the same as VS Code's (`steps::Step`, text and symbol anchors), so scripts and Teach work the same across editors.
