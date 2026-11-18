use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use serde::{Deserialize, Serialize};

use crate::term::{self, Launch, Mouse, Terminal};

/// The card every console starts in; the cards consoles are moved out to have other numbers.
pub const MAIN: u64 = 0;
const REMEMBERED: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bash,
    PowerShell,
    Cmd,
    Wsl,
    Other,
}

/// A kind of console this computer has: Git Bash, PowerShell, the Command Prompt, a WSL distribution...
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub program: PathBuf,
    pub args: Vec<String>,
}

const BASH_SCRIPT: &str = r#"if [ -n "$CODE_CANVAS_LOGIN" ]; then
  builtin unset CODE_CANVAS_LOGIN
  [ -r /etc/profile ] && . /etc/profile
  if [ -r ~/.bash_profile ]; then . ~/.bash_profile
  elif [ -r ~/.bash_login ]; then . ~/.bash_login
  elif [ -r ~/.profile ]; then . ~/.profile
  fi
elif [ -r ~/.bashrc ]; then
  . ~/.bashrc
fi
__cc_prompt() {
  local code=$?
  if [ "$PWD" != "$__cc_pwd" ]; then
    __cc_pwd=$PWD
    __cc_where=$(builtin pwd -W 2>/dev/null || builtin pwd)
  fi
  builtin printf '\e]633;D;%s\a\e]633;P;Cwd=%s\a\e]633;A\a' "$code" "$__cc_where"
  return $code
}
__cc_input() {
  local code=$?
  case "$PS1" in *'633;B'*) ;; *) PS1="$PS1"'\[\e]633;B\a\]' ;; esac
  return $code
}
__cc_user=$PROMPT_COMMAND
while [[ "$__cc_user" == *[\;\ ] ]]; do __cc_user=${__cc_user%?}; done
PROMPT_COMMAND="__cc_prompt${__cc_user:+; $__cc_user}; __cc_input"
builtin unset __cc_user
"#;

const POWERSHELL_SCRIPT: &str = r#"if (-not (Test-Path variable:global:__ccPrompt)) {
  $global:__ccPrompt = $function:prompt
  function global:prompt {
    $ok = $?
    $code = $global:LASTEXITCODE
    $e = [char]27; $b = [char]7
    $c = if ($ok) { 0 } elseif ($code -is [int] -and $code -ne 0) { $code } else { 1 }
    $here = $executionContext.SessionState.Path.CurrentLocation
    $cwd = if ($here.Provider.Name -eq 'FileSystem') { $here.ProviderPath } else { '' }
    $shown = & $global:__ccPrompt
    "$e]633;D;$c$b$e]633;P;Cwd=$cwd$b$e]633;A$b$shown$e]633;B$b"
  }
}
"#;

fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(LETTERS[(n >> (18 - 6 * i) & 63) as usize] as char),
                false => out.push('='),
            }
        }
    }
    out
}

