use std::time::{Duration, Instant};

use super::*;
use crate::agents::{Agent, Doing, Step};
use crate::ui::{AgentMark, AgentOp, AgentTrail, AgentView, TrailStep};

const LOOKED_FOR: Duration = Duration::from_secs(5);
const CONNECT: &str = "Connect";
pub(super) const CLAUDE_OFFER: &str = "claude on the canvas";

struct Place {
    card: egui::Rect,
    lines: Vec<egui::Rect>,
    line_h: f32,
}

fn step_said(step: &Step) -> String {
    let name = step.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let did = match step.doing {
        Doing::Editing => "edited",
        Doing::Searching => "searched",
        _ => "read",
    };
    let mut said = format!("{did} {name}");
    match step.lines {
        Some((first, 1)) => said = format!("{said}, line {}", first + 1),
        Some((first, count)) => said = format!("{said}, lines {}–{}", first + 1, first + count),
        None => {}
    }
    match (step.note.is_empty(), step.doing) {
        (true, _) => {}
        (false, Doing::Searching) => said = format!("{said} {}", step.note),
        (false, _) => said = format!("{said} · {}", step.note),
    }
    said
}

impl App {
    fn agent_file(&self, agent: &Agent) -> Option<(usize, String)> {
        self.file_on_canvas(agent.path.as_deref()?)
    }

    fn file_on_canvas(&self, path: &Path) -> Option<(usize, String)> {
        let key = |path: &Path| ext::path_key(path.to_string_lossy().trim_end_matches(['/', '\\']));
        let wanted = key(path);
        if let Some(index) = self.pages.iter().position(|page| page.ready().is_some() && key(&page.root) == wanted) {
            return Some((index, String::new()));
        }
        let (index, rel) = self.locate_file(path)?;
        let rel = rel.trim_end_matches('/').to_string();
        let r = self.pages[index].ready()?;
        (rel.is_empty() || r.layout.node_for_path(&r.repo, &rel).is_some()).then_some((index, rel))
    }

