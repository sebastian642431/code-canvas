//! Switching branch without making the person wait: the branch clicked shows as the current one at
//! once while git works in the background, clicks that come while git is still at it only keep the
//! last branch asked for, and the files git writes are taken in once, when it is done.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::git::{GitOp, GitState};

/// How long the files written by git get to arrive from the watcher before they are taken in.
const QUIET: Duration = Duration::from_millis(500);
/// How long the watcher may keep the switch waiting when other folders keep changing.
const LONGEST: Duration = Duration::from_secs(3);

/// The branch a switch goes to.
pub fn target(op: &GitOp) -> Option<&str> {
    match op {
        GitOp::Switch(branch) => Some(branch),
        GitOp::Track { name, .. } => Some(name),
        _ => None,
    }
}

/// Shows `branch` as the current one in a state git read before it got there.
pub fn show_on(git: &mut GitState, branch: &str, switching: bool) {
    git.branch = branch.to_string();
    git.switching = switching;
    for local in &mut git.locals {
        local.current = local.name == branch;
    }
}

/// Whether the files git wrote have had time to come in from the watcher.
pub fn settled(waited: Duration, watching: bool) -> bool {
    waited >= QUIET && (!watching || waited >= LONGEST)
}

/// What to do when git is done with a switch.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// Another branch was asked for meanwhile: switch to it now.
    Run(GitOp),
    /// On the last branch asked for.
    Arrived,
    /// git refused; the branch shown is again the one git is on.
    Refused,
}

/// The switches asked for in a row: the one git is running and the last one asked for after it.
#[derive(Debug)]
pub struct Plan {
    real: String,
    running: Option<GitOp>,
    queued: Option<GitOp>,
    moved: bool,
}

impl Plan {
    /// A switch from the branch git is on; none when it goes nowhere.
    pub fn start(real: String, op: GitOp) -> Option<Plan> {
        let to = target(&op)?;
        (to != real).then(|| Plan { real, running: Some(op), queued: None, moved: false })
    }

    /// The branch shown as the current one: the last one asked for.
    pub fn shown(&self) -> &str {
        let asked = self.queued.as_ref().or(self.running.as_ref());
        asked.and_then(target).unwrap_or(&self.real)
    }

    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    /// Whether git switched at least once, so files on disk may have changed.
    pub fn moved(&self) -> bool {
        self.moved
    }

    /// Another branch is asked for. While git runs it waits (in place of what waited before), and
    /// the branch git is going to anyway takes back what waited. Gives the switch to run now when git
    /// is not running.
    pub fn ask(&mut self, op: GitOp) -> Option<GitOp> {
        let to = target(&op)?;
        match self.running.as_ref().and_then(target) {
            Some(going) if going == to => self.queued = None,
            Some(_) => self.queued = Some(op),
            None if to == self.real => {}
            None => {
                self.running = Some(op.clone());
                return Some(op);
            }
        }
        None
    }

    /// git is done with the switch it was running. A refusal forgets what waited: the person sees
    /// why and chooses again.
    pub fn finished(&mut self, ok: bool) -> Next {
        let done = self.running.take();
        if !ok {
            self.queued = None;
            return Next::Refused;
        }
        if let Some(to) = done.as_ref().and_then(target) {
            self.real = to.to_string();
            self.moved = true;
        }
        match self.queued.take() {
            Some(op) if target(&op) != Some(self.real.as_str()) => {
                self.running = Some(op.clone());
                Next::Run(op)
            }
            _ => Next::Arrived,
        }
    }
}

/// Where a switch is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// git is switching.
    Git,
    /// git is done; the files it wrote are still coming in from the watcher.
    Settling(Instant),
    /// The folder is being read again.
    Reloading,
    /// git refused before anything changed: nothing is left to wait for.
    Over,
}

/// A switch of branch in a folder, and the files that changed on disk meanwhile, kept to be taken
/// in all at once when it is done.
#[derive(Debug)]
pub struct Switch {
    pub plan: Plan,
    pub stage: Stage,
    changed: Vec<PathBuf>,
    overflow: bool,
}

impl Switch {
    pub fn new(plan: Plan) -> Switch {
        Switch { plan, stage: Stage::Git, changed: Vec::new(), overflow: false }
    }

    /// Keeps a file that changed on disk. Past `most` files the folder is read again whole, so the
    /// rest are only counted as too many.
    pub fn changed(&mut self, path: PathBuf, most: usize) {
        if self.overflow || self.changed.contains(&path) {
            return;
        }
        match self.changed.len() < most {
            true => self.changed.push(path),
            false => {
                self.overflow = true;
                self.changed.clear();
            }
        }
    }

    /// Whether more files changed than are worth taking in one by one.
    pub fn too_many(&self) -> bool {
        self.overflow
    }

    /// The files kept, and whether there were too many to keep.
    pub fn take_changed(&mut self) -> (Vec<PathBuf>, bool) {
        (std::mem::take(&mut self.changed), std::mem::take(&mut self.overflow))
    }

