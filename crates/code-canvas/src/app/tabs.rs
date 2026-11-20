use super::*;
use crate::file_tabs::{same_file, Cell, Reveal, TabOp};

impl App {
    /// The folder and path inside it of the file in front, when a file tab is.
    pub(super) fn front_file(&self) -> Option<(usize, String)> {
        self.locate_file(self.ui.tabs.front_path()?)
    }

    fn in_tab(&self) -> bool {
        self.ui.tabs.front().is_some()
    }

    pub(super) fn tab_op(&mut self, op: TabOp) {
        match op {
            TabOp::Open(path) => self.open_tab(path),
            TabOp::Canvas => {
                self.ui.tabs.set_front(None);
            }
            TabOp::Front(i) => {
                self.ui.tabs.set_front(Some(i));
                self.front_changed();
            }
            TabOp::Close(i) => {
                self.ui.tabs.close(i);
                self.front_changed();
            }
            TabOp::CloseOthers(i) => {
                let Some(kept) = self.ui.tabs.list.get(i).map(|tab| tab.path.clone()) else { return };
                self.ui.tabs.retain(|path| same_file(path, &kept));
                self.ui.tabs.set_front(Some(0));
                self.front_changed();
            }
            TabOp::CloseAll => self.ui.tabs.retain(|_| false),
            TabOp::Cycle(delta) => {
                self.ui.tabs.cycle(delta);
                self.front_changed();
            }
            TabOp::Move(from, slot) => self.ui.tabs.move_tab(from, slot),
            TabOp::Reveal => {
                let Some(path) = self.ui.tabs.front_path().map(Path::to_path_buf) else { return };
                self.ui.tabs.set_front(None);
                self.show_path(&path, None);
            }
            TabOp::Press { row, col, shift, alt, double } => self.press_in_tab(row, col, shift, alt, double),
            TabOp::DragTo { row, col } => self.drag_in_tab(row, col),
        }
        self.request_redraw();
    }

    fn open_tab(&mut self, path: PathBuf) {
        let path = match (path.is_relative(), self.pages.get(self.active)) {
            (true, Some(page)) => page.root.join(path),
            (true, None) => return,
            (false, _) => path,
        };
        let path = canvas_core::scan::clean_path(path);
        if path.is_dir() {
            return self.ext.notice(ext::Level::Info, "Tabs", "A folder is shown on the canvas, not in a tab.");
        }
        if self.locate_file(&path).is_none() {
            let text = format!("{} is not in an open folder, so it does not open in a tab.", path.display());
            return self.ext.notice(ext::Level::Info, "Tabs", text);
        }
        log::info!("opened {} in a tab", path.display());
        self.ui.tabs.open(path);
        self.front_changed();
    }

    /// The folder of the file in front is the one in view, and only that file takes what is typed.
    pub(super) fn front_changed(&mut self) {
        let Some((i, rel)) = self.front_file() else { return };
        if i != self.active {
            self.switch_to(i);
        }
        let page = &mut self.pages[i];
        if page.editing.as_deref().is_some_and(|editing| editing != rel) {
            page.editing = None;
            self.completion = None;
        }
    }

    fn press_in_tab(&mut self, row: usize, col: usize, shift: bool, alt: bool, double: bool) {
        self.completion = None;
        self.ui.tabs.mark = None;
        let Some((i, rel)) = self.front_file() else { return };
        if i != self.active {
            self.switch_to(i);
        }
        let page = &mut self.pages[i];
        let text = page.ready().and_then(|r| r.repo.file_index(&rel).map(|fi| &r.repo.files[fi]));
        let editable = !page.read_only && page.showing.is_none() && text.is_some_and(|f| f.note.is_none());
        let line = page.rows(&rel).and_then(|rows| rows.get(row)).map_or(row, |shown| shown.line as usize);
        if editable && page.begin_edit(&rel) {
            self.ui.tabs.pick = None;
            if let Some(editor) = page.editor_mut() {
                let at = editor.pos_at_display(line, col);
                if alt && !shift {
                    editor.toggle_caret(at);
                } else if double {
                    editor.select_word(at);
                } else {
                    editor.move_to(at, shift);
                }
            }
            return;
        }
        let word = double
            .then(|| page.ready()?.repo.file_index(&rel).map(|fi| word_at(&page_line(page, fi, row), col)))
            .flatten();
        let tabs = &mut self.ui.tabs;
        tabs.pick = match (word, shift, tabs.pick) {
            (Some((from, to)), ..) => Some(((row, from), (row, to))),
            (None, true, Some((anchor, _))) => Some((anchor, (row, col))),
            _ => Some(((row, col), (row, col))),
        };
    }

