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
    pub from: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Back {
    Commit,
    Staged,
    Lines,
}

impl FileChange {
    pub fn goes_back(&self) -> Option<Back> {
        let new = matches!(self.staged, Some(Change::Added | Change::Renamed));
        match (self.change, new) {
            (Change::Untracked, _) => None,
            (Change::Conflicted, _) | (_, false) => Some(Back::Commit),
            (_, true) => (self.unstaged.is_some() || self.unsaved).then_some(Back::Staged),
        }
    }
}

pub fn what_is_lost(files: &[(String, Back)]) -> Option<String> {
    let any = |back: Back| files.iter().any(|(_, goes)| *goes == back);
    let text = match (any(Back::Lines), any(Back::Commit), any(Back::Staged)) {
        (true, ..) => "These lines go back to how they were before the change.",
        (_, true, false) => "The files go back to how the last commit left them.",
        (_, true, true) => {
            "The files go back to how the last commit left them; the ones that are not in it, to what is staged."
        }
        (_, false, true) => "What is not in the last commit goes back to what is staged of it.",
        (_, false, false) => return None,
    };
    Some(format!("{text} This cannot be undone."))
}

pub fn head_texts(root: &Path, prefix: &str, paths: &[String]) -> Vec<Option<String>> {
    let named: Vec<String> = paths.iter().map(|path| format!("HEAD:{prefix}{path}")).collect();
    texts(root, &named)
}

#[derive(Clone, Debug)]
pub struct CommitFile {
    pub path: String,
    pub change: Change,
    pub before: Option<String>,
    pub after: Option<String>,
}

fn parse_names(listed: &str, prefix: &str) -> Vec<(String, Change)> {
    let mut parts = listed.split('\0');
    let mut found = Vec::new();
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        let change = match status.chars().next() {
            Some('A') => Change::Added,
            Some('D') => Change::Deleted,
            Some(_) => Change::Modified,
            None => continue,
        };
        if let Some(rel) = path.strip_prefix(prefix) {
            found.push((rel.to_string(), change));
        }
    }
    found
}

pub fn commit_files(root: &Path, prefix: &str, hash: &str, max: usize) -> (Vec<CommitFile>, usize) {
    let args =
        ["diff-tree", "--no-commit-id", "--name-status", "-r", "-z", "--root", "--diff-merges=first-parent", hash];
    let listed = git(root, &args).map(|out| String::from_utf8_lossy(&out).into_owned()).unwrap_or_default();
    let mut names = parse_names(&listed, prefix);
    let total = names.len();
    names.truncate(max);
    let named: Vec<String> = names
        .iter()
        .flat_map(|(path, _)| [format!("{hash}^:{prefix}{path}"), format!("{hash}:{prefix}{path}")])
        .collect();
    let mut found = texts(root, &named).into_iter();
    let files = names
        .into_iter()
        .map(|(path, change)| CommitFile {
            path,
            change,
            before: found.next().flatten(),
            after: found.next().flatten(),
        })
        .collect();
    (files, total)
}

pub fn texts(root: &Path, named: &[String]) -> Vec<Option<String>> {
    use std::io::{Read, Write};
    let mut texts = vec![None; named.len()];
    let mut c = command(root);
    c.args(["cat-file", "--batch"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child) = c.spawn() else { return texts };
    let asked: String = named.iter().map(|name| format!("{name}\n")).collect();
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

pub fn shows(lines: &[u32], rows: &[Row], line: u32) -> bool {
    let row = rows.get(row_of(lines, rows.len(), line) as usize);
    row.is_some_and(|row| row.kind != RowKind::Gap && row.line == line)
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
    pub locals: Vec<LocalBranch>,
    pub remote_branches: Vec<RemoteBranch>,
    /// The tags of the repository, the newest first.
    pub tags: Vec<String>,
    pub stashes: Vec<Stash>,
    pub changes: Vec<FileChange>,
    pub ignored: Vec<String>,
    pub truncated: bool,
    pub ms: u32,
    pub prefix: String,
    pub head: String,
    /// A merge, cherry-pick, revert or rebase that stopped and waits to be finished.
    pub in_progress: Option<InProgress>,
    /// Where HEAD has been lately, the newest first (the "undo" list).
    pub reflog: Vec<ReflogEntry>,
}

impl GitState {
    /// How many files are in conflict.
    pub fn conflicts(&self) -> usize {
        self.changes.iter().filter(|change| change.change == Change::Conflicted).count()
    }
}

/// What git is in the middle of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Merge,
    CherryPick,
    Revert,
    Rebase,
}

impl Operation {
    /// What the banner says it is doing.
    pub fn doing(self) -> &'static str {
        match self {
            Operation::Merge => "Merging",
            Operation::CherryPick => "Cherry-picking",
            Operation::Revert => "Reverting",
            Operation::Rebase => "Rebasing",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Operation::Merge => "Merge",
            Operation::CherryPick => "Cherry-pick",
            Operation::Revert => "Revert",
            Operation::Rebase => "Rebase",
        }
    }

    /// The git command that goes with `--continue`, `--skip` and `--abort`.
    fn command(self) -> &'static str {
        match self {
            Operation::Merge => "merge",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
            Operation::Rebase => "rebase",
        }
    }

    /// A merge cannot leave one of its steps out; the others can.
    pub fn can_skip(self) -> bool {
        self != Operation::Merge
    }
}

/// An operation that stopped, and how far it got when it has steps (a rebase: step 2 of 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InProgress {
    pub operation: Operation,
    pub step: Option<(u32, u32)>,
}

/// What can be done with an operation that stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carry {
    Continue,
    Skip,
    Abort,
}

/// Which side of a conflict a file is taken from, as git names them (`--ours` is the branch the
/// folder is on; in a rebase that is the branch being rebased onto).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

/// Tells what is in progress from the names that are in the git folder (`MERGE_HEAD`,
/// `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `rebase-merge`, `rebase-apply`). A rebase wins over the
/// others: it leaves `CHERRY_PICK_HEAD` while it replays a commit that conflicts. `git am` also
/// uses `rebase-apply`, and is not one of these.
fn in_progress_of(present: &[&str], step: Option<(u32, u32)>) -> Option<InProgress> {
    let has = |name: &str| present.contains(&name);
    let operation = if has("rebase-merge") || (has("rebase-apply") && !has("rebase-apply/applying")) {
        Operation::Rebase
    } else if has("MERGE_HEAD") {
        Operation::Merge
    } else if has("CHERRY_PICK_HEAD") {
        Operation::CherryPick
    } else if has("REVERT_HEAD") {
        Operation::Revert
    } else {
        return None;
    };
    Some(InProgress { operation, step: step.filter(|_| operation == Operation::Rebase) })
}

/// Looks in the git folder for an operation that stopped.
fn read_in_progress(git_dir: &Path) -> Option<InProgress> {
    let names =
        ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply", "rebase-apply/applying"];
    let present: Vec<&str> = names.iter().copied().filter(|name| git_dir.join(name).exists()).collect();
    let number = |file: &str| std::fs::read_to_string(git_dir.join(file)).ok()?.trim().parse::<u32>().ok();
    let step = if present.contains(&"rebase-merge") {
        number("rebase-merge/msgnum").zip(number("rebase-merge/end"))
    } else {
        number("rebase-apply/next").zip(number("rebase-apply/last"))
    };
    in_progress_of(&present, step)
}

/// A line of `git reflog`: where HEAD was, and what moved it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflogEntry {
    pub hash: String,
    /// `HEAD@{3}`.
    pub name: String,
    /// What was done, as git writes it: `commit: Add x`, `checkout: moving from a to b`.
    pub what: String,
    pub time: i64,
}

/// Reads `git reflog -n 30 --format=%H%x1f%gd%x1f%gs%x1f%ct`.
fn parse_reflog(text: &str) -> Vec<ReflogEntry> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let hash = parts.next().filter(|hash| !hash.is_empty())?;
            let name = parts.next()?;
            let what = parts.next()?;
            let time = parts.next()?.trim().parse().ok()?;
            Some(ReflogEntry { hash: hash.to_string(), name: name.to_string(), what: what.to_string(), time })
        })
        .collect()
}

/// A branch of this computer, with the remote branch it follows and how far apart they are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalBranch {
    pub name: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub current: bool,
}

/// A branch that lives on a remote such as GitHub, with the local branch that follows it, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteBranch {
    pub remote: String,
    pub name: String,
    pub local: Option<String>,
}

/// A set of changes put aside with stash: its number in `stash@{N}` and the words it was saved with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stash {
    pub index: u32,
    pub message: String,
}

/// Reads `git for-each-ref refs/tags --sort=-creatordate --format=%(refname:short)`.
fn parse_tags(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|tag| !tag.is_empty()).map(str::to_string).collect()
}

/// Reads `git stash list --format=%gd%x1f%s`, where `%gd` looks like `stash@{2}`.
fn parse_stashes(text: &str) -> Vec<Stash> {
    text.lines()
        .filter_map(|line| {
            let (name, message) = line.split_once('\u{1f}').unwrap_or((line, ""));
            let index = name.trim().strip_prefix("stash@{")?.strip_suffix('}')?.parse().ok()?;
            Some(Stash { index, message: message.trim().to_string() })
        })
        .collect()
}

