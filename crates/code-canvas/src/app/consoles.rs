use super::*;
use crate::consoles::{MenuKind, MenuPage, MAIN};
use crate::ui::ConsoleOp;

fn card_panel(card: u64) -> Panel {
    match card {
        MAIN => Panel::Console,
        card => Panel::Consoles(card),
    }
}

impl App {
    /// The folder a new console opens in: the one asked for, else the active folder's, else home.
    fn console_dir(&self, dir: Option<PathBuf>) -> PathBuf {
        let folder = self.pages.get(self.active).map(|page| page.root.clone());
        dir.or(folder).or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir)
    }

    fn console_profile(&mut self, profile: Option<String>) -> String {
        if !self.ui.console.looked {
            self.ui.console.look();
        }
        let picked = self.ui.console.default_profile(&self.ui.prefs.console_profile).map(|profile| profile.id.clone());
        profile.or(picked).unwrap_or_else(|| "bash".into())
    }

    fn console_tab(&self, tab: Option<u64>) -> Option<u64> {
        tab.or(self.ui.console.current).filter(|id| self.ui.console.tab(*id).is_some())
    }

    /// A new console that is shown, with its shell started; its number.
    pub(super) fn new_console(&mut self, card: Option<u64>, profile: Option<String>, dir: Option<PathBuf>) -> u64 {
        let profile = self.console_profile(profile);
        let card = card.or_else(|| self.ui.console.current.and_then(|id| self.ui.console.tab(id)).map(|tab| tab.card));
        let card = card.unwrap_or(MAIN);
        let dir = self.console_dir(dir);
        let id = self.ui.console.open(card, profile, dir);
        self.ui.console.start(id);
        self.ui.show(card_panel(card));
        id
    }

    /// The console the keyboard goes to, shown, with a shell; a new one when there is none.
    pub(super) fn console_at_hand(&mut self) -> u64 {
        let id = match self.console_tab(None) {
            Some(id) => id,
            None => self.new_console(Some(MAIN), None, None),
        };
        let card = self.ui.console.tab(id).map_or(MAIN, |tab| tab.card);
        self.ui.show(card_panel(card));
        if self.ui.console.tab(id).is_some_and(|tab| tab.term.is_none()) {
            self.ui.console.start(id);
        }
        id
    }

    fn console_type(&mut self, text: &str, run: bool) {
        let id = self.console_at_hand();
        self.ui.console.bring(id);
        if let Some(term) = self.ui.console.tab_mut(id).and_then(|tab| tab.term.as_mut()) {
            let mut bytes = text.replace("\r\n", "\n").replace('\n', "\r").into_bytes();
            if run {
                bytes.push(b'\r');
            }
            term.send(&bytes);
        }
    }

    fn console_kill(&mut self, id: u64) {
        let Some(tab) = self.ui.console.tab(id) else { return };
        if !tab.busy() {
            return self.console(ConsoleOp::End(id));
        }
        let title = tab.title();
        let text = match tab.term.as_ref().and_then(|term| term.command()) {
            Some(command) => format!("{command} is still running in it. Ending it stops that and what it started."),
            None => "Something may still be running in it. Ending it stops that and what it started.".to_string(),
        };
        let ask = ui::Ask::new(ui::Asked::EndConsole { tab: id }, &format!("End {title}?"), text)
            .choice("End", ui::ChoiceLook::Danger, ui::Answer::Yes)
            .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
        self.ui.ask = Some(ask);
    }

    /// Closes the cards that were left with no console.
    pub(super) fn console_cards(&mut self) {
        for card in std::mem::take(&mut self.ui.console.emptied) {
            let panel = card_panel(card);
            if self.ui.console.of_card(card).next().is_none() && self.ui.is_open(&panel) {
                self.ui.close_panel(&panel);
                if card != MAIN {
                    self.ui.rails.cards.remove(&panel.key());
                }
            }
        }
        if std::mem::take(&mut self.ui.console.changed) {
            self.session_dirty = true;
        }
    }

    pub(super) fn console(&mut self, op: ConsoleOp) {
        match op {
            ConsoleOp::Install => self.ui.console.install(),
            ConsoleOp::Page => {
                if let Err(why) = crate::files::open_web_link(crate::term::GIT_PAGE) {
                    self.ext.notice(ext::Level::Warn, "Console", why);
                }
            }
            ConsoleOp::New { card, profile, dir } => {
                self.new_console(card, profile, dir);
            }
            ConsoleOp::Split { tab, profile } => {
                let Some(of) = self.console_tab(tab) else {
                    self.new_console(None, profile, None);
                    return self.request_redraw();
                };
                let profile = profile.or_else(|| self.ui.console.tab(of).map(|tab| tab.profile.clone()));
                let profile = self.console_profile(profile);
                let dir = self.ui.console.tab(of).map(|tab| tab.cwd()).filter(|dir| dir.is_dir());
                let dir = self.console_dir(dir);
                if let Some(id) = self.ui.console.split(of, profile, dir) {
                    self.ui.console.start(id);
                    let card = self.ui.console.tab(id).map_or(MAIN, |tab| tab.card);
                    self.ui.show(card_panel(card));
                }
            }
            ConsoleOp::Start(id) => self.ui.console.start(id),
            ConsoleOp::Kill(tab) => {
                if let Some(id) = self.console_tab(tab) {
                    self.console_kill(id);
                }
            }
            ConsoleOp::End(id) => {
                if let Some(tab) = self.ui.console.remove(id) {
                    log::info!("console: {} ended", tab.title());
                }
                self.console_cards();
            }
            ConsoleOp::Unsplit(tab) => {
                if let Some(id) = self.console_tab(tab) {
                    self.ui.console.unsplit(id);
                }
            }
            ConsoleOp::ToCard(tab) => {
                if let Some(card) = self.console_tab(tab).and_then(|id| self.ui.console.move_out(id)) {
                    self.ui.show(Panel::Consoles(card));
                    self.console_cards();
                }
            }
            ConsoleOp::Front(id) => self.ui.console.bring(id),
            ConsoleOp::Cycle(step) => {
                if self.ui.console.current.is_none() {
                    self.console_at_hand();
                }
                self.ui.console.cycle(step);
                let card = self.ui.console.current.and_then(|id| self.ui.console.tab(id)).map_or(MAIN, |tab| tab.card);
                self.ui.show(card_panel(card));
            }
            ConsoleOp::Renaming(tab) => {
                if let Some(id) = self.console_tab(tab) {
                    let title = self.ui.console.tab(id).map(|tab| tab.title()).unwrap_or_default();
                    self.ui.console.renaming = Some((id, title));
                    self.ui.console.rename_focused = false;
                    let card = self.ui.console.tab(id).map_or(MAIN, |tab| tab.card);
                    self.ui.show(card_panel(card));
                }
            }
            ConsoleOp::Rename(id, name) => self.ui.console.rename(id, name),
            ConsoleOp::Colour(id, color) => {
                if let Some(tab) = self.ui.console.tab_mut(id) {
                    tab.color = color;
                }
                self.ui.console.changed = true;
            }
            ConsoleOp::Icon(id, icon) => {
                if let Some(tab) = self.ui.console.tab_mut(id) {
                    tab.icon = icon;
                }
                self.ui.console.changed = true;
            }
            ConsoleOp::Clear(tab) => {
                let Some(id) = self.console_tab(tab) else { return };
                let Some(tab) = self.ui.console.tab(id) else { return };
                let idle = !tab.busy() || tab.term.as_ref().is_some_and(|term| !term.shell().integrated);
                let redraw = match (idle, self.ui.console.profile(&tab.profile)) {
                    (true, Some(profile)) => profile.redraw(),
                    _ => b"",
                };
                if let Some(term) = self.ui.console.tab_mut(id).and_then(|tab| tab.term.as_mut()) {
                    term.clear(redraw);
                }
            }
            ConsoleOp::ToCommand(up) => {
                if let Some(term) = self.ui.console.current_mut().and_then(|tab| tab.term.as_mut()) {
                    term.scroll_to_command(up);
                }
            }
            ConsoleOp::RunSelection => {
                let text =
                    self.pages.get(self.active).and_then(|page| page.editor()).map(|editor| editor.selected_text());
                match text.filter(|text| !text.trim().is_empty()) {
                    Some(text) => self.console_type(text.trim_end(), true),
                    None => self.ext.notice(ext::Level::Info, "Console", "Select some text in a file first."),
                }
            }
            ConsoleOp::RunFile => self.console_run_file(),
            ConsoleOp::Type { text, run } => self.console_type(&text, run),
            ConsoleOp::Cd(dir) => {
                let id = self.console_at_hand();
                let profile = self.ui.console.tab(id).and_then(|tab| self.ui.console.profile(&tab.profile)).cloned();
                if let Some(profile) = profile {
                    self.console_type(&profile.cd(&dir), true);
                }
            }
            ConsoleOp::Default(profile) => {
                let prefs = crate::prefs::Prefs { console_profile: profile, ..self.ui.prefs.clone() };
                self.apply(Action::SetPrefs(prefs));
            }
            ConsoleOp::Fill(card) => {
                let card = card
                    .or_else(|| self.ui.console.current.and_then(|id| self.ui.console.tab(id)).map(|tab| tab.card))
                    .unwrap_or(MAIN);
                self.ui.console.filled = match self.ui.console.filled == Some(card) {
                    true => None,
                    false => Some(card),
                };
                self.ui.show(card_panel(card));
            }
            ConsoleOp::Profiles => {
                let id = self.console_at_hand();
                let card = self.ui.console.tab(id).map_or(MAIN, |tab| tab.card);
                let at = self
                    .ui
                    .console
                    .heads
                    .get(&card)
                    .map_or(egui::pos2(400.0, 120.0), |head| egui::pos2(head.right() - 260.0, head.bottom() + 4.0));
                self.ui.console.menu =
                    Some(crate::consoles::Menu { card, kind: MenuKind::New, page: MenuPage::Default, at, place: None });
            }
        }
        self.console_cards();
        self.request_redraw();
    }

    fn console_run_file(&mut self) {
        let Some(page) = self.pages.get(self.active) else { return };
        let Some(rel) = page.editing.clone() else {
            return self.ext.notice(ext::Level::Info, "Console", "Open a file to edit first: that is the one run.");
        };
        let file = page.root.join(&rel);
        let id = self.console_at_hand();
        let Some(tab) = self.ui.console.tab(id) else { return };
        let cwd = tab.cwd();
        let shown = file.strip_prefix(&cwd).map(Path::to_path_buf).unwrap_or_else(|_| file.clone());
        let command = self.ui.console.profile(&tab.profile).and_then(|profile| profile.run_file(&file, &shown));
        match command {
            Some(command) => self.console_type(&command, true),
            None => {
                let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                let text = format!("{name} is not a kind of file the console knows how to run.");
                self.ext.notice(ext::Level::Info, "Console", text);
            }
        }
    }
}