    fn drag_in_tab(&mut self, row: usize, col: usize) {
        let Some((i, rel)) = self.front_file() else { return };
        let page = &mut self.pages[i];
        if page.editing.as_deref() == Some(rel.as_str()) {
            let line = page.rows(&rel).and_then(|rows| rows.get(row)).map_or(row, |shown| shown.line as usize);
            if let Some(editor) = page.editor_mut() {
                let at = editor.pos_at_display(line, col);
                editor.move_to(at, true);
            }
        } else if let Some((anchor, _)) = self.ui.tabs.pick {
            self.ui.tabs.pick = Some((anchor, (row, col)));
        }
    }

    /// Keys for the file in front while it is not being edited: copying and scrolling. Returns whether one was taken.
    pub(super) fn tab_key(&mut self, event: &KeyEvent) -> bool {
        let Some((i, rel)) = self.front_file() else { return false };
        if i != self.active {
            self.switch_to(i);
        }
        let page = &mut self.pages[i];
        match page.editing.as_deref() {
            Some(editing) if editing == rel => return false,
            Some(_) => page.editing = None,
            None => {}
        }
        let cmd = self.mods.control_key() || self.mods.super_key();
        let rows = self.ui.tabs.page_rows.max(1.0);
        let tabs = &mut self.ui.tabs;
        match &event.logical_key {
            Key::Character(c) if cmd && c.eq_ignore_ascii_case("c") => {
                let page = &self.pages[i];
                let text = tabs.pick.and_then(|pick| picked_text(page, &rel, pick));
                if let Some(text) = text.filter(|text| !text.is_empty()) {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                }
            }
            Key::Character(c) if cmd && c.eq_ignore_ascii_case("a") => {
                let page = &self.pages[i];
                let file = page.ready().and_then(|r| r.repo.file_index(&rel).map(|fi| &r.repo.files[fi]));
                if let Some(f) = file {
                    let last = f.line_count().saturating_sub(1);
                    tabs.pick = Some(((0, 0), (last, f.line(last).chars().count())));
                }
            }
            Key::Named(NamedKey::ArrowUp) => tabs.nudge -= 1.0,
            Key::Named(NamedKey::ArrowDown) => tabs.nudge += 1.0,
            Key::Named(NamedKey::PageUp) => tabs.nudge -= rows - 1.0,
            Key::Named(NamedKey::PageDown) => tabs.nudge += rows - 1.0,
            Key::Named(NamedKey::Home) if cmd => tabs.reveal = Some(Reveal::Row(0, 0)),
            Key::Named(NamedKey::End) if cmd => tabs.nudge += 1.0e7,
            Key::Named(NamedKey::Escape) if tabs.pick.is_some() || tabs.mark.is_some() => {
                tabs.pick = None;
                tabs.mark = None;
            }
            _ => return false,
        }
        self.request_redraw();
        true
    }

    /// While a file is in front, a place in the code opens in a tab instead of flying the canvas there.
    pub(super) fn site_in_tab(&mut self, site: Site) -> bool {
        if !self.in_tab() {
            return false;
        }
        let Some(page) = self.pages.get(self.active) else { return false };
        let Some(rel) = page.ready().and_then(|r| r.repo.files.get(site.file as usize)).map(|f| f.rel_path.clone())
        else {
            return false;
        };
        let path = page.root.join(&rel);
        if !self.ui.tabs.front_path().is_some_and(|front| same_file(front, &path)) {
            self.open_tab(path);
        }
        let (row, col) = (site.line as usize, site.col as usize);
        self.ui.tabs.reveal = Some(Reveal::Row(row, col));
        self.ui.tabs.mark = Some((row, col, site.len as usize));
        self.request_redraw();
        true
    }