/// Reads `git for-each-ref refs/heads --format=%(refname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(HEAD)`.
fn parse_locals(text: &str) -> Vec<LocalBranch> {
    let count = |track: &str, word: &str| -> u32 {
        let after = track.split(word).nth(1).unwrap_or("");
        after.trim_start().split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let name = parts.next().filter(|name| !name.is_empty())?;
            let upstream = parts.next().filter(|up| !up.is_empty()).map(str::to_string);
            let track = parts.next().unwrap_or("");
            let current = parts.next().is_some_and(|head| head.trim() == "*");
            Some(LocalBranch {
                name: name.to_string(),
                upstream,
                ahead: count(track, "ahead "),
                behind: count(track, "behind "),
                current,
            })
        })
        .collect()
}

/// Reads `git for-each-ref refs/remotes --format=%(refname:short)%1f%(symref)`, leaving out `origin/HEAD` and bare remote names.
fn parse_remote_branches(text: &str, remotes: &[String], locals: &[LocalBranch]) -> Vec<RemoteBranch> {
    let mut found = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('\u{1f}');
        let short = parts.next().unwrap_or("");
        if short.is_empty() || parts.next().is_some_and(|symref| !symref.is_empty()) {
            continue;
        }
        let Some(remote) = remotes
            .iter()
            .filter(|r| short.strip_prefix(r.as_str()).is_some_and(|rest| rest.starts_with('/')))
            .max_by_key(|r| r.len())
        else {
            continue;
        };
        let name = &short[remote.len() + 1..];
        if name.is_empty() || name == "HEAD" {
            continue;
        }
        let local = locals.iter().find(|l| l.upstream.as_deref() == Some(short)).map(|l| l.name.clone());
        found.push(RemoteBranch { remote: remote.clone(), name: name.to_string(), local });
    }
    found
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Stage,
    Unstage,
    Discard,
}

struct Cut {
    old: (u32, u32),
    new: (u32, u32),
    from: usize,
    to: usize,
}

fn pieces_of(diff: &[u8]) -> (usize, Vec<Cut>) {
    let mut found: Vec<Cut> = Vec::new();
    let (mut head, mut at) = (diff.len(), 0);
    while at < diff.len() {
        let end = diff[at..].iter().position(|b| *b == b'\n').map_or(diff.len(), |n| at + n + 1);
        if diff[at..end].starts_with(b"@@ ") {
            match found.last_mut() {
                Some(last) => last.to = at,
                None => head = at,
            }
            let text = String::from_utf8_lossy(&diff[at..end]).into_owned();
            let mut ranges = text.split(' ').skip(1).take(2).map(|range| {
                let mut numbers = range.get(1..).unwrap_or("").split(',').map(|n| n.parse::<u32>().unwrap_or(0));
                (numbers.next().unwrap_or(0), numbers.next().unwrap_or(1))
            });
            let (old, new) = (ranges.next().unwrap_or((0, 0)), ranges.next().unwrap_or((0, 0)));
            found.push(Cut { old, new, from: at, to: diff.len() });
        }
        at = end;
    }
    (head, found)
}

pub fn holds(first: u32, count: u32, line: u32) -> bool {
    match count {
        0 => line == first || line + 1 == first,
        _ => line >= first && line < first + count,
    }
}

fn in_worktree(line: u32, unstaged: &[Cut]) -> u32 {
    let mut moved = 0i64;
    for piece in unstaged {
        let (first, count) = piece.old;
        let last = if count == 0 { first } else { first + count - 1 };
        if last < line {
            moved += piece.new.1 as i64 - count as i64;
        } else if count > 0 && first <= line {
            return piece.new.0;
        }
    }
    (line as i64 + moved).max(0) as u32
}

fn alone(piece: &Cut, back: bool) -> String {
    let beside = |first: u32, count: u32, other: u32| match (count, other) {
        (0, 0) => first,
        (0, _) => first + 1,
        (_, 0) => first.saturating_sub(1),
        _ => first,
    };
    let (old, new) = match back {
        false => (piece.old.0, beside(piece.old.0, piece.old.1, piece.new.1)),
        true => (beside(piece.new.0, piece.new.1, piece.old.1), piece.new.0),
    };
    format!("@@ -{old},{} +{new},{} @@\n", piece.old.1, piece.new.1)
}

