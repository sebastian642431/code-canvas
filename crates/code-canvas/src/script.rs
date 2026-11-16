//! Scripted driving of the app, for tests and demos that must not touch the
//! real mouse or keyboard:
//!
//! ```text
//! CODE_CANVAS_SCRIPT="goto src/main.rs; wait 800; select main; locate" code-canvas <folder>
//! ```
//!
//! Steps are separated by `;` and run in order once the active page has
//! finished loading and the camera has settled. See [`Step`] for the list.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// `wait <ms>`
    Wait(u64),
    /// `fit`: show the whole canvas.
    Fit,
    /// `locate`: frame the selected symbols.
    Locate,
    /// `goto <path>`: select a file or folder and fly to it.
    Goto(String),
    /// `select <name>`: select the first symbol with that name.
    Select(String),
    /// `search <text>`: search in files.
    Search(String),
    /// `left files|search|help|none`
    Left(String),
    /// `right symbols|claude|none`
    Right(String),
    CollapseAll,
    ExpandAll,
    /// `connected on|off`: show only connected files.
    Connected(bool),
    /// `tab <n>` (1-based)
    Tab(usize),
    /// `folders`: show the list of opened folders.
    Folders,
    /// `zoom <factor>`
    Zoom(f32),
    /// `quick <text>`: open "go to file" with a query typed in.
    Quick(String),
    /// `chat <text>`: send a message to Claude Code.
    Chat(String),
    /// `maximize`
    Maximize,
    /// `quit`
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
        "right" => Step::Right(arg.into()),
        "collapse-all" => Step::CollapseAll,
        "expand-all" => Step::ExpandAll,
        "connected" => Step::Connected(arg == "on"),
        "tab" => Step::Tab(arg.parse().ok()?),
        "folders" => Step::Folders,
        "zoom" => Step::Zoom(arg.parse().ok()?),
        "quick" => Step::Quick(arg.into()),
        "chat" => Step::Chat(arg.into()),
        "maximize" => Step::Maximize,
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

    /// The next step to run now, if it is time. `Wait` steps are consumed here.
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
                // A short pause between steps lets each one draw.
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
