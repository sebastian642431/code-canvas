//! Preferences that are neither looks nor keys: `prefs.json` in the data
//! directory. The settings panel edits them.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Prefs {
    /// Ask npm and crates.io for the latest version of each dependency and
    /// show it next to the line (`versions.rs`). Only package names are sent.
    pub check_versions: bool,
    /// Offer file and folder names while a path is typed in a string
    /// (`paths.rs`).
    pub complete_paths: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { check_versions: true, complete_paths: true }
    }
}

fn path() -> std::path::PathBuf {
    crate::session::data_dir().join("prefs.json")
}

pub fn load() -> Prefs {
    std::fs::read_to_string(path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(prefs: &Prefs) {
    let result = serde_json::to_string_pretty(prefs)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(path(), json));
    if let Err(e) = result {
        log::warn!("could not save the preferences: {e}");
    }
}
