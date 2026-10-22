//! EmergencyStop and InputGuard (PROJECT_OUTLINE §4), each on its own thread with its own
//! message loop, so they keep working when the UI thread hangs.
//!
//! - EmergencyStop is a registered hotkey, Ctrl+Alt+Esc, and latches a stop flag that the
//!   InputEngine checks between every step (every 8 ms while moving or typing). A hotkey rather
//!   than a hook: while a Scenra window has focus, Scenra's own low-level keyboard hooks don't see
//!   its key events (found in Phase 0 testing), but `WM_HOTKEY` still arrives. If another app
//!   already owns the combo, a low-level hook is the fallback.
//! - InputGuard tags-checks every mouse and keyboard event: Scenra's own events carry
//!   [`crate::input::TAG`] in `dwExtraInfo` (expected); anything else is the user (foreign).
//!   While Scenra is driving input, foreign input interrupts the run.
//!   Windows doesn't call Scenra's low-level keyboard hook for keys headed to Scenra's own windows
//!   (found in Phase 0 testing: not even the hook callback runs). So every Scenra window also
//!   reports its own key presses through [`own_window_key`]. Scenra only ever sends input to the
//!   target, so a key arriving in a Scenra window is the user's.

use crate::input::TAG;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_UP, MSG,
    MSLLHOOKSTRUCT,
    SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOWS_HOOK_ID, WM_HOTKEY,
};

static STOPPED: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static FOREIGN: AtomicBool = AtomicBool::new(false);
static EXPECTED_EVENTS: AtomicU64 = AtomicU64::new(0);
static FOREIGN_EVENTS: AtomicU64 = AtomicU64::new(0);
/// Every key event the guard hook was called for, before any filtering (diagnostics).
static KEY_EVENTS: AtomicU64 = AtomicU64::new(0);
/// Called (on its own short-lived thread) when the emergency stop fires.
static ON_STOP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Installs both hooks, each on its own thread. Safe to call more than once.
pub fn start(on_stop: impl Fn() + Send + Sync + 'static) -> Result<(), String> {
    static STARTED: OnceLock<Result<(), String>> = OnceLock::new();
    let _ = ON_STOP.set(Box::new(on_stop));
    STARTED
        .get_or_init(|| {
            if spawn_hotkey().is_err() {
                spawn_hook("scenra-emergency-stop", WH_KEYBOARD_LL, stop_proc)?;
            }
            spawn_hook("scenra-input-guard-mouse", WH_MOUSE_LL, guard_mouse_proc)?;
            spawn_hook("scenra-input-guard-keys", WH_KEYBOARD_LL, guard_key_proc)
        })
        .clone()
}

const MODIFIERS: [u16; 11] = [
    VK_CONTROL.0, VK_LCONTROL.0, VK_RCONTROL.0, VK_MENU.0, VK_LMENU.0, VK_RMENU.0,
    VK_SHIFT.0, VK_LSHIFT.0, VK_RSHIFT.0, VK_LWIN.0, VK_RWIN.0,
];

/// Registers Ctrl+Alt+Esc on its own thread and waits for it.
fn spawn_hotkey() -> Result<(), String> {
    let (ready, registered) = mpsc::channel();
    std::thread::Builder::new()
        .name("scenra-emergency-stop".into())
        .spawn(move || unsafe {
            if let Err(e) = RegisterHotKey(None, 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_ESCAPE.0 as u32) {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
            let _ = ready.send(Ok(()));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY {
                    trigger_stop();
                }
            }
        })
        .map_err(|e| e.to_string())?;
    registered.recv().map_err(|e| e.to_string())?
}

fn trigger_stop() {
    if !STOPPED.swap(true, Ordering::SeqCst) {
        // Off the hook/hotkey thread: the callback sends input (release_all), and input sent
        // from inside a low-level hook stalls the hook chain.
        std::thread::spawn(|| {
            if let Some(on_stop) = ON_STOP.get() {
                on_stop();
            }
        });
    }
}

type HookProc = unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT;

