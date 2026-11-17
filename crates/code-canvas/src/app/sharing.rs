use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;
use crate::share::ot::TextOp;
use crate::share::{self, Caret, Event, Invite, Seen, Session};
use crate::ui::{PeerMark, PersonView, ShareMode, ShareOp};

const MOST_FILES: usize = 20_000;
const SEEN_EVERY: Duration = Duration::from_millis(70);
const LOOKED_OVER_EVERY: Duration = Duration::from_secs(2);

pub(super) struct Sharing {
    pub session: Option<Session>,
    dir: Option<PathBuf>,
    waiting: HashMap<u32, PathBuf>,
    pub touched: Vec<(u32, String)>,
    revs: HashMap<(u32, String), u64>,
    edited: HashSet<(u32, String)>,
    refused: HashSet<u32>,
    too_much: HashSet<(u32, String)>,
    settle: Vec<(u32, String)>,
    made: HashMap<(u32, String), u64>,
    mirrored: HashMap<(u32, String), u64>,
    _held: Option<std::fs::File>,
    said: Option<Seen>,
    said_at: Instant,
    looked_over: Instant,
    told: Option<Instant>,
    pub following: Option<u32>,
    followed: Option<Seen>,
}

impl Default for Sharing {
    fn default() -> Self {
        Sharing {
            session: None,
            dir: None,
            waiting: HashMap::new(),
            touched: Vec::new(),
            revs: HashMap::new(),
            edited: HashSet::new(),
            refused: HashSet::new(),
            too_much: HashSet::new(),
            settle: Vec::new(),
            made: HashMap::new(),
            mirrored: HashMap::new(),
            _held: None,
            said: None,
            said_at: Instant::now(),
            looked_over: Instant::now(),
            told: None,
            following: None,
            followed: None,
        }
    }
}

const HELD: &str = ".held";

fn hold(dir: &std::path::Path) -> Option<std::fs::File> {
    std::fs::create_dir_all(dir).ok()?;
    let mut how = std::fs::OpenOptions::new();
    how.write(true).create(true).truncate(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        how.share_mode(0);
    }
    how.open(dir.join(HELD)).ok()
}

fn in_use(dir: &std::path::Path) -> bool {
    let held = dir.join(HELD);
    if cfg!(windows) {
        return matches!(std::fs::remove_file(&held), Err(e) if e.kind() != std::io::ErrorKind::NotFound);
    }
    let age = std::fs::metadata(&held).and_then(|file| file.modified()).ok().and_then(|at| at.elapsed().ok());
    age.is_some_and(|age| age < Duration::from_secs(86_400))
}

