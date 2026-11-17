use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Palette,
    GoToFile,
    SearchInFiles,
    FormatFile,
    GoToDefinition,
    OpenFolder,
    CloseFolder,
    NextFolder,
    PreviousFolder,
    Rescan,
    ToggleLeftPanel,
    ToggleSideBySide,
    Settings,
    ShowAll,
    GoToSelection,
    ZoomToParent,
    ZoomIn,
    ZoomOut,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Anywhere,
    Window,
    Canvas,
}

pub struct CommandInfo {
    pub command: Command,
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    pub scope: Scope,
    pub defaults: &'static [&'static str],
}

const fn info(
    command: Command,
    id: &'static str,
    title: &'static str,
    group: &'static str,
    scope: Scope,
    defaults: &'static [&'static str],
) -> CommandInfo {
    CommandInfo { command, id, title, group, scope, defaults }
}

pub const COMMANDS: &[CommandInfo] = &[
    info(Command::Palette, "commandPalette", "Command palette", "Find", Scope::Anywhere, &["ctrl+shift+p"]),
    info(Command::GoToFile, "goToFile", "Go to a file or folder", "Find", Scope::Anywhere, &["ctrl+p"]),
    info(Command::SearchInFiles, "searchInFiles", "Search in files", "Find", Scope::Anywhere, &["ctrl+shift+f"]),
    info(Command::GoToDefinition, "goToDefinition", "Go to the definition", "Find", Scope::Canvas, &["f12"]),
    info(Command::OpenFolder, "openFolder", "Open another folder", "Folders", Scope::Window, &["ctrl+o"]),
    info(Command::CloseFolder, "closeFolder", "Close the folder", "Folders", Scope::Window, &["ctrl+w"]),
    info(Command::NextFolder, "nextFolder", "Next folder", "Folders", Scope::Window, &["ctrl+tab"]),
    info(Command::PreviousFolder, "previousFolder", "Previous folder", "Folders", Scope::Window, &["ctrl+shift+tab"]),
    info(Command::Rescan, "rescan", "Reload the folder from disk", "Folders", Scope::Window, &["f5"]),
    info(
        Command::FormatFile,
        "formatFile",
        "Format the file under the pointer",
        "Editing",
        Scope::Window,
        &["shift+alt+f"],
    ),
    info(
        Command::ToggleLeftPanel,
        "toggleLeftPanel",
        "Show or hide the left panel",
        "Panels",
        Scope::Window,
        &["ctrl+b"],
    ),
    info(
        Command::ToggleSideBySide,
        "toggleSideBySide",
        "Every folder on the canvas, or one at a time",
        "Folders",
        Scope::Window,
        &["ctrl+e"],
    ),
    info(Command::Settings, "settings", "Settings", "Panels", Scope::Window, &["ctrl+,"]),
    info(Command::ShowAll, "showAll", "Show the whole canvas", "Canvas", Scope::Canvas, &["f", "home"]),
    info(Command::GoToSelection, "goToSelection", "Go to the selection", "Canvas", Scope::Canvas, &["l"]),
    info(
        Command::ZoomToParent,
        "zoomToParent",
        "Zoom out to the parent folder",
        "Canvas",
        Scope::Canvas,
        &["backspace"],
    ),
    info(Command::ZoomIn, "zoomIn", "Zoom in", "Canvas", Scope::Canvas, &["+", "=", "ctrl++", "ctrl+="]),
    info(Command::ZoomOut, "zoomOut", "Zoom out", "Canvas", Scope::Canvas, &["-", "ctrl+-"]),
    info(Command::PanLeft, "panLeft", "Move left", "Canvas", Scope::Canvas, &["left"]),
    info(Command::PanRight, "panRight", "Move right", "Canvas", Scope::Canvas, &["right"]),
    info(Command::PanUp, "panUp", "Move up", "Canvas", Scope::Canvas, &["up"]),
    info(Command::PanDown, "panDown", "Move down", "Canvas", Scope::Canvas, &["down"]),
];

impl Command {
    pub fn info(self) -> &'static CommandInfo {
        COMMANDS.iter().find(|c| c.command == self).expect("every command is in COMMANDS")
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

impl Chord {
    pub fn new(key: &str, ctrl: bool, shift: bool, alt: bool) -> Chord {
        let key = key.to_lowercase();
        let key = key.strip_prefix("arrow").filter(|rest| !rest.is_empty()).unwrap_or(&key).to_string();
        let symbol = key.chars().count() == 1 && !key.chars().all(char::is_alphanumeric);
        Chord { ctrl, shift: shift && !symbol, alt, key }
    }