fn spawn_hook(name: &str, id: WINDOWS_HOOK_ID, proc: HookProc) -> Result<(), String> {
    let (ready, installed) = mpsc::channel();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || unsafe {
            let module = GetModuleHandleW(None).ok().map(Into::into);
            match SetWindowsHookExW(id, Some(proc), module, 0) {
                Ok(_) => {
                    let _ = ready.send(Ok(()));
                    // LL hooks are called on the installing thread's message loop.
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    installed.recv().map_err(|e| e.to_string())?
}

// The user's Ctrl and Alt, tracked from the hook's own events. `GetAsyncKeyState` isn't
// reliable inside a low-level hook: the async state can lag the events the hook is seeing.
static CTRL_DOWN: AtomicBool = AtomicBool::new(false);
static ALT_DOWN: AtomicBool = AtomicBool::new(false);

/// Updates the user's modifier state from a key event and says whether it completes
/// Ctrl+Alt+Esc. Scenra's own (tagged) events are ignored.
fn track(key: &KBDLLHOOKSTRUCT) -> bool {
    if key.dwExtraInfo == TAG {
        return false;
    }
    let pressed = key.flags.0 & LLKHF_UP.0 == 0;
    let vk = key.vkCode as u16;
    if [VK_CONTROL.0, VK_LCONTROL.0, VK_RCONTROL.0].contains(&vk) {
        CTRL_DOWN.store(pressed, Ordering::SeqCst);
    } else if [VK_MENU.0, VK_LMENU.0, VK_RMENU.0].contains(&vk) {
        ALT_DOWN.store(pressed, Ordering::SeqCst);
    }
    vk == VK_ESCAPE.0 && CTRL_DOWN.load(Ordering::SeqCst) && ALT_DOWN.load(Ordering::SeqCst)
}

unsafe extern "system" fn stop_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let pressed = key.flags.0 & LLKHF_UP.0 == 0;
        if track(key) && pressed {
            trigger_stop();
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn classify(extra: usize) {
    if extra == TAG {
        EXPECTED_EVENTS.fetch_add(1, Ordering::Relaxed);
    } else if ARMED.load(Ordering::Relaxed) {
        FOREIGN_EVENTS.fetch_add(1, Ordering::Relaxed);
        FOREIGN.store(true, Ordering::SeqCst);
    }
}

unsafe extern "system" fn guard_mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // Moves, buttons and the wheel all count: any of them means the user has the mouse.
        let mouse = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        classify(mouse.dwExtraInfo);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn guard_key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        KEY_EVENTS.fetch_add(1, Ordering::Relaxed);
        // Modifiers alone aren't interference, and Esc belongs to the emergency stop: counting it
        // here would race the stop hotkey and report the wrong reason.
        let vk = key.vkCode as u16;
        let modifier = MODIFIERS.contains(&vk);
        track(key);
        if !modifier && vk != VK_ESCAPE.0 {
            classify(key.dwExtraInfo);
        } else if key.dwExtraInfo == TAG {
            EXPECTED_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// A key press seen by one of Scenra's own windows, reported by its webview: the keyboard hook is
/// blind to these. Counted like any untagged event, so it only interrupts while armed.
pub fn own_window_key() {
    classify(0);
}

pub fn stopped() -> bool {
    STOPPED.load(Ordering::SeqCst)
}

/// Clears the emergency stop, e.g. when the presenter resumes.
pub fn clear_stop() {
    STOPPED.store(false, Ordering::SeqCst);
}

pub fn foreign_seen() -> bool {
    FOREIGN.load(Ordering::SeqCst)
}

/// Marks a span of Scenra-driven input. Foreign input inside it interrupts the run.
pub struct Armed(());

pub fn arm() -> Armed {
    FOREIGN.store(false, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    Armed(())
}

impl Drop for Armed {
    fn drop(&mut self) {
        ARMED.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardCounts {
    pub expected: u64,
    pub foreign: u64,
    pub key_events: u64,
    pub stopped: bool,
}

pub fn counts() -> GuardCounts {
    GuardCounts {
        expected: EXPECTED_EVENTS.load(Ordering::Relaxed),
        foreign: FOREIGN_EVENTS.load(Ordering::Relaxed),
        key_events: KEY_EVENTS.load(Ordering::Relaxed),
        stopped: stopped(),
    }
}
