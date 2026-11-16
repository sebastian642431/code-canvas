//! What survives between launches: opened folders, cameras, collapsed and
//! hidden paths. Stored as JSON in the user's config directory.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Session {
    pub pages: Vec<PageSession>,
    pub active: usize,
    pub left_open: bool,
    pub right_open: bool,
    /// Which panel each side shows (index into the rail).
    pub left_panel: u8,
    pub right_panel: u8,
    /// Logical window size.
    pub window: Option<[f32; 2]>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct PageSession {
    pub path: PathBuf,
    /// Camera centre and zoom (zoom in logical pixels per world unit).
    pub camera: Option<([f32; 2], f32)>,
    pub collapsed: Vec<String>,
    pub hidden: Vec<String>,
    pub tree_open: Vec<String>,
}

/// `%APPDATA%\code-canvas` on Windows, `~/.config/code-canvas` on Linux,
/// `~/Library/Application Support/code-canvas` on macOS.
///
/// `CODE_CANVAS_DATA_DIR` overrides it (tests use this to leave the real
/// session alone).
pub fn data_dir() -> PathBuf {
    let dir = match std::env::var_os("CODE_CANVAS_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("code-canvas"),
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn session_path() -> PathBuf {
    data_dir().join("session.json")
}

pub fn load() -> Session {
    let fresh = Session { left_open: true, right_open: true, ..Default::default() };
    match std::fs::read_to_string(session_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            log::warn!("ignoring unreadable session file: {e}");
            fresh
        }),
        Err(_) => fresh,
    }
}

pub fn save(session: &Session) {
    let path = session_path();
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_string_pretty(session)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(&tmp, json))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        log::warn!("could not save session to {}: {e}", path.display());
    }
}
