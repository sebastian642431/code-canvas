//! A watchdog for work that repeats without end: a file some program keeps writing (a
//! server's log), git read again and again, a folder scanned over and over. Plain counts
//! over the last minute, so the person can be told what keeps Code Canvas busy.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WINDOW_SECS: usize = 60;
pub const MAX_PATHS: usize = 256;
pub const PATH_WRITES: u32 = 30;
pub const GIT_RELOADS: u32 = 20;
pub const RESCANS: u32 = 5;
pub const WARN_AGAIN: Duration = Duration::from_secs(10 * 60);
pub const LOG_EVERY: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(1);
const TOP: usize = 5;

/// Counts of the last minute in one-second buckets: fixed size, O(1) to add to.
#[derive(Clone)]
struct Window {
    buckets: [u32; WINDOW_SECS],
    second: u64,
    total: u32,
    last: u64,
}

impl Default for Window {
    fn default() -> Self {
        Window { buckets: [0; WINDOW_SECS], second: 0, total: 0, last: 0 }
    }
}

impl Window {
    fn advance(&mut self, second: u64) {
        if second <= self.second {
            return;
        }
        let gap = (second - self.second).min(WINDOW_SECS as u64);
        for k in 1..=gap {
            let slot = ((self.second + k) % WINDOW_SECS as u64) as usize;
            self.total -= self.buckets[slot];
            self.buckets[slot] = 0;
        }
        self.second = second;
    }

    fn add(&mut self, second: u64, n: u32) {
        self.advance(second);
        let slot = (self.second % WINDOW_SECS as u64) as usize;
        self.buckets[slot] = self.buckets[slot].saturating_add(n);
        self.total = self.total.saturating_add(n);
        self.last = self.second;
    }

    fn count(&mut self, second: u64) -> u32 {
        self.advance(second);
        self.total
    }
}

#[derive(Default)]
struct Folder {
    git: Window,
    scans: Window,
    paths: HashMap<String, Window>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Cause {
    Writes { folder: PathBuf, rel: String },
    GitReloads(PathBuf),
    Rescans(PathBuf),
}

impl Cause {
    pub fn folder(&self) -> &Path {
        match self {
            Cause::Writes { folder, .. } | Cause::GitReloads(folder) | Cause::Rescans(folder) => folder,
        }
    }