    /// While a file is in front, going to another file opens it in a tab; a folder is shown on the canvas.
    pub(super) fn path_in_tab(&mut self, rel: &str) -> bool {
        if !self.in_tab() {
            return false;
        }
        let Some(path) = self.pages.get(self.active).map(|page| page.root.join(rel)) else { return false };
        if path.is_file() {
            self.open_tab(path);
            return true;
        }
        self.ui.tabs.set_front(None);
        false
    }

    /// A link to the file in front scrolls it there; a link anywhere else goes to the canvas, as before tabs.
    pub(super) fn link_in_tab(&mut self, path: &Path, at: Option<(u32, u32, u32)>) -> bool {
        let Some(front) = self.ui.tabs.front_path() else { return false };
        if !same_file(front, path) {
            self.ui.tabs.set_front(None);
            return false;
        }
        if let (Some((line, col, len)), Some((i, rel))) = (at, self.front_file()) {
            let row = self.pages[i].row_of(&rel, line) as usize;
            self.ui.tabs.reveal = Some(Reveal::Row(row, col as usize));
            self.ui.tabs.mark = Some((row, col as usize, len.max(1) as usize));
        }
        self.request_redraw();
        true
    }

    /// The caret of the file in front is kept in sight by its tab, not by the camera.
    pub(super) fn caret_in_tab(&mut self) -> bool {
        if !self.in_tab() {
            return false;
        }
        let editing = self.pages.get(self.active).and_then(|page| page.editing.clone());
        if let (Some((i, rel)), Some(editing)) = (self.front_file(), editing) {
            if i == self.active && rel == editing {
                self.ui.tabs.reveal = Some(Reveal::Caret);
            }
        }
        true
    }

    pub(super) fn place_card_button(&mut self) {
        self.ui.tabs.card_button = self.card_button();
    }

    fn card_button(&mut self) -> Option<(egui::Rect, PathBuf)> {
        let moving = self.drag.as_ref().is_some_and(|drag| drag.moved);
        if self.in_tab() || self.ui.bare || moving || !self.canvas.contains(self.cursor) {
            return None;
        }
        let (vp, s) = (self.viewport(), self.scale());
        let page = self.pages.get(self.active)?;
        let r = page.ready()?;
        let view = page.camera.view(vp);
        let z = view.zoom;
        let world = view.to_world(self.cursor);
        let file_at = |p: [f32; 2]| {
            let n = &r.layout.nodes[r.layout.hit(p)?];
            matches!(n.kind, NodeKind::File(_)).then_some(n)
        };
        let n = file_at(world).or_else(|| {
            let below = file_at([world[0], world[1] + FILE_LABEL_H])?;
            (world[1] < below.rect.y).then_some(below)
        })?;
        let tl = view.to_screen([n.rect.x, n.rect.y]);
        let (w_px, lab_px) = (n.rect.w * z, FILE_LABEL_H * z);
        if lab_px < 9.0 || w_px <= 30.0 {
            return None;
        }
        let size = (lab_px * 0.7).min(14.0 * s).round();
        let (left, top) = (tl[0] + 2.0, tl[1] - lab_px + (lab_px - size * 1.3) * 0.5);
        let unsaved = page.editors.get(&n.rel_path).is_some_and(|e| e.dirty);
        let shown = if unsaved { format!("● {}", n.name) } else { n.name.clone() };
        let text = &mut self.gfx.as_mut()?.renderer.text;
        let named = text.measure(&shown, size, false).min(w_px);
        let x = match crate::markdown::is_markdown(Path::new(&n.name)) {
            true => {
                let tw = (n.name.chars().count() as f32 * size * 0.58).min(w_px);
                left + named.max(tw) + 10.0 * s + text.measure("Preview", size, false) + 7.0 * s
            }
            false => left + named + 6.0 * s,
        };
        let side = (size * 1.3).round();
        if x + side > tl[0] + w_px {
            return None;
        }
        let at = egui::Rect::from_min_size(egui::pos2(x / s, top / s), egui::vec2(side / s, side / s));
        Some((at, page.root.join(&n.rel_path)))
    }

