use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Prefs {
    pub check_versions: bool,
    pub version_prereleases: bool,
    pub complete_paths: bool,
    pub complete_code: bool,
    pub check_updates: bool,
    pub follow_chat: bool,
    pub follow_changes: bool,
    pub edit: bool,
    pub unwanted: Vec<String>,
    pub line_numbers: bool,
    pub share_name: String,
    pub asked_claude: bool,
    pub codex: bool,
    pub asked_codex: bool,
    pub share_writers: bool,
    pub skip_dirs: Option<Vec<String>>,
    pub skip_files: Option<Vec<String>>,
    pub timeline: bool,
    pub rail: bool,
    pub status: bool,
    pub toolbar: bool,
    pub autosave: bool,
    pub dock_right: bool,
    pub trails: bool,
    pub code_lenses: bool,
    pub wrap: bool,
    pub minimap: bool,
    pub grammars_everywhere: bool,
    pub share_relay: bool,
    pub relay: String,
    pub other_panel: bool,
    #[serde(skip_serializing)]
    pub docked: Vec<String>,
    pub console_profile: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            check_versions: true,
            version_prereleases: false,
            complete_paths: true,
            complete_code: true,
            check_updates: true,
            follow_chat: true,
            follow_changes: true,
            edit: false,
            unwanted: Vec::new(),
            line_numbers: true,
            share_name: String::new(),
            asked_claude: false,
            codex: false,
            asked_codex: false,
            share_writers: false,
            skip_dirs: None,
            skip_files: None,
            timeline: true,
            rail: true,
            status: true,
            toolbar: true,
            autosave: true,
            dock_right: false,
            trails: true,
            code_lenses: true,
            wrap: false,
            minimap: true,
            grammars_everywhere: false,
            share_relay: false,
            relay: String::new(),
            other_panel: false,
            docked: Vec::new(),
            console_profile: String::new(),
        }
    }
}

impl Prefs {
    pub fn settled(mut self) -> Prefs {
        if !std::mem::take(&mut self.docked).is_empty() {
            self.other_panel = true;
        }
        self
    }
}

fn path() -> std::path::PathBuf {
    crate::session::data_dir().join("prefs.json")
}

pub fn load() -> Prefs {
    let read = std::fs::read_to_string(path()).ok().and_then(|text| serde_json::from_str::<Prefs>(&text).ok());
    read.unwrap_or_default().settled()
}

pub fn save(prefs: &Prefs) {
    let result = serde_json::to_string_pretty(prefs)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(path(), json));
    if let Err(e) = result {
        log::warn!("could not save the preferences: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::Prefs;

    fn read(json: &str) -> Prefs {
        serde_json::from_str::<Prefs>(json).unwrap().settled()
    }

    #[test]
    fn the_other_bar_floats_its_views_until_its_panel_is_on() {
        assert!(!Prefs::default().other_panel);
        let old = read(r#"{"edit": true, "dock_right": true, "relay": "example.org"}"#);
        assert!(old.edit && old.dock_right);
        assert!(!old.other_panel, "a file of before: everything of the other bar floats as it did");
    }

    #[test]
    fn views_docked_by_an_older_version_turn_the_panel_of_the_other_bar_on() {
        let docked = read(r#"{"dock_right": true, "docked": ["view:claudeVSCodeSidebar"]}"#);
        assert!(docked.other_panel && docked.dock_right);
        assert!(docked.docked.is_empty());
        assert!(!read(r#"{"docked": []}"#).other_panel, "nothing docked: floating, as before");
        let saved = serde_json::to_string_pretty(&docked).unwrap();
        assert!(!saved.contains("docked"), "the old list is not written again");
        assert_eq!(read(&saved), docked);
        let off = read(r#"{"other_panel": false}"#);
        assert_eq!(read(&serde_json::to_string(&off).unwrap()), off);
    }
}
