use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use serde_json::{json, Map, Value};

pub const ROLE: &str = "Codex";
pub const KEY: &str = "codex:";
const TICK: Duration = Duration::from_millis(1500);
const RESTS_AFTER: Duration = Duration::from_secs(5 * 60);
const RESTING_LOOK: Duration = Duration::from_secs(15);
const FRESH: Duration = Duration::from_secs(60);
const RECENT: Duration = Duration::from_secs(15 * 60);
const LAST_LOOK: u64 = 256 * 1024;
const LONGEST_LINE: u64 = 32 << 20;
const DAYS: usize = 2;
const KEPT_OF_A_PROMPT: usize = 400;

pub fn home() -> Option<PathBuf> {
    let named = std::env::var_os("CODEX_HOME").filter(|dir| !dir.is_empty()).map(PathBuf::from);
    named.or_else(|| Some(dirs::home_dir()?.join(".codex")))
}

pub fn there() -> bool {
    home().is_some_and(|home| home.join("sessions").is_dir() || home.join("config.toml").is_file())
}

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty())
}

fn find_str<'a>(value: &'a Value, key: &str, depth: u8) -> Option<&'a str> {
    if let Some(found) = str_of(value, key) {
        return Some(found);
    }
    match value {
        Value::Object(fields) if depth < 4 => fields.values().find_map(|inner| find_str(inner, key, depth + 1)),
        _ => None,
    }
}

fn from_url(text: &str) -> String {
    let Some(rest) = text.strip_prefix("file://") else { return text.to_string() };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let rest = match bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' && cfg!(windows) {
        true => &rest[1..],
        false => rest,
    };
    let mut plain = Vec::with_capacity(rest.len());
    let bytes = rest.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let hex =
            bytes.get(at + 1..at + 3).and_then(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                plain.push(byte);
                at += 3;
            }
            (byte, _) => {
                plain.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&plain).into_owned()
}

