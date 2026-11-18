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
    Part { path: String, line: u32, what: Part },
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
        GitOp::Fetch => vec![s(&["fetch", "--prune"])],
        GitOp::Publish(branch) => vec![s(&["push", "-u", "origin", branch])],
        GitOp::Discard(_) | GitOp::DiscardAll => Vec::new(),
        GitOp::Stash => vec![s(&["stash", "push", "--include-untracked"])],
        GitOp::StashPop => vec![s(&["stash", "pop"])],
        GitOp::Switch(branch) => vec![s(&["switch", branch])],
    }
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
    c.env("GIT_OPTIONAL_LOCKS", "0").arg("-C").arg(root).args(["-c", "core.quotepath=off", "--literal-pathspecs"]);
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
