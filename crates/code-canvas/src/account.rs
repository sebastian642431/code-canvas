use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const SIDE: u32 = 96;

enum Found {
    User(Option<String>),
    Face(egui::ColorImage),
}

#[derive(Default)]
pub struct Account {
    asked: bool,
    rx: Option<Receiver<Found>>,
    pub user: Option<String>,
    pub face: Option<egui::TextureHandle>,
    pub known: bool,
}

pub fn user_in(said: &str) -> Option<String> {
    let user = said.lines().find_map(|line| line.strip_prefix("username="))?.trim();
    let fits = !user.is_empty() && user.len() <= 39 && user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    fits.then(|| user.to_string())
}

fn git_user() -> Option<String> {
    let mut command = Command::new("git");
    command
        .args(["credential", "fill"])
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    child.stdin.take()?.write_all(b"protocol=https\nhost=github.com\n\n").ok()?;
    let started = Instant::now();
    while child.try_wait().ok()?.is_none() {
        if started.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let said = child.wait_with_output().ok()?.stdout;
    user_in(&String::from_utf8_lossy(&said))
}

fn picture(bytes: &[u8]) -> Option<egui::ColorImage> {
    let face = image::load_from_memory(bytes).ok()?.thumbnail(SIDE, SIDE).to_rgba8();
    let size = [face.width() as usize, face.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, face.as_raw()))
}

fn face_of(user: &str) -> Option<egui::ColorImage> {
    let kept = crate::session::data_dir().join(format!("github-{user}.png"));
    if let Some(face) = std::fs::read(&kept).ok().and_then(|bytes| picture(&bytes)) {
        return Some(face);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let url = format!("https://github.com/{user}.png?size={SIDE}");
    let mut answer =
        agent.get(&url).header("User-Agent", concat!("code-canvas/", env!("CARGO_PKG_VERSION"))).call().ok()?;
    let bytes = answer.body_mut().read_to_vec().ok()?;
    let face = picture(&bytes)?;
    let _ = std::fs::write(kept, &bytes);
    Some(face)
}

impl Account {
    pub fn poll(&mut self, ctx: &egui::Context) {
        if !std::mem::replace(&mut self.asked, true) {
            let (tx, rx) = mpsc::channel();
            self.rx = Some(rx);
            std::thread::spawn(move || {
                let user = git_user();
                let _ = tx.send(Found::User(user.clone()));
                crate::ext::wake_ui();
                if let Some(face) = user.as_deref().and_then(face_of) {
                    let _ = tx.send(Found::Face(face));
                    crate::ext::wake_ui();
                }
            });
        }
        let Some(rx) = &self.rx else { return };
        while let Ok(found) = rx.try_recv() {
            match found {
                Found::User(user) => (self.user, self.known) = (user, true),
                Found::Face(face) => {
                    self.face = Some(ctx.load_texture("github face", face, egui::TextureOptions::LINEAR))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_user_is_taken_from_what_git_answers() {
        let said = "protocol=https\nhost=github.com\nusername=sebastian642431\npassword=gho_secret\n";
        assert_eq!(user_in(said).as_deref(), Some("sebastian642431"));
        assert_eq!(user_in("protocol=https\nhost=github.com\n"), None);
        assert_eq!(user_in("username=a b/../c\n"), None, "nothing that is not a GitHub login");
        assert_eq!(user_in("username=\n"), None);
    }
}
