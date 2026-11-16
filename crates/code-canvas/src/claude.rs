//! Chat with Claude Code from inside the app.
//!
//! This does not talk to an API directly: it runs the user's own `claude`
//! command-line tool in non-interactive mode (`claude -p`), so it uses
//! whatever account or subscription that tool is already signed in with and
//! needs no API key. Each message is one process; `--resume <session id>`
//! keeps the conversation going. Output is read as `stream-json`: one JSON
//! event per line.
//!
//! By default Claude may only read the repository (`Read`, `Grep`, `Glob`).
//! Editing files requires the user to switch on "allow edits".

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    User,
    Assistant,
    /// A tool Claude used (e.g. reading a file).
    Tool,
    Info,
    Error,
}

pub struct ChatMsg {
    pub role: Role,
    pub text: String,
}

enum Event {
    Session(String),
    Text(String),
    Tool(String),
    Done { is_error: bool, cost_usd: f64, duration_ms: u64 },
    Failed(String),
}

struct Run {
    rx: Receiver<Event>,
    pid: u32,
    started: Instant,
    /// Repository root, to show tool paths relative to it.
    root: String,
}

/// Show a tool's path argument relative to the repository root,
/// e.g. `Read <root>/src/a.rs` becomes `Read src/a.rs`.
fn relative_to_root(text: &str, root: &str) -> String {
    let unified = text.replace('\\', "/");
    let root = root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    match unified.find(root) {
        Some(at) if !root.is_empty() => {
            let rest = unified[at + root.len()..].trim_start_matches('/');
            format!("{}{}", &unified[..at], rest)
        }
        _ => text.to_string(),
    }
}

#[derive(Default)]
pub struct Chat {
    pub messages: Vec<ChatMsg>,
    pub input: String,
    /// Let Claude edit files in the repository (off by default).
    pub allow_edits: bool,
    /// Send the selected symbols / file along with the message.
    pub include_selection: bool,
    session_id: Option<String>,
    run: Option<Run>,
}

/// `claude --version`, or why it could not be run.
pub fn detect() -> Result<String, String> {
    let out = command().arg("--version").stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The `claude` command. On Windows it is an npm `.cmd` shim, which has to
/// be started through `cmd`; no console window is shown.
fn command() -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = Command::new("cmd");
        c.args(["/C", "claude"]).creation_flags(CREATE_NO_WINDOW);
        c
    }
    #[cfg(not(windows))]
    {
        Command::new("claude")
    }
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `cmd /C` is only the parent: kill the whole tree.
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/// One line describing a tool call, e.g. `Read src/app.rs`.
fn describe_tool(name: &str, input: &serde_json::Value) -> String {
    let arg = ["file_path", "path", "pattern", "command", "query", "url"]
        .iter()
        .find_map(|k| input.get(*k).and_then(|v| v.as_str()))
        .unwrap_or("");
    let arg: String = arg.chars().take(120).collect();
    format!("{name} {arg}").trim().to_string()
}

/// Turn one `stream-json` line into events.
fn parse_line(line: &str, out: &mut Vec<Event>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("system") => {
            if let Some(id) = v.get("session_id").and_then(|s| s.as_str()) {
                out.push(Event::Session(id.to_string()));
            }
        }
        Some("assistant") => {
            let blocks = v.pointer("/message/content").and_then(|c| c.as_array());
            for b in blocks.into_iter().flatten() {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(|t| t.as_str()).filter(|t| !t.trim().is_empty()) {
                            out.push(Event::Text(t.to_string()));
                        }
                    }
                    Some("tool_use") => {
                        let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                        out.push(Event::Tool(describe_tool(name, b.get("input").unwrap_or(&serde_json::Value::Null))));
                    }
                    _ => {}
                }
            }
        }
        Some("result") => {
            if let Some(id) = v.get("session_id").and_then(|s| s.as_str()) {
                out.push(Event::Session(id.to_string()));
            }
            let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
            if is_error {
                if let Some(text) = v.get("result").and_then(|r| r.as_str()) {
                    out.push(Event::Failed(text.to_string()));
                }
            }
            out.push(Event::Done {
                is_error,
                cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0),
                duration_ms: v.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0),
            });
        }
        _ => {}
    }
}

