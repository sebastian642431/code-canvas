use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const FILE: &str = "trust.json";

#[derive(Default)]
pub struct Trust {
    file: Option<PathBuf>,
    kept: BTreeMap<String, bool>,
    for_now: HashMap<String, bool>,
}

fn key(folder: &Path) -> String {
    let real = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
    let mut key = crate::ext::path_key(&canvas_core::scan::clean_path(real).to_string_lossy());
    while key.len() > 1 && key.ends_with('/') {
        key.pop();
    }
    key
}

impl Trust {
    pub fn load(data_dir: &Path, known_before: impl FnOnce() -> Vec<PathBuf>) -> Trust {
        let file = data_dir.join(FILE);
        let kept: Option<BTreeMap<String, bool>> =
            std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str(&text).ok());
        let mut trust =
            Trust { file: Some(file.clone()), kept: kept.clone().unwrap_or_default(), for_now: HashMap::new() };
        if kept.is_none() && !file.exists() {
            for folder in known_before() {
                trust.kept.insert(key(&folder), true);
            }
            trust.save();
        }
        trust
    }

    pub fn decided(&self, folder: &Path, shared: bool) -> Option<bool> {
        let key = key(folder);
        match shared {
            true => self.for_now.get(&format!("shared:{key}")).copied(),
            false => self.for_now.get(&key).or(self.kept.get(&key)).copied(),
        }
    }

    pub fn set(&mut self, folder: &Path, shared: bool, trusted: bool, remember: bool) {
        let key = key(folder);
        if shared {
            self.for_now.insert(format!("shared:{key}"), trusted);
            return;
        }
        match remember {
            true => {
                self.for_now.remove(&key);
                self.kept.insert(key, trusted);
                self.save();
            }
            false => {
                self.for_now.insert(key, trusted);
            }
        }
    }

    pub fn forget_shared(&mut self, folder: &Path) {
        self.for_now.remove(&format!("shared:{}", key(folder)));
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let text = serde_json::to_string_pretty(&self.kept).unwrap_or_default();
        if let Err(why) = std::fs::write(file, text) {
            log::warn!("which folders are trusted could not be kept: {why}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Trust {
        fn trusted(&self, folder: &Path, shared: bool) -> bool {
            self.decided(folder, shared) == Some(true)
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code-canvas-trust-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn new_folders_are_not_trusted_until_said_and_the_answer_is_kept() {
        let data = temp("kept");
        let (a, b) = (data.join("a"), data.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut trust = Trust::load(&data, Vec::new);
        assert_eq!(trust.decided(&a, false), None);
        trust.set(&a, false, true, true);
        trust.set(&b, false, false, true);
        let again = Trust::load(&data, || vec![b.clone()]);
        assert!(again.trusted(&a, false));
        assert_eq!(again.decided(&b, false), Some(false), "folders known before count only on the first start");
        assert!(again.trusted(&a.join("."), false), "the same folder written another way");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn folders_open_before_trust_existed_stay_trusted() {
        let data = temp("seed");
        let old = data.join("old");
        std::fs::create_dir_all(&old).unwrap();
        let trust = Trust::load(&data, || vec![old.clone()]);
        assert!(trust.trusted(&old, false));
        assert!(!trust.trusted(&old, true), "never for a shared folder at the same place");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_shared_folder_is_trusted_only_for_this_session_and_never_kept() {
        let data = temp("shared");
        let guest = data.join("guest");
        std::fs::create_dir_all(&guest).unwrap();
        let mut trust = Trust::load(&data, Vec::new);
        trust.set(&guest, true, true, true);
        assert!(trust.trusted(&guest, true));
        assert!(!trust.trusted(&guest, false));
        assert!(!Trust::load(&data, Vec::new).trusted(&guest, true));
        trust.forget_shared(&guest);
        assert_eq!(trust.decided(&guest, true), None);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_choice_for_now_is_forgotten_on_the_next_start() {
        let data = temp("now");
        let folder = data.join("f");
        std::fs::create_dir_all(&folder).unwrap();
        let mut trust = Trust::load(&data, Vec::new);
        trust.set(&folder, false, false, false);
        assert_eq!(trust.decided(&folder, false), Some(false));
        assert_eq!(Trust::load(&data, Vec::new).decided(&folder, false), None);
        let _ = std::fs::remove_dir_all(&data);
    }
}
