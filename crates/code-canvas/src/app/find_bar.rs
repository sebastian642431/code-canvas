use std::collections::HashSet;

use canvas_core::layout::{NodeKind, NONE};
use canvas_core::Site;

use super::App;
use crate::ext;
use crate::find::Matcher;
use crate::page::{FoundNames, Page, SearchResults};
use crate::ui::{self, Action, FindIn, FindOp, Panel};

impl App {
    pub(super) fn find(&mut self, op: FindOp) {
        match op {
            FindOp::Open | FindOp::OpenReplace => self.find_open(op == FindOp::OpenReplace),
            FindOp::Changed => {
                if let Some(find) = self.ui.find.as_mut() {
                    find.at = 0;
                    find.landed = find.mode == FindIn::File;
                }
                self.find_refresh(true);
            }
            FindOp::Next | FindOp::Previous => {
                if let Some(find) = self.ui.find.as_mut() {
                    self.ui.find_kept.remember(&find.query);
                    find.history_at = None;
                    match (find.count, find.landed) {
                        (0, _) => {}
                        (count, true) => find.at = (find.at + if op == FindOp::Next { 1 } else { count - 1 }) % count,
                        (count, false) if op == FindOp::Previous => find.at = count - 1,
                        _ => {}
                    }
                    find.landed = true;
                }
                self.find_refresh(true);
            }
            FindOp::Replace | FindOp::ReplaceAll => self.find_replace(op == FindOp::ReplaceAll),
            FindOp::Close => {
                let Some(find) = self.ui.find.take() else { return };
                self.ui.find_kept.remember(&find.query);
                if let Some(page) = self.pages.get_mut(self.active).filter(|_| find.mode == FindIn::File) {
                    page.search = None;
                }
                for page in &mut self.pages {
                    page.found_names = None;
                }
                self.request_redraw();
            }
            FindOp::Mode(FindIn::Text) => {
                let query = self.ui.find.as_ref().map(|find| find.query.clone()).unwrap_or_default();
                let options = self.ui.find_kept.options;
                // The find bar is smart-case: a capital in a plain query makes it exact.
                self.ui.search.case = options.case || (!options.regex && query.chars().any(char::is_uppercase));
                self.ui.search.word = options.word;
                self.ui.search.regex = options.regex;
                self.find(FindOp::Close);
                self.ui.show(Panel::Search);
                if !query.trim().is_empty() {
                    self.ui.search_query = query.clone();
                    self.apply(Action::Search(query));
                }
            }
            FindOp::Mode(mode) => {
                let Some(find) = self.ui.find.as_mut() else { return };
                if mode == find.mode || (mode == FindIn::File && find.file.is_empty()) {
                    return;
                }
                let left_the_file = find.mode == FindIn::File;
                (find.mode, find.at, find.landed, find.focus) = (mode, 0, mode == FindIn::File, true);
                if let Some(page) = self.pages.get_mut(self.active) {
                    if left_the_file {
                        page.search = None;
                    }
                    page.found_names = None;
                }
                self.find_refresh(mode == FindIn::File);
            }
        }
    }

    /// The file being edited, or else the one whose card is selected.
    fn find_target(&self) -> Option<String> {
        let page = self.pages.get(self.active)?;
        if let Some(editing) = &page.editing {
            return Some(editing.clone());
        }
        let r = page.ready()?;
        match r.layout.nodes.get(page.selected?)?.kind {
            NodeKind::File(fi) => Some(r.repo.files[fi].rel_path.clone()),
            NodeKind::Dir => None,
        }
    }

    /// The text selected in the file being edited (one line of it), or else the word at its caret.
    fn find_seed(&self, file: &str) -> String {
        let editor = self.pages.get(self.active).and_then(|page| page.editor()).filter(|e| e.rel_path == file);
        let Some(editor) = editor else { return String::new() };
        match editor.selected_text() {
            chosen if chosen.contains('\n') => String::new(),
            chosen if chosen.is_empty() => editor.word_at_caret(),
            chosen => chosen,
        }
    }