    fn bring_into_view(&mut self, path: &Path) {
        let Some((index, rel)) = self.locate_file(path) else { return };
        let filter = &mut self.pages[index].filter;
        let mut changed = false;
        let mut at = String::new();
        for part in rel.split('/').filter(|part| !part.is_empty()) {
            if !at.is_empty() {
                at.push('/');
            }
            at.push_str(part);
            for key in [at.clone(), format!("{at}/")] {
                changed |= filter.hidden.remove(&key);
                changed |= filter.collapsed.remove(&key);
            }
        }
        if changed {
            log::info!("{rel} was folded or hidden on the canvas: it is shown to follow an assistant there");
            self.switch_to(index);
            self.relayout();
        }
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

    pub(super) fn offer_claude(&mut self) {
        let quiet = std::env::var_os("CODE_CANVAS_SCRIPT").is_some() || std::env::var_os("CODE_CANVAS_BENCH").is_some();
        if quiet || !crate::agents::claude_there() {
            return;
        }
        let own = crate::agents::own_program();
        let built_here = own.as_deref().is_some_and(|own| own.replace('\\', "/").contains("/target/"));
        match crate::agents::claude_hook_program() {
            Some(theirs) => {
                let moved = own.as_deref().is_some_and(|own| ext::path_key(own) != ext::path_key(&theirs));
                if moved && !built_here {
                    match crate::agents::connect_claude(true) {
                        Ok(()) => log::info!("the hooks of Claude Code pointed at {theirs}: they now run this program"),
                        Err(why) => log::warn!("the hooks of Claude Code still point at {theirs}: {why}"),
                    }
                }
            }
            None if self.ui.prefs.asked_claude || built_here => {}
            None => {
                self.ui.prefs.asked_claude = true;
                crate::prefs::save(&self.ui.prefs);
                let text = "Claude Code can show on the canvas while it works: each session and each of its agents with a face of its own, where it reads and what it changes.";
                self.ext.ask("Assistants", text, &[CONNECT, "Not now"], CLAUDE_OFFER);
            }
        }
    }

    pub(super) fn claude_answered(&mut self, choice: &str) {
        if choice == CONNECT {
            self.agent_op(AgentOp::ConnectClaude(true));
        }
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
                self.agents.list[at].found(line);
                changed = true;
            }
        }
        let followed = self.agents.following.as_ref().and_then(|key| self.agents.find(key)).cloned();
        let busy = self.drag.is_some() || self.pages.get(self.active).is_some_and(|page| page.editing.is_some());
        if let Some(agent) = followed.filter(|_| !busy) {
            let now = (agent.moved, agent.lines.map(|lines| lines.0));
            let looking = agent.find.is_some() && agent.lines.is_none() && agent.at.elapsed() < LOOKED_FOR;
            if self.agent_followed != Some(now) && !looking {
                match self.agent_file(&agent).filter(|(_, rel)| !rel.is_empty()) {
                    Some((index, rel)) => {
                        let is_file = self.pages[index].ready().is_some_and(|r| r.repo.file_index(&rel).is_some());
                        if is_file {
                            if index != self.active {
                                self.switch_to(index);
                            }
                            self.following = Some((index, rel, now.1.unwrap_or(0), Instant::now()));
                            self.agent_followed = Some(now);
                        }
                    }
                    None => {
                        if let Some(path) = agent.path.clone().filter(|path| path.is_file()) {
                            self.agent_followed = Some(now);
                            self.bring_into_view(&path);
                            self.show_path(&path, now.1.map(|line| (line, 0, 1)));
                        }
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

    fn place(&self, (index, rel): (usize, &str), lines: Option<(u32, u32)>) -> Option<Place> {
        let (s, vp) = (self.scale(), self.viewport());
        let page = &self.pages[index];
        let r = page.ready()?;
        let node = if rel.is_empty() { Some(0) } else { r.layout.node_for_path(&r.repo, rel) }?;
        let node = &r.layout.nodes[node];
        let view = page.camera.view(vp);
        let on_screen = |rect: canvas_core::Rect| {
            let (a, b) = (view.to_screen([rect.x, rect.y]), view.to_screen([rect.right(), rect.bottom()]));
            egui::Rect::from_min_max(egui::pos2(a[0] / s, a[1] / s), egui::pos2(b[0] / s, b[1] / s))
        };
        let (mut shown, mut line_h) = (Vec::new(), 0.0);
        if let (Some(file), Some((first, count))) = (r.repo.file_index(rel), lines) {
            let all = r.repo.files[file].line_count() as u32;
            let first = page.row_of(rel, first).min(all.saturating_sub(1));
            shown = node.line_rects(first..first.saturating_add(count).min(all)).into_iter().map(on_screen).collect();
            let one = node.line_rects(first..first.saturating_add(1).min(all));
            line_h = one.first().map_or(0.0, |line| on_screen(*line).height());
        }
        Some(Place { card: on_screen(node.rect), lines: shown, line_h })
    }

    fn trails_view(&self, shown: &[usize]) -> Vec<AgentTrail> {
        if !self.ui.prefs.trails {
            return Vec::new();
        }
        let mut files: std::collections::HashMap<&Path, Option<(usize, String)>> = Default::default();
        let mut trails = Vec::new();
        for agent in self.agents.list.iter().filter(|agent| !agent.trail.is_empty()) {
            let mut steps = Vec::new();
            for step in &agent.trail {
                let file = files
                    .entry(step.path.as_path())
                    .or_insert_with(|| self.file_on_canvas(&step.path).filter(|(index, _)| shown.contains(index)));
                let Some((index, rel)) = file.as_ref() else { continue };
                let Some(place) = self.place((*index, rel), step.lines) else { continue };
                let at = match place.lines.first() {
                    Some(line) => egui::pos2(line.left() - 7.0, line.top() + place.line_h * 0.5),
                    None => place.card.left_top() + egui::vec2(9.0, 9.0),
                };
                steps.push(TrailStep {
                    at,
                    line_h: place.line_h,
                    lines: place.lines,
                    edited: step.doing == Doing::Editing,
                    did: step_said(step),
                    age: step.at.elapsed(),
                    left: step.left(),
                });
            }
            if !steps.is_empty() {
                let name = match self.share_owner(agent) {
                    Some((owner, _)) => format!("{} ({owner})", agent.name()),
                    None => agent.name(),
                };
                trails.push(AgentTrail { color: agent.color(), name, steps });
            }
        }
        trails
    }

    pub(super) fn agents_view(&mut self) {
        if self.ui.settings.open && self.claude_checked.is_none_or(|at| at.elapsed() > Duration::from_secs(2)) {
            self.claude_checked = Some(Instant::now());
            self.ui.claude_connected = crate::agents::claude_connected();
        }
        let shown = self.shown();
        let (mut views, mut marks) = (Vec::new(), Vec::new());
        for agent in &self.agents.list {
            let said = self.agent_said(agent);
            let owner = self.share_owner(agent);
            views.push(AgentView {
                key: agent.key.clone(),
                name: agent.name(),
                role: agent.role.clone(),
                said: said.clone(),
                color: agent.color(),
                avatar: agent.avatar,
                quiet: agent.quiet(),
                followed: self.agents.following.as_deref() == Some(agent.key.as_str()),
                owner: owner.clone(),
            });
            let Some((index, rel)) = self.agent_file(agent).filter(|(index, _)| shown.contains(index)) else {
                continue;
            };
            let Some(place) = self.place((index, &rel), agent.lines) else { continue };
            let (card, lines) = (place.card, place.lines);
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
                owner,
            });
        }
        self.ui.agent_trails = self.trails_view(&shown);
        self.ui.agents = views;
        self.ui.agent_marks = marks;
    }
}
