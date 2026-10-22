//! API keys live in Windows Credential Manager (PROJECT_OUTLINE §4, SecretStore), never in the
//! repo, config files or logs. Store one with:
//!
//! ```text
//! cmdkey /generic:Scenra/gemini /user:gemini /pass:<key>
//! ```
//!
//! `SCENRA_GEMINI_API_KEY` is a fallback for development shells.

use windows::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW};
use windows::core::HSTRING;

pub const GEMINI_TARGET: &str = "Scenra/gemini";
pub const OBS_TARGET: &str = "Scenra/obs";
pub const GEMINI_ENV: &str = "SCENRA_GEMINI_API_KEY";

/// Where a key was found, for status display. Never carries the key itself.
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeySource {
    CredentialManager,
    Environment,
}

pub fn gemini_key() -> Option<(String, KeySource)> {
    if let Some(key) = read_generic(GEMINI_TARGET) {
        return Some((key, KeySource::CredentialManager));
    }
    std::env::var(GEMINI_ENV)
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(|k| (k.trim().to_owned(), KeySource::Environment))
}

/// The obs-websocket password, if OBS has authentication on.
pub fn obs_password() -> Option<String> {
    read_generic(OBS_TARGET)
}

/// Reads a generic credential's secret. `cmdkey` stores it as UTF-16.
fn read_generic(target: &str) -> Option<String> {
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe { CredReadW(&HSTRING::from(target), CRED_TYPE_GENERIC, None, &mut credential) }.ok()?;
    let secret = unsafe {
        let c = &*credential;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        String::from_utf16(&units).ok()
    };
    unsafe { CredFree(credential.cast()) };
    secret.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}
