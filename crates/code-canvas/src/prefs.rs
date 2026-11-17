use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Prefs {
    pub check_versions: bool,
    pub complete_paths: bool,
    pub check_updates: bool,
    pub follow_chat: bool,
    pub follow_changes: bool,
    pub edit: bool,
    pub unwanted: Vec<String>,
    pub line_numbers: bool,
    pub skip_dirs: Option<Vec<String>>,
    pub skip_files: Option<Vec<String>>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            check_versions: true,
            complete_paths: true,
            check_updates: true,
            follow_chat: true,
            follow_changes: true,
            edit: false,
            unwanted: Vec::new(),
            line_numbers: true,
            skip_dirs: None,
            skip_files: None,
        }
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
