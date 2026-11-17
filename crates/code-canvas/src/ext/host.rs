//! The extension host process: Node running `exthost/host.js`, which loads
//! the installed VS Code extensions. One JSON object per line each way.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::Value;

use super::Waker;

/// The host script, embedded so an installed app needs no extra files.
const HOST_JS: &str = include_str!("../../exthost/host.js");

pub enum HostEvent {
    Message(Value),
    /// Something the host wrote outside the protocol (Node's own errors).
    Stderr(String),
    Exited,
}

pub struct Host {
    child: Child,
    /// Lines for the host. A thread of its own writes them: when the host
    /// is busy its pipe fills up, and a write would block whoever does it.
    /// That must never be the UI thread.
    out: Option<mpsc::Sender<String>>,
    pub rx: Receiver<HostEvent>,
}

pub fn node_command() -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut c = Command::new("node");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// `node --version`, or why Node cannot be run.
pub fn detect_node() -> Result<String, String> {
    let out = node_command().arg("--version").stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

impl Host {
    /// Write the host script under `dir` and start it.
    pub fn start(dir: &Path, waker: Waker) -> std::io::Result<Host> {
        std::fs::create_dir_all(dir)?;
        let script = dir.join("host.js");
        if std::fs::read_to_string(&script).ok().as_deref() != Some(HOST_JS) {
            std::fs::write(&script, HOST_JS)?;
        }
        let mut child =
            node_command().arg(&script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let stdin: Option<ChildStdin> = child.stdin.take();
        let (out, lines) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let Some(mut stdin) = stdin else { return };
            for line in lines {
                if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
            // The channel closed (or the host is gone): closing the pipe tells it to stop.
        });
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();

        let (tx_err, wake_err) = (tx.clone(), waker.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx_err.send(HostEvent::Stderr(line)).is_err() {
                    break;
                }
                wake_err.wake();
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let event = match serde_json::from_str::<Value>(&line) {
                    Ok(v) => HostEvent::Message(v),
                    Err(_) => HostEvent::Stderr(line),
                };
                if tx.send(event).is_err() {
                    return;
                }
                waker.wake();
            }
            let _ = tx.send(HostEvent::Exited);
            waker.wake();
        });
        Ok(Host { child, out: Some(out), rx })
    }

    /// Send one message; false when the host is gone.
    pub fn send(&mut self, message: &Value) -> bool {
        let Some(out) = self.out.as_ref() else { return false };
        let mut line = message.to_string();
        line.push('\n');
        out.send(line).is_ok()
    }

    /// Ask the host to deactivate its extensions and leave; it is killed if
    /// it does not within a few seconds. Does not block the caller.
    pub fn stop(mut self) {
        self.send(&serde_json::json!({ "type": "shutdown" }));
        self.out = None; // the writer ends and closes the pipe, which also tells it to stop
        std::thread::spawn(move || {
            for _ in 0..40 {
                if matches!(self.child.try_wait(), Ok(Some(_)) | Err(_)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            kill_tree(&mut self.child);
        });
    }
}

/// Kill the host and whatever it started (language servers).
fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let _ = node_command_free("taskkill").args(["/PID", &child.id().to_string(), "/T", "/F"]).output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(windows)]
fn node_command_free(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}
