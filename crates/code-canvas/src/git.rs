//! What changed in a folder according to git: the files of `git status` and
//! the lines of `git diff HEAD`. Read by running the user's `git`, off the
//! UI thread; a folder that is not in a repository simply has no changes.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Modified,
    Added,
    Untracked,
    Deleted,
    Renamed,
    Conflicted,
}

impl Change {
    /// The letter source-control lists show.
    pub fn letter(self) -> &'static str {
        match self {
            Change::Modified => "M",
            Change::Added => "A",
            Change::Untracked => "U",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Conflicted => "!",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Change::Modified => "modified",
            Change::Added => "added",
            Change::Untracked => "new (untracked)",
            Change::Deleted => "deleted",
            Change::Renamed => "renamed",
            Change::Conflicted => "in conflict",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HunkKind {
    /// New lines.
    Added,
    /// Lines that replace older ones.
    Modified,
    /// Lines were removed just before `line`.
    Removed,
}

/// A run of changed lines in the file as it is now (0-based).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hunk {
    pub line: u32,
    pub count: u32,
    pub kind: HunkKind,
}

#[derive(Clone, Debug)]
pub struct FileChange {
    /// Relative to the folder, `/` separators.
    pub path: String,
    pub change: Change,
    pub hunks: Vec<Hunk>,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, Default)]
pub struct GitState {
    pub branch: String,
    /// Sorted by path.
    pub changes: Vec<FileChange>,
    /// More changes existed than are kept.
    pub truncated: bool,
    pub ms: u32,
}

/// Changes kept at most (a fresh `node_modules` that is not ignored…).
const MAX_CHANGES: usize = 5000;

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(["-c", "core.quotepath=off"]).args(args).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let out = c.output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// `None` when `root` is not inside a git repository (or git is missing).
pub fn load(root: &Path) -> Option<GitState> {
    let started = Instant::now();
    let status = git(root, &["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=all", "--", "."])?;
    // Paths in the status are relative to the repository, not to `root`.
    let prefix = git(root, &["rev-parse", "--show-prefix"])
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .unwrap_or_default();
    let mut state = parse_status(&String::from_utf8_lossy(&status), &prefix);
    if let Some(diff) = git(root, &["diff", "HEAD", "-U0", "--no-color", "--no-ext-diff", "--relative", "--", "."]) {
        apply_diff(&mut state, &String::from_utf8_lossy(&diff));
    }
    state.ms = started.elapsed().as_millis() as u32;
    Some(state)
}

fn parse_status(text: &str, prefix: &str) -> GitState {
    let mut state = GitState::default();
    let mut entries = text.split('\0');
    while let Some(entry) = entries.next() {
        if let Some(head) = entry.strip_prefix("## ") {
            // `main...origin/main [ahead 1]`, or `No commits yet on main`.
            let name = head.strip_prefix("No commits yet on ").unwrap_or(head);
            state.branch = name.split("...").next().unwrap_or(name).split(' ').next().unwrap_or(name).to_string();
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry.as_bytes()[0], entry.as_bytes()[1]);
        let path = &entry[3..];
        if x == b'R' || x == b'C' {
            entries.next(); // the path it was renamed or copied from
        }
        let change = match (x, y) {
            (b'?', b'?') => Change::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => Change::Conflicted,
            (b'D', _) | (_, b'D') => Change::Deleted,
            (b'R', _) => Change::Renamed,
            (b'A', _) => Change::Added,
            _ => Change::Modified,
        };
        let Some(rel) = path.strip_prefix(prefix) else { continue };
        if state.changes.len() >= MAX_CHANGES {
            state.truncated = true;
            break;
        }
        state.changes.push(FileChange {
            path: rel.to_string(),
            change,
            hunks: Vec::new(),
            insertions: 0,
            deletions: 0,
        });
    }
    state.changes.sort_by(|a, b| Path::new(&a.path).cmp(Path::new(&b.path)));
    state
}

/// Fill in the changed lines from a `git diff -U0` (paths relative to the folder).
fn apply_diff(state: &mut GitState, diff: &str) {
    let mut current: Option<usize> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            let path = path.strip_prefix("b/").unwrap_or(path).trim_end_matches('\t');
            current = state.changes.binary_search_by(|c| Path::new(&c.path).cmp(Path::new(path))).ok();
        } else if line.starts_with("diff --git ") {
            current = None;
        } else if let (Some(i), Some(rest)) = (current, line.strip_prefix("@@ -")) {
            // `@@ -old_start,old_count +new_start,new_count @@`; a count of 1 is left out.
            let mut parts = rest.split(' ');
            let pair = |s: &str| -> (u32, u32) {
                let mut it = s.split(',');
                let start = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                (start, it.next().map_or(1, |v| v.parse().unwrap_or(0)))
            };
            let (_, old_count) = pair(parts.next().unwrap_or(""));
            let (new_start, new_count) = pair(parts.next().unwrap_or("").trim_start_matches('+'));
            let c = &mut state.changes[i];
            c.insertions += new_count;
            c.deletions += old_count;
            c.hunks.push(match (old_count, new_count) {
                // A pure removal sits after line `new_start`.
                (_, 0) => Hunk { line: new_start, count: 0, kind: HunkKind::Removed },
                (0, n) => Hunk { line: new_start.saturating_sub(1), count: n, kind: HunkKind::Added },
                (_, n) => Hunk { line: new_start.saturating_sub(1), count: n, kind: HunkKind::Modified },
            });
        }
    }
}