fn spawn(
    cwd: &Path,
    prompt: &str,
    resume: Option<&str>,
    allow_edits: bool,
) -> std::io::Result<(Child, Receiver<Event>)> {
    let mut cmd = command();
    cmd.args(["-p", "--output-format", "stream-json", "--verbose"]);
    if let Some(id) = resume {
        cmd.args(["--resume", id]);
    }
    // Only the tools this panel needs, and none of the user's MCP servers:
    // loading everything makes every message slower and far more expensive.
    cmd.arg("--strict-mcp-config");
    if allow_edits {
        cmd.args(["--tools", "Read,Grep,Glob,Edit,Write", "--permission-mode", "acceptEdits"]);
    } else {
        cmd.args(["--tools", "Read,Grep,Glob", "--allowedTools", "Read,Grep,Glob"]);
    }
    cmd.args([
        "--append-system-prompt",
        "You are running inside Code Canvas, a visual code explorer. Be concise. When you mention code, give the file path relative to the repository root followed by a colon and the line number, so the user can jump to it on the canvas.",
    ]);
    let mut child = cmd.current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    // The prompt goes through stdin: no shell quoting problems.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::channel();

    let err_tx = tx.clone();
    std::thread::spawn(move || {
        // Keep the last non-empty stderr line in case the run fails without a result.
        let mut last = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() && !line.starts_with("[claude-code:") {
                last = line;
            }
        }
        if !last.is_empty() {
            let _ = err_tx.send(Event::Failed(last));
        }
    });
    std::thread::spawn(move || {
        let mut finished = false;
        let mut events = Vec::new();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            parse_line(&line, &mut events);
            for e in events.drain(..) {
                finished |= matches!(e, Event::Done { .. });
                if tx.send(e).is_err() {
                    return;
                }
            }
        }
        if !finished {
            let _ = tx.send(Event::Done { is_error: true, cost_usd: 0.0, duration_ms: 0 });
        }
    });
    Ok((child, rx))
}

impl Chat {
    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    pub fn elapsed_secs(&self) -> f32 {
        self.run.as_ref().map(|r| r.started.elapsed().as_secs_f32()).unwrap_or(0.0)
    }

    /// Send `text` (what the user sees) with `context` prepended (what the
    /// model also gets: selected symbols, files…).
    pub fn send(&mut self, root: &Path, text: String, context: &str) {
        if self.run.is_some() || text.trim().is_empty() {
            return;
        }
        let prompt = if context.is_empty() { text.clone() } else { format!("{context}\n\n{text}") };
        self.messages.push(ChatMsg { role: Role::User, text });
        match spawn(root, &prompt, self.session_id.as_deref(), self.allow_edits) {
            Ok((child, rx)) => {
                let pid = child.id();
                // Reap the process when it ends so it does not linger as a zombie.
                std::thread::spawn(move || {
                    let mut child = child;
                    let _ = child.wait();
                });
                self.run = Some(Run { rx, pid, started: Instant::now(), root: root.to_string_lossy().into_owned() });
            }
            Err(e) => self.messages.push(ChatMsg {
                role: Role::Error,
                text: format!(
                    "Could not start the `claude` command: {e}.\nInstall Claude Code and sign in, then try again."
                ),
            }),
        }
    }

    pub fn stop(&mut self) {
        if let Some(run) = self.run.take() {
            kill_tree(run.pid);
            self.messages.push(ChatMsg { role: Role::Info, text: "Stopped.".into() });
        }
    }

    /// Start a fresh conversation.
    pub fn reset(&mut self) {
        self.stop();
        self.messages.clear();
        self.session_id = None;
    }

