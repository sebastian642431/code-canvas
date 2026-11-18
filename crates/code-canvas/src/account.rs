use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use egui::Rect;

const SIDE: u32 = 96;

#[derive(Default)]
pub struct Account {
    pub user: Option<String>,
    pub face: Option<egui::TextureHandle>,
    pub signing: bool,
    pub code: Option<(String, String)>,
    pub said: String,
    pub synced: String,
    pub menu: bool,
    pub button: Option<Rect>,
    face_for: Option<String>,
    rx: Option<Receiver<egui::ColorImage>>,
}

fn picture(bytes: &[u8]) -> Option<egui::ColorImage> {
    let face = image::load_from_memory(bytes).ok()?.thumbnail(SIDE, SIDE).to_rgba8();
    let size = [face.width() as usize, face.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, face.as_raw()))
}

pub fn fits(user: &str) -> bool {
    !user.is_empty() && user.len() <= 39 && user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
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
        if self.user != self.face_for {
            self.face_for = self.user.clone();
            self.face = None;
            self.rx = None;
            if let Some(user) = self.user.clone().filter(|user| fits(user)) {
                let (tx, rx) = mpsc::channel();
                self.rx = Some(rx);
                std::thread::spawn(move || {
                    if let Some(face) = face_of(&user) {
                        let _ = tx.send(face);
                        crate::ext::wake_ui();
                    }
                });
            }
        }
        if let Some(face) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.face = Some(ctx.load_texture("github face", face, egui::TextureOptions::LINEAR));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_github_login_is_looked_up() {
        assert!(fits("octocat42") && fits("a-b"));
        assert!(!fits("") && !fits("a b") && !fits("../x") && !fits(&"a".repeat(40)));
    }
}
