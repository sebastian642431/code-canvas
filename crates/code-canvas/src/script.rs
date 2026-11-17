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
    /// `left files|search|extensions|problems|changes|settings|help|none`
    Left(String),
    /// `install <publisher.name>`: install an extension from Open VSX.
    Install(String),
    /// `ext-search <text>`: search the extension registry.
    ExtSearch(String),
    /// `format <path>`: format a file with the installed formatter.
    Format(String),
    /// `command <id>`: run an extension's command.
    Command(String),
    /// `resize <width> <height>`: resize the window (physical pixels).
    Resize(u32, u32),
    /// `palette <text>`: open the command palette with a filter typed in.
    Palette(String),
    /// `edit <path> <line> <column>` (1-based): put the caret in a file.
    Edit(String),
    /// `type <text>`: type at the caret.
    Type(String),
    /// `key enter|backspace|delete|tab|left|right|up|down|home|end|undo|redo|save|escape`
    Key(String),
    /// `git stage [path]`, `git unstage [path]`, `git commit <message>`.
    Git(String),
    /// `changed on|off`: show only the files git reports as changed.
    OnlyChanged(bool),
    /// `right symbols|<extension id>|none`
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
        "install" => Step::Install(arg.into()),
        "ext-search" => Step::ExtSearch(arg.into()),
        "format" => Step::Format(arg.into()),
        "command" => Step::Command(arg.into()),
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