fn part(root: &Path, path: &str, line: u32, what: Part) -> Result<String, String> {
    use std::io::Write;
    let place = git(root, &["rev-parse", "--show-cdup", "--show-prefix"]).unwrap_or_default();
    let place = String::from_utf8_lossy(&place).into_owned();
    let mut place = place.lines();
    let top = root.join(place.next().unwrap_or_default().trim());
    let path = format!("{}{path}", place.next().unwrap_or_default().trim());
    let diff = |staged: bool| {
        let mut args =
            vec!["diff", "-U0", "--no-color", "--no-ext-diff", "--no-textconv", "--src-prefix=a/", "--dst-prefix=b/"];
        args.extend(staged.then_some("--cached"));
        args.extend(["--", path.as_str()]);
        git(&top, &args).unwrap_or_default()
    };
    let unstaged = diff(false);
    let (head, pieces) = pieces_of(&unstaged);
    let staged = if what == Part::Unstage { diff(true) } else { Vec::new() };
    let (staged_head, staged_pieces) = pieces_of(&staged);
    let at = |first: u32, count: u32| holds(if count == 0 { first } else { first.saturating_sub(1) }, count, line);
    let (text, head, found) = match what {
        Part::Unstage => {
            let here = |piece: &&Cut| at(in_worktree(piece.new.0, &pieces), piece.new.1);
            (&staged, staged_head, staged_pieces.iter().find(here))
        }
        _ => (&unstaged, head, pieces.iter().find(|piece| at(piece.new.0, piece.new.1))),
    };
    let Some(piece) = found else {
        return Err(match what {
            Part::Stage => "Nothing left to stage at this line.",
            Part::Unstage => "Nothing is staged at this line.",
            Part::Discard => "Nothing to discard at this line: what changed here is staged.",
        }
        .to_string());
    };
    let lines = &text[piece.from..piece.to];
    let body = lines.iter().position(|b| *b == b'\n').map_or(lines.len(), |end| end + 1);
    let mut patch = text[..head].to_vec();
    patch.extend_from_slice(alone(piece, what != Part::Stage).as_bytes());
    patch.extend_from_slice(&lines[body..]);
    let how: &[&str] = match what {
        Part::Stage => &["--cached"],
        Part::Unstage => &["--cached", "-R"],
        Part::Discard => &["-R"],
    };
    let mut c = command(&top);
    c.args(["apply", "--unidiff-zero", "--whitespace=nowarn"]).args(how).arg("-");
    c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("git could not be run: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&patch);
    }
    let out = child.wait_with_output().map_err(|e| format!("git could not be run: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let lines = piece.old.1.max(piece.new.1);
    let done = match what {
        Part::Stage => "Staged",
        Part::Unstage => "Unstaged",
        Part::Discard => "Discarded",
    };
    Ok(format!("{done} a change of {lines} {}.", if lines == 1 { "line" } else { "lines" }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitOp {
    Part {
        path: String,
        line: u32,
        what: Part,
    },
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit {
        message: String,
        all: bool,
        amend: bool,
        then: After,
    },
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
    /// Bring a branch that is only on a remote to a local branch of the same name, and switch to it.
    Track {
        remote: String,
        name: String,
    },
    /// Bring another branch into the current one.
    Merge(String),
    /// Replay the current branch on top of another one.
    Rebase(String),
    /// Make a branch, optionally starting from another one, and switch to it.
    NewBranch {
        name: String,
        from: Option<String>,
    },
    RenameBranch {
        from: String,
        to: String,
    },
    /// Delete a local branch that is already merged.
    DeleteBranch(String),
    /// Delete a branch on a remote such as GitHub.
    DeleteRemoteBranch {
        remote: String,
        name: String,
    },
    /// Send a local branch to a remote and make it follow it.
    PushBranch {
        remote: String,
        name: String,
    },
    NewTag {
        name: String,
        at: Option<String>,
    },
    DeleteTag(String),
    PushTag(String),
    /// Switch to a tag, leaving every branch (a detached head).
    CheckoutTag(String),
    StashApply(u32),
    StashPopAt(u32),
    StashDrop(u32),
    /// Stand on a commit with no branch (a detached head).
    CheckoutCommit(String),
    /// Copy a commit on top of the current branch.
    CherryPick(String),
    /// Make a new commit that undoes another one.
    Revert(String),
    /// Move the current branch to a commit.
    Reset {
        to: String,
        mode: ResetMode,
    },
    /// Take one side of a file in conflict, and mark it resolved.
    Take {
        path: String,
        side: Side,
    },
    /// Carry on with, skip a step of, or give up an operation that stopped.
    Carry {
        operation: Operation,
        how: Carry,
    },
    /// Rebase with the todo list the person arranged: `base` is the commit to start after (none: from
    /// the first commit of the history), `todo` the text of the list, `stash` puts the changes aside first.
    InteractiveRebase {
        base: Option<String>,
        todo: String,
        stash: bool,
    },
}

/// How far `git reset` goes: only the branch, the branch and the index, or the files too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetMode {
    pub fn word(self) -> &'static str {
        match self {
            ResetMode::Soft => "soft",
            ResetMode::Mixed => "mixed",
            ResetMode::Hard => "hard",
        }
    }
}

impl GitOp {
    pub fn commit(message: &str) -> GitOp {
        GitOp::Commit { message: message.to_string(), all: false, amend: false, then: After::Nothing }
    }
}

fn steps(op: &GitOp) -> Vec<Vec<String>> {
    let s = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<String>>();
    match op {
        GitOp::Part { .. } => Vec::new(),
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
        GitOp::Fetch => vec![s(&["fetch", "--all", "--prune"])],
        GitOp::Publish(branch) => vec![s(&["push", "-u", "origin", branch])],
        GitOp::Discard(_) | GitOp::DiscardAll => Vec::new(),
        GitOp::Stash => vec![s(&["stash", "push", "--include-untracked"])],
        GitOp::StashPop => vec![s(&["stash", "pop"])],
        GitOp::Switch(branch) => vec![s(&["switch", branch])],
        GitOp::Track { remote, name } => vec![s(&["switch", "-c", name, "--track", &format!("{remote}/{name}")])],
        GitOp::Merge(branch) => vec![s(&["merge", branch])],
        GitOp::Rebase(branch) => vec![s(&["rebase", branch])],
        GitOp::NewBranch { name, from } => {
            let mut args = s(&["switch", "-c", name]);
            args.extend(from.iter().cloned());
            vec![args]
        }
        GitOp::RenameBranch { from, to } => vec![s(&["branch", "-m", from, to])],
        GitOp::DeleteBranch(name) => vec![s(&["branch", "-d", name])],
        GitOp::DeleteRemoteBranch { remote, name } => vec![s(&["push", remote, "--delete", name])],
        GitOp::PushBranch { remote, name } => vec![s(&["push", "-u", remote, name])],
        GitOp::NewTag { name, at } => {
            let mut args = s(&["tag", name]);
            args.extend(at.iter().cloned());
            vec![args]
        }
        GitOp::DeleteTag(name) => vec![s(&["tag", "-d", name])],
        GitOp::PushTag(name) => vec![s(&["push", "origin", name])],
        GitOp::CheckoutTag(name) => vec![s(&["switch", "--detach", name])],
        GitOp::StashApply(n) => vec![s(&["stash", "apply", &format!("stash@{{{n}}}")])],
        GitOp::StashPopAt(n) => vec![s(&["stash", "pop", &format!("stash@{{{n}}}")])],
        GitOp::StashDrop(n) => vec![s(&["stash", "drop", &format!("stash@{{{n}}}")])],
        GitOp::CheckoutCommit(hash) => vec![s(&["switch", "--detach", hash])],
        GitOp::CherryPick(hash) => vec![s(&["cherry-pick", hash])],
        GitOp::Revert(hash) => vec![s(&["revert", "--no-edit", hash])],
        GitOp::Reset { to, mode } => vec![s(&["reset", &format!("--{}", mode.word()), to])],
        GitOp::Take { path, side } => {
            let side = if *side == Side::Ours { "--ours" } else { "--theirs" };
            vec![s(&["checkout", side, "--", path]), s(&["add", "--", path])]
        }
        GitOp::Carry { operation, how } => match (operation, how) {
            (Operation::Merge, Carry::Continue) => vec![s(&["commit", "--no-edit"])],
            (Operation::Merge, Carry::Skip) => Vec::new(),
            (Operation::Merge, Carry::Abort) => vec![s(&["merge", "--abort"])],
            (_, how) => {
                let flag = match how {
                    Carry::Continue => "--continue",
                    Carry::Skip => "--skip",
                    Carry::Abort => "--abort",
                };
                vec![s(&[operation.command(), flag])]
            }
        },
        GitOp::InteractiveRebase { .. } => Vec::new(),
    }
}

/// Quotes a text for `sh` so that it is read back as it is: in single quotes, with each single
/// quote closed, escaped and opened again.
pub fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Whether some file is still in conflict.
fn has_conflicts(root: &Path) -> bool {
    git(root, &["diff", "--name-only", "--diff-filter=U"]).is_some_and(|out| !out.iter().all(u8::is_ascii_whitespace))
}

/// The stretch of history an interactive rebase replays.
pub struct RebaseRange {
    /// The commit it starts after; none when it starts at the first commit of the history.
    pub base: Option<String>,
    /// `(hash, subject)`, oldest first.
    pub commits: Vec<(String, String)>,
}

/// The commits an interactive rebase from `hash` replays, oldest first as `(hash, subject)`, and
/// the commit it starts after (none when `hash` is the first commit of the history). Merges are
/// left out, as a rebase does. None when `hash` is not behind HEAD or is a merge.
pub fn rebase_range(root: &Path, hash: &str) -> Option<RebaseRange> {
    let text = |out: Vec<u8>| String::from_utf8_lossy(&out).into_owned();
    git(root, &["merge-base", "--is-ancestor", hash, "HEAD"])?;
    let parents = git(root, &["rev-list", "--parents", "-n", "1", hash]).map(text)?;
    let mut words = parents.split_whitespace().skip(1);
    let base = words.next().map(str::to_string);
    if words.next().is_some() {
        return None;
    }
    let range = base.as_ref().map(|base| format!("{base}..HEAD")).unwrap_or_else(|| "HEAD".to_string());
    let args = ["log", "--no-merges", "--topo-order", "--reverse", "--format=%H%x1f%s", range.as_str()];
    let listed = git(root, &args).map(text)?;
    let commits = listed
        .lines()
        .filter_map(|line| line.split_once('\u{1f}'))
        .map(|(hash, subject)| (hash.to_string(), subject.to_string()))
        .collect();
    Some(RebaseRange { base, commits })
}

/// Runs a rebase with the todo list given. The list is written to a file and the sequence editor
/// of this one run is `cp` of that file over the list git made, so no editor ever opens.
fn interactive_rebase(root: &Path, base: Option<&str>, todo: &str, stash: bool) -> Result<String, String> {
    if stash {
        run_steps(root, &[vec!["stash".to_string(), "push".to_string(), "--include-untracked".to_string()]], false)?;
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let file = std::env::temp_dir().join(format!("code-canvas-rebase-{}-{nanos}.txt", std::process::id()));
    std::fs::write(&file, todo).map_err(|e| format!("The list of the rebase could not be written: {e}"))?;
    let editor = format!("cp {}", sh_quote(&file.to_string_lossy().replace('\\', "/")));
    let mut c = command(root);
    c.env("GIT_SEQUENCE_EDITOR", editor).env("GIT_EDITOR", "true");
    c.args(["rebase", "-i"]);
    match base {
        Some(base) => c.arg(base),
        None => c.arg("--root"),
    };
    let out = c.stdin(Stdio::null()).output();
    let _ = std::fs::remove_file(&file);
    let out = out.map_err(|e| format!("git could not be run: {e}"))?;
    if out.status.success() {
        return Ok(String::new());
    }
    Err(failure(root, &out, true))
}

/// What to tell when git failed: a conflict that stopped the operation is not an error to read.
fn failure(root: &Path, out: &std::process::Output, may_stop: bool) -> String {
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if may_stop && has_conflicts(root) {
        return "Stopped on a conflict: fix the files marked in red, then Continue.".to_string();
    }
    let err = text(&out.stderr);
    if err.is_empty() {
        text(&out.stdout)
    } else {
        err
    }
}

fn run_steps(root: &Path, steps: &[Vec<String>], may_stop: bool) -> Result<String, String> {
    let mut said = String::new();
    for args in steps {
        let mut c = command(root);
        let out = c.args(args).stdin(Stdio::null()).output().map_err(|e| format!("git could not be run: {e}"))?;
        if !out.status.success() {
            return Err(failure(root, &out, may_stop));
        }
        if said.is_empty() {
            said = String::from_utf8_lossy(&out.stdout).trim().lines().next().unwrap_or_default().to_string();
        }
    }
    Ok(said)
}

fn checkout(root: &Path, from: Option<&str>, paths: &[&str]) -> Result<(), String> {
    use std::io::Write;
    if paths.is_empty() {
        return Ok(());
    }
    let mut c = command(root);
    c.arg("checkout").args(from).args(["--pathspec-from-file=-", "--pathspec-file-nul"]);
    c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("git could not be run: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(paths.join("\0").as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("git could not be run: {e}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

fn discard(root: &Path, only: Option<&str>) -> Result<String, String> {
    let text = |out: Vec<u8>| String::from_utf8_lossy(&out).into_owned();
    let status = git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=no", "--", only.unwrap_or(".")]);
    let status = status.map(text).ok_or("git could not say what changed here.")?;
    let prefix = git(root, &["rev-parse", "--show-prefix"]).map(text).unwrap_or_default();
    let changes = parse_status(&status, prefix.trim()).changes;
    let mut listed = vec!["ls-tree", "-r", "-z", "--name-only", "HEAD"];
    listed.extend(only.iter().flat_map(|path| ["--", *path]));
    let committed = git(root, &listed).map(text).unwrap_or_default();
    let committed: std::collections::HashSet<&str> = committed.split('\0').filter(|path| !path.is_empty()).collect();
    let (mut to_commit, mut to_staged) = (Vec::new(), Vec::new());
    for change in &changes {
        match (committed.contains(change.path.as_str()), change.change, change.unstaged) {
            (true, ..) => to_commit.push(change.path.as_str()),
            (false, Change::Conflicted, _) | (false, _, None) => {}
            (false, _, Some(_)) => to_staged.push(change.path.as_str()),
        }
    }
    checkout(root, Some("HEAD"), &to_commit)?;
    checkout(root, None, &to_staged)?;
    Ok(String::new())
}

pub fn run(root: &Path, op: &GitOp) -> Result<String, String> {
    match op {
        GitOp::Part { path, line, what } => return part(root, path, *line, *what),
        GitOp::Discard(path) => return discard(root, Some(path)),
        GitOp::DiscardAll => return discard(root, None),
        _ => {}
    }
    let op = match op {
        GitOp::Track { name, .. }
            if git(root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_some() =>
        {
            GitOp::Switch(name.clone())
        }
        other => other.clone(),
    };
    if let GitOp::InteractiveRebase { base, todo, stash } = &op {
        return interactive_rebase(root, base.as_deref(), todo, *stash);
    }
    let may_stop = matches!(
        op,
        GitOp::Merge(_)
            | GitOp::Rebase(_)
            | GitOp::CherryPick(_)
            | GitOp::Revert(_)
            | GitOp::Carry { how: Carry::Continue | Carry::Skip, .. }
    );
    run_steps(root, &steps(&op), may_stop)
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
    // Nothing the app runs may open an editor or a pager: an editor is `true` (keep the message git
    // made), and the rebase that needs a list sets its own sequence editor.
    c.env("GIT_OPTIONAL_LOCKS", "0").env("GIT_EDITOR", "true").env("GIT_SEQUENCE_EDITOR", "true");
    c.env("GIT_PAGER", "cat").arg("-C").arg(root).args(["-c", "core.quotepath=off", "--literal-pathspecs"]);
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
    let (status, prefix, diff, log, list, ignored, remote_refs, remote_names, tag_refs, stash_list, git_dir, reflog) =
        std::thread::scope(|s| {
            let status = s.spawn(|| {
                git(root, &["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=all", "--", "."])
            });
            let prefix = s.spawn(|| git(root, &["rev-parse", "--show-prefix"]));
            let diff =
                s.spawn(|| git(root, &["diff", "HEAD", "-U0", "--no-color", "--no-ext-diff", "--relative", "--", "."]));
            let log = s.spawn(|| git(root, &["log", "-n", "40", "--format=%H%x1f%an%x1f%at%x1f%D%x1f%s"]));
            let list = s.spawn(|| {
                git(
                    root,
                    &[
                        "for-each-ref",
                        "refs/heads",
                        "--format=%(refname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(HEAD)",
                    ],
                )
            });
            let remote_refs =
                s.spawn(|| git(root, &["for-each-ref", "refs/remotes", "--format=%(refname:short)%1f%(symref)"]));
            let remote_names = s.spawn(|| git(root, &["remote"]));
            let tag_refs = s.spawn(|| {
                git(root, &["for-each-ref", "refs/tags", "--sort=-creatordate", "--format=%(refname:short)"])
            });
            let stash_list = s.spawn(|| git(root, &["stash", "list", "--format=%gd%x1f%s"]));
            let git_dir = s.spawn(|| git(root, &["rev-parse", "--git-dir"]));
            let reflog = s.spawn(|| git(root, &["reflog", "-n", "30", "--format=%H%x1f%gd%x1f%gs%x1f%ct"]));
            let ignored = s.spawn(|| {
                git(root, &["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z", "--", "."])
            });
            let done = |h: std::thread::ScopedJoinHandle<Option<Vec<u8>>>| h.join().ok().flatten();
            (
                done(status),
                done(prefix),
                done(diff),
                done(log),
                done(list),
                done(ignored),
                done(remote_refs),
                done(remote_names),
                done(tag_refs),
                done(stash_list),
                done(git_dir),
                done(reflog),
            )
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
        state.locals = parse_locals(&String::from_utf8_lossy(&list));
        state.branches = state.locals.iter().map(|l| l.name.clone()).collect();
    }
    if let (Some(refs), Some(names)) = (remote_refs, remote_names) {
        let names: Vec<String> = String::from_utf8_lossy(&names).lines().map(|n| n.trim().to_string()).collect();
        state.remote_branches = parse_remote_branches(&String::from_utf8_lossy(&refs), &names, &state.locals);
    }
    if let Some(tags) = tag_refs {
        state.tags = parse_tags(&String::from_utf8_lossy(&tags));
    }
    if let Some(dir) = git_dir {
        let dir = String::from_utf8_lossy(&dir).trim().to_string();
        state.in_progress = read_in_progress(&root.join(dir));
    }
    if let Some(reflog) = reflog {
        state.reflog = parse_reflog(&String::from_utf8_lossy(&reflog));
    }
    if let Some(stashes) = stash_list {
        state.stashes = parse_stashes(&String::from_utf8_lossy(&stashes));
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
        let from = if x == b'R' || x == b'C' { entries.next() } else { None };
        let from = from.and_then(|old| old.strip_prefix(prefix)).map(str::to_string);
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
            from,
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

/// The day and minute of a moment as `2026-11-18 14:03 UTC`.
pub fn date_of(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86400), seconds.rem_euclid(86400));
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", rest / 3600, rest % 3600 / 60)
}

/// A commit of the whole history, with the commits it grew from and the branches and tags on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCommit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    /// The names git decorates it with: `HEAD -> main`, `HEAD`, `tag: v1`, `origin/main`, `feature`.
    pub refs: Vec<String>,
    pub subject: String,
}

impl HistoryCommit {
    /// The plain commit the canvas knows how to show.
    pub fn commit(&self) -> Commit {
        Commit { hash: self.hash.clone(), author: self.author.clone(), time: self.time, subject: self.subject.clone() }
    }
}

/// What the search box of the commit list looks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Search {
    #[default]
    None,
    Message(String),
    Author(String),
    /// A whole hash or its first letters.
    Hash(String),
    /// Commits that changed a file.
    File(String),
}

/// Which commits the list shows: every branch by default, or only the chosen ones, minus the hidden
/// ones, narrowed by a search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    pub only: Vec<String>,
    pub hidden: Vec<String>,
    pub search: Search,
}

const HISTORY_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s";
/// A search by hash reads this many commits and keeps the ones whose hash starts with the text.
const HASH_SEARCH_READS: usize = 100_000;

fn history_args(query: &HistoryQuery, skip: usize, count: usize) -> Vec<String> {
    let mut args: Vec<String> = ["log", "--topo-order", HISTORY_FORMAT].iter().map(|a| a.to_string()).collect();
    let shown: Vec<&String> = query.only.iter().filter(|name| !query.hidden.contains(name)).collect();
    if query.only.is_empty() {
        for name in &query.hidden {
            args.push(format!("--exclude=refs/heads/{name}"));
            args.push(format!("--exclude=refs/remotes/*/{name}"));
            args.push(format!("--exclude=refs/remotes/{name}"));
        }
        args.push("--exclude=refs/stash".to_string());
        args.push("--all".to_string());
    }
    match &query.search {
        Search::Hash(_) => args.extend(["-n".to_string(), HASH_SEARCH_READS.to_string()]),
        _ => args.extend(["--skip".to_string(), skip.to_string(), "-n".to_string(), count.to_string()]),
    }
    match &query.search {
        Search::Message(t) => args.extend([format!("--grep={t}"), "-i".to_string()]),
        Search::Author(t) => args.extend([format!("--author={t}"), "-i".to_string()]),
        _ => {}
    }
    args.extend(shown.into_iter().cloned());
    args.push("--".to_string());
    if let Search::File(path) = &query.search {
        args.push(path.clone());
    }
    args
}

fn parse_history(text: &str) -> Vec<HistoryCommit> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(7, '\u{1f}');
            let hash = parts.next().filter(|h| !h.is_empty())?;
            let parents = parts.next()?.split(' ').filter(|p| !p.is_empty()).map(str::to_string).collect();
            let (author, email, time, refs) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            let refs = refs
                .split(", ")
                .map(str::trim)
                .filter(|r| !r.is_empty() && !r.ends_with("/HEAD"))
                .map(str::to_string)
                .collect();
            Some(HistoryCommit {
                hash: hash.to_string(),
                parents,
                author: author.to_string(),
                email: email.to_string(),
                time: time.parse().ok()?,
                refs,
                subject: parts.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Reads a page of the history: `count` commits after the first `skip` of what the query shows,
/// children always before their parents (topological order).
pub fn history(root: &Path, query: &HistoryQuery, skip: usize, count: usize) -> Vec<HistoryCommit> {
    let args = history_args(query, skip, count);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let found = git(root, &args).map(|out| parse_history(&String::from_utf8_lossy(&out))).unwrap_or_default();
    match &query.search {
        Search::Hash(t) => {
            let t = t.trim().to_lowercase();
            found.into_iter().filter(|c| c.hash.starts_with(&t)).skip(skip).take(count).collect()
        }
        _ => found,
    }
}

/// Where a commit sits in the picture of the history and which lines run through its row.
///
/// Lanes are columns, counted from the left. `above` and `below` hold, for every lane, the color of
/// the line that comes in from the top of the row or leaves through its bottom (none when the lane
/// is empty there). The dot is in `lane`. `joins` are lanes that end in the dot (their lines come
/// from the top, slanting in) and `forks` are lanes the dot sends a line to (slanting out to the
/// bottom): a merge opens a fork for each parent after the first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    pub lane: usize,
    pub color: usize,
    pub above: Vec<Option<usize>>,
    pub below: Vec<Option<usize>>,
    pub joins: Vec<(usize, usize)>,
    pub forks: Vec<(usize, usize)>,
}

impl GraphRow {
    /// How many lanes the row needs room for.
    pub fn width(&self) -> usize {
        let drawn = |lanes: &[(usize, usize)]| lanes.iter().map(|(lane, _)| lane + 1).max().unwrap_or(0);
        (self.lane + 1).max(self.above.len()).max(self.below.len()).max(drawn(&self.joins)).max(drawn(&self.forks))
    }
}

/// Gives the commits of a history, fed in order and in pages, their lanes. Every lane waits for
/// one commit (the parent of the last commit that was in it); a commit takes the lane that waits
/// for it. A color belongs to a lane from where it starts to where it ends, and colors are counted
/// up: the picture takes them modulo its palette.
#[derive(Clone, Debug, Default)]
pub struct Lanes {
    waiting: Vec<Option<(String, usize)>>,
    colors: usize,
}

impl Lanes {
    /// How many lanes are waiting for a commit that has not come yet.
    #[cfg(test)]
    pub fn open(&self) -> usize {
        self.waiting.iter().flatten().count()
    }

    fn free(&mut self) -> usize {
        match self.waiting.iter().position(Option::is_none) {
            Some(at) => at,
            None => {
                self.waiting.push(None);
                self.waiting.len() - 1
            }
        }
    }

    fn next_color(&mut self) -> usize {
        self.colors += 1;
        self.colors - 1
    }

    fn colors(&self) -> Vec<Option<usize>> {
        let mut seen: Vec<Option<usize>> = self.waiting.iter().map(|w| w.as_ref().map(|(_, color)| *color)).collect();
        while seen.last() == Some(&None) {
            seen.pop();
        }
        seen
    }

    pub fn push(&mut self, commit: &HistoryCommit) -> GraphRow {
        let coming: Vec<usize> = (0..self.waiting.len())
            .filter(|i| self.waiting[*i].as_ref().is_some_and(|(hash, _)| *hash == commit.hash))
            .collect();
        let (lane, color) = match coming.first() {
            Some(at) => (*at, self.waiting[*at].as_ref().map_or(0, |(_, color)| *color)),
            None => {
                let (lane, color) = (self.free(), self.next_color());
                (lane, color)
            }
        };
        let above = self.colors();
        let mut joins = Vec::new();
        for other in coming.iter().skip(1) {
            if let Some((_, other_color)) = self.waiting[*other].take() {
                joins.push((*other, other_color));
            }
        }
        let mut parents = commit.parents.iter();
        self.waiting[lane] = parents.next().map(|first| (first.clone(), color));
        let mut forks: Vec<(usize, usize)> = Vec::new();
        for parent in parents {
            let taken = self.waiting.iter().position(|w| w.as_ref().is_some_and(|(hash, _)| hash == parent));
            match taken {
                Some(at) => {
                    let color = self.waiting[at].as_ref().map_or(0, |(_, color)| *color);
                    if !forks.iter().any(|(to, _)| *to == at) {
                        forks.push((at, color));
                    }
                }
                None => {
                    let (at, color) = (self.free(), self.next_color());
                    self.waiting[at] = Some((parent.clone(), color));
                    forks.push((at, color));
                }
            }
        }
        GraphRow { lane, color, above, below: self.colors(), joins, forks }
    }
}

pub fn head_of(folder: &std::path::Path) -> Option<String> {
    let dot = folder.join(".git");
    let dir = match dot.is_file() {
        true => {
            let pointed = std::fs::read_to_string(&dot).ok()?;
            let at = std::path::PathBuf::from(pointed.trim().strip_prefix("gitdir:")?.trim());
            if at.is_absolute() {
                at
            } else {
                folder.join(at)
            }
        }
        false => dot,
    };
    head_named(&std::fs::read_to_string(dir.join("HEAD")).ok()?)
}

pub fn head_named(head: &str) -> Option<String> {
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(name) => Some(name.trim().trim_start_matches("refs/heads/").to_string()).filter(|name| !name.is_empty()),
        None if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) => Some(head[..7].to_string()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_branch_is_read_from_head() {
        assert_eq!(head_named("ref: refs/heads/develop\n").as_deref(), Some("develop"));
        assert_eq!(head_named("ref: refs/heads/feature/x").as_deref(), Some("feature/x"));
        assert_eq!(
            head_named("3b5d3698c0ffee1234567890abcdef1234567890").as_deref(),
            Some("3b5d369"),
            "a detached head"
        );
        assert_eq!(head_named("garbage"), None);
    }

    #[test]
    fn the_files_of_a_commit_are_the_ones_under_the_folder() {
        let listed = "M\0app/src/a.rs\0A\0app/new file.rs\0D\0docs/old.md\0";
        assert_eq!(
            parse_names(listed, "app/"),
            [("src/a.rs".to_string(), Change::Modified), ("new file.rs".to_string(), Change::Added)]
        );
        assert_eq!(parse_names(listed, "").len(), 3);
        assert!(parse_names("", "").is_empty());
    }

    #[test]
    fn a_diff_is_cut_in_its_changes() {
        let diff = b"diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -2 +2 @@ fn a()\n-two\n+TWO\n@@ -4 +3,0 @@\n-four\n@@ -6,0 +6,2 @@\n+x\n+y\n";
        let (head, pieces) = pieces_of(diff);
        assert!(diff[..head].ends_with(b"+++ b/f\n"));
        let ranges: Vec<_> = pieces.iter().map(|piece| (piece.old, piece.new)).collect();
        assert_eq!(ranges, [((2, 1), (2, 1)), ((4, 1), (3, 0)), ((6, 0), (6, 2))]);
        assert_eq!(&diff[pieces[1].from..pieces[1].to], b"@@ -4 +3,0 @@\n-four\n");
        assert!(holds(1, 1, 1) && !holds(1, 1, 2));
        assert!(holds(3, 0, 3) && holds(3, 0, 2) && !holds(3, 0, 4));
        assert_eq!(in_worktree(1, &pieces), 1);
        assert_eq!(in_worktree(2, &pieces), 2);
        assert_eq!(in_worktree(5, &pieces), 4);
        assert_eq!(in_worktree(7, &pieces), 8);
    }

    #[test]
    fn one_change_of_a_file_is_staged_unstaged_and_discarded() {
        let dir = std::env::temp_dir().join(format!("code-canvas-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let run = |args: &[&str]| git(&dir, args).map(|out| String::from_utf8_lossy(&out).into_owned());
        if run(&["init", "-q"]).is_none() {
            return;
        }
        run(&["config", "core.autocrlf", "false"]).unwrap();
        let file = dir.join("sub").join("f.txt");
        std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\nsix\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(&file, "zero\none\ntwo\nthree\nfive\nSIX\n").unwrap();
        let root = dir.join("sub");
        let op = |line: u32, what: Part| super::run(&root, &GitOp::Part { path: "f.txt".into(), line, what });
        assert_eq!(op(5, Part::Stage).unwrap(), "Staged a change of 1 line.");
        let staged = run(&["diff", "--cached", "-U0"]).unwrap();
        assert!(staged.contains("+SIX") && !staged.contains("+zero") && !staged.contains("-four"), "{staged}");
        assert!(op(5, Part::Stage).is_err());
        assert!(op(2, Part::Unstage).is_err());
        op(4, Part::Stage).unwrap();
        assert!(run(&["diff", "--cached", "-U0"]).unwrap().contains("-four"));
        op(5, Part::Unstage).unwrap();
        let staged = run(&["diff", "--cached", "-U0"]).unwrap();
        assert!(!staged.contains("+SIX") && staged.contains("-four"), "{staged}");
        assert_eq!(op(0, Part::Discard).unwrap(), "Discarded a change of 1 line.");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\nthree\nfive\nSIX\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_alone_gets_the_header_of_a_diff_of_only_that_change() {
        let cut = |old, new| Cut { old, new, from: 0, to: 0 };
        assert_eq!(alone(&cut((8, 0), (13, 1)), false), "@@ -8,0 +9,1 @@\n", "added below four lines not staged");
        assert_eq!(alone(&cut((8, 1), (11, 0)), false), "@@ -8,1 +7,0 @@\n");
        assert_eq!(alone(&cut((10, 1), (14, 1)), false), "@@ -10,1 +10,1 @@\n");
        assert_eq!(alone(&cut((10, 2), (14, 5)), false), "@@ -10,2 +10,5 @@\n");
        assert_eq!(alone(&cut((1, 2), (0, 0)), false), "@@ -1,2 +0,0 @@\n");
        assert_eq!(alone(&cut((0, 0), (1, 4)), false), "@@ -0,0 +1,4 @@\n");
        assert_eq!(alone(&cut((8, 1), (11, 0)), true), "@@ -12,1 +11,0 @@\n", "taken back: it returns after line 11");
        assert_eq!(alone(&cut((8, 0), (13, 1)), true), "@@ -12,0 +13,1 @@\n");
        assert_eq!(alone(&cut((10, 1), (14, 1)), true), "@@ -14,1 +14,1 @@\n");
        assert_eq!(alone(&cut((0, 0), (1, 4)), true), "@@ -0,0 +1,4 @@\n");
        assert_eq!(alone(&cut((1, 2), (0, 0)), true), "@@ -1,2 +0,0 @@\n");
    }

    fn repository(name: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("code-canvas-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"])?;
        for (key, value) in [("core.autocrlf", "false"), ("user.name", "t"), ("user.email", "t@t")] {
            git(&dir, &["config", key, value]).unwrap();
        }
        Some(dir)
    }

    fn said(dir: &Path, args: &[&str]) -> String {
        String::from_utf8_lossy(&git(dir, args).unwrap()).into_owned()
    }

    #[test]
    fn one_change_lands_where_it_is_whatever_is_above_it() {
        let Some(dir) = repository("alone") else { return };
        let top = "// one\n// two\n// three\n// four\n";
        let lines: String = (1..=12).map(|n| format!("line {n}\n")).collect();
        let same: String = "abcde".chars().map(|name| format!("fn {name}() -> u32 {{\n    0\n}}\n\n")).collect();
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        for name in ["insert.txt", "delete.txt", "back.txt", "lost.txt"] {
            write(name, &lines);
        }
        write("same.rs", &same);
        said(&dir, &["add", "."]);
        said(&dir, &["commit", "-q", "-m", "first"]);
        let inserted = lines.replace("line 8\n", "line 8\ninserted\n");
        let deleted = lines.replace("line 8\n", "");
        let three = same.replace("fn c() -> u32 {\n    0\n", "fn c() -> u32 {\n    3\n");
        write("insert.txt", &format!("{top}{inserted}"));
        write("delete.txt", &format!("{top}{deleted}"));
        write("same.rs", &format!("{top}{three}"));
        write("back.txt", &format!("{top}{deleted}"));
        write("lost.txt", &format!("{top}{deleted}"));
        let op = |path: &str, line: u32, what: Part| super::run(&dir, &GitOp::Part { path: path.into(), line, what });
        let staged = |path: &str| said(&dir, &["show", &format!(":{path}")]);
        let on_disk = |path: &str| std::fs::read_to_string(dir.join(path)).unwrap();

        op("insert.txt", 12, Part::Stage).unwrap();
        assert_eq!(staged("insert.txt"), inserted, "a line added under lines that are not staged");
        op("same.rs", 13, Part::Stage).unwrap();
        assert_eq!(staged("same.rs"), three, "a line among others with the same text");
        op("delete.txt", 11, Part::Stage).unwrap();
        assert_eq!(staged("delete.txt"), deleted, "a line removed under lines that are not staged");

        said(&dir, &["add", "back.txt"]);
        op("back.txt", 11, Part::Unstage).unwrap();
        assert_eq!(staged("back.txt"), format!("{top}{lines}"), "a removal unstaged under staged lines");
        said(&dir, &["add", "same.rs"]);
        op("same.rs", 13, Part::Unstage).unwrap();
        assert_eq!(staged("same.rs"), format!("{top}{same}"), "a staged line among others with the same text");

        op("lost.txt", 11, Part::Discard).unwrap();
        assert_eq!(on_disk("lost.txt"), format!("{top}{lines}"), "a removed line discarded under other changes");
        op("insert.txt", 12, Part::Unstage).unwrap();
        op("insert.txt", 12, Part::Discard).unwrap();
        assert_eq!(on_disk("insert.txt"), format!("{top}{lines}"), "an added line discarded under other changes");
        op("same.rs", 13, Part::Discard).unwrap();
        assert_eq!(on_disk("same.rs"), format!("{top}{same}"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discarding_brings_a_file_back_to_the_commit_or_to_what_is_staged() {
        let Some(dir) = repository("discard") else { return };
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        let on_disk = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        write("a.txt", "a\n");
        assert_eq!(super::run(&dir, &GitOp::DiscardAll), Ok(String::new()), "nothing is tracked yet");
        said(&dir, &["add", "a.txt"]);
        write("a.txt", "a\nmore\n");
        super::run(&dir, &GitOp::DiscardAll).unwrap();
        assert_eq!(on_disk("a.txt"), "a\n", "with no commit a file goes back to what is staged");

        for (name, text) in [("b.txt", "b\n"), ("gone.txt", "gone\n"), ("sub/old.txt", "one\ntwo\nthree\n")] {
            write(name, text);
        }
        said(&dir, &["add", "."]);
        said(&dir, &["commit", "-q", "-m", "first"]);
        write("a.txt", "a\nstaged\n");
        write("added.txt", "added\n");
        write("only staged.txt", "staged\n");
        said(&dir, &["mv", "sub/old.txt", "sub/new.txt"]);
        said(&dir, &["rm", "-q", "gone.txt"]);
        said(&dir, &["add", "."]);
        write("a.txt", "a\nstaged\nnot staged\n");
        write("b.txt", "B\n");
        write("added.txt", "added\nand more\n");
        write("sub/new.txt", "one\nTWO\nthree\n");
        write("new.txt", "nobody knows me\n");

        super::run(&dir, &GitOp::Discard("added.txt".into())).unwrap();
        assert_eq!(on_disk("added.txt"), "added\n");
        write("added.txt", "added\nagain\n");
        super::run(&dir.join("sub"), &GitOp::Discard("new.txt".into())).unwrap();
        assert_eq!(on_disk("sub/new.txt"), "one\ntwo\nthree\n");
        write("sub/new.txt", "one\nTWO\nthree\n");
        assert_eq!(said(&dir, &["status", "--porcelain"]), "MM a.txt\nAM added.txt\n M b.txt\nD  gone.txt\nA  \"only staged.txt\"\nRM sub/old.txt -> sub/new.txt\n?? new.txt\n");

        let state = parse_status(&said(&dir, &["status", "--porcelain=v1", "-z"]), "");
        let back: Vec<_> = state.changes.iter().map(|change| (change.path.as_str(), change.goes_back())).collect();
        assert_eq!(
            back,
            [
                ("a.txt", Some(Back::Commit)),
                ("added.txt", Some(Back::Staged)),
                ("b.txt", Some(Back::Commit)),
                ("gone.txt", Some(Back::Commit)),
                ("new.txt", None),
                ("only staged.txt", None),
                ("sub/new.txt", Some(Back::Staged)),
            ]
        );
        assert_eq!(state.change("sub/new.txt").and_then(|change| change.from.as_deref()), Some("sub/old.txt"));

        super::run(&dir, &GitOp::DiscardAll).unwrap();
        assert_eq!(
            said(&dir, &["status", "--porcelain"]),
            "A  added.txt\nA  \"only staged.txt\"\nR  sub/old.txt -> sub/new.txt\n?? new.txt\n"
        );
        assert_eq!(
            (on_disk("a.txt"), on_disk("b.txt"), on_disk("gone.txt")),
            ("a\n".into(), "b\n".into(), "gone\n".into())
        );
        assert_eq!((on_disk("added.txt"), on_disk("sub/new.txt")), ("added\n".into(), "one\ntwo\nthree\n".into()));
        assert_eq!(on_disk("new.txt"), "nobody knows me\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        let seen = |line| shows(&folded.lines, &folded.rows, line);
        assert!(!seen(0) && !seen(1) && !seen(2), "the lines folded at the top are not shown");
        assert!(seen(3) && seen(4) && seen(5), "the change and the lines around it are");
        assert!(!seen(6) && !seen(8) && seen(9) && seen(10));
        assert!((0..11).all(|line| shows(&all.lines, &all.rows, line)), "nothing is folded without a fold");
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
    fn pulling_brings_in_what_the_remote_has_but_never_merges() {
        let Some(origin) = repository("pull-origin") else { return };
        let clone = origin.with_file_name(format!("code-canvas-pull-clone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&clone);
        let commit = |dir: &Path, name: &str| {
            std::fs::write(dir.join(name), format!("{name}\n")).unwrap();
            said(dir, &["add", name]);
            said(dir, &["commit", "-q", "-m", name]);
        };
        commit(&origin, "first.txt");
        said(&origin, &["clone", "-q", &origin.to_string_lossy(), &clone.to_string_lossy()]);
        for (key, value) in [("core.autocrlf", "false"), ("user.name", "t"), ("user.email", "t@t")] {
            said(&clone, &["config", key, value]);
        }
        let head = |dir: &Path| said(dir, &["rev-parse", "HEAD"]);

        commit(&origin, "second.txt");
        super::run(&clone, &GitOp::Pull).unwrap();
        assert_eq!(head(&clone), head(&origin), "a branch only behind is fast-forwarded");

        commit(&origin, "theirs.txt");
        commit(&clone, "mine.txt");
        let mine = head(&clone);
        assert!(super::run(&clone, &GitOp::Pull).is_err(), "a branch that went its own way is not merged");
        assert_eq!(head(&clone), mine, "and is left where it was");
        let _ = std::fs::remove_dir_all(&clone);
        let _ = std::fs::remove_dir_all(&origin);
    }

    #[test]
    fn local_branches_with_their_remote() {
        let text = "main\u{1f}origin/main\u{1f}ahead 1, behind 2\u{1f}*\nfeature/a\u{1f}origin/feature/a\u{1f}\u{1f} \nold\u{1f}origin/old\u{1f}gone\u{1f} \nlone\u{1f}\u{1f}\u{1f} \n";
        let got = parse_locals(text);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0].name, "main");
        assert_eq!(got[0].upstream.as_deref(), Some("origin/main"));
        assert_eq!((got[0].ahead, got[0].behind, got[0].current), (1, 2, true));
        assert_eq!((got[1].ahead, got[1].behind, got[1].current), (0, 0, false));
        assert_eq!((got[2].ahead, got[2].behind), (0, 0));
        assert_eq!(got[3].upstream, None);
    }

    #[test]
    fn remote_branches_skip_the_head_and_find_the_local() {
        let locals = parse_locals("main\u{1f}origin/main\u{1f}\u{1f}*\n");
        let remotes = vec!["origin".to_string(), "origin/fork".to_string()];
        let text = "origin\u{1f}refs/remotes/origin/main\norigin/main\u{1f}\norigin/feature/b/deep\u{1f}\norigin/fork/x\u{1f}\n";
        let got = parse_remote_branches(text, &remotes, &locals);
        assert_eq!(
            got,
            vec![
                RemoteBranch { remote: "origin".into(), name: "main".into(), local: Some("main".into()) },
                RemoteBranch { remote: "origin".into(), name: "feature/b/deep".into(), local: None },
                RemoteBranch { remote: "origin/fork".into(), name: "x".into(), local: None },
            ]
        );
    }

    #[test]
    fn what_bringing_a_remote_branch_runs() {
        let op = GitOp::Track { remote: "origin".into(), name: "fix/c".into() };
        let run = steps(&op).iter().map(|s| s.join(" ")).collect::<Vec<_>>();
        assert_eq!(run, vec!["switch -c fix/c --track origin/fix/c"]);
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
    fn tags_and_stashes() {
        assert_eq!(parse_tags("v2\nv1.0\n\n"), vec!["v2", "v1.0"]);
        let stashes = parse_stashes("stash@{0}\u{1f}WIP on main: abc one\nstash@{1}\u{1f}On x: two\nodd line\n");
        assert_eq!(
            stashes,
            vec![
                Stash { index: 0, message: "WIP on main: abc one".into() },
                Stash { index: 1, message: "On x: two".into() },
            ]
        );
    }

    #[test]
    fn branch_tag_and_stash_steps() {
        let one = |op: GitOp| steps(&op);
        let v = |args: &[&str]| vec![args.iter().map(|a| a.to_string()).collect::<Vec<String>>()];
        assert_eq!(one(GitOp::Merge("dev".into())), v(&["merge", "dev"]));
        assert_eq!(one(GitOp::Rebase("dev".into())), v(&["rebase", "dev"]));
        assert_eq!(one(GitOp::NewBranch { name: "n".into(), from: None }), v(&["switch", "-c", "n"]));
        assert_eq!(
            one(GitOp::NewBranch { name: "n".into(), from: Some("dev".into()) }),
            v(&["switch", "-c", "n", "dev"])
        );
        assert_eq!(one(GitOp::RenameBranch { from: "a".into(), to: "b".into() }), v(&["branch", "-m", "a", "b"]));
        assert_eq!(one(GitOp::DeleteBranch("a".into())), v(&["branch", "-d", "a"]));
        assert_eq!(
            one(GitOp::DeleteRemoteBranch { remote: "origin".into(), name: "a".into() }),
            v(&["push", "origin", "--delete", "a"])
        );
        assert_eq!(one(GitOp::PushBranch { remote: "up".into(), name: "a".into() }), v(&["push", "-u", "up", "a"]));
        assert_eq!(one(GitOp::NewTag { name: "t".into(), at: None }), v(&["tag", "t"]));
        assert_eq!(one(GitOp::NewTag { name: "t".into(), at: Some("dev".into()) }), v(&["tag", "t", "dev"]));
        assert_eq!(one(GitOp::DeleteTag("t".into())), v(&["tag", "-d", "t"]));
        assert_eq!(one(GitOp::PushTag("t".into())), v(&["push", "origin", "t"]));
        assert_eq!(one(GitOp::CheckoutTag("t".into())), v(&["switch", "--detach", "t"]));
        assert_eq!(one(GitOp::StashApply(2)), v(&["stash", "apply", "stash@{2}"]));
        assert_eq!(one(GitOp::StashPopAt(0)), v(&["stash", "pop", "stash@{0}"]));
        assert_eq!(one(GitOp::StashDrop(1)), v(&["stash", "drop", "stash@{1}"]));
    }

    #[test]
    fn what_is_in_progress() {
        assert_eq!(in_progress_of(&[], None), None);
        let merging = in_progress_of(&["MERGE_HEAD"], None).unwrap();
        assert_eq!(merging.operation, Operation::Merge);
        assert_eq!(in_progress_of(&["CHERRY_PICK_HEAD"], None).unwrap().operation, Operation::CherryPick);
        assert_eq!(in_progress_of(&["REVERT_HEAD"], None).unwrap().operation, Operation::Revert);
        let rebase = in_progress_of(&["rebase-merge", "CHERRY_PICK_HEAD"], Some((2, 5))).unwrap();
        assert_eq!((rebase.operation, rebase.step), (Operation::Rebase, Some((2, 5))));
        assert_eq!(in_progress_of(&["rebase-apply"], None).unwrap().operation, Operation::Rebase);
        assert_eq!(in_progress_of(&["rebase-apply", "rebase-apply/applying"], None), None, "git am is not a rebase");
        assert_eq!(in_progress_of(&["MERGE_HEAD"], Some((1, 2))).unwrap().step, None, "only a rebase has steps");
    }

    #[test]
    fn conflict_and_carry_steps() {
        let v = |args: &[&[&str]]| {
            args.iter().map(|a| a.iter().map(|s| s.to_string()).collect::<Vec<String>>()).collect::<Vec<_>>()
        };
        let take = |side| steps(&GitOp::Take { path: "a b.txt".into(), side });
        assert_eq!(take(Side::Ours), v(&[&["checkout", "--ours", "--", "a b.txt"], &["add", "--", "a b.txt"]]));
        assert_eq!(take(Side::Theirs), v(&[&["checkout", "--theirs", "--", "a b.txt"], &["add", "--", "a b.txt"]]));
        let carry = |operation, how| steps(&GitOp::Carry { operation, how });
        assert_eq!(carry(Operation::Merge, Carry::Continue), v(&[&["commit", "--no-edit"]]));
        assert_eq!(carry(Operation::Merge, Carry::Abort), v(&[&["merge", "--abort"]]));
        assert!(carry(Operation::Merge, Carry::Skip).is_empty());
        assert_eq!(carry(Operation::Rebase, Carry::Continue), v(&[&["rebase", "--continue"]]));
        assert_eq!(carry(Operation::Rebase, Carry::Skip), v(&[&["rebase", "--skip"]]));
        assert_eq!(carry(Operation::CherryPick, Carry::Abort), v(&[&["cherry-pick", "--abort"]]));
        assert_eq!(carry(Operation::Revert, Carry::Continue), v(&[&["revert", "--continue"]]));
        assert!(!Operation::Merge.can_skip() && Operation::Rebase.can_skip());
    }

    #[test]
    fn the_reflog_is_read() {
        let text = "aaa\u{1f}HEAD@{0}\u{1f}commit: Add x\u{1f}1700000100\naaa\u{1f}HEAD@{1}\u{1f}checkout: moving from a to b\u{1f}1700000000\nbad line\n";
        let entries = parse_reflog(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "HEAD@{0}");
        assert_eq!(entries[0].what, "commit: Add x");
        assert_eq!(entries[1].what, "checkout: moving from a to b");
        assert_eq!(entries[1].time, 1_700_000_000);
    }

    /// A conflict stops a cherry-pick, the banner's state is read, a side is taken and it goes on.
    #[test]
    fn a_conflict_is_resolved_and_the_cherry_pick_continues() {
        let Some(dir) = repository("conflict") else { return };
        let write = |text: &str| std::fs::write(dir.join("f.txt"), text).unwrap();
        write("base\n");
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &["commit", "-q", "-m", "base"]).unwrap();
        git(&dir, &["switch", "-q", "-c", "topic"]).unwrap();
        write("topic\n");
        git(&dir, &["commit", "-q", "-am", "topic change"]).unwrap();
        git(&dir, &["switch", "-q", "-"]).unwrap();
        write("main\n");
        git(&dir, &["commit", "-q", "-am", "main change"]).unwrap();
        let picked = run(&dir, &GitOp::CherryPick("topic".into()));
        assert!(picked.is_err(), "the pick stops on the conflict");
        let state = load(&dir).unwrap();
        assert_eq!(state.in_progress.map(|p| p.operation), Some(Operation::CherryPick));
        assert_eq!(state.conflicts(), 1);
        run(&dir, &GitOp::Take { path: "f.txt".into(), side: Side::Theirs }).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap().trim(), "topic");
        assert_eq!(load(&dir).unwrap().conflicts(), 0);
        run(&dir, &GitOp::Carry { operation: Operation::CherryPick, how: Carry::Continue }).unwrap();
        let state = load(&dir).unwrap();
        assert_eq!(state.in_progress, None);
        assert_eq!(state.graph[0].commit.subject, "topic change");
        assert!(state.reflog.iter().any(|entry| entry.what.contains("topic change")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The list we make reorders, squashes, rewords (with quotes in the message) and drops.
    #[test]
    fn an_interactive_rebase_runs_the_list() {
        let Some(dir) = repository("rebase-list") else { return };
        let commit = |name: &str| {
            std::fs::write(dir.join(format!("{name}.txt")), name).unwrap();
            git(&dir, &["add", "-A"]).unwrap();
            git(&dir, &["commit", "-q", "-m", name]).unwrap();
        };
        for name in ["base", "c1", "c2", "c3", "c4"] {
            commit(name);
        }
        let range = rebase_range(&dir, said(&dir, &["rev-parse", "HEAD~3"]).trim()).unwrap();
        assert_eq!(range.commits.len(), 4, "c1 to c4");
        let mut plan = crate::rebase::Plan::new(range.base.clone(), range.commits);
        use crate::rebase::Verb;
        plan.move_to(3, 2);
        plan.set(1, Verb::Squash);
        plan.set(3, Verb::Drop);
        plan.entries[2].pick = Verb::Reword;
        plan.entries[2].message = "Say \"hi\" it's $HOME".to_string();
        let op = GitOp::InteractiveRebase { base: plan.base.clone(), todo: plan.todo(), stash: false };
        run(&dir, &op).unwrap();
        let subjects = said(&dir, &["log", "--format=%s", "--reverse"]);
        let subjects: Vec<&str> = subjects.lines().collect();
        assert_eq!(subjects.len(), 3, "base, c1 with c2 melted in, and the reworded one: {subjects:?}");
        assert_eq!(subjects[2], "Say \"hi\" it's $HOME");
        assert!(dir.join("c2.txt").exists() && dir.join("c4.txt").exists());
        assert!(!dir.join("c3.txt").exists(), "c3 was dropped");
        assert_eq!(load(&dir).unwrap().in_progress, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_steps() {
        let one = |op: GitOp| steps(&op);
        let v = |args: &[&str]| vec![args.iter().map(|a| a.to_string()).collect::<Vec<String>>()];
        assert_eq!(one(GitOp::CheckoutCommit("abc".into())), v(&["switch", "--detach", "abc"]));
        assert_eq!(one(GitOp::CherryPick("abc".into())), v(&["cherry-pick", "abc"]));
        assert_eq!(one(GitOp::Revert("abc".into())), v(&["revert", "--no-edit", "abc"]));
        let reset = |mode| one(GitOp::Reset { to: "abc".into(), mode });
        assert_eq!(reset(ResetMode::Soft), v(&["reset", "--soft", "abc"]));
        assert_eq!(reset(ResetMode::Mixed), v(&["reset", "--mixed", "abc"]));
        assert_eq!(reset(ResetMode::Hard), v(&["reset", "--hard", "abc"]));
    }

    fn commit(hash: &str, parents: &[&str]) -> HistoryCommit {
        HistoryCommit {
            hash: hash.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            email: String::new(),
            time: 0,
            refs: Vec::new(),
            subject: String::new(),
        }
    }

    fn rows(commits: &[HistoryCommit]) -> Vec<GraphRow> {
        let mut lanes = Lanes::default();
        commits.iter().map(|c| lanes.push(c)).collect()
    }

    #[test]
    fn lanes_of_a_straight_history() {
        let r = rows(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert!(r.iter().all(|row| row.lane == 0 && row.joins.is_empty() && row.forks.is_empty()));
        assert_eq!((r[0].above.clone(), r[0].below.clone()), (vec![], vec![Some(0)]), "the tip has nothing above");
        assert_eq!((r[1].above.clone(), r[1].below.clone()), (vec![Some(0)], vec![Some(0)]));
        assert_eq!(r[2].below, Vec::<Option<usize>>::new(), "the root ends the line");
    }

    #[test]
    fn lanes_of_a_branch_and_its_merge() {
        // m merges f (second parent) into main; f and b both start from a.
        let r = rows(&[commit("m", &["b", "f"]), commit("f", &["a"]), commit("b", &["a"]), commit("a", &[])]);
        assert_eq!(r[0].lane, 0);
        assert_eq!(r[0].forks, vec![(1, 1)], "the merge opens a second lane for the branch");
        assert_eq!(r[1].lane, 1, "the branch commit sits in the second lane");
        assert_eq!(r[1].below, vec![Some(0), Some(1)]);
        assert_eq!(r[2].lane, 0);
        assert_eq!(r[2].below, vec![Some(0), Some(1)], "the branch lane still waits for a");
        assert_eq!(r[3].lane, 0);
        assert_eq!(r[3].joins, vec![(1, 1)], "the lane of the branch ends in a");
        assert_eq!(r[3].below, Vec::<Option<usize>>::new());
    }

    #[test]
    fn lanes_of_an_octopus_and_two_merges() {
        let r = rows(&[
            commit("o", &["a", "b", "c"]),
            commit("a", &["z"]),
            commit("b", &["z"]),
            commit("c", &["z"]),
            commit("z", &[]),
        ]);
        assert_eq!(r[0].forks, vec![(1, 1), (2, 2)], "two extra lanes for an octopus merge");
        assert_eq!((r[1].lane, r[2].lane, r[3].lane), (0, 1, 2));
        assert_eq!(r[4].lane, 0);
        assert_eq!(r[4].joins, vec![(1, 1), (2, 2)]);
        // Two merges: the second one reuses the lane the first one freed.
        let r = rows(&[
            commit("m2", &["m1", "y"]),
            commit("m1", &["a", "x"]),
            commit("y", &["a"]),
            commit("x", &["a"]),
            commit("a", &[]),
        ]);
        assert_eq!(r[0].forks, vec![(1, 1)]);
        assert_eq!(r[1].forks, vec![(2, 2)], "lane 1 is taken by y, so x gets the third one");
        assert_eq!((r[2].lane, r[3].lane), (1, 2));
        assert_eq!(r[4].joins, vec![(1, 1), (2, 2)]);
    }

    #[test]
    fn a_merge_into_a_lane_that_is_already_waiting() {
        // x waits for c in lane 0; the merge m (a new tip, lane 1) has c as its second parent.
        let r = rows(&[commit("x", &["c"]), commit("m", &["b", "c"]), commit("b", &["a"]), commit("c", &[])]);
        assert_eq!(r[1].lane, 1);
        assert_eq!(r[1].forks, vec![(0, 0)], "the merge line goes into the lane that already waits for c");
        assert_eq!(r[3].lane, 0);
    }

    #[test]
    fn lanes_carry_over_a_page_boundary() {
        let all = [commit("m", &["b", "f"]), commit("f", &["a"]), commit("b", &["a"]), commit("a", &[])];
        let whole = rows(&all);
        let mut lanes = Lanes::default();
        let mut paged: Vec<GraphRow> = all[..2].iter().map(|c| lanes.push(c)).collect();
        paged.extend(all[2..].iter().map(|c| lanes.push(c)));
        assert_eq!(whole, paged, "feeding the commits in pages gives the same rows");
        assert!(lanes.open() == 0, "nothing is waiting after the root");
    }

    #[test]
    fn history_arguments() {
        let q = HistoryQuery::default();
        let line = history_args(&q, 0, 300).join(" ");
        assert!(line.starts_with("log --topo-order --format="), "{line}");
        assert!(line.contains("--exclude=refs/stash --all --skip 0 -n 300"), "{line}");
        assert!(line.ends_with(" --"), "{line}");
        let q = HistoryQuery { only: vec![], hidden: vec!["x".into()], search: Search::Message("fix".into()) };
        let line = history_args(&q, 300, 300).join(" ");
        assert!(
            line.contains(
                "--exclude=refs/heads/x --exclude=refs/remotes/*/x --exclude=refs/remotes/x --exclude=refs/stash --all"
            ),
            "{line}"
        );
        assert!(line.contains("--skip 300 -n 300 --grep=fix -i"), "{line}");
        let q = HistoryQuery {
            only: vec!["a".into(), "b".into()],
            hidden: vec!["b".into()],
            search: Search::File("src/a.rs".into()),
        };
        let line = history_args(&q, 0, 10).join(" ");
        assert!(line.contains("--topo-order") && !line.contains("--all") && !line.contains("--exclude"), "{line}");
        assert!(line.ends_with("-n 10 a -- src/a.rs"), "{line}");
        let q = HistoryQuery { search: Search::Author("Ana".into()), ..HistoryQuery::default() };
        assert!(history_args(&q, 0, 5).join(" ").contains("--author=Ana -i"));
    }

    #[test]
    fn dates() {
        assert_eq!(date_of(0), "1970-01-01 00:00 UTC");
        assert_eq!(date_of(1_700_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(date_of(1_709_164_800 + 3600), "2024-02-29 01:00 UTC");
    }

    #[test]
    fn history_lines() {
        let line = "aaa\u{1f}bbb ccc\u{1f}Ana\u{1f}ana@x.dev\u{1f}100\u{1f}HEAD -> main, tag: v1, origin/main, origin/HEAD\u{1f}Fix: the thing";
        let got = parse_history(&format!("{line}\nbbb\u{1f}\u{1f}Bo\u{1f}b@x\u{1f}50\u{1f}\u{1f}root\n"));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].parents, vec!["bbb".to_string(), "ccc".to_string()]);
        assert_eq!(got[0].refs, vec!["HEAD -> main", "tag: v1", "origin/main"]);
        assert_eq!((got[0].email.as_str(), got[0].subject.as_str()), ("ana@x.dev", "Fix: the thing"));
        assert!(got[1].parents.is_empty() && got[1].refs.is_empty());
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