    pub fn parse(text: &str) -> Option<Chord> {
        let mut rest = text.trim().to_lowercase();
        let (mut ctrl, mut shift, mut alt) = (false, false, false);
        loop {
            let found = [("ctrl+", 0), ("shift+", 1), ("alt+", 2)]
                .into_iter()
                .find(|(p, _)| rest.starts_with(p) && rest.len() > p.len());
            let Some((prefix, which)) = found else { break };
            match which {
                0 => ctrl = true,
                1 => shift = true,
                _ => alt = true,
            }
            rest = rest[prefix.len()..].to_string();
        }
        let valid = rest.chars().count() == 1 || (!rest.is_empty() && rest.chars().all(char::is_alphanumeric));
        valid.then(|| Chord::new(&rest, ctrl, shift, alt))
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, "ctrl+"), (self.shift, "shift+"), (self.alt, "alt+")] {
            if on {
                out.push_str(name);
            }
        }
        out + &self.key
    }

    pub fn label(&self) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, "Ctrl+"), (self.shift, "Shift+"), (self.alt, "Alt+")] {
            if on {
                out.push_str(name);
            }
        }
        let key = match self.key.as_str() {
            "left" => "←".to_string(),
            "right" => "→".to_string(),
            "up" => "↑".to_string(),
            "down" => "↓".to_string(),
            "-" => "−".to_string(),
            "pageup" => "Page Up".to_string(),
            "pagedown" => "Page Down".to_string(),
            k => {
                let mut c = k.chars();
                c.next().map(|first| first.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        };
        out + &key
    }

    pub fn is_typing(&self) -> bool {
        !self.ctrl && !self.alt && self.key.chars().count() == 1
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(transparent)]
pub struct KeyFile(pub BTreeMap<String, Vec<String>>);

impl KeyFile {
    pub fn chords(&self, command: Command) -> Vec<Chord> {
        let info = command.info();
        match self.0.get(info.id) {
            Some(list) => list.iter().filter_map(|t| Chord::parse(t)).collect(),
            None => info.defaults.iter().filter_map(|t| Chord::parse(t)).collect(),
        }
    }

    pub fn is_changed(&self, command: Command) -> bool {
        self.0.contains_key(command.info().id)
    }

    pub fn bind(&mut self, command: Command, chord: Chord) -> bool {
        if chord.is_typing() && command.info().scope != Scope::Canvas {
            return false;
        }
        for other in COMMANDS.iter().filter(|c| c.command != command) {
            let chords = self.chords(other.command);
            if chords.contains(&chord) {
                let left: Vec<Chord> = chords.into_iter().filter(|c| *c != chord).collect();
                self.store(other.command, &left);
            }
        }
        self.store(command, &[chord]);
        true
    }

    pub fn unbind(&mut self, command: Command) {
        self.store(command, &[]);
    }

    pub fn reset(&mut self, command: Command) {
        self.0.remove(command.info().id);
    }

    fn store(&mut self, command: Command, chords: &[Chord]) {
        let info = command.info();
        let own: Vec<Chord> = info.defaults.iter().filter_map(|t| Chord::parse(t)).collect();
        if own == chords {
            self.0.remove(info.id);
        } else {
            self.0.insert(info.id.to_string(), chords.iter().map(Chord::to_text).collect());
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    bound: Vec<(Chord, Command)>,
}

impl Keymap {
    pub fn new(file: &KeyFile) -> Keymap {
        let mut bound = Vec::new();
        for info in COMMANDS {
            for chord in file.chords(info.command) {
                if !bound.iter().any(|(c, _)| *c == chord) {
                    bound.push((chord, info.command));
                }
            }
        }
        Keymap { bound }
    }

    pub fn command(&self, chord: &Chord) -> Option<Command> {
        self.bound.iter().find(|(c, _)| c == chord).map(|(_, command)| *command)
    }

    pub fn chords(&self, command: Command) -> impl Iterator<Item = &Chord> {
        self.bound.iter().filter(move |(_, c)| *c == command).map(|(chord, _)| chord)
    }

    pub fn label(&self, command: Command) -> String {
        self.chords(command).next().map(Chord::label).unwrap_or_default()
    }

    pub fn hint(&self, text: &str, command: Command) -> String {
        match self.label(command) {
            key if key.is_empty() => text.to_string(),
            key => format!("{text} ({key})"),
        }
    }
}

fn path() -> PathBuf {
    crate::session::data_dir().join("keybindings.json")
}

pub fn load() -> KeyFile {
    match std::fs::read_to_string(path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            log::warn!("ignoring unreadable keybindings file: {e}");
            KeyFile::default()
        }),
        Err(_) => KeyFile::default(),
    }
}

pub fn save(file: &KeyFile) {
    let path = path();
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_string_pretty(file)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(&tmp, json))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        log::warn!("could not save the keybindings to {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).unwrap()
    }

    #[test]
    fn chords_parse_and_print() {
        assert_eq!(chord("ctrl+shift+p"), Chord { ctrl: true, shift: true, alt: false, key: "p".into() });
        assert_eq!(chord("F5"), Chord { ctrl: false, shift: false, alt: false, key: "f5".into() });
        assert_eq!(chord("ctrl++").key, "+");
        assert_eq!(chord("shift+alt+f").label(), "Shift+Alt+F");
        assert_eq!(chord("ctrl+tab").label(), "Ctrl+Tab");
        assert_eq!(chord("left").label(), "←");
        for text in ["ctrl+shift+p", "f1", "ctrl++", "shift+alt+f", "ctrl+,", "-"] {
            assert_eq!(chord(text).to_text(), text);
        }
        for bad in ["", "ctrl+", "ctrl+shift+", "two words", "ctrl+p+q"] {
            assert_eq!(Chord::parse(bad), None, "{bad:?} is not a chord");
        }
    }

    #[test]
    fn a_key_as_pressed_matches_how_it_is_written() {
        assert_eq!(Chord::new("P", true, true, false), chord("ctrl+shift+p"));
        assert_eq!(Chord::new("+", false, true, false), chord("+"));
        assert_eq!(Chord::new("ArrowLeft", false, false, false), chord("left"));
    }

    #[test]
    fn every_default_is_valid_and_unique() {
        let map = Keymap::new(&KeyFile::default());
        let mut seen: Vec<Chord> = Vec::new();
        for info in COMMANDS {
            assert!(!info.defaults.is_empty(), "{} has no shortcut", info.id);
            for text in info.defaults {
                let c = Chord::parse(text).unwrap_or_else(|| panic!("{text} is not a chord"));
                assert!(!seen.contains(&c), "{text} is given to two commands");
                assert!(!c.is_typing() || info.scope == Scope::Canvas, "{text} would be typed, not run");
                assert_eq!(map.command(&c), Some(info.command));
                seen.push(c);
            }
        }
        let mut ids: Vec<&str> = COMMANDS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), COMMANDS.len());
    }

    #[test]
    fn binding_takes_the_chord_from_its_owner() {
        let mut file = KeyFile::default();
        assert!(file.bind(Command::GoToFile, chord("ctrl+w")));
        let map = Keymap::new(&file);
        assert_eq!(map.command(&chord("ctrl+w")), Some(Command::GoToFile));
        assert_eq!(map.command(&chord("ctrl+p")), None);
        assert_eq!(map.label(Command::CloseFolder), "");
        assert_eq!(map.hint("Close", Command::CloseFolder), "Close");
        assert_eq!(map.hint("Go", Command::GoToFile), "Go (Ctrl+W)");

        file.reset(Command::GoToFile);
        file.reset(Command::CloseFolder);
        assert_eq!(file, KeyFile::default());
    }

    #[test]
    fn plain_letters_are_for_the_canvas_only() {
        let mut file = KeyFile::default();
        assert!(!file.bind(Command::GoToFile, chord("g")));
        assert!(file.bind(Command::ShowAll, chord("g")));
        assert!(file.bind(Command::GoToFile, chord("f2")));
        let expected: BTreeMap<String, Vec<String>> = [("showAll", "g"), ("goToFile", "f2")]
            .into_iter()
            .map(|(id, key)| (id.to_string(), vec![key.to_string()]))
            .collect();
        assert_eq!(file.0, expected);
    }

    #[test]
    fn the_file_reads_like_the_example() {
        let file: KeyFile =
            serde_json::from_str(r#"{ "goToFile": ["ctrl+e"], "rescan": [], "noSuchCommand": ["f9"] }"#).unwrap();
        let map = Keymap::new(&file);
        assert_eq!(map.command(&chord("ctrl+e")), Some(Command::GoToFile));
        assert_eq!(map.command(&chord("f5")), None);
        assert_eq!(map.command(&chord("f9")), None);
        assert!(file.is_changed(Command::Rescan));
        assert!(!file.is_changed(Command::ZoomIn));
    }
}