    /// What the canvas says while the switch lasts.
    pub fn label(&self) -> String {
        format!("Switching to {}…", self.plan.shown())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::LocalBranch;

    fn switch(to: &str) -> GitOp {
        GitOp::Switch(to.to_string())
    }

    #[test]
    fn one_switch_shows_the_branch_at_once_and_lands_on_it() {
        let mut plan = Plan::start("main".into(), switch("dev")).unwrap();
        assert_eq!(plan.shown(), "dev");
        assert!(plan.running());
        assert_eq!(plan.finished(true), Next::Arrived);
        assert_eq!(plan.shown(), "dev");
        assert!(!plan.running());
    }

    #[test]
    fn switching_to_the_branch_it_is_on_does_nothing() {
        assert!(Plan::start("main".into(), switch("main")).is_none());
        assert!(Plan::start("main".into(), GitOp::Fetch).is_none());
    }

    #[test]
    fn clicks_while_git_runs_keep_only_the_last_branch() {
        let mut plan = Plan::start("main".into(), switch("a")).unwrap();
        assert_eq!(plan.ask(switch("b")), None);
        assert_eq!(plan.ask(switch("c")), None);
        assert_eq!(plan.shown(), "c");
        assert_eq!(plan.finished(true), Next::Run(switch("c")));
        assert_eq!(plan.shown(), "c");
        assert_eq!(plan.finished(true), Next::Arrived);
        assert_eq!(plan.shown(), "c");
    }

    #[test]
    fn clicking_the_branch_git_is_going_to_drops_what_waited() {
        let mut plan = Plan::start("main".into(), switch("a")).unwrap();
        plan.ask(switch("b"));
        plan.ask(switch("a"));
        assert_eq!(plan.shown(), "a");
        assert_eq!(plan.finished(true), Next::Arrived);
    }

    #[test]
    fn clicking_back_to_where_it_started_switches_back_after() {
        let mut plan = Plan::start("main".into(), switch("a")).unwrap();
        plan.ask(switch("main"));
        assert_eq!(plan.shown(), "main");
        assert_eq!(plan.finished(true), Next::Run(switch("main")));
        assert_eq!(plan.finished(true), Next::Arrived);
        assert_eq!(plan.shown(), "main");
    }

    #[test]
    fn a_refusal_goes_back_to_the_real_branch_and_forgets_what_waited() {
        let mut plan = Plan::start("main".into(), switch("a")).unwrap();
        plan.ask(switch("b"));
        assert_eq!(plan.finished(false), Next::Refused);
        assert_eq!(plan.shown(), "main");
        assert!(!plan.running());
        assert!(!plan.moved());
    }

    #[test]
    fn a_refusal_after_a_switch_that_went_through_stays_on_that_one() {
        let mut plan = Plan::start("main".into(), switch("a")).unwrap();
        plan.ask(switch("b"));
        assert_eq!(plan.finished(true), Next::Run(switch("b")));
        assert_eq!(plan.finished(false), Next::Refused);
        assert_eq!(plan.shown(), "a");
        assert!(plan.moved());
    }

    #[test]
    fn asking_once_git_is_done_runs_at_once() {
        let mut plan = Plan::start("main".into(), switch("a")).unwrap();
        plan.finished(true);
        assert_eq!(plan.ask(switch("a")), None);
        assert_eq!(plan.ask(switch("b")), Some(switch("b")));
        assert!(plan.running());
        assert_eq!(plan.shown(), "b");
    }

    #[test]
    fn a_branch_from_a_remote_goes_by_its_name() {
        let op = GitOp::Track { remote: "origin".into(), name: "fix/x".into() };
        assert_eq!(target(&op), Some("fix/x"));
        assert_eq!(Plan::start("main".into(), op).unwrap().shown(), "fix/x");
    }

    #[test]
    fn the_branch_shown_is_the_one_ticked() {
        let local = |name: &str, current| LocalBranch { name: name.into(), current, ..Default::default() };
        let mut git = GitState {
            branch: "main".into(),
            locals: vec![local("main", true), local("dev", false)],
            ..Default::default()
        };
        show_on(&mut git, "dev", true);
        assert_eq!(git.branch, "dev");
        assert!(git.switching);
        assert_eq!(git.locals.iter().filter(|l| l.current).map(|l| l.name.as_str()).collect::<Vec<_>>(), ["dev"]);
    }

    #[test]
    fn many_changed_files_are_not_kept_one_by_one() {
        let mut going = Switch::new(Plan::start("main".into(), switch("a")).unwrap());
        for n in 0..3 {
            going.changed(PathBuf::from(format!("f{n}")), 3);
        }
        going.changed(PathBuf::from("f0"), 3);
        assert!(!going.too_many());
        going.changed(PathBuf::from("f3"), 3);
        assert!(going.too_many());
        let (kept, overflow) = going.take_changed();
        assert!(kept.is_empty() && overflow);
        assert!(!going.too_many());
    }

    #[test]
    fn the_files_wait_for_the_watcher_but_not_for_ever() {
        assert!(!settled(Duration::from_millis(100), false));
        assert!(settled(QUIET, false));
        assert!(!settled(QUIET, true));
        assert!(settled(LONGEST, true));
    }
}
