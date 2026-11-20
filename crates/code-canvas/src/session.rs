use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Session {
    pub pages: Vec<PageSession>,
    pub active: usize,
    pub left_open: bool,
    pub dock: Option<String>,
    pub beside: Option<String>,
    pub cards: Vec<String>,
    pub rails: crate::rails::Rails,
    pub window: Option<[f32; 2]>,
    pub window_pos: Option<[i32; 2]>,
    pub maximized: bool,
    pub single: bool,
    pub workspace: Vec<Known>,
    pub consoles: crate::consoles::Saved,
    /// The files open in tabs above the canvas, and which of them is in front (none: the canvas).
    pub tabs: Vec<PathBuf>,
    pub tab: Option<usize>,
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
    pub arranged: bool,
}

pub fn data_dir() -> PathBuf {
    let dir = match std::env::var_os("CODE_CANVAS_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => named_data_dir(&dirs::config_dir().unwrap_or_else(std::env::temp_dir)),
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The folder of the app's data under `base`: `hawk-canvas`. The first time, the folder of the
/// name before it (`code-canvas`) is renamed to it, so nothing is lost; while that cannot be done
/// (a file in it still open), the old folder keeps being used.
pub fn named_data_dir(base: &Path) -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| take_over_data_dir(base)).clone()
}

fn take_over_data_dir(base: &Path) -> PathBuf {
    let (new, old) = (base.join("hawk-canvas"), base.join("code-canvas"));
    if new.exists() || !old.is_dir() {
        return new;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => new,
        Err(_) => old,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_of_code_canvas_becomes_the_data_of_hawk_canvas() {
        let base = std::env::temp_dir().join(format!("hawk-canvas-test-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("code-canvas")).unwrap();
        std::fs::write(base.join("code-canvas").join("session.json"), "{}").unwrap();
        let dir = take_over_data_dir(&base);
        assert_eq!(dir, base.join("hawk-canvas"));
        assert_eq!(std::fs::read_to_string(dir.join("session.json")).unwrap(), "{}", "nothing is lost");
        assert!(!base.join("code-canvas").exists());

        std::fs::create_dir_all(base.join("code-canvas")).unwrap();
        assert_eq!(take_over_data_dir(&base), base.join("hawk-canvas"), "once there, the new folder stays");
        assert!(base.join("code-canvas").exists(), "an old folder made later is left alone");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(take_over_data_dir(&base), base.join("hawk-canvas"), "a first start of all");
    }

    #[test]
    fn file_tabs_survive_a_round_trip_and_old_sessions_have_none() {
        let session = Session {
            tabs: vec![PathBuf::from("/r/src/a.rs"), PathBuf::from("/r/b.ts")],
            tab: Some(1),
            ..Default::default()
        };
        let back: Session = serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!((back.tabs, back.tab), (session.tabs, Some(1)));
        let old: Session = serde_json::from_str(r#"{"pages": [], "active": 0}"#).unwrap();
        assert!(old.tabs.is_empty() && old.tab.is_none());
    }
}
