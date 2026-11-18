use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::Value;

use super::Waker;

const HOST_JS: &str = include_str!("../../exthost/host.js");

const MAX_LINE: usize = 96 * 1024 * 1024;

fn next_line(reader: &mut impl BufRead, most: usize) -> Option<Result<String, usize>> {
    let mut line = Vec::new();
    let read = std::io::Read::take(&mut *reader, most as u64).read_until(b'\n', &mut line).ok()?;
    if read == 0 {
        return None;
    }
    if line.last() != Some(&b'\n') && read == most {
        let mut skipped = read;
        loop {
            line.clear();
            let more = std::io::Read::take(&mut *reader, most as u64).read_until(b'\n', &mut line).ok()?;
            skipped += more;
            if more == 0 || line.last() == Some(&b'\n') {
                return Some(Err(skipped));
            }
        }
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Some(Ok(String::from_utf8_lossy(&line).into_owned()))
}

pub enum HostEvent {
    Message(Value),
    Stderr(String),
    Exited,
}

pub struct Host {
    child: Child,
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

pub fn detect_node() -> Result<String, String> {
    let out = node_command().arg("--version").stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

impl Host {
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
        });
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();

        let (tx_err, wake_err) = (tx.clone(), waker.clone());
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            while let Some(line) = next_line(&mut reader, MAX_LINE) {
                let line = line.unwrap_or_else(|skipped| format!("a line of {skipped} bytes was left out"));
                if tx_err.send(HostEvent::Stderr(line)).is_err() {
                    break;
                }
                wake_err.wake();
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(line) = next_line(&mut reader, MAX_LINE) {
                let event = match line {
                    Ok(line) => match serde_json::from_str::<Value>(&line) {
                        Ok(v) => HostEvent::Message(v),
                        Err(_) => HostEvent::Stderr(line),
                    },
                    Err(skipped) => HostEvent::Stderr(format!("a message of {skipped} bytes was left out")),
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

    pub fn send(&mut self, message: &Value) -> bool {
        let Some(out) = self.out.as_ref() else { return false };
        let mut line = message.to_string();
        line.push('\n');
        out.send(line).is_ok()
    }

    pub fn stop(self) {
        std::thread::spawn(move || self.stop_and_wait(40));
    }

    pub fn stop_and_wait(mut self, tenths: u32) {
        self.send(&serde_json::json!({ "type": "shutdown" }));
        self.out = None;
        for _ in 0..tenths {
            if matches!(self.child.try_wait(), Ok(Some(_)) | Err(_)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        kill_tree(&mut self.child);
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_too_long_is_left_out_and_the_next_one_still_read() {
        let text = format!("short\r\n{}\nafter\nlast", "x".repeat(25));
        let mut reader = std::io::Cursor::new(text.into_bytes());
        assert_eq!(next_line(&mut reader, 10), Some(Ok("short".to_string())));
        assert_eq!(next_line(&mut reader, 10), Some(Err(26)));
        assert_eq!(next_line(&mut reader, 10), Some(Ok("after".to_string())));
        assert_eq!(next_line(&mut reader, 10), Some(Ok("last".to_string())));
        assert_eq!(next_line(&mut reader, 10), None);
        let mut broken = std::io::Cursor::new(b"caf\xe9\n".to_vec());
        assert_eq!(
            next_line(&mut broken, 10),
            Some(Ok("caf\u{fffd}".to_string())),
            "not UTF-8 does not stop the reading"
        );
    }
}
