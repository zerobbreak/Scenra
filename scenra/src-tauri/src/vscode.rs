//! VS Code as a target (docs/adr/0003-code-navigation.md): Scenra launches it with a debug port
//! and its own recording profile, then drives it like any Chromium page (the workbench is an
//! Electron page), plus the `code` CLI for instant open-at-line.
//!
//! The recording profile (`--user-data-dir`) is separate from the presenter's VS Code, so their
//! settings, extensions and windows are never touched. It's seeded once with their theme and
//! font family, plus defaults that read well on video.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

#[derive(Clone, Debug)]
pub struct VsCode {
    pub exe: PathBuf,
    /// The recording profile (`--user-data-dir`).
    pub profile: PathBuf,
    /// The codebase folder VS Code opened.
    pub folder: PathBuf,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The installed VS Code: per-user install, machine-wide install, or whatever `code` on PATH is.
pub fn find_exe() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    for (var, sub) in [("LOCALAPPDATA", r"Programs\Microsoft VS Code\Code.exe"), ("ProgramFiles", r"Microsoft VS Code\Code.exe")] {
        if let Some(base) = std::env::var_os(var) {
            candidates.push(PathBuf::from(base).join(sub));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("code.cmd").is_file() {
                candidates.push(dir.join("..").join("Code.exe"));
            }
        }
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| "VS Code isn't installed (looked for Code.exe in the usual places and on PATH)".to_owned())
}

/// `bin\code.cmd` next to `Code.exe`: the CLI that talks to a running instance.
fn cli(exe: &Path) -> PathBuf {
    exe.parent().map(|d| d.join("bin").join("code.cmd")).unwrap_or_else(|| PathBuf::from("code.cmd"))
}

/// The value of a string setting in a VS Code `settings.json` (JSON with comments), if set.
fn string_setting(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut value = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(value),
            '\\' => value.push(chars.next()?),
            c => value.push(c),
        }
    }
    None
}

#[derive(Serialize)]
struct Seed<'a> {
    #[serde(rename = "workbench.colorTheme")]
    theme: &'a str,
    #[serde(rename = "editor.fontFamily", skip_serializing_if = "Option::is_none")]
    font_family: Option<&'a str>,
    #[serde(rename = "editor.fontSize")]
    font_size: u32,
    #[serde(rename = "editor.minimap.enabled")]
    minimap: bool,
    #[serde(rename = "breadcrumbs.enabled")]
    breadcrumbs: bool,
    #[serde(rename = "workbench.startupEditor")]
    startup_editor: &'a str,
    #[serde(rename = "editor.smoothScrolling")]
    smooth_scrolling: bool,
    #[serde(rename = "workbench.list.smoothScrolling")]
    list_smooth_scrolling: bool,
    #[serde(rename = "files.autoSave")]
    auto_save: &'a str,
    #[serde(rename = "workbench.editor.enablePreview")]
    preview_tabs: bool,
    #[serde(rename = "security.workspace.trust.enabled")]
    workspace_trust: bool,
    #[serde(rename = "update.mode")]
    update_mode: &'a str,
    #[serde(rename = "telemetry.telemetryLevel")]
    telemetry: &'a str,
    #[serde(rename = "workbench.tips.enabled")]
    tips: bool,
    #[serde(rename = "window.restoreWindows")]
    restore_windows: &'a str,
    #[serde(rename = "extensions.ignoreRecommendations")]
    ignore_recommendations: bool,
    /// VS Code 1.14x's first-run "Welcome / sign in" overlay takes keyboard focus.
    #[serde(rename = "workbench.welcomePage.experimentalOnboarding")]
    onboarding: bool,
    /// The Chat panel on the right would take a third of the screen.
    #[serde(rename = "workbench.secondarySideBar.defaultVisibility")]
    secondary_side_bar: &'a str,
}

/// The recording profile's settings, from the presenter's own settings text (if any).
pub fn seed_settings(user_settings: Option<&str>) -> String {
    let theme = user_settings.and_then(|t| string_setting(t, "workbench.colorTheme"));
    let font = user_settings.and_then(|t| string_setting(t, "editor.fontFamily"));
    let seed = Seed {
        theme: theme.as_deref().unwrap_or("Default Dark Modern"),
        font_family: font.as_deref(),
        font_size: 16,
        minimap: false,
        breadcrumbs: false,
        startup_editor: "none",
        smooth_scrolling: true,
        list_smooth_scrolling: true,
        auto_save: "afterDelay",
        preview_tabs: false,
        workspace_trust: false,
        update_mode: "none",
        telemetry: "off",
        tips: false,
        restore_windows: "none",
        ignore_recommendations: true,
        onboarding: false,
        secondary_side_bar: "hidden",
    };
    serde_json::to_string_pretty(&seed).unwrap_or_default()
}

