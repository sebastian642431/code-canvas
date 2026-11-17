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
    pub unsaved: bool,
}

pub fn head_texts(root: &Path, prefix: &str, paths: &[String]) -> Vec<Option<String>> {
    use std::io::{Read, Write};
    let mut texts = vec![None; paths.len()];
    let mut c = command(root);
    c.args(["cat-file", "--batch"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child) = c.spawn() else { return texts };
    let asked: String = paths.iter().map(|path| format!("HEAD:{prefix}{path}\n")).collect();
    let stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(asked.as_bytes());
        }
    });
    let mut bytes = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut bytes);
    }
    let _ = writer.join();
    let _ = child.wait();
    let mut at = 0;
    for text in texts.iter_mut() {
        let Some(end) = bytes[at..].iter().position(|b| *b == b'\n') else { break };
        let header = String::from_utf8_lossy(&bytes[at..at + end]).into_owned();
        at += end + 1;
        let Some(size) = header.rsplit(' ').next().and_then(|size| size.parse::<usize>().ok()) else { continue };
        if at + size > bytes.len() {
            break;
        }
        if header.contains(" blob ") {
            *text = String::from_utf8(bytes[at..at + size].to_vec()).ok();
        }
        at += size + 1;
    }
    texts
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    Same,
    Added,
    Removed,
    Gap,
    Before,
    Blank,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    pub kind: RowKind,
    pub line: u32,
}

pub struct DiffText {
    pub text: String,
    pub rows: Vec<Row>,
    pub lines: Vec<u32>,
    pub fold: usize,
}

impl DiffText {
    fn new(shown: Vec<(RowKind, u32, &str)>, lines_in_file: usize, fold: usize) -> DiffText {
        let text = shown.iter().map(|(_, _, content)| *content).collect::<Vec<_>>().join("\n");
        let rows: Vec<Row> = shown.iter().map(|(kind, line, _)| Row { kind: *kind, line: *line }).collect();
        DiffText { text, lines: rows_of_lines(&rows, lines_in_file), rows, fold }
    }

    #[cfg(test)]
    fn row(&self, line: u32) -> u32 {
        row_of(&self.lines, self.rows.len(), line)
    }
}

pub fn row_of(lines: &[u32], rows: usize, line: u32) -> u32 {
    lines.get(line as usize).copied().unwrap_or(rows.saturating_sub(1) as u32)
}

fn rows_of_lines(rows: &[Row], lines_in_file: usize) -> Vec<u32> {
    const UNSET: u32 = u32::MAX;
    let mut at = vec![UNSET; lines_in_file];
    for (i, row) in rows.iter().enumerate() {
        if let (RowKind::Same | RowKind::Added, Some(slot)) = (row.kind, at.get_mut(row.line as usize)) {
            *slot = i as u32;
        }
    }
    for (i, row) in rows.iter().enumerate().rev().filter(|(_, row)| row.kind == RowKind::Gap) {
        for slot in at.iter_mut().skip(row.line as usize).take_while(|slot| **slot == UNSET) {
            *slot = i as u32;
        }
    }
    let mut last = 0;
    for slot in &mut at {
        if *slot == UNSET {
            *slot = last;
        }
        last = *slot;
    }
    at
}

enum Piece<'a> {
    Same(&'a str),
    Gap(String),
    Change(&'a [&'a str], &'a [&'a str]),
}

