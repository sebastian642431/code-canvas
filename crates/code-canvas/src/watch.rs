use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

const SETTLE: Duration = Duration::from_millis(350);

pub struct Watcher {
    inner: Option<RecommendedWatcher>,
    rx: Receiver<PathBuf>,
    watched: Vec<(PathBuf, bool)>,
    pending: HashMap<PathBuf, Instant>,
    own: HashMap<PathBuf, Instant>,
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
        self.own.insert(path.to_path_buf(), Instant::now());
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn poll(&mut self) -> Option<Batch> {
        let now = Instant::now();
        while let Ok(path) = self.rx.try_recv() {
            self.pending.insert(path, now);
        }
        self.own.retain(|_, t| t.elapsed() < Duration::from_secs(2));
        if self.pending.is_empty() || self.pending.values().any(|t| t.elapsed() < SETTLE) {
            return None;
        }
        let mut batch = Batch::default();
        for (path, _) in self.pending.drain() {
            if self.own.contains_key(&path) {
                continue;
            }
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