    /// `tab open <path>`, `tab canvas`, `tab close [n]`, `tab front <n>`, `tab next`, `tab previous`, `tab reveal`,
    /// `tab line <n>`, `tab click <line> <column>` (a click in the file in front, both from 1), `tab report`.
    pub(super) fn tab_step(&mut self, arg: &str) {
        let (verb, rest) = arg.split_once(char::is_whitespace).map_or((arg, ""), |(verb, rest)| (verb, rest.trim()));
        let numbers: Vec<usize> = rest.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        let op = match (verb, numbers.as_slice()) {
            ("open", _) => TabOp::Open(PathBuf::from(rest)),
            ("canvas", _) => TabOp::Canvas,
            ("close", [n]) => TabOp::Close(n.saturating_sub(1)),
            ("close", _) => match self.ui.tabs.front() {
                Some(front) => TabOp::Close(front),
                None => return log::warn!("script: tab close, with the canvas in front"),
            },
            ("front", [n]) => TabOp::Front(n.saturating_sub(1)),
            ("next", _) => TabOp::Cycle(1),
            ("previous", _) => TabOp::Cycle(-1),
            ("reveal", _) => TabOp::Reveal,
            ("line", [n]) => {
                self.ui.tabs.reveal = Some(Reveal::Row(n.saturating_sub(1), 0));
                return self.request_redraw();
            }
            ("click", [line, col]) => TabOp::Press {
                row: line.saturating_sub(1),
                col: col.saturating_sub(1),
                shift: false,
                alt: false,
                double: false,
            },
            ("report", _) => {
                let names: Vec<String> = self.ui.tabs.list.iter().map(|tab| tab.path.display().to_string()).collect();
                let editing = self.pages.get(self.active).and_then(|page| page.editing.clone());
                let scroll = self.ui.tabs.front().and_then(|i| self.ui.tabs.list.get(i)).map(|tab| tab.scroll);
                return log::info!(
                    "tabs: [{}], in front: {:?}, scrolled: {scroll:?}, editing: {editing:?}, picked: {:?}",
                    names.join(", "),
                    self.ui.tabs.front(),
                    self.ui.tabs.pick
                );
            }
            _ => return log::warn!("script: tab open <path>|canvas|close [n]|front <n>|next|previous|reveal|line <n>|click <line> <column>|report, not '{arg}'"),
        };
        self.tab_op(op);
    }
}

fn page_line(page: &Page, fi: usize, row: usize) -> String {
    page.ready().and_then(|r| r.repo.files.get(fi)).map_or_else(String::new, |f| f.line(row).to_string())
}

fn word_at(line: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = line.chars().collect();
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    let col = col.min(chars.len());
    let from = (0..col).rev().take_while(|&k| word(&chars[k])).last().unwrap_or(col);
    let to = (col..chars.len()).take_while(|&k| word(&chars[k])).last().map_or(col, |k| k + 1);
    (from, to)
}

fn picked_text(page: &Page, rel: &str, (a, b): (Cell, Cell)) -> Option<String> {
    let r = page.ready()?;
    let f = &r.repo.files[r.repo.file_index(rel)?];
    let ((a_row, a_col), (b_row, b_col)) = if a <= b { (a, b) } else { (b, a) };
    let last = f.line_count().saturating_sub(1);
    let mut out = String::new();
    for row in a_row..=b_row.min(last) {
        let line: Vec<char> = f.line(row).chars().collect();
        let from = if row == a_row { a_col.min(line.len()) } else { 0 };
        let to = if row == b_row { b_col.min(line.len()) } else { line.len() };
        out.extend(&line[from..to.max(from)]);
        if row != b_row {
            out.push('\n');
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::word_at;

    #[test]
    fn a_double_click_takes_the_whole_word() {
        assert_eq!(word_at("let some_name = 1;", 6), (4, 13));
        assert_eq!(word_at("let some_name = 1;", 4), (4, 13));
        assert_eq!(word_at("a + b", 2), (2, 2));
        assert_eq!(word_at("end", 3), (0, 3));
    }
}
