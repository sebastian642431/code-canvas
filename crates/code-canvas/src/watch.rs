use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

const SETTLE: Duration = Duration::from_millis(350);
const OWN: Duration = Duration::from_secs(10);

type Stamp = Option<u64>;

fn stamp_of(bytes: &[u8]) -> Stamp {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

fn stamp(path: &Path) -> Stamp {
    stamp_of(&std::fs::read(path).ok()?)
}

struct Own {
    at: Instant,
    left: Option<Stamp>,
}

pub struct Watcher {
    inner: Option<RecommendedWatcher>,
    rx: Receiver<PathBuf>,
    watched: Vec<(PathBuf, bool)>,
    pending: HashMap<PathBuf, Instant>,
    own: HashMap<PathBuf, Own>,
}

#[derive(Default)]
pub struct Batch {
    pub files: Vec<PathBuf>,
    pub git: Vec<PathBuf>,
}

impl Watcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let inner = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            for path in event.paths {
                let _ = tx.send(path);
            }
            crate::ext::wake_ui();
        });
        if let Err(e) = &inner {
            log::warn!("files will not be watched for changes: {e}");
        }
        Watcher { inner: inner.ok(), rx, watched: Vec::new(), pending: HashMap::new(), own: HashMap::new() }
    }

    pub fn set_folders(&mut self, folders: &[(PathBuf, bool)]) {
        if folders == self.watched.as_slice() {
            return;
        }
        let Some(w) = self.inner.as_mut() else { return };
        for (old, _) in self.watched.iter().filter(|o| !folders.contains(o)) {
            let _ = w.unwatch(old);
        }
        for (new, deep) in folders.iter().filter(|n| !self.watched.contains(n)) {
            let mode = if *deep { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            if let Err(e) = w.watch(new, mode) {
                log::warn!("cannot watch {}: {e}", new.display());
            }
        }
        self.watched = folders.to_vec();
    }

    pub fn expect(&mut self, path: &Path) {
        self.own.insert(path.to_path_buf(), Own { at: Instant::now(), left: None });
    }

    pub fn write(&mut self, path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
        std::fs::write(path, contents.as_ref())?;
        self.own.insert(path.to_path_buf(), Own { at: Instant::now(), left: Some(stamp_of(contents.as_ref())) });
        Ok(())
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn poll(&mut self) -> Option<Batch> {
        let now = Instant::now();
        while let Ok(path) = self.rx.try_recv() {
            self.pending.insert(path, now);
        }
        self.own.retain(|_, own| own.at.elapsed() < OWN);
        for (path, own) in self.own.iter_mut().filter(|(_, own)| own.left.is_none()) {
            own.left = Some(stamp(path));
        }
        if self.pending.is_empty() || self.pending.values().any(|t| t.elapsed() < SETTLE) {
            return None;
        }
        let mut batch = Batch::default();
        for (path, _) in self.pending.drain() {
            if self.own.get(&path).is_some_and(|own| own.left == Some(stamp(&path))) {
                continue;
            }
            self.own.remove(&path);
            let Some((root, _)) = self.watched.iter().find(|(r, _)| path.starts_with(r)) else { continue };
            let inside_git = path.strip_prefix(root).is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == ".git"));
            if inside_git {
                if path.extension().is_none_or(|e| e != "lock") && !batch.git.contains(root) {
                    batch.git.push(root.clone());
                }
            } else {
                batch.files.push(path);
            }
        }
        (!batch.files.is_empty() || !batch.git.is_empty()).then_some(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_the_app_wrote_itself_goes_unnoticed() {
        let dir = std::env::temp_dir().join(format!("code-canvas-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (saved, mirrored) = (dir.join("saved.txt"), dir.join("mirrored.txt"));
        let mut watcher = Watcher::new();
        watcher.inner = None;
        watcher.watched = vec![(dir.clone(), true)];
        let seen = |watcher: &mut Watcher, path: &Path| {
            watcher.pending.insert(path.to_path_buf(), Instant::now().checked_sub(SETTLE * 2).unwrap());
            watcher.poll().map(|batch| batch.files)
        };

        watcher.write(&saved, "one\n").unwrap();
        assert_eq!(seen(&mut watcher, &saved), None, "its own save");
        std::fs::write(&saved, "one\ntwo, by another program\n").unwrap();
        assert_eq!(seen(&mut watcher, &saved), Some(vec![saved.clone()]), "another program, right after the save");
        assert_eq!(seen(&mut watcher, &saved), Some(vec![saved.clone()]));
        watcher.write(&saved, "two\n").unwrap();
        std::fs::write(&saved, "TWO\n").unwrap();
        assert_eq!(seen(&mut watcher, &saved).map(|files| files.len()), Some(1), "the same size, written later");

        watcher.expect(&mirrored);
        std::fs::write(&mirrored, "one\n").unwrap();
        assert_eq!(seen(&mut watcher, &mirrored), None, "written after saying it would");
        assert_eq!(seen(&mut watcher, &mirrored), None, "the same write, told twice");
        std::fs::remove_file(&mirrored).unwrap();
        assert_eq!(seen(&mut watcher, &mirrored), Some(vec![mirrored.clone()]), "deleted by another program");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
