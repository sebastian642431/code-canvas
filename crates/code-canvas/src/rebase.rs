//! The plan of an interactive rebase: the commits from the one chosen up to HEAD, each with what
//! to do to it, in the order they are replayed, and the todo text git is given when it starts.

use crate::git::sh_quote;

/// The most commits the card takes; a longer stretch is not something to arrange by hand.
pub const MOST: usize = 200;

/// What a rebase does with a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Keep it as it is.
    Pick,
    /// Keep it with a new message.
    Reword,
    /// Melt it into the commit before it, joining the messages.
    Squash,
    /// Melt it into the commit before it, dropping its message.
    Fixup,
    /// Leave it out.
    Drop,
}

impl Verb {
    pub const ALL: [Verb; 5] = [Verb::Pick, Verb::Reword, Verb::Squash, Verb::Fixup, Verb::Drop];

    pub fn label(self) -> &'static str {
        match self {
            Verb::Pick => "Pick",
            Verb::Reword => "Reword",
            Verb::Squash => "Squash",
            Verb::Fixup => "Fixup",
            Verb::Drop => "Drop",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Verb::Pick => "Keep the commit as it is",
            Verb::Reword => "Keep the commit and change its message",
            Verb::Squash => "Melt it into the commit above, joining their messages",
            Verb::Fixup => "Melt it into the commit above, keeping only that one's message",
            Verb::Drop => "Leave the commit out: what it changed is lost",
        }
    }

    /// The word of this action in a script.
    pub fn from_word(word: &str) -> Option<Verb> {
        Verb::ALL.into_iter().find(|pick| pick.label().eq_ignore_ascii_case(word))
    }

    fn joins(self) -> bool {
        matches!(self, Verb::Squash | Verb::Fixup)
    }
}

/// One commit of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub hash: String,
    /// The subject it has now.
    pub subject: String,
    pub pick: Verb,
    /// The subject typed for a Reword: it starts as the subject it has.
    pub message: String,
}

/// The commits to replay, oldest first, and the commit they start after.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// The commit the rebase starts after; none when it starts at the first commit of the history.
    pub base: Option<String>,
    pub entries: Vec<Entry>,
    /// The hashes in the order the plan started with.
    original: Vec<String>,
    /// The row picked up with the mouse.
    pub dragging: Option<usize>,
    /// The row Alt+Up and Alt+Down move.
    pub selected: Option<usize>,
}

impl Plan {
    pub fn new(base: Option<String>, commits: Vec<(String, String)>) -> Plan {
        let entries: Vec<Entry> = commits
            .into_iter()
            .map(|(hash, subject)| Entry { hash, message: subject.clone(), subject, pick: Verb::Pick })
            .collect();
        let original = entries.iter().map(|entry: &Entry| entry.hash.clone()).collect();
        Plan { base, entries, original, dragging: None, selected: None }
    }

    /// Whether the row can be a Squash or a Fixup: something that is not dropped has to be above it.
    pub fn can_join(&self, row: usize) -> bool {
        self.entries.iter().take(row).any(|entry| entry.pick != Verb::Drop)
    }

    /// Sets what is done to a row, if that makes sense there.
    pub fn set(&mut self, row: usize, pick: Verb) {
        if row >= self.entries.len() || (pick.joins() && !self.can_join(row)) {
            return;
        }
        self.entries[row].pick = pick;
        self.fix_first();
    }

    /// Moves a row to another place.
    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.entries.len() || to >= self.entries.len() || from == to {
            return;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        for slot in [&mut self.dragging, &mut self.selected] {
            if *slot == Some(from) {
                *slot = Some(to);
            }
        }
        self.fix_first();
    }

    /// A Squash or a Fixup with nothing to melt into (the first one that is kept, or one under a
    /// row that was dropped) becomes a Pick.
    fn fix_first(&mut self) {
        let mut kept = false;
        for entry in &mut self.entries {
            if entry.pick.joins() && !kept {
                entry.pick = Verb::Pick;
            }
            kept |= entry.pick != Verb::Drop;
        }
    }

    /// Anything to do at all: the plan differs from picking every commit as it is.
    pub fn changes_something(&self) -> bool {
        let same_order = self.entries.iter().map(|entry| &entry.hash).eq(self.original.iter());
        !same_order
            || self.entries.iter().any(|entry| {
                entry.pick != Verb::Pick && !(entry.pick == Verb::Reword && entry.message == entry.subject)
            })
    }

    /// Whether the plan keeps at least one commit.
    pub fn keeps_one(&self) -> bool {
        self.entries.iter().any(|entry| entry.pick != Verb::Drop)
    }

    /// The text of the todo list git is given.
    pub fn todo(&self) -> String {
        todo_text(&self.entries)
    }
}

/// A reworded message in one line: a line break would end the line of the todo list.
fn one_line(message: &str) -> String {
    let joined: Vec<&str> = message.split(['\r', '\n']).map(str::trim).filter(|line| !line.is_empty()).collect();
    joined.join(" ")
}