    /// Drain pending output. Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        let Some(run) = &self.run else { return false };
        let mut changed = false;
        let mut done = false;
        loop {
            match run.rx.try_recv() {
                Ok(Event::Session(id)) => self.session_id = Some(id),
                Ok(Event::Text(text)) => {
                    self.messages.push(ChatMsg { role: Role::Assistant, text });
                    changed = true;
                }
                Ok(Event::Tool(text)) => {
                    let text = relative_to_root(&text, &run.root);
                    self.messages.push(ChatMsg { role: Role::Tool, text });
                    changed = true;
                }
                Ok(Event::Failed(text)) => {
                    self.messages.push(ChatMsg { role: Role::Error, text });
                    changed = true;
                }
                Ok(Event::Done { is_error, cost_usd, duration_ms }) => {
                    if !is_error {
                        self.messages.push(ChatMsg {
                            role: Role::Info,
                            text: format!("{:.1} s · ${:.3}", duration_ms as f32 / 1000.0, cost_usd),
                        });
                    }
                    done = true;
                    changed = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = true;
                    changed = true;
                    break;
                }
            }
        }
        if done {
            self.run = None;
        }
        changed
    }
}

/// A place in the repository mentioned in a message: `path` or `path:line`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    pub path: String,
    /// Zero-based.
    pub line: Option<u32>,
}

/// Find `path` / `path:line` mentions of known files in `text`.
/// `is_file` says whether a relative path exists in the repository.
pub fn mentions(text: &str, is_file: impl Fn(&str) -> bool) -> Vec<Mention> {
    let mut out: Vec<Mention> = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || "`()[]<>\"'*,".contains(c)) {
        let word = raw.trim_end_matches(['.', ':', ';']).trim_start_matches("./");
        if word.len() < 3 || !word.contains('.') {
            continue;
        }
        let word = word.replace('\\', "/");
        // `path:12`, `path:12-30`, `path:12:5`
        let (path, line) = match word.split_once(':') {
            Some((p, rest)) => {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                (p.to_string(), digits.parse::<u32>().ok().map(|n| n.saturating_sub(1)))
            }
            None => (word.clone(), None),
        };
        if is_file(&path) {
            let m = Mention { path, line };
            if !out.contains(&m) {
                out.push(m);
            }
        }
        if out.len() >= 12 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stream_json() {
        let mut ev = Vec::new();
        parse_line(r#"{"type":"system","subtype":"init","session_id":"abc"}"#, &mut ev);
        parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hola"},{"type":"tool_use","name":"Read","input":{"file_path":"src/a.rs"}}]}}"#,
            &mut ev,
        );
        parse_line("[claude-code:warning] not json", &mut ev);
        parse_line(
            r#"{"type":"result","is_error":false,"result":"Hola","session_id":"abc","total_cost_usd":0.02,"duration_ms":1500}"#,
            &mut ev,
        );
        assert!(matches!(&ev[0], Event::Session(s) if s == "abc"));
        assert!(matches!(&ev[1], Event::Text(t) if t == "Hola"));
        assert!(matches!(&ev[2], Event::Tool(t) if t == "Read src/a.rs"));
        assert!(matches!(&ev[3], Event::Session(_)));
        assert!(matches!(&ev[4], Event::Done { is_error: false, duration_ms: 1500, .. }));
        assert_eq!(ev.len(), 5);
    }

    #[test]
    fn tool_paths_are_shown_relative_to_the_repository() {
        assert_eq!(relative_to_root(r"Read C:\repo\src\a.rs", r"C:\repo"), "Read src/a.rs");
        assert_eq!(relative_to_root("Read /home/me/repo/src/a.rs", "/home/me/repo/"), "Read src/a.rs");
        assert_eq!(relative_to_root("Grep useThing", r"C:\repo"), "Grep useThing");
    }

    #[test]
    fn finds_file_mentions() {
        let known = ["src/app.rs", "README.md", "src/api/types.ts"];
        let m = mentions(
            "See `src/app.rs:42` and (src/api/types.ts:10-20). Also README.md, but not other/file.rs or v1.2.",
            |p| known.contains(&p),
        );
        assert_eq!(
            m,
            vec![
                Mention { path: "src/app.rs".into(), line: Some(41) },
                Mention { path: "src/api/types.ts".into(), line: Some(9) },
                Mention { path: "README.md".into(), line: None },
            ]
        );
    }
}
