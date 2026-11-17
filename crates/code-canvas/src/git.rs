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
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hunk {
    pub line: u32,
    pub count: u32,
    pub kind: HunkKind,
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub path: String,
    pub change: Change,
    pub staged: Option<Change>,
    pub unstaged: Option<Change>,
    pub hunks: Vec<Hunk>,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, Default)]
pub struct GitState {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub upstream: bool,
    pub graph: Vec<GraphCommit>,
    pub branches: Vec<String>,
    pub changes: Vec<FileChange>,
    pub truncated: bool,
    pub ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommit {
    pub commit: Commit,
    pub refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Nothing,
    Push,
    Sync,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitOp {
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit { message: String, all: bool, amend: bool, then: After },
    Sync,
    Push,
    Pull,
    Fetch,
    Publish(String),
    Discard(String),
    DiscardAll,
    Stash,
    StashPop,
    Switch(String),
}

impl GitOp {
    pub fn commit(message: &str) -> GitOp {
        GitOp::Commit { message: message.to_string(), all: false, amend: false, then: After::Nothing }
    }
}

fn steps(op: &GitOp) -> Vec<Vec<String>> {
    let s = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<String>>();
    match op {
        GitOp::Stage(path) => vec![s(&["add", "--", path])],
        GitOp::Unstage(path) => vec![s(&["reset", "-q", "--", path])],
        GitOp::StageAll => vec![s(&["add", "-A", "--", "."])],
        GitOp::UnstageAll => vec![s(&["reset", "-q", "--", "."])],
        GitOp::Commit { message, all, amend, then } => {
            let mut steps = Vec::new();
            if *all {
                steps.push(s(&["add", "-A", "--", "."]));
            }
            let mut commit = s(&["commit"]);
            if *amend {
                commit.push("--amend".into());
            }
            if message.trim().is_empty() && *amend {
                commit.push("--no-edit".into());
            } else {
                commit.extend(s(&["-m", message]));
            }
            steps.push(commit);
            match then {
                After::Nothing => {}
                After::Push => steps.push(s(&["push"])),
                After::Sync => steps.extend([s(&["pull", "--ff-only"]), s(&["push"])]),
            }
            steps
        }
        GitOp::Sync => vec![s(&["pull", "--ff-only"]), s(&["push"])],
        GitOp::Push => vec![s(&["push"])],
        GitOp::Pull => vec![s(&["pull", "--ff-only"])],
        GitOp::Fetch => vec![s(&["fetch", "--prune"])],
        GitOp::Publish(branch) => vec![s(&["push", "-u", "origin", branch])],
        GitOp::Discard(path) => vec![s(&["checkout", "HEAD", "--", path])],
        GitOp::DiscardAll => vec![s(&["checkout", "HEAD", "--", "."])],
        GitOp::Stash => vec![s(&["stash", "push", "--include-untracked"])],
        GitOp::StashPop => vec![s(&["stash", "pop"])],
        GitOp::Switch(branch) => vec![s(&["switch", branch])],
    }
}

pub fn run(root: &Path, op: &GitOp) -> Result<String, String> {
    let mut said = String::new();
    for args in steps(op) {
        let mut c = command(root);
        let out = c.args(&args).stdin(Stdio::null()).output().map_err(|e| format!("git could not be run: {e}"))?;
        let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
        if !out.status.success() {
            let err = text(&out.stderr);
            return Err(if err.is_empty() { text(&out.stdout) } else { err });
        }
        if said.is_empty() {
            said = text(&out.stdout).lines().next().unwrap_or_default().to_string();
        }
    }
    Ok(said)
}

pub fn change_for_message(root: &Path) -> Option<crate::commit_ai::Change> {
    let text = |args: &[&str]| git(root, args).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let flags = ["--no-color", "--no-ext-diff"];
    let mut diff = text(&[&["diff", "--cached"][..], &flags[..]].concat());
    let staged = !diff.trim().is_empty();
    if !staged {
        diff = text(&[&["diff"][..], &flags[..]].concat());
        let new = text(&["ls-files", "--others", "--exclude-standard"]);
        if !new.trim().is_empty() {
            diff.push_str("\n\nUntracked files:\n");
            diff.push_str(new.trim_end());
        }
    }
    if diff.trim().is_empty() {
        return None;
    }
    let recent = text(&["log", "-n", "10", "--format=%s"]).lines().map(str::to_string).collect();
    Some(crate::commit_ai::Change { staged, diff, recent })
}

const MAX_CHANGES: usize = 5000;

fn command(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.env("GIT_OPTIONAL_LOCKS", "0").arg("-C").arg(root).args(["-c", "core.quotepath=off"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = command(root).args(args).stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        log::debug!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
        return None;
    }
    Some(out.stdout)
}

pub fn load(root: &Path) -> Option<GitState> {
    let started = Instant::now();
    let (status, prefix, diff, log, list) = std::thread::scope(|s| {
        let status =
            s.spawn(|| git(root, &["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=all", "--", "."]));
        let prefix = s.spawn(|| git(root, &["rev-parse", "--show-prefix"]));
        let diff =
            s.spawn(|| git(root, &["diff", "HEAD", "-U0", "--no-color", "--no-ext-diff", "--relative", "--", "."]));
        let log = s.spawn(|| git(root, &["log", "-n", "40", "--format=%H%x1f%an%x1f%at%x1f%D%x1f%s"]));
        let list = s.spawn(|| git(root, &["branch", "--format=%(refname:short)"]));
        let done = |h: std::thread::ScopedJoinHandle<Option<Vec<u8>>>| h.join().ok().flatten();
        (done(status), done(prefix), done(diff), done(log), done(list))
    });
    let status = status?;
    let prefix = prefix.map(|b| String::from_utf8_lossy(&b).trim().to_string()).unwrap_or_default();
    let mut state = parse_status(&String::from_utf8_lossy(&status), &prefix);
    if let Some(diff) = diff {
        apply_diff(&mut state, &String::from_utf8_lossy(&diff));
    }
    if let Some(log) = log {
        state.graph = parse_graph(&String::from_utf8_lossy(&log));
    }
    if let Some(list) = list {
        state.branches = String::from_utf8_lossy(&list).lines().map(str::to_string).filter(|b| !b.is_empty()).collect();
    }
    state.ms = started.elapsed().as_millis() as u32;
    Some(state)
}

fn parse_status(text: &str, prefix: &str) -> GitState {
    let mut state = GitState::default();
    let mut entries = text.split('\0');
    while let Some(entry) = entries.next() {
        if let Some(head) = entry.strip_prefix("## ") {
            let name = head.strip_prefix("No commits yet on ").unwrap_or(head);
            state.branch = name.split("...").next().unwrap_or(name).split(' ').next().unwrap_or(name).to_string();
            let count = |word: &str| -> u32 {
                let after = head.split(word).nth(1).unwrap_or("");
                after.trim_start().split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok()).unwrap_or(0)
            };
            state.ahead = count("ahead ");
            state.behind = count("behind ");
            state.upstream = head.contains("...");
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry.as_bytes()[0], entry.as_bytes()[1]);
        let path = &entry[3..];
        if x == b'R' || x == b'C' {
            entries.next();
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
        let side = |code: u8| match code {
            b'M' | b'T' => Some(Change::Modified),
            b'A' => Some(Change::Added),
            b'D' => Some(Change::Deleted),
            b'R' | b'C' => Some(Change::Renamed),
            b'U' => Some(Change::Conflicted),
            b'?' => Some(Change::Untracked),
            _ => None,
        };
        state.changes.push(FileChange {
            path: rel.to_string(),
            change,
            staged: if x == b'?' { None } else { side(x) },
            unstaged: side(y),
            hunks: Vec::new(),
            insertions: 0,
            deletions: 0,
        });
    }
    state.changes.sort_by(|a, b| Path::new(&a.path).cmp(Path::new(&b.path)));
    state
}

fn apply_diff(state: &mut GitState, diff: &str) {
    let mut current: Option<usize> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            let path = path.strip_prefix("b/").unwrap_or(path).trim_end_matches('\t');
            current = state.changes.binary_search_by(|c| Path::new(&c.path).cmp(Path::new(path))).ok();
        } else if line.starts_with("diff --git ") {
            current = None;
        } else if let (Some(i), Some(rest)) = (current, line.strip_prefix("@@ -")) {
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
                (_, 0) => Hunk { line: new_start, count: 0, kind: HunkKind::Removed },
                (0, n) => Hunk { line: new_start.saturating_sub(1), count: n, kind: HunkKind::Added },
                (_, n) => Hunk { line: new_start.saturating_sub(1), count: n, kind: HunkKind::Modified },
            });
        }
    }
}