/// The todo list of a rebase. A dropped commit is left out. A Reword is a pick followed by a command
/// that changes the message, put after the squashes and fixups that follow it so that they still
/// have a commit to melt into.
pub fn todo_text(entries: &[Entry]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut reword: Option<String> = None;
    let flush = |lines: &mut Vec<String>, reword: &mut Option<String>| {
        if let Some(message) = reword.take() {
            lines.push(format!("exec git commit --amend --only -m {}", sh_quote(&message)));
        }
    };
    for entry in entries {
        let subject = one_line(&entry.subject);
        match entry.pick {
            Verb::Drop => continue,
            Verb::Squash => lines.push(format!("squash {} {subject}", entry.hash)),
            Verb::Fixup => lines.push(format!("fixup {}", entry.hash)),
            Verb::Pick | Verb::Reword => {
                flush(&mut lines, &mut reword);
                lines.push(format!("pick {} {subject}", entry.hash));
                if entry.pick == Verb::Reword {
                    let message = one_line(&entry.message);
                    reword = Some(if message.is_empty() { subject } else { message });
                }
            }
        }
    }
    flush(&mut lines, &mut reword);
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(subjects: &[&str]) -> Plan {
        let commits = subjects.iter().enumerate().map(|(i, s)| (format!("{:040x}", i + 1), s.to_string())).collect();
        Plan::new(Some("base".to_string()), commits)
    }

    #[test]
    fn a_plain_plan_picks_every_commit() {
        let plan = plan(&["one", "two"]);
        let text = plan.todo();
        let first = format!("{:040x}", 1);
        assert!(text.starts_with(&format!("pick {first} one\n")));
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn drop_leaves_the_line_out_and_squash_and_fixup_keep_theirs() {
        let mut plan = plan(&["a", "b", "c", "d"]);
        plan.set(1, Verb::Squash);
        plan.set(2, Verb::Fixup);
        plan.set(3, Verb::Drop);
        let lines: Vec<String> = plan.todo().lines().map(str::to_string).collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].starts_with("squash ") && lines[1].ends_with(" b"));
        assert_eq!(lines[2], format!("fixup {:040x}", 3));
    }

    #[test]
    fn the_first_kept_commit_cannot_melt_into_nothing() {
        let mut plan = plan(&["a", "b", "c"]);
        plan.set(0, Verb::Squash);
        assert_eq!(plan.entries[0].pick, Verb::Pick, "nothing is above the first row");
        plan.set(1, Verb::Squash);
        plan.set(0, Verb::Drop);
        assert_eq!(plan.entries[1].pick, Verb::Pick, "the row above was dropped: it has nothing to melt into");
        plan.set(2, Verb::Fixup);
        plan.move_to(2, 0);
        assert_eq!(plan.entries[0].pick, Verb::Pick, "a fixup moved to the top becomes a pick");
    }

    #[test]
    fn a_reword_comes_after_the_commits_melted_into_it() {
        let mut plan = plan(&["a", "b", "c"]);
        plan.set(0, Verb::Reword);
        plan.entries[0].message = "new".to_string();
        plan.set(1, Verb::Squash);
        let lines: Vec<String> = plan.todo().lines().map(str::to_string).collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("pick ") && lines[1].starts_with("squash "));
        assert_eq!(lines[2], "exec git commit --amend --only -m 'new'");
        assert!(lines[3].starts_with("pick "));
    }

    #[test]
    fn quoting_survives_quotes_dollars_and_line_breaks() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("say \"hi\" $HOME `x`"), "'say \"hi\" $HOME `x`'");
        assert_eq!(sh_quote("a\nb"), "'a\nb'");
        let mut plan = plan(&["a"]);
        plan.set(0, Verb::Reword);
        plan.entries[0].message = "line one\nline 'two' $HOME\r\n".to_string();
        let text = plan.todo();
        assert_eq!(text.lines().count(), 2, "a line break in the message cannot split the list: {text}");
        assert!(text.ends_with("-m 'line one line '\\''two'\\'' $HOME'\n"), "{text}");
    }

    #[test]
    fn moving_a_row_keeps_the_one_being_dragged_in_hand() {
        let mut plan = plan(&["a", "b", "c"]);
        plan.dragging = Some(2);
        plan.move_to(2, 0);
        assert_eq!(plan.dragging, Some(0));
        assert_eq!(plan.entries[0].subject, "c");
    }

    #[test]
    fn a_plan_that_changes_nothing_is_noticed() {
        let mut plan = plan(&["a", "b"]);
        assert!(!plan.changes_something());
        plan.set(0, Verb::Reword);
        assert!(!plan.changes_something(), "a reword with the same words is not a change");
        plan.entries[0].message = "other".to_string();
        assert!(plan.changes_something());
    }
}