    fn find_open(&mut self, replace: bool) {
        let file = self.find_target();
        let seed = file.as_deref().map(|file| self.find_seed(file)).unwrap_or_default();
        if replace {
            self.ui.find_kept.replace = true;
        }
        let Some(find) = self.ui.find.as_mut() else {
            let mode = if file.is_some() { FindIn::File } else { FindIn::Files };
            let travel = !seed.is_empty();
            let query = if travel { seed } else { self.ui.find_kept.history.last().cloned().unwrap_or_default() };
            let landed = mode == FindIn::File;
            let file = file.unwrap_or_default();
            self.ui.find =
                Some(ui::Find { query, focus: true, select: true, file, mode, landed, ..Default::default() });
            return self.find_refresh(travel);
        };
        (find.focus, find.select) = (true, true);
        let moved = file.as_ref().is_some_and(|file| *file != find.file);
        if let Some(file) = file {
            find.file = file;
        }
        let seeded = find.mode == FindIn::File && !seed.is_empty() && seed != find.query;
        if seeded {
            find.query = seed;
        }
        if replace && find.mode != FindIn::File {
            self.find(FindOp::Mode(FindIn::File));
        } else if find.mode == FindIn::File && (moved || seeded) {
            self.find(FindOp::Changed);
        }
    }

    pub(super) fn find_refresh(&mut self, travel: bool) {
        match self.ui.find.as_ref().map(|find| find.mode) {
            Some(FindIn::Files) => self.find_names(travel),
            Some(_) => self.find_again(travel, 5000),
            None => {}
        }
    }

    fn find_names(&mut self, travel: bool) {
        let options = self.ui.find_kept.options;
        let Some(find) = self.ui.find.as_mut() else { return };
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let found = page.ready().map(|r| {
            let dirs = r.layout.dir_node.iter().filter(|(path, _)| !path.is_empty());
            let mut places: Vec<(&str, u32)> = dirs.map(|(path, &node)| (path.as_str(), node)).collect();
            let files = r.repo.files.iter().zip(&r.layout.file_node).filter(|(_, node)| **node != NONE);
            places.extend(files.map(|(f, &node)| (f.rel_path.as_str(), node)));
            let paths: Vec<&str> = places.iter().map(|(path, _)| *path).collect();
            crate::find::names(&paths, &find.query, options).map(|ranked| {
                let mut seen = HashSet::new();
                let alone = ranked.into_iter().filter(|&i| seen.insert(places[i].1));
                alone.map(|i| places[i].0.to_string()).collect::<Vec<String>>()
            })
        });
        let (paths, error) = match found {
            Some(Ok(paths)) => (paths, None),
            Some(Err(error)) => (Vec::new(), Some(error)),
            None => (Vec::new(), None),
        };
        find.error = error;
        find.count = paths.len();
        find.at = find.at.min(paths.len().saturating_sub(1));
        let current = Some(find.at).filter(|_| find.landed && !paths.is_empty());
        let target = current.and_then(|at| paths.get(at).cloned());
        page.found_names = (!paths.is_empty()).then_some(FoundNames { paths, current });
        if let (true, Some(path)) = (travel, target) {
            let node = page.ready().and_then(|r| {
                let node = r.layout.node_for_path(&r.repo, &path)?;
                Some((node, r.repo.file_index(&path).is_some()))
            });
            if let Some((node, is_file)) = node {
                if is_file {
                    find.file = path;
                }
                page.selected = Some(node);
                self.moved_by_itself = true;
                self.frame_node(node);
                self.moved_by_itself = false;
            }
        }
        self.request_redraw();
    }

    fn found_in(page: &Page, file: &str, matcher: &Matcher, most: usize) -> Vec<Site> {
        let mut hits: Vec<Site> = Vec::new();
        let Some(r) = page.ready() else { return hits };
        let Some(index) = r.repo.file_index(file) else { return hits };
        let f = &r.repo.files[index];
        for line in 0..f.line_count() {
            matcher.each(f.line(line), |col, len| {
                if col + len > u16::MAX as usize {
                    return true;
                }
                hits.push(Site { file: index as u32, line: line as u32, col: col as u16, len: len as u16 });
                hits.len() < most
            });
            if hits.len() >= most {
                break;
            }
        }
        hits
    }