fn pieces<'a>(before: &'a [&'a str], after: &'a [&'a str], around: Option<u32>) -> Option<Vec<(u32, Piece<'a>)>> {
    use similar::DiffOp;
    let deadline = Instant::now() + std::time::Duration::from_millis(60);
    let ops = similar::capture_diff_slices_deadline(similar::Algorithm::Myers, before, after, Some(deadline));
    if ops.iter().all(|op| matches!(op, DiffOp::Equal { .. })) {
        return None;
    }
    let mut out = Vec::new();
    let last = ops.len() - 1;
    for (i, op) in ops.iter().enumerate() {
        match *op {
            DiffOp::Equal { new_index, len, .. } => {
                let keep = around.map_or(len, |n| n as usize);
                let (head, tail) = (if i == 0 { 0 } else { keep }, if i == last { 0 } else { keep });
                let folded = around.is_some() && len > head + tail + 1;
                for k in 0..len {
                    let line = (new_index + k) as u32;
                    if folded && k == head {
                        out.push((line, Piece::Gap(format!("··· {} unchanged lines ···", len - head - tail))));
                    }
                    if !folded || k < head || k >= len - tail {
                        out.push((line, Piece::Same(after[new_index + k])));
                    }
                }
            }
            DiffOp::Delete { old_index, old_len, new_index } => {
                out.push((new_index as u32, Piece::Change(&before[old_index..old_index + old_len], &[])));
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                out.push((new_index as u32, Piece::Change(&[], &after[new_index..new_index + new_len])));
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                let (old, new) = (&before[old_index..old_index + old_len], &after[new_index..new_index + new_len]);
                out.push((new_index as u32, Piece::Change(old, new)));
            }
        }
    }
    Some(out)
}

pub fn unified(old: &str, new: &str, around: Option<u32>) -> Option<DiffText> {
    let (before, after): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let pieces = pieces(&before, &after, around)?;
    let mut shown: Vec<(RowKind, u32, &str)> = Vec::new();
    for (line, piece) in &pieces {
        match piece {
            Piece::Same(text) => shown.push((RowKind::Same, *line, text)),
            Piece::Gap(text) => shown.push((RowKind::Gap, *line, text)),
            Piece::Change(old, new) => {
                shown.extend(old.iter().map(|text| (RowKind::Removed, *line, *text)));
                shown.extend(new.iter().enumerate().map(|(k, text)| (RowKind::Added, line + k as u32, *text)));
            }
        }
    }
    Some(DiffText::new(shown, after.len(), 0))
}

pub fn side_by_side(old: &str, new: &str, around: Option<u32>) -> Option<DiffText> {
    let (before, after): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let pieces = pieces(&before, &after, around)?;
    let mut left: Vec<(RowKind, u32, &str)> = Vec::new();
    let mut right: Vec<(RowKind, u32, &str)> = Vec::new();
    for (line, piece) in &pieces {
        match piece {
            Piece::Same(text) => {
                left.push((RowKind::Before, *line, text));
                right.push((RowKind::Same, *line, text));
            }
            Piece::Gap(text) => {
                left.push((RowKind::Gap, *line, text));
                right.push((RowKind::Gap, *line, text));
            }
            Piece::Change(old, new) => {
                for k in 0..old.len().max(new.len()) {
                    let at = line + k.min(new.len()) as u32;
                    left.push(old.get(k).map_or((RowKind::Blank, at, ""), |text| (RowKind::Removed, at, *text)));
                    right.push(new.get(k).map_or((RowKind::Blank, at, ""), |text| (RowKind::Added, at, *text)));
                }
            }
        }
    }
    let per = left.len().clamp(1, canvas_core::layout::FOLD_LINES);
    let mut shown: Vec<(RowKind, u32, &str)> = Vec::new();
    for (was, is) in left.chunks(per).zip(right.chunks(per)) {
        shown.extend_from_slice(was);
        let line = was.last().map_or(0, |row| row.1);
        shown.extend(std::iter::repeat_n((RowKind::Blank, line, ""), per - was.len()));
        shown.extend_from_slice(is);
    }
    Some(DiffText::new(shown, after.len(), per))
}

pub fn line_of(rows: Option<&[Row]>, row: u32) -> Option<u32> {
    let Some(rows) = rows else { return Some(row) };
    rows.get(row as usize).filter(|r| matches!(r.kind, RowKind::Same | RowKind::Added)).map(|r| r.line)
}

pub fn compare(old: &str, new: &str) -> (Vec<Hunk>, u32, u32) {
    use similar::DiffOp;
    let (before, after): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let deadline = Instant::now() + std::time::Duration::from_millis(60);
    let ops = similar::capture_diff_slices_deadline(similar::Algorithm::Myers, &before, &after, Some(deadline));
    let mut hunks = Vec::new();
    let (mut insertions, mut deletions) = (0, 0);
    for op in ops {
        let (line, removed, added) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Insert { new_index, new_len, .. } => (new_index, 0, new_len),
            DiffOp::Delete { old_len, new_index, .. } => (new_index, old_len, 0),
            DiffOp::Replace { old_len, new_index, new_len, .. } => (new_index, old_len, new_len),
        };
        insertions += added as u32;
        deletions += removed as u32;
        let kind = match (removed, added) {
            (_, 0) => HunkKind::Removed,
            (0, _) => HunkKind::Added,
            _ => HunkKind::Modified,
        };
        hunks.push(Hunk { line: line as u32, count: added as u32, kind });
    }
    (hunks, insertions, deletions)
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
    pub ignored: Vec<String>,
    pub truncated: bool,
    pub ms: u32,
    pub prefix: String,
    pub head: String,
}

