//! Visual Studio's automation object (DTE), driven by late-bound COM (docs/adr/0004).
//!
//! A running `devenv` registers its DTE in the Running Object Table as
//! `!VisualStudio.DTE.<version>:<pid>`. Calls go through `IDispatch` by member name, so no type
//! library is needed. Visual Studio rejects calls while it's busy (a modal dialog, a build
//! starting); those are retried for a short while.

use std::time::{Duration, Instant};
use windows::Win32::Foundation::{RPC_E_CALL_REJECTED, RPC_E_SERVERCALL_RETRYLATER};
use windows::Win32::System::Com::{
    CreateBindCtx, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPPARAMS, EXCEPINFO, GetRunningObjectTable, IDispatch,
    IMoniker,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, GUID, Interface, PCWSTR, HSTRING};

/// How long to keep retrying a call Visual Studio rejected because it was busy.
const BUSY_RETRY: Duration = Duration::from_secs(5);

/// A COM object driven by member name.
#[derive(Clone)]
pub struct Dispatch(IDispatch);

fn err(e: windows::core::Error) -> String {
    e.message().to_string()
}

impl Dispatch {
    fn id(&self, name: &str) -> Result<i32, String> {
        let wide = HSTRING::from(name);
        let names = [PCWSTR(wide.as_ptr())];
        let mut id = 0i32;
        unsafe { self.0.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0x0400, &mut id) }
            .map_err(|e| format!("{name}: {}", err(e)))?;
        Ok(id)
    }

    fn invoke(&self, name: &str, flags: DISPATCH_FLAGS, args: &[VARIANT]) -> Result<VARIANT, String> {
        let id = self.id(name)?;
        // IDispatch takes arguments last to first.
        let mut reversed: Vec<VARIANT> = args.iter().rev().cloned().collect();
        let params = DISPPARAMS {
            rgvarg: if reversed.is_empty() { std::ptr::null_mut() } else { reversed.as_mut_ptr() },
            rgdispidNamedArgs: std::ptr::null_mut(),
            cArgs: reversed.len() as u32,
            cNamedArgs: 0,
        };
        let deadline = Instant::now() + BUSY_RETRY;
        loop {
            let mut result = VARIANT::default();
            let mut exception = EXCEPINFO::default();
            let called = unsafe {
                self.0.Invoke(id, &GUID::zeroed(), 0x0400, flags, &params, Some(&mut result), Some(&mut exception), None)
            };
            match called {
                Ok(()) => return Ok(result),
                Err(e) if (e.code() == RPC_E_CALL_REJECTED || e.code() == RPC_E_SERVERCALL_RETRYLATER) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    let detail = exception.bstrDescription.to_string();
                    return Err(format!("{name}: {} ({:#010x}, scode {:#010x})", if detail.is_empty() { err(e.clone()) } else { detail }, e.code().0, exception.scode));
                }
            }
        }
    }

    /// Reads a property (or calls a method with no arguments).
    pub fn get(&self, name: &str) -> Result<VARIANT, String> {
        self.invoke(name, DISPATCH_PROPERTYGET | DISPATCH_METHOD, &[])
    }

    /// Calls a method, or reads an indexed property.
    pub fn call(&self, name: &str, args: &[VARIANT]) -> Result<VARIANT, String> {
        self.invoke(name, DISPATCH_METHOD | DISPATCH_PROPERTYGET, args)
    }

    /// A property or method result that is itself an object.
    pub fn object(&self, name: &str, args: &[VARIANT]) -> Result<Dispatch, String> {
        self.unknown(name, args)?.cast::<IDispatch>().map(Dispatch).map_err(|_| format!("{name} can't be automated"))
    }

    /// A property or method result as a plain COM object (some, like event sources, aren't
    /// `IDispatch`).
    pub fn unknown(&self, name: &str, args: &[VARIANT]) -> Result<IUnknown, String> {
        let value = self.call(name, args)?;
        IDispatch::try_from(&value)
            .map(|d| d.cast::<IUnknown>())
            .unwrap_or_else(|_| IUnknown::try_from(&value))
            .map_err(|_| format!("{name} is empty"))
    }

    pub fn string(&self, name: &str) -> Result<String, String> {
        BSTR::try_from(&self.get(name)?).map(|s| s.to_string()).map_err(|e| format!("{name}: {}", err(e)))
    }

    pub fn int(&self, name: &str) -> Result<i32, String> {
        i32::try_from(&self.get(name)?).map_err(|e| format!("{name}: {}", err(e)))
    }
}