impl Profile {
    pub fn icon(&self) -> &'static str {
        match self.kind {
            Kind::Bash => "terminal-bash",
            Kind::PowerShell => "terminal-powershell",
            Kind::Cmd => "terminal-cmd",
            Kind::Wsl => "terminal-linux",
            Kind::Other => "terminal",
        }
    }

    /// How it is started, with the script that makes it say where its prompts and commands are, written into
    /// `scripts` when it is not there yet.
    pub fn launch(&self, scripts: &Path) -> Launch {
        let mut launch = Launch { program: self.program.clone(), args: Vec::new(), env: Vec::new() };
        let own = |text: &str| OsString::from(text);
        match self.kind {
            Kind::Bash => {
                let script = scripts.join("bash-integration.sh");
                if std::fs::read_to_string(&script).ok().as_deref() != Some(BASH_SCRIPT) {
                    let _ = std::fs::create_dir_all(scripts);
                    if let Err(e) = std::fs::write(&script, BASH_SCRIPT) {
                        log::warn!("console: the script for bash was not written: {e}");
                    }
                }
                match script.is_file() {
                    true => {
                        let path = script.to_string_lossy().replace('\\', "/");
                        launch.args = vec![own("--init-file"), own(&path), own("-i")];
                        launch.env.push(("CODE_CANVAS_LOGIN".into(), "1".into()));
                    }
                    false => launch.args = vec![own("--login"), own("-i")],
                }
                launch.env.push(("CHERE_INVOKING".into(), "1".into()));
            }
            Kind::PowerShell => {
                let wide: Vec<u8> = POWERSHELL_SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
                launch.args = ["-NoLogo", "-NoExit", "-EncodedCommand"].map(own).to_vec();
                launch.args.push(own(&base64(&wide)));
            }
            Kind::Cmd => {
                let theirs = std::env::var("PROMPT").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "$P$G".into());
                let prompt = format!("$E]633;P;Cwd=$P$E\\$E]633;A$E\\{theirs}$E]633;B$E\\");
                launch.env.push(("PROMPT".into(), prompt));
            }
            Kind::Wsl | Kind::Other => launch.args = self.args.iter().map(|arg| own(arg)).collect(),
        }
        launch
    }

    /// What makes the shell show its prompt again on a cleared screen.
    pub fn redraw(&self) -> &'static [u8] {
        match self.kind {
            Kind::Bash | Kind::PowerShell | Kind::Wsl | Kind::Other => b"\x0c",
            Kind::Cmd => b"",
        }
    }

    /// A path as this shell reads it in a command.
    pub fn quote(&self, path: &Path) -> String {
        let text = path.to_string_lossy();
        match self.kind {
            Kind::Bash | Kind::Wsl | Kind::Other => format!("'{}'", text.replace('\\', "/").replace('\'', r"'\''")),
            Kind::PowerShell => format!("'{}'", text.replace('\'', "''")),
            Kind::Cmd => format!("\"{text}\""),
        }
    }

    pub fn cd(&self, dir: &Path) -> String {
        match self.kind {
            Kind::Cmd => format!("cd /d {}", self.quote(dir)),
            _ => format!("cd {}", self.quote(dir)),
        }
    }

    /// The command that runs a file, by its kind; `None` for one it does not know how to run.
    pub fn run_file(&self, file: &Path, shown: &Path) -> Option<String> {
        let ext = file.extension()?.to_string_lossy().to_lowercase();
        let path = self.quote(shown);
        let runner = match ext.as_str() {
            "py" => "python",
            "js" | "mjs" | "cjs" => "node",
            "ts" | "mts" | "cts" => "npx tsx",
            "sh" | "bash" => "bash",
            "rb" => "ruby",
            "php" => "php",
            "pl" => "perl",
            "lua" => "lua",
            "go" => "go run",
            "ps1" => match self.kind {
                Kind::PowerShell => "&",
                _ => "powershell -NoProfile -ExecutionPolicy Bypass -File",
            },
            "bat" | "cmd" if self.kind == Kind::Cmd => "call",
            _ => return None,
        };
        Some(format!("{runner} {path}"))
    }
}

fn found(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|place| place.is_file())
}

fn system() -> PathBuf {
    std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("System32")
}

fn profile(id: &str, name: &str, kind: Kind, program: PathBuf) -> Profile {
    Profile { id: id.into(), name: name.into(), kind, program, args: Vec::new() }
}

/// The consoles this computer has, but WSL's (asking WSL takes a while: [`wsl`]).
pub fn profiles() -> Vec<Profile> {
    let mut out = Vec::new();
    if let Some(bash) = term::bash() {
        out.push(profile("bash", if cfg!(windows) { "Git Bash" } else { "bash" }, Kind::Bash, bash));
    }
    if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        let seven = program_files.map(|dir| dir.join("PowerShell").join("7").join("pwsh.exe"));
        if let Some(pwsh) = on_path("pwsh.exe").or_else(|| seven.and_then(found)) {
            out.push(profile("pwsh", "PowerShell", Kind::PowerShell, pwsh));
        }
        if let Some(old) = found(system().join("WindowsPowerShell").join("v1.0").join("powershell.exe")) {
            out.push(profile("powershell", "Windows PowerShell", Kind::PowerShell, old));
        }
        let comspec = std::env::var_os("ComSpec").map(PathBuf::from).and_then(found);
        if let Some(cmd) = comspec.or_else(|| found(system().join("cmd.exe"))) {
            out.push(profile("cmd", "Command Prompt", Kind::Cmd, cmd));
        }
    } else {
        for name in ["zsh", "fish"] {
            let places =
                ["/bin", "/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"].map(|dir| Path::new(dir).join(name));
            if let Some(program) = places.into_iter().find(|place| place.is_file()).or_else(|| on_path(name)) {
                out.push(profile(name, name, Kind::Other, program));
            }
        }
    }
    out
}

