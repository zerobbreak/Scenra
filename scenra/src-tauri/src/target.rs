//! Phase 0 · Step 2 — launch the demo target with a WebView2 debug port and attach over CDP.
//!
//! WebView2 reads extra Chromium flags from `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`. Scenra sets
//! it only on the child it spawns, never globally, so the port exists for that run only.
//! `WEBVIEW2_USER_DATA_FOLDER` points at a Scenra-owned folder for the same reason: a second
//! instance that shares a user data folder joins the first one's browser process, and the
//! debug flag is then silently ignored (e.g. while `pnpm target` is also running).
//!
//! The capability profile follows PROJECT_OUTLINE §5. Only the CDP section exists so far.

use crate::cdp::{self, Session};
use serde::Serialize;
use std::collections::HashMap;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

/// Built by `cargo build -p demo-target` (or `pnpm target`) in the shared workspace target dir.
const DEMO_TARGET_EXE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug/demo-target.exe");
/// A fresh user data folder makes WebView2 unpack its profile on first start, so be patient.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct LaunchedTarget(Mutex<Option<Running>>);

struct Running {
    child: Child,
    port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProfile {
    /// The page title once CDP answers, else the executable name.
    application: String,
    process: ProcessInfo,
    capabilities: Capabilities,
    /// `cdp` when it is usable, else `None` until the UIA and vision locators exist.
    preferred_locator: Option<&'static str>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessInfo {
    pid: u32,
    exe: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    cdp: CdpCapability,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct CdpCapability {
    available: bool,
    /// `webview2`, `edge`, `chrome`, `electron`, or `unknown`.
    flavor: Option<&'static str>,
    endpoint: Option<String>,
    browser: Option<String>,
    /// The process that owns the listening socket (`msedgewebview2.exe` for WebView2).
    endpoint_process: Option<String>,
    /// The endpoint's process is the target or one of its descendants.
    owned_by_target: bool,
    loopback_only: bool,
    page_url: Option<String>,
    /// Elements with a `data-testid`, a first sign the CdpLocator will have something to find.
    test_ids: Option<u64>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command(async)]
pub fn launch_target(
    app: AppHandle,
    state: State<'_, LaunchedTarget>,
) -> Result<CapabilityProfile, String> {
    let user_data = app.path().app_local_data_dir().map_err(err)?.join("targets/demo-target");
    replace_running(&state, || spawn_demo_target(&user_data))
}

/// Launches Chrome on `url` with its own demo profile and a debug port (PROJECT_OUTLINE §5:
/// recent Chrome refuses remote debugging on the default profile, and a demo profile keeps the
/// recording free of your own logins and extensions).
#[tauri::command(async)]
pub fn launch_chrome(
    app: AppHandle,
    state: State<'_, LaunchedTarget>,
    url: String,
) -> Result<CapabilityProfile, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("enter a full http(s) URL".into());
    }
    let profile = app.path().app_local_data_dir().map_err(err)?.join("targets/chrome-profile");
    replace_running(&state, || spawn_chrome(&url, &profile))
}

/// Stops whatever target is running, starts a new one and profiles it.
fn replace_running(
    state: &LaunchedTarget,
    spawn: impl FnOnce() -> Result<(Child, u16), String>,
) -> Result<CapabilityProfile, String> {
    let mut slot = state.0.lock().map_err(err)?;
    if let Some(mut previous) = slot.take() {
        let _ = previous.child.kill();
        let _ = previous.child.wait();
    }
    let (child, port) = spawn()?;
    let profile = detect(child.id(), port);
    *slot = Some(Running { child, port });
    Ok(profile)
}

/// The debug port of the running target, if it is still alive.
pub fn running_port(state: &LaunchedTarget) -> Result<u16, String> {
    running(state).map(|(_, port)| port)
}

/// The running target's process id and debug port.
pub fn running(state: &LaunchedTarget) -> Result<(u32, u16), String> {
    let mut slot = state.0.lock().map_err(err)?;
    if let Some(running) = slot.as_mut()
        && running.child.try_wait().map_err(err)?.is_none()
    {
        return Ok((running.child.id(), running.port));
    }
    *slot = None;
    Err("no target is running: launch Chrome or the demo target first".into())
}