fn seconds_of(stamp: &str) -> Option<i64> {
    let number = |from: usize, to: usize| stamp.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn age_of_stamp(stamp: &str) -> Option<Duration> {
    let then = seconds_of(stamp)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(Duration::from_secs(now.saturating_sub(then).max(0) as u64))
}

fn age_of(line: &Value) -> Option<Duration> {
    age_of_stamp(str_of(line, "timestamp")?)
}

#[derive(Clone, Debug, PartialEq)]
struct Place {
    action: &'static str,
    path: PathBuf,
    lines: Option<(u64, u64)>,
    find: Option<String>,
    note: String,
}

impl Place {
    fn new(action: &'static str, path: PathBuf) -> Place {
        Place { action, path, lines: None, find: None, note: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Did {
    At(Place),
    Run(String),
    Think(String),
    Wait,
    Done,
}

/// What one session of Codex has said so far, and how each new line of its file
/// is told to the canvas: the plain events of `agents.rs`.
#[derive(Clone, Debug, Default)]
pub struct Session {
    id: String,
    parent: Option<String>,
    role: String,
    cwd: Option<PathBuf>,
    task: String,
    task_sure: bool,
    once: bool,
    hidden: bool,
}

impl Session {
    pub fn of_file(file: &Path) -> Session {
        let stem = file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let id = match stem.len().checked_sub(36).filter(|at| *at > 0 && stem.is_char_boundary(*at)) {
            Some(at) => stem[at..].to_string(),
            None => stem,
        };
        Session { id, role: ROLE.to_string(), ..Default::default() }
    }

    fn key(&self) -> String {
        match &self.parent {
            Some(parent) => format!("{KEY}{parent}/{}", self.id),
            None => format!("{KEY}{}", self.id),
        }
    }

    pub fn hear(&mut self, line: &Value) -> Vec<Value> {
        let did = self.understand(line);
        if self.hidden {
            return Vec::new();
        }
        did.into_iter().map(|did| self.told(did)).collect()
    }

    fn told(&self, did: Did) -> Value {
        let (action, note, place) = match did {
            Did::At(place) => (place.action, place.note.clone(), Some(place)),
            Did::Run(note) => ("run", note, None),
            Did::Think(note) => ("think", note, None),
            Did::Wait => ("wait", String::new(), None),
            Did::Done => ("done", String::new(), None),
        };
        let mut event = Map::new();
        event.insert("agent".into(), json!(self.key()));
        event.insert("name".into(), json!(self.role));
        event.insert("action".into(), json!(action));
        if let Some(parent) = &self.parent {
            event.insert("session".into(), json!(format!("{KEY}{parent}")));
        }
        if !self.task.is_empty() {
            event.insert("task".into(), json!(self.task));
        }
        if !note.is_empty() {
            event.insert("note".into(), json!(note));
        }
        if let Some(place) = place {
            event.insert("path".into(), json!(place.path.to_string_lossy()));
            if let Some((line, lines)) = place.lines {
                event.insert("line".into(), json!(line));
                event.insert("lines".into(), json!(lines));
            }
            if let Some(find) = place.find {
                event.insert("find".into(), json!(find));
            }
        }
        Value::Object(event)
    }

    fn whole(&self, path: &str, cwd: Option<&Path>) -> PathBuf {
        let path = from_url(path.trim().trim_matches(['"', '\'']));
        let mut path = path.as_str();
        while let Some(rest) = path.strip_prefix("./").or_else(|| path.strip_prefix(".\\")) {
            path = rest;
        }
        let cwd = cwd.or(self.cwd.as_deref());
        match (path, cwd) {
            ("." | "", Some(cwd)) => cwd.to_path_buf(),
            (path, Some(cwd)) if !Path::new(path).is_absolute() => cwd.join(path),
            (path, _) => PathBuf::from(path),
        }
    }

    fn understand(&mut self, line: &Value) -> Vec<Did> {
        let Some(payload) = line.get("payload") else { return Vec::new() };
        let kind = str_of(payload, "type").unwrap_or_default();
        match str_of(line, "type").unwrap_or_default() {
            "session_meta" => {
                self.meta(payload);
                vec![Did::Think(String::new())]
            }
            "turn_context" => {
                if let Some(cwd) = str_of(payload, "cwd") {
                    self.cwd = Some(PathBuf::from(from_url(cwd)));
                }
                Vec::new()
            }
            "event_msg" => match kind {
                "task_started" | "turn_started" => vec![Did::Think(String::new())],
                "task_complete" | "turn_complete" | "turn_aborted" => {
                    vec![if self.once { Did::Done } else { Did::Wait }]
                }
                "user_message" => self.asked(str_of(payload, "message"), true),
                "item_completed" => payload.get("item").map(|item| self.item(item)).unwrap_or_default(),
                "exec_command_begin" => self.ran(payload),
                "patch_apply_begin" => self.changed(payload.get("changes")),
                _ => Vec::new(),
            },
            "response_item" => match kind {
                "message" if str_of(payload, "role") == Some("user") => {
                    let said = first_worth(payload.get("content"));
                    self.asked(said, false)
                }
                "function_call" => {
                    let args = match payload.get("arguments") {
                        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or(Value::Null),
                        Some(other) => other.clone(),
                        None => Value::Null,
                    };
                    self.tool(str_of(payload, "name").unwrap_or_default(), &args)
                }
                "custom_tool_call" => {
                    let input = str_of(payload, "input").unwrap_or_default();
                    match str_of(payload, "name").unwrap_or_default() {
                        "apply_patch" => self.patched(input),
                        "exec" => self.script(input),
                        other => vec![Did::Think(other.to_string())],
                    }
                }
                "local_shell_call" => {
                    let action = payload.get("action").cloned().unwrap_or(Value::Null);
                    let workdir = str_of(&action, "working_directory").map(|dir| self.whole(dir, None));
                    self.shell(&command_text(action.get("command")), workdir.as_deref())
                }
                "web_search_call" => vec![Did::Think("searching the web".to_string())],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn meta(&mut self, payload: &Value) {
        if let Some(id) = str_of(payload, "id").or_else(|| str_of(payload, "session_id")) {
            self.id = id.to_string();
        }
        if let Some(cwd) = str_of(payload, "cwd") {
            self.cwd = Some(PathBuf::from(from_url(cwd)));
        }
        let source = payload.get("source").cloned().unwrap_or(Value::Null);
        self.once = source.as_str() == Some("exec");
        let Some(helper) = source.get("subagent") else { return };
        let kind = match helper {
            Value::String(kind) => kind.clone(),
            Value::Object(fields) => fields.keys().next().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        self.hidden = matches!(kind.as_str(), "compact" | "memory_consolidation") || kind.contains("guardian");
        self.parent = find_str(helper, "parent_thread_id", 0).map(str::to_string);
        let named = ["agent_role", "agent_nickname"]
            .iter()
            .find_map(|key| find_str(payload, key, 0).or_else(|| find_str(helper, key, 0)).map(str::to_string));
        self.role = named.unwrap_or_else(|| match kind.as_str() {
            "review" => "Codex review".to_string(),
            _ => "Codex agent".to_string(),
        });
    }

    fn asked(&mut self, said: Option<&str>, sure: bool) -> Vec<Did> {
        let Some(said) = said.map(request_of).filter(|said| sure || worth(said)) else { return Vec::new() };
        if said.trim().is_empty() {
            return Vec::new();
        }
        if !self.task_sure && (sure || self.task.is_empty()) {
            self.task = said.chars().take(KEPT_OF_A_PROMPT).collect();
            self.task_sure = sure;
        }
        vec![Did::Think(String::new())]
    }

    fn item(&mut self, item: &Value) -> Vec<Did> {
        match str_of(item, "type").unwrap_or_default() {
            "UserMessage" | "userMessage" => {
                let said = first_worth(item.get("content"));
                self.asked(said, true)
            }
            "CommandExecution" | "commandExecution" => self.ran(item),
            "FileChange" | "fileChange" => self.changed(item.get("changes")),
            "ImageView" | "imageView" => match str_of(item, "path") {
                Some(path) => vec![Did::At(Place::new("read", self.whole(path, None)))],
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn ran(&mut self, item: &Value) -> Vec<Did> {
        let cwd = str_of(item, "cwd").map(|cwd| PathBuf::from(from_url(cwd))).or_else(|| self.cwd.clone());
        let command = command_text(item.get("command"));
        let mut did = Vec::new();
        for one in item.get("parsed_cmd").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
            let cmd = str_of(one, "cmd").unwrap_or_default();
            let named = str_of(one, "path").or_else(|| str_of(one, "name"));
            match str_of(one, "type").unwrap_or_default() {
                "read" => {
                    let Some(path) = named else { continue };
                    let mut place = Place::new("read", self.whole(path, cwd.as_deref()));
                    place.lines = shell::read_range(cmd);
                    did.push(Did::At(place));
                }
                "search" | "list_files" => {
                    let at = named.map(|path| shell::place_of(&self.whole(path, cwd.as_deref())));
                    let Some(at) = at.or_else(|| cwd.clone()) else { continue };
                    let mut place = Place::new("search", at);
                    if let Some(query) = str_of(one, "query") {
                        place.note = format!("for {query}");
                    }
                    did.push(Did::At(place));
                }
                _ => {}
            }
        }
        if did.is_empty() {
            did = self.shell(&command, cwd.as_deref());
        }
        did
    }

    fn shell(&self, command: &str, workdir: Option<&Path>) -> Vec<Did> {
        if command.trim().is_empty() {
            return vec![Did::Run(String::new())];
        }
        let workdir = workdir.or(self.cwd.as_deref());
        let found: Vec<Did> = shell::understood(command)
            .into_iter()
            .map(|seen| {
                let mut place = match seen.path.as_deref().filter(|path| !path.starts_with(['(', '$', '@', '{'])) {
                    Some(path) => Place::new(seen.action, shell::place_of(&self.whole(path, workdir))),
                    None => Place::new(seen.action, workdir.map(Path::to_path_buf).unwrap_or_default()),
                };
                (place.lines, place.note) = (seen.lines, seen.note);
                place
            })
            .filter(|place| !place.path.as_os_str().is_empty())
            .map(Did::At)
            .collect();
        match found.is_empty() {
            true => vec![Did::Run(command.trim().to_string())],
            false => found,
        }
    }

    fn changed(&self, changes: Option<&Value>) -> Vec<Did> {
        let Some(changes) = changes.and_then(Value::as_object) else { return Vec::new() };
        let mut did = Vec::new();
        for (path, change) in changes {
            let (kind, body) = match str_of(change, "type") {
                Some(kind) => (kind, change),
                None => match change.as_object().and_then(|fields| fields.iter().next()) {
                    Some((kind, body)) => (kind.as_str(), body),
                    None => continue,
                },
            };
            let path = str_of(body, "move_path").unwrap_or(path);
            let mut place = Place::new("edit", self.whole(path, None));
            match kind {
                "add" => {
                    let lines = str_of(body, "content").map_or(1, |content| content.lines().count() as u64);
                    place.lines = Some((1, lines.clamp(1, 40)));
                }
                "update" => {
                    let diff = str_of(body, "unified_diff").unwrap_or_default();
                    place.lines = first_change(diff);
                    if place.lines.is_none() {
                        place.find = written(diff);
                    }
                }
                _ => {}
            }
            did.push(Did::At(place));
        }
        did
    }

    fn patched(&self, patch: &str) -> Vec<Did> {
        let mut did = Vec::new();
        let mut now: Option<(Place, &str, Vec<&str>)> = None;
        let finish = |now: Option<(Place, &str, Vec<&str>)>, did: &mut Vec<Did>| {
            let Some((mut place, kind, body)) = now else { return };
            match kind {
                "add" => place.lines = Some((1, body.len().clamp(1, 40) as u64)),
                "update" => place.find = written(&body.join("\n")),
                _ => {}
            }
            did.push(Did::At(place));
        };
        for line in patch.lines().map(|line| line.trim_end_matches('\r')) {
            let header = [("*** Add File: ", "add"), ("*** Update File: ", "update"), ("*** Delete File: ", "delete")]
                .iter()
                .find_map(|(start, kind)| Some((line.strip_prefix(start)?, *kind)));
            if let Some((path, kind)) = header {
                finish(now.take(), &mut did);
                now = Some((Place::new("edit", self.whole(path, None)), kind, Vec::new()));
            } else if let Some(path) = line.strip_prefix("*** Move to: ") {
                if let Some((place, _, _)) = now.as_mut() {
                    place.path = self.whole(path, None);
                }
            } else if line.starts_with("*** End Patch") {
                finish(now.take(), &mut did);
            } else if let Some((_, kind, body)) = now.as_mut() {
                if *kind != "add" || line.starts_with('+') {
                    body.push(line);
                }
            }
        }
        finish(now, &mut did);
        did
    }

    fn tool(&mut self, name: &str, args: &Value) -> Vec<Did> {
        let workdir = str_of(args, "workdir").or_else(|| str_of(args, "cwd")).map(|dir| self.whole(dir, None));
        let number = |key: &str| args.get(key).and_then(Value::as_u64);
        match name {
            "shell" | "container.exec" | "local_shell" | "shell_command" | "exec_command" => {
                let command = command_text(args.get("command").or_else(|| args.get("cmd")));
                self.shell(&command, workdir.as_deref())
            }
            "apply_patch" => self.patched(str_of(args, "input").or_else(|| str_of(args, "patch")).unwrap_or_default()),
            "read_file" | "view_image" => {
                let Some(path) = str_of(args, "file_path").or_else(|| str_of(args, "path")) else { return Vec::new() };
                let mut place = Place::new("read", self.whole(path, None));
                if name == "read_file" {
                    let first = number("offset").unwrap_or(1).max(1);
                    place.lines = Some((first, number("limit").unwrap_or(60).clamp(1, 400)));
                }
                vec![Did::At(place)]
            }
            "list_dir" | "grep_files" => {
                let at = str_of(args, "dir_path").or_else(|| str_of(args, "path")).map(|path| self.whole(path, None));
                let Some(at) = at.or_else(|| self.cwd.clone()) else { return Vec::new() };
                let mut place = Place::new("search", at);
                if let Some(pattern) = str_of(args, "pattern") {
                    place.note = format!("for {pattern}");
                }
                vec![Did::At(place)]
            }
            "" => Vec::new(),
            other => vec![Did::Think(other.to_string())],
        }
    }

    fn script(&mut self, code: &str) -> Vec<Did> {
        let mut did = Vec::new();
        let reads = code.contains("view_image") || code.contains("read_file");
        for (key, value) in literals(code) {
            if value.contains("*** Begin Patch") {
                did.extend(self.patched(&value));
                continue;
            }
            match key.as_deref() {
                Some("cmd" | "command") => did.extend(self.shell(&value, None)),
                Some("path" | "file_path") if reads => {
                    did.push(Did::At(Place::new("read", self.whole(&value, None))));
                }
                _ => {}
            }
        }
        if did.is_empty() {
            let named = code
                .split("tools.")
                .nth(1)
                .map(|rest| rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>());
            did.push(Did::Think(named.unwrap_or_default()));
        }
        did
    }
}

fn request_of(said: &str) -> &str {
    match said.split_once("## My request for Codex:") {
        Some((_, asked)) => asked.trim(),
        None => said.trim(),
    }
}

fn worth(said: &str) -> bool {
    let said = said.trim_start();
    let made = ["<", "# AGENTS.md", "Codex could not read", "# Context from my IDE"];
    !made.iter().any(|start| said.starts_with(start)) && said.chars().filter(|c| c.is_alphanumeric()).count() > 3
}

fn first_worth(content: Option<&Value>) -> Option<&str> {
    let texts = content.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let all: Vec<&str> = texts.iter().filter_map(|item| str_of(item, "text")).collect();
    all.iter().copied().find(|text| worth(request_of(text))).or(all.first().copied())
}

fn command_text(command: Option<&Value>) -> String {
    match command {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(words)) => {
            let words: Vec<&str> = words.iter().filter_map(Value::as_str).collect();
            let program = words.first().map(|first| shell::program(first)).unwrap_or_default();
            let a_shell = matches!(program.as_str(), "bash" | "sh" | "zsh" | "powershell" | "pwsh" | "cmd");
            match (a_shell && words.len() > 1, words.last()) {
                (true, Some(last)) => last.to_string(),
                _ => words.join(" "),
            }
        }
        _ => String::new(),
    }
}

fn first_change(diff: &str) -> Option<(u64, u64)> {
    let lines: Vec<&str> = diff.lines().collect();
    let (mut at, mut removed) = (None::<u64>, None);
    for (index, line) in lines.iter().enumerate() {
        if let Some(rest) = line.strip_prefix("@@") {
            let new = rest.split_whitespace().find_map(|part| part.strip_prefix('+'));
            at = new.and_then(|new| new.split(',').next()?.parse().ok());
            continue;
        }
        let Some(line_now) = at.as_mut() else { continue };
        match line.chars().next() {
            Some('+') => {
                let added = lines[index..].iter().take_while(|line| line.starts_with('+')).count() as u64;
                return Some(((*line_now).max(1), added.clamp(1, 400)));
            }
            Some('-') => removed = removed.or(Some(((*line_now).max(1), 1))),
            Some('\\') => {}
            _ => *line_now += 1,
        }
    }
    removed
}

fn written(body: &str) -> Option<String> {
    let added = body.lines().filter_map(|line| line.strip_prefix('+')).filter(|line| !line.starts_with("++"));
    let anchor =
        || body.lines().find_map(|line| line.strip_prefix("@@").map(str::trim).filter(|context| !context.is_empty()));
    let worth = |line: &&str| line.trim().chars().filter(|c| c.is_alphanumeric()).count() > 2;
    let found = added.clone().find(worth).or_else(anchor).or_else(|| added.clone().next());
    found.map(|line| line.trim().to_string()).filter(|line| !line.is_empty())
}

fn literals(code: &str) -> Vec<(Option<String>, String)> {
    let chars: Vec<char> = code.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let next = chars.get(at + 1).copied();
        match chars[at] {
            '/' if next == Some('/') => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            '/' if next == Some('*') => {
                at += 2;
                while at + 1 < chars.len() && !(chars[at] == '*' && chars[at + 1] == '/') {
                    at += 1;
                }
                at += 2;
            }
            quote @ ('"' | '\'' | '`') => {
                let start = at;
                let mut value = String::new();
                at += 1;
                while at < chars.len() && chars[at] != quote {
                    if chars[at] == '\\' && at + 1 < chars.len() {
                        at += 1;
                        match chars[at] {
                            'n' => value.push('\n'),
                            't' => value.push('\t'),
                            'r' => value.push('\r'),
                            'u' => {
                                let hex: String = chars[at + 1..].iter().take(4).collect();
                                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                    Some(c) if hex.len() == 4 => {
                                        value.push(c);
                                        at += 4;
                                    }
                                    _ => value.push('u'),
                                }
                            }
                            other => value.push(other),
                        }
                    } else {
                        value.push(chars[at]);
                    }
                    at += 1;
                }
                found.push((key_before(&chars[..start]), value));
                at += 1;
                continue;
            }
            _ => {}
        }
        at += 1;
    }
    found
}

fn key_before(before: &[char]) -> Option<String> {
    let mut end = before.len();
    while end > 0 && before[end - 1].is_whitespace() {
        end -= 1;
    }
    if end == 0 || before[end - 1] != ':' {
        return None;
    }
    end -= 1;
    while end > 0 && before[end - 1].is_whitespace() {
        end -= 1;
    }
    let quoted = end > 0 && matches!(before[end - 1], '"' | '\'');
    let stop = if quoted { end - 1 } else { end };
    let mut start = stop;
    while start > 0 && (before[start - 1].is_alphanumeric() || matches!(before[start - 1], '_' | '$')) {
        start -= 1;
    }
    (start < stop).then(|| before[start..stop].iter().collect())
}

/// What a command line says it does, read the way a person would: the first
/// word of each command, and its arguments.
mod shell {
    use std::path::{Path, PathBuf};

    #[derive(Debug, PartialEq)]
    pub struct Seen {
        pub action: &'static str,
        pub path: Option<String>,
        pub lines: Option<(u64, u64)>,
        pub note: String,
    }

    enum Piece {
        Word(String),
        Pipe,
        End,
    }

    #[derive(Default)]
    struct Word {
        text: String,
        started: bool,
        dropped: bool,
    }

    impl Word {
        fn end(&mut self, pieces: &mut Vec<Piece>) {
            let word = std::mem::take(self);
            if word.started && !word.dropped {
                pieces.push(Piece::Word(word.text));
            }
        }
    }

    fn pieces(text: &str) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut word = Word::default();
        let chars: Vec<char> = text.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let c = chars[at];
            let next = chars.get(at + 1).copied();
            match c {
                '\'' | '"' => {
                    word.started = true;
                    at += 1;
                    while at < chars.len() && chars[at] != c {
                        if c == '"' && chars[at] == '\\' && matches!(chars.get(at + 1), Some('"' | '\\')) {
                            at += 1;
                        }
                        word.text.push(chars[at]);
                        at += 1;
                    }
                }
                ' ' | '\t' | '\r' => word.end(&mut pieces),
                '\n' | ';' => {
                    word.end(&mut pieces);
                    pieces.push(Piece::End);
                }
                '|' => {
                    word.end(&mut pieces);
                    if next == Some('|') {
                        at += 1;
                        pieces.push(Piece::End);
                    } else {
                        pieces.push(Piece::Pipe);
                    }
                }
                '&' => {
                    word.end(&mut pieces);
                    if next == Some('&') {
                        at += 1;
                        pieces.push(Piece::End);
                    }
                }
                '>' | '<' => {
                    if word.text.chars().all(|c| c.is_ascii_digit()) {
                        word = Word::default();
                    }
                    word.end(&mut pieces);
                    let mut joined = false;
                    while let Some(more) = chars.get(at + 1) {
                        match more {
                            '>' => at += 1,
                            '&' => {
                                joined = true;
                                at += 1;
                            }
                            c if joined && c.is_ascii_digit() => at += 1,
                            _ => break,
                        }
                    }
                    word.dropped = !joined;
                    while chars.get(at + 1) == Some(&' ') {
                        at += 1;
                    }
                }
                c => {
                    word.started = true;
                    word.text.push(c);
                }
            }
            at += 1;
        }
        word.end(&mut pieces);
        pieces
    }

    pub fn program(word: &str) -> String {
        let name = word.rsplit(['/', '\\']).next().unwrap_or(word).to_lowercase();
        [".exe", ".cmd", ".bat"].iter().find_map(|end| name.strip_suffix(end)).unwrap_or(&name).to_string()
    }

    struct Args {
        positional: Vec<String>,
        named: Vec<(String, String)>,
        flags: Vec<String>,
    }

    impl Args {
        fn get(&self, names: &[&str]) -> Option<&str> {
            self.named.iter().find(|(name, _)| names.contains(&name.as_str())).map(|(_, value)| value.as_str())
        }

        fn has(&self, names: &[&str]) -> bool {
            self.flags.iter().any(|flag| names.contains(&flag.as_str()))
        }
    }

    fn args(words: &[String], valued: &[&str], slash: bool) -> Args {
        let mut args = Args { positional: Vec::new(), named: Vec::new(), flags: Vec::new() };
        let mut at = 0;
        while at < words.len() {
            let word = &words[at];
            let dashed = word.starts_with('-') && word.len() > 1;
            let slashed = slash && word.starts_with('/') && word.len() > 1;
            let inline = match (dashed, slashed) {
                (true, _) if word.starts_with("--") => word.split_once('='),
                (_, true) => word.split_once(':'),
                _ => None,
            };
            let takes_value = valued.contains(&word.as_str()) || valued.contains(&word.to_lowercase().as_str());
            if !dashed && !slashed {
                args.positional.push(word.clone());
            } else if let Some((name, value)) = inline {
                args.named.push((name.to_lowercase(), value.to_string()));
            } else if takes_value && at + 1 < words.len() {
                args.named.push((word.to_lowercase(), words[at + 1].clone()));
                at += 1;
            } else {
                args.flags.push(word.to_lowercase());
            }
            at += 1;
        }
        args
    }

    fn number(text: Option<&str>) -> Option<u64> {
        text?.trim().parse().ok()
    }

    fn sed_range(script: &str) -> Option<(u64, u64)> {
        let script = script.trim().strip_suffix('p')?;
        let (first, last) = script.split_once(',').unwrap_or((script, script));
        let first: u64 = first.trim().parse().ok()?;
        let last: u64 = match last.trim().strip_prefix('+') {
            Some(more) => first + more.parse::<u64>().ok()?,
            None => last.trim().parse().ok()?,
        };
        (last >= first).then(|| (first.max(1), (last - first + 1).clamp(1, 400)))
    }

    pub fn read_range(cmd: &str) -> Option<(u64, u64)> {
        understood(cmd).into_iter().find(|seen| seen.action == "read").and_then(|seen| seen.lines)
    }

    pub fn place_of(path: &Path) -> PathBuf {
        let text = path.to_string_lossy();
        let Some(wild) = text.find(['*', '?']) else { return path.to_path_buf() };
        PathBuf::from(&text[..text[..wild].rfind(['/', '\\']).unwrap_or(0)])
    }

    const READ: [&str; 12] =
        ["cat", "type", "get-content", "gc", "bat", "batcat", "less", "more", "nl", "head", "tail", "sed"];
    const SEARCH: [&str; 9] = ["rg", "grep", "egrep", "fgrep", "ag", "ack", "findstr", "select-string", "sls"];
    const LIST: [&str; 8] = ["ls", "dir", "gci", "get-childitem", "tree", "find", "fd", "fdfind"];
    const POWERSHELL: [&str; 10] = [
        "-path",
        "-literalpath",
        "-totalcount",
        "-head",
        "-first",
        "-tail",
        "-last",
        "-encoding",
        "-readcount",
        "-delimiter",
    ];

    fn read(program: &str, words: &[String]) -> Option<Seen> {
        let valued: &[&str] = match program {
            "sed" => &["-e", "-f", "--expression"],
            "head" | "tail" => &["-n", "-c", "--lines", "--bytes"],
            "get-content" | "gc" | "type" | "cat" => &POWERSHELL,
            _ => &[],
        };
        let args = args(words, valued, false);
        let dash_count = || words.iter().find_map(|word| word.strip_prefix('-')?.parse::<u64>().ok());
        let (path, lines) = match program {
            "sed" => {
                if !args.has(&["-n"]) {
                    return None;
                }
                let (script, files) = match args.get(&["-e"]) {
                    Some(script) => (Some(script), &args.positional[..]),
                    None => (args.positional.first().map(String::as_str), args.positional.get(1..).unwrap_or_default()),
                };
                (files.first().cloned(), script.and_then(sed_range))
            }
            "head" => {
                let count = number(args.get(&["-n", "--lines"])).or_else(dash_count);
                (args.positional.first().cloned(), Some((1, count.unwrap_or(10).clamp(1, 400))))
            }
            "get-content" | "gc" | "type" | "cat" => {
                let path = args.get(&["-literalpath", "-path"]).map(str::to_string);
                let count = number(args.get(&["-totalcount", "-head", "-first"]));
                (path.or_else(|| args.positional.first().cloned()), count.map(|count| (1, count.clamp(1, 400))))
            }
            _ => (args.positional.first().cloned(), None),
        };
        let path = path?;
        if path.contains(['*', '?']) {
            return Some(Seen { action: "search", path: Some(path), lines: None, note: String::new() });
        }
        Some(Seen { action: "read", path: Some(path), lines, note: String::new() })
    }

    fn search(program: &str, words: &[String]) -> Option<Seen> {
        let valued = [
            "-e",
            "-f",
            "-g",
            "--glob",
            "--iglob",
            "-t",
            "--type",
            "-T",
            "--type-not",
            "-A",
            "-B",
            "-C",
            "-m",
            "--max-count",
            "-M",
            "--max-columns",
            "--max-depth",
            "--maxdepth",
            "-j",
            "--threads",
            "--sort",
            "--context",
            "--after-context",
            "--before-context",
            "--color",
            "--colors",
            "--encoding",
            "--regexp",
            "-pattern",
            "-path",
            "-literalpath",
            "-context",
            "-include",
            "-exclude",
        ];
        let args = args(words, &valued, program == "findstr");
        let files_only = args.has(&["--files", "-l", "--files-with-matches"]) && args.get(&["-e"]).is_none();
        let given = args.get(&["-e", "--regexp", "-pattern", "/c"]);
        let mut positional = args.positional.iter();
        let pattern = match (given, args.has(&["--files"])) {
            (Some(pattern), _) => Some(pattern.to_string()),
            (None, true) => None,
            (None, false) => positional.next().cloned(),
        };
        let place = args.get(&["-path", "-literalpath"]).map(str::to_string).or_else(|| positional.next().cloned());
        let note = match (pattern, files_only) {
            (Some(pattern), _) if !pattern.is_empty() => format!("for {pattern}"),
            _ => String::new(),
        };
        Some(Seen { action: "search", path: place, lines: None, note })
    }

    fn list(program: &str, words: &[String]) -> Option<Seen> {
        let valued: &[&str] = match program {
            "fd" | "fdfind" => &["-e", "--extension", "-t", "--type", "-d", "--max-depth", "-E", "--exclude"],
            "find" => &["-name", "-iname", "-type", "-maxdepth", "-mindepth", "-path", "-newer", "-size"],
            _ => &["-path", "-literalpath", "-filter", "-include", "-exclude", "-depth", "-I", "--ignore"],
        };
        let args = args(words, valued, program == "dir");
        let named = args.get(&["-path", "-literalpath"]).map(str::to_string).filter(|_| program != "find");
        let (place, pattern) = match program {
            "fd" | "fdfind" => (args.positional.get(1).cloned(), args.positional.first().cloned()),
            _ => (named.or_else(|| args.positional.first().cloned()), None),
        };
        let pattern = pattern.or_else(|| args.get(&["-filter", "-include", "-name", "-iname"]).map(str::to_string));
        let note = pattern.map(|pattern| format!("for {pattern}")).unwrap_or_default();
        Some(Seen { action: "search", path: place, lines: None, note })
    }

    fn piped(seen: &mut Seen, stage: &[String]) {
        let Some(first) = stage.first() else { return };
        let program = program(first);
        let rest = &stage[1..];
        let valued = ["-n", "--lines", "-first", "-skip", "-last", "-index"];
        let args = args(rest, &valued, false);
        let dash_count = || rest.iter().find_map(|word| word.strip_prefix('-')?.parse::<u64>().ok());
        let lines = match program.as_str() {
            "head" => number(args.get(&["-n", "--lines"])).or_else(dash_count).map(|count| (1, count)),
            "select-object" | "select" => {
                let first = number(args.get(&["-first"]));
                let skip = number(args.get(&["-skip"])).unwrap_or(0);
                first.map(|count| (skip + 1, count))
            }
            "sed" => args.positional.first().and_then(|script| sed_range(script)),
            _ => None,
        };
        if let (None, Some((first, count))) = (seen.lines, lines) {
            seen.lines = Some((first.max(1), count.clamp(1, 400)));
        }
    }

    pub fn understood(command: &str) -> Vec<Seen> {
        let mut stages: Vec<Vec<Vec<String>>> = vec![vec![Vec::new()]];
        for piece in pieces(command) {
            let statement = stages.last_mut().expect("never empty");
            match piece {
                Piece::Word(word) => statement.last_mut().expect("never empty").push(word),
                Piece::Pipe => statement.push(Vec::new()),
                Piece::End => stages.push(vec![Vec::new()]),
            }
        }
        let mut seen = Vec::new();
        for statement in stages {
            let Some(words) = statement.first().filter(|words| !words.is_empty()) else { continue };
            let mut words = words.as_slice();
            while words.len() > 1 && matches!(program(&words[0]).as_str(), "sudo" | "command" | "time" | "nice") {
                words = &words[1..];
            }
            let mut name = program(&words[0]);
            if name == "git" && words.get(1).is_some_and(|word| word == "grep") {
                words = &words[1..];
                name = "grep".to_string();
            }
            let rest = &words[1..];
            let found = match name.as_str() {
                name if READ.contains(&name) => read(name, rest),
                name if SEARCH.contains(&name) => search(name, rest),
                name if LIST.contains(&name) => list(name, rest),
                _ => None,
            };
            let Some(mut found) = found else { continue };
            let stages = if found.action == "read" { &statement[1..] } else { &[] };
            for stage in stages {
                piped(&mut found, stage);
            }
            seen.push(found);
        }
        seen
    }
}

struct Followed {
    offset: u64,
    session: Session,
    unread: bool,
    grew: Instant,
    looked: Instant,
}

impl Followed {
    fn new(file: &Path) -> Followed {
        let now = Instant::now();
        Followed { offset: 0, session: Session::of_file(file), unread: false, grew: now, looked: now }
    }
}

fn is_rollout(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    name.starts_with("rollout-") && name.ends_with(".jsonl")
}

fn numbered(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()))
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    found.sort_by(|a, b| b.cmp(a));
    found
}

/// The session files worth following: those of the newest days (Codex keeps
/// them in `sessions/<year>/<month>/<day>/`) and any left in `sessions/` itself.
fn rollouts(sessions: &Path) -> Vec<PathBuf> {
    let mut days = vec![sessions.to_path_buf()];
    'days: for year in numbered(sessions) {
        for month in numbered(&year) {
            for day in numbered(&month) {
                days.push(day);
                if days.len() > DAYS {
                    break 'days;
                }
            }
        }
    }
    let mut files = Vec::new();
    for day in days {
        let Ok(entries) = std::fs::read_dir(&day) else { continue };
        files.extend(entries.flatten().map(|entry| entry.path()).filter(|path| is_rollout(path)));
    }
    files
}

fn last_said(path: &Path) -> Option<Duration> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(LAST_LOOK))).ok()?;
    let mut tail = Vec::new();
    file.take(LAST_LOOK).read_to_end(&mut tail).ok()?;
    let text = String::from_utf8_lossy(&tail);
    let mark = "\"timestamp\"";
    let after = text[text.rfind(mark)? + mark.len()..].trim_start().strip_prefix(':')?;
    let stamp = after.trim_start().strip_prefix('"')?;
    age_of_stamp(&stamp[..stamp.find('"')?])
}

