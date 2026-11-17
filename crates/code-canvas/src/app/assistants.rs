use std::time::{Duration, Instant};

use super::*;
use crate::agents::{Agent, Doing};
use crate::ui::{AgentMark, AgentOp, AgentView};

const LOOKED_FOR: Duration = Duration::from_secs(5);

impl App {
    fn agent_file(&self, agent: &Agent) -> Option<(usize, String)> {
        let (index, rel) = self.locate_file(agent.path.as_deref()?)?;
        let rel = rel.trim_end_matches('/').to_string();
        let r = self.pages[index].ready()?;
        (rel.is_empty() || r.layout.node_for_path(&r.repo, &rel).is_some()).then_some((index, rel))
    }

    fn agent_line(&self, index: usize, rel: &str, wanted: &str) -> Option<u32> {
        let page = &self.pages[index];
        if let Some(editor) = page.editors.get(rel) {
            return editor.display_source().lines().position(|line| line.contains(wanted)).map(|line| line as u32);
        }
        if let Some(shown) = page.diffs.get(rel) {
            return shown.plain.lines().position(|line| line.contains(wanted)).map(|line| line as u32);
        }
        let r = page.ready()?;
        let file = &r.repo.files[r.repo.file_index(rel)?];
        file.note.is_none().then(|| file.text().lines().position(|line| line.contains(wanted)))?.map(|line| line as u32)
    }

    pub(super) fn poll_agents(&mut self) -> bool {
        let mut changed = self.agents.poll();
        for at in 0..self.agents.list.len() {
            let agent = &self.agents.list[at];
            let (Some(wanted), None, true) = (&agent.find, agent.lines, agent.at.elapsed() < LOOKED_FOR) else {
                continue;
            };
            let Some((index, rel)) = self.agent_file(agent) else { continue };
            if let Some(line) = self.agent_line(index, &rel, wanted) {
                self.agents.list[at].lines = Some((line, 1));
                changed = true;
            }
        }
        let followed = self.agents.following.as_ref().and_then(|key| self.agents.find(key)).cloned();
        if let Some(agent) = followed {
            let now = (agent.moved, agent.lines.map(|lines| lines.0));
            if self.agent_followed != Some(now) {
                if let Some((index, rel)) = self.agent_file(&agent).filter(|(_, rel)| !rel.is_empty()) {
                    let is_file = self.pages[index].ready().is_some_and(|r| r.repo.file_index(&rel).is_some());
                    if is_file {
                        if index != self.active {
                            self.switch_to(index);
                        }
                        self.following = Some((index, rel, now.1.unwrap_or(0), Instant::now()));
                        self.agent_followed = Some(now);
                    }
                }
            }
        }
        if changed {
            self.request_redraw();
        }
        self.agents.lively()
    }

    pub(super) fn agent_op(&mut self, op: AgentOp) {
        match op {
            AgentOp::Follow(key) => {
                self.share_let_go();
                self.agents.following = key;
                self.agent_followed = None;
            }
            AgentOp::ConnectClaude(on) => match crate::agents::connect_claude(on) {
                Ok(()) if on => {
                    let text = "Claude Code now shows on the canvas: every session, and each of its agents, where it reads and edits.";
                    self.ext.notice(ext::Level::Info, "Assistants", text);
                }
                Ok(()) => self.ext.notice(
                    ext::Level::Info,
                    "Assistants",
                    "Claude Code no longer tells the canvas what it does.",
                ),
                Err(why) => self.ext.notice(ext::Level::Error, "Assistants", why),
            },
        }
        self.claude_checked = None;
        self.request_redraw();
    }

    fn agent_said(&self, agent: &Agent) -> String {
        let named =
            agent.path.as_ref().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned());
        let mut said = agent.doing.said().to_string();
        if let (Some(name), true) = (named, matches!(agent.doing, Doing::Reading | Doing::Editing | Doing::Searching)) {
            said = format!("{said} {name}");
        }
        if !agent.note.is_empty() {
            said = format!("{said} · {}", agent.note);
        }
        said
    }

    pub(super) fn agents_view(&mut self) {
        if self.ui.settings.open && self.claude_checked.is_none_or(|at| at.elapsed() > Duration::from_secs(2)) {
            self.claude_checked = Some(Instant::now());
            self.ui.claude_connected = crate::agents::claude_connected();
        }
        let (s, vp) = (self.scale(), self.viewport());
        let shown = self.shown();
        let (mut views, mut marks) = (Vec::new(), Vec::new());
        for agent in &self.agents.list {
            let said = self.agent_said(agent);
            views.push(AgentView {
                key: agent.key.clone(),
                name: agent.name(),
                role: agent.role.clone(),
                said: said.clone(),
                color: agent.color(),
                avatar: agent.avatar,
                quiet: agent.quiet(),
                followed: self.agents.following.as_deref() == Some(agent.key.as_str()),
            });
            let Some((index, rel)) = self.agent_file(agent).filter(|(index, _)| shown.contains(index)) else {
                continue;
            };
            let page = &self.pages[index];
            let Some(r) = page.ready() else { continue };
            let Some(node) = (if rel.is_empty() { Some(0) } else { r.layout.node_for_path(&r.repo, &rel) }) else {
                continue;
            };
            let node = &r.layout.nodes[node];
            let view = page.camera.view(vp);
            let on_screen = |rect: canvas_core::Rect| {
                let (a, b) = (view.to_screen([rect.x, rect.y]), view.to_screen([rect.right(), rect.bottom()]));
                egui::Rect::from_min_max(egui::pos2(a[0] / s, a[1] / s), egui::pos2(b[0] / s, b[1] / s))
            };
            let file = r.repo.file_index(&rel);
            let lines: Vec<egui::Rect> = match (file, agent.lines) {
                (Some(file), Some((first, count))) => {
                    let all = r.repo.files[file].line_count() as u32;
                    let first = page.row_of(&rel, first).min(all.saturating_sub(1));
                    node.line_rects(first..(first + count).min(all)).into_iter().map(on_screen).collect()
                }
                _ => Vec::new(),
            };
            let card = on_screen(node.rect);
            let at = lines.first().map_or(card.left_top(), |line| egui::pos2(card.left(), line.top()));
            let beside = marks.iter().filter(|mark: &&AgentMark| (mark.at - at).length() < 4.0).count() as f32;
            marks.push(AgentMark {
                avatar: agent.avatar,
                color: agent.color(),
                name: agent.name(),
                said,
                at: at + egui::vec2(0.0, beside * 30.0),
                lines: if agent.quiet() { Vec::new() } else { lines },
                busy: agent.busy(),
                quiet: agent.quiet(),
            });
        }
        self.ui.agents = views;
        self.ui.agent_marks = marks;
    }
}