    pub fn limit(&self) -> u32 {
        match self {
            Cause::Writes { .. } => PATH_WRITES,
            Cause::GitReloads(_) => GIT_RELOADS,
            Cause::Rescans(_) => RESCANS,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Count {
    pub cause: Cause,
    pub count: u32,
}

impl Count {
    fn weight(&self) -> f32 {
        self.count as f32 / self.cause.limit() as f32
    }

    fn over(&self) -> bool {
        self.count >= self.cause.limit()
    }
}

/// What to tell about one folder: its most written file when that one is over its limit,
/// and how often git was read and the folder scanned in the last minute.
#[derive(Debug, PartialEq)]
pub struct Warning {
    pub folder: PathBuf,
    pub path: Option<(String, u32)>,
    pub git: u32,
    pub scans: u32,
}

#[derive(Default)]
pub struct Verdict {
    /// The worst offenders of the last minute, worst first; empty while nothing is over its limit.
    pub top: Vec<Count>,
    pub warning: Option<Warning>,
    /// Whether to write the top offenders to the log now (once a minute while busy).
    pub log: bool,
}

pub struct Churn {
    start: Instant,
    folders: HashMap<PathBuf, Folder>,
    tracked: usize,
    warned: HashMap<Cause, Instant>,
    logged: Option<Instant>,
    checked: Option<Instant>,
    busy: bool,
}

impl Default for Churn {
    fn default() -> Self {
        Churn::new(Instant::now())
    }
}

impl Churn {
    pub fn new(start: Instant) -> Self {
        Churn {
            start,
            folders: HashMap::new(),
            tracked: 0,
            warned: HashMap::new(),
            logged: None,
            checked: None,
            busy: false,
        }
    }

    fn second(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.start).as_secs()
    }

    pub fn wrote(&mut self, at: Instant, folder: &Path, rel: &str) {
        let second = self.second(at);
        let known = self.folders.get(folder).is_some_and(|f| f.paths.contains_key(rel));
        if !known && self.tracked >= MAX_PATHS {
            self.evict(second);
        }
        let paths = &mut self.folders.entry(folder.to_path_buf()).or_default().paths;
        if !known {
            self.tracked += 1;
        }
        paths.entry(rel.to_string()).or_default().add(second, 1);
    }

    pub fn git_reloaded(&mut self, at: Instant, folder: &Path, times: u32) {
        if times > 0 {
            let second = self.second(at);
            self.folders.entry(folder.to_path_buf()).or_default().git.add(second, times);
        }
    }

    pub fn rescanned(&mut self, at: Instant, folder: &Path, times: u32) {
        if times > 0 {
            let second = self.second(at);
            self.folders.entry(folder.to_path_buf()).or_default().scans.add(second, times);
        }
    }

    pub fn forget(&mut self, folder: &Path, rel: &str) {
        if self.folders.get_mut(folder).is_some_and(|f| f.paths.remove(rel).is_some()) {
            self.tracked -= 1;
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Drops the quarter of the tracked paths written least, the least recent first among equals.
    fn evict(&mut self, second: u64) {
        let mut all: Vec<(u32, u64, PathBuf, String)> = Vec::with_capacity(self.tracked);
        for (folder, f) in &mut self.folders {
            for (rel, window) in &mut f.paths {
                all.push((window.count(second), window.last, folder.clone(), rel.clone()));
            }
        }
        all.sort_unstable_by_key(|(count, last, ..)| (*count, *last));
        for (_, _, folder, rel) in all.into_iter().take(MAX_PATHS / 4) {
            self.forget(&folder, &rel);
        }
    }

    /// Every count above zero, worst first; what has gone quiet is forgotten.
    fn counts(&mut self, second: u64) -> Vec<Count> {
        let mut all = Vec::new();
        let mut gone = 0;
        self.folders.retain(|folder, f| {
            f.paths.retain(|rel, window| {
                let count = window.count(second);
                if count == 0 {
                    gone += 1;
                    return false;
                }
                all.push(Count { cause: Cause::Writes { folder: folder.clone(), rel: rel.clone() }, count });
                true
            });
            let (git, scans) = (f.git.count(second), f.scans.count(second));
            if git > 0 {
                all.push(Count { cause: Cause::GitReloads(folder.clone()), count: git });
            }
            if scans > 0 {
                all.push(Count { cause: Cause::Rescans(folder.clone()), count: scans });
            }
            !f.paths.is_empty() || git > 0 || scans > 0
        });
        self.tracked -= gone;
        all.sort_by(|a, b| b.weight().total_cmp(&a.weight()));
        all
    }

    pub fn top(&mut self, at: Instant, n: usize) -> Vec<Count> {
        let second = self.second(at);
        self.counts(second).into_iter().take(n).collect()
    }

    fn warning_in(folder: &Path, all: &[Count]) -> Warning {
        let mut warning = Warning { folder: folder.to_path_buf(), path: None, git: 0, scans: 0 };
        for count in all.iter().filter(|c| c.cause.folder() == folder) {
            match &count.cause {
                Cause::Writes { rel, .. } if warning.path.is_none() && count.over() => {
                    warning.path = Some((rel.clone(), count.count));
                }
                Cause::GitReloads(_) => warning.git = count.count,
                Cause::Rescans(_) => warning.scans = count.count,
                Cause::Writes { .. } => {}
            }
        }
        warning
    }

    /// Marks every cause of the folder as told, so one notice covers what one loop sets off.
    fn mark_warned(&mut self, at: Instant, folder: &Path, over: &[Count]) {
        for count in over.iter().filter(|c| c.cause.folder() == folder) {
            self.warned.insert(count.cause.clone(), at);
        }
        self.warned.insert(Cause::GitReloads(folder.to_path_buf()), at);
        self.warned.insert(Cause::Rescans(folder.to_path_buf()), at);
    }

    /// At most once a second: what is over its limit, whether to warn (each cause at most
    /// once every `WARN_AGAIN`) and whether to log.
    pub fn check(&mut self, at: Instant) -> Option<Verdict> {
        if self.checked.is_some_and(|t| at.saturating_duration_since(t) < CHECK_EVERY) {
            return None;
        }
        self.checked = Some(at);
        let all = self.counts(self.second(at));
        let over: Vec<Count> = all.iter().filter(|c| c.over()).cloned().collect();
        self.busy = !over.is_empty();
        if over.is_empty() {
            self.logged = None;
            return Some(Verdict::default());
        }
        let log = self.logged.is_none_or(|t| at.saturating_duration_since(t) >= LOG_EVERY);
        if log {
            self.logged = Some(at);
        }
        self.warned.retain(|_, t| at.saturating_duration_since(*t) < WARN_AGAIN);
        let fresh = over.iter().find(|c| !self.warned.contains_key(&c.cause)).map(|c| c.cause.folder().to_path_buf());
        let warning = fresh.map(|folder| {
            self.mark_warned(at, &folder, &over);
            Self::warning_in(&folder, &all)
        });
        Some(Verdict { top: all.into_iter().take(TOP).collect(), warning, log })
    }

    /// The warning for the worst folder right now, whether or not it was told already.
    pub fn warning_now(&mut self, at: Instant) -> Option<Warning> {
        let all = self.counts(self.second(at));
        let folder = all.iter().find(|c| c.over())?.cause.folder().to_path_buf();
        let over: Vec<Count> = all.iter().filter(|c| c.over()).cloned().collect();
        self.mark_warned(at, &folder, &over);
        Some(Self::warning_in(&folder, &all))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> (Instant, impl Fn(u64) -> Instant) {
        let start = Instant::now();
        (start, move |s| start + Duration::from_secs(s))
    }

    fn write(churn: &mut Churn, at: Instant, rel: &str, times: u32) {
        for _ in 0..times {
            churn.wrote(at, Path::new("/repo"), rel);
        }
    }

    #[test]
    fn the_window_forgets_what_is_older_than_a_minute() {
        let mut window = Window::default();
        window.add(0, 10);
        window.add(30, 5);
        assert_eq!(window.count(59), 15);
        assert_eq!(window.count(60), 5);
        assert_eq!(window.count(90), 0);
        window.add(1000, 1);
        assert_eq!(window.count(1000), 1);
        assert_eq!(window.count(5000), 0);
    }

    #[test]
    fn tracked_paths_stay_bounded_and_the_noisy_one_is_kept() {
        let (start, s) = clock();
        let mut churn = Churn::new(start);
        write(&mut churn, s(0), "logs/today.log", 50);
        for i in 0..MAX_PATHS * 4 {
            churn.wrote(s(1), Path::new("/repo"), &format!("src/{i}.rs"));
            assert!(churn.tracked <= MAX_PATHS);
        }
        let paths = &churn.folders[Path::new("/repo")].paths;
        assert_eq!(paths.len(), churn.tracked);
        assert!(paths.contains_key("logs/today.log"));
    }

    #[test]
    fn quiet_paths_and_folders_are_forgotten() {
        let (start, s) = clock();
        let mut churn = Churn::new(start);
        write(&mut churn, s(0), "a.txt", 3);
        churn.git_reloaded(s(0), Path::new("/repo"), 2);
        assert_eq!(churn.top(s(10), 10).len(), 2);
        assert!(churn.top(s(70), 10).is_empty());
        assert_eq!(churn.tracked, 0);
        assert!(churn.folders.is_empty());
    }

    #[test]
    fn a_file_written_past_its_limit_is_warned_about_with_git_of_its_folder() {
        let (start, s) = clock();
        let mut churn = Churn::new(start);
        write(&mut churn, s(0), "logs/today.log", PATH_WRITES - 1);
        churn.git_reloaded(s(0), Path::new("/repo"), 7);
        let verdict = churn.check(s(1)).unwrap();
        assert!(verdict.warning.is_none() && verdict.top.is_empty() && !churn.is_busy());
        write(&mut churn, s(2), "logs/today.log", 1);
        let verdict = churn.check(s(2)).unwrap();
        assert!(churn.is_busy());
        let expected = Warning {
            folder: PathBuf::from("/repo"),
            path: Some(("logs/today.log".into(), PATH_WRITES)),
            git: 7,
            scans: 0,
        };
        assert_eq!(verdict.warning, Some(expected));
        assert_eq!(verdict.top[0].cause, Cause::Writes { folder: "/repo".into(), rel: "logs/today.log".into() });
    }

    #[test]
    fn git_reloads_and_rescans_have_their_own_limits() {
        let (start, s) = clock();
        let mut churn = Churn::new(start);
        churn.git_reloaded(s(0), Path::new("/a"), GIT_RELOADS - 1);
        churn.rescanned(s(0), Path::new("/b"), RESCANS - 1);
        assert!(churn.check(s(0)).unwrap().warning.is_none());
        churn.git_reloaded(s(1), Path::new("/a"), 1);
        let warning = churn.check(s(1)).unwrap().warning.unwrap();
        assert_eq!((warning.folder.as_path(), warning.path, warning.git), (Path::new("/a"), None, GIT_RELOADS));
        churn.rescanned(s(2), Path::new("/b"), 1);
        let warning = churn.check(s(2)).unwrap().warning.unwrap();
        assert_eq!((warning.folder.as_path(), warning.scans), (Path::new("/b"), RESCANS));
    }

    #[test]
    fn the_same_cause_warns_once_in_ten_minutes_and_its_git_does_not_warn_apart() {
        let (start, s) = clock();
        let mut churn = Churn::new(start);
        write(&mut churn, s(0), "server.log", PATH_WRITES);
        assert!(churn.check(s(0)).unwrap().warning.is_some());
        churn.git_reloaded(s(5), Path::new("/repo"), GIT_RELOADS);
        write(&mut churn, s(5), "server.log", PATH_WRITES);
        let verdict = churn.check(s(5)).unwrap();
        assert!(verdict.warning.is_none());
        assert_eq!(verdict.top.len(), 2);
        write(&mut churn, s(30), "other.log", PATH_WRITES);
        assert!(churn.check(s(30)).unwrap().warning.is_some(), "another file is another cause");
        let later = WARN_AGAIN.as_secs();
        write(&mut churn, s(later - 1), "server.log", PATH_WRITES);
        assert!(churn.check(s(later - 1)).unwrap().warning.is_none());
        write(&mut churn, s(later + 1), "server.log", PATH_WRITES);
        assert!(churn.check(s(later + 1)).unwrap().warning.is_some());
        assert!(churn.warning_now(s(later + 2)).is_some(), "asked for, it is told again");
    }

    #[test]
    fn checks_once_a_second_and_logs_once_a_minute_while_busy() {
        let (start, s) = clock();
        let mut churn = Churn::new(start);
        write(&mut churn, s(0), "server.log", PATH_WRITES);
        assert!(churn.check(s(0)).unwrap().log);
        assert!(churn.check(s(0)).is_none());
        assert!(!churn.check(s(1)).unwrap().log);
        write(&mut churn, s(30), "server.log", PATH_WRITES);
        assert!(!churn.check(s(59)).unwrap().log);
        assert!(churn.check(s(60)).unwrap().log);
        let calm = churn.check(s(200)).unwrap();
        assert!(!calm.log && calm.top.is_empty() && !churn.is_busy());
        write(&mut churn, s(201), "server.log", PATH_WRITES);
        assert!(churn.check(s(201)).unwrap().log, "busy again logs at once");
    }

    #[test]
    fn a_forgotten_file_no_longer_counts() {
        let (start, s) = clock();
        let mut churn = Churn::new(start);
        write(&mut churn, s(0), "server.log", PATH_WRITES);
        churn.forget(Path::new("/repo"), "server.log");
        assert_eq!(churn.tracked, 0);
        assert!(churn.check(s(0)).unwrap().warning.is_none());
        assert!(churn.warning_now(s(1)).is_none());
    }
}
