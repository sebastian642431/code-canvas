use std::io::Read;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const PORTS: &str = "agents";
const LARGEST: usize = 60_000;
const KEPT_OF_A_TEXT: usize = 600;
const QUIET_AFTER: Duration = Duration::from_secs(45);
const GONE_AFTER: Duration = Duration::from_secs(15 * 60);
const LEAVING: Duration = Duration::from_secs(6);
const MODEL_UNKNOWN_AGAIN: Duration = Duration::from_secs(3);
const MODEL_AGAIN: Duration = Duration::from_secs(30);
pub const TRAIL: usize = 12;
pub const TRAIL_LASTS: Duration = Duration::from_secs(10 * 60);

pub const FACES: usize = 50;
const REMEMBERED: usize = 600;
pub const COLORS: [u32; FACES] = [
    0xa039ffff, 0xff54adff, 0xff9144ff, 0xa336ffff, 0xff4155ff, 0x1afef7ff, 0x26b4ffff, 0xff28cbff, 0x28fcafff,
    0x2461f3ff, 0xff6b5aff, 0xffc11bff, 0x19ddffff, 0xfb1d42ff, 0xaa70ffff, 0x27f0eaff, 0x68c0ffff, 0x85ff27ff,
    0xff2822ff, 0x32e634ff, 0x1966ffff, 0xff8c6bff, 0x9a19ffff, 0xff20a1ff, 0xffcd19ff, 0x19fff8ff, 0x19ff62ff,
    0xe2498bff, 0x19fcffff, 0x1957ffff, 0xffbe19ff, 0x1969ffff, 0x23f53eff, 0xff7419ff, 0x2b91ecff, 0xff1933ff,
    0x2448f4ff, 0xe928f0ff, 0xff4b19ff, 0x1989ffff, 0xffcc19ff, 0x5f89b9ff, 0x9ee137ff, 0xff3219ff, 0x6dff19ff,
    0xe234f9ff, 0x4d26ffff, 0x4d21f7ff, 0x1960ffff, 0x1bfcb9ff,
];

fn shortened(value: &Value, depth: u8) -> Value {
    match value {
        Value::String(text) if text.chars().count() > KEPT_OF_A_TEXT => {
            Value::String(text.chars().take(KEPT_OF_A_TEXT).collect())
        }
        Value::Array(items) => Value::Array(items.iter().take(6).map(|item| shortened(item, depth + 1)).collect()),
        Value::Object(_) if depth > 3 => Value::Null,
        Value::Object(fields) => {
            let kept = fields.iter().filter(|(key, _)| key.as_str() != "tool_response" || depth > 0);
            Value::Object(kept.map(|(key, value)| (key.clone(), shortened(value, depth + 1))).collect())
        }
        other => other.clone(),
    }
}

fn ports_dir() -> PathBuf {
    crate::session::data_dir().join(PORTS)
}

fn listening() -> Vec<u16> {
    let Ok(files) = std::fs::read_dir(ports_dir()) else { return Vec::new() };
    files.flatten().filter_map(|file| file.file_name().to_str()?.parse::<u16>().ok()).take(16).collect()
}

