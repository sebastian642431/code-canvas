use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::OnceLock;

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

const KEPT_ENV: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "TEMP",
    "TMP",
    "TMPDIR",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_IDENTIFIER",
    "OS",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "TZ",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSH_AUTH_SOCK",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "JAVA_HOME",
    "GOPATH",
    "GOROOT",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "DOTNET_ROOT",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "NVM_HOME",
    "NVM_SYMLINK",
    "VOLTA_HOME",
    "PNPM_HOME",
    "CODEX_HOME",
];

const KEPT_PREFIXES: &[&str] = &["LC_", "ANTHROPIC_", "CLAUDE_"];
const KEPT_PREFIXES_UNIX: &[&str] = &["XDG_"];
const BEDROCK: (&str, &[&str]) = ("CLAUDE_CODE_USE_BEDROCK", &["AWS_"]);
const VERTEX: (&str, &[&str]) = ("CLAUDE_CODE_USE_VERTEX", &["GOOGLE_", "CLOUD_ML_REGION", "VERTEX_REGION_"]);
const NEVER_ENV: &[&str] = &["NODE_OPTIONS", "NODE_PATH"];

pub fn host_env(vars: impl IntoIterator<Item = (OsString, OsString)>, windows: bool) -> Vec<(OsString, OsString)> {
    let vars: Vec<(String, OsString)> =
        vars.into_iter().filter_map(|(k, v)| Some((k.into_string().ok()?, v))).filter(|(k, _)| !k.is_empty()).collect();
    let same = |a: &str, b: &str| if windows { a.eq_ignore_ascii_case(b) } else { a == b };
    let starts = |name: &str, prefix: &str| {
        name.len() >= prefix.len() && name.is_char_boundary(prefix.len()) && same(&name[..prefix.len()], prefix)
    };
    let set = |flag: &str| vars.iter().any(|(k, v)| same(k, flag) && !v.is_empty() && v != "0");
    let mut prefixes: Vec<&str> = KEPT_PREFIXES.to_vec();
    if !windows {
        prefixes.extend(KEPT_PREFIXES_UNIX);
    }
    for (flag, more) in [BEDROCK, VERTEX] {
        if set(flag) {
            prefixes.extend(more);
        }
    }
    vars.into_iter()
        .filter(|(k, _)| !NEVER_ENV.iter().any(|never| same(k, never)))
        .filter(|(k, _)| KEPT_ENV.iter().any(|kept| same(k, kept)) || prefixes.iter().any(|p| starts(k, p)))
        .map(|(k, v)| (OsString::from(k), v))
        .collect()
}

fn node_name() -> &'static str {
    if cfg!(windows) {
        "node.exe"
    } else {
        "node"
    }
}

fn known_installs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for (var, sub) in [("ProgramFiles", "nodejs"), ("LOCALAPPDATA", "Programs\\nodejs"), ("NVM_SYMLINK", "")] {
            if let Some(base) = std::env::var_os(var) {
                out.push(PathBuf::from(base).join(sub));
            }
        }
    } else {
        out.extend(["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"].map(PathBuf::from));
    }
    out
}

pub fn find_node(path: Option<&OsStr>, known: &[PathBuf], here: Option<&Path>) -> Option<PathBuf> {
    let here = here.and_then(|h| h.canonicalize().ok());
    let listed: Vec<PathBuf> = path.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    listed.iter().chain(known).filter(|dir| dir.is_absolute()).find_map(|dir| {
        let real_dir = dir.canonicalize().ok()?;
        if here.as_ref() == Some(&real_dir) {
            return None;
        }
        let node = real_dir.join(node_name());
        node.is_file().then(|| canvas_core::scan::clean_path(node))
    })
}

pub fn node_path() -> Option<PathBuf> {
    static NODE: OnceLock<Option<PathBuf>> = OnceLock::new();
    NODE.get_or_init(|| {
        let here = std::env::current_dir().ok();
        let found = find_node(std::env::var_os("PATH").as_deref(), &known_installs(), here.as_deref());
        match &found {
            Some(node) => log::info!("extensions run on {}", node.display()),
            None => log::warn!("Node.js was not found on the PATH nor where it is usually installed"),
        }
        found
    })
    .clone()
}

pub fn node_command(dir: &Path) -> Option<Command> {
    let mut c = Command::new(node_path()?);
    c.env_clear().envs(host_env(std::env::vars_os(), cfg!(windows))).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    Some(c)
}

