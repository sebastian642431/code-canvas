use std::path::{Path, PathBuf};

use super::App;
use crate::ext;
use crate::keys::Chord;
use crate::tree_ops::{self, Clash, Job, Outcome};
use crate::ui::{self, Action, FileOp, NamingKind, TreeOp};

/// The relative path of every folder under `dir`, at any depth.
fn collect_folders(dir: &canvas_core::scan::Dir, out: &mut Vec<String>) {
    for sub in &dir.dirs {
        out.push(sub.rel_path.clone());
        collect_folders(sub, out);
    }
}

pub struct Undoable {
    pub from_root: PathBuf,
    pub to_root: PathBuf,
    pub done: Outcome,
}

impl App {
    fn can_change_files(&mut self) -> bool {
        let shared_read_only = self.pages.get(self.active).is_some_and(|p| p.shared.is_some() && p.read_only);
        if shared_read_only {
            let text = "You are looking, not editing: the one who shares can let you make, rename and delete files.";
            self.ext.notice(ext::Level::Info, "Sharing", text);
        }
        !shared_read_only
    }

    fn folder_name(root: &Path) -> String {
        root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string())
    }

    pub(super) fn tree_op(&mut self, op: TreeOp) {
        let Some(root) = self.pages.get(self.active).map(|p| p.root.clone()) else { return };
        match op {
            TreeOp::Refused(why) => {
                log::info!("files: refused: {why}");
                self.ext.notice(ext::Level::Info, "files", why)
            }
            TreeOp::Cut(paths) => self.ui.tree.clip = Some(ui::TreeClip { root, paths, cut: true }),
            TreeOp::CopyFiles(paths) => self.ui.tree.clip = Some(ui::TreeClip { root, paths, cut: false }),
            TreeOp::Paste(dir) => {
                let Some(clip) = self.ui.tree.clip.as_ref() else { return };
                let job = Job {
                    from_root: clip.root.clone(),
                    to_root: root,
                    items: clip.paths.clone(),
                    to_dir: dir,
                    copy: !clip.cut,
                };
                if clip.cut {
                    self.pasting_cut = true;
                }
                self.transfer(job);
            }
            TreeOp::Move { from_root, to_root, items, to_dir, copy } => {
                self.transfer(Job { from_root, to_root, items, to_dir, copy });
            }
            TreeOp::Duplicate(items) => {
                if !self.can_change_files() {
                    return;
                }
                let mut made: Vec<tree_ops::Moved> = Vec::new();
                for item in tree_ops::top_most(&items) {
                    let job = Job {
                        from_root: root.clone(),
                        to_root: root.clone(),
                        items: vec![item.clone()],
                        to_dir: tree_ops::parent(&item).to_string(),
                        copy: true,
                    };
                    match tree_ops::run(&job, Clash::KeepBoth) {
                        Ok(done) => made.extend(done.moved),
                        Err(why) => self.ext.notice(ext::Level::Error, "files", why),
                    }
                }
                if !made.is_empty() {
                    let done = Outcome { copy: true, moved: made };
                    self.finish_transfer(&root, &root, "", done);
                }
            }
            TreeOp::Delete(items) => self.ask_delete_many(root, items),
            TreeOp::Undo => self.undo_tree(),
        }
        self.request_redraw();
    }

    /// Alt+click or Alt+Enter on a folder of the file list: opens it and every folder inside it, or closes them all.
    pub(super) fn toggle_tree_deep(&mut self, path: &str) {
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let mut folders = Vec::new();
        let Some(r) = page.ready_mut() else { return };
        collect_folders(&r.repo.tree, &mut folders);
        r.tree_dirty = true;
        tree_ops::toggle_deep(&mut page.tree_open, &folders, path);
        self.request_redraw();
    }

    fn ask_delete_many(&mut self, root: PathBuf, items: Vec<String>) {
        let items = tree_ops::top_most(&items);
        match items.as_slice() {
            [] => {}
            [one] => self.apply(Action::File(FileOp::Delete(one.clone()))),
            many => {
                if !self.can_change_files() {
                    return;
                }
                let bin = if cfg!(windows) { "the Recycle Bin" } else { "the trash" };
                let title = format!("Move {} items to {bin}?", many.len());
                let text = format!("They can be brought back from {bin}. What is not saved in them is lost.");
                let about = ui::Asked::DeleteMany { folder: root, paths: many.to_vec() };
                let mut ask = ui::Ask::new(about, &title, text)
                    .choice("Delete", ui::ChoiceLook::Danger, ui::Answer::Yes)
                    .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
                ask.items = many.to_vec();
                self.ui.ask = Some(ask);
            }
        }
    }

    pub(super) fn delete_many(&mut self, folder: PathBuf, paths: Vec<String>) {
        for path in paths {
            self.answered(ui::Asked::Delete { folder: folder.clone(), path }, ui::Answer::Yes);
        }
        self.ui.tree.sel.clear();
    }

    pub(super) fn transfer(&mut self, job: Job) {
        if !self.can_change_files() {
            self.pasting_cut = false;
            return;
        }
        if let Some(why) = job.refusals().into_iter().next() {
            self.pasting_cut = false;
            return self.ext.notice(ext::Level::Info, "files", why);
        }
        if job.from_root != job.to_root {
            let names: Vec<&str> = job.items.iter().map(|item| tree_ops::base(item)).collect();
            let what = match names.as_slice() {
                [one] => format!("\"{one}\""),
                many => format!("{} items", many.len()),
            };
            let (verb, plain) = if job.copy { ("Copy", "Copy") } else { ("Move", "Move") };
            let target = Self::folder_name(&job.to_root);
            let title = format!("{verb} {what} to {target}?");
            let text = format!(
                "{target} is another folder of the workspace, a different project{}.",
                if job.copy { "" } else { ": it leaves this one" }
            );
            let mut ask = ui::Ask::new(ui::Asked::Across, &title, text)
                .choice(plain, ui::ChoiceLook::Main, ui::Answer::Yes)
                .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
            ask.danger = false;
            ask.items = job.items.clone();
            self.ui.ask = Some(ask);
            self.pending_job = Some(job);
            return;
        }
        self.transfer_checked(job);
    }

    pub(super) fn transfer_checked(&mut self, job: Job) {
        let clashes = job.clashes();
        if clashes.is_empty() {
            return self.run_transfer(job, Clash::Replace);
        }
        let title = match clashes.as_slice() {
            [one] => format!("\"{one}\" is already there"),
            many => format!("{} names are already there", many.len()),
        };
        let place = if job.to_dir.is_empty() { "the folder's root".to_string() } else { job.to_dir.clone() };
        let text = format!(
            "{place} has something with {}. Replace sends the old one to the Recycle Bin; Keep both adds \" (2)\" to the new name.",
            if clashes.len() == 1 { "that name" } else { "those names" }
        );
        let mut ask = ui::Ask::new(ui::Asked::Clash, &title, text)
            .choice("Replace", ui::ChoiceLook::Danger, ui::Answer::Yes)
            .choice("Keep both", ui::ChoiceLook::Main, ui::Answer::Other)
            .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
        ask.items = clashes;
        self.ui.ask = Some(ask);
        self.pending_job = Some(job);
    }

    fn page_of(&self, root: &Path) -> Option<usize> {
        self.pages.iter().position(|page| page.root == root)
    }

    pub(super) fn run_transfer(&mut self, job: Job, clash: Clash) {
        let cut = self.pasting_cut;
        self.pasting_cut = false;
        if !job.copy {
            let mut held = Vec::new();
            if let Some(i) = self.page_of(&job.from_root) {
                let inside =
                    |rel: &str| job.items.iter().any(|item| rel == item || rel.starts_with(&format!("{item}/")));
                let dirty: Vec<(usize, String)> =
                    self.pages[i].unsaved().filter(|e| inside(&e.rel_path)).map(|e| (i, e.rel_path.clone())).collect();
                if !dirty.is_empty() {
                    self.save_these(dirty, false);
                }
                held = self.pages[i].unsaved().filter(|e| inside(&e.rel_path)).map(|e| e.rel_path.clone()).collect();
            }
            if let Some(first) = held.first() {
                let text = format!("{first} has changes that could not be saved, so it was not moved.");
                return self.ext.notice(ext::Level::Warn, "files", text);
            }
        }
        match tree_ops::run(&job, clash) {
            Ok(done) => {
                if cut {
                    self.ui.tree.clip = None;
                }
                self.finish_transfer(&job.from_root, &job.to_root, &job.to_dir, done);
            }
            Err(why) => self.ext.notice(ext::Level::Error, "files", why),
        }
        self.request_redraw();
    }

    fn rel_in(root: &Path, path: &Path) -> String {
        let rel = path.strip_prefix(root).unwrap_or(path);
        rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
    }

    fn finish_transfer(&mut self, from_root: &Path, to_root: &Path, to_dir: &str, done: Outcome) {
        let pairs: Vec<(String, String)> =
            done.moved.iter().map(|m| (Self::rel_in(from_root, &m.from), Self::rel_in(to_root, &m.to))).collect();
        if let Some(i) = self.page_of(from_root).filter(|_| !done.copy) {
            let same = from_root == to_root;
            let page = &mut self.pages[i];
            for (old, new) in &pairs {
                let inside = format!("{old}/");
                page.editors.retain(|rel, _| rel != old && !rel.starts_with(&inside));
                if page.editing.as_deref().is_some_and(|rel| rel == old || rel.starts_with(&inside)) {
                    page.editing = None;
                }
                let carry = |set: &mut std::collections::HashSet<String>| {
                    let moving: Vec<String> =
                        set.iter().filter(|p| *p == old || p.starts_with(&inside)).cloned().collect();
                    for p in moving {
                        set.remove(&p);
                        if let Some(to) = tree_ops::remap(&p, old, new).filter(|_| same) {
                            set.insert(to);
                        }
                    }
                };
                carry(&mut page.tree_open);
                carry(&mut page.filter.hidden);
                carry(&mut page.filter.collapsed);
            }
            page.preview_unsaved();
        }
        if let Some(i) = self.page_of(to_root) {
            let mut at = to_dir;
            let page = &mut self.pages[i];
            if !at.is_empty() {
                page.tree_open.insert(at.to_string());
            }
            while let Some((parent, _)) = at.rsplit_once('/') {
                page.tree_open.insert(parent.to_string());
                at = parent;
            }
            if let Some(r) = page.ready_mut() {
                r.tree_dirty = true;
            }
        }
        let reveal = pairs.first().map(|(_, new)| new.clone());
        for root in [from_root, to_root] {
            if let Some(i) = self.page_of(root) {
                let wanted = reveal.clone().filter(|_| root == to_root);
                if !self.rescan_quietly(i, wanted) {
                    self.rescan_later.insert(self.pages[i].id);
                }
            }
        }
        if self.pages.get(self.active).is_some_and(|page| page.root == to_root) {
            self.ui.explorer_reveal = reveal;
        }
        self.ui.tree.sel =
            pairs.iter().filter(|_| to_root == from_root || done.copy).map(|(_, new)| new.clone()).collect();
        let label = if done.copy { "copy" } else { "move" };
        let text = format!("{} Ctrl+Z in the file list takes the {label} back.", tree_ops::describe(&done, to_root));
        log::info!("files: {text}");
        self.ext.notice(ext::Level::Info, "files", text);
        self.ui.tree.can_undo = Some(label.to_string());
        self.tree_undo = Some(Undoable { from_root: from_root.to_path_buf(), to_root: to_root.to_path_buf(), done });
        self.request_redraw();
    }

    fn undo_tree(&mut self) {
        let Some(undoable) = self.tree_undo.take() else { return };
        self.ui.tree.can_undo = None;
        let Undoable { from_root, to_root, done } = undoable;
        match tree_ops::undo(&done) {
            Ok(()) => {
                let what = if done.copy { "The copy is in the Recycle Bin again." } else { "Moved back." };
                log::info!("files: undone");
                self.ext.notice(ext::Level::Info, "files", what);
            }
            Err(why) => self.ext.notice(ext::Level::Warn, "files", format!("It could not be undone: {why}.")),
        }
        for root in [&from_root, &to_root] {
            if let Some(i) = self.page_of(root) {
                if !self.rescan_quietly(i, None) {
                    self.rescan_later.insert(self.pages[i].id);
                }
            }
        }
        self.ui.tree.sel.clear();
    }

    pub(super) fn tree_key(&mut self, chord: &Chord) -> bool {
        if !self.ui.tree.focus || self.ui.ask.is_some() || self.ui.naming.is_some() {
            return false;
        }
        let Some(root) = self.pages.get(self.active).map(|p| p.root.clone()) else { return false };
        let sel: Vec<String> = self.ui.tree.sel.iter().cloned().collect();
        let acted = tree_ops::top_most(&sel);
        let op = match (chord.key.as_str(), chord.ctrl, chord.shift, chord.alt) {
            ("z", true, false, false) if self.tree_undo.is_some() => TreeOp::Undo,
            ("c", true, false, false) if !acted.is_empty() => TreeOp::CopyFiles(acted),
            ("x", true, false, false) if !acted.is_empty() => TreeOp::Cut(acted),
            ("d", true, false, false) if !acted.is_empty() => TreeOp::Duplicate(acted),
            ("v", true, false, false) if self.ui.tree.clip.is_some() => {
                let dir = match sel.last() {
                    Some(last) => tree_ops::paste_dir(last, root.join(last).is_dir()),
                    None => String::new(),
                };
                TreeOp::Paste(dir)
            }
            ("delete", false, false, false) if !acted.is_empty() => TreeOp::Delete(acted),
            ("f2", false, false, false) if sel.len() == 1 => {
                let path = sel[0].clone();
                let text = tree_ops::base(&path).to_string();
                self.ui.naming = Some(ui::Naming { kind: NamingKind::Rename, path, text, focus: true });
                self.request_redraw();
                return true;
            }
            ("enter", false, false, true) if sel.len() == 1 && root.join(&sel[0]).is_dir() => {
                self.apply(Action::ToggleTreeOpenDeep(sel[0].clone()));
                return true;
            }
            ("enter", false, false, false) if sel.len() == 1 => {
                let path = sel[0].clone();
                let action =
                    if root.join(&path).is_dir() { Action::ToggleTreeOpen(path) } else { Action::FlyToPath(path) };
                self.apply(action);
                return true;
            }
            ("escape", false, false, false) if !sel.is_empty() => {
                self.ui.tree.sel.clear();
                self.request_redraw();
                return true;
            }
            _ => return false,
        };
        self.apply(Action::Tree(op));
        true
    }

    pub(super) fn drop_from_system(&mut self, file: PathBuf, at: [f32; 2]) -> bool {
        let Some(root) = self.pages.get(self.active).map(|p| p.root.clone()) else { return false };
        if file.is_dir() {
            return false;
        }
        let s = self.scale();
        let p = egui::pos2(at[0] / s, at[1] / s);
        let Some(target) = self.ui.tree.target_at(p, &root) else { return false };
        let (to_root, to_dir) = match target.at {
            ui::DropAt::Dir(dir) => (root, dir),
            ui::DropAt::Folder(folder) => (folder, String::new()),
        };
        self.copy_from_system(file, &to_root, &to_dir);
        true
    }

    pub(super) fn copy_from_system(&mut self, file: PathBuf, to_root: &Path, to_dir: &str) {
        let Ok(dir) = crate::files::inside(to_root, to_dir) else { return };
        let shown = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let target = dir.join(&shown);
        let result = if target.exists() {
            let fresh = tree_ops::unique_name(&shown, false, |name| dir.join(name).exists());
            Ok(dir.join(fresh))
        } else {
            Ok(target)
        }
        .and_then(|to: PathBuf| crate::files::copy_tree(&file, &to).map(|_| to));
        match result {
            Ok(to) => {
                let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let place = if to_dir.is_empty() { "the folder's root".to_string() } else { to_dir.to_string() };
                log::info!("files: \"{name}\" was copied in from outside into {place}");
                self.ext.notice(ext::Level::Info, "files", format!("\"{name}\" was copied into {place}."));
                let rel = Self::rel_in(to_root, &to);
                if let Some(i) = self.page_of(to_root) {
                    let page = &mut self.pages[i];
                    let mut at = to_dir;
                    if !at.is_empty() {
                        page.tree_open.insert(at.to_string());
                    }
                    while let Some((parent, _)) = at.rsplit_once('/') {
                        page.tree_open.insert(parent.to_string());
                        at = parent;
                    }
                    if !self.rescan_quietly(i, Some(rel.clone())) {
                        self.rescan_later.insert(self.pages[i].id);
                    }
                    self.ui.explorer_reveal = Some(rel);
                }
            }
            Err(why) => self.ext.notice(ext::Level::Warn, "files", format!("\"{shown}\" was not copied: {why}.")),
        }
        self.request_redraw();
    }
}