/// The WSL distributions, by `wsl -l -q` (which writes UTF-16), when WSL is there.
pub fn wsl() -> Vec<Profile> {
    let program = system().join("wsl.exe");
    if !cfg!(windows) || !program.is_file() {
        return Vec::new();
    }
    let mut ask = std::process::Command::new(&program);
    ask.args(["-l", "-q"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        ask.creation_flags(0x0800_0000);
    }
    let Ok(out) = ask.output() else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    distributions(&out.stdout)
        .into_iter()
        .map(|name| Profile {
            id: format!("wsl:{name}"),
            name: format!("{name} (WSL)"),
            kind: Kind::Wsl,
            program: program.clone(),
            args: vec!["-d".into(), name],
        })
        .collect()
}

fn distributions(said: &[u8]) -> Vec<String> {
    let wide: Vec<u16> = said.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    String::from_utf16_lossy(&wide)
        .lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0').to_string())
        .filter(|name| !name.is_empty() && !name.starts_with("docker-desktop"))
        .collect()
}

/// One console: a shell of its own with its screen, in a card, side by side with the others of its group.
pub struct Tab {
    pub id: u64,
    pub card: u64,
    pub group: u64,
    pub name: Option<String>,
    pub profile: String,
    pub dir: PathBuf,
    pub color: Option<u8>,
    pub icon: Option<String>,
    pub term: Option<Terminal>,
    pub failed: Option<String>,
    /// It printed something while it was not shown.
    pub news: bool,
    /// A command in it, or the shell itself, ended badly while it was not shown.
    pub broke: bool,
    pub focus: bool,
    pub wheel: f32,
    pub mouse: Mouse,
}

impl Tab {
    fn new(id: u64, card: u64, profile: String, dir: PathBuf) -> Tab {
        Tab {
            id,
            card,
            group: id,
            name: None,
            profile,
            dir,
            color: None,
            icon: None,
            term: None,
            failed: None,
            news: false,
            broke: false,
            focus: true,
            wheel: 0.0,
            mouse: Mouse::default(),
        }
    }

    /// Where its shell is now, as it says, or where it started.
    pub fn cwd(&self) -> PathBuf {
        self.term.as_ref().and_then(|term| term.shell().cwd.clone()).unwrap_or_else(|| self.dir.clone())
    }

    /// Its name: the one given, the command running in it, or its folder's.
    pub fn title(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        if let Some(command) = self.term.as_ref().and_then(Terminal::command) {
            return command.to_string();
        }
        let cwd = self.cwd();
        cwd.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| cwd.display().to_string())
    }

    pub fn busy(&self) -> bool {
        self.term.as_ref().is_some_and(Terminal::busy)
    }

    pub fn ended(&self) -> bool {
        self.term.as_ref().is_some_and(|term| term.ended)
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SavedTab {
    pub id: u64,
    pub card: u64,
    pub group: u64,
    pub name: Option<String>,
    pub profile: String,
    pub dir: PathBuf,
    pub color: Option<u8>,
    pub icon: Option<String>,
}

/// What is remembered of the consoles from one start of the app to the next: no shell, no command run again.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Saved {
    pub tabs: Vec<SavedTab>,
    pub fronts: Vec<[u64; 2]>,
    pub current: Option<u64>,
    pub history: Vec<String>,
    pub dirs: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    New,
    More,
    Tab(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPage {
    Main,
    SplitWith,
    Default,
    Folders,
    Colour,
    Icon,
    Dirs,
    Commands,
}

pub struct Menu {
    pub card: u64,
    pub kind: MenuKind,
    pub page: MenuPage,
    pub at: egui::Pos2,
    pub place: Option<egui::Rect>,
}

#[derive(Default)]
pub struct Consoles {
    pub tabs: Vec<Tab>,
    fronts: BTreeMap<u64, u64>,
    /// The console the keyboard goes to.
    pub current: Option<u64>,
    next: u64,
    pub profiles: Vec<Profile>,
    pub looked: bool,
    asking_wsl: Option<Receiver<Vec<Profile>>>,
    installing: Option<Receiver<Result<(), String>>>,
    pub failed: Option<String>,
    pub scripts: PathBuf,
    pub history: Vec<String>,
    pub dirs: Vec<PathBuf>,
    /// The card that fills the canvas.
    pub filled: Option<u64>,
    pub menu: Option<Menu>,
    pub renaming: Option<(u64, String)>,
    pub rename_focused: bool,
    /// Where the header of each card was drawn, for a menu to open under it.
    pub heads: BTreeMap<u64, egui::Rect>,
    /// Cards that were left with no console, to be closed.
    pub emptied: Vec<u64>,
    pub changed: bool,
    /// The size of the screen drawn last, in letters: what a new shell is started with.
    pub size: Option<(u16, u16)>,
}

impl Consoles {
    fn fresh_id(&mut self) -> u64 {
        let used = self.tabs.iter().flat_map(|tab| [tab.id, tab.group, tab.card]).max().unwrap_or(0);
        self.next = self.next.max(used) + 1;
        self.next
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn current_mut(&mut self) -> Option<&mut Tab> {
        let id = self.current?;
        self.tab_mut(id)
    }

    pub fn of_card(&self, card: u64) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(move |tab| tab.card == card)
    }

    /// The console in front of a card: the one picked last, or its first.
    pub fn front(&self, card: u64) -> Option<u64> {
        let picked = self.fronts.get(&card).copied().filter(|id| self.tab(*id).is_some_and(|tab| tab.card == card));
        picked.or_else(|| self.of_card(card).next().map(|tab| tab.id))
    }

    /// The consoles shown side by side in a card: the group of the one in front.
    pub fn shown(&self, card: u64) -> Vec<u64> {
        let Some(group) = self.front(card).and_then(|id| self.tab(id)).map(|tab| tab.group) else { return Vec::new() };
        self.of_card(card).filter(|tab| tab.group == group).map(|tab| tab.id).collect()
    }

    pub fn open(&mut self, card: u64, profile: String, dir: PathBuf) -> u64 {
        let id = self.fresh_id();
        let at = self.tabs.iter().rposition(|tab| tab.card == card).map_or(self.tabs.len(), |at| at + 1);
        self.tabs.insert(at, Tab::new(id, card, profile, dir));
        self.bring(id);
        id
    }

    /// A new console beside `of`, in its group.
    pub fn split(&mut self, of: u64, profile: String, dir: PathBuf) -> Option<u64> {
        let (card, group) = self.tab(of).map(|tab| (tab.card, tab.group))?;
        let id = self.fresh_id();
        let at = self.tabs.iter().position(|tab| tab.id == of)? + 1;
        let mut tab = Tab::new(id, card, profile, dir);
        tab.group = group;
        self.tabs.insert(at, tab);
        self.bring(id);
        Some(id)
    }

    /// Puts a console in front of its card and gives it the keyboard.
    pub fn bring(&mut self, id: u64) {
        let Some(tab) = self.tab_mut(id) else { return };
        (tab.focus, tab.news, tab.broke) = (true, false, false);
        let card = tab.card;
        self.fronts.insert(card, id);
        self.current = Some(id);
        self.changed = true;
    }

    /// Takes a console away (its shell ends when the tab is dropped); the one beside it comes to the front.
    pub fn remove(&mut self, id: u64) -> Option<Tab> {
        let at = self.tabs.iter().position(|tab| tab.id == id)?;
        let tab = self.tabs.remove(at);
        let card = tab.card;
        let near = |within: &dyn Fn(&Tab) -> bool| {
            let after = self.tabs[at..].iter().find(|other| within(other));
            after.or_else(|| self.tabs[..at].iter().rev().find(|other| within(other))).map(|other| other.id)
        };
        let next =
            near(&|other| other.card == card && other.group == tab.group).or_else(|| near(&|other| other.card == card));
        if self.fronts.get(&card) == Some(&id) || !self.fronts.contains_key(&card) {
            match next {
                Some(next) => {
                    self.fronts.insert(card, next);
                }
                None => {
                    self.fronts.remove(&card);
                }
            }
        }
        if self.current == Some(id) {
            self.current = next.or_else(|| self.front(MAIN));
            if let Some(tab) = self.current.and_then(|now| self.tab_mut(now)) {
                tab.focus = true;
            }
        }
        if next.is_none() {
            self.emptied.push(card);
            if self.filled == Some(card) {
                self.filled = None;
            }
        }
        if self.renaming.as_ref().is_some_and(|(renamed, _)| *renamed == id) {
            self.renaming = None;
        }
        self.changed = true;
        Some(tab)
    }

    /// The next (or previous) console of the card of the current one, in the order of the list.
    pub fn cycle(&mut self, step: i32) {
        let Some(card) = self.current.and_then(|id| self.tab(id)).map(|tab| tab.card) else { return };
        let ids: Vec<u64> = self.of_card(card).map(|tab| tab.id).collect();
        let Some(at) = self.current.and_then(|id| ids.iter().position(|other| *other == id)) else { return };
        let to = (at as i64 + i64::from(step)).rem_euclid(ids.len() as i64) as usize;
        self.bring(ids[to]);
    }

    /// Takes a console out of its group, to be shown alone.
    pub fn unsplit(&mut self, id: u64) {
        let Some(tab) = self.tab(id) else { return };
        let (card, group) = (tab.card, tab.group);
        if self.of_card(card).filter(|other| other.group == group).count() < 2 {
            return;
        }
        let fresh = self.fresh_id();
        let at = self.tabs.iter().position(|other| other.id == id).unwrap_or_default();
        let mut tab = self.tabs.remove(at);
        tab.group = fresh;
        let last = self.tabs.iter().rposition(|other| other.card == card && other.group == group);
        let into = last.map_or(self.tabs.len(), |last| last + 1);
        self.tabs.insert(into, tab);
        self.bring(id);
    }

    /// Moves a console to a card of its own; the number of that card.
    pub fn move_out(&mut self, id: u64) -> Option<u64> {
        let old = self.tab(id)?.card;
        let card = self.fresh_id();
        let at = self.tabs.iter().position(|tab| tab.id == id)?;
        let mut tab = self.tabs.remove(at);
        (tab.card, tab.group) = (card, card);
        self.tabs.push(tab);
        if self.fronts.get(&old) == Some(&id) {
            self.fronts.remove(&old);
        }
        if self.of_card(old).next().is_none() {
            self.emptied.push(old);
        }
        self.bring(id);
        Some(card)
    }

    /// The consoles of a card that is closed go back to the first card.
    pub fn fold(&mut self, card: u64) {
        if card == MAIN {
            return;
        }
        let moved: Vec<u64> = self.of_card(card).map(|tab| tab.id).collect();
        let mut tabs: Vec<Tab> = Vec::new();
        for id in &moved {
            if let Some(at) = self.tabs.iter().position(|tab| tab.id == *id) {
                tabs.push(self.tabs.remove(at));
            }
        }
        for mut tab in tabs {
            tab.card = MAIN;
            self.tabs.push(tab);
        }
        self.fronts.remove(&card);
        if self.filled == Some(card) {
            self.filled = None;
        }
        if let Some(first) = moved.first() {
            self.fronts.insert(MAIN, *first);
        }
        self.changed = true;
    }

    pub fn rename(&mut self, id: u64, name: Option<String>) {
        if let Some(tab) = self.tab_mut(id) {
            tab.name = name.map(|name| name.trim().to_string()).filter(|name| !name.is_empty());
            self.changed = true;
        }
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// The profile new consoles get: the one picked in the settings when this computer has it, else the first.
    pub fn default_profile(&self, picked: &str) -> Option<&Profile> {
        self.profile(picked).or_else(|| self.profiles.first())
    }

    pub fn look(&mut self) {
        self.profiles = profiles();
        self.looked = true;
        if cfg!(windows) && self.asking_wsl.is_none() {
            let (said, heard) = mpsc::channel();
            self.asking_wsl = Some(heard);
            std::thread::spawn(move || {
                let _ = said.send(wsl());
                crate::ext::wake_ui();
            });
        }
    }

    /// Starts the shell of a console (again, when it had one).
    pub fn start(&mut self, id: u64) {
        if !self.looked {
            self.look();
        }
        let (scripts, size) = (self.scripts.clone(), self.size);
        let Some(tab) = self.tab(id) else { return };
        let profile = self.profile(&tab.profile).or_else(|| self.profiles.first()).cloned();
        let Some(tab) = self.tab_mut(id) else { return };
        let Some(profile) = profile else {
            tab.failed = Some("No shell was found on this computer.".into());
            return;
        };
        let (rows, cols) = tab.term.as_ref().map(Terminal::size).or(size).unwrap_or((24, 80));
        let dir = Some(tab.cwd()).filter(|dir| dir.is_dir()).or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir);
        tab.term = None;
        match Terminal::open(&profile.launch(&scripts), &dir, rows, cols) {
            Ok(term) => {
                (tab.term, tab.failed, tab.focus, tab.dir) = (Some(term), None, true, dir);
            }
            Err(why) => {
                log::warn!("console: {} did not start: {why}", profile.program.display());
                tab.failed = Some(why);
            }
        }
    }

    pub fn installing(&self) -> bool {
        self.installing.is_some()
    }

    pub fn install(&mut self) {
        if self.installing.is_some() {
            return;
        }
        let (done, heard) = mpsc::channel();
        self.installing = Some(heard);
        self.failed = None;
        std::thread::spawn(move || {
            let mut winget = std::process::Command::new("winget");
            winget.args(["install", "--id", "Git.Git", "-e", "--source", "winget"]);
            winget.args(["--accept-source-agreements", "--accept-package-agreements"]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                winget.creation_flags(0x0800_0000);
            }
            let ran = winget.stdin(std::process::Stdio::null()).output();
            let result = match ran {
                Ok(out) if out.status.success() => Ok(()),
                Ok(out) => {
                    let said = String::from_utf8_lossy(&out.stdout);
                    let last = said.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("it did not say why");
                    Err(format!("winget did not install Git: {}", last.trim()))
                }
                Err(e) => Err(format!("winget could not be run ({e})")),
            };
            let _ = done.send(result);
            crate::ext::wake_ui();
        });
    }

    /// Feeds every console what its shell printed and keeps their marks; tells when Git was installed.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        let mut ran = Vec::new();
        let mut dirs = Vec::new();
        for tab in self.tabs.iter_mut() {
            let Some(term) = tab.term.as_mut() else { continue };
            term.poll();
            tab.news |= std::mem::take(&mut term.news);
            let shell = term.shell_mut();
            tab.broke |= std::mem::take(&mut shell.failed) > 0;
            ran.append(&mut shell.ran);
            dirs.extend(shell.cwd.clone());
        }
        for command in ran {
            self.remember_command(command);
        }
        for dir in dirs {
            self.remember_dir(dir);
        }
        if let Some(more) = self.asking_wsl.as_ref().and_then(|heard| heard.try_recv().ok()) {
            self.asking_wsl = None;
            self.profiles.retain(|profile| profile.kind != Kind::Wsl);
            self.profiles.extend(more);
        }
        let done = self.installing.as_ref().and_then(|heard| heard.try_recv().ok())?;
        self.installing = None;
        self.look();
        Some(match (&done, self.profile("bash")) {
            (Ok(()), None) => {
                Err("Git was installed but bash is not where it is looked for: start the app again.".into())
            }
            _ => done,
        })
    }

    /// The consoles in front of the cards that are drawn were seen: their marks go.
    pub fn seen(&mut self, card: u64) {
        for id in self.shown(card) {
            if let Some(tab) = self.tab_mut(id) {
                (tab.news, tab.broke) = (false, false);
            }
        }
    }

    pub fn remember_command(&mut self, command: String) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }
        self.history.retain(|old| *old != command);
        self.history.insert(0, command);
        self.history.truncate(REMEMBERED);
        self.changed = true;
    }

    fn remember_dir(&mut self, dir: PathBuf) {
        if self.dirs.first() == Some(&dir) {
            return;
        }
        self.dirs.retain(|old| *old != dir);
        self.dirs.insert(0, dir);
        self.dirs.truncate(REMEMBERED);
        self.changed = true;
    }

    /// Every shell ends, with what it started.
    pub fn end_all(&mut self) {
        for tab in self.tabs.iter_mut() {
            tab.term = None;
        }
    }

    pub fn saved(&self) -> Saved {
        Saved {
            tabs: self
                .tabs
                .iter()
                .map(|tab| SavedTab {
                    id: tab.id,
                    card: tab.card,
                    group: tab.group,
                    name: tab.name.clone(),
                    profile: tab.profile.clone(),
                    dir: tab.cwd(),
                    color: tab.color,
                    icon: tab.icon.clone(),
                })
                .collect(),
            fronts: self.fronts.iter().map(|(card, id)| [*card, *id]).collect(),
            current: self.current,
            history: self.history.clone(),
            dirs: self.dirs.clone(),
        }
    }

    /// The consoles as they were left, with no shell yet: each starts when it is first shown.
    pub fn restore(&mut self, saved: &Saved) {
        self.tabs = saved
            .tabs
            .iter()
            .map(|kept| {
                let mut tab = Tab::new(kept.id, kept.card, kept.profile.clone(), kept.dir.clone());
                (tab.name, tab.color, tab.icon) = (kept.name.clone(), kept.color, kept.icon.clone());
                tab.group = if kept.group == 0 { kept.id } else { kept.group };
                tab.focus = false;
                tab
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        self.tabs.retain(|tab| seen.insert(tab.id));
        let mut order: Vec<Tab> = Vec::new();
        for tab in std::mem::take(&mut self.tabs) {
            match order.iter().rposition(|other| other.card == tab.card && other.group == tab.group) {
                Some(at) => order.insert(at + 1, tab),
                None => order.push(tab),
            }
        }
        self.tabs = order;
        self.fronts = saved.fronts.iter().map(|[card, id]| (*card, *id)).collect();
        self.current = saved.current.filter(|id| self.tab(*id).is_some()).or_else(|| self.front(MAIN));
        self.history = saved.history.clone();
        self.dirs = saved.dirs.clone();
        self.next = 0;
        self.changed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console() -> Consoles {
        let mut consoles = Consoles { looked: true, ..Default::default() };
        consoles.profiles = vec![profile("bash", "Git Bash", Kind::Bash, PathBuf::from("bash"))];
        consoles
    }

    fn cards(consoles: &Consoles) -> Vec<u64> {
        let mut cards: Vec<u64> = consoles.tabs.iter().map(|tab| tab.card).collect();
        cards.sort_unstable();
        cards.dedup();
        cards
    }

    fn names(consoles: &Consoles, card: u64) -> Vec<u64> {
        consoles.of_card(card).map(|tab| tab.id).collect()
    }

    #[test]
    fn consoles_open_close_and_take_turns_in_front() {
        let mut c = console();
        let (a, b, d) = (c.open(MAIN, "bash".into(), "x".into()), c.open(MAIN, "bash".into(), "y".into()), 0);
        let e = c.open(MAIN, "bash".into(), "z".into());
        assert_eq!(names(&c, MAIN), vec![a, b, e]);
        assert_eq!((c.front(MAIN), c.current), (Some(e), Some(e)), "a new one comes to the front");
        c.cycle(1);
        assert_eq!(c.current, Some(a), "next goes round");
        c.cycle(-1);
        assert_eq!(c.current, Some(e), "previous goes round the other way");
        c.bring(b);
        assert_eq!(c.shown(MAIN), vec![b]);
        assert!(c.remove(b).is_some());
        assert_eq!(c.front(MAIN), Some(e), "the one after it comes to the front");
        assert!(c.remove(e).is_some());
        assert_eq!((c.front(MAIN), c.current), (Some(a), Some(a)), "or the one before");
        assert!(c.remove(a).is_some() && c.front(MAIN).is_none() && c.current.is_none());
        assert_eq!(c.emptied, vec![MAIN]);
        assert!(c.remove(d).is_none());
    }

    #[test]
    fn a_split_shows_consoles_side_by_side_until_one_is_taken_out() {
        let mut c = console();
        let a = c.open(MAIN, "bash".into(), "x".into());
        let other = c.open(MAIN, "bash".into(), "x".into());
        let b = c.split(a, "bash".into(), "y".into()).unwrap();
        assert_eq!(names(&c, MAIN), vec![a, b, other], "the split one goes right after");
        assert_eq!(c.shown(MAIN), vec![a, b]);
        let d = c.split(b, "bash".into(), "y".into()).unwrap();
        assert_eq!(c.shown(MAIN), vec![a, b, d]);
        c.unsplit(b);
        assert_eq!(names(&c, MAIN), vec![a, d, b, other], "taken out of its group, it goes after it");
        assert_eq!(c.shown(MAIN), vec![b]);
        c.bring(d);
        assert_eq!(c.shown(MAIN), vec![a, d]);
        c.remove(d);
        assert_eq!(c.front(MAIN), Some(a), "the other of its group comes to the front");
        c.unsplit(a);
        assert_eq!(names(&c, MAIN), vec![a, b, other], "alone, nothing changes");
    }

    #[test]
    fn a_console_moved_to_a_card_of_its_own_comes_back_when_the_card_closes() {
        let mut c = console();
        let a = c.open(MAIN, "bash".into(), "x".into());
        let b = c.split(a, "bash".into(), "x".into()).unwrap();
        let card = c.move_out(b).unwrap();
        assert_ne!(card, MAIN);
        assert_eq!((names(&c, MAIN), names(&c, card)), (vec![a], vec![b]));
        assert_eq!(c.shown(MAIN), vec![a], "it left the group");
        assert_eq!(cards(&c), vec![MAIN, card]);
        let alone = c.move_out(a).unwrap();
        assert_eq!(c.emptied, vec![MAIN], "the first card has none left");
        c.fold(card);
        c.fold(alone);
        assert_eq!(names(&c, MAIN), vec![b, a]);
        assert_eq!(cards(&c), vec![MAIN]);
    }

    #[test]
    fn names_are_given_and_taken_back() {
        let mut c = console();
        let a = c.open(MAIN, "bash".into(), PathBuf::from("/work/shop"));
        assert_eq!(c.tab(a).unwrap().title(), "shop", "the folder's name");
        c.rename(a, Some("  server ".into()));
        assert_eq!(c.tab(a).unwrap().title(), "server");
        c.rename(a, Some("   ".into()));
        assert_eq!(c.tab(a).unwrap().title(), "shop", "an empty name gives the folder's back");
    }

    #[test]
    fn what_is_seen_loses_its_marks_and_what_is_behind_keeps_them() {
        let mut c = console();
        let a = c.open(MAIN, "bash".into(), "x".into());
        let b = c.open(MAIN, "bash".into(), "x".into());
        for id in [a, b] {
            let tab = c.tab_mut(id).unwrap();
            (tab.news, tab.broke) = (true, true);
        }
        c.seen(MAIN);
        let marks = |c: &Consoles, id| c.tab(id).map(|tab| (tab.news, tab.broke)).unwrap();
        assert_eq!((marks(&c, a), marks(&c, b)), ((true, true), (false, false)));
        c.bring(a);
        assert_eq!(marks(&c, a), (false, false), "brought to the front, it was seen");
    }

    #[test]
    fn the_consoles_are_remembered_and_come_back_in_their_order() {
        let mut c = console();
        let a = c.open(MAIN, "bash".into(), PathBuf::from("/one"));
        let b = c.open(MAIN, "cmd".into(), PathBuf::from("/two"));
        let d = c.split(a, "bash".into(), PathBuf::from("/three")).unwrap();
        c.rename(b, Some("server".into()));
        c.tab_mut(d).unwrap().color = Some(2);
        let card = c.move_out(b).unwrap();
        c.remember_command("npm run dev".into());
        c.remember_command("ls".into());
        c.remember_command("npm run dev".into());
        c.bring(d);
        let saved = c.saved();
        let text = serde_json::to_string(&saved).unwrap();
        let back: Saved = serde_json::from_str(&text).unwrap();
        assert_eq!(back, saved);

        let mut again = console();
        again.restore(&back);
        assert_eq!(names(&again, MAIN), vec![a, d]);
        assert_eq!(names(&again, card), vec![b]);
        assert_eq!(again.shown(MAIN), vec![a, d]);
        assert_eq!(again.current, Some(d));
        assert_eq!(again.tab(b).unwrap().title(), "server");
        assert_eq!(again.tab(d).unwrap().color, Some(2));
        assert_eq!(again.tab(b).unwrap().profile, "cmd");
        assert!(again.tabs.iter().all(|tab| tab.term.is_none()), "no shell until shown");
        assert_eq!(again.history, vec!["npm run dev".to_string(), "ls".to_string()]);
        let fresh = again.open(MAIN, "bash".into(), "x".into());
        assert!(![a, b, d, card].contains(&fresh), "new numbers do not take old ones");
    }

    #[test]
    fn a_session_from_before_the_consoles_reads() {
        let old = r#"{ "pages": [], "active": 0, "left_open": true, "cards": ["console"] }"#;
        let session: crate::session::Session = serde_json::from_str(old).unwrap();
        assert_eq!(session.consoles, Saved::default());
        let mut c = console();
        c.restore(&session.consoles);
        assert!(c.tabs.is_empty() && c.current.is_none());
        let half: Saved = serde_json::from_str(r#"{ "tabs": [{ "id": 4, "dir": "/x" }] }"#).unwrap();
        c.restore(&half);
        assert_eq!((c.front(MAIN), c.current, c.tab(4).unwrap().group), (Some(4), Some(4), 4));
    }

    #[test]
    fn the_shells_are_found_and_quoted() {
        let utf16: Vec<u8> =
            "Ubuntu\r\n\u{feff}docker-desktop\r\nDebian\r\n\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(distributions(&utf16), vec!["Ubuntu".to_string(), "Debian".to_string()]);
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        let bash = profile("bash", "Git Bash", Kind::Bash, "bash".into());
        let cmd = profile("cmd", "Command Prompt", Kind::Cmd, "cmd".into());
        let pwsh = profile("pwsh", "PowerShell", Kind::PowerShell, "pwsh".into());
        assert_eq!(bash.quote(Path::new(r"C:\it's\x")), r"'C:/it'\''s/x'");
        assert_eq!(pwsh.quote(Path::new(r"C:\it's")), r"'C:\it''s'");
        assert_eq!(cmd.cd(Path::new(r"D:\a b")), r#"cd /d "D:\a b""#);
        assert_eq!(bash.run_file(Path::new("x/run.py"), Path::new("run.py")).as_deref(), Some("python 'run.py'"));
        assert_eq!(pwsh.run_file(Path::new("go.ps1"), Path::new("go.ps1")).as_deref(), Some("& 'go.ps1'"));
        assert_eq!(bash.run_file(Path::new("x.txt"), Path::new("x.txt")), None);
        let launched = pwsh.launch(Path::new("unused"));
        assert_eq!(launched.args[..3], ["-NoLogo", "-NoExit", "-EncodedCommand"].map(OsString::from));
        let prompt = cmd.launch(Path::new("unused")).env.into_iter().find(|(name, _)| name == "PROMPT").unwrap().1;
        assert!(prompt.starts_with("$E]633;P;Cwd=$P$E\\") && prompt.ends_with("$E]633;B$E\\"));
    }
}