fn read_lines(path: &Path, followed: &mut Followed, upto: u64, heard: &mut Vec<Value>, silent: bool) {
    let Ok(mut file) = File::open(path) else { return };
    if upto <= followed.offset || file.seek(SeekFrom::Start(followed.offset)).is_err() {
        return;
    }
    let mut reader = BufReader::with_capacity(64 << 10, file.take(upto - followed.offset));
    let mut line = Vec::new();
    let mut withheld: Option<(Value, Duration)> = None;
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.last() != Some(&b'\n') {
            if line.len() as u64 > LONGEST_LINE {
                followed.offset += line.len() as u64;
            }
            break;
        }
        followed.offset += line.len() as u64;
        let Ok(value) = serde_json::from_slice::<Value>(&line) else { continue };
        let age = age_of(&value);
        let events = followed.session.hear(&value);
        if !silent && age.is_none_or(|age| age <= FRESH) {
            withheld = None;
            heard.extend(events);
        } else if let Some(last) = events.into_iter().last() {
            withheld = Some((last, age.unwrap_or_default()));
        }
    }
    if let (false, Some((last, age))) = (silent, withheld) {
        if age < RECENT && last.get("action").and_then(Value::as_str) != Some("done") {
            heard.push(last);
        }
    }
}

struct Follower {
    sessions: PathBuf,
    tx: Sender<Value>,
    files: HashMap<PathBuf, Followed>,
    watcher: Option<RecommendedWatcher>,
    unwatchable: bool,
    woken: Receiver<PathBuf>,
    wake: Sender<PathBuf>,
    scanned: Option<Instant>,
}