fn chrome_exe() -> Result<PathBuf, String> {
    let mut candidates = vec![PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe")];
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join(r"Google\Chrome\Application\chrome.exe"));
    }
    candidates.push(PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"));
    candidates.into_iter().find(|p| p.exists()).ok_or_else(|| "Chrome isn't installed".into())
}

pub(crate) fn spawn_chrome(url: &str, profile: &Path) -> Result<(Child, u16), String> {
    let exe = chrome_exe()?;
    let port = free_port()?;
    let child = Command::new(&exe)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(["--no-first-run", "--no-default-browser-check", "--start-maximized", "--new-window", url])
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", exe.display()))?;
    wait_for_devtools(child, port, "Chrome")
}

/// Starts the demo target with a debug port and waits until DevTools answers.
pub(crate) fn spawn_demo_target(user_data: &Path) -> Result<(Child, u16), String> {
    let exe = PathBuf::from(DEMO_TARGET_EXE);
    if !exe.exists() {
        return Err(format!(
            "demo target not built: {} is missing. Run `cargo build -p demo-target` first.",
            exe.display()
        ));
    }
    let port = free_port()?;

    let child = Command::new(&exe)
        .env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", format!("--remote-debugging-port={port}"))
        .env("WEBVIEW2_USER_DATA_FOLDER", user_data)
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", exe.display()))?;
    wait_for_devtools(child, port, "demo target")
}

