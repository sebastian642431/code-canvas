use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::github::{self, Files, Gist, Refused, Waited};
use crate::sync::{self, Plan, State};
use crate::ui::AccountOp;

const LOOK_EVERY: Duration = Duration::from_secs(4);

enum Heard {
    Code(github::Code),
    SignedIn(String, String),
    SignInFailed(String),
    Remote(Result<Option<Gist>, Refused>),
    Sent(Result<Gist, Refused>, String),
    Described(Vec<sync::Looked>, Vec<String>),
}

pub(super) struct GitHub {
    asks: std::collections::VecDeque<ui::Ask>,
    describing: bool,
    offer: Vec<(String, String)>,
    enable: Vec<String>,
    held_commit: Option<String>,
    tx: Sender<Heard>,
    rx: Receiver<Heard>,
    started: bool,
    token: Option<String>,
    state: State,
    busy: bool,
    looked: Option<Instant>,
    waiting: Option<Arc<AtomicBool>>,
    pending: Option<Gist>,
}

impl Default for GitHub {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        GitHub {
            asks: Default::default(),
            describing: false,
            offer: Vec::new(),
            enable: Vec::new(),
            held_commit: None,
            tx,
            rx,
            started: false,
            token: None,
            state: State::default(),
            busy: false,
            looked: None,
            waiting: None,
            pending: None,
        }
    }
}

fn send(tx: &Sender<Heard>, heard: Heard) {
    let _ = tx.send(heard);
    crate::ext::wake_ui();
}

impl App {
    fn snapshot(&self) -> Files {
        let mut installed: Vec<String> = self.ext.installed.iter().map(|e| e.id.clone()).collect();
        for id in &self.github.state.declined {
            if !installed.contains(id) {
                installed.push(id.clone());
            }
        }
        let disabled = self.ext.installed.iter().filter(|e| !e.enabled).map(|e| e.id.clone()).collect();
        sync::snapshot(installed, disabled)
    }

