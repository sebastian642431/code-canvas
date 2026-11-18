use super::*;
use crate::consoles::{Menu, MenuKind, MenuPage};
use crate::ui::ConsoleOp;

impl App {
    /// The `term` step of a script: the mouse and the copy key on the console's screen, aimed at text shown on it, the
    /// consoles' own commands and menus, and reports of what they show.
    pub(super) fn term_step(&mut self, what: &str) {
        let mut words = what.split_whitespace();
        let verb = words.next().unwrap_or_default();
        let args: Vec<&str> = words.collect();
        let nth = |n: Option<&&str>| n.and_then(|n| n.parse::<usize>().ok()).and_then(|n| n.checked_sub(1));
        let tab_at = |app: &App, n: Option<&&str>| nth(n).and_then(|n| app.ui.console.tabs.get(n)).map(|tab| tab.id);
        let profile = |rest: &[&str]| rest.first().map(|p| p.to_string());
        let op = match (verb, args.as_slice()) {
            ("report", _) => return self.term_report(),
            ("copy", _) => return self.ui_events.push_back(vec![egui::Event::Copy]),
            ("new", rest) => Some(ConsoleOp::New { card: None, profile: profile(rest), dir: None }),
            ("split", rest) => Some(ConsoleOp::Split { tab: None, profile: profile(rest) }),
            ("next", _) => Some(ConsoleOp::Cycle(1)),
            ("prev", _) => Some(ConsoleOp::Cycle(-1)),
            ("front", [n]) => tab_at(self, Some(n)).map(ConsoleOp::Front),
            ("kill", rest) => Some(ConsoleOp::Kill(tab_at(self, rest.first()))),
            ("rename", words) => self.ui.console.current.map(|id| ConsoleOp::Rename(id, Some(words.join(" ")))),
            ("tocard", _) => Some(ConsoleOp::ToCard(None)),
            ("unsplit", _) => Some(ConsoleOp::Unsplit(None)),
            ("colour", [n]) => self.ui.console.current.map(|id| ConsoleOp::Colour(id, n.parse().ok())),
            ("icon", [name]) => self.ui.console.current.map(|id| ConsoleOp::Icon(id, Some(name.to_string()))),
            ("clear", _) => Some(ConsoleOp::Clear(None)),
            ("fill", _) => Some(ConsoleOp::Fill(None)),
            ("up", _) => Some(ConsoleOp::ToCommand(true)),
            ("down", _) => Some(ConsoleOp::ToCommand(false)),
            ("default", [profile]) => Some(ConsoleOp::Default(profile.to_string())),
            ("has", [n, text]) => {
                let tab = nth(Some(n)).and_then(|n| self.ui.console.tabs.get(n));
                let term = tab.and_then(|tab| tab.term.as_ref());
                let said = match term.is_some_and(|term| term.screen().contents().contains(text)) {
                    true => "shows",
                    false => "does not show",
                };
                return log::info!("script: console tab {n} {said} '{text}'");
            }
            ("menu", [kind, rest @ ..]) => return self.term_menu(kind, rest.first().copied()),
            ("page", [page]) => {
                let page = match *page {
                    "split" => MenuPage::SplitWith,
                    "default" => MenuPage::Default,
                    "folders" => MenuPage::Folders,
                    "colour" => MenuPage::Colour,
                    "icon" => MenuPage::Icon,
                    "dirs" => MenuPage::Dirs,
                    "commands" => MenuPage::Commands,
                    _ => MenuPage::Main,
                };
                if let Some(menu) = self.ui.console.menu.as_mut() {
                    (menu.page, menu.place) = (page, None);
                }
                return self.request_redraw();
            }
            ("drag" | "click" | "right", _) => None,
            _ => {
                return log::warn!(
                    "script: term needs drag, click, right, copy, report, new, split, next, prev, front, kill, rename, \
                     tocard, unsplit, colour, icon, clear, fill, up, down, default, has, menu or page"
                )
            }
        };
        if let Some(op) = op {
            return self.console(op);
        }
        let Some(tab) = self.ui.console.current.and_then(|id| self.ui.console.tab(id)) else {
            return log::warn!("script: term {what}: there is no console");
        };
        let (Some(term), Some((origin, cell))) = (tab.term.as_ref(), tab.mouse.place) else {
            return log::warn!("script: term {what}: the console is not on screen");
        };
        let (rows, cols) = term.size();
        // The row and column where a text starts, the lowest row it is on: output comes after the line typed.
        let find = |text: &str| {
            let rows = term.screen().rows(0, cols).enumerate();
            let found = rows.filter_map(|(row, line)| line.find(text).map(|i| (row as u16, line[..i].chars().count())));
            let found = found.last();
            if found.is_none() {
                log::warn!("script: term {what}: '{text}' is not on the console's screen");
            }
            found
        };
        let point = |row: u16, col: f32| origin + egui::vec2(col * cell.x, (f32::from(row) + 0.5) * cell.y);
        let press = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (left, right) = (egui::PointerButton::Primary, egui::PointerButton::Secondary);
        let frames: Vec<Vec<egui::Event>> = match (verb, args.as_slice()) {
            ("drag", [from, to]) => {
                let (Some(start), Some(end)) = (find(from), find(to)) else { return };
                let from = point(start.0, start.1 as f32 + 0.2);
                let to = point(end.0, (end.1 + to.chars().count()) as f32 - 0.2);
                let mut frames = vec![vec![egui::Event::PointerMoved(from)], vec![press(from, left, true)]];
                frames.extend((1..=8).map(|i| vec![egui::Event::PointerMoved(from.lerp(to, i as f32 / 8.0))]));
                frames.push(vec![press(to, left, false)]);
                frames
            }
            ("click", [text, times @ ..]) => {
                let Some((row, col)) = find(text) else { return };
                let times = times.first().and_then(|n| n.parse().ok()).unwrap_or(1usize);
                let at = point(row, col as f32 + 0.5);
                let mut frames = vec![vec![egui::Event::PointerMoved(at)]];
                for _ in 0..times {
                    frames.extend([vec![press(at, left, true)], vec![press(at, left, false)]]);
                }
                frames
            }
            ("right", rest) if rest.len() <= 1 => {
                let at = match rest.first() {
                    Some(text) => {
                        let Some((row, col)) = find(text) else { return };
                        point(row, col as f32 + 0.5)
                    }
                    None => point(rows / 2, f32::from(cols) / 2.0),
                };
                let moved = vec![egui::Event::PointerMoved(at)];
                vec![moved, vec![press(at, right, true)], vec![press(at, right, false)]]
            }
            _ => {
                log::warn!("script: term needs drag <text> <text>, click <text> [times] or right [text]");
                Vec::new()
            }
        };
        self.ui_events.extend(frames);
    }