pub fn detect_node() -> Result<String, String> {
    let dir = std::env::temp_dir();
    let mut node = node_command(&dir).ok_or("Node.js was not found on the PATH")?;
    let out = node.arg("--version").stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
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
        let not_found = || std::io::Error::new(std::io::ErrorKind::NotFound, "Node.js was not found");
        let mut node = node_command(dir).ok_or_else(not_found)?;
        let mut child =
            node.arg(&script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
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

    fn env(pairs: &[(&str, &str)], windows: bool) -> Vec<String> {
        let vars = pairs.iter().map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let mut kept: Vec<String> =
            host_env(vars, windows).into_iter().map(|(k, _)| k.into_string().unwrap()).collect();
        kept.sort();
        kept
    }

    #[test]
    fn the_host_gets_only_what_it_needs_of_the_environment() {
        let all = [
            ("Path", "C:\\bin"),
            ("SYSTEMROOT", "C:\\Windows"),
            ("USERPROFILE", "C:\\Users\\me"),
            ("HOME", "C:\\Users\\me"),
            ("https_proxy", "http://proxy"),
            ("LC_ALL", "C"),
            ("NODE_OPTIONS", "--require evil.js"),
            ("node_path", "C:\\evil"),
            ("GITHUB_TOKEN", "ghp_secret"),
            ("OPENAI_API_KEY", "sk-secret"),
            ("AWS_SECRET_ACCESS_KEY", "aws"),
            ("ANTHROPIC_API_KEY", "sk-ant"),
            ("CLAUDE_CONFIG_DIR", "C:\\claude"),
            ("CODE_CANVAS_SCRIPT", "x"),
            ("NODE_EXTRA_CA_CERTS", "C:\\ca.pem"),
            ("XDG_CONFIG_HOME", "/x"),
        ];
        assert_eq!(
            env(&all, true),
            [
                "ANTHROPIC_API_KEY",
                "CLAUDE_CONFIG_DIR",
                "HOME",
                "LC_ALL",
                "NODE_EXTRA_CA_CERTS",
                "Path",
                "SYSTEMROOT",
                "USERPROFILE",
                "https_proxy"
            ]
        );
        let unix = env(&all, false);
        assert!(unix.contains(&"XDG_CONFIG_HOME".to_string()) && unix.contains(&"HOME".to_string()));
        assert!(!unix.contains(&"Path".to_string()), "names are exact where the system says so");
        assert!(!unix.iter().any(|k| k.starts_with("NODE_OPTIONS") || k == "GITHUB_TOKEN"));

        let bedrock = env(&[("CLAUDE_CODE_USE_BEDROCK", "1"), ("AWS_REGION", "us-east-1"), ("GOOGLE_X", "g")], true);
        assert_eq!(bedrock, ["AWS_REGION", "CLAUDE_CODE_USE_BEDROCK"], "a cloud's keys only for Claude Code on it");
        let off = env(&[("CLAUDE_CODE_USE_BEDROCK", "0"), ("AWS_REGION", "us-east-1")], true);
        assert_eq!(off, ["CLAUDE_CODE_USE_BEDROCK"]);
        let vertex = env(&[("CLAUDE_CODE_USE_VERTEX", "1"), ("CLOUD_ML_REGION", "eu"), ("AWS_REGION", "x")], true);
        assert_eq!(vertex, ["CLAUDE_CODE_USE_VERTEX", "CLOUD_ML_REGION"]);
    }

    #[test]
    fn node_is_taken_from_a_full_path_never_from_the_current_folder() {
        let root = std::env::temp_dir().join(format!("code-canvas-test-node-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (here, listed, known, empty) =
            (root.join("here"), root.join("listed"), root.join("known"), root.join("empty"));
        for dir in [&here, &listed, &known, &empty] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for dir in [&here, &listed, &known] {
            std::fs::write(dir.join(node_name()), "").unwrap();
        }
        std::fs::create_dir_all(empty.join(node_name())).unwrap();
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
        let real = |dir: &Path| canvas_core::scan::clean_path(dir.canonicalize().unwrap().join(node_name()));
        let found = |p: &OsStr, known: &[PathBuf]| find_node(Some(p), known, Some(&here));
        let none: &[PathBuf] = &[];
        assert_eq!(found(&path(&[&here, &listed]), none), Some(real(&listed)), "the current folder is skipped");
        assert_eq!(found(&path(&[Path::new("."), &listed]), none), Some(real(&listed)));
        let known_too = [known.clone()];
        assert_eq!(
            found(&path(&[Path::new("relative"), &empty]), &known_too),
            Some(real(&known)),
            "a folder is no program"
        );
        assert_eq!(found(&path(&[&empty]), none), None);
        assert_eq!(find_node(None, none, None), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