pub fn not_ignored(root: &Path, paths: Vec<String>) -> Vec<String> {
    use std::io::Write;
    let mut c = command(root);
    c.args(["check-ignore", "--stdin", "-z"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child) = c.spawn() else { return paths };
    if let Some(mut stdin) = child.stdin.take() {
        for p in &paths {
            let _ = stdin.write_all(p.as_bytes());
            let _ = stdin.write_all(b"\0");
        }
    }
    let Ok(out) = child.wait_with_output() else { return paths };
    if !matches!(out.status.code(), Some(0) | Some(1)) {
        return paths;
    }
    let listed = String::from_utf8_lossy(&out.stdout);
    let ignored: std::collections::HashSet<&str> = listed.split('\0').filter(|s| !s.is_empty()).collect();
    paths.into_iter().filter(|p| !ignored.contains(p.as_str())).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    pub time: i64,
    pub subject: String,
}

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

fn parse_graph(text: &str) -> Vec<GraphCommit> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\u{1f}');
            let (hash, author, time, refs) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            let commit = Commit {
                hash: hash.to_string(),
                author: author.to_string(),
                time: time.parse().ok()?,
                subject: parts.next().unwrap_or_default().to_string(),
            };
            let refs = refs
                .split(", ")
                .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r).trim())
                .filter(|r| !r.is_empty() && *r != "HEAD" && !r.ends_with("/HEAD"))
                .map(str::to_string)
                .collect();
            Some(GraphCommit { commit, refs })
        })
        .collect()
}

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
    fn staged_and_unstaged_sides() {
        let s = parse_status("## main...origin/main [ahead 2, behind 1]\0MM a.rs\0D  b.rs\0?? c.rs\0 M d.rs\0", "");
        assert_eq!((s.ahead, s.behind), (2, 1));
        let sides: Vec<_> = s.changes.iter().map(|c| (c.path.as_str(), c.staged, c.unstaged)).collect();
        assert_eq!(
            sides,
            vec![
                ("a.rs", Some(Change::Modified), Some(Change::Modified)),
                ("b.rs", Some(Change::Deleted), None),
                ("c.rs", None, Some(Change::Untracked)),
                ("d.rs", None, Some(Change::Modified)),
            ]
        );
    }

    #[test]
    fn what_a_commit_runs() {
        let run = |op: GitOp| steps(&op).iter().map(|s| s.join(" ")).collect::<Vec<_>>();
        assert_eq!(run(GitOp::commit("Fix it")), vec!["commit -m Fix it"]);
        let all = GitOp::Commit { message: "Fix it".into(), all: true, amend: false, then: After::Sync };
        assert_eq!(run(all), vec!["add -A -- .", "commit -m Fix it", "pull --ff-only", "push"]);
        let amend = GitOp::Commit { message: "  ".into(), all: false, amend: true, then: After::Push };
        assert_eq!(run(amend), vec!["commit --amend --no-edit", "push"]);
        let reword = GitOp::Commit { message: "Better".into(), all: false, amend: true, then: After::Nothing };
        assert_eq!(run(reword), vec!["commit --amend -m Better"]);
        assert_eq!(run(GitOp::Publish("feature".into())), vec!["push -u origin feature"]);
        assert_eq!(run(GitOp::Pull), vec!["pull --ff-only"]);
    }

    #[test]
    fn a_branch_without_a_remote() {
        assert!(parse_status("## main...origin/main\0", "").upstream);
        assert!(!parse_status("## feature\0 M a.rs\0", "").upstream);
        assert!(!parse_status("## No commits yet on main\0", "").upstream);
    }

    #[test]
    fn history_with_refs() {
        let text = "aaa\u{1f}Ana\u{1f}1700000000\u{1f}HEAD -> main, origin/main, origin/HEAD\u{1f}Ship it\nbbb\u{1f}Leo\u{1f}1690000000\u{1f}\u{1f}First\n";
        let graph = parse_graph(text);
        assert_eq!(graph.len(), 2);
        assert_eq!(graph[0].refs, vec!["main", "origin/main"]);
        assert_eq!(graph[0].commit.subject, "Ship it");
        assert!(graph[1].refs.is_empty());
        assert_eq!(graph[1].commit.subject, "First");
    }

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
    #[ignore]
    fn load_a_real_repository() {
        let root = std::env::var("GIT_LOAD_TEST").expect("set GIT_LOAD_TEST to a folder");
        let state = load(Path::new(&root)).expect("a repository");
        println!(
            "{} on {}: {} changes, {} commits, {:?}, {} ms",
            root,
            state.branch,
            state.changes.len(),
            state.graph.len(),
            state.branches,
            state.ms
        );
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