    fn term_menu(&mut self, kind: &str, n: Option<&str>) {
        let kind = match kind {
            "close" => {
                self.ui.console.menu = None;
                return self.request_redraw();
            }
            "new" => MenuKind::New,
            "more" => MenuKind::More,
            "tab" => {
                let n = n.and_then(|n| n.parse::<usize>().ok()).and_then(|n| n.checked_sub(1));
                match n.and_then(|n| self.ui.console.tabs.get(n)) {
                    Some(tab) => MenuKind::Tab(tab.id),
                    None => return log::warn!("script: term menu tab needs the number of a console"),
                }
            }
            _ => return log::warn!("script: term menu needs new, more, tab <n> or close"),
        };
        let card = match kind {
            MenuKind::Tab(id) => self.ui.console.tab(id).map(|tab| tab.card),
            _ => self.ui.console.current.and_then(|id| self.ui.console.tab(id)).map(|tab| tab.card),
        };
        let card = card.unwrap_or(crate::consoles::MAIN);
        let head = self.ui.console.heads.get(&card).copied();
        let at = match kind {
            MenuKind::Tab(_) => head.map(|head| egui::pos2(head.right() - 420.0, head.bottom() + 40.0)),
            _ => head.map(|head| egui::pos2(head.right() - 280.0, head.bottom() + 4.0)),
        };
        let at = at.unwrap_or(egui::pos2(400.0, 120.0));
        self.ui.console.menu = Some(Menu { card, kind, page: MenuPage::Main, at, place: None });
        self.request_redraw();
    }

    fn term_report(&mut self) {
        let current = self.ui.console.current;
        for (n, tab) in self.ui.console.tabs.iter().enumerate() {
            let (n, title, card, group) = (n + 1, tab.title(), tab.card, tab.group);
            let (busy, news, broke, ended) = (tab.busy(), tab.news, tab.broke, tab.ended());
            let marks = tab.term.as_ref().map(|term| term.shell().marks.clone()).unwrap_or_default();
            let failed = marks.iter().filter(|mark| mark.failed()).count();
            let here = if current == Some(tab.id) { " (current)" } else { "" };
            log::info!(
                "script: console tab {n}{here} \"{title}\" card {card} group {group} busy {busy} news {news} broke \
                 {broke} ended {ended} commands {} failed {failed} cwd {}",
                marks.len(),
                tab.cwd().display()
            );
        }
        let Some(term) = self.ui.console.current_mut().and_then(|tab| tab.term.as_mut()) else {
            return log::warn!("script: term report: the console has no shell");
        };
        let selected = term.selected_text();
        let (row, _) = term.screen().cursor_position();
        let cols = term.size().1;
        let line = term.screen().rows(0, cols).nth(usize::from(row)).unwrap_or_default();
        log::info!("script: console selection {selected:?}");
        log::info!("script: console clipboard {:?}", crate::term::clipboard().unwrap_or_default());
        log::info!("script: console line at the cursor {:?}", line.trim_end());
    }
}