/// Creates the recording profile's `settings.json` the first time; later the presenter may edit it.
pub fn ensure_profile(profile: &Path) -> Result<(), String> {
    let settings = profile.join("User").join("settings.json");
    if settings.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(settings.parent().unwrap_or(profile)).map_err(err)?;
    let user = std::env::var_os("APPDATA")
        .map(|a| PathBuf::from(a).join(r"Code\User\settings.json"))
        .and_then(|p| std::fs::read_to_string(p).ok());
    std::fs::write(&settings, seed_settings(user.as_deref())).map_err(|e| format!("{}: {e}", settings.display()))
}

/// Starts VS Code on `vs.folder` with a debug port and the recording profile.
pub fn spawn(vs: &VsCode, port: u16) -> Result<Child, String> {
    ensure_profile(&vs.profile)?;
    let extensions = vs.profile.with_file_name("vscode-extensions");
    Command::new(&vs.exe)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", vs.profile.display()))
        .arg(format!("--extensions-dir={}", extensions.display()))
        .args(["--new-window", "--disable-workspace-trust", "--skip-welcome"])
        .arg(&vs.folder)
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", vs.exe.display()))
}

/// Opens `file` at `line` in the running recording window, instantly (`code -r -g`).
pub fn goto(vs: &VsCode, file: &Path, line: u32) -> Result<(), String> {
    let status = Command::new(cli(&vs.exe))
        .arg(format!("--user-data-dir={}", vs.profile.display()))
        .args(["-r", "-g"])
        .arg(format!("{}:{}", file.display(), line.max(1)))
        .status()
        .map_err(|e| format!("couldn't run the code CLI: {e}"))?;
    if !status.success() {
        return Err(format!("the code CLI couldn't open {}:{line}", file.display()));
    }
    Ok(())
}

/// Turns a codebase-relative path (forward or back slashes) into a full path inside `folder`,
/// refusing anything that would leave it.
pub fn resolve(folder: &Path, relative: &str) -> Result<PathBuf, String> {
    let rel = relative.replace('\\', "/");
    if rel.split('/').any(|p| p == ".." || p.contains(':')) || rel.starts_with('/') {
        return Err(format!("{relative} isn't a path inside the codebase"));
    }
    Ok(folder.join(rel.replace('/', "\\")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_string_settings_from_jsonc() {
        let text = r#"{
            // my theme
            "workbench.colorTheme": "Solarized \"Light\"",
            "editor.fontFamily" : 'not json',
            "editor.fontSize": 14,
        }"#;
        assert_eq!(string_setting(text, "workbench.colorTheme").as_deref(), Some("Solarized \"Light\""));
        assert_eq!(string_setting(text, "editor.fontFamily"), None);
        assert_eq!(string_setting(text, "missing"), None);
    }

    #[test]
    fn seed_keeps_the_presenters_look_with_recording_defaults() {
        let seeded: serde_json::Value = serde_json::from_str(&seed_settings(Some(r#"{ "workbench.colorTheme": "Monokai", "editor.fontFamily": "Cascadia Code" }"#))).unwrap();
        assert_eq!(seeded["workbench.colorTheme"], "Monokai");
        assert_eq!(seeded["editor.fontFamily"], "Cascadia Code");
        assert_eq!(seeded["editor.fontSize"], 16);
        assert_eq!(seeded["editor.minimap.enabled"], false);
        let plain: serde_json::Value = serde_json::from_str(&seed_settings(None)).unwrap();
        assert_eq!(plain["workbench.colorTheme"], "Default Dark Modern");
        assert!(plain.get("editor.fontFamily").is_none());
    }

    #[test]
    fn codebase_paths_stay_inside_the_folder() {
        let folder = Path::new(r"C:\code\app");
        assert_eq!(resolve(folder, "src/main.rs").unwrap(), PathBuf::from(r"C:\code\app\src\main.rs"));
        for bad in ["../x", "/etc", r"C:\x", "a/../../b"] {
            assert!(resolve(folder, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn finds_vs_code_when_installed() {
        // Not an assertion on CI-less machines without VS Code: just that it doesn't panic.
        let _ = find_exe();
    }
}
