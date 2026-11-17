use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Session {
    pub pages: Vec<PageSession>,
    pub active: usize,
    pub left_open: bool,
    pub cards: Vec<String>,
    pub rails: crate::rails::Rails,
    pub window: Option<[f32; 2]>,
    pub window_pos: Option<[i32; 2]>,
    pub maximized: bool,
    pub single: bool,
    pub workspace: Vec<Known>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Known {
    pub path: PathBuf,
    pub files: Option<u32>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct PageSession {
    pub path: PathBuf,
    pub camera: Option<([f32; 2], f32)>,
    pub collapsed: Vec<String>,
    pub hidden: Vec<String>,
    pub tree_open: Vec<String>,
}

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
    let fresh = Session { left_open: true, ..Default::default() };
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