impl Follower {
    fn new(sessions: PathBuf, tx: Sender<Value>) -> Follower {
        let (wake, woken) = mpsc::channel();
        Follower { sessions, tx, files: HashMap::new(), watcher: None, unwatchable: false, woken, wake, scanned: None }
    }

    fn run(mut self, stop: &AtomicBool) {
        while !stop.load(Ordering::Relaxed) {
            self.watch();
            let mut named = Vec::new();
            match self.woken.recv_timeout(TICK) {
                Ok(path) => {
                    named.push(path);
                    named.extend(self.woken.try_iter());
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
            if stop.load(Ordering::Relaxed) {
                return;
            }
            named.sort();
            named.dedup();
            let mut heard = Vec::new();
            for path in named.iter().filter(|path| is_rollout(path)) {
                self.look(path, &mut heard);
            }
            if self.scanned.is_none_or(|at| at.elapsed() >= TICK) {
                self.scan(&mut heard);
            }
            if heard.is_empty() {
                continue;
            }
            for event in heard {
                if self.tx.send(event).is_err() {
                    return;
                }
            }
            crate::ext::wake_ui();
        }
    }

    fn watch(&mut self) {
        if self.watcher.is_some() || self.unwatchable || !self.sessions.is_dir() {
            return;
        }
        let wake = self.wake.clone();
        let made = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            for path in event.paths {
                let _ = wake.send(path);
            }
        });
        let watching = made.and_then(|mut watcher| {
            watcher.watch(&self.sessions, RecursiveMode::Recursive)?;
            Ok(watcher)
        });
        match watching {
            Ok(watcher) => self.watcher = Some(watcher),
            Err(why) => {
                log::info!("Codex's sessions are not watched ({why}): they are looked at every {TICK:?}");
                self.unwatchable = true;
            }
        }
    }

