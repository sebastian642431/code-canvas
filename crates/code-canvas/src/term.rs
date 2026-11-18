use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const KEPT: usize = 5000;
pub const GIT_PAGE: &str = "https://git-scm.com/downloads";

pub fn bash() -> Option<PathBuf> {
    let name = if cfg!(windows) { "bash.exe" } else { "bash" };
    let mut places: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            let Some(base) = std::env::var_os(base).map(PathBuf::from) else { continue };
            places.push(base.join("Git").join("bin").join(name));
            places.push(base.join("Programs").join("Git").join("bin").join(name));
        }
    } else {
        places.extend(["/bin", "/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"].map(|dir| Path::new(dir).join(name)));
    }
    let not_a_shell = |dir: &Path| {
        let dir = dir.to_string_lossy().to_lowercase();
        dir.contains("\\windows\\system32") || dir.contains("\\windowsapps")
    };
    if let Some(path) = std::env::var_os("PATH") {
        places.extend(std::env::split_paths(&path).filter(|dir| !not_a_shell(dir)).map(|dir| dir.join(name)));
    }
    places.into_iter().find(|place| place.is_file())
}

fn whole(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap_or(NonZeroU16::MIN)
}

pub struct Terminal {
    parser: vt100::Parser,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    heard: Receiver<Vec<u8>>,
    size: (u16, u16),
    pub ended: bool,
}

impl Terminal {
    pub fn open(shell: &Path, dir: &Path, rows: u16, cols: u16) -> Result<Terminal, String> {
        let (rows, cols) = (rows.max(2), cols.max(8));
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;
        let mut command = CommandBuilder::new(shell);
        command.args(["--login", "-i"]);
        if dir.is_dir() {
            command.cwd(dir);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("CHERE_INVOKING", "1");
        let child = pair.slave.spawn_command(command).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let (said, heard) = mpsc::channel();
        let read = move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 || said.send(chunk[..n].to_vec()).is_err() {
                    break;
                }
                crate::ext::wake_ui();
            }
            crate::ext::wake_ui();
        };
        std::thread::Builder::new().name("console".into()).spawn(read).map_err(|e| e.to_string())?;
        log::info!("console: {} in {}", shell.display(), dir.display());
        Ok(Terminal {
            parser: vt100::Parser::new(whole(rows), whole(cols), KEPT),
            writer,
            master: pair.master,
            child,
            heard,
            size: (rows, cols),
            ended: false,
        })
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.heard.try_recv() {
            self.parser.process(&bytes);
            changed = true;
            let asks = |what: &[u8]| bytes.windows(what.len()).any(|part| part == what);
            if asks(b"\x1b[6n") {
                let (row, col) = self.parser.screen().cursor_position();
                self.answer(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            if asks(b"\x1b[c") || asks(b"\x1b[0c") {
                self.answer(b"\x1b[?1;2c");
            }
        }
        if !self.ended && matches!(self.child.try_wait(), Ok(Some(_))) {
            self.ended = true;
            changed = true;
        }
        changed
    }

    fn answer(&mut self, bytes: &[u8]) {
        if self.writer.write_all(bytes).and_then(|_| self.writer.flush()).is_err() {
            self.ended = true;
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        if self.ended || bytes.is_empty() {
            return;
        }
        self.parser.screen_mut().set_scrollback(0);
        self.answer(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(8));
        if (rows, cols) == self.size {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.parser.set_size(whole(rows), whole(cols));
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn scroll(&mut self, rows: isize) {
        let now = self.parser.screen().scrollback() as isize;
        self.parser.screen_mut().set_scrollback((now + rows).max(0) as usize);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Default)]
pub struct Console {
    pub term: Option<Terminal>,
    pub shell: Option<PathBuf>,
    pub looked: bool,
    pub failed: Option<String>,
    pub focus: bool,
    pub wheel: f32,
    installing: Option<Receiver<Result<(), String>>>,
}

impl Console {
    pub fn look(&mut self) {
        self.shell = bash();
        self.looked = true;
    }

    pub fn start(&mut self, dir: &Path) {
        if !self.looked || self.shell.is_none() {
            self.look();
        }
        let Some(shell) = self.shell.clone() else { return };
        let (rows, cols) = self.term.as_ref().map_or((24, 80), Terminal::size);
        self.term = None;
        match Terminal::open(&shell, dir, rows, cols) {
            Ok(term) => {
                self.term = Some(term);
                self.failed = None;
                self.focus = true;
            }
            Err(why) => {
                log::warn!("console: {} did not start: {why}", shell.display());
                self.failed = Some(why);
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

    pub fn poll(&mut self) -> Option<Result<(), String>> {
        if let Some(term) = self.term.as_mut() {
            term.poll();
        }
        let done = self.installing.as_ref().and_then(|heard| heard.try_recv().ok())?;
        self.installing = None;
        self.look();
        Some(match (&done, &self.shell) {
            (Ok(()), None) => {
                Err("Git was installed but bash is not where it is looked for: start the app again.".into())
            }
            _ => done,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_runs_and_what_it_prints_is_on_the_screen() {
        let Some(shell) = bash() else { return };
        let dir = std::env::temp_dir();
        let mut term = Terminal::open(&shell, &dir, 24, 80).expect("the shell starts");
        let started = std::time::Instant::now();
        let seen = |term: &mut Terminal, wanted: &str| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while std::time::Instant::now() < until {
                term.poll();
                if term.screen().contents().contains(wanted) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            false
        };
        term.send(b"echo can$((20+22))vas\r");
        assert!(seen(&mut term, "can42vas"), "{:?} after {:?}", term.screen().contents(), started.elapsed());
        term.resize(30, 100);
        assert_eq!(term.size(), (30, 100));
        term.send(b"exit\r");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !term.ended && std::time::Instant::now() < until {
            term.poll();
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        assert!(term.ended, "the shell ends when told to");
    }
}
