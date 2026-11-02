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
                    return Err(format!("{name}: {}", if detail.is_empty() { err(e) } else { detail }));
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
        let value = self.call(name, args)?;
        IDispatch::try_from(&value).map(Dispatch).map_err(|_| format!("{name} is empty"))
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
}