    fn find_again(&mut self, travel: bool, most: usize) {
        let options = self.ui.find_kept.options;
        let Some(find) = self.ui.find.as_mut() else { return };
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let matcher = if find.query.is_empty() { Ok(None) } else { Matcher::new(&find.query, options).map(Some) };
        find.error = matcher.as_ref().err().cloned();
        let hits = match &matcher {
            Ok(Some(matcher)) => Self::found_in(page, &find.file, matcher, most),
            _ => Vec::new(),
        };
        find.count = hits.len();
        find.at = find.at.min(hits.len().saturating_sub(1));
        let current = hits.get(find.at).copied();
        page.search = (!hits.is_empty()).then(|| SearchResults {
            query: find.query.clone(),
            hits,
            truncated: false,
            ms: 0.0,
            current: Some(find.at),
        });
        if let (true, Some(site)) = (travel, current) {
            if self.site_in_tab(site) {
                self.ui.tabs.mark = None;
                return;
            }
            self.moved_by_itself = true;
            self.frame_site(site);
            self.moved_by_itself = false;
        }
        self.request_redraw();
    }

    fn find_replace(&mut self, all: bool) {
        let Some(find) = self.ui.find.as_ref().filter(|find| find.mode == FindIn::File) else { return };
        let (file, query) = (find.file.clone(), find.query.clone());
        if !self.ui.prefs.edit {
            let text = "Replacing changes the file: switch to the pencil first.";
            return self.ext.notice(ext::Level::Info, "Find", text.to_string());
        }
        let Some(page) = self.pages.get_mut(self.active) else { return };
        if !page.begin_edit(&file) {
            return;
        }
        self.sync_diffs();
        self.find_again(false, if all { usize::MAX } else { 5000 });
        let Some(at) = self.ui.find.as_ref().map(|find| find.at) else { return };
        let kept = &self.ui.find_kept;
        let (with, preserve) = (kept.with.clone(), kept.preserve);
        let matcher = Matcher::new(&query, kept.options).ok().filter(|_| kept.options.regex || preserve);
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let hits: Vec<Site> = page.search.as_ref().map(|found| found.hits.clone()).unwrap_or_default();
        let chosen: Vec<Site> = if all { hits } else { hits.get(at).copied().into_iter().collect() };
        let texts: Vec<String> = chosen
            .iter()
            .map(|hit| match (&matcher, page.ready()) {
                (Some(matcher), Some(r)) => {
                    let line = r.repo.files[hit.file as usize].line(hit.line as usize);
                    matcher.replacement(line, hit.col as usize, hit.len as usize, &with, preserve)
                }
                _ => with.clone(),
            })
            .collect();
        let Some(editor) = page.editor_mut().filter(|_| !chosen.is_empty()) else { return };
        let place = |editor: &crate::editor::Editor, hit: &Site| {
            let (line, col) = (hit.line as usize, hit.col as usize);
            (editor.pos_at_display(line, col), editor.pos_at_display(line, col + hit.len as usize))
        };
        let mut after = None;
        if all && matcher.is_none() {
            let places: Vec<_> = chosen.iter().map(|hit| place(editor, hit)).collect();
            editor.replace_each(&places, &with);
        } else if all {
            let spans: Vec<_> = chosen
                .iter()
                .zip(texts)
                .map(|(hit, text)| {
                    let (from, to) = place(editor, hit);
                    (from, to, text)
                })
                .collect();
            editor.replace_spans(&spans);
        } else if let (Some(hit), Some(text)) = (chosen.first(), texts.first()) {
            let (from, to) = place(editor, hit);
            editor.move_to(from, false);
            editor.move_to(to, true);
            editor.insert(text);
            after = Some((hit.line, editor.display_col(editor.caret) as u16));
        }
        self.after_edit();
        if let (Some(after), Some(find)) = (after, self.ui.find.as_mut()) {
            let hits = self.pages.get(self.active).and_then(|page| page.search.as_ref()).map(|found| &found.hits);
            let next = hits.and_then(|hits| hits.iter().position(|hit| (hit.line, hit.col) >= after));
            find.at = next.unwrap_or(0);
        }
        self.find_refresh(!all);
    }
}