impl App {
    pub(super) fn files_step(&mut self, what: &str) {
        let mut words = what.split_whitespace();
        let verb = words.next().unwrap_or_default();
        let rest: Vec<&str> = words.collect();
        let row = |me: &App, path: &str| me.ui.tree.rows.iter().find(|r| r.1 == path).map(|r| r.0);
        let spot = |me: &App, name: &str| match name {
            "empty" => Some(egui::pos2(me.ui.tree.list.center().x, me.ui.tree.list.bottom() - 6.0)),
            "root" => Some(me.ui.tree.root_row.center()),
            other => row(me, other).map(|rect| rect.center()).or_else(|| {
                let folder = me.ui.tree.folders.iter().find(|(_, path)| path.file_name().is_some_and(|n| n == other));
                folder.map(|(rect, _)| rect.center())
            }),
        };
        let press = |at: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        match (verb, rest.as_slice()) {
            ("mods", [which]) => {
                self.script_mods = match *which {
                    "ctrl" => egui::Modifiers::CTRL,
                    "shift" => egui::Modifiers::SHIFT,
                    "alt" => egui::Modifiers::ALT,
                    _ => egui::Modifiers::NONE,
                };
                self.ui_events.push_back(vec![egui::Event::ModifiersChanged(self.script_mods)]);
            }
            ("drag", [from, to]) => match (spot(self, from), spot(self, to)) {
                (Some(from), Some(to)) => {
                    let mut frames = vec![vec![egui::Event::PointerMoved(from)], vec![press(from, true)]];
                    frames.extend((1..=5).map(|i| vec![egui::Event::PointerMoved(from.lerp(to, i as f32 / 5.0))]));
                    frames.push(vec![egui::Event::PointerMoved(to)]);
                    frames.push(vec![egui::Event::PointerMoved(to)]);
                    frames.push(vec![press(to, false)]);
                    self.ui_events.extend(frames);
                }
                _ => log::warn!("script: files drag cannot find '{from}' or '{to}' in the list"),
            },
            ("hold", [from, to]) => match (spot(self, from), spot(self, to)) {
                (Some(from), Some(to)) => {
                    let mut frames = vec![vec![egui::Event::PointerMoved(from)], vec![press(from, true)]];
                    frames.extend((1..=5).map(|i| vec![egui::Event::PointerMoved(from.lerp(to, i as f32 / 5.0))]));
                    self.ui_events.extend(frames);
                }
                _ => log::warn!("script: files hold cannot find '{from}' or '{to}' in the list"),
            },
            ("release", [at]) => match spot(self, at) {
                Some(at) => self.ui_events.push_back(vec![press(at, false)]),
                None => log::warn!("script: files release cannot find '{at}'"),
            },
            ("click" | "right", [path]) => match spot(self, path) {
                Some(at) => {
                    let button =
                        if verb == "click" { egui::PointerButton::Primary } else { egui::PointerButton::Secondary };
                    let ev =
                        |pressed| egui::Event::PointerButton { pos: at, button, pressed, modifiers: self.script_mods };
                    self.ui_events.extend([vec![egui::Event::PointerMoved(at)], vec![ev(true)], vec![ev(false)]]);
                }
                None => log::warn!("script: files {verb} cannot find '{path}'"),
            },
            ("hover", [path]) => match spot(self, path) {
                Some(at) => self.ui_events.push_back(vec![egui::Event::PointerMoved(at)]),
                None => log::warn!("script: files hover cannot find '{path}'"),
            },
            ("plus", [path]) => match row(self, path) {
                Some(rect) => {
                    let at = egui::pos2(rect.right() - 20.0, rect.center().y);
                    self.ui_events.extend([
                        vec![egui::Event::PointerMoved(rect.center())],
                        vec![egui::Event::PointerMoved(at)],
                        vec![press(at, true)],
                        vec![press(at, false)],
                    ]);
                }
                None => log::warn!("script: files plus cannot find '{path}'"),
            },
            ("key", [name]) => {
                let (mut ctrl, mut key) = (false, *name);
                if let Some(rest) = key.strip_prefix("ctrl+") {
                    (ctrl, key) = (true, rest);
                }
                self.ui.tree.focus = true;
                if !self.tree_key(&Chord::new(key, ctrl, false, false)) {
                    log::warn!("script: files key {name} did nothing");
                }
            }
            ("drop-os", [file, target]) => match spot(self, target) {
                Some(at) => {
                    let s = self.scale();
                    if !self.drop_from_system(PathBuf::from(file), [at.x * s, at.y * s]) {
                        log::warn!("script: files drop-os found no folder at '{target}'");
                    }
                }
                None => log::warn!("script: files drop-os cannot find '{target}'"),
            },
            ("text", words) if !words.is_empty() => {
                self.ui_events.push_back(vec![egui::Event::Text(words.join(" "))]);
            }
            ("log", []) => {
                let sel: Vec<&String> = self.ui.tree.sel.iter().collect();
                log::info!("script: files selected {sel:?}, undo {:?}", self.ui.tree.can_undo);
            }
            _ => log::warn!("script: files needs mods|drag|hold|release|click|right|hover|plus|key|text|drop-os|log"),
        }
        self.request_redraw();
    }
}

#[cfg(windows)]
pub fn cursor_on_screen() -> Option<[i32; 2]> {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
    }
    let mut at = Point { x: 0, y: 0 };
    let done = unsafe { GetCursorPos(&mut at) };
    (done != 0).then_some([at.x, at.y])
}

#[cfg(not(windows))]
pub fn cursor_on_screen() -> Option<[i32; 2]> {
    None
}
