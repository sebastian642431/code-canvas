//! Watching the open folders: when files change on disk (another editor, a
//! formatter, `git checkout`, an AI assistant) the canvas follows.
//!
//! The operating system reports changes per path; [`Watcher::poll`] hands
//! them over in batches once things have been quiet for a moment, already
//! split into "git's own files changed" and "these files changed".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// Quiet time before a batch is handed over.
const SETTLE: Duration = Duration::from_millis(350);

pub struct Watcher {
    inner: Option<RecommendedWatcher>,
    rx: Receiver<PathBuf>,
    watched: Vec<PathBuf>,
    /// Paths reported and not handed over yet, with when they were last seen.
    pending: HashMap<PathBuf, Instant>,
    /// Files this app wrote itself: their change is not news.
    own: HashMap<PathBuf, Instant>,
}

/// What changed, for one batch.
#[derive(Default)]
pub struct Batch {
    /// Files that were created, changed or removed.
    pub files: Vec<PathBuf>,
    /// Folders (their roots) whose `.git` changed: status and history may differ.
    pub git: Vec<PathBuf>,
}

impl Watcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let inner = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            // Reads are not changes.
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

    /// Watch exactly these folders.
    pub fn set_folders(&mut self, folders: &[PathBuf]) {
        if folders == self.watched.as_slice() {
            return;
        }
        let Some(w) = self.inner.as_mut() else { return };
        for old in self.watched.iter().filter(|o| !folders.contains(o)) {
            let _ = w.unwatch(old);
        }
        for new in folders.iter().filter(|n| !self.watched.contains(n)) {
            if let Err(e) = w.watch(new, RecursiveMode::Recursive) {
                log::warn!("cannot watch {}: {e}", new.display());
            }
        }
        self.watched = folders.to_vec();
    }

    /// The app is about to write `path` itself.
    pub fn expect(&mut self, path: &Path) {
        self.own.insert(path.to_path_buf(), Instant::now());
    }

    /// True while changes are waiting to settle (keep polling).
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Changes that have settled, if any.
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
            let Some(root) = self.watched.iter().find(|r| path.starts_with(r)) else { continue };
            let inside_git = path.strip_prefix(root).is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == ".git"));
            if inside_git {
                // Lock files come and go while git works; the rest means "look again".
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