    fn scan(&mut self, heard: &mut Vec<Value>) {
        let first = self.scanned.is_none();
        self.scanned = Some(Instant::now());
        for file in rollouts(&self.sessions) {
            if !self.files.contains_key(&file) {
                self.found(file, first, heard);
            }
        }
        let due: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|(_, file)| file.grew.elapsed() < RESTS_AFTER || file.looked.elapsed() >= RESTING_LOOK)
            .map(|(path, _)| path.clone())
            .collect();
        for path in due {
            self.look(&path, heard);
        }
    }

    fn found(&mut self, file: PathBuf, before_us: bool, heard: &mut Vec<Value>) {
        let mut followed = Followed::new(&file);
        if before_us && last_said(&file).is_none_or(|age| age > RECENT) {
            followed.offset = std::fs::metadata(&file).map(|meta| meta.len()).unwrap_or(0);
            followed.unread = followed.offset > 0;
            followed.grew = Instant::now().checked_sub(RESTS_AFTER).unwrap_or(followed.grew);
            self.files.insert(file, followed);
            return;
        }
        self.files.insert(file.clone(), followed);
        self.look(&file, heard);
    }

    fn look(&mut self, path: &Path, heard: &mut Vec<Value>) {
        if !self.files.contains_key(path) {
            if path.is_file() {
                self.found(path.to_path_buf(), false, heard);
            }
            return;
        }
        let Ok(meta) = std::fs::metadata(path) else {
            self.files.remove(path);
            return;
        };
        let Some(followed) = self.files.get_mut(path) else { return };
        followed.looked = Instant::now();
        let length = meta.len();
        if length < followed.offset {
            *followed = Followed::new(path);
        }
        if length == followed.offset {
            return;
        }
        followed.grew = Instant::now();
        if std::mem::take(&mut followed.unread) {
            let upto = std::mem::take(&mut followed.offset);
            read_lines(path, followed, upto, &mut Vec::new(), true);
        }
        read_lines(path, followed, length, heard, false);
    }
}

