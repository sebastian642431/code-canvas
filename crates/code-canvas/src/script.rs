use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Wait(u64),
    Fit,
    Locate,
    Goto(String),
    Select(String),
    Search(String),
    Left(String),
    Install(String),
    ExtSearch(String),
    Format(String),
    Command(String),
    Answer(String),
    Open(String),
    Focus(String),
    Updates,
    Resize(u32, u32),
    Palette(String),
    Edit(String),
    Type(String),
    Key(String),
    Git(String),
    OnlyChanged(bool),
    Right(String),
    CollapseAll,
    ExpandAll,
    Connected(bool),
    Tab(usize),
    Folders,
    Zoom(f32),
    Quick(String),
    Maximize,
    Click(f32, f32),
    Drag([f32; 2], [f32; 2]),
    Move(String),
    Mark(String),
    Drop(String),
    Preview(String),
    Ui(String),
    Tool(String),
    Touch(String),
    Page(String),
    Back,
    Forward,
    Diff(String),
    Commit(String),
    Theme(String),
    Share(String),
    Workspace(String),
    Agent(String),
    Menu(f32, f32),
    Interface,
    Delete(String),
    Rename(String),
    Ignored(String),
    Part(String),
    Find(String),
    Quit,
}

pub struct Script {
    pub steps: VecDeque<Step>,
    pub not_before: Instant,
}

fn parse_step(text: &str) -> Option<Step> {
    let text = text.trim();
    let (cmd, arg) = match text.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (text, ""),
    };
    Some(match cmd {
        "wait" => Step::Wait(arg.parse().ok()?),
        "fit" => Step::Fit,
        "locate" => Step::Locate,
        "goto" => Step::Goto(arg.into()),
        "select" => Step::Select(arg.into()),
        "search" => Step::Search(arg.into()),
        "left" => Step::Left(arg.into()),
        "install" => Step::Install(arg.into()),
        "ext-search" => Step::ExtSearch(arg.into()),
        "format" => Step::Format(arg.into()),
        "command" => Step::Command(arg.into()),
        "answer" => Step::Answer(arg.into()),
        "open" => Step::Open(arg.into()),
        "focus" => Step::Focus(arg.into()),
        "updates" => Step::Updates,
        "resize" => {
            let mut parts = arg.split_whitespace().map(|n| n.parse::<u32>());
            Step::Resize(parts.next()?.ok()?, parts.next()?.ok()?)
        }
        "palette" => Step::Palette(arg.into()),
        "edit" => Step::Edit(arg.into()),
        "type" => Step::Type(arg.into()),
        "key" => Step::Key(arg.into()),
        "git" => Step::Git(arg.into()),
        "changed" => Step::OnlyChanged(arg == "on"),
        "right" => Step::Right(arg.into()),
        "collapse-all" => Step::CollapseAll,
        "expand-all" => Step::ExpandAll,
        "connected" => Step::Connected(arg == "on"),
        "tab" => Step::Tab(arg.parse().ok()?),
        "folders" => Step::Folders,
        "zoom" => Step::Zoom(arg.parse().ok()?),
        "quick" => Step::Quick(arg.into()),
        "maximize" => Step::Maximize,
        "move" => Step::Move(arg.into()),
        "mark" => Step::Mark(arg.into()),
        "drop" => Step::Drop(arg.into()),
        "preview" => Step::Preview(arg.into()),
        "ui" => Step::Ui(arg.into()),
        "tool" => Step::Tool(arg.into()),
        "touch" => Step::Touch(arg.into()),
        "page" => Step::Page(arg.into()),
        "back" => Step::Back,
        "forward" => Step::Forward,
        "diff" => Step::Diff(arg.into()),
        "commit" => Step::Commit(arg.into()),
        "theme" => Step::Theme(arg.into()),
        "share" => Step::Share(arg.into()),
        "workspace" => Step::Workspace(arg.into()),
        "agent" => Step::Agent(arg.into()),
        "part" => Step::Part(arg.into()),
        "interface" => Step::Interface,
        "delete" => Step::Delete(arg.into()),
        "rename" => Step::Rename(arg.into()),
        "ignored" => Step::Ignored(arg.into()),
        "menu" => {
            let mut parts = arg.split_whitespace().map(|n| n.parse::<f32>());
            Step::Menu(parts.next()?.ok()?, parts.next()?.ok()?)
        }
        "find" => Step::Find(arg.into()),
        "drag" => {
            let mut parts = arg.split_whitespace().map(|n| n.parse::<f32>());
            let mut next = || parts.next()?.ok();
            Step::Drag([next()?, next()?], [next()?, next()?])
        }
        "click" => {
            let mut parts = arg.split_whitespace().map(|n| n.parse::<f32>());
            Step::Click(parts.next()?.ok()?, parts.next()?.ok()?)
        }
        "quit" => Step::Quit,
        _ => return None,
    })
}

impl Script {
    pub fn parse(text: &str) -> Script {
        let mut steps = VecDeque::new();
        for part in text.split(';').filter(|p| !p.trim().is_empty()) {
            match parse_step(part) {
                Some(s) => steps.push_back(s),
                None => log::warn!("script: ignoring unknown step '{}'", part.trim()),
            }
        }
        Script { steps, not_before: Instant::now() }
    }

    pub fn from_env() -> Option<Script> {
        let text = std::env::var("CODE_CANVAS_SCRIPT").ok()?;
        let script = Script::parse(&text);
        log::info!("script: {} steps", script.steps.len());
        Some(script)
    }

    pub fn next(&mut self, now: Instant) -> Option<Step> {
        if now < self.not_before {
            return None;
        }
        match self.steps.pop_front()? {
            Step::Wait(ms) => {
                self.not_before = now + Duration::from_millis(ms);
                None
            }
            step => {
                self.not_before = now + Duration::from_millis(120);
                Some(step)
            }
        }
    }

    pub fn is_done(&self) -> bool {
        self.steps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steps() {
        let s = Script::parse("goto src/a b.rs; wait 500 ;select main;connected on; bogus; zoom 1.5;quit");
        assert_eq!(
            s.steps,
            vec![
                Step::Goto("src/a b.rs".into()),
                Step::Wait(500),
                Step::Select("main".into()),
                Step::Connected(true),
                Step::Zoom(1.5),
                Step::Quit,
            ]
        );
    }
}