pub fn hook(source: &str) -> i32 {
    let mut said = String::new();
    if std::io::stdin().take(8 << 20).read_to_string(&mut said).is_err() {
        return 0;
    }
    let Ok(event) = serde_json::from_str::<Value>(&said) else { return 0 };
    let ports = listening();
    if ports.is_empty() {
        return 0;
    }
    let mut packed = json!({ "source": source, "hook": shortened(&event, 0) }).to_string();
    if packed.len() > LARGEST {
        let mut small = event.clone();
        if let Some(fields) = small.get_mut("tool_input").and_then(Value::as_object_mut) {
            fields.retain(|_, value| value.as_str().is_none_or(|text| text.len() < 400));
        }
        packed = json!({ "source": source, "hook": shortened(&small, 0) }).to_string();
    }
    if packed.len() <= LARGEST {
        if let Ok(socket) = UdpSocket::bind(("127.0.0.1", 0)) {
            for port in ports {
                let _ = socket.send_to(packed.as_bytes(), ("127.0.0.1", port));
            }
        }
    }
    0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doing {
    Reading,
    Editing,
    Searching,
    Running,
    Thinking,
    Waiting,
    Leaving,
}

const DOINGS: [Doing; 7] =
    [Doing::Reading, Doing::Editing, Doing::Searching, Doing::Running, Doing::Thinking, Doing::Waiting, Doing::Leaving];

impl Doing {
    pub fn code(self) -> u8 {
        DOINGS.iter().position(|doing| *doing == self).unwrap_or(4) as u8
    }

    pub fn of_code(code: u8) -> Doing {
        DOINGS.get(code as usize).copied().unwrap_or(Doing::Thinking)
    }

    pub fn said(self) -> &'static str {
        match self {
            Doing::Reading => "reading",
            Doing::Editing => "editing",
            Doing::Searching => "searching",
            Doing::Running => "running a command",
            Doing::Thinking => "thinking",
            Doing::Waiting => "waiting for you",
            Doing::Leaving => "done",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Agent {
    pub key: String,
    pub session: String,
    pub avatar: usize,
    pub role: String,
    pub task: String,
    pub doing: Doing,
    pub note: String,
    pub path: Option<PathBuf>,
    pub lines: Option<(u32, u32)>,
    pub find: Option<String>,
    pub at: Instant,
    pub moved: u64,
    pub trail: Vec<Step>,
    pub from: Option<u32>,
    pub model: String,
    pub transcript: Option<PathBuf>,
    pub model_asked: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub doing: Doing,
    pub path: PathBuf,
    pub lines: Option<(u32, u32)>,
    pub note: String,
    pub at: Instant,
    pub moved: u64,
}

impl Step {
    pub fn left(&self) -> f32 {
        1.0 - (self.at.elapsed().as_secs_f32() / TRAIL_LASTS.as_secs_f32()).min(1.0)
    }
}

impl Agent {
    pub fn name(&self) -> String {
        match self.task.is_empty() {
            true => self.role.clone(),
            false => self.task.clone(),
        }
    }

    pub fn color(&self) -> u32 {
        COLORS[self.avatar % COLORS.len()]
    }

    pub fn quiet(&self) -> bool {
        self.at.elapsed() > QUIET_AFTER || matches!(self.doing, Doing::Waiting | Doing::Leaving)
    }

    pub fn busy(&self) -> bool {
        self.at.elapsed() < Duration::from_millis(1600) && !matches!(self.doing, Doing::Waiting | Doing::Leaving)
    }

    fn stepped(&mut self, again: bool) {
        let Some(path) = self.path.clone() else { return };
        if !matches!(self.doing, Doing::Reading | Doing::Editing | Doing::Searching) {
            return;
        }
        let (lines, note, at, moved) = (self.lines, self.note.clone(), Instant::now(), self.moved);
        let step = Step { doing: self.doing, path, lines, note, at, moved };
        let same =
            |last: &Step| last.path == step.path && last.doing == step.doing && (again || last.lines == step.lines);
        if let Some(last) = self.trail.last_mut().filter(|last| same(last)) {
            last.lines = step.lines.or(last.lines);
            (last.note, last.at, last.moved) = (step.note, step.at, step.moved);
            return;
        }
        self.trail.push(step);
        if self.trail.len() > TRAIL {
            self.trail.remove(0);
        }
    }

    pub fn found(&mut self, line: u32) {
        self.lines = Some((line, 1));
        if let Some(last) = self.trail.last_mut().filter(|last| last.moved == self.moved) {
            last.lines = self.lines;
        }
    }

    fn set_model(&mut self, model: &str) {
        if self.model != model {
            log::info!("an assistant ({}) works with {model}", self.role);
            self.model = model.to_string();
        }
    }

    fn forget_old_steps(&mut self) -> bool {
        let before = self.trail.len();
        self.trail.retain(|step| step.at.elapsed() < TRAIL_LASTS);
        before != self.trail.len()
    }
}

pub const AVATARS: [&[u8]; FACES] = [
    include_bytes!("../assets/agents/00.png"),
    include_bytes!("../assets/agents/01.png"),
    include_bytes!("../assets/agents/02.png"),
    include_bytes!("../assets/agents/03.png"),
    include_bytes!("../assets/agents/04.png"),
    include_bytes!("../assets/agents/05.png"),
    include_bytes!("../assets/agents/06.png"),
    include_bytes!("../assets/agents/07.png"),
    include_bytes!("../assets/agents/08.png"),
    include_bytes!("../assets/agents/09.png"),
    include_bytes!("../assets/agents/10.png"),
    include_bytes!("../assets/agents/11.png"),
    include_bytes!("../assets/agents/12.png"),
    include_bytes!("../assets/agents/13.png"),
    include_bytes!("../assets/agents/14.png"),
    include_bytes!("../assets/agents/15.png"),
    include_bytes!("../assets/agents/16.png"),
    include_bytes!("../assets/agents/17.png"),
    include_bytes!("../assets/agents/18.png"),
    include_bytes!("../assets/agents/19.png"),
    include_bytes!("../assets/agents/20.png"),
    include_bytes!("../assets/agents/21.png"),
    include_bytes!("../assets/agents/22.png"),
    include_bytes!("../assets/agents/23.png"),
    include_bytes!("../assets/agents/24.png"),
    include_bytes!("../assets/agents/25.png"),
    include_bytes!("../assets/agents/26.png"),
    include_bytes!("../assets/agents/27.png"),
    include_bytes!("../assets/agents/28.png"),
    include_bytes!("../assets/agents/29.png"),
    include_bytes!("../assets/agents/30.png"),
    include_bytes!("../assets/agents/31.png"),
    include_bytes!("../assets/agents/32.png"),
    include_bytes!("../assets/agents/33.png"),
    include_bytes!("../assets/agents/34.png"),
    include_bytes!("../assets/agents/35.png"),
    include_bytes!("../assets/agents/36.png"),
    include_bytes!("../assets/agents/37.png"),
    include_bytes!("../assets/agents/38.png"),
    include_bytes!("../assets/agents/39.png"),
    include_bytes!("../assets/agents/40.png"),
    include_bytes!("../assets/agents/41.png"),
    include_bytes!("../assets/agents/42.png"),
    include_bytes!("../assets/agents/43.png"),
    include_bytes!("../assets/agents/44.png"),
    include_bytes!("../assets/agents/45.png"),
    include_bytes!("../assets/agents/46.png"),
    include_bytes!("../assets/agents/47.png"),
    include_bytes!("../assets/agents/48.png"),
    include_bytes!("../assets/agents/49.png"),
];

const HOOKED: [(&str, bool); 8] = [
    ("PreToolUse", true),
    ("PostToolUse", true),
    ("UserPromptSubmit", false),
    ("Stop", false),
    ("SubagentStart", false),
    ("SubagentStop", false),
    ("SessionStart", false),
    ("SessionEnd", false),
];
const HOOK_FLAG: &str = "--agent-hook";

fn claude_settings() -> Option<PathBuf> {
    let named = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()).map(PathBuf::from);
    Some(named.or_else(|| Some(dirs::home_dir()?.join(".claude")))?.join("settings.json"))
}

fn is_ours(hook: &Value) -> bool {
    let args = hook.get("args").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    args.iter().any(|arg| arg.as_str() == Some(HOOK_FLAG))
}

fn without_ours(entries: &mut Vec<Value>) -> bool {
    let mut removed = false;
    entries.retain_mut(|entry| {
        let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
        let before = hooks.len();
        hooks.retain(|hook| !is_ours(hook));
        removed |= hooks.len() != before;
        hooks.len() == before || !hooks.is_empty()
    });
    removed
}

pub fn with_hooks(settings: &Value, program: Option<&str>) -> Result<Value, String> {
    let mut settings = settings.clone();
    let Some(all) = settings.as_object_mut() else { return Err("it is not a JSON object".to_string()) };
    if program.is_none() && !all.contains_key("hooks") {
        return Ok(settings);
    }
    let hooks = all.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else { return Err("its \"hooks\" is not an object".to_string()) };
    let mut took = false;
    for (event, of_tools) in HOOKED {
        let removed = match hooks.get_mut(event) {
            Some(Value::Array(entries)) => without_ours(entries),
            Some(_) => return Err(format!("its hooks for {event} are not a list")),
            None => false,
        };
        took |= removed;
        match program {
            Some(program) => {
                let run = json!({ "type": "command", "command": program, "args": [HOOK_FLAG, "claude"], "async": true, "timeout": 5 });
                let entry = match of_tools {
                    true => json!({ "matcher": "*", "hooks": [run] }),
                    false => json!({ "hooks": [run] }),
                };
                if let Some(entries) = hooks.entry(event).or_insert_with(|| json!([])).as_array_mut() {
                    entries.push(entry);
                }
            }
            None => {
                if removed && hooks.get(event).and_then(Value::as_array).is_some_and(Vec::is_empty) {
                    hooks.shift_remove(event);
                }
            }
        }
    }
    if program.is_none() && took && hooks.is_empty() {
        all.shift_remove("hooks");
    }
    Ok(settings)
}

fn read_settings(file: &Path) -> Result<Value, String> {
    let untouched = |why: String| format!("{} {why}: it was left as it is", file.display());
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(e) => return Err(untouched(format!("could not be read ({e})"))),
    };
    let text = String::from_utf8(bytes).map_err(|_| untouched("is not UTF-8".to_string()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(text).map_err(|e| untouched(format!("is not readable as JSON ({e})")))
}

pub fn claude_there() -> bool {
    claude_settings().and_then(|file| Some(file.parent()?.is_dir())).unwrap_or(false)
}

pub fn claude_hook_program() -> Option<String> {
    let settings = read_settings(&claude_settings()?).ok()?;
    let entries = settings.pointer("/hooks/PreToolUse")?.as_array()?;
    let ours =
        entries.iter().filter_map(|entry| entry.get("hooks")?.as_array()).flatten().find(|hook| is_ours(hook))?;
    Some(ours.get("command")?.as_str()?.to_string())
}

pub fn claude_connected() -> bool {
    claude_hook_program().is_some()
}

pub fn own_program() -> Option<String> {
    Some(std::env::current_exe().ok()?.to_string_lossy().into_owned())
}

pub fn connect_claude(on: bool) -> Result<(), String> {
    let file = claude_settings().ok_or("the folder of Claude Code's settings was not found")?;
    let settings = read_settings(&file)?;
    let program = own_program().ok_or("the path of this program is not known")?;
    let changed = with_hooks(&settings, on.then_some(program.as_str()))
        .map_err(|why| format!("{}: {why}. It was left as it is", file.display()))?;
    if changed == settings {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let kept = file.with_extension("json.before-code-canvas");
    if file.is_file() && !kept.exists() {
        std::fs::copy(&file, &kept)
            .map_err(|e| format!("a copy of {} could not be kept ({e}): nothing was changed", file.display()))?;
    }
    let text = serde_json::to_string_pretty(&changed).map_err(|e| e.to_string())?;
    let fresh = file.with_extension("json.code-canvas-new");
    let written = std::fs::write(&fresh, text + "\n").and_then(|_| std::fs::rename(&fresh, &file));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&fresh);
        format!("{} could not be written ({e}): it was left as it is", file.display())
    })
}

pub struct Agents {
    heard: Option<Receiver<Value>>,
    tell: Option<Sender<Value>>,
    codex: Option<crate::codex::Tail>,
    port: Option<u16>,
    pub list: Vec<Agent>,
    pub following: Option<String>,
    moves: u64,
    faces: Vec<(String, usize)>,
    faces_changed: bool,
    handed: std::collections::HashMap<String, std::collections::VecDeque<String>>,
    models: Option<crate::models::Reader>,
}

fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key).and_then(Value::as_str)).filter(|text| !text.is_empty())
}

fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| line.len() > 3).unwrap_or(text.trim());
    line.chars().take(200).collect()
}

fn spoken(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => {
                inside = false;
                plain.push(' ');
            }
            c if !inside => plain.push(c),
            _ => {}
        }
    }
    let said = plain.lines().map(str::trim).find(|line| line.chars().filter(|c| c.is_alphanumeric()).count() > 3);
    said.map(|line| line.chars().take(200).collect()).unwrap_or_default()
}

fn brief(text: &str, most: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one.chars().count() > most {
        true => format!("{}…", one.chars().take(most).collect::<String>()),
        false => one,
    }
}

impl Agents {
    pub fn listen() -> Agents {
        let _ = std::fs::remove_file(crate::session::data_dir().join("agents.port"));
        for port in listening() {
            if UdpSocket::bind(("127.0.0.1", port)).is_ok() {
                let _ = std::fs::remove_file(ports_dir().join(port.to_string()));
            }
        }
        let socket = UdpSocket::bind(("127.0.0.1", 0)).ok();
        let port = socket.as_ref().and_then(|socket| socket.local_addr().ok()).map(|address| address.port());
        let (tell, heard) = mpsc::channel();
        match port {
            Some(port) => {
                let _ = std::fs::create_dir_all(ports_dir());
                let _ = std::fs::write(ports_dir().join(port.to_string()), "");
                let tx = tell.clone();
                std::thread::spawn(move || {
                    let (Some(socket), mut bytes) = (socket, vec![0u8; 65_536]) else { return };
                    while let Ok((length, _)) = socket.recv_from(&mut bytes) {
                        let Ok(event) = serde_json::from_slice::<Value>(&bytes[..length]) else { continue };
                        if tx.send(event).is_err() {
                            return;
                        }
                        crate::ext::wake_ui();
                    }
                });
            }
            None => log::warn!("assistants other than Codex will not be seen on the canvas: no port to listen on"),
        }
        let faces = std::fs::read_to_string(crate::session::data_dir().join("faces.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<(String, usize)>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, avatar)| *avatar < FACES)
            .collect();
        Agents {
            heard: Some(heard),
            tell: Some(tell),
            codex: None,
            port,
            list: Vec::new(),
            following: None,
            moves: 0,
            faces,
            faces_changed: false,
            handed: Default::default(),
            models: None,
        }
    }

    /// Codex is followed or let go: its session files are read as they grow (`codex.rs`).
    pub fn follow_codex(&mut self, on: bool) {
        match (on, self.codex.is_some()) {
            (true, false) => {
                let home = crate::codex::home();
                self.codex =
                    self.tell.clone().zip(home).and_then(|(tell, home)| crate::codex::Tail::start(&home, tell));
            }
            (false, true) => {
                self.codex = None;
                log::info!("Codex is no longer followed");
                let theirs = self
                    .list
                    .iter_mut()
                    .filter(|agent| agent.from.is_none() && agent.key.starts_with(crate::codex::KEY));
                for agent in theirs {
                    (agent.doing, agent.at) = (Doing::Leaving, Instant::now());
                }
            }
            _ => {}
        }
    }

    pub fn close(&mut self) {
        self.codex = None;
        if let Some(port) = self.port.take() {
            let _ = std::fs::remove_file(ports_dir().join(port.to_string()));
        }
    }

    fn face_of(&mut self, key: &str) -> usize {
        if let Some(at) = self.faces.iter().position(|(kept, _)| kept == key) {
            let kept = self.faces.remove(at);
            self.faces.push(kept.clone());
            return kept.1;
        }
        let in_use = |avatar: usize| self.list.iter().any(|agent| agent.avatar == avatar);
        let kept_by = |avatar: usize| self.faces.iter().rposition(|(_, kept)| *kept == avatar);
        let avatar = (0..FACES)
            .filter(|avatar| !in_use(*avatar))
            .min_by_key(|avatar| kept_by(*avatar).map_or(0, |at| at + 1))
            .unwrap_or(self.list.len() % FACES);
        self.faces.push((key.to_string(), avatar));
        if self.faces.len() > REMEMBERED {
            self.faces.remove(0);
        }
        self.faces_changed = true;
        avatar
    }

    fn save_faces(&mut self) {
        if !std::mem::take(&mut self.faces_changed) {
            return;
        }
        let file = crate::session::data_dir().join("faces.json");
        if let Err(why) = std::fs::write(&file, serde_json::to_string(&self.faces).unwrap_or_default()) {
            log::warn!("the faces of the assistants could not be kept: {why}");
        }
    }

    pub fn find(&self, key: &str) -> Option<&Agent> {
        self.list.iter().find(|agent| agent.key == key)
    }

    fn agent(&mut self, key: &str, session: &str, role: &str) -> &mut Agent {
        if let Some(at) = self.list.iter().position(|agent| agent.key == key && agent.from.is_none()) {
            return &mut self.list[at];
        }
        let avatar = self.face_of(key);
        log::info!("an assistant ({role}) is on the canvas with face {avatar}");
        let task = match key.split_once('/') {
            Some(_) => self.handed.get_mut(session).and_then(|tasks| tasks.pop_front()).unwrap_or_default(),
            None => String::new(),
        };
        self.list.push(Agent {
            key: key.to_string(),
            session: session.to_string(),
            avatar,
            role: role.to_string(),
            task,
            doing: Doing::Thinking,
            note: String::new(),
            path: None,
            lines: None,
            find: None,
            at: Instant::now(),
            moved: 0,
            trail: Vec::new(),
            from: None,
            model: String::new(),
            transcript: None,
            model_asked: None,
        });
        self.list.last_mut().unwrap()
    }

    fn ask_model(&mut self, key: &str, transcript: PathBuf) {
        let Some(tell) = self.tell.clone() else { return };
        if self.models.is_none() {
            self.models = crate::models::Reader::start(tell);
        }
        if let Some(reader) = &self.models {
            reader.ask(key, transcript);
        }
    }

    pub fn poll(&mut self) -> bool {
        let heard: Vec<Value> = self.heard.as_ref().map(|heard| heard.try_iter().collect()).unwrap_or_default();
        let before = self.list.len();
        let mut changed = !heard.is_empty();
        for event in heard {
            self.hear(&event);
        }
        self.list.retain(|agent| match agent.doing {
            Doing::Leaving => agent.at.elapsed() < LEAVING,
            _ => agent.at.elapsed() < GONE_AFTER,
        });
        changed |= before != self.list.len();
        for agent in &mut self.list {
            changed |= agent.forget_old_steps();
        }
        self.save_faces();
        if self.following.as_ref().is_some_and(|key| self.find(key).is_none()) {
            self.following = None;
        }
        changed
    }

    pub fn lively(&self) -> bool {
        self.list.iter().any(|agent| agent.at.elapsed() < Duration::from_secs(3))
    }

    pub fn hear(&mut self, event: &Value) {
        if let Some(key) = text(event, &["model_of"]) {
            let own = self.list.iter_mut().find(|agent| agent.key == key && agent.from.is_none());
            if let (Some(agent), Some(model)) = (own, text(event, &["model"])) {
                agent.set_model(model);
            }
            return;
        }
        match event.get("hook") {
            Some(hook) => self.hear_hook(text(event, &["source"]).unwrap_or("assistant"), hook),
            None => self.hear_plain(event),
        }
    }

    fn hear_plain(&mut self, event: &Value) {
        let Some(key) = text(event, &["agent"]) else { return };
        let session = text(event, &["session"]).unwrap_or(key);
        let role = text(event, &["name"]).unwrap_or("assistant").to_string();
        self.moves += 1;
        let moves = self.moves;
        let agent = self.agent(key, session, &role);
        agent.role = role;
        agent.at = Instant::now();
        if let Some(task) = text(event, &["task"]) {
            agent.task = brief(&spoken(task), 48);
        }
        agent.note = text(event, &["note"]).map(|note| brief(note, 90)).unwrap_or_default();
        if let Some(model) = text(event, &["model"]) {
            agent.set_model(model);
        }
        agent.doing = match text(event, &["action"]).unwrap_or("read") {
            "edit" | "write" => Doing::Editing,
            "search" => Doing::Searching,
            "run" => Doing::Running,
            "think" => Doing::Thinking,
            "wait" => Doing::Waiting,
            "done" | "leave" => Doing::Leaving,
            _ => Doing::Reading,
        };
        if let Some(path) = text(event, &["path"]) {
            let line = event.get("line").and_then(Value::as_u64).map(|line| line.saturating_sub(1) as u32);
            let lines = event.get("lines").and_then(Value::as_u64).unwrap_or(1).clamp(1, 400) as u32;
            agent.path = Some(PathBuf::from(path));
            agent.lines = line.map(|line| (line.min(u32::MAX / 2), lines));
            agent.find =
                text(event, &["find"]).map(first_line).filter(|line| !line.is_empty() && agent.lines.is_none());
            agent.moved = moves;
            agent.stepped(false);
        }
    }

    fn hear_hook(&mut self, source: &str, hook: &Value) {
        let Some(session) = text(hook, &["session_id"]) else { return };
        let inner = text(hook, &["agent_id", "subagent_id"]);
        let key = match inner {
            Some(inner) => format!("{session}/{inner}"),
            None => session.to_string(),
        };
        let role = match (inner, text(hook, &["agent_type", "subagent_type"])) {
            (Some(_), Some(kind)) => kind.to_string(),
            (Some(_), None) => "agent".to_string(),
            (None, _) => match source {
                "claude" => "Claude Code".to_string(),
                other => other.to_string(),
            },
        };
        let event = text(hook, &["hook_event_name"]).unwrap_or_default();
        if event == "SessionEnd" {
            for agent in self.list.iter_mut().filter(|agent| agent.session == session) {
                (agent.doing, agent.at) = (Doing::Leaving, Instant::now());
            }
            return;
        }
        let of_tool = matches!(event, "PreToolUse" | "PostToolUse");
        let left = self.find(&key).is_some_and(|agent| agent.doing == Doing::Leaving);
        if (of_tool && left) || (event == "SubagentStop" && inner.is_none()) {
            return;
        }
        let cwd = text(hook, &["cwd"]).map(PathBuf::from);
        self.moves += 1;
        let moves = self.moves;
        let handed = match (event, text(hook, &["tool_name"])) {
            ("PreToolUse", Some("Agent" | "Task")) => {
                hook.get("tool_input").and_then(|input| text(input, &["description"]))
            }
            _ => None,
        };
        if let Some(task) = handed {
            self.handed.entry(session.to_string()).or_default().push_back(brief(&spoken(task), 48));
        }
        let transcript = match inner {
            None => text(hook, &["transcript_path"]).map(PathBuf::from),
            Some(inner) => text(hook, &["agent_transcript_path"]).map(PathBuf::from).or_else(|| {
                text(hook, &["transcript_path"]).map(|main| crate::models::of_helper(Path::new(main), inner))
            }),
        };
        let agent = self.agent(&key, session, &role);
        agent.at = Instant::now();
        if transcript.is_some() {
            agent.transcript = transcript;
        }
        let again = if agent.model.is_empty() { MODEL_UNKNOWN_AGAIN } else { MODEL_AGAIN };
        let ended = matches!(event, "Stop" | "SubagentStop");
        let ask_model =
            agent.transcript.clone().filter(|_| ended || agent.model_asked.is_none_or(|at| at.elapsed() >= again));
        if ask_model.is_some() {
            agent.model_asked = Some(Instant::now());
        }
        if let Some(task) = text(hook, &["prompt"]).filter(|_| inner.is_none() && event == "UserPromptSubmit") {
            agent.task = brief(&spoken(task), 48);
        }
        match event {
            "SubagentStop" => (agent.doing, agent.note) = (Doing::Leaving, String::new()),
            "Stop" => (agent.doing, agent.note) = (Doing::Waiting, String::new()),
            "UserPromptSubmit" | "SessionStart" | "SubagentStart" => {
                (agent.doing, agent.note) = (Doing::Thinking, String::new())
            }
            "PreToolUse" | "PostToolUse" => {
                let empty = json!({});
                let input = hook.get("tool_input").unwrap_or(&empty);
                let tool = text(hook, &["tool_name"]).unwrap_or_default();
                let whole = |path: &str| {
                    let path = PathBuf::from(path);
                    match (path.is_absolute(), &cwd) {
                        (false, Some(cwd)) => cwd.join(path),
                        _ => path,
                    }
                };
                let file = text(input, &["file_path", "notebook_path"]).map(whole);
                let number = |key: &str| input.get(key).and_then(Value::as_u64);
                match tool {
                    "Read" | "NotebookRead" => {
                        agent.doing = Doing::Reading;
                        agent.note = String::new();
                        agent.find = None;
                        agent.lines = Some((
                            number("offset").unwrap_or(1).saturating_sub(1).min(u32::MAX as u64 / 2) as u32,
                            number("limit").unwrap_or(60).clamp(1, 400) as u32,
                        ));
                        (agent.path, agent.moved) = (file, moves);
                    }
                    "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
                        agent.doing = Doing::Editing;
                        agent.note = String::new();
                        let written = text(input, &["new_string", "content", "new_source"]);
                        let before = text(input, &["old_string"]);
                        let look_for = if event == "PostToolUse" { written.or(before) } else { before.or(written) };
                        agent.find = look_for.map(first_line).filter(|line| !line.is_empty());
                        agent.lines = None;
                        (agent.path, agent.moved) = (file, moves);
                    }
                    "Grep" | "Glob" | "LS" => {
                        agent.doing = Doing::Searching;
                        let what = text(input, &["pattern", "glob"]).unwrap_or_default();
                        agent.note = if what.is_empty() { String::new() } else { format!("for {}", brief(what, 60)) };
                        let place = text(input, &["path"]).map(whole).or_else(|| cwd.clone());
                        (agent.find, agent.lines) = (None, None);
                        (agent.path, agent.moved) = (place, moves);
                    }
                    "Bash" | "PowerShell" => {
                        agent.doing = Doing::Running;
                        agent.note = brief(text(input, &["description", "command"]).unwrap_or_default(), 80);
                    }
                    "Agent" | "Task" => {
                        agent.doing = Doing::Thinking;
                        let about = text(input, &["description"]).unwrap_or("a task");
                        agent.note = format!("hands over: {}", brief(about, 70));
                    }
                    other => {
                        agent.doing = Doing::Thinking;
                        agent.note = brief(other, 40);
                    }
                }
            }
            _ => {}
        }
        if agent.moved == moves {
            agent.stepped(event == "PostToolUse");
        }
        if let Some(transcript) = ask_model {
            self.ask_model(&key, transcript);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_assistant_is_named_by_words_not_by_markup() {
        let wrapped =
            "<agent-message from=\"a098ff741e8394478\">\n[Subagent hand-back] Write the tests\n</agent-message>";
        assert_eq!(spoken(wrapped), "[Subagent hand-back] Write the tests");
        assert_eq!(spoken("Make the parser faster\nand more"), "Make the parser faster");
        assert_eq!(spoken("<only-a-tag>"), "");
    }

    fn heard(events: &[Value]) -> Agents {
        let mut agents = Agents {
            heard: None,
            tell: None,
            codex: None,
            port: None,
            list: Vec::new(),
            following: None,
            moves: 0,
            faces: Vec::new(),
            faces_changed: false,
            handed: Default::default(),
            models: None,
        };
        for event in events {
            agents.hear(event);
        }
        agents
    }

    #[test]
    fn what_an_assistant_reads_edits_and_searches_is_where_it_is() {
        let root = if cfg!(windows) { "C:/work/app" } else { "/work/app" };
        let under = |rel: &str| format!("{root}/{rel}");
        let hook = |event: &str, tool: &str, input: Value| {
            json!({ "source": "claude", "hook": { "session_id": "s1", "hook_event_name": event, "cwd": root,
                    "tool_name": tool, "tool_input": input } })
        };
        let agents =
            heard(&[hook("PreToolUse", "Read", json!({ "file_path": under("src/a.rs"), "offset": 40, "limit": 20 }))]);
        let agent = &agents.list[0];
        assert_eq!(
            (agent.name().as_str(), agent.role.as_str(), agent.doing),
            ("Claude Code", "Claude Code", Doing::Reading)
        );
        assert_eq!(agent.path.as_deref(), Some(std::path::Path::new(&under("src/a.rs"))));
        assert_eq!(agent.lines, Some((39, 20)));

        let agents = heard(&[
            hook("PreToolUse", "Read", json!({ "file_path": "src/b.rs" })),
            hook(
                "PostToolUse",
                "Edit",
                json!({ "file_path": "src/b.rs", "old_string": "let a = 1;", "new_string": "\n\n    let a = 2;\nmore" }),
            ),
        ]);
        let agent = &agents.list[0];
        assert_eq!(agents.list.len(), 1, "one session is one assistant");
        assert_eq!(
            agent.path.as_deref(),
            Some(std::path::Path::new(&under("src/b.rs"))),
            "a relative path is the session's"
        );
        assert_eq!((agent.doing, agent.find.as_deref(), agent.lines), (Doing::Editing, Some("let a = 2;"), None));

        let agents = heard(&[hook("PreToolUse", "Grep", json!({ "pattern": "fn main", "path": under("crates") }))]);
        assert_eq!((agents.list[0].doing, agents.list[0].note.as_str()), (Doing::Searching, "for fn main"));
        let agents = heard(&[hook("PreToolUse", "Bash", json!({ "command": "cargo   build\n --release" }))]);
        assert_eq!((agents.list[0].doing, agents.list[0].note.as_str()), (Doing::Running, "cargo build --release"));
    }

    #[test]
    fn each_helper_of_a_session_is_someone_else_and_leaves_when_done() {
        let hook = |event: &str, inner: Option<&str>| {
            let mut hook = json!({ "session_id": "s1", "hook_event_name": event, "tool_name": "Read",
                                   "tool_input": { "file_path": "C:/a.rs" } });
            if let Some(inner) = inner {
                hook["agent_id"] = json!(inner);
                hook["agent_type"] = json!("Explore");
            }
            json!({ "source": "claude", "hook": hook })
        };
        let mut asks = hook("UserPromptSubmit", None);
        asks["hook"]["prompt"] = json!("Make the parser faster\nand keep the tests green");
        let mut hands = hook("PreToolUse", None);
        hands["hook"]["tool_name"] = json!("Agent");
        hands["hook"]["tool_input"] = json!({ "description": "Find where tokens are read" });
        let mut agents = heard(&[asks, hands, hook("PreToolUse", Some("a1")), hook("PreToolUse", Some("a2"))]);
        let seen: Vec<(String, &str, usize)> =
            agents.list.iter().map(|agent| (agent.name(), agent.role.as_str(), agent.avatar)).collect();
        let named = |name: &str, role: &'static str, face| (name.to_string(), role, face);
        assert_eq!(
            seen,
            [
                named("Make the parser faster", "Claude Code", 0),
                named("Find where tokens are read", "Explore", 1),
                named("Explore", "Explore", 2)
            ]
        );
        let again = agents.face_of("s1/a1");
        assert_eq!((again, agents.face_of("s9")), (1, 3), "a face is kept, and a new one is one never given");
        agents.hear(&hook("SubagentStop", Some("a1")));
        assert_eq!(agents.list[1].doing, Doing::Leaving);
        agents.hear(&hook("Stop", None));
        assert!(agents.list[0].quiet() && agents.list[0].doing == Doing::Waiting);
        agents.hear(&json!({ "source": "claude", "hook": { "session_id": "s1", "hook_event_name": "SessionEnd" } }));
        assert!(agents.list.iter().all(|agent| agent.doing == Doing::Leaving));
    }

    #[test]
    fn any_program_can_say_where_it_is_in_plain_words() {
        let agents = heard(&[
            json!({ "agent": "codex-1", "name": "Codex", "action": "edit", "path": "C:/a.rs", "line": 12, "lines": 3, "note": "fixing   the parser" }),
        ]);
        let agent = &agents.list[0];
        assert_eq!((agent.role.as_str(), agent.doing, agent.lines), ("Codex", Doing::Editing, Some((11, 3))));
        assert_eq!(agent.note, "fixing the parser");
        assert!(heard(&[json!({ "note": "nobody" })]).list.is_empty());
        let agents = heard(&[
            json!({ "agent": "codex:p/h", "session": "codex:p", "name": "explorer", "action": "edit", "path": "C:/a.rs", "find": "\n  let b = 2;\nmore" }),
        ]);
        let agent = &agents.list[0];
        assert_eq!((agent.session.as_str(), agent.find.as_deref(), agent.lines), ("codex:p", Some("let b = 2;"), None));
    }

    #[test]
    fn an_assistant_leaves_a_trail_of_the_places_it_read_edited_and_searched() {
        let plain = |action: &str, path: &str, line: Option<u64>| {
            let mut event = json!({ "agent": "c1", "name": "Codex", "action": action, "path": path, "lines": 4 });
            if let Some(line) = line {
                event["line"] = json!(line);
            }
            event
        };
        let mut agents = heard(&[
            plain("read", "C:/a.rs", Some(10)),
            plain("read", "C:/a.rs", Some(10)),
            plain("read", "C:/b.rs", Some(1)),
            plain("edit", "C:/b.rs", Some(3)),
            plain("search", "C:/src", None),
            plain("run", "C:/src", None),
            plain("think", "C:/c.rs", None),
        ]);
        type Step<'a> = (Doing, &'a str, Option<(u32, u32)>);
        let trail: Vec<Step> =
            agents.list[0].trail.iter().map(|step| (step.doing, step.path.to_str().unwrap(), step.lines)).collect();
        assert_eq!(
            trail,
            [
                (Doing::Reading, "C:/a.rs", Some((9, 4))),
                (Doing::Reading, "C:/b.rs", Some((0, 4))),
                (Doing::Editing, "C:/b.rs", Some((2, 4))),
                (Doing::Searching, "C:/src", None),
            ],
            "the same place twice is one step; running and thinking go nowhere"
        );
        assert!(agents.list[0].trail.iter().all(|step| step.left() > 0.99));

        for line in 0..20 {
            agents.hear(&plain("read", "C:/long.rs", Some(line * 10 + 1)));
        }
        let trail = &agents.list[0].trail;
        assert_eq!(trail.len(), TRAIL, "only the last places are kept");
        assert_eq!((trail[0].lines, trail[TRAIL - 1].lines), (Some((80, 4)), Some((190, 4))));

        if let Some(long_ago) = Instant::now().checked_sub(TRAIL_LASTS + Duration::from_secs(1)) {
            agents.list[0].trail[0].at = long_ago;
            agents.faces_changed = false;
            assert!(agents.poll(), "a place left long ago fades out");
            assert_eq!(agents.list[0].trail.len(), TRAIL - 1);
        }
    }

    #[test]
    fn an_edit_heard_before_and_after_is_one_step_that_takes_the_line_it_is_found_on() {
        let edit = |event: &str, old: &str, new: &str| {
            json!({ "source": "claude", "hook": { "session_id": "s1", "hook_event_name": event, "tool_name": "Edit",
                    "tool_input": { "file_path": "C:/a.rs", "old_string": old, "new_string": new } } })
        };
        let mut agents = heard(&[edit("PreToolUse", "one", "two"), edit("PostToolUse", "one", "two")]);
        assert_eq!(agents.list[0].trail.len(), 1, "before and after the same edit");
        agents.list[0].found(41);
        assert_eq!(agents.list[0].trail[0].lines, Some((41, 1)));
        agents.hear(&edit("PreToolUse", "three", "four"));
        agents.hear(&edit("PostToolUse", "three", "four"));
        agents.list[0].found(7);
        let steps: Vec<_> = agents.list[0].trail.iter().map(|step| (step.doing, step.lines)).collect();
        let two = [(Doing::Editing, Some((41, 1))), (Doing::Editing, Some((7, 1)))];
        assert_eq!(steps, two, "another edit is another step");
    }

    #[test]
    fn the_hooks_of_claude_code_are_added_and_taken_out_without_touching_the_rest() {
        let before: Value = serde_json::from_str(
            r#"{ "model": "opus", "zeta": 1, "alpha": 2,
                 "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "mine.sh" }] }] } }"#,
        )
        .unwrap();
        let program = "C:\\Program Files\\Code Canvas\\code-canvas.exe";
        let on = with_hooks(&before, Some(program)).unwrap();
        let keys: Vec<&String> = on.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "zeta", "alpha", "hooks"], "the keys keep the order the user gave them");
        let tools = on["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(tools.len(), 2, "theirs stays, ours is added");
        assert_eq!(tools[1]["matcher"], "*");
        assert_eq!(tools[1]["hooks"][0]["command"], program);
        assert_eq!(tools[1]["hooks"][0]["args"], json!(["--agent-hook", "claude"]));
        assert_eq!(tools[1]["hooks"][0]["async"], true);
        assert!(on["hooks"]["SessionEnd"][0].get("matcher").is_none());
        assert_eq!(with_hooks(&on, Some(program)).unwrap(), on, "adding twice adds once");
        let off = with_hooks(&on, None).unwrap();
        assert_eq!(off, before, "and taking out leaves what was there");
        assert_eq!(serde_json::to_string(&off).unwrap(), serde_json::to_string(&before).unwrap());
        assert_eq!(with_hooks(&json!({}), None).unwrap(), json!({}));
        assert_eq!(with_hooks(&json!({ "hooks": {} }), None).unwrap(), json!({ "hooks": {} }));
    }

    #[test]
    fn a_hook_of_ours_among_theirs_is_the_only_one_taken() {
        let mixed = json!({ "hooks": { "Stop": [{ "hooks": [
            { "type": "command", "command": "theirs" },
            { "type": "command", "command": "old path", "args": ["--agent-hook", "claude"] } ] }] } });
        let off = with_hooks(&mixed, None).unwrap();
        assert_eq!(off, json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "theirs" }] }] } }));
        let again = with_hooks(&mixed, Some("new path")).unwrap();
        let stop = again["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(
            (stop.len(), stop[0]["hooks"].as_array().unwrap().len()),
            (2, 1),
            "the old one is replaced, not doubled"
        );
        assert_eq!(stop[1]["hooks"][0]["command"], "new path");
    }

    #[test]
    fn settings_of_a_shape_not_understood_are_refused_not_rewritten() {
        assert!(with_hooks(&json!([1, 2]), Some("p")).is_err());
        assert!(with_hooks(&json!({ "hooks": "none" }), Some("p")).is_err());
        assert!(with_hooks(&json!({ "hooks": { "Stop": { "odd": true } } }), Some("p")).is_err());
        assert!(with_hooks(&json!({ "hooks": { "Stop": { "odd": true } } }), None).is_err());
    }

    #[test]
    fn a_settings_file_that_cannot_be_understood_is_never_taken_for_an_empty_one() {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        assert_eq!(read_settings(&file).unwrap(), json!({}), "no file yet is an empty one");
        std::fs::write(&file, b"{ \"name\": \"caf\xe9\" }").unwrap();
        assert!(read_settings(&file).unwrap_err().contains("not UTF-8"));
        std::fs::write(&file, "{ // a comment\n \"a\": 1 }").unwrap();
        assert!(read_settings(&file).is_err());
        std::fs::write(&file, "\u{feff}{ \"a\": 1 }").unwrap();
        assert_eq!(read_settings(&file).unwrap(), json!({ "a": 1 }), "a mark at the start is not a mistake");
        std::fs::write(&file, "  \n").unwrap();
        assert_eq!(read_settings(&file).unwrap(), json!({}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_arrives_late_does_not_bring_back_who_left() {
        let hook = |event: &str, inner: Option<&str>| {
            let mut hook = json!({ "session_id": "s1", "hook_event_name": event, "tool_name": "Read",
                                   "tool_input": { "file_path": "C:/a.rs", "offset": 9_000_000_000u64, "limit": 5 } });
            if let Some(inner) = inner {
                hook["agent_id"] = json!(inner);
            }
            json!({ "source": "claude", "hook": hook })
        };
        let mut agents =
            heard(&[hook("PreToolUse", Some("a1")), hook("SubagentStop", Some("a1")), hook("PostToolUse", Some("a1"))]);
        assert_eq!(agents.list[0].doing, Doing::Leaving);
        assert!(agents.list[0].lines.is_some_and(|(first, count)| first.checked_add(count).is_some()));
        agents.hear(&hook("PreToolUse", None));
        agents.hear(&hook("SubagentStop", None));
        assert_eq!(agents.list[1].doing, Doing::Reading, "a stop that names no agent is nobody's");
        agents.hear(&json!({ "agent": "x", "path": "C:/a.rs", "line": 10, "lines": 4_294_967_295u64 }));
        assert_eq!(agents.list[2].lines, Some((9, 400)));
    }

    #[test]
    fn the_model_of_each_assistant_comes_from_its_own_transcript_or_from_what_it_says() {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-agent-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let main = dir.join("s1.jsonl");
        let helper = dir.join("s1").join("subagents").join("agent-a1.jsonl");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        let answer = |model: &str| format!("{}\n", json!({ "type": "assistant", "message": { "model": model } }));
        std::fs::write(&main, answer("claude-opus-5-5")).unwrap();
        std::fs::write(&helper, answer("claude-haiku-5-5")).unwrap();
        let hook = |event: &str, inner: Option<&str>| {
            let mut hook = json!({ "session_id": "s1", "hook_event_name": event, "transcript_path": main,
                                   "tool_name": "Read", "tool_input": { "file_path": "C:/a.rs" } });
            if let Some(inner) = inner {
                hook["agent_id"] = json!(inner);
            }
            json!({ "source": "claude", "hook": hook })
        };
        let mut agents = heard(&[]);
        let (tell, listened) = mpsc::channel();
        agents.tell = Some(tell);
        agents.heard = Some(listened);
        agents.hear(&hook("PreToolUse", None));
        agents.hear(&hook("PreToolUse", Some("a1")));
        assert_eq!(
            (agents.list[0].transcript.as_ref(), agents.list[1].transcript.as_ref()),
            (Some(&main), Some(&helper))
        );
        assert!(agents.list.iter().all(|agent| agent.model.is_empty()), "nothing is said before it is read");
        let until = Instant::now() + Duration::from_secs(10);
        while agents.list.iter().any(|agent| agent.model.is_empty()) && Instant::now() < until {
            agents.faces_changed = false;
            agents.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        let models: Vec<&str> = agents.list.iter().map(|agent| agent.model.as_str()).collect();
        assert_eq!(models, ["claude-opus-5-5", "claude-haiku-5-5"]);
        let asked = agents.list[0].model_asked;
        agents.hear(&hook("PostToolUse", None));
        assert_eq!(agents.list[0].model_asked, asked, "a model known is not read again at each step");
        agents.hear(&hook("Stop", None));
        assert_ne!(agents.list[0].model_asked, asked, "but again when a turn ends");
        agents.hear(&json!({ "model_of": "nobody", "model": "x" }));
        assert_eq!(agents.list.len(), 2, "a model read late does not bring back who left");

        let agents = heard(&[json!({ "agent": "c1", "name": "Codex", "action": "think", "model": "gpt-5-codex" })]);
        assert_eq!(agents.list[0].model, "gpt-5-codex");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_event_is_cut_to_what_matters_before_it_is_sent() {
        let event = json!({ "session_id": "s", "tool_input": { "content": "x".repeat(50_000), "file_path": "a.rs" },
                            "tool_response": { "stdout": "y".repeat(90_000) } });
        let cut = shortened(&event, 0);
        assert_eq!(cut["tool_input"]["content"].as_str().unwrap().len(), KEPT_OF_A_TEXT);
        assert_eq!(cut["tool_input"]["file_path"], "a.rs");
        assert!(cut.get("tool_response").is_none());
        assert!(cut.to_string().len() < 2000);
    }
}