/// Codex followed while this is kept: a thread reads the session files it
/// writes as they grow, and sends what each line says as a plain event.
pub struct Tail {
    stop: Arc<AtomicBool>,
}

impl Tail {
    pub fn start(home: &Path, tx: Sender<Value>) -> Option<Tail> {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, sessions) = (stop.clone(), home.join("sessions"));
        log::info!("Codex is followed: the sessions it writes in {} are read as they grow", sessions.display());
        let spawned = std::thread::Builder::new()
            .name("codex sessions".to_string())
            .spawn(move || Follower::new(sessions, tx).run(&flag));
        match spawned {
            Ok(_) => Some(Tail { stop }),
            Err(why) => {
                log::warn!("Codex cannot be followed: {why}");
                None
            }
        }
    }
}

impl Drop for Tail {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static str {
        if cfg!(windows) {
            "C:/work/app"
        } else {
            "/work/app"
        }
    }

    fn session() -> Session {
        let mut session =
            Session::of_file(Path::new("rollout-2026-11-17T22-42-47-01a1ec52-1f3a-77a2-92d3-3d8c4f4b2d98.jsonl"));
        let meta = json!({ "timestamp": "2026-11-18T01:42:48.113Z", "type": "session_meta",
            "payload": { "id": "01a1ec52-1f3a-77a2-92d3-3d8c4f4b2d98", "cwd": root(), "originator": "codex_vscode",
                         "cli_version": "0.169.0", "source": "vscode", "base_instructions": { "text": "..." } } });
        let heard = session.hear(&meta);
        assert_eq!(heard[0]["action"], "think");
        session
    }

    fn item(item: Value) -> Value {
        json!({ "type": "event_msg", "payload": { "type": "item_completed", "thread_id": "t", "turn_id": "u", "item": item } })
    }

    fn response(payload: Value) -> Value {
        json!({ "type": "response_item", "payload": payload })
    }

    fn under(rel: &str) -> String {
        Path::new(root()).join(rel).to_string_lossy().into_owned()
    }