impl GitState {
    pub fn change(&self, rel_path: &str) -> Option<&FileChange> {
        let at = self.changes.binary_search_by(|c| Path::new(&c.path).cmp(Path::new(rel_path))).ok()?;
        self.changes.get(at)
    }
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
    let (status, prefix, diff, log, list, ignored) = std::thread::scope(|s| {
        let status =
            s.spawn(|| git(root, &["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=all", "--", "."]));
        let prefix = s.spawn(|| git(root, &["rev-parse", "--show-prefix"]));
        let diff =
            s.spawn(|| git(root, &["diff", "HEAD", "-U0", "--no-color", "--no-ext-diff", "--relative", "--", "."]));
        let log = s.spawn(|| git(root, &["log", "-n", "40", "--format=%H%x1f%an%x1f%at%x1f%D%x1f%s"]));
        let list = s.spawn(|| git(root, &["branch", "--format=%(refname:short)"]));
        let ignored = s.spawn(|| {
            git(root, &["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z", "--", "."])
        });
        let done = |h: std::thread::ScopedJoinHandle<Option<Vec<u8>>>| h.join().ok().flatten();
        (done(status), done(prefix), done(diff), done(log), done(list), done(ignored))
    });
    let status = status?;
    let prefix = prefix.map(|b| String::from_utf8_lossy(&b).trim().to_string()).unwrap_or_default();
    let mut state = parse_status(&String::from_utf8_lossy(&status), &prefix);
    state.prefix = prefix;
    if let Some(diff) = diff {
        apply_diff(&mut state, &String::from_utf8_lossy(&diff));
    }
    if let Some(log) = log {
        state.graph = parse_graph(&String::from_utf8_lossy(&log));
        state.head = state.graph.first().map(|first| first.commit.hash.clone()).unwrap_or_default();
    }
    if let Some(list) = list {
        state.branches = String::from_utf8_lossy(&list).lines().map(str::to_string).filter(|b| !b.is_empty()).collect();
    }
    if let Some(ignored) = ignored {
        state.ignored = parse_ignored(&String::from_utf8_lossy(&ignored));
    }
    state.ms = started.elapsed().as_millis() as u32;
    Some(state)
}

const MAX_IGNORED: usize = 400;