/// One commit of a file's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
}

/// The last `max` commits that touched `rel_path` (following renames),
/// newest first. Empty outside a repository or for a file never committed.
pub fn log(root: &Path, rel_path: &str, max: usize) -> Vec<Commit> {
    let count = max.to_string();
    let args = ["log", "--follow", "-n", &count, "--format=%H%x1f%an%x1f%at%x1f%s", "--", rel_path];
    git(root, &args).map(|out| parse_log(&String::from_utf8_lossy(&out))).unwrap_or_default()
}

fn parse_log(text: &str) -> Vec<Commit> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\u{1f}');
            Some(Commit {
                hash: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                time: parts.next()?.parse().ok()?,
                subject: parts.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// "3 days ago", for a moment `seconds` in the past.
pub fn ago(seconds: i64) -> String {
    let units = [
        (365 * 86400, "year"),
        (30 * 86400, "month"),
        (7 * 86400, "week"),
        (86400, "day"),
        (3600, "hour"),
        (60, "minute"),
    ];
    for (size, name) in units {
        let n = seconds / size;
        if n >= 1 {
            return format!("{n} {name}{} ago", if n == 1 { "" } else { "s" });
        }
    }
    "just now".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_history() {
        let text = "abc123\u{1f}Ana\u{1f}1700000000\u{1f}Fix: the | parser\nbroken line\ndef456\u{1f}Leo\u{1f}1690000000\u{1f}\n";
        let log = parse_log(text);
        assert_eq!(log.len(), 2);
        let first = Commit {
            hash: "abc123".into(),
            author: "Ana".into(),
            time: 1_700_000_000,
            subject: "Fix: the | parser".into(),
        };
        assert_eq!(log[0], first);
        assert_eq!(log[1].subject, "");
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(3600), "1 hour ago");
        assert_eq!(ago(3 * 86400), "3 days ago");
        assert_eq!(ago(800 * 86400), "2 years ago");
    }

    #[test]
    fn status_entries() {
        let text = "## main...origin/main [ahead 1]\0 M web/src/a.ts\0?? web/new file.ts\0R  web/src/b.ts\0web/src/old.ts\0 D web/gone.ts\0A  api/x.rs\0UU web/src/c.ts\0";
        let s = parse_status(text, "web/");
        assert_eq!(s.branch, "main");
        let got: Vec<(&str, Change)> = s.changes.iter().map(|c| (c.path.as_str(), c.change)).collect();
        assert_eq!(
            got,
            vec![
                ("gone.ts", Change::Deleted),
                ("new file.ts", Change::Untracked),
                ("src/a.ts", Change::Modified),
                ("src/b.ts", Change::Renamed),
                ("src/c.ts", Change::Conflicted),
            ],
            "paths are relative to the folder; files outside it and rename sources are left out"
        );
        assert_eq!(parse_status("## No commits yet on trunk\0", "").branch, "trunk");
    }

    #[test]
    fn diff_hunks() {
        let mut s = parse_status(" M src/a.ts\0 M src/b.ts\0", "");
        let diff = "diff --git a/src/a.ts b/src/a.ts\nindex 1..2 100644\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -3 +3,2 @@ fn x\n-old\n+new\n+more\n@@ -10,2 +10,0 @@\n-gone\n-gone\n@@ -20,0 +19,3 @@\n+a\n+b\n+c\ndiff --git a/other b/other\n+++ b/other\n@@ -1 +1 @@\n";
        apply_diff(&mut s, diff);
        let a = &s.changes[0];
        assert_eq!(
            a.hunks,
            vec![
                Hunk { line: 2, count: 2, kind: HunkKind::Modified },
                Hunk { line: 10, count: 0, kind: HunkKind::Removed },
                Hunk { line: 18, count: 3, kind: HunkKind::Added },
            ]
        );
        assert_eq!((a.insertions, a.deletions), (5, 3));
        assert!(s.changes[1].hunks.is_empty(), "a file not in the diff keeps no hunks");
    }
}