fn mark_of(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn own_name() -> String {
    let named = ["USERNAME", "USER"].iter().find_map(|key| std::env::var(key).ok());
    named.filter(|name| !name.trim().is_empty()).unwrap_or_else(|| "Someone".to_string())
}

impl App {
    fn share_name(&self) -> String {
        match self.ui.share.name.trim() {
            "" => own_name(),
            name => name.to_string(),
        }
    }

    pub(super) fn share_setup(&mut self) {
        self.ui.share.name = match self.ui.prefs.share_name.trim() {
            "" => own_name(),
            name => name.to_string(),
        };
        self.ui.share.guests_write = self.ui.prefs.share_writers;
        let shared = session::data_dir().join("shared");
        for left in std::fs::read_dir(&shared).into_iter().flatten().flatten() {
            if !in_use(&left.path()) {
                let _ = std::fs::remove_dir_all(left.path());
            }
        }
        self.workspace.retain(|known| !known.path.starts_with(&shared));
    }

    fn share_folder_of(&self, page: &Page) -> Option<u32> {
        let session = self.sharing.session.as_ref()?;
        match session.host {
            true => session.shares(page.id).then_some(page.id),
            false => page.shared,
        }
    }

    fn share_page(&self, folder: u32) -> Option<usize> {
        let host = self.sharing.session.as_ref()?.host;
        self.pages.iter().position(|page| if host { page.id == folder } else { page.shared == Some(folder) })
    }

    pub(super) fn share_op(&mut self, op: ShareOp) {
        match op {
            ShareOp::Offer => self.share_ask(false),
            ShareOp::Enter => self.share_ask(true),
            ShareOp::Start => self.share_start(),
            ShareOp::Stop => self.share_stop(None),
            ShareOp::Join => {
                let typed = self.ui.share.typed.clone();
                self.share_join(&typed);
            }
            ShareOp::SetWriter(id, writer) => {
                if let Some(session) = self.sharing.session.as_mut().filter(|session| session.host) {
                    session.set_writer(id, writer);
                }
            }
            ShareOp::SendAway(id) => {
                if let Some(session) = self.sharing.session.as_mut().filter(|session| session.host) {
                    session.send_away(id);
                }
            }
            ShareOp::Follow(who) => {
                self.share_let_go();
                self.sharing.following = who;
            }
            ShareOp::Panel => self.ui.share.open = true,
            ShareOp::CallEveryone => {
                if let Some(session) = self.sharing.session.as_mut() {
                    session.call_everyone();
                    self.ext.notice(
                        ext::Level::Info,
                        "Sharing",
                        "Everyone now follows what you look at, until they move.",
                    );
                }
            }
            ShareOp::CopyInvite => {
                let invite = self.ui.share.invite.clone();
                let copied = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(invite));
                match copied {
                    Ok(()) => self.ext.notice(
                        ext::Level::Info,
                        "Sharing",
                        "The invitation is copied: send it to who should join.",
                    ),
                    Err(e) => self.ext.notice(ext::Level::Warn, "Sharing", format!("It could not be copied: {e}")),
                }
            }
            ShareOp::Changed => {
                let (name, writers) = (self.ui.share.name.trim().to_string(), self.ui.share.guests_write);
                if (name.as_str(), writers) != (self.ui.prefs.share_name.as_str(), self.ui.prefs.share_writers) {
                    self.ui.prefs.share_name = name;
                    self.ui.prefs.share_writers = writers;
                    crate::prefs::save(&self.ui.prefs);
                }
                let address = self.ui.share.address.trim().to_string();
                if let Some(session) = self.sharing.session.as_mut().filter(|session| session.host) {
                    session.guests_write = writers;
                    if let Some(invite) = session.invite.as_mut() {
                        invite.host = if address.is_empty() { share::local_address() } else { address };
                    }
                }
            }
        }
        self.request_redraw();
    }

    fn share_ask(&mut self, join: bool) {
        if self.sharing.session.is_some() {
            return;
        }
        if join && Invite::read(&self.ui.share.typed).is_none() {
            let text = "That is not an invitation: it starts with cc1: and is copied whole from who shares.";
            return self.ext.notice(ext::Level::Warn, "Sharing", text);
        }
        let (title, text, yes) = match join {
            false => (
                "Share this session?",
                "Whoever has the invitation opens your folders on their computer. Someone you let edit changes your files, and so do the extensions and assistants they use. Share it only with someone you trust.",
                "Share",
            ),
            true => (
                "Join this session?",
                "Its folders are copied to this computer, and your extensions and assistants work on them as on your own: that can run what those files say. Join only a session of someone you trust.",
                "Join",
            ),
        };
        let ask = ui::Ask::new(ui::Asked::Share { join }, title, text.to_string())
            .choice(yes, ui::ChoiceLook::Main, ui::Answer::Yes)
            .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
        self.ui.share.open = false;
        self.ui.ask = Some(ask);
    }

    fn share_start(&mut self) {
        if self.sharing.session.is_some() {
            return;
        }
        match Session::open(&self.share_name(), self.ui.share.guests_write) {
            Ok(mut session) => {
                let address = self.ui.share.address.trim();
                if let (Some(invite), false) = (session.invite.as_mut(), address.is_empty()) {
                    invite.host = address.to_string();
                }
                if std::env::var_os("CODE_CANVAS_SCRIPT").is_some() {
                    let written = session.invite.as_ref().map(Invite::written).unwrap_or_default();
                    let _ = std::fs::write(session::data_dir().join("invite.txt"), written);
                }
                self.sharing = Sharing { session: Some(session), ..Default::default() };
            }
            Err(e) => self.ext.notice(ext::Level::Error, "Sharing", format!("The session could not be opened: {e}")),
        }
    }

    fn share_join(&mut self, typed: &str) {
        if self.sharing.session.is_some() {
            return;
        }
        let Some(invite) = Invite::read(typed) else {
            let text = "That is not an invitation: it starts with cc1: and is copied whole from who shares.";
            return self.ext.notice(ext::Level::Warn, "Sharing", text);
        };
        let dir = session::data_dir().join("shared").join(format!("{:x}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let held = hold(&dir);
        self.sharing = Sharing {
            session: Some(Session::join(&invite, &self.share_name())),
            dir: Some(dir),
            _held: held,
            ..Default::default()
        };
        self.ui.share.typed.clear();
    }

    pub(super) fn share_stop(&mut self, why: Option<String>) {
        let Some(mut session) = self.sharing.session.take() else { return };
        let (host, connected) = (session.host, session.connected);
        session.close(if host { "the one sharing ended the session" } else { "left" });
        drop(session);
        let dir = self.sharing.dir.take();
        self.sharing = Sharing::default();
        if !host {
            for i in (0..self.pages.len()).rev() {
                if self.pages[i].shared.is_some() {
                    crate::page::forget_scan(&self.pages[i].root);
                    self.close_page(i);
                }
            }
            if let Some(dir) = dir {
                self.workspace.retain(|known| !known.path.starts_with(&dir));
                let _ = std::fs::remove_dir_all(dir);
                self.save_session();
            }
        }
        match why {
            Some(why) if connected => {
                self.ext.notice(ext::Level::Info, "Sharing", format!("The session is over: {why}."))
            }
            Some(why) => {
                let why = match why.as_str() {
                    "the connection was closed" => "the invitation is not of a session that is open now",
                    other => other,
                };
                self.ext.notice(ext::Level::Warn, "Sharing", format!("It did not connect: {why}."))
            }
            None => {}
        }
        self.request_redraw();
    }

    fn share_files(&mut self, index: usize, kept: usize) -> Option<Vec<(String, String)>> {
        let page = &self.pages[index];
        let r = page.ready()?;
        let texts = r.repo.files.iter().filter(|file| file.note.is_none());
        if texts.clone().count() > MOST_FILES {
            return None;
        }
        let mut total = kept;
        let mut files = Vec::new();
        for file in texts {
            let text = match page.editors.get(&file.rel_path) {
                Some(editor) => editor.display_source(),
                None => match std::fs::read_to_string(page.root.join(&file.rel_path)) {
                    Ok(text) => share::body(&text),
                    Err(_) => continue,
                },
            };
            if text.len() > share::LARGEST_FILE || total + text.len() > share::MOST_TEXT {
                continue;
            }
            total += text.len();
            files.push((file.rel_path.clone(), text));
        }
        Some(files)
    }

    fn share_announce(&mut self) {
        let Some(session) = self.sharing.session.as_ref().filter(|session| session.host) else { return };
        let open: Vec<u32> = self.pages.iter().filter(|page| !page.only).map(|page| page.id).collect();
        let closed: Vec<u32> = session.folder_ids().into_iter().filter(|id| !open.contains(id)).collect();
        let fresh: Vec<usize> = (0..self.pages.len())
            .filter(|i| {
                let page = &self.pages[*i];
                let fit = page.ready().is_some() && !page.only && page.shared.is_none();
                fit && !session.shares(page.id) && !self.sharing.refused.contains(&page.id)
            })
            .collect();
        for index in fresh {
            let kept = self.sharing.session.as_ref().map_or(0, Session::text_kept);
            let (id, name) = (self.pages[index].id, self.pages[index].title.clone());
            match self.share_files(index, kept) {
                Some(files) => {
                    log::info!("share: {name} is shared, {} files", files.len());
                    if let Some(session) = self.sharing.session.as_mut() {
                        session.share_folder(id, &name, files);
                    }
                }
                None => {
                    self.sharing.refused.insert(id);
                    let text = format!("{name} has too many files to be shared: the others will not see it.");
                    self.ext.notice(ext::Level::Warn, "Sharing", text);
                }
            }
        }
        if let Some(session) = self.sharing.session.as_mut() {
            for id in closed {
                session.unshare_folder(id);
            }
        }
    }

    fn share_look_over(&mut self) {
        if self.sharing.looked_over.elapsed() < LOOKED_OVER_EVERY {
            return;
        }
        self.sharing.looked_over = Instant::now();
        let Some(session) = self.sharing.session.as_ref().filter(|session| session.host || session.writer()) else {
            return;
        };
        let mut found = Vec::new();
        for page in &self.pages {
            let Some(folder) = self.share_folder_of(page) else { continue };
            let Some(r) = page.ready() else { continue };
            let known: HashSet<String> = session.paths(folder).into_iter().collect();
            for file in r.repo.files.iter().filter(|file| file.note.is_none()) {
                let key = (page.id, file.rel_path.clone());
                if !known.contains(&file.rel_path) && !self.sharing.too_much.contains(&key) {
                    found.push(key);
                }
            }
            for path in known {
                if r.repo.file_index(&path).is_none() && !page.root.join(&path).is_file() {
                    found.push((page.id, path));
                }
            }
        }
        self.sharing.touched.extend(found);
    }

    fn share_flush(&mut self) {
        let Some(host) = self.sharing.session.as_ref().map(|session| session.host) else { return };
        let mut changed: Vec<(u32, String, String)> = Vec::new();
        let mut now: HashSet<(u32, String)> = HashSet::new();
        for page in &self.pages {
            let Some(folder) = self.share_folder_of(page) else { continue };
            for (rel, editor) in &page.editors {
                let key = (page.id, rel.clone());
                now.insert(key.clone());
                if self.sharing.revs.get(&key) != Some(&editor.rev) {
                    self.sharing.revs.insert(key, editor.rev);
                    changed.push((folder, rel.clone(), editor.display_source()));
                }
            }
        }
        let closed: Vec<(u32, String)> = self.sharing.edited.difference(&now).cloned().collect();
        self.sharing.touched.extend(closed);
        self.sharing.edited = now;
        let mut removed: Vec<(u32, String)> = Vec::new();
        let mut later: Vec<(u32, String)> = Vec::new();
        let mut written: Vec<(u32, String, String)> = Vec::new();
        let mut made: Vec<(u32, String, String)> = Vec::new();
        let mut back: Vec<(u32, String)> = Vec::new();
        let mut room = share::MOST_TEXT.saturating_sub(self.sharing.session.as_ref().map_or(0, Session::text_kept));
        for (id, rel) in std::mem::take(&mut self.sharing.touched) {
            let Some(page) = self.pages.iter().find(|page| page.id == id) else { continue };
            let Some(folder) = self.share_folder_of(page) else { continue };
            if page.editors.contains_key(&rel) {
                continue;
            }
            let Some(r) = page.ready() else {
                later.push((id, rel));
                continue;
            };
            if !host {
                let Some(session) = self.sharing.session.as_ref() else { continue };
                let writer = session.writer();
                let path = page.root.join(&rel);
                let key = (folder, rel.clone());
                let size = std::fs::metadata(&path).ok().filter(|file| file.is_file()).map(|file| file.len() as usize);
                let text = match size {
                    Some(size) if size <= share::LARGEST_FILE && share::wire::safe_path(&rel) => {
                        std::fs::read_to_string(&path).ok().map(|text| share::body(&text))
                    }
                    Some(_) => None,
                    None => {
                        match session.text(folder, &rel).is_some() && !path.exists() {
                            true if writer => removed.push(key.clone()),
                            true => back.push(key.clone()),
                            false => {}
                        }
                        None
                    }
                };
                let Some(text) = text else { continue };
                let mark = mark_of(&text);
                match session.text(folder, &rel) {
                    Some(known) if known == text || self.sharing.mirrored.get(&key) == Some(&mark) => {}
                    Some(_) if writer => written.push((folder, rel, text)),
                    Some(_) => back.push(key),
                    None if !writer => {}
                    None => {
                        let shown = r.repo.file_index(&rel).is_some_and(|file| r.repo.files[file].note.is_none());
                        if shown && self.sharing.made.get(&key) != Some(&mark) {
                            self.sharing.made.insert(key, mark);
                            made.push((folder, rel, text));
                        }
                    }
                }
                continue;
            }
            self.sharing.revs.remove(&(id, rel.clone()));
            let known = self.sharing.session.as_ref().is_some_and(|session| session.text(folder, &rel).is_some());
            let path = page.root.join(&rel);
            let size = std::fs::metadata(&path).ok().filter(|file| file.is_file()).map(|file| file.len() as usize);
            let on_canvas = r.repo.file_index(&rel).is_some_and(|file| r.repo.files[file].note.is_none());
            let text = match size {
                Some(size) if on_canvas && size <= share::LARGEST_FILE && (known || size <= room) => {
                    std::fs::read_to_string(&path).ok()
                }
                Some(size) if on_canvas && !known && size <= share::LARGEST_FILE => {
                    self.sharing.too_much.insert((id, rel.clone()));
                    None
                }
                _ => None,
            };
            match text {
                Some(text) => {
                    if !known {
                        room = room.saturating_sub(text.len());
                    }
                    changed.push((folder, rel, share::body(&text)));
                }
                None if known => removed.push((folder, rel)),
                None => {}
            }
        }
        self.sharing.touched = later;
        for (folder, rel) in back {
            let kept = self.sharing.session.as_ref().and_then(|session| session.text(folder, &rel));
            if let Some(text) = kept.map(str::to_string) {
                self.share_mirror(folder, &rel, &text, false);
            }
        }
        let Some(session) = self.sharing.session.as_mut() else { return };
        for (folder, rel, text) in changed {
            session.changed(folder, &rel, &text);
        }
        for (folder, rel, text) in written {
            log::info!("share: {rel} changed on this side, the others get it");
            session.changed(folder, &rel, &text);
            session.saved(folder, &rel);
        }
        for (folder, rel, text) in made {
            log::info!("share: {rel} is new on this side, the one sharing gets it");
            session.made(folder, &rel, &text);
        }
        for (folder, rel) in removed {
            match host {
                true => session.removed(folder, &rel),
                false => session.dropped(folder, &rel),
            }
        }
    }

    fn share_mirror(&mut self, folder: u32, rel: &str, text: &str, quiet: bool) {
        let root = match self.share_page(folder) {
            Some(index) => Some(self.pages[index].root.clone()),
            None => self.sharing.waiting.get(&folder).cloned(),
        };
        let Some(path) = root.and_then(|root| crate::files::inside(&root, rel).ok()) else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.sharing.mirrored.insert((folder, rel.to_string()), mark_of(text));
        let whole = format!("{text}\n");
        let written = match quiet {
            true => self.watcher.write(&path, whole),
            false => std::fs::write(&path, whole),
        };
        if let Err(e) = written {
            log::warn!("share: {} could not be written: {e}", path.display());
        }
    }

    fn share_remote_edit(&mut self, folder: u32, rel: &str, op: &TextOp) {
        let Some(index) = self.share_page(folder) else { return };
        let Some(host) = self.sharing.session.as_ref().map(|session| session.host) else { return };
        self.sharing.settle.push((folder, rel.to_string()));
        let (id, path) = (self.pages[index].id, self.pages[index].root.join(rel));
        if !self.pages[index].editors.contains_key(rel) {
            if !host {
                return;
            }
            let Ok(disk) = std::fs::read_to_string(&path) else { return };
            self.pages[index].editors.insert(rel.to_string(), crate::editor::Editor::open(rel.to_string(), &disk));
        }
        let Some(editor) = self.pages[index].editors.get_mut(rel) else { return };
        let lines = editor.apply_remote(op);
        self.sharing.edited.insert((id, rel.to_string()));
        if index == self.active && self.pages[index].editing.as_deref() == Some(rel) {
            self.completion = None;
        }
        let page = &mut self.pages[index];
        page.touch(rel, lines.into_iter().filter(|(_, count)| *count > 0).collect());
    }

    fn share_settle(&mut self) {
        let mut settled: HashSet<(u32, String)> = HashSet::new();
        for (folder, rel) in std::mem::take(&mut self.sharing.settle) {
            if !settled.insert((folder, rel.clone())) {
                continue;
            }
            let Some(index) = self.share_page(folder) else { continue };
            let Some(session) = self.sharing.session.as_ref() else { return };
            let (host, Some(text)) = (session.host, session.text(folder, &rel).map(str::to_string)) else { continue };
            let (id, path) = (self.pages[index].id, self.pages[index].root.join(&rel));
            let Some(editor) = self.pages[index].editors.get_mut(&rel) else {
                if !host {
                    self.share_mirror(folder, &rel, &text, false);
                }
                continue;
            };
            let mine = editor.display_source();
            if mine != text && host {
                log::warn!("share: {rel} was not what the others had: they get it as it is here");
                if let Some(session) = self.sharing.session.as_mut() {
                    session.changed(folder, &rel, &mine);
                }
            } else if mine != text {
                editor.become_text(&text);
            }
            if !host {
                editor.mark_saved();
            }
            self.sharing.revs.insert((id, rel.clone()), editor.rev);
            if !host {
                self.share_mirror(folder, &rel, &text, true);
            }
            let page = &mut self.pages[index];
            if let Some(g) = self.gfx.as_mut() {
                g.renderer.text.drop_page(page.id);
                page.show_edit(&rel, &g.renderer, &g.device, &self.ui.filters);
            }
            page.want_head(&rel);
            page.preview_unsaved();
            self.symbols_stale = Some(Instant::now());
            self.unsent_edit = Some((path, Instant::now()));
            self.request_redraw();
        }
    }

    fn share_event(&mut self, event: Event) {
        let who = |me: &App, id: u32| {
            let person = me.sharing.session.as_ref().and_then(|session| session.person(id));
            person.map_or("Someone".to_string(), |person| person.name.clone())
        };
        match event {
            Event::Joined(id) => {
                let text = format!("{} joined the session.", who(self, id));
                self.ext.notice(ext::Level::Info, "Sharing", text);
            }
            Event::Left(name) => self.ext.notice(ext::Level::Info, "Sharing", format!("{name} left the session.")),
            Event::Folder { id, name } => {
                let Some(dir) = self.sharing.dir.clone() else { return };
                let safe: String =
                    name.chars().map(|c| if c.is_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect();
                let named = !safe.trim_matches(['.', ' ']).is_empty();
                let root =
                    dir.join(format!("{id}")).join(if named { safe.trim().to_string() } else { "shared".to_string() });
                let _ = std::fs::create_dir_all(&root);
                self.sharing.waiting.insert(id, root);
            }
            Event::File { folder, path, text, .. } => {
                if self.sharing.made.remove(&(folder, path.clone())).is_some() {
                    let page =
                        self.share_page(folder).map(|index| (self.pages[index].id, self.pages[index].root.clone()));
                    let here = page.as_ref().and_then(|(_, root)| crate::files::inside(root, &path).ok());
                    let mine = here.and_then(|file| std::fs::read_to_string(file).ok()).map(|mine| share::body(&mine));
                    if let (Some((id, _)), true) = (page, mine.is_some_and(|mine| mine != text)) {
                        self.sharing.touched.push((id, path));
                        return;
                    }
                }
                if let Some(index) = self.share_page(folder) {
                    let id = self.pages[index].id;
                    if let Some(editor) = self.pages[index].editors.get_mut(&path) {
                        editor.become_text(&text);
                        editor.mark_saved();
                        self.sharing.revs.insert((id, path.clone()), editor.rev);
                        if let Some(g) = self.gfx.as_mut() {
                            g.renderer.text.drop_page(id);
                            self.pages[index].show_edit(&path, &g.renderer, &g.device, &self.ui.filters);
                        }
                    }
                }
                self.share_mirror(folder, &path, &text, false);
            }
            Event::FolderReady(id) => {
                let Some(root) = self.sharing.waiting.remove(&id) else { return };
                let writer = self.sharing.session.as_ref().is_some_and(Session::writer);
                self.open_folder(root.clone(), None);
                if let Some(page) = self.pages.iter_mut().find(|page| page.root == root) {
                    page.shared = Some(id);
                    page.read_only = !writer;
                }
                self.workspace.retain(|known| known.path != root);
            }
            Event::FolderGone(id) => {
                if let Some(index) = self.share_page(id) {
                    let root = self.pages[index].root.clone();
                    self.pages[index].shared = None;
                    crate::page::forget_scan(&root);
                    self.close_page(index);
                    self.workspace.retain(|known| known.path != root);
                    let _ = std::fs::remove_dir_all(root);
                }
            }
            Event::Gone { folder, path } => {
                let root = self.share_page(folder).map(|index| self.pages[index].root.clone());
                if let Some(file) = root.and_then(|root| crate::files::inside(&root, &path).ok()) {
                    let _ = std::fs::remove_file(file);
                }
            }
            Event::Edit { folder, path, op, .. } => self.share_remote_edit(folder, &path, &op),
            Event::Save { folder, path, .. } => {
                if let Some(index) = self.share_page(folder) {
                    self.save_file(index, &path, true);
                }
            }
            Event::Make { folder, path, text, by } => {
                let Some(index) = self.share_page(folder) else { return };
                let Ok(file) = crate::files::inside(&self.pages[index].root, &path) else { return };
                let key = (folder, path.clone());
                if file.exists() && !self.sharing.made.contains_key(&key) {
                    log::warn!("share: {path} is here already: what {} made does not replace it", who(self, by));
                    return;
                }
                if let Some(parent) = file.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let whole = if text.is_empty() { String::new() } else { format!("{text}\n") };
                match std::fs::write(&file, whole) {
                    Ok(()) => {
                        log::info!("share: {} made {path}", who(self, by));
                        self.sharing.made.insert(key, 0);
                    }
                    Err(e) => log::warn!("share: {path} could not be made: {e}"),
                }
            }
            Event::Drop { folder, path, by } => {
                let Some(index) = self.share_page(folder) else { return };
                let file = crate::files::inside(&self.pages[index].root, &path);
                match file.and_then(|file| crate::files::recycle(&file)) {
                    Ok(()) => log::info!("share: {} removed {path}", who(self, by)),
                    Err(why) => log::warn!("share: {path} could not be removed: {why}"),
                }
            }
            Event::Role(writer) => {
                for page in self.pages.iter_mut().filter(|page| page.shared.is_some()) {
                    page.read_only = !writer;
                    if !writer {
                        page.editing = None;
                    }
                }
                let text = match writer {
                    true => "You can edit now: take the pencil and type.",
                    false => "You can look, not edit: the one sharing changed it.",
                };
                self.ext.notice(ext::Level::Info, "Sharing", text);
            }
            Event::Come(id) => {
                self.sharing.following = Some(id);
                self.sharing.followed = None;
                let text = format!("{} brought everyone to what they are looking at.", who(self, id));
                self.ext.notice(ext::Level::Info, "Sharing", text);
            }
            Event::Ended(why) => self.share_stop(Some(why)),
        }
    }

    fn share_seen_now(&self) -> Option<Seen> {
        let page = self.pages.get(self.active)?;
        let folder = self.share_folder_of(page)?;
        let r = page.ready()?;
        let view = page.camera.view(self.viewport());
        let round = |v: f32| (v * 2.0).round() / 2.0;
        let place = |world: [f32; 2]| {
            let node = &r.layout.nodes[r.layout.hit(world).unwrap_or(0)];
            (node.rel_path.clone(), [round(world[0] - node.rect.x), round(world[1] - node.rect.y)])
        };
        let (anchor, offset) = place(view.to_world(self.canvas.center()));
        let zoom = page.camera.zoom.max(0.0001);
        let pointer = (self.canvas.contains(self.cursor) && !self.over_ui(self.cursor))
            .then(|| place(view.to_world(self.cursor)));
        let caret = page.editor().map(|editor| {
            let cell = |p: crate::editor::Pos| (p.line as u32, editor.display_col(p) as u32);
            let (line, col) = cell(editor.caret);
            Caret {
                path: editor.rel_path.clone(),
                line,
                col,
                from: editor.anchor.filter(|a| *a != editor.caret).map(cell),
            }
        });
        Some(Seen {
            folder,
            anchor,
            offset,
            view: [round(self.canvas.w / zoom), round(self.canvas.h / zoom)],
            pointer,
            caret,
        })
    }

    fn share_place(&self, index: usize, anchor: &str, offset: [f32; 2]) -> Option<[f32; 2]> {
        let r = self.pages[index].ready()?;
        let node = match anchor {
            "" => 0,
            path => r.layout.node_for_path(&r.repo, path)?,
        };
        let rect = r.layout.nodes.get(node)?.rect;
        Some([rect.x + offset[0], rect.y + offset[1]])
    }

    fn share_follow(&mut self) {
        let Some(who) = self.sharing.following else { return };
        let Some(session) = self.sharing.session.as_ref() else { return };
        if session.person(who).is_none() {
            self.sharing.following = None;
            return;
        }
        let Some(seen) = session.seen.get(&who).cloned() else { return };
        if self.sharing.followed.as_ref() == Some(&seen) {
            return;
        }
        let Some(index) = self.share_page(seen.folder) else { return };
        if index != self.active {
            self.switch_to(index);
        }
        let Some(at) = self.share_place(index, &seen.anchor, seen.offset) else { return };
        let (area, vp) = (self.canvas, self.viewport());
        let zoom = (area.w / seen.view[0].max(1.0)).min(area.h / seen.view[1].max(1.0));
        let zoom = zoom.clamp(crate::camera::MIN_ZOOM, crate::camera::MAX_ZOOM);
        let middle = area.center();
        let center = [at[0] - (middle[0] - vp[0] * 0.5) / zoom, at[1] - (middle[1] - vp[1] * 0.5) / zoom];
        if !(center[0].is_finite() && center[1].is_finite() && zoom.is_finite()) {
            return;
        }
        self.pages[index].target = Some(Camera { center, zoom });
        self.sharing.followed = Some(seen);
        self.request_redraw();
    }

    pub(super) fn share_let_go(&mut self) {
        self.sharing.following = None;
        self.sharing.followed = None;
        self.agents.following = None;
        self.agent_followed = None;
    }

    pub(super) fn poll_share(&mut self) -> bool {
        if self.sharing.session.is_none() {
            return false;
        }
        self.share_announce();
        self.share_look_over();
        self.share_flush();
        let events = self.sharing.session.as_mut().map(Session::poll).unwrap_or_default();
        let changed = !events.is_empty();
        for event in events {
            self.share_event(event);
        }
        self.share_settle();
        if self.sharing.session.is_none() {
            return false;
        }
        let seen = self.share_seen_now().filter(Seen::sane);
        let due = seen.is_some() && seen != self.sharing.said && self.sharing.said_at.elapsed() >= SEEN_EVERY;
        if let (true, Some(seen), Some(session)) = (due, seen, self.sharing.session.as_mut()) {
            session.see(seen.clone());
            self.sharing.said = Some(seen);
            self.sharing.said_at = Instant::now();
        }
        self.share_follow();
        let mut refused = false;
        for page in &mut self.pages {
            refused |= std::mem::take(&mut page.refused_edit);
        }
        if refused && self.sharing.told.is_none_or(|at| at.elapsed() > Duration::from_secs(6)) {
            self.sharing.told = Some(Instant::now());
            let text = "You are looking, not editing: the one who shares can let you edit.";
            self.ext.notice(ext::Level::Info, "Sharing", text);
        }
        if changed {
            self.request_redraw();
        }
        true
    }

    pub(super) fn share_marks(&self) -> Vec<PeerMark> {
        let Some(session) = self.sharing.session.as_ref() else { return Vec::new() };
        let (s, vp) = (self.scale(), self.viewport());
        let shown = self.shown();
        let mut marks = Vec::new();
        for person in session.others() {
            let Some(seen) = session.seen.get(&person.id) else { continue };
            let Some(index) = self.share_page(seen.folder).filter(|index| shown.contains(index)) else { continue };
            let page = &self.pages[index];
            let Some(r) = page.ready() else { continue };
            let view = page.camera.view(vp);
            let point = |world: [f32; 2]| {
                let at = view.to_screen(world);
                egui::pos2(at[0] / s, at[1] / s)
            };
            let looked_at = self.share_place(index, &seen.anchor, seen.offset).map(|middle| {
                let half = [seen.view[0] * 0.5, seen.view[1] * 0.5];
                egui::Rect::from_min_max(
                    point([middle[0] - half[0], middle[1] - half[1]]),
                    point([middle[0] + half[0], middle[1] + half[1]]),
                )
            });
            let pointer =
                seen.pointer.as_ref().and_then(|(anchor, offset)| self.share_place(index, anchor, *offset)).map(point);
            let caret = seen.caret.as_ref().and_then(|caret| {
                let file = r.repo.file_index(&caret.path)?;
                let node = r.layout.file_node.get(file).filter(|&&n| n != canvas_core::layout::NONE)?;
                let node = &r.layout.nodes[*node as usize];
                let cell = |line: u32, col: u32, cols: u32| {
                    let rect = node.cell_rect(page.row_of(&caret.path, line), col, cols);
                    egui::Rect::from_min_max(point([rect.x, rect.y]), point([rect.right(), rect.bottom()]))
                };
                let chosen = match caret.from {
                    Some((line, col)) if line == caret.line && col != caret.col => {
                        Some(cell(line, col.min(caret.col), col.abs_diff(caret.col)))
                    }
                    _ => None,
                };
                Some((cell(caret.line, caret.col, 1), chosen))
            });
            marks.push(PeerMark {
                name: person.name.clone(),
                color: share::COLORS[person.color as usize % share::COLORS.len()],
                looked_at: looked_at.filter(|_| self.sharing.following != Some(person.id)),
                pointer,
                caret: caret.as_ref().map(|(at, _)| *at),
                chosen: caret.and_then(|(_, chosen)| chosen),
            });
        }
        marks
    }

    pub(super) fn share_view(&mut self) {
        let marks = self.share_marks();
        let view = &mut self.ui.share;
        view.marks = marks;
        let Some(session) = self.sharing.session.as_ref() else {
            view.mode = ShareMode::Off;
            view.people.clear();
            view.following = None;
            return;
        };
        view.mode = match (session.host, session.connected) {
            (true, _) => ShareMode::Hosting,
            (false, false) => ShareMode::Joining,
            (false, true) => ShareMode::Joined,
        };
        view.invite = session.invite.as_ref().map(Invite::written).unwrap_or_default();
        view.writer = session.writer();
        view.following = self.sharing.following;
        view.people = session
            .people
            .iter()
            .map(|person| PersonView {
                id: person.id,
                name: person.name.clone(),
                color: share::COLORS[person.color as usize % share::COLORS.len()],
                writer: person.writer,
                host: person.host,
                me: person.id == session.me,
            })
            .collect();
    }
}