fn parse_ignored(text: &str) -> Vec<String> {
    text.split('\0').filter(|path| !path.is_empty() && *path != "./").take(MAX_IGNORED).map(str::to_string).collect()
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
            unsaved: false,
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
    fn ignored_paths_keep_the_mark_of_a_folder() {
        assert_eq!(parse_ignored("clip.mp4\0target/\0docs/shot.png\0"), ["clip.mp4", "target/", "docs/shot.png"]);
        assert!(parse_ignored("").is_empty());
        assert_eq!(parse_ignored(&"a\0".repeat(MAX_IGNORED + 50)).len(), MAX_IGNORED);
    }

    #[test]
    fn a_text_against_another_gives_the_hunks_git_would() {
        let old = "one\ntwo\nthree\nfour\nfive\nsix\n";
        let new = "one\nTWO\nthree\nfive\nsix\nseven\neight\r\n";
        let (hunks, insertions, deletions) = compare(old, new);
        assert_eq!(
            hunks,
            vec![
                Hunk { line: 1, count: 1, kind: HunkKind::Modified },
                Hunk { line: 3, count: 0, kind: HunkKind::Removed },
                Hunk { line: 5, count: 2, kind: HunkKind::Added },
            ]
        );
        assert_eq!((insertions, deletions), (3, 2));
        assert!(compare("a\r\nb\r\n", "a\nb").0.is_empty(), "line endings are not a change");
    }

    #[test]
    fn a_file_with_what_was_removed_and_the_rest_folded() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nb\nc\nd\nE\nf\ng\nh\ni\nj\nk\n";
        let all = unified(old, new, None).expect("they differ");
        assert_eq!(all.text, "a\nb\nc\nd\ne\nE\nf\ng\nh\ni\nj\nk");
        assert_eq!(all.rows[4], Row { kind: RowKind::Removed, line: 4 });
        assert_eq!(all.rows[5], Row { kind: RowKind::Added, line: 4 });
        assert_eq!(all.rows[11], Row { kind: RowKind::Added, line: 10 });
        assert_eq!((all.row(3), all.row(4), all.row(5), all.row(10), all.row(99)), (3, 5, 6, 11, 11));
        let rows = Some(all.rows.as_slice());
        assert_eq!((line_of(rows, 4), line_of(rows, 5), line_of(rows, 6)), (None, Some(4), Some(5)));

        let folded = unified(old, new, Some(1)).expect("they differ");
        assert_eq!(folded.text, "··· 3 unchanged lines ···\nd\ne\nE\nf\n··· 3 unchanged lines ···\nj\nk");
        assert_eq!(folded.rows[0], Row { kind: RowKind::Gap, line: 0 });
        assert_eq!((folded.row(1), folded.row(3), folded.row(4), folded.row(7)), (0, 1, 3, 5));
        assert_eq!((folded.row(6), folded.row(9), folded.row(10)), (5, 6, 7));
        let rows = Some(folded.rows.as_slice());
        assert_eq!((line_of(rows, 0), line_of(rows, 1), line_of(rows, 2)), (None, Some(3), None));
        assert_eq!(line_of(None, 7), Some(7));
        assert!(unified(old, old, Some(3)).is_none(), "nothing to show for a file that did not change");
    }

    #[test]
    fn old_and_new_next_to_each_other() {
        let split = side_by_side("a\nb\nc\n", "a\nB\nc\nd\n", None).expect("they differ");
        assert_eq!(split.text, "a\nb\nc\n\na\nB\nc\nd");
        assert_eq!(split.fold, 4, "one column for what it was, one for what it is");
        let kinds: Vec<RowKind> = split.rows.iter().map(|row| row.kind).collect();
        use RowKind::*;
        assert_eq!(kinds, [Before, Removed, Before, Blank, Same, Added, Same, Added]);
        assert_eq!((split.row(0), split.row(1), split.row(3)), (4, 5, 7));
        let rows = Some(split.rows.as_slice());
        assert_eq!((line_of(rows, 1), line_of(rows, 5), line_of(rows, 3)), (None, Some(1), None));
        assert_eq!(split.rows[1].line, 1, "a click on the old side lands on the line that replaced it");

        let folded = side_by_side("x\n1\n2\n3\n4\n5\n", "y\n1\n2\n3\n4\n5\n", Some(1)).expect("they differ");
        assert_eq!(folded.text, "x\n1\n··· 4 unchanged lines ···\ny\n1\n··· 4 unchanged lines ···");
        assert_eq!((folded.row(0), folded.row(1), folded.row(4)), (3, 4, 5));
    }

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
