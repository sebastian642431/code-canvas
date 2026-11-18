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
    pub grammars_everywhere: bool,
    pub share_relay: bool,
    pub relay: String,
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
            grammars_everywhere: false,
            share_relay: false,
            relay: String::new(),
            docked: Vec::new(),
            console_profile: String::new(),
        }
    }
}

impl Prefs {
    pub fn is_docked(&self, key: &str) -> bool {
        self.docked.iter().any(|docked| docked == key)
    }

    pub fn dock(&mut self, key: &str, before: Option<&str>) {
        self.docked.retain(|docked| docked != key);
        let at = before.and_then(|next| self.docked.iter().position(|docked| docked == next));
        self.docked.insert(at.unwrap_or(self.docked.len()), key.to_string());
    }

    pub fn float(&mut self, key: &str) -> bool {
        let before = self.docked.len();
        self.docked.retain(|docked| docked != key);
        self.docked.len() != before
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

#[cfg(test)]
mod tests {
    use super::Prefs;

    #[test]
    fn every_view_floats_until_it_is_docked() {
        let prefs = Prefs::default();
        assert!(prefs.docked.is_empty());
        assert!(!prefs.is_docked("view:claudeVSCodeSidebar"));
    }

    #[test]
    fn preferences_of_before_docking_still_load() {
        let old = r#"{"edit": true, "dock_right": true, "relay": "example.org"}"#;
        let prefs: Prefs = serde_json::from_str(old).unwrap();
        assert!(prefs.edit && prefs.dock_right);
        assert!(prefs.docked.is_empty(), "nothing docked: every extension floats as before");
    }

    #[test]
    fn docked_views_keep_their_order_through_a_save() {
        let mut prefs = Prefs::default();
        prefs.dock("view:chat", None);
        prefs.dock("tree:shelf", None);
        prefs.dock("view:codex", Some("tree:shelf"));
        assert_eq!(prefs.docked, ["view:chat", "view:codex", "tree:shelf"]);
        let again: Prefs = serde_json::from_str(&serde_json::to_string_pretty(&prefs).unwrap()).unwrap();
        assert_eq!(again, prefs);
        assert!(again.is_docked("view:codex"));
    }

    #[test]
    fn a_tab_is_reordered_and_floated_again() {
        let mut prefs = Prefs::default();
        for key in ["view:a", "view:b", "view:c"] {
            prefs.dock(key, None);
        }
        prefs.dock("view:c", Some("view:a"));
        assert_eq!(prefs.docked, ["view:c", "view:a", "view:b"]);
        prefs.dock("view:c", Some("view:gone"));
        assert_eq!(prefs.docked, ["view:a", "view:b", "view:c"], "a place before a tab not there is the end");
        assert!(prefs.float("view:a"));
        assert!(!prefs.float("view:a"), "floating twice changes nothing");
        assert_eq!(prefs.docked, ["view:b", "view:c"]);
        assert!(!prefs.is_docked("view:a"));
    }
}
