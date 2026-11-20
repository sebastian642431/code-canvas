use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_core::grep::{self, FileHits, Hit, Matcher, PathFilter, Query, Source};

pub const MAX_HITS: usize = 5000;
/// How long files changed on disk or typed in are left alone before they are searched again.
const SETTLE: Duration = Duration::from_millis(400);
/// Past this many changed files the whole search runs again instead of one file at a time.
const ONE_BY_ONE: usize = 64;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub query: Query,
    pub include: String,
    pub exclude: String,
}

pub struct Results {
    pub asked: Request,
    pub matcher: Arc<Matcher>,
    pub files: Vec<FileHits>,
    pub truncated: bool,
    pub ms: f32,
}

impl Results {
    pub fn count(&self) -> usize {
        self.files.iter().map(|file| file.hits.len()).sum()
    }
}

struct Done {
    found: grep::Found,
    ms: f32,
}

struct Job {
    asked: Request,
    matcher: Arc<Matcher>,
    /// Only these files are searched again, into the results there are.
    only: Option<Vec<String>>,
    stop: Arc<AtomicBool>,
    rx: Receiver<Done>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A file with its matches replaced, to be written (or put in its editor when it has unsaved changes).
pub struct Rewritten {
    pub rel: String,
    pub text: String,
    pub count: usize,
    pub unsaved: bool,
}

pub enum Due {
    All(Request),
    Files(Request, Vec<String>),
}

/// The Search panel's results of one folder, and the searches running for it.
#[derive(Default)]
pub struct Grep {
    pub results: Option<Results>,
    pub error: Option<String>,
    /// The result shown last on the canvas: file and match.
    pub at: Option<(usize, usize)>,
    /// The marks on the canvas are to be made again from the results.
    pub marks_due: bool,
    job: Option<Job>,
    replacing: Option<Receiver<(Vec<Rewritten>, Vec<String>)>>,
    touched: HashSet<String>,
    touched_at: Option<Instant>,
    again: bool,
}

impl Grep {
    pub fn searching(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.only.is_none())
    }

    pub fn replacing(&self) -> bool {
        self.replacing.is_some()
    }

    pub fn busy(&self) -> bool {
        self.job.is_some() || self.replacing.is_some() || !self.touched.is_empty() || self.again
    }

    pub fn clear(&mut self) {
        let replacing = self.replacing.take();
        *self = Grep { replacing, marks_due: true, ..Grep::default() };
    }