/// The DTE of the Visual Studio process `pid`, once it has registered (it takes a few seconds
/// after launch).
pub fn find(pid: u32) -> Result<Dispatch, String> {
    let suffix = format!(":{pid}");
    unsafe {
        let table = GetRunningObjectTable(0).map_err(err)?;
        let running = table.EnumRunning().map_err(err)?;
        loop {
            let mut moniker: [Option<IMoniker>; 1] = [None];
            if running.Next(&mut moniker, None).is_err() {
                break;
            }
            let Some(moniker) = moniker[0].take() else { break };
            let context = CreateBindCtx(0).map_err(err)?;
            let Ok(name) = moniker.GetDisplayName(&context, None) else { continue };
            let display = name.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(name.0 as *const _));
            if display.starts_with("!VisualStudio.DTE.") && display.ends_with(&suffix) {
                let object = table.GetObject(&moniker).map_err(err)?;
                return object.cast::<IDispatch>().map(Dispatch).map_err(err);
            }
        }
    }
    Err(format!("Visual Studio (process {pid}) isn't ready for automation yet"))
}

/// Waits up to `timeout` for Visual Studio `pid` to register its DTE.
pub fn wait_for(pid: u32, timeout: Duration) -> Result<Dispatch, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match find(pid) {
            Ok(dte) => return Ok(dte),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

// ---- Command events ---------------------------------------------------------------------------

use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::System::Com::{IConnectionPoint, IConnectionPointContainer, IDispatch_Impl, IDispatch_Vtbl, ITypeInfo};
use windows::core::{IUnknown, implement, interface};

/// EnvDTE's `_dispCommandEvents` (IID from `Microsoft.VisualStudio.Interop.dll`; not the
/// `CommandEventsClass` ID). Visual Studio calls a subscriber through `IDispatch` only, so the
/// interface adds no methods of its own.
#[interface("FF2D5C12-FEEA-11D0-BBC8-00A0C90F2744")]
unsafe trait DispCommandEvents: IDispatch {}

/// `_dispCommandEvents.AfterExecute(Guid, ID, CustomIn, CustomOut)`.
const AFTER_EXECUTE: i32 = 2;

/// A command Visual Studio ran: its group GUID and ID, and when (Unix epoch ms).
#[derive(Clone, Debug, PartialEq)]
pub struct RanCommand {
    pub guid: String,
    pub id: i32,
    pub at: f64,
}

fn epoch_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64() * 1000.0
}

#[implement(DispCommandEvents)]
struct CommandSink {
    sent: Sender<RanCommand>,
}

impl DispCommandEvents_Impl for CommandSink_Impl {}

impl IDispatch_Impl for CommandSink_Impl {
    fn GetTypeInfoCount(&self) -> windows::core::Result<u32> {
        Ok(0)
    }

    fn GetTypeInfo(&self, _: u32, _: u32) -> windows::core::Result<ITypeInfo> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }

    fn GetIDsOfNames(&self, _: *const GUID, _: *const PCWSTR, _: u32, _: u32, _: *mut i32) -> windows::core::Result<()> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }

    fn Invoke(
        &self, member: i32, _: *const GUID, _: u32, _: DISPATCH_FLAGS, params: *const DISPPARAMS, _: *mut VARIANT,
        _: *mut EXCEPINFO, _: *mut u32,
    ) -> windows::core::Result<()> {
        // Only note the command here: Visual Studio is waiting on this call, so calling back into
        // it (to look up the command's name) could deadlock.
        if member == AFTER_EXECUTE && !params.is_null() {
            let params = unsafe { &*params };
            if params.cArgs >= 2 && !params.rgvarg.is_null() {
                // Arguments arrive last to first: Guid is the first argument, so the last slot.
                let args = unsafe { std::slice::from_raw_parts(params.rgvarg, params.cArgs as usize) };
                let n = args.len();
                let guid = BSTR::try_from(&args[n - 1]).map(|s| s.to_string()).unwrap_or_default();
                let id = i32::try_from(&args[n - 2]).unwrap_or_default();
                let _ = self.sent.send(RanCommand { guid, id, at: epoch_ms() });
            }
        }
        Ok(())
    }
}

/// A subscription to Visual Studio's commands; unsubscribes when dropped.
pub struct CommandWatch {
    point: IConnectionPoint,
    cookie: u32,
    /// Visual Studio stops raising events once its events object is released.
    _events: IUnknown,
    pub ran: Receiver<RanCommand>,
}