/// Waits until DevTools answers on `port`, failing early if the process exits.
fn wait_for_devtools(mut child: Child, port: u16, name: &str) -> Result<(Child, u16), String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while cdp::version(port).is_err() {
        if let Some(status) = child.try_wait().map_err(err)? {
            return Err(format!("{name} exited during startup ({status})"));
        }
        if Instant::now() > deadline {
            // Keep the process: the profile still reports what it can, with CDP unavailable.
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok((child, port))
}

/// Re-runs detection against the running target, as preflight will.
#[tauri::command(async)]
pub fn target_profile(state: State<'_, LaunchedTarget>) -> Result<Option<CapabilityProfile>, String> {
    let mut slot = state.0.lock().map_err(err)?;
    if let Some(running) = slot.as_mut()
        && running.child.try_wait().map_err(err)?.is_some()
    {
        *slot = None;
    }
    Ok(slot.as_ref().map(|running| detect(running.child.id(), running.port)))
}

#[tauri::command]
pub fn stop_target(state: State<'_, LaunchedTarget>) -> Result<(), String> {
    if let Some(mut running) = state.0.lock().map_err(err)?.take() {
        let _ = running.child.kill();
        let _ = running.child.wait();
    }
    Ok(())
}

/// Called on app exit so the target doesn't outlive Scenra with an open debug port.
pub fn shutdown(app: &AppHandle) {
    if let Ok(mut slot) = app.state::<LaunchedTarget>().0.lock()
        && let Some(mut running) = slot.take()
    {
        let _ = running.child.kill();
    }
}

fn free_port() -> Result<u16, String> {
    // Ask the OS for a free loopback port, then release it for the target to take.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(err)?;
    Ok(listener.local_addr().map_err(err)?.port())
}

fn detect(pid: u32, port: u16) -> CapabilityProfile {
    let processes = processes();
    let exe = processes.get(&pid).map(|p| p.exe.clone()).unwrap_or_default();
    let mut warnings = Vec::new();
    let mut application = exe.clone();

    let cdp = probe_cdp(pid, port, &processes, &mut warnings, &mut application).unwrap_or_else(|e| {
        warnings.push(format!("CDP unavailable on port {port}: {e}"));
        CdpCapability::default()
    });

    CapabilityProfile {
        application,
        process: ProcessInfo { pid, exe },
        preferred_locator: cdp.available.then_some("cdp"),
        capabilities: Capabilities { cdp },
        warnings,
    }
}

fn probe_cdp(
    pid: u32,
    port: u16,
    processes: &HashMap<u32, ProcessEntry>,
    warnings: &mut Vec<String>,
    application: &mut String,
) -> Result<CdpCapability, String> {
    let version = cdp::version(port)?;

    let listener = listener(port).ok_or("no process is listening on the port")?;
    let endpoint_process = processes.get(&listener.pid).map(|p| p.exe.clone());
    let owned_by_target = descends_from(processes, listener.pid, pid);
    if !owned_by_target {
        warnings.push(format!(
            "port {port} belongs to {} (pid {}), not the launched target",
            endpoint_process.as_deref().unwrap_or("an unknown process"),
            listener.pid
        ));
    }
    if !listener.loopback {
        warnings.push(format!("debug port {port} is reachable from the network, not just loopback"));
    }

    let flavor = match endpoint_process.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("msedgewebview2.exe") => "webview2",
        Some("msedge.exe") => "edge",
        Some("chrome.exe") => "chrome",
        _ if version.user_agent.contains("Electron/") => "electron",
        _ => "unknown",
    };

    // DevTools answers before the app page loads, and WebView2 first shows an empty document
    // that already reports `complete`, so also wait for the body to have content.
    let deadline = Instant::now() + PAGE_LOAD_TIMEOUT;
    let (page, info) = loop {
        let page = cdp::targets(port)?
            .into_iter()
            .find(|t| t.kind == "page" && t.url != "about:blank");
        if let Some(page) = page {
            let mut session = Session::connect(&page.web_socket_debugger_url)?;
            let info = session.evaluate(
                "({ ready: document.readyState === 'complete' && document.body?.childElementCount > 0, \
                    title: document.title, \
                    testIds: document.querySelectorAll('[data-testid]').length })",
            )?;
            if info["ready"] == true || Instant::now() > deadline {
                break (page, info);
            }
        } else if Instant::now() > deadline {
            return Err("no page target".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if info["ready"] != true {
        warnings.push(format!("page {} hadn't finished loading", page.url));
    }
    if let Some(title) = info["title"].as_str().filter(|t| !t.is_empty()) {
        *application = title.to_owned();
    }

    Ok(CdpCapability {
        available: owned_by_target,
        flavor: Some(flavor),
        endpoint: Some(format!("127.0.0.1:{port}")),
        browser: Some(version.browser),
        endpoint_process,
        owned_by_target,
        loopback_only: listener.loopback,
        page_url: Some(page.url),
        test_ids: info["testIds"].as_u64(),
    })
}

struct Listener {
    pid: u32,
    loopback: bool,
}

/// Finds the IPv4 process listening on `port`, from the OS TCP table.
fn listener(port: u16) -> Option<Listener> {
    let mut size = 0u32;
    let mut table: Vec<u32> = Vec::new();
    // The table can grow between the size query and the read, so retry a few times.
    for _ in 0..3 {
        table.resize((size as usize).div_ceil(4), 0);
        let ptr = (!table.is_empty()).then(|| table.as_mut_ptr().cast());
        let rc = unsafe {
            GetExtendedTcpTable(ptr, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)
        };
        if rc == 0 && ptr.is_some() {
            let table = table.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
            let rows = unsafe {
                std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize)
            };
            // Ports and addresses are in network byte order.
            return rows.iter().find(|row| u16::from_be(row.dwLocalPort as u16) == port).map(|row| {
                Listener {
                    pid: row.dwOwningPid,
                    loopback: Ipv4Addr::from(u32::from_be(row.dwLocalAddr)).is_loopback(),
                }
            });
        }
    }
    None
}

struct ProcessEntry {
    parent: u32,
    exe: String,
}

fn processes() -> HashMap<u32, ProcessEntry> {
    let mut map = HashMap::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return map;
    };
    let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while ok {
        let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
        map.insert(entry.th32ProcessID, ProcessEntry { parent: entry.th32ParentProcessID, exe });
        ok = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    map
}

/// `root` and every process descended from it, e.g. Chrome's or WebView2's browser processes.
pub fn process_tree(root: u32) -> std::collections::HashSet<u32> {
    let all = processes();
    all.keys().copied().filter(|pid| descends_from(&all, *pid, root)).collect()
}

/// True if `pid` is `ancestor` or a descendant of it. WebView2's browser process is a
/// child of the host app, so a few levels are enough; the cap also guards against PID reuse loops.
fn descends_from(processes: &HashMap<u32, ProcessEntry>, mut pid: u32, ancestor: u32) -> bool {
    for _ in 0..8 {
        if pid == ancestor {
            return true;
        }
        match processes.get(&pid) {
            Some(entry) if entry.parent != 0 && entry.parent != pid => pid = entry.parent,
            _ => return false,
        }
    }
    false
}

/// Serves the Lumina fixture over plain HTTP on loopback, for live tests against Chrome.
#[cfg(test)]
pub(crate) mod fixture {
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    use std::time::Duration;

    /// Stops serving when dropped.
    pub(crate) struct Server {
        pub addr: SocketAddr,
        stopping: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    /// `page_delay` holds back every page response, to test against a slow network.
    pub(crate) fn serve(page_delay: Duration) -> Server {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let stopping = Arc::new(AtomicBool::new(false));
        let stopped = stopping.clone();
        let thread = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stopped.load(Ordering::SeqCst) { break; }
                let Ok(mut stream) = connection else { break };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                let mut request = [0u8; 4096];
                let Ok(n) = stream.read(&mut request) else { continue };
                if n == 0 { continue; }
                let request = String::from_utf8_lossy(&request[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (content, kind) = match path {
                    "/main.js" => (include_str!("../../../fixtures/demo-target/src/main.js"), "text/javascript"),
                    "/styles.css" => (include_str!("../../../fixtures/demo-target/src/styles.css"), "text/css"),
                    _ => { std::thread::sleep(page_delay); (include_str!("../../../fixtures/demo-target/src/index.html"), "text/html") }
                };
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{content}", content.len());
            }
        });
        Server { addr, stopping, thread: Some(thread) }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stopping.store(true, Ordering::SeqCst);
            // Wake the accept loop so it sees the flag.
            let _ = std::net::TcpStream::connect(self.addr);
            if let Some(thread) = self.thread.take() { let _ = thread.join(); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the demo target window for a few seconds. Needs `cargo build -p demo-target`.
    /// Run with `cargo test -p scenra -- --ignored`.
    #[test]
    #[ignore = "launches the demo target window"]
    fn demo_target_attaches_over_cdp_as_webview2() {
        let user_data = std::env::temp_dir().join("scenra-test-demo-target");
        let (mut child, port) = spawn_demo_target(&user_data).unwrap();
        let profile = detect(child.id(), port);
        let _ = child.kill();
        let _ = child.wait();

        let cdp = &profile.capabilities.cdp;
        assert!(profile.warnings.is_empty(), "warnings: {:?}", profile.warnings);
        assert!(cdp.available);
        assert_eq!(cdp.flavor, Some("webview2"));
        assert!(cdp.owned_by_target && cdp.loopback_only);
        assert_eq!(profile.application, "Lumina (demo target)");
        assert!(cdp.test_ids.unwrap_or(0) > 0);
        assert_eq!(profile.preferred_locator, Some("cdp"));
    }

    /// B-3: right after launch, preflight waits for the page to paint and settle rather than
    /// letting a take start on a blank window. Run with `cargo test -p scenra -- --ignored`.
    #[test]
    #[ignore = "launches the demo target window"]
    fn preflight_waits_for_a_freshly_launched_page_to_settle() {
        let user_data = std::env::temp_dir().join("scenra-test-demo-target");
        let (mut child, port) = spawn_demo_target(&user_data).unwrap();
        let settled = crate::page::wait_until_settled(port, Duration::from_secs(15));
        let _ = child.kill();
        let _ = child.wait();
        settled.unwrap();
    }

    #[test]
    #[ignore = "launches the demo target window"]
    fn scene_preflight_rejects_overlay_and_recovers_after_reset() {
        let user_data = std::env::temp_dir().join("scenra-test-scene-start");
        let (mut child, port) = spawn_demo_target(&user_data).unwrap();
        let result = (|| -> Result<(), String> {
            crate::page::wait_until_settled(port, Duration::from_secs(15))?;
            let mut page = crate::page::page_session(port)?;
            let baseline = crate::page::initial_state(&mut page)?;
            let outline = crate::page::outline(&mut page)?;
            let label = &outline.controls.first().ok_or("fixture has no controls")?.label;
            crate::page::check_control(&mut page, label)?;
            page.evaluate(r#"(() => {
              const d = document.createElement('dialog'); d.id = 'scene-test';
              d.textContent = 'Settings'; document.body.append(d); d.showModal(); return true;
            })()"#)?;
            assert!(crate::page::check_initial(&baseline, &crate::page::initial_state(&mut page)?).is_err());
            assert!(crate::page::check_control(&mut page, label).is_err());
            page.evaluate("document.querySelector('#scene-test').remove(); true")?;
            crate::page::check_initial(&baseline, &crate::page::initial_state(&mut page)?)?;
            crate::page::check_control(&mut page, label)?;
            page.call("Page.navigate", serde_json::json!({ "url": baseline.url }))?;
            crate::page::wait_until_settled(port, Duration::from_secs(15))?;
            let mut page = crate::page::page_session(port)?;
            crate::page::check_initial(&baseline, &crate::page::initial_state(&mut page)?)?;
            crate::page::check_control(&mut page, label)
        })();
        let _ = child.kill();
        let _ = child.wait();
        result.unwrap();
    }

    #[test]
    #[ignore = "launches and minimises the demo target window"]
    fn locator_restores_minimised_target_before_measuring() {
        use windows::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_MINIMIZE};
        use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        let previous_dpi = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let user_data = std::env::temp_dir().join("scenra-test-minimised-target");
        let (mut child, port) = spawn_demo_target(&user_data).unwrap();
        let result = (|| -> Result<(), String> {
            crate::page::wait_until_settled(port, Duration::from_secs(15))?;
            let mut page = crate::page::page_session(port)?;
            let outline = crate::page::outline(&mut page)?;
            let selector = crate::locate::Selector::Label(outline.controls.first().ok_or("No controls")?.label.clone());
            let (top, _, _) = crate::locate::render_widget(child.id())?;
            unsafe { let _ = ShowWindow(top, SW_MINIMIZE); }
            assert!(unsafe { IsIconic(top) }.as_bool());
            let found = crate::locate::locate(&mut page, child.id(), &selector)?;
            assert!(!unsafe { IsIconic(top) }.as_bool());
            assert!(found.screen.width > 0.0 && found.screen.height > 0.0);
            Ok(())
        })();
        let _ = child.kill(); let _ = child.wait();
        unsafe { SetThreadDpiAwarenessContext(previous_dpi); }
        result.unwrap();
    }

    /// T-7: a locked screen or covered window leaves the page hidden and unpainted, and a document
    /// swap briefly leaves no root element. Preflight must say which, not "still loading".
    #[test]
    #[ignore = "launches the demo target window"]
    fn settle_names_hidden_page_and_tolerates_missing_root() {
        let user_data = std::env::temp_dir().join("scenra-test-hidden-target");
        let (mut child, port) = spawn_demo_target(&user_data).unwrap();
        let result = (|| -> Result<(), String> {
            // Settle first: an override applied during the initial navigation is lost with that document.
            crate::page::wait_until_settled(port, Duration::from_secs(15))?;
            let mut page = crate::page::page_session(port)?;
            page.evaluate("Object.defineProperty(document, 'visibilityState', { get: () => 'hidden' }); true")?;
            // Failures return Err rather than panic, so the demo target is always killed below.
            let hidden = crate::page::wait_until_settled(port, Duration::from_secs(1)).err().unwrap_or_default();
            if !hidden.contains("hidden") { return Err(format!("hidden page not named: {hidden:?}")); }
            page.evaluate("document.documentElement.remove(); true")?;
            let rootless = crate::page::wait_until_settled(port, Duration::from_secs(1)).err().unwrap_or_default();
            if rootless.is_empty() || rootless.contains("evaluate failed") { return Err(format!("rootless page: {rootless:?}")); }
            Ok(())
        })();
        let _ = child.kill(); let _ = child.wait();
        result.unwrap();
    }

    #[test]
    #[ignore = "launches the local fixture and replays approved CDP clicks"]
    fn fast_forward_restores_dependent_scene_and_stops_on_cancel_or_covered_targets() {
        use crate::ai::{Action, Line, Scenario, Scene};
        let user_data = std::env::temp_dir().join(format!("scenra-test-setup-{}", crate::take::stamp()));
        let (mut child, port) = spawn_demo_target(&user_data).unwrap();
        let result = (|| -> Result<(), String> {
            crate::page::wait_until_settled(port, Duration::from_secs(15))?;
            let mut page = crate::page::page_session(port)?;
            let baseline = crate::page::initial_state(&mut page)?;
            let action = |target: &str, word| Action { word, target: target.into(), label: target.into() };
            let scene = |title: &str, actions| Scene { title: title.into(), lines: vec![Line { text: "Open the control".into(), actions }] };
            let mut scenario = Scenario { title: "Setup regression".into(), warnings: vec![], scenes: vec![
                scene("Open recording", vec![action("New Recording", 2)]),
                scene("Ready to start", vec![action("Start", 0)]),
            ] };
            let states = vec![baseline.clone(), baseline.clone()];
            let outcome = crate::setup::replay_to_start(port, &scenario, &states, 1, || Ok(()))?;
            assert_eq!(outcome.actions_run, 1);
            let mut page = crate::page::page_session(port)?;
            assert_eq!(page.evaluate("document.querySelector('[data-testid=recordings]').children.length")?, serde_json::json!(0));
            crate::page::check_control(&mut page, "Start")?;
            page.evaluate("document.addEventListener('click',e=>{window.__setupTrusted=e.isTrusted;},true); true")?;
            crate::page::setup_click(&mut page, "Settings", &|| Ok(()))?;
            assert_eq!(page.evaluate("window.__setupTrusted")?, serde_json::json!(true));
            let settings = crate::page::initial_state(&mut page)?;
            assert!(crate::page::setup_click(&mut page, "New Recording", &|| Ok(())).is_err());
            scenario.scenes[0].lines[0].actions.push(action("Settings", 3));
            scenario.scenes[1].lines[0].actions = vec![action("Confirm", 0)];
            let states = vec![baseline.clone(), settings];
            let outcome = crate::setup::replay_to_start(port, &scenario, &states, 1, || Ok(()))?;
            assert_eq!(outcome.actions_run, 2);
            let mut page = crate::page::page_session(port)?;
            crate::page::check_control(&mut page, "Confirm")?;
            // Reset first, then cancel as soon as the first approved click opens the panel.
            crate::page::restore_url(port, &baseline.url, Duration::from_secs(15), &|| Ok(()))?;
            let cancelled = std::cell::Cell::new(false);
            let checkpoint = || {
                if let Ok(mut page) = crate::page::page_session(port)
                    && let Ok(open) = page.evaluate("!document.querySelector('[data-testid=recording-panel]').hidden")
                    && open == serde_json::json!(true)
                { cancelled.set(true); return Err("Cancelled after first click".into()); }
                Ok(())
            };
            let outcome = crate::setup::replay_to_start(port, &scenario, &states, 1, checkpoint);
            assert!(outcome.err().unwrap().contains("Cancelled after first click"));
            assert!(cancelled.get());
            let mut page = crate::page::page_session(port)?;
            assert_eq!(page.evaluate("document.querySelector('dialog').open")?, serde_json::json!(false));
            // Cancellation after locating but before pressing must not dispatch a click.
            page.evaluate("window.__setupClicks=0;document.addEventListener('click',()=>{window.__setupClicks++;},true); true")?;
            let count = page.evaluate("window.__setupClicks")?;
            assert!(crate::page::setup_click(&mut page, "New Recording", &|| Err("Cancelled before press".into())).is_err());
            assert_eq!(page.evaluate("window.__setupClicks")?, count);
            // Duplicate labels are refused rather than guessing which one the script meant.
            page.evaluate("{const b=document.createElement('button');b.textContent='New Recording';document.body.append(b);} true")?;
            assert!(crate::page::setup_click(&mut page, "New Recording", &|| Ok(())).is_err());
            Ok(())
        })();
        let _ = child.kill(); let _ = child.wait();
        result.unwrap();
    }

    /// Local-only Chrome regression: the first click runs after a delayed page paint/settle.
    #[test]
    #[ignore = "launches Chrome and moves the real pointer"]
    fn chrome_preflight_then_first_click() {
        use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        let server = fixture::serve(Duration::from_millis(700));
        let addr = server.addr;
        let previous_dpi = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let profile = std::env::temp_dir().join(format!("scenra-chrome-preflight-{}", crate::take::stamp()));
        let spawned = spawn_chrome(&format!("http://{addr}/"), &profile);
        let result = match spawned {
            Ok((mut child, port)) => {
                let result = (|| -> Result<(), String> {
                    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_MINIMIZE};
                    crate::page::page_session(port)?.call("Page.bringToFront", serde_json::json!({}))?;
                    // T-7: a minimised Chrome never paints. Preflight names it, and restoring the
                    // window (as scene start, reset and setup do) lets the page settle.
                    let deadline = Instant::now() + Duration::from_secs(15);
                    let top = loop {
                        match crate::locate::render_widget(child.id()) {
                            Ok((top, _, _)) => break top,
                            Err(e) if Instant::now() > deadline => return Err(e),
                            Err(_) => std::thread::sleep(Duration::from_millis(200)),
                        }
                    };
                    unsafe { let _ = ShowWindow(top, SW_MINIMIZE); }
                    let hidden = crate::page::wait_until_settled(port, Duration::from_secs(3)).err().unwrap_or_default();
                    if !hidden.contains("hidden") { return Err(format!("minimised Chrome should be reported hidden, got {hidden:?}")); }
                    crate::locate::restore_target(child.id())?;
                    if unsafe { IsIconic(top) }.as_bool() { return Err("restore_target left Chrome minimised".into()); }
                    crate::page::wait_until_settled(port, Duration::from_secs(15)).map_err(|e| {
                        let state = crate::page::page_session(port).and_then(|mut p| p.evaluate("({ready:document.readyState,visibility:document.visibilityState,fonts:document.fonts.status,paint:performance.getEntriesByType('paint').map(p=>p.name)})"));
                        format!("initial Chrome preflight: {e}; {state:?}")
                    })?;
                    let mut page = crate::page::page_session(port)?;
                    let old_document = page.evaluate("performance.timeOrigin")?;
                    let url = crate::page::initial_state(&mut page)?.url;
                    crate::page::restore_url(port, &url, Duration::from_secs(15), &|| Ok(())).map_err(|e| {
                        let state = crate::page::page_session(port).and_then(|mut p| p.evaluate("({ready:document.readyState,visibility:document.visibilityState,fonts:document.fonts.status,paint:performance.getEntriesByType('paint').map(p=>p.name)})"));
                        format!("Chrome reset: {e}; {state:?}")
                    })?;
                    let mut page = crate::page::page_session(port)?;
                    // Failures return Err rather than panic, so Chrome is always killed below.
                    if page.evaluate("performance.timeOrigin")? == old_document {
                        return Err("restore returned before the delayed navigation created a new document".into());
                    }
                    let selector = crate::locate::Selector::TestId("create-recording".into());
                    let probe = crate::locate::probe(&mut page, child.id(), &selector, crate::input::Pace::Natural).map_err(|e| e.to_string())?;
                    if !probe.passed { return Err("Chrome hover probe missed".into()); }
                    crate::input::click().map_err(|e| e.to_string())?;
                    std::thread::sleep(Duration::from_millis(250));
                    if page.evaluate("!document.querySelector('[data-testid=recording-panel]').hidden")? != serde_json::json!(true) {
                        return Err("the first click didn't open the recording panel".into());
                    }
                    Ok(())
                })();
                let _ = child.kill(); let _ = child.wait(); result
            }
            Err(e) => Err(e),
        };
        unsafe { SetThreadDpiAwarenessContext(previous_dpi); }
        drop(server);
        result.unwrap();
    }

    #[test]
    fn descends_from_walks_parents() {
        let processes = HashMap::from([
            (30, ProcessEntry { parent: 20, exe: "msedgewebview2.exe".into() }),
            (20, ProcessEntry { parent: 10, exe: "demo-target.exe".into() }),
            (10, ProcessEntry { parent: 0, exe: "explorer.exe".into() }),
            (40, ProcessEntry { parent: 40, exe: "loop.exe".into() }),
        ]);
        assert!(descends_from(&processes, 30, 20));
        assert!(descends_from(&processes, 20, 20));
        assert!(!descends_from(&processes, 20, 30));
        assert!(!descends_from(&processes, 40, 20));
    }
}