    pub fn start(&mut self, root: PathBuf, sources: Vec<Source>, asked: Request, only: Option<Vec<String>>) {
        let known = only.as_ref().and(self.results.as_ref()).map(|results| results.matcher.clone());
        let matcher = match known {
            Some(matcher) => matcher,
            None => match Matcher::new(&asked.query) {
                Ok(matcher) => Arc::new(matcher),
                Err(why) => {
                    log::info!("search '{}': {why}", asked.query.text);
                    (self.job, self.results, self.at, self.error) = (None, None, None, Some(why));
                    self.marks_due = true;
                    return;
                }
            },
        };
        if only.is_none() {
            self.error = None;
            self.touched.clear();
            (self.touched_at, self.again) = (None, false);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (stopped, looking) = (stop.clone(), matcher.clone());
        let paths = PathFilter::new(&asked.include, &asked.exclude);
        let spawned = std::thread::Builder::new().name("search in files".into()).spawn(move || {
            let t = Instant::now();
            let found = grep::search(&root, sources, &looking, &paths, MAX_HITS, &stopped);
            if !stopped.load(Ordering::Relaxed) {
                let _ = tx.send(Done { found, ms: t.elapsed().as_secs_f32() * 1000.0 });
                crate::ext::wake_ui();
            }
        });
        if let Err(e) = spawned {
            log::error!("the search could not start: {e}");
            return;
        }
        self.job = Some(Job { asked, matcher, only, stop, rx });
    }

    pub fn poll(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let done = match job.rx.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                return true;
            }
        };
        let Some(job) = self.job.take() else { return false };
        let asked = job.asked.clone();
        let only = job.only.clone();
        let matcher = job.matcher.clone();
        drop(job);
        match (only, self.results.as_mut()) {
            (Some(only), Some(results)) => {
                let gone: HashSet<&str> = only.iter().map(String::as_str).collect();
                results.files.retain(|file| !gone.contains(file.rel.as_str()));
                for file in done.found.files {
                    let at = results.files.partition_point(|f| Path::new(&f.rel) < Path::new(&file.rel));
                    results.files.insert(at, file);
                }
                results.truncated |= done.found.truncated;
                let valid = |(file, hit): (usize, usize)| results.files.get(file).is_some_and(|f| hit < f.hits.len());
                self.at = self.at.filter(|at| valid(*at));
            }
            (Some(_), None) => {}
            (None, _) => {
                let count: usize = done.found.files.iter().map(|file| file.hits.len()).sum();
                log::info!("search '{}': {count} hits in {:.1} ms", asked.query.text, done.ms);
                let (files, truncated, ms) = (done.found.files, done.found.truncated, done.ms);
                self.results = Some(Results { asked, matcher, files, truncated, ms });
                self.at = None;
            }
        }
        self.marks_due = true;
        true
    }

    /// A file changed (written, typed in, made or deleted): it is searched again in a moment.
    pub fn touched(&mut self, rel: &str) {
        self.marks_due = true;
        if self.results.is_some() || self.searching() {
            self.touched.insert(rel.to_string());
            self.touched_at = Some(Instant::now());
        }
    }

    /// The folder was read again: everything is searched again once it is ready.
    pub fn everything(&mut self) {
        self.marks_due = true;
        if self.results.is_some() || self.searching() {
            self.again = true;
        }
    }

    pub fn due(&mut self, ready: bool) -> Option<Due> {
        if !ready || self.job.is_some() {
            return None;
        }
        let Some(results) = &self.results else {
            (self.again, self.touched_at) = (false, None);
            self.touched.clear();
            return None;
        };
        let asked = results.asked.clone();
        if std::mem::take(&mut self.again) {
            self.touched.clear();
            self.touched_at = None;
            return Some(Due::All(asked));
        }
        if self.touched.is_empty() || self.touched_at.is_some_and(|at| at.elapsed() < SETTLE) {
            return None;
        }
        self.touched_at = None;
        let files: Vec<String> = self.touched.drain().collect();
        Some(if files.len() > ONE_BY_ONE { Due::All(asked) } else { Due::Files(asked, files) })
    }

    /// Takes a match, or every match of a file, out of the results.
    pub fn dismiss(&mut self, file: usize, hit: Option<usize>) {
        let Some(results) = self.results.as_mut() else { return };
        let Some(f) = results.files.get_mut(file) else { return };
        match hit.filter(|hit| *hit < f.hits.len()) {
            Some(hit) => {
                f.hits.remove(hit);
            }
            None => f.hits.clear(),
        }
        if f.hits.is_empty() {
            results.files.remove(file);
        }
        self.at = None;
        self.marks_due = true;
    }

    /// The next (or previous) result after the one shown last.
    pub fn step(&mut self, forward: bool) -> Option<(usize, usize)> {
        let files = &self.results.as_ref()?.files;
        let last = files.len().checked_sub(1)?;
        let hits = |file: usize| files[file].hits.len().max(1);
        let at = self.at.filter(|(file, hit)| files.get(*file).is_some_and(|f| *hit < f.hits.len()));
        let next = match (at, forward) {
            (None, true) => (0, 0),
            (None, false) => (last, hits(last) - 1),
            (Some((file, hit)), true) if hit + 1 < hits(file) => (file, hit + 1),
            (Some((file, _)), true) => (if file == last { 0 } else { file + 1 }, 0),
            (Some((file, hit)), false) if hit > 0 => (file, hit - 1),
            (Some((file, _)), false) => {
                let file = if file == 0 { last } else { file - 1 };
                (file, hits(file) - 1)
            }
        };
        self.at = Some(next);
        Some(next)
    }

    /// Takes the matches to replace out of the results (all, a file's or one) and gives them by file.
    pub fn take(&mut self, file: Option<usize>, hit: Option<usize>) -> Vec<(String, Vec<Hit>)> {
        let Some(results) = self.results.as_mut() else { return Vec::new() };
        self.marks_due = true;
        self.at = None;
        match (file, hit) {
            (None, _) => results.files.drain(..).map(|f| (f.rel, f.hits)).collect(),
            (Some(file), None) if file < results.files.len() => {
                let f = results.files.remove(file);
                vec![(f.rel, f.hits)]
            }
            (Some(file), Some(hit)) if results.files.get(file).is_some_and(|f| hit < f.hits.len()) => {
                let f = &mut results.files[file];
                let taken = f.hits.remove(hit);
                let rel = f.rel.clone();
                if f.hits.is_empty() {
                    results.files.remove(file);
                }
                vec![(rel, vec![taken])]
            }
            _ => Vec::new(),
        }
    }

    /// Replaces the matches in the background: the files come back to be written by the app.
    pub fn replace(&mut self, root: PathBuf, files: Vec<(String, Option<String>, Vec<Hit>)>, with: String) {
        let Some(matcher) = self.results.as_ref().map(|results| results.matcher.clone()) else { return };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut done, mut failed) = (Vec::new(), Vec::new());
            for (rel, unsaved, hits) in files {
                let in_editor = unsaved.is_some();
                let text = match unsaved {
                    Some(text) => text,
                    None => match std::fs::read(root.join(&rel)).map(String::from_utf8) {
                        Ok(Ok(text)) => text,
                        _ => {
                            failed.push(rel);
                            continue;
                        }
                    },
                };
                let (text, count) = grep::replace_in(&text, &matcher, &with, &hits);
                if count > 0 {
                    done.push(Rewritten { rel, text, count, unsaved: in_editor });
                }
            }
            let _ = tx.send((done, failed));
            crate::ext::wake_ui();
        });
        self.replacing = Some(rx);
    }

    /// The files rewritten by a replacement, and the ones that could not be read, once it is done.
    pub fn poll_replace(&mut self) -> Option<(Vec<Rewritten>, Vec<String>)> {
        let rx = self.replacing.as_ref()?;
        let done = match rx.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => (Vec::new(), Vec::new()),
        };
        self.replacing = None;
        Some(done)
    }
}