    #[test]
    fn a_session_is_named_by_the_first_thing_asked() {
        let mut session = session();
        let context = response(json!({ "type": "message", "role": "user", "content": [
            { "type": "input_text", "text": "# AGENTS.md instructions\n\n<INSTRUCTIONS>be kind</INSTRUCTIONS>" },
            { "type": "input_text", "text": "<environment_context>\n  <cwd>C:/work/app</cwd>\n</environment_context>" } ] }));
        assert!(session.hear(&context).is_empty(), "what Codex adds is not what was asked");
        let asked = item(json!({ "type": "UserMessage", "id": "1", "content": [
            { "type": "local_image", "path": "a.png" },
            { "type": "text", "text": "# Context from my IDE setup:\n\n## Active file: src/a.rs\n\n## My request for Codex:\nMake the parser faster\nand keep the tests" } ] }));
        let heard = session.hear(&asked);
        assert_eq!(heard[0]["task"], "Make the parser faster\nand keep the tests");
        assert_eq!(heard[0]["name"], "Codex");
        assert_eq!(heard[0]["agent"], "codex:01a1ec52-1f3a-77a2-92d3-3d8c4f4b2d98");
        session.hear(&json!({ "type": "event_msg", "payload": { "type": "user_message", "message": "Now the docs" } }));
        assert_eq!(session.task, "Make the parser faster\nand keep the tests", "the first prompt names it");
        let done = json!({ "type": "event_msg", "payload": { "type": "task_complete", "turn_id": "u" } });
        assert_eq!(session.hear(&done)[0]["action"], "wait");
    }

    #[test]
    fn what_codex_runs_is_read_as_reading_searching_or_running() {
        let mut session = session();
        let ran = item(json!({ "type": "CommandExecution", "id": "exec-1", "command": ["powershell.exe", "-Command",
                "Get-Content -LiteralPath 'src/lib.rs' | Select-Object -Skip 39 -First 20"],
            "cwd": format!("file:///{}", root().trim_start_matches('/')), "parsed_cmd": [
                { "type": "read", "cmd": "Get-Content -LiteralPath 'src/lib.rs'", "name": "lib.rs", "path": "src/lib.rs" } ],
            "status": "completed", "exit_code": 0 }));
        let heard = session.hear(&ran);
        assert_eq!(
            (heard[0]["action"].as_str(), heard[0]["path"].as_str()),
            (Some("read"), Some(under("src/lib.rs").as_str()))
        );

        let shell = |command: &str| {
            let call = response(json!({ "type": "function_call", "name": "shell_command", "call_id": "c",
                "arguments": json!({ "command": command, "workdir": root() }).to_string() }));
            session.clone().hear(&call)
        };
        let heard = shell("sed -n '120,160p' src/main.rs");
        assert_eq!(
            (heard[0]["action"].as_str(), heard[0]["line"].as_u64(), heard[0]["lines"].as_u64()),
            (Some("read"), Some(120), Some(41))
        );
        let heard = shell("cat src/a.rs | head -n 30");
        assert_eq!((heard[0]["line"].as_u64(), heard[0]["lines"].as_u64()), (Some(1), Some(30)));
        let heard = shell("rg -n \"fn parse|fn lex\" crates -g '*.rs' 2>&1");
        assert_eq!(heard[0]["action"], "search");
        assert_eq!(heard[0]["note"], "for fn parse|fn lex");
        assert_eq!(heard[0]["path"].as_str(), Some(under("crates").as_str()));
        let heard = shell("Select-String -Pattern 'TODO' -Path src\\*.rs");
        assert_eq!(
            (heard[0]["note"].as_str(), heard[0]["path"].as_str()),
            (Some("for TODO"), Some(under("src").as_str()))
        );
        let heard = shell("cargo test --all && git status");
        assert_eq!(
            (heard[0]["action"].as_str(), heard[0]["note"].as_str()),
            (Some("run"), Some("cargo test --all && git status"))
        );
        let heard = shell("ls");
        assert_eq!((heard[0]["action"].as_str(), heard[0]["path"].as_str()), (Some("search"), Some(root())));
        let heard = shell("rg -l parse src | head -n 5");
        assert!(heard[0].get("line").is_none(), "a search has no lines of its own");
        let heard = shell("Get-ChildItem -Path (Join-Path $root 'x') -Filter *.png");
        assert_eq!((heard[0]["path"].as_str(), heard[0]["note"].as_str()), (Some(root()), Some("for *.png")));
    }

