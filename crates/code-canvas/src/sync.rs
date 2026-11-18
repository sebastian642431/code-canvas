use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::github::{Files, MARK};

pub const KEPT: [&str; 4] = ["prefs.json", "theme.json", "keybindings.json", "commit.json"];
pub const EXTENSIONS: &str = "extensions.json";

fn data(name: &str) -> PathBuf {
    crate::session::data_dir().join(name)
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct State {
    pub user: String,
    pub gist: String,
    pub synced: String,
}

pub fn load_state() -> State {
    std::fs::read_to_string(data("github.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_state(state: &State) {
    if let Err(why) = std::fs::write(data("github.json"), serde_json::to_string_pretty(state).unwrap_or_default()) {
        log::warn!("the state of the settings sync could not be kept: {why}");
    }
}

pub fn forget() {
    let _ = std::fs::remove_file(data("github.json"));
    let _ = std::fs::remove_file(data("github.token"));
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Extensions {
    pub installed: Vec<String>,
    pub disabled: Vec<String>,
}

pub fn snapshot(mut installed: Vec<String>, mut disabled: Vec<String>) -> Files {
    let mut files = Files::new();
    for name in KEPT {
        if let Ok(text) = std::fs::read_to_string(data(name)) {
            files.insert(name.to_string(), text);
        }
    }
    installed.sort();
    disabled.sort();
    let extensions = Extensions { installed, disabled };
    files.insert(EXTENSIONS.to_string(), serde_json::to_string_pretty(&extensions).unwrap_or_default());
    files.insert(MARK.to_string(), json!({ "app": "code-canvas", "format": 1 }).to_string());
    files
}

pub fn digest(files: &Files) -> String {
    let mut hasher = DefaultHasher::new();
    for (name, text) in files.iter().filter(|(name, _)| *name != MARK) {
        name.hash(&mut hasher);
        let plain: Value = serde_json::from_str(text).unwrap_or(Value::String(text.clone()));
        plain.to_string().hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Same,
    Send,
    Take,
    Ask,
}

pub fn plan(here: &Files, there: Option<&Files>, last: &str) -> Plan {
    let Some(there) = there else { return Plan::Send };
    let untouched = !here.keys().any(|name| KEPT.contains(&name.as_str()));
    let (here, there) = (digest(here), digest(there));
    match () {
        _ if here == there => Plan::Same,
        _ if last.is_empty() && untouched => Plan::Take,
        _ if last.is_empty() => Plan::Ask,
        _ if here == last => Plan::Take,
        _ if there == last => Plan::Send,
        _ => Plan::Ask,
    }
}

pub fn write_down(files: &Files) -> Extensions {
    for name in KEPT {
        if let Some(text) = files.get(name).filter(|text| serde_json::from_str::<Value>(text).is_ok()) {
            if let Err(why) = std::fs::write(data(name), text) {
                log::warn!("{name} from GitHub could not be written: {why}");
            }
        }
    }
    files.get(EXTENSIONS).and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default()
}

pub fn keep_token(token: &str) {
    let kept = protect(token.as_bytes()).and_then(|sealed| std::fs::write(data("github.token"), sealed).ok());
    if kept.is_none() {
        log::warn!("the GitHub sign-in could not be kept on this computer");
    }
}

pub fn token() -> Option<String> {
    let sealed = std::fs::read(data("github.token")).ok()?;
    String::from_utf8(unprotect(&sealed)?).ok().filter(|token| !token.is_empty())
}

#[cfg(windows)]
mod sealed {
    use std::ffi::c_void;

    #[repr(C)]
    struct Blob {
        len: u32,
        data: *mut u8,
    }

    const NO_UI: u32 = 0x1;

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            about: *const u16,
            entropy: *const Blob,
            reserved: *const c_void,
            prompt: *const c_void,
            flags: u32,
            out: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            about: *mut *mut u16,
            entropy: *const Blob,
            reserved: *const c_void,
            prompt: *const c_void,
            flags: u32,
            out: *mut Blob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    fn run(bytes: &[u8], seal: bool) -> Option<Vec<u8>> {
        let input = Blob { len: bytes.len() as u32, data: bytes.as_ptr() as *mut u8 };
        let mut out = Blob { len: 0, data: std::ptr::null_mut() };
        let (none, blob) = (std::ptr::null::<c_void>(), std::ptr::null::<Blob>());
        let done = unsafe {
            match seal {
                true => CryptProtectData(&input, std::ptr::null(), blob, none, none, NO_UI, &mut out),
                false => CryptUnprotectData(&input, std::ptr::null_mut(), blob, none, none, NO_UI, &mut out),
            }
        };
        if done == 0 || out.data.is_null() {
            return None;
        }
        let result = unsafe { std::slice::from_raw_parts(out.data, out.len as usize).to_vec() };
        unsafe { LocalFree(out.data as *mut c_void) };
        Some(result)
    }

    pub fn protect(bytes: &[u8]) -> Option<Vec<u8>> {
        run(bytes, true)
    }

    pub fn unprotect(bytes: &[u8]) -> Option<Vec<u8>> {
        run(bytes, false)
    }
}

#[cfg(not(windows))]
mod sealed {
    pub fn protect(bytes: &[u8]) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }

    pub fn unprotect(bytes: &[u8]) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }
}

use sealed::{protect, unprotect};

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> Files {
        pairs.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect()
    }

    #[test]
    fn the_newer_side_wins_and_two_changes_are_asked_about() {
        let old = files(&[("prefs.json", "{\"edit\": false}")]);
        let new = files(&[("prefs.json", "{\"edit\": true}")]);
        let last = digest(&old);
        assert_eq!(plan(&old, None, &last), Plan::Send, "nothing on GitHub yet");
        assert_eq!(plan(&old, Some(&old), &last), Plan::Same);
        assert_eq!(plan(&old, Some(&new), &last), Plan::Take, "only GitHub changed");
        assert_eq!(plan(&new, Some(&old), &last), Plan::Send, "only this computer changed");
        let other = files(&[("prefs.json", "{\"edit\": true, \"timeline\": false}")]);
        assert_eq!(plan(&new, Some(&other), &last), Plan::Ask, "both changed");
        assert_eq!(plan(&new, Some(&other), ""), Plan::Ask, "a first sign-in with settings on both sides");
        let fresh = files(&[(EXTENSIONS, "{}")]);
        assert_eq!(plan(&fresh, Some(&other), ""), Plan::Take, "a computer never set up takes GitHub's");
    }

    #[test]
    fn spacing_and_the_mark_do_not_count_as_changes() {
        let a = files(&[("prefs.json", "{\"edit\": true}"), (MARK, "{}")]);
        let b = files(&[("prefs.json", "{\n  \"edit\": true\n}")]);
        assert_eq!(digest(&a), digest(&b));
    }

    #[test]
    fn a_sign_in_is_sealed_for_this_user_only() {
        let sealed = protect(b"gho_example").unwrap();
        assert_eq!(unprotect(&sealed).as_deref(), Some(&b"gho_example"[..]));
        if cfg!(windows) {
            assert_ne!(sealed, b"gho_example".to_vec(), "never kept as it is");
        }
    }
}
