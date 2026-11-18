//! What the "All Commits" card of a folder holds: the commits read so far, their lanes, what is
//! searched, which branches are shown or hidden and which commit is selected. Pages of commits are
//! read from git on a thread of their own, the first one when the card opens and the next ones when
//! the list is scrolled near its end.

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::git::{GraphRow, HistoryCommit, HistoryQuery, Lanes, Search};

/// Commits read the first time and each time after.
pub const PAGE: usize = 300;
/// How long the search box waits after the last key before it asks git.
const TYPING: Duration = Duration::from_millis(280);

/// What the search box looks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Message,
    Author,
    Hash,
    File,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Message, Mode::Author, Mode::Hash, Mode::File];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Message => "Message",
            Mode::Author => "Author",
            Mode::Hash => "Hash",
            Mode::File => "File",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Mode::Message => "Search the messages of the commits",
            Mode::Author => "Search by the name of the author",
            Mode::Hash => "A hash or its first letters",
            Mode::File => "A file or folder, as it is in the repository",
        }
    }

    fn search(self, text: &str) -> Search {
        let text = text.trim().to_string();
        match (self, text.is_empty()) {
            (_, true) => Search::None,
            (Mode::Message, _) => Search::Message(text),
            (Mode::Author, _) => Search::Author(text),
            (Mode::Hash, _) => Search::Hash(text),
            (Mode::File, _) => Search::File(text),
        }
    }
}

struct Reading {
    skip: usize,
    rx: Receiver<Vec<HistoryCommit>>,
}

pub struct HistoryView {
    /// What is being shown now (the search and the branches of the last time git was asked).
    pub query: HistoryQuery,
    pub mode: Mode,
    /// What is typed in the search box; it becomes the search a moment after the last key.
    pub draft: String,
    edited: Option<Instant>,
    pub commits: Vec<HistoryCommit>,
    pub rows: Vec<GraphRow>,
    lanes: Lanes,
    /// Another page may be there to read.
    pub more: bool,
    /// The most lanes any commit read so far needs.
    pub width: usize,
    /// The first page has come.
    pub loaded: bool,
    /// The list was scrolled near its end: read the next page.
    pub want_more: bool,
    reading: Option<Reading>,
    stale: bool,
    stamp: String,
    pub selected: Option<String>,
    /// A row the list is to scroll to.
    pub scroll_to: Option<usize>,
    /// The commits read so far that HEAD grew from, HEAD itself too.
    behind_head: HashSet<String>,
    behind_head_for: (String, usize),
}

impl Default for HistoryView {
    fn default() -> Self {
        HistoryView {
            query: HistoryQuery::default(),
            mode: Mode::Message,
            draft: String::new(),
            edited: None,
            commits: Vec::new(),
            rows: Vec::new(),
            lanes: Lanes::default(),
            more: true,
            width: 1,
            loaded: false,
            want_more: false,
            reading: None,
            stale: true,
            stamp: String::new(),
            selected: None,
            scroll_to: None,
            behind_head: HashSet::new(),
            behind_head_for: (String::new(), 0),
        }
    }
}

impl HistoryView {
    /// Whether the lanes make sense: they do not when a search leaves commits out.
    pub fn graph(&self) -> bool {
        self.query.search == Search::None
    }

    pub fn reading(&self) -> bool {
        self.reading.is_some()
    }

    /// Reads everything again from the top.
    pub fn reload(&mut self) {
        self.reading = None;
        self.commits.clear();
        self.rows.clear();
        self.lanes = Lanes::default();
        self.more = true;
        self.width = 1;
        self.loaded = false;
        self.stale = true;
    }

    /// Shows another set of commits; nothing is read when it is the same.
    pub fn set_query(&mut self, query: HistoryQuery) {
        if query != self.query {
            self.query = query;
            self.reload();
        }
    }

    /// Shows only this branch, or every branch again when it already was the only one.
    pub fn toggle_only(&mut self, name: &str) {
        let mut query = self.query.clone();
        match query.only.as_slice() {
            [one] if one == name => query.only.clear(),
            _ => query.only = vec![name.to_string()],
        }
        query.hidden.retain(|hidden| hidden != name);
        self.set_query(query);
    }

    /// Hides this branch from the list, or shows it again.
    pub fn toggle_hidden(&mut self, name: &str) {
        let mut query = self.query.clone();
        match query.hidden.iter().position(|hidden| hidden == name) {
            Some(at) => {
                query.hidden.remove(at);
            }
            None => {
                query.hidden.push(name.to_string());
                query.only.retain(|only| only != name);
            }
        }
        self.set_query(query);
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.apply_draft();
    }

    /// The text was edited: it is searched when the typing stops.
    pub fn typed(&mut self) {
        self.edited = Some(Instant::now());
    }

    fn apply_draft(&mut self) {
        self.edited = None;
        let query = HistoryQuery { search: self.mode.search(&self.draft), ..self.query.clone() };
        self.set_query(query);
    }