    #[test]
    fn what_codex_changes_is_where_it_is_with_the_line_when_it_is_known() {
        let mut session = session();
        let changed = item(json!({ "type": "FileChange", "id": "p", "status": "completed", "changes": {
            under("src/new.rs"): { "type": "add", "content": "fn one() {}\nfn two() {}\n" },
            under("src/lib.rs"): { "type": "update", "move_path": null,
                "unified_diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,4 +10,5 @@ impl Parser {\n     let a = 1;\n-    let b = 2;\n+    let b = 3;\n+    let c = 4;\n     a\n" } } }));
        let heard = session.hear(&changed);
        assert_eq!(heard.len(), 2);
        assert_eq!(
            (heard[0]["action"].as_str(), heard[0]["line"].as_u64(), heard[0]["lines"].as_u64()),
            (Some("edit"), Some(1), Some(2))
        );
        assert_eq!(
            (heard[1]["path"].as_str(), heard[1]["line"].as_u64(), heard[1]["lines"].as_u64()),
            (Some(under("src/lib.rs").as_str()), Some(11), Some(2))
        );

        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@ fn main() {\n-    old();\n+    new_call(1);\n*** Delete File: src/gone.rs\n*** End Patch";
        let heard =
            session.hear(&response(json!({ "type": "custom_tool_call", "name": "apply_patch", "input": patch })));
        assert_eq!(heard.len(), 2);
        assert_eq!(
            (heard[0]["path"].as_str(), heard[0]["find"].as_str()),
            (Some(under("src/a.rs").as_str()), Some("new_call(1);"))
        );
        assert!(heard[1].get("find").is_none() && heard[1]["action"] == "edit");

        let legacy = json!({ "type": "event_msg", "payload": { "type": "patch_apply_begin", "call_id": "c", "auto_approved": true,
            "changes": { under("b.rs"): { "update": { "unified_diff": "@@ -1 +1 @@\n-a\n+b\n", "move_path": under("c.rs") } } } } });
        let heard = session.hear(&legacy);
        assert_eq!((heard[0]["path"].as_str(), heard[0]["line"].as_u64()), (Some(under("c.rs").as_str()), Some(1)));
    }

    #[test]
    fn the_code_codex_runs_its_tools_from_is_read_for_what_they_do() {
        let mut session = session();
        let code = "// @exec: {\"yield_time_ms\": 1000}\ntext((await tools.exec_command({cmd:\"Get-Content -LiteralPath 'C:/work/app/src/a.rs' -TotalCount 80\",\"max_output_tokens\":3500})).output)\n";
        let heard = session
            .hear(&response(json!({ "type": "custom_tool_call", "name": "exec", "call_id": "c", "input": code })));
        assert_eq!(
            (heard[0]["action"].as_str(), heard[0]["line"].as_u64(), heard[0]["lines"].as_u64()),
            (Some("read"), Some(1), Some(80))
        );
        assert!(heard[0]["path"].as_str().unwrap().ends_with("src/a.rs"));
        let code = "await tools.apply_patch(\"*** Begin Patch\\n*** Update File: src/b.rs\\n@@\\n-x\\n+let y = \\\"two\\\";\\n*** End Patch\")";
        let heard = session.hear(&response(json!({ "type": "custom_tool_call", "name": "exec", "input": code })));
        assert_eq!((heard[0]["action"].as_str(), heard[0]["find"].as_str()), (Some("edit"), Some("let y = \"two\";")));
        let code = "const r = await tools.image_gen__imagegen({prompt: \"a cat\"})";
        let heard = session.hear(&response(json!({ "type": "custom_tool_call", "name": "exec", "input": code })));
        assert_eq!(
            (heard[0]["action"].as_str(), heard[0]["note"].as_str()),
            (Some("think"), Some("image_gen__imagegen"))
        );
    }

    #[test]
    fn what_is_not_understood_is_let_be() {
        let mut session = session();
        for odd in [
            json!(null),
            json!([1, 2]),
            json!({ "type": "response_item" }),
            json!({ "type": "response_item", "payload": { "type": "function_call", "name": "shell", "arguments": "{not json" } }),
            json!({ "type": "event_msg", "payload": { "type": "item_completed", "item": { "type": "FileChange", "changes": [1] } } }),
            json!({ "type": "event_msg", "payload": { "type": "item_completed", "item": { "type": "Hologram", "path": 3 } } }),
            json!({ "type": "something_new", "payload": { "type": "message" } }),
            json!({ "type": "event_msg", "payload": { "type": "exec_command_begin", "parsed_cmd": "no" } }),
        ] {
            let heard = session.hear(&odd);
            assert!(heard.iter().all(|event| event["agent"].as_str().is_some()), "{odd}");
        }
    }

    #[test]
    fn a_helper_belongs_to_its_session_and_codex_exec_leaves_when_done() {
        let mut helper = Session::of_file(Path::new("rollout-x-11111111-2222-3333-4444-555555555555.jsonl"));
        helper.hear(&json!({ "type": "session_meta", "payload": { "id": "11111111-2222-3333-4444-555555555555",
            "source": { "subagent": { "thread_spawn": { "parent_thread_id": "p-1", "depth": 1, "agent_role": "explorer" } } } } }));
        let heard = helper.hear(&json!({ "type": "event_msg", "payload": { "type": "task_started" } }));
        assert_eq!(heard[0]["agent"], "codex:p-1/11111111-2222-3333-4444-555555555555");
        assert_eq!((heard[0]["session"].as_str(), heard[0]["name"].as_str()), (Some("codex:p-1"), Some("explorer")));
        let mut compact = Session::of_file(Path::new("rollout-c.jsonl"));
        compact.hear(&json!({ "type": "session_meta", "payload": { "id": "c", "source": { "subagent": "compact" } } }));
        assert!(compact.hear(&json!({ "type": "event_msg", "payload": { "type": "task_started" } })).is_empty());
        let mut once = Session::of_file(Path::new("rollout-e.jsonl"));
        once.hear(&json!({ "type": "session_meta", "payload": { "id": "e", "source": "exec" } }));
        let heard = once.hear(&json!({ "type": "event_msg", "payload": { "type": "task_complete" } }));
        assert_eq!(heard[0]["action"], "done");
    }

    #[test]
    fn a_time_of_codex_is_read_without_a_calendar_library() {
        assert_eq!(seconds_of("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(seconds_of("2026-11-18T01:44:29.074Z"), Some(1_794_966_269));
        assert_eq!(seconds_of("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(seconds_of("soon"), None);
        assert_eq!(
            from_url("file:///C:/rysted/my%20app"),
            if cfg!(windows) { "C:/rysted/my app" } else { "/C:/rysted/my app" }
        );
    }

    #[test]
    fn a_session_file_is_followed_as_it_grows_and_an_old_one_is_not_told_again() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("code-canvas-test-codex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let day = dir.join("sessions").join("2026").join("01").join("02");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::create_dir_all(dir.join("sessions").join("2025").join("12").join("31")).unwrap();
        let old = dir.join("sessions").join("2025").join("12").join("30");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("rollout-old.jsonl"), "{}\n").unwrap();
        assert_eq!(rollouts(&dir.join("sessions")), Vec::<PathBuf>::new(), "only the newest days are looked at");

        let stale = day.join("rollout-2026-01-02T10-00-00-aaaaaaaa-0000-0000-0000-000000000000.jsonl");
        let line = |stamp: &str, payload: Value| {
            format!("{}\n", json!({ "timestamp": stamp, "type": "event_msg", "payload": payload }))
        };
        std::fs::write(
            &stale,
            line("2026-01-02T10:00:00.000Z", json!({ "type": "user_message", "message": "Old work" })),
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut follower = Follower::new(dir.join("sessions"), tx);
        let mut heard = Vec::new();
        follower.scan(&mut heard);
        assert!(heard.is_empty(), "a session that said nothing for long is not brought back");

        let file = day.join("rollout-2026-01-02T11-00-00-bbbbbbbb-0000-0000-0000-000000000000.jsonl");
        let mut writer = std::fs::OpenOptions::new().create(true).append(true).open(&file).unwrap();
        writer
            .write_all(
                line("2020-01-01T00:00:00Z", json!({ "type": "user_message", "message": "Fix the build" })).as_bytes(),
            )
            .unwrap();
        writer.write_all(b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_mess").unwrap();
        writer.flush().unwrap();
        follower.scan(&mut heard);
        assert!(heard.is_empty(), "what was said long ago is not told, and half a line waits for the rest");
        writer.write_all(b"age\",\"message\":\"x\"}}\n").unwrap();
        writer
            .write_all(
                format!(
                    "{}\n",
                    json!({ "type": "event_msg", "payload": { "type": "exec_command_begin",
            "command": ["bash", "-lc", "cat src/a.rs"], "cwd": "/w", "parsed_cmd": [] } })
                )
                .as_bytes(),
            )
            .unwrap();
        writer.flush().unwrap();
        follower.look(&file, &mut heard);
        assert_eq!(heard.len(), 2, "{heard:?}");
        assert_eq!((heard[1]["action"].as_str(), heard[1]["task"].as_str()), (Some("read"), Some("Fix the build")));

        std::fs::write(&stale, "").unwrap();
        writer.write_all(line("2020-01-01T00:00:00Z", json!({ "type": "task_complete" })).as_bytes()).unwrap();
        writer.flush().unwrap();
        heard.clear();
        follower.scan(&mut heard);
        assert!(heard.is_empty(), "an old line is not told");
        drop((writer, rx));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads (never writes) every session file of the Codex of this computer and
    /// counts what was understood: `CODEX_SESSIONS_TEST=<CODEX_HOME>/sessions`.
    #[test]
    #[ignore]
    fn real_sessions_are_understood() {
        let root = PathBuf::from(std::env::var("CODEX_SESSIONS_TEST").expect("set CODEX_SESSIONS_TEST"));
        let mut files = Vec::new();
        let mut folders = vec![root];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
                match entry.path() {
                    path if path.is_dir() => folders.push(path),
                    path if is_rollout(&path) => files.push(path),
                    _ => {}
                }
            }
        }
        let mut counted: std::collections::BTreeMap<String, usize> = Default::default();
        for file in &files {
            let mut session = Session::of_file(file);
            let text = std::fs::read(file).unwrap_or_default();
            for line in text.split(|byte| *byte == b'\n') {
                let Ok(value) = serde_json::from_slice::<Value>(line) else { continue };
                for event in session.hear(&value) {
                    let action = event["action"].as_str().unwrap_or_default().to_string();
                    let ranged = if event.get("line").is_some() { " with lines" } else { "" };
                    *counted.entry(format!("{action}{ranged}")).or_default() += 1;
                    if std::env::var_os("CODEX_SESSIONS_SHOW").is_some() && action != "think" {
                        let note: String = event["note"].as_str().unwrap_or_default().chars().take(100).collect();
                        println!("{action} {} {:?} {note:?}", event["path"], event.get("line"));
                    }
                }
            }
            assert!(!session.task.starts_with('<'), "{}", session.task);
        }
        println!("{} sessions: {counted:?}", files.len());
    }
}