    pub(super) fn account_op(&mut self, op: AccountOp) {
        match op {
            AccountOp::SignIn => self.sign_in(),
            AccountOp::Cancel => {
                if let Some(stop) = self.github.waiting.take() {
                    stop.store(true, Ordering::Relaxed);
                }
                (self.ui.account.signing, self.ui.account.code) = (false, None);
            }
            AccountOp::CopyAndOpen => {
                if let Some((code, page)) = self.ui.account.code.clone() {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(code));
                    if let Err(why) = crate::files::open_web_link(&page) {
                        self.ui.account.said = why;
                    }
                }
            }
            AccountOp::SyncNow => self.look_on_github(),
            AccountOp::SignOut => {
                sync::forget();
                self.github.token = None;
                self.github.state = State::default();
                self.ui.account.user = None;
                self.ui.account.synced.clear();
                self.ui.account.menu = false;
            }
        }
        self.request_redraw();
    }

    fn sign_in(&mut self) {
        self.ui.account.signing = true;
        self.ui.account.code = None;
        self.ui.account.said.clear();
        if github::client_id().is_empty() {
            self.ui.account.said = "This version of Code Canvas has no GitHub app to sign in with yet.".into();
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.github.waiting = Some(stop.clone());
        let tx = self.github.tx.clone();
        std::thread::spawn(move || {
            let code = match github::ask_code() {
                Ok(code) => code,
                Err(why) => return send(&tx, Heard::SignInFailed(why)),
            };
            send(&tx, Heard::Code(code.clone()));
            let (began, mut every) = (Instant::now(), code.every);
            while began.elapsed() < code.lasts {
                std::thread::sleep(every);
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                match github::ask_token(&code) {
                    Ok(Waited::Pending) => {}
                    Ok(Waited::Slower) => every += Duration::from_secs(5),
                    Ok(Waited::Token(token)) => {
                        return match github::who(&token) {
                            Ok(user) => send(&tx, Heard::SignedIn(token, user)),
                            Err(why) => send(&tx, Heard::SignInFailed(why.to_string())),
                        };
                    }
                    Err(why) => return send(&tx, Heard::SignInFailed(why)),
                }
            }
            send(&tx, Heard::SignInFailed("the code ran out before it was entered".into()));
        });
    }

    fn look_on_github(&mut self) {
        let Some(token) = self.github.token.clone().filter(|_| !self.github.busy) else { return };
        self.github.busy = true;
        let (tx, id) = (self.github.tx.clone(), self.github.state.gist.clone());
        std::thread::spawn(move || {
            let found = match id.is_empty() {
                true => github::find_gist(&token),
                false => match github::read_gist(&token, &id) {
                    Ok(None) => github::find_gist(&token),
                    other => other,
                },
            };
            send(&tx, Heard::Remote(found));
        });
    }

    fn send_to_github(&mut self, files: Files) {
        let Some(token) = self.github.token.clone() else { return };
        self.github.busy = true;
        let tx = self.github.tx.clone();
        let id = Some(self.github.state.gist.clone()).filter(|id| !id.is_empty());
        let sent = sync::digest(&files);
        std::thread::spawn(move || send(&tx, Heard::Sent(github::write_gist(&token, id.as_deref(), &files), sent)));
    }

    pub(super) fn take_from_github(&mut self, gist: Gist) {
        let command = sync::command_change(&self.ui.commit_ai, gist.files.get("commit.json").map(String::as_str));
        let held: &[&str] = if command.is_some() { &["commit.json"] } else { &[] };
        let wanted = sync::write_down(&gist.files, held);
        self.apply(Action::SetPrefs(crate::prefs::load()));
        let mut theme = crate::themes::load();
        theme.outside = outside_theme(&self.ext, &theme.theme);
        self.ui.theme = theme;
        self.theme_changed();
        self.ui.key_file = crate::keys::load();
        self.ui.keys = Keymap::new(&self.ui.key_file);
        self.ui.commit_ai = crate::commit_ai::load();
        self.github.state.declined.retain(|id| wanted.installed.contains(id));
        let here: Vec<(String, bool)> = self.ext.installed.iter().map(|e| (e.id.clone(), e.enabled)).collect();
        let changes = sync::changes(&wanted, &here, &self.github.state.declined);
        for id in &changes.disable {
            self.ext.set_enabled(id, false);
        }
        if let (Some(command), Some(text)) = (command, gist.files.get("commit.json")) {
            log::info!("the command for commit messages from GitHub waits for an answer");
            self.github.held_commit = Some(text.clone());
            let ask = ui::Ask::new(
                ui::Asked::SyncCommand,
                "Run this command for commit messages?",
                "The settings kept on GitHub set a command of their own to write commit messages. It would run on this computer each time one is written. Use it only if you set it yourself.".to_string(),
            )
            .choice("Keep mine", ui::ChoiceLook::Main, ui::Answer::Other)
            .choice("Use it", ui::ChoiceLook::Danger, ui::Answer::Yes);
            self.github.asks.push_back(ui::Ask { items: vec![command], ..ask });
        }
        if !changes.install.is_empty() || !changes.enable.is_empty() {
            log::info!(
                "GitHub lists extensions to install {:?} and to turn on {:?}: asking",
                changes.install,
                changes.enable
            );
            self.github.describing = true;
            let (tx, install, enable) = (self.github.tx.clone(), changes.install, changes.enable);
            std::thread::spawn(move || {
                let found = install
                    .into_iter()
                    .map(|id| {
                        let (namespace, name) = id.split_once('.').unwrap_or_default();
                        let about = ext::market::details(namespace, name).map(|d| (d.item.version, d.item.verified));
                        (id, about)
                    })
                    .collect();
                send(&tx, Heard::Described(found, enable));
            });
        }
        self.github.state.gist = gist.id;
        self.github.state.synced = sync::digest(&gist.files);
        sync::save_state(&self.github.state);
        self.ui.account.synced = "Brought from GitHub".into();
        self.ext.notice(ext::Level::Info, "GitHub", "The settings kept on GitHub are on this computer now.");
    }

    pub(super) fn sync_install_answered(&mut self, answer: ui::Answer) {
        let offer = std::mem::take(&mut self.github.offer);
        let enable = std::mem::take(&mut self.github.enable);
        if answer != ui::Answer::Yes {
            log::info!("the extensions GitHub lists are not installed: {offer:?}");
            self.github.state.declined.extend(offer.into_iter().map(|(id, _)| id));
            sync::save_state(&self.github.state);
            return;
        }
        for (id, version) in offer {
            if let Some((namespace, name)) = id.split_once('.') {
                log::info!("installing {id}@{version}, as GitHub lists it");
                self.ext.install_exact(namespace, name, Some(version));
            }
        }
        for id in enable {
            self.ext.set_enabled(&id, true);
        }
    }

    pub(super) fn sync_command_answered(&mut self, answer: ui::Answer) {
        let Some(text) = self.github.held_commit.take() else { return };
        if answer != ui::Answer::Yes {
            log::info!("the command for commit messages from GitHub is not used");
            return;
        }
        if let Err(why) = std::fs::write(crate::session::data_dir().join("commit.json"), text) {
            log::warn!("commit.json from GitHub could not be written: {why}");
        }
        self.ui.commit_ai = crate::commit_ai::load();
    }

    fn deciding(&self) -> bool {
        let shown = self.ui.ask.as_ref().map(|ask| &ask.about);
        self.github.describing
            || !self.github.asks.is_empty()
            || matches!(shown, Some(ui::Asked::SyncInstall | ui::Asked::SyncCommand))
    }

    pub(super) fn sync_answered(&mut self, take: bool) {
        let Some(gist) = self.github.pending.take() else { return };
        match take {
            true => self.take_from_github(gist),
            false => {
                self.github.state.gist = gist.id;
                let here = self.snapshot();
                self.send_to_github(here);
            }
        }
    }

    pub(super) fn poll_github(&mut self) -> bool {
        if !std::mem::replace(&mut self.github.started, true) {
            self.github.token = sync::token();
            self.github.state = sync::load_state();
            if self.github.token.is_some() {
                self.ui.account.user = Some(self.github.state.user.clone()).filter(|user| !user.is_empty());
                self.look_on_github();
            }
        }
        let mut changed = false;
        while let Ok(heard) = self.github.rx.try_recv() {
            changed = true;
            match heard {
                Heard::Code(code) => {
                    self.ui.account.code = Some((code.user, code.page));
                    if std::env::var_os("CODE_CANVAS_SCRIPT").is_none() {
                        self.account_op(AccountOp::CopyAndOpen);
                    }
                }
                Heard::SignInFailed(why) => {
                    self.github.waiting = None;
                    self.ui.account.code = None;
                    self.ui.account.said = format!("Could not sign in: {why}");
                }
                Heard::SignedIn(token, user) => {
                    self.github.waiting = None;
                    sync::keep_token(&token);
                    self.github.token = Some(token);
                    self.github.state = State { user: user.clone(), ..State::default() };
                    sync::save_state(&self.github.state);
                    self.ui.account.user = Some(user);
                    (self.ui.account.signing, self.ui.account.code) = (false, None);
                    self.look_on_github();
                }
                Heard::Remote(found) => {
                    self.github.busy = false;
                    match found {
                        Ok(remote) => self.settle(remote),
                        Err(Refused::NotSignedIn) => {
                            self.account_op(AccountOp::SignOut);
                            let text =
                                "GitHub no longer takes this sign-in: sign in again to keep the settings in sync.";
                            self.ext.notice(ext::Level::Warn, "GitHub", text);
                        }
                        Err(why) => self.ui.account.synced = format!("Could not reach GitHub: {why}"),
                    }
                }
                Heard::Sent(sent, digest) => {
                    self.github.busy = false;
                    match sent {
                        Ok(gist) => {
                            self.github.state.gist = gist.id;
                            self.github.state.synced = digest;
                            sync::save_state(&self.github.state);
                            self.ui.account.synced = "Kept on GitHub".into();
                        }
                        Err(why) => self.ui.account.synced = format!("Could not send to GitHub: {why}"),
                    }
                }
                Heard::Described(found, enable) => {
                    self.github.describing = false;
                    let lines = sync::offer_lines(&found, &enable);
                    self.github.offer = found
                        .into_iter()
                        .filter_map(|(id, about)| about.ok().map(|(version, _)| (id, version)))
                        .collect();
                    self.github.enable = enable;
                    let ask = ui::Ask::new(
                        ui::Asked::SyncInstall,
                        "Install the extensions kept on GitHub?",
                        "The settings kept on GitHub list extensions this computer does not have or has turned off. An extension runs code with your permissions: say yes only if you added these yourself.".to_string(),
                    )
                    .choice("Not now", ui::ChoiceLook::Main, ui::Answer::Other)
                    .choice("Install them", ui::ChoiceLook::Danger, ui::Answer::Yes);
                    self.github.asks.push_back(ui::Ask { items: lines, ..ask });
                }
            }
        }
        if self.ui.ask.is_none() {
            if let Some(ask) = self.github.asks.pop_front() {
                log::info!("asking: {} ({})", ask.title, ask.items.join(" | "));
                self.ui.ask = Some(ask);
                changed = true;
            }
        }
        let due = self.github.looked.is_none_or(|at| at.elapsed() > LOOK_EVERY);
        let ready =
            self.github.token.is_some() && !self.github.busy && self.github.pending.is_none() && !self.deciding();
        if due && ready && self.ext.installing.is_empty() && !self.github.state.synced.is_empty() {
            self.github.looked = Some(Instant::now());
            let here = self.snapshot();
            if sync::digest(&here) != self.github.state.synced {
                self.send_to_github(here);
            }
        }
        changed || self.github.busy
    }

    fn settle(&mut self, remote: Option<Gist>) {
        let here = self.snapshot();
        let last = self.github.state.synced.clone();
        match sync::plan(&here, remote.as_ref().map(|gist| &gist.files), &last) {
            Plan::Same => {
                if let Some(gist) = remote {
                    self.github.state.gist = gist.id;
                }
                self.github.state.synced = sync::digest(&here);
                sync::save_state(&self.github.state);
                self.ui.account.synced = "In sync with GitHub".into();
            }
            Plan::Send => {
                if let Some(gist) = remote {
                    self.github.state.gist = gist.id;
                }
                self.send_to_github(here);
            }
            Plan::Take => {
                if let Some(gist) = remote {
                    self.take_from_github(gist);
                }
            }
            Plan::Ask => {
                self.github.pending = remote;
                let text = "Both this computer and GitHub have settings and extensions of their own. Which ones do you want here?".to_string();
                let ask = ui::Ask::new(ui::Asked::Sync, "Settings on GitHub", text)
                    .choice("Bring GitHub's", ui::ChoiceLook::Main, ui::Answer::Yes)
                    .choice("Keep this computer's", ui::ChoiceLook::Plain, ui::Answer::Other);
                self.ui.ask = Some(ui::Ask { danger: false, ..ask });
            }
        }
    }
}