impl CommandWatch {
    /// Subscribes to every command `dte` runs, whether from a menu, a shortcut or automation.
    pub fn start(dte: &Dispatch) -> Result<Self, String> {
        // The all-zero GUID and ID 0 mean every command.
        let events = dte.object("Events", &[])?.unknown("CommandEvents", &[VARIANT::from("{00000000-0000-0000-0000-000000000000}"), VARIANT::from(0)])?;
        let (sent, ran) = channel();
        let sink: IUnknown = CommandSink { sent }.into();
        unsafe {
            let container = events.cast::<IConnectionPointContainer>().map_err(err)?;
            let point = container.FindConnectionPoint(&DispCommandEvents::IID).map_err(err)?;
            let cookie = point.Advise(&sink).map_err(|e| format!("couldn't subscribe to Visual Studio's commands: {}", err(e)))?;
            Ok(Self { point, cookie, _events: events, ran })
        }
    }
}

impl Drop for CommandWatch {
    fn drop(&mut self) {
        let _ = unsafe { self.point.Unadvise(self.cookie) };
    }
}

/// A command's name ("Edit.ToggleWordWrap"), or `None` for unnamed internal commands.
pub fn command_name(dte: &Dispatch, ran: &RanCommand) -> Option<String> {
    let command = dte.object("Commands", &[]).ok()?.object("Item", &[VARIANT::from(ran.guid.as_str()), VARIANT::from(ran.id)]).ok()?;
    command.string("Name").ok().filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live: needs a Visual Studio running on this repo (as the ADR 0004 probe left it).
    #[test]
    #[ignore = "needs a running Visual Studio"]
    fn drives_a_running_visual_studio() {
        unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED).ok().unwrap() };
        let pid = crate::target::process_ids("devenv.exe").into_iter().next().expect("start Visual Studio first");
        let dte = find(pid).unwrap();
        eprintln!("{} {}", dte.string("Name").unwrap(), dte.string("Version").unwrap());
        let file = concat!(env!("CARGO_MANIFEST_DIR"), r"\src\take.rs");
        dte.object("ItemOperations", &[]).unwrap().call("OpenFile", &[VARIANT::from(file)]).unwrap();
        let document = dte.object("ActiveDocument", &[]).unwrap();
        assert!(document.string("FullName").unwrap().eq_ignore_ascii_case(file));
        let selection = document.object("Selection", &[]).unwrap();
        selection.call("GotoLine", &[VARIANT::from(10), VARIANT::from(true)]).unwrap();
        selection.call("LineDown", &[VARIANT::from(true), VARIANT::from(2)]).unwrap();
        assert_eq!((selection.int("TopLine").unwrap(), selection.int("BottomLine").unwrap()), (10, 12));
        dte.call("ExecuteCommand", &[VARIANT::from("Edit.ToggleWordWrap")]).unwrap();
        dte.call("ExecuteCommand", &[VARIANT::from("Edit.ToggleWordWrap")]).unwrap();
        assert!(dte.call("ExecuteCommand", &[VARIANT::from("No.Such.Command")]).is_err());
    }

    /// Live: needs a running Visual Studio. Commands run through automation raise the same
    /// events as ones from menus and shortcuts.
    #[test]
    #[ignore = "needs a running Visual Studio"]
    fn hears_the_commands_visual_studio_runs() {
        unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED).ok().unwrap() };
        let pid = crate::target::process_ids("devenv.exe").into_iter().next().expect("start Visual Studio first");
        let dte = find(pid).unwrap();
        // Word wrap needs an open text editor.
        let file = concat!(env!("CARGO_MANIFEST_DIR"), r"\src\take.rs");
        dte.object("ItemOperations", &[]).unwrap().call("OpenFile", &[VARIANT::from(file)]).unwrap();
        let watch = CommandWatch::start(&dte).unwrap();
        dte.call("ExecuteCommand", &[VARIANT::from("Edit.ToggleWordWrap")]).unwrap();
        dte.call("ExecuteCommand", &[VARIANT::from("Edit.ToggleWordWrap")]).unwrap();
        let heard: Vec<String> = std::iter::from_fn(|| watch.ran.recv_timeout(Duration::from_secs(3)).ok())
            .filter_map(|ran| command_name(&dte, &ran))
            .collect();
        eprintln!("heard: {heard:?}");
        assert_eq!(heard.iter().filter(|n| *n == "Edit.ToggleWordWrap").count(), 2, "{heard:?}");
        drop(watch);
        dte.call("ExecuteCommand", &[VARIANT::from("Edit.ToggleWordWrap")]).unwrap();
        dte.call("ExecuteCommand", &[VARIANT::from("Edit.ToggleWordWrap")]).unwrap();
    }
}