    /// How long until the typed text is searched, if it is waiting.
    pub fn typing_left(&self) -> Option<Duration> {
        self.edited.map(|at| TYPING.saturating_sub(at.elapsed()))
    }

    /// Picks up what the thread read and starts reading what is missing. `stamp` changes when the
    /// repository did (a commit, a branch, a tag): the list is read again. Says whether the list changed.
    pub fn update(&mut self, root: &Path, stamp: &str) -> bool {
        let mut changed = false;
        if self.stamp != stamp {
            if !self.stamp.is_empty() {
                self.reload();
            }
            self.stamp = stamp.to_string();
        }
        if self.edited.is_some_and(|at| at.elapsed() >= TYPING) {
            self.apply_draft();
            changed = true;
        }
        if let Some(reading) = &self.reading {
            match reading.rx.try_recv() {
                Ok(page) => {
                    let skip = reading.skip;
                    self.reading = None;
                    if skip == self.commits.len() {
                        self.more = page.len() >= PAGE;
                        for commit in &page {
                            let row = self.lanes.push(commit);
                            self.width = self.width.max(row.width());
                            self.rows.push(row);
                        }
                        self.commits.extend(page);
                        self.loaded = true;
                        changed = true;
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.reading = None;
                    self.more = false;
                    self.loaded = true;
                }
            }
        }
        if self.reading.is_none() && (self.stale || (self.want_more && self.more)) {
            self.stale = false;
            self.want_more = false;
            let (tx, rx) = channel();
            let (root, query, skip) = (root.to_path_buf(), self.query.clone(), self.commits.len());
            std::thread::spawn(move || {
                let _ = tx.send(crate::git::history(&root, &query, skip, PAGE));
                crate::ext::wake_ui();
            });
            self.reading = Some(Reading { skip, rx });
        }
        changed
    }

    /// Works out which of the commits read so far HEAD grew from (children come before their
    /// parents in the list, so one pass is enough). Nothing is done when it is already known.
    pub fn mark_ancestors(&mut self, head: &str) {
        if self.behind_head_for.0 == head && self.behind_head_for.1 == self.commits.len() {
            return;
        }
        self.behind_head_for = (head.to_string(), self.commits.len());
        self.behind_head.clear();
        self.behind_head.insert(head.to_string());
        for commit in &self.commits {
            if self.behind_head.contains(&commit.hash) {
                self.behind_head.extend(commit.parents.iter().cloned());
            }
        }
    }

    /// Whether HEAD grew from this commit, or it is HEAD: a commit an interactive rebase can start at.
    pub fn is_behind_head(&self, hash: &str) -> bool {
        self.behind_head.contains(hash)
    }

    pub fn index_of(&self, hash: &str) -> Option<usize> {
        self.commits.iter().position(|commit| commit.hash == hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_and_hidden_toggle() {
        let mut view = HistoryView::default();
        view.toggle_only("a");
        assert_eq!(view.query.only, vec!["a".to_string()]);
        view.toggle_only("b");
        assert_eq!(view.query.only, vec!["b".to_string()], "showing only one branch replaces the other");
        view.toggle_only("b");
        assert!(view.query.only.is_empty(), "a second time shows every branch again");
        view.toggle_hidden("c");
        assert_eq!(view.query.hidden, vec!["c".to_string()]);
        view.toggle_only("c");
        assert!(view.query.hidden.is_empty(), "a branch shown alone is not hidden");
        view.toggle_hidden("d");
        view.toggle_hidden("d");
        assert!(view.query.hidden.is_empty());
    }

    #[test]
    fn the_commits_head_grew_from_are_found() {
        let commit = |hash: &str, parents: &[&str]| HistoryCommit {
            hash: hash.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            email: String::new(),
            time: 0,
            refs: Vec::new(),
            subject: String::new(),
        };
        // main: m3 <- m2 <- m1 and a branch b2 <- b1 that left m1 (children come before parents).
        let commits = vec![
            commit("b2", &["b1"]),
            commit("m3", &["m2"]),
            commit("m2", &["m1"]),
            commit("b1", &["m1"]),
            commit("m1", &[]),
        ];
        let mut view = HistoryView { commits, ..HistoryView::default() };
        view.mark_ancestors("m3");
        assert!(view.is_behind_head("m3") && view.is_behind_head("m2") && view.is_behind_head("m1"));
        assert!(!view.is_behind_head("b1") && !view.is_behind_head("b2"));
    }

    #[test]
    fn a_search_is_made_of_the_mode_and_the_text() {
        let mut view = HistoryView { draft: "  fix  ".to_string(), ..HistoryView::default() };
        view.set_mode(Mode::Author);
        assert_eq!(view.query.search, Search::Author("fix".to_string()));
        view.draft.clear();
        view.set_mode(Mode::File);
        assert_eq!(view.query.search, Search::None);
        assert!(view.graph());
    }
}
