use canvas_core::TAB_WIDTH;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self {
        Pos { line, col }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caret {
    pub at: Pos,
    pub anchor: Option<Pos>,
    goal: Option<usize>,
}

impl Caret {
    fn new(at: Pos) -> Caret {
        Caret { at, anchor: None, goal: None }
    }

    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        (a != self.at).then(|| (a.min(self.at), a.max(self.at)))
    }

    fn span(&self) -> (Pos, Pos) {
        self.selection().unwrap_or((self.at, self.at))
    }
}

#[derive(Default)]
struct Shift {
    lines: isize,
    line: Option<usize>,
    cols: isize,
}

impl Shift {
    fn map(&self, p: Pos) -> Pos {
        let col = if Some(p.line) == self.line { (p.col as isize + self.cols).max(0) as usize } else { p.col };
        Pos::new((p.line as isize + self.lines).max(0) as usize, col)
    }

    fn after(&mut self, old_end: Pos, mapped_end: Pos, new_end: Pos) {
        self.lines += new_end.line as isize - mapped_end.line as isize;
        self.line = Some(old_end.line);
        self.cols = new_end.col as isize - old_end.col as isize;
    }
}

pub const MOST_CARETS: usize = 5000;

struct Change {
    at: Pos,
    removed: String,
    inserted: String,
    caret_before: Pos,
    anchor_before: Option<Pos>,
    others_before: Vec<Caret>,
    joined: bool,
}

pub struct Editor {
    pub rel_path: String,
    lines: Vec<String>,
    ends: Vec<bool>,
    windows: bool,
    marked: bool,
    final_newline: bool,
    pub caret: Pos,
    pub anchor: Option<Pos>,
    goal: Option<usize>,
    others: Vec<Caret>,
    whole_words: bool,
    undo: Vec<Change>,
    redo: Vec<Change>,
    saved_at: Option<usize>,
    on_disk: String,
    pub dirty: bool,
    pub rev: u64,
}

fn next_rev() -> u64 {
    static REVS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    REVS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

impl Editor {
    pub fn open(rel_path: String, source: &str) -> Editor {
        let marked = source.starts_with('\u{feff}');
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let (with_return, all) = (source.matches("\r\n").count(), source.matches('\n').count());
        let windows = with_return * 2 > all;
        let final_newline = source.ends_with('\n');
        let body = source.strip_suffix('\n').unwrap_or(source);
        let ends: Vec<bool> = body.split('\n').map(|l| l.ends_with('\r')).collect();
        let lines: Vec<String> = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
        Editor {
            rel_path,
            on_disk: lines.join("\n"),
            lines,
            ends,
            windows,
            marked,
            final_newline,
            caret: Pos::default(),
            anchor: None,
            goal: None,
            others: Vec::new(),
            whole_words: false,
            undo: Vec::new(),
            redo: Vec::new(),
            saved_at: Some(0),
            dirty: false,
            rev: next_rev(),
        }
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.marked {
            out.push('\u{feff}');
        }
        let last = self.lines.len() - 1;
        for (i, line) in self.lines.iter().enumerate() {
            out.push_str(line);
            if i < last || self.final_newline {
                out.push_str(if self.ends[i] { "\r\n" } else { "\n" });
            }
        }
        out
    }

    pub fn display_source(&self) -> String {
        self.lines.join("\n")
    }

    fn line_len(&self, line: usize) -> usize {
        self.lines[line].chars().count()
    }

    fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        Pos::new(line, p.col.min(self.line_len(line)))
    }

    pub fn display_col(&self, p: Pos) -> usize {
        let mut col = 0;
        for c in self.lines[p.line.min(self.lines.len() - 1)].chars().take(p.col) {
            col += if c == '\t' { TAB_WIDTH - col % TAB_WIDTH } else { 1 };
        }
        col
    }

    pub fn pos_at_display(&self, line: usize, display_col: usize) -> Pos {
        let line = line.min(self.lines.len() - 1);
        let mut col = 0;
        for (i, c) in self.lines[line].chars().enumerate() {
            let w = if c == '\t' { TAB_WIDTH - col % TAB_WIDTH } else { 1 };
            if col + w > display_col {
                return Pos::new(line, if display_col - col > w / 2 { i + 1 } else { i });
            }
            col += w;
        }
        Pos::new(line, self.line_len(line))
    }

    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        (a != self.caret).then(|| (a.min(self.caret), a.max(self.caret)))
    }

    fn range_text(&self, start: Pos, end: Pos) -> String {
        if start.line == end.line {
            let l = &self.lines[start.line];
            return l[byte_at(l, start.col)..byte_at(l, end.col)].to_string();
        }
        let first = &self.lines[start.line];
        let mut out = first[byte_at(first, start.col)..].to_string();
        for l in &self.lines[start.line + 1..end.line] {
            out.push('\n');
            out.push_str(l);
        }
        let last = &self.lines[end.line];
        out.push('\n');
        out.push_str(&last[..byte_at(last, end.col)]);
        out
    }

    pub fn selected_text(&self) -> String {
        let mut all = self.carets();
        all.sort_by_key(Caret::span);
        let parts: Vec<String> = all.iter().filter_map(Caret::selection).map(|(a, b)| self.range_text(a, b)).collect();
        parts.join("\n")
    }

    fn primary(&self) -> Caret {
        Caret { at: self.caret, anchor: self.anchor, goal: self.goal }
    }

    fn set_primary(&mut self, c: Caret) {
        (self.caret, self.anchor, self.goal) = (c.at, c.anchor, c.goal);
    }

    pub fn carets(&self) -> Vec<Caret> {
        let mut all = Vec::with_capacity(1 + self.others.len());
        all.push(self.primary());
        all.extend(self.others.iter().copied());
        all
    }

    pub fn caret_count(&self) -> usize {
        1 + self.others.len()
    }

    pub fn single(&mut self) -> bool {
        self.whole_words = false;
        !std::mem::take(&mut self.others).is_empty()
    }

    fn clamp_caret(&self, c: Caret) -> Caret {
        Caret { at: self.clamp(c.at), anchor: c.anchor.map(|a| self.clamp(a)), goal: c.goal }
    }

    fn settle(&mut self, primary: Caret, others: Vec<Caret>) {
        let mut all: Vec<(Caret, bool)> = others.into_iter().map(|c| (self.clamp_caret(c), false)).collect();
        all.push((self.clamp_caret(primary), true));
        all.sort_by_key(|(c, _)| c.span());
        let mut kept: Vec<(Caret, bool)> = Vec::with_capacity(all.len());
        for (c, main) in all {
            if let Some((last, last_main)) = kept.last_mut() {
                let ((a, b), (c0, c1)) = (last.span(), c.span());
                if c0 < b || (c0 == b && (a == b || c0 == c1)) {
                    let (start, end) = (a.min(c0), b.max(c1));
                    let leader = if main { c } else { *last };
                    let backward = leader.anchor.is_some_and(|x| x > leader.at);
                    *last = match (start == end, backward) {
                        (true, _) => Caret { at: end, anchor: None, goal: leader.goal },
                        (false, true) => Caret { at: start, anchor: Some(end), goal: None },
                        (false, false) => Caret { at: end, anchor: Some(start), goal: None },
                    };
                    *last_main |= main;
                    continue;
                }
            }
            kept.push((c, main));
        }
        let main = kept.iter().position(|(_, main)| *main).unwrap_or(0);
        let (p, _) = kept.remove(main);
        self.set_primary(p);
        self.others = kept.into_iter().map(|(c, _)| c).collect();
    }

    fn each(&mut self, step: impl Fn(&mut Editor)) {
        if self.others.is_empty() {
            return step(self);
        }
        let (primary, others) = (self.primary(), std::mem::take(&mut self.others));
        let mut moved = Vec::with_capacity(others.len());
        for c in others {
            self.set_primary(c);
            step(self);
            moved.push(self.primary());
        }
        self.set_primary(primary);
        step(self);
        let p = self.primary();
        self.settle(p, moved);
    }

    fn edit_each(&mut self, what: impl Fn(&Editor, &Caret) -> Option<(Pos, Pos, String)>) {
        if self.others.is_empty() {
            if let Some((a, b, text)) = what(self, &self.primary()) {
                self.replace(a, b, &text);
            }
            return;
        }
        let primary = self.primary();
        let mut all: Vec<(Caret, bool)> = self.others.iter().map(|c| (*c, false)).collect();
        all.push((primary, true));
        all.sort_by_key(|(c, _)| c.span());
        let edits: Vec<_> = all.iter().map(|(c, _)| what(&*self, c)).collect();
        let (mut shift, mut floor, mut done) = (Shift::default(), Pos::default(), 0);
        let mut placed: Vec<(Caret, bool)> = Vec::with_capacity(all.len());
        for ((c, main), edit) in all.into_iter().zip(edits) {
            let edit =
                edit.map(|(a, b, text)| (a.max(floor), b.max(floor), text.replace("\r\n", "\n").replace('\r', "\n")));
            match edit {
                Some((a, b, text)) if a < b || !text.is_empty() => {
                    let (from, to) = (shift.map(a), shift.map(b));
                    let removed = self.range_text(from, to);
                    let end = self.splice(from, to, &text);
                    if self.saved_at.is_some_and(|depth| depth > self.undo.len()) {
                        self.saved_at = None;
                    }
                    let others_before = if done == 0 { self.others.clone() } else { Vec::new() };
                    self.undo.push(Change {
                        at: from,
                        removed,
                        inserted: text,
                        caret_before: primary.at,
                        anchor_before: primary.anchor,
                        others_before,
                        joined: done > 0,
                    });
                    shift.after(b, to, end);
                    (floor, done) = (b, done + 1);
                    placed.push((Caret::new(end), main));
                }
                _ => {
                    let at = |p: Pos| shift.map(p.max(floor));
                    placed.push((Caret { at: at(c.at), anchor: c.anchor.map(at), goal: c.goal }, main));
                }
            }
        }
        if done == 0 {
            return;
        }
        self.redo.clear();
        self.whole_words = false;
        let main = placed.iter().position(|(_, main)| *main).unwrap_or(0);
        let (p, _) = placed.remove(main);
        self.settle(p, placed.into_iter().map(|(c, _)| c).collect());
        self.touch();
    }

    fn splice(&mut self, start: Pos, end: Pos, text: &str) -> Pos {
        let head = self.lines[start.line][..byte_at(&self.lines[start.line], start.col)].to_string();
        let tail = self.lines[end.line][byte_at(&self.lines[end.line], end.col)..].to_string();
        let mut new: Vec<String> = text.split('\n').map(str::to_string).collect();
        let last = new.len() - 1;
        let end_pos = Pos::new(start.line + last, if last == 0 { start.col } else { 0 } + new[last].chars().count());
        new[0].insert_str(0, &head);
        new[last].push_str(&tail);
        let old: Vec<bool> = self.ends[start.line..=end.line].to_vec();
        let end_of = |i: usize| match i {
            _ if new.len() == old.len() => old[i],
            _ if i == last => old[old.len() - 1],
            _ if i + 1 < old.len() => old[i],
            _ => self.windows,
        };
        let ends: Vec<bool> = (0..new.len()).map(end_of).collect();
        self.ends.splice(start.line..=end.line, ends);
        self.lines.splice(start.line..=end.line, new);
        end_pos
    }

    fn replace(&mut self, start: Pos, end: Pos, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let removed = self.range_text(start, end);
        if removed.is_empty() && text.is_empty() {
            return;
        }
        let change = Change {
            at: start,
            removed,
            inserted: text,
            caret_before: self.caret,
            anchor_before: self.anchor,
            others_before: std::mem::take(&mut self.others),
            joined: false,
        };
        self.caret = self.splice(start, end, &change.inserted);
        self.anchor = None;
        self.goal = None;
        self.whole_words = false;
        if self.saved_at.is_some_and(|depth| depth > self.undo.len()) {
            self.saved_at = None;
        }
        self.undo.push(change);
        self.redo.clear();
        self.touch();
    }

    fn touch(&mut self) {
        self.dirty = self.saved_at != Some(self.undo.len());
        self.rev = next_rev();
    }

    fn offset_of(&self, p: Pos) -> usize {
        let p = self.clamp(p);
        self.lines[..p.line].iter().map(|line| line.chars().count() + 1).sum::<usize>() + p.col
    }

    fn pos_of(&self, mut offset: usize) -> Pos {
        for (i, line) in self.lines.iter().enumerate() {
            let len = line.chars().count();
            if offset <= len {
                return Pos::new(i, offset);
            }
            offset -= len + 1;
        }
        let last = self.lines.len() - 1;
        Pos::new(last, self.line_len(last))
    }

    pub fn apply_remote(&mut self, op: &crate::share::ot::TextOp) -> Option<(u32, u32)> {
        const ONE_BY_ONE: usize = 64;
        let was = self.display_source();
        let is = op.apply(&was)?;
        if is == was {
            return Some((self.caret.line as u32, 0));
        }
        let (caret, anchor, goal) = (self.offset_of(self.caret), self.anchor.map(|a| self.offset_of(a)), self.goal);
        let others: Vec<(usize, Option<usize>, Option<usize>)> =
            self.others.iter().map(|c| (self.offset_of(c.at), c.anchor.map(|a| self.offset_of(a)), c.goal)).collect();
        let mut places = op.splices();
        if places.len() > ONE_BY_ONE {
            let (was, is): (Vec<char>, Vec<char>) = (was.chars().collect(), is.chars().collect());
            let most = was.len().min(is.len());
            let head = (0..most).take_while(|i| was[*i] == is[*i]).count();
            let tail = (0..most - head).take_while(|i| was[was.len() - 1 - i] == is[is.len() - 1 - i]).count();
            places = vec![(head, was.len() - tail, is[head..is.len() - tail].iter().collect())];
        }
        let (start, end) = (places.first()?.0, places.last()?.1);
        self.redo.clear();
        for (from, to, put) in places.iter().rev() {
            let (from, to) = (self.pos_of(*from), self.pos_of(*to));
            self.keep_history(from, to, put);
            self.splice(from, to, put);
        }
        self.saved_at = None;
        self.touch();
        let (first, stop) = (self.pos_of(start).line, self.pos_of(op.place(end)));
        let last = if stop.col == 0 && stop.line > first { stop.line - 1 } else { stop.line };
        self.caret = self.pos_of(op.place(caret));
        self.anchor = anchor.map(|a| self.pos_of(op.place(a)));
        self.goal = goal;
        if !others.is_empty() {
            let place = |e: &Editor, at: usize| e.pos_of(op.place(at));
            let others: Vec<Caret> = others
                .into_iter()
                .map(|(at, anchor, goal)| Caret {
                    at: place(&*self, at),
                    anchor: anchor.map(|a| place(&*self, a)),
                    goal,
                })
                .collect();
            self.settle(self.primary(), others);
        }
        Some((first as u32, (last.max(first) - first + 1) as u32))
    }

    fn moved(at: Pos, to: Pos, end: Pos) -> Pos {
        match at.line == to.line {
            true => Pos::new(end.line, end.col + at.col - to.col),
            false => Pos::new(at.line + end.line - to.line, at.col),
        }
    }

    fn keep_history(&mut self, mut from: Pos, mut to: Pos, put: &str) {
        let mut lost = 0;
        for i in (0..self.undo.len()).rev() {
            let change = &mut self.undo[i];
            let end = Self::end_of(change.at, &change.inserted);
            if to <= change.at {
                change.at = Self::moved(change.at, to, Self::end_of(from, put));
            } else if from >= end {
                let back = Self::end_of(change.at, &change.removed);
                (from, to) = (Self::moved(from, end, back), Self::moved(to, end, back));
            } else {
                lost = i + 1;
                break;
            }
        }
        self.undo.drain(..lost);
        if let Some(first) = self.undo.first_mut() {
            first.joined = false;
        }
    }

    pub fn take_in(&mut self, disk: &str) -> Option<(u32, u32)> {
        use crate::share::ot::TextOp;
        if disk == self.on_disk {
            return Some((self.caret.line as u32, 0));
        }
        let theirs = TextOp::between(&self.on_disk, disk);
        let mine = TextOp::between(&self.on_disk, &self.display_source());
        let (theirs, _) = TextOp::transform(&theirs, &mine)?;
        let lines = self.apply_remote(&theirs)?;
        self.on_disk = disk.to_string();
        self.saved_at = None;
        self.dirty = true;
        Some(lines)
    }

    pub fn become_text(&mut self, body: &str) {
        let (caret, last) = (self.caret, self.lines.len() - 1);
        let end = Pos::new(last, self.line_len(last));
        self.replace(Pos::default(), end, body);
        self.caret = self.clamp(caret);
    }

    fn end_of(at: Pos, text: &str) -> Pos {
        match text.rsplit_once('\n') {
            Some((before, last)) => Pos::new(at.line + before.matches('\n').count() + 1, last.chars().count()),
            None => Pos::new(at.line, at.col + text.chars().count()),
        }
    }

    pub fn undo(&mut self) -> bool {
        if self.undo.is_empty() {
            return false;
        }
        while let Some(change) = self.undo.pop() {
            self.splice(change.at, Self::end_of(change.at, &change.inserted), &change.removed);
            let head = !change.joined;
            if head {
                self.caret = self.clamp(change.caret_before);
                self.anchor = change.anchor_before.map(|anchor| self.clamp(anchor));
                self.others = change.others_before.iter().map(|c| self.clamp_caret(*c)).collect();
            }
            self.redo.push(change);
            if head {
                break;
            }
        }
        self.goal = None;
        self.whole_words = false;
        self.touch();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(mut change) = self.redo.pop() else { return false };
        let mut ends = Vec::new();
        loop {
            ends.push(self.splice(change.at, Self::end_of(change.at, &change.removed), &change.inserted));
            self.undo.push(change);
            match self.redo.pop() {
                Some(next) if next.joined => change = next,
                Some(next) => {
                    self.redo.push(next);
                    break;
                }
                None => break,
            }
        }
        self.caret = ends.pop().unwrap_or(self.caret);
        self.anchor = None;
        self.goal = None;
        self.others = ends.into_iter().map(Caret::new).collect();
        self.whole_words = false;
        self.touch();
        true
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    pub fn replace_each(&mut self, places: &[(Pos, Pos)], with: &str) {
        let (Some(first), Some(last)) = (places.first(), places.last()) else { return };
        let caret = self.caret;
        let (top, bottom) = (first.0.line, last.1.line);
        let mut lines: Vec<String> = self.lines[top..=bottom].to_vec();
        for (from, to) in places.iter().rev() {
            let line = &mut lines[from.line - top];
            let (a, b) = (byte_at(line, from.col), byte_at(line, to.col));
            line.replace_range(a..b, with);
        }
        let end = Pos::new(bottom, self.line_len(bottom));
        self.replace(Pos::new(top, 0), end, &lines.join("\n"));
        self.caret = self.clamp(caret);
    }

    pub fn replace_spans(&mut self, spans: &[(Pos, Pos, String)]) {
        let mut spans: Vec<&(Pos, Pos, String)> = spans
            .iter()
            .filter(|(from, to, _)| from.line == to.line && from <= to && to.line < self.lines.len())
            .collect();
        spans.sort_by_key(|(from, ..)| *from);
        let (Some(first), Some(last)) = (spans.first(), spans.last()) else { return };
        let (top, bottom) = (first.0.line, last.1.line);
        let caret = Pos::new(first.0.line, first.0.col + first.2.chars().count());
        let mut lines: Vec<String> = self.lines[top..=bottom].to_vec();
        for (from, to, with) in spans.iter().rev() {
            let line = &mut lines[from.line - top];
            let (a, b) = (byte_at(line, from.col), byte_at(line, to.col));
            line.replace_range(a..b, with);
        }
        let end = Pos::new(bottom, self.line_len(bottom));
        self.replace(
            Pos::new(top, 0),
            end,
            &lines.join(
                "
",
            ),
        );
        self.caret = self.clamp(caret);
    }

    pub fn complete(&mut self, from: usize, text: &str) {
        let end = self.caret;
        self.replace(Pos::new(end.line, from.min(end.col)), end, text);
    }

    pub fn edit_before(&mut self, from: Pos, to: Pos, text: &str) -> bool {
        let (caret, anchor) = (self.caret, self.anchor);
        if to.line >= caret.line || from > to {
            return false;
        }
        let lines = self.lines.len();
        self.replace(from, to, text);
        let added = self.lines.len() as isize - lines as isize;
        let moved = |p: Pos| Pos::new((p.line as isize + added).max(0) as usize, p.col);
        (self.caret, self.anchor) = (moved(caret), anchor.map(moved));
        true
    }

    pub fn insert(&mut self, text: &str) {
        self.edit_each(|_, c| {
            let (start, end) = c.span();
            Some((start, end, text.to_string()))
        });
    }

    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n");
        let lines: Vec<&str> = text.strip_suffix('\n').unwrap_or(&text).split('\n').collect();
        if self.others.is_empty() || lines.len() != self.caret_count() {
            return self.insert(&text);
        }
        let mut order: Vec<Pos> = self.carets().iter().map(|c| c.span().0).collect();
        order.sort();
        self.edit_each(|_, c| {
            let (start, end) = c.span();
            let nth = order.binary_search(&start).unwrap_or(0);
            Some((start, end, lines[nth].to_string()))
        });
    }

    pub fn delete_selected(&mut self) {
        self.edit_each(|_, c| c.selection().map(|(a, b)| (a, b, String::new())));
    }

    pub fn newline(&mut self) {
        self.edit_each(|e, c| {
            let line = &e.lines[c.at.line];
            let indent: String = line.chars().take(c.at.col).take_while(|c| *c == ' ' || *c == '\t').collect();
            let (start, end) = c.span();
            Some((start, end, format!("\n{indent}")))
        });
    }

    pub fn tab(&mut self) {
        let uses_tabs = self.lines.iter().any(|l| l.starts_with('\t'));
        if uses_tabs {
            return self.insert("\t");
        }
        let step = self
            .lines
            .iter()
            .map(|l| l.len() - l.trim_start_matches(' ').len())
            .filter(|n| *n > 0 && *n <= 8)
            .min()
            .unwrap_or(TAB_WIDTH);
        self.edit_each(|e, c| {
            let (start, end) = c.span();
            let col = e.display_col(start);
            Some((start, end, " ".repeat(step - col % step)))
        });
    }

    pub fn backspace(&mut self) {
        self.edit_each(|e, c| {
            if let Some((a, b)) = c.selection() {
                return Some((a, b, String::new()));
            }
            let c = c.at;
            if c.col > 0 {
                Some((Pos::new(c.line, c.col - 1), c, String::new()))
            } else if c.line > 0 {
                Some((Pos::new(c.line - 1, e.line_len(c.line - 1)), c, String::new()))
            } else {
                None
            }
        });
    }

    pub fn delete(&mut self) {
        self.edit_each(|e, c| {
            if let Some((a, b)) = c.selection() {
                return Some((a, b, String::new()));
            }
            let c = c.at;
            if c.col < e.line_len(c.line) {
                Some((c, Pos::new(c.line, c.col + 1), String::new()))
            } else if c.line + 1 < e.lines.len() {
                Some((c, Pos::new(c.line + 1, 0), String::new()))
            } else {
                None
            }
        });
    }

    pub fn move_to(&mut self, p: Pos, select: bool) {
        let p = self.clamp(p);
        self.anchor = match (select, self.anchor) {
            (true, None) => Some(self.caret),
            (true, kept) => kept,
            (false, _) => None,
        };
        self.caret = p;
        self.goal = None;
        self.single();
    }

    pub fn left(&mut self, select: bool) {
        self.each(|e| e.left_one(select));
    }

    fn left_one(&mut self, select: bool) {
        if let (false, Some((a, _))) = (select, self.selection()) {
            return self.move_to(a, false);
        }
        let c = self.caret;
        let p = if c.col > 0 {
            Pos::new(c.line, c.col - 1)
        } else if c.line > 0 {
            Pos::new(c.line - 1, self.line_len(c.line - 1))
        } else {
            c
        };
        self.move_to(p, select);
    }

    pub fn right(&mut self, select: bool) {
        self.each(|e| e.right_one(select));
    }

    fn right_one(&mut self, select: bool) {
        if let (false, Some((_, b))) = (select, self.selection()) {
            return self.move_to(b, false);
        }
        let c = self.caret;
        let p = if c.col < self.line_len(c.line) {
            Pos::new(c.line, c.col + 1)
        } else if c.line + 1 < self.lines.len() {
            Pos::new(c.line + 1, 0)
        } else {
            c
        };
        self.move_to(p, select);
    }

    pub fn vertical(&mut self, delta: isize, select: bool) {
        self.each(|e| {
            let goal = e.goal.unwrap_or_else(|| e.display_col(e.caret));
            let line = (e.caret.line as isize + delta).clamp(0, e.lines.len() as isize - 1) as usize;
            let p = e.pos_at_display(line, goal);
            e.move_to(p, select);
            e.goal = Some(goal);
        });
    }

    pub fn vertical_in(&mut self, delta: isize, select: bool, rows: Option<&canvas_core::Wrap>) {
        let Some(rows) = rows else { return self.vertical(delta, select) };
        self.each(|e| {
            let at = e.display_col(e.caret) as u32;
            let (line, col, goal) = rows.step(e.caret.line, at, e.goal.map(|goal| goal as u32), delta);
            let p = e.pos_at_display(line.min(e.lines.len().saturating_sub(1)), col as usize);
            e.move_to(p, select);
            e.goal = Some(goal as usize);
        });
    }

    pub fn home(&mut self, select: bool) {
        self.each(|e| {
            let line = &e.lines[e.caret.line];
            let first = line.chars().take_while(|c| c.is_whitespace()).count();
            let col = if e.caret.col == first { 0 } else { first };
            e.move_to(Pos::new(e.caret.line, col), select);
        });
    }

    pub fn end(&mut self, select: bool) {
        self.each(|e| e.move_to(Pos::new(e.caret.line, e.line_len(e.caret.line)), select));
    }

    pub fn word(&mut self, forward: bool, select: bool) {
        self.each(|e| e.word_one(forward, select));
    }

    fn word_one(&mut self, forward: bool, select: bool) {
        let chars: Vec<char> = self.lines[self.caret.line].chars().collect();
        let mut col = self.caret.col;
        if forward {
            if col >= chars.len() {
                return self.right_one(select);
            }
            while col < chars.len() && !is_word(chars[col]) {
                col += 1;
            }
            while col < chars.len() && is_word(chars[col]) {
                col += 1;
            }
        } else {
            if col == 0 {
                return self.left_one(select);
            }
            while col > 0 && !is_word(chars[col - 1]) {
                col -= 1;
            }
            while col > 0 && is_word(chars[col - 1]) {
                col -= 1;
            }
        }
        self.move_to(Pos::new(self.caret.line, col), select);
    }

    pub fn select_all(&mut self) {
        let last = self.lines.len() - 1;
        self.single();
        self.anchor = Some(Pos::default());
        self.caret = Pos::new(last, self.line_len(last));
    }

    fn word_around(&self, p: Pos) -> (Pos, Pos) {
        let p = self.clamp(p);
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        let (mut a, mut b) = (p.col.min(chars.len()), p.col.min(chars.len()));
        while a > 0 && is_word(chars[a - 1]) {
            a -= 1;
        }
        while b < chars.len() && is_word(chars[b]) {
            b += 1;
        }
        (Pos::new(p.line, a), Pos::new(p.line, b))
    }

    pub fn word_at_caret(&self) -> String {
        let (a, b) = self.word_around(self.caret);
        self.line(a.line).chars().skip(a.col).take(b.col - a.col).collect()
    }

    pub fn select_word(&mut self, p: Pos) {
        let (a, b) = self.word_around(p);
        self.single();
        self.anchor = Some(a);
        self.caret = b;
    }

    pub fn toggle_caret(&mut self, p: Pos) {
        let p = self.clamp(p);
        let mut all = self.carets();
        self.whole_words = false;
        let hit = all.iter().position(|c| c.at == p || c.selection().is_some_and(|(a, b)| (a..=b).contains(&p)));
        match hit {
            Some(_) if all.len() == 1 => {}
            Some(i) => {
                all.remove(i);
                let primary = if i == 0 { all.pop() } else { Some(all.remove(0)) };
                if let Some(primary) = primary {
                    self.settle(primary, all);
                }
            }
            None if all.len() < MOST_CARETS => self.settle(Caret::new(p), all),
            None => {}
        }
    }

    pub fn add_caret_vertical(&mut self, delta: isize) -> bool {
        let p = self.primary();
        let goal = p.goal.unwrap_or_else(|| self.display_col(p.at));
        let line = p.at.line as isize + delta;
        if line < 0 || line >= self.lines.len() as isize || self.caret_count() >= MOST_CARETS {
            return false;
        }
        let at = self.pos_at_display(line as usize, goal);
        let others = self.carets();
        self.settle(Caret { at, anchor: None, goal: Some(goal) }, others);
        self.whole_words = false;
        true
    }

    fn occurrences(&self, needle: &str, whole: bool) -> Vec<(Pos, Pos)> {
        let mut found = Vec::new();
        if needle.is_empty() {
            return found;
        }
        let text = self.display_source();
        let mut starts = Vec::with_capacity(self.lines.len());
        let mut byte = 0;
        for line in &self.lines {
            starts.push(byte);
            byte += line.len() + 1;
        }
        let pos = |byte: usize| {
            let line = starts.partition_point(|s| *s <= byte) - 1;
            Pos::new(line, text[starts[line]..byte].chars().count())
        };
        let mut from = 0;
        while let Some(i) = text[from..].find(needle) {
            let (s, e) = (from + i, from + i + needle.len());
            let alone =
                !text[..s].chars().next_back().is_some_and(is_word) && !text[e..].chars().next().is_some_and(is_word);
            if alone || !whole {
                found.push((pos(s), pos(e)));
                from = e;
            } else {
                from = s + needle.chars().next().map_or(1, char::len_utf8);
            }
        }
        found
    }

    fn select_words(&mut self) -> bool {
        let primary = self.primary();
        let others = std::mem::take(&mut self.others);
        let word = |e: &Editor, c: Caret| match c.selection() {
            Some(_) => c,
            None => match e.word_around(c.at) {
                (a, b) if a == b => c,
                (a, b) => Caret { at: b, anchor: Some(a), goal: None },
            },
        };
        let others: Vec<Caret> = others.into_iter().map(|c| word(&*self, c)).collect();
        let primary = word(self, primary);
        self.settle(primary, others);
        self.whole_words = true;
        self.selection().is_some()
    }

    pub fn add_next_occurrence(&mut self) -> bool {
        let Some((a, b)) = self.selection() else { return self.select_words() };
        let whole = self.whole_words;
        let taken: Vec<(Pos, Pos)> = self.carets().iter().filter_map(Caret::selection).collect();
        let found = self.occurrences(&self.range_text(a, b), whole);
        let free = |range: &&(Pos, Pos)| !taken.contains(range);
        let next = found.iter().filter(free).find(|(start, _)| *start >= b).or_else(|| found.iter().find(free));
        let Some(&(start, end)) = next.filter(|_| self.caret_count() < MOST_CARETS) else { return false };
        let others = self.carets();
        self.settle(Caret { at: end, anchor: Some(start), goal: None }, others);
        self.whole_words = whole;
        true
    }

    pub fn select_all_occurrences(&mut self) -> bool {
        if self.selection().is_none() && !self.select_words() {
            return false;
        }
        let Some((a, b)) = self.selection() else { return false };
        let whole = self.whole_words;
        let found = self.occurrences(&self.range_text(a, b), whole);
        let mut all: Vec<Caret> =
            found.into_iter().take(MOST_CARETS).map(|(s, e)| Caret { at: e, anchor: Some(s), goal: None }).collect();
        let main = all.iter().position(|c| c.span() == (a, b));
        let primary = match main {
            Some(i) => all.remove(i),
            None => self.primary(),
        };
        self.settle(primary, all);
        self.whole_words = whole;
        true
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo.len());
        self.on_disk = self.display_source();
        self.dirty = false;
    }

    pub fn written_as(&mut self, unsaved: bool) {
        self.mark_saved();
        if unsaved {
            self.saved_at = None;
            self.dirty = true;
        }
    }

    pub fn read_as(&self, disk: &str) -> bool {
        self.on_disk == disk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(e: &mut Editor, line: usize, col: usize) {
        e.move_to(Pos::new(line, col), false);
    }

    #[test]
    fn several_spans_are_replaced_as_one_undo() {
        let source = "{
  \"a\": \"^1.0.0\",
  \"b\": \"~2.1\",
  \"c\": \"3\"
}
";
        let mut e = Editor::open("package.json".into(), source);
        let span = |line: usize, col: usize, len: usize, with: &str| {
            (Pos::new(line, col), Pos::new(line, col + len), with.to_string())
        };
        e.replace_spans(&[span(3, 8, 1, "4"), span(1, 8, 6, "^1.2.0"), span(2, 8, 4, "~2.3")]);
        assert_eq!(
            e.display_source(),
            "{
  \"a\": \"^1.2.0\",
  \"b\": \"~2.3\",
  \"c\": \"4\"
}"
        );
        assert_eq!(e.caret, Pos::new(1, 14));
        assert!(e.dirty);
        assert!(e.undo());
        assert_eq!(e.text(), source);
        assert!(!e.dirty);
        assert!(!e.undo());
        e.replace_spans(&[]);
        assert_eq!(e.text(), source);
    }

    #[test]
    fn keeps_the_files_own_line_endings() {
        let e = Editor::open("a.txt".into(), "one\r\n\ttwo\r\n");
        assert_eq!(e.text(), "one\r\n\ttwo\r\n");
        assert_eq!(e.display_source(), "one\n\ttwo");
        let no_newline = Editor::open("b.txt".into(), "x\ny");
        assert_eq!(no_newline.text(), "x\ny");
        assert_eq!(Editor::open("c.txt".into(), "").text(), "");
    }

    #[test]
    fn what_someone_else_types_goes_in_and_the_caret_stays_on_its_text() {
        use crate::share::ot::TextOp;
        let before = "fn main() {\n    run();\n}";
        let mut e = Editor::open("a.rs".into(), "fn main() {\r\n    run();\r\n}\r\n");
        at(&mut e, 1, 8);
        let (rev, text) = (e.rev, e.display_source());
        assert_ne!(
            Editor::open("a.rs".into(), "x").rev,
            Editor::open("a.rs".into(), "x").rev,
            "no two editors share a mark"
        );
        assert_eq!(text, before);
        let after = "// theirs\nfn main() {\n    run();\n    stop();\n}";
        let lines = e.apply_remote(&TextOp::between(before, after));
        assert_eq!(e.display_source(), after);
        assert_eq!(e.caret, Pos::new(2, 8), "the caret is still after run(");
        assert_eq!(lines, Some((0, 4)), "from the first line touched to the last");
        assert!(e.dirty && e.rev > rev);
        assert_eq!(
            e.text(),
            "// theirs\r\nfn main() {\r\n    run();\r\n    stop();\r\n}\r\n",
            "the file keeps its line ends"
        );

        let typed_at_caret = TextOp::between(after, "// theirs\nfn main() {\n    run(now);\n    stop();\n}");
        e.apply_remote(&typed_at_caret);
        assert_eq!(e.caret, Pos::new(2, 11), "what is typed where the caret is pushes it");
        assert!(e.apply_remote(&TextOp::between("another text", "x")).is_none(), "an edit for another text is refused");
        let long: String = "line\n".repeat(4000);
        let mut big = Editor::open("big.txt".into(), &long);
        let body = big.display_source();
        let mut every_other = TextOp::default();
        for _ in 0..body.chars().count() / 2 {
            every_other.retain(1);
            every_other.insert("x");
            every_other.retain(1);
        }
        every_other.retain(body.chars().count() % 2);
        let started = std::time::Instant::now();
        assert!(big.apply_remote(&every_other).is_some());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "an edit of many parts costs the text once, not once per part"
        );
        assert_eq!(big.display_source(), every_other.apply(&body).unwrap());
        e.become_text("all new\ntext");
        assert_eq!(e.display_source(), "all new\ntext");
        assert!(e.undo());
        assert_eq!(e.display_source(), "// theirs\nfn main() {\n    run(now);\n    stop();\n}");
    }

    #[test]
    fn a_text_written_for_others_stays_not_saved_when_what_was_typed_is_not() {
        let mut e = Editor::open("a.txt".into(), "one\ntwo\n");
        at(&mut e, 0, 3);
        e.insert(" mine");
        e.written_as(true);
        assert!(e.dirty && e.read_as("one mine\ntwo"), "what is on disk is known, what was typed is still to save");
        e.written_as(false);
        assert!(!e.dirty);
    }

    #[test]
    fn what_another_program_wrote_joins_what_is_not_saved() {
        let mut e = Editor::open("a.txt".into(), "one\ntwo\nthree\n");
        at(&mut e, 1, 3);
        e.insert(" mine");
        assert_eq!(e.take_in("zero\none\ntwo\nthree\nfour"), Some((0, 5)));
        assert_eq!(e.display_source(), "zero\none\ntwo mine\nthree\nfour");
        assert_eq!(e.caret, Pos::new(2, 8), "the caret stays after what was typed");
        assert!(e.dirty);
        assert!(e.undo());
        assert_eq!(
            e.display_source(),
            "zero
one
two
three
four",
            "undo takes back what was typed here, not what came"
        );
        assert!(e.dirty, "the text is the file's again, but nothing here knows that for sure until it is saved");
        assert!(!e.undo());
        e.mark_saved();
        assert_eq!(e.take_in(&e.display_source()), Some((e.caret.line as u32, 0)), "nothing new on disk");
        assert!(!e.dirty);
    }

    #[test]
    fn every_line_changed_at_once_leaves_the_caret_on_its_line() {
        use crate::share::ot::TextOp;
        let before = "line 01\nline 02\nline 03\nline 04";
        let mut e = Editor::open("a.txt".into(), before);
        at(&mut e, 2, 7);
        let all = TextOp::between(before, "row 01\nrow 02\nrow 03\nrow 04");
        assert!(e.apply_remote(&all).is_some());
        assert_eq!(e.caret, Pos::new(2, 6));
        assert_eq!(e.line(2), "row 03");
    }

    #[test]
    fn an_edit_above_the_caret_leaves_it_on_its_text() {
        let mut e = Editor::open("a.rs".into(), "mod util;\n\nfn main() {\n    let m = HashMap\n}\n");
        at(&mut e, 3, 19);
        assert!(e.edit_before(Pos::new(0, 0), Pos::new(0, 0), "use std::collections::HashMap;\n\n"));
        assert_eq!(e.caret, Pos::new(5, 19));
        assert_eq!(e.line(0), "use std::collections::HashMap;");
        assert_eq!(e.line(5), "    let m = HashMap");
        assert!(!e.edit_before(Pos::new(5, 0), Pos::new(5, 4), ""), "not on the caret's own line");
        assert!(e.edit_before(Pos::new(0, 0), Pos::new(2, 0), ""));
        assert_eq!(e.caret, Pos::new(3, 19));
        assert!(e.undo() && e.undo());
        assert_eq!(e.text(), "mod util;\n\nfn main() {\n    let m = HashMap\n}\n");
    }

    fn places(e: &Editor) -> Vec<(usize, usize)> {
        let mut all: Vec<(usize, usize)> = e.carets().iter().map(|c| (c.at.line, c.at.col)).collect();
        all.sort();
        all
    }

    #[test]
    fn carets_added_below_type_together_and_undo_as_one_step() {
        let mut e = Editor::open("a.rs".into(), "let a = 1;\nlet b = 2;\nlet c = 3;\n");
        at(&mut e, 0, 4);
        assert!(e.add_caret_vertical(1) && e.add_caret_vertical(1));
        assert!(!e.add_caret_vertical(1), "no line below the last one");
        assert_eq!(e.caret, Pos::new(2, 4), "the newest caret is the main one");
        assert_eq!(places(&e), vec![(0, 4), (1, 4), (2, 4)]);
        e.insert("my_");
        assert_eq!(e.display_source(), "let my_a = 1;\nlet my_b = 2;\nlet my_c = 3;");
        assert_eq!(places(&e), vec![(0, 7), (1, 7), (2, 7)]);
        e.backspace();
        e.newline();
        assert_eq!(e.display_source(), "let my\na = 1;\nlet my\nb = 2;\nlet my\nc = 3;");
        assert_eq!(places(&e), vec![(1, 0), (3, 0), (5, 0)]);
        assert!(e.undo());
        assert_eq!(e.display_source(), "let mya = 1;\nlet myb = 2;\nlet myc = 3;");
        assert_eq!(places(&e), vec![(0, 6), (1, 6), (2, 6)], "the carets come back with the text");
        assert!(e.undo() && e.undo());
        assert_eq!(e.display_source(), "let a = 1;\nlet b = 2;\nlet c = 3;");
        assert!(!e.dirty);
        assert!(!e.undo());
        assert!(e.redo());
        assert_eq!(e.display_source(), "let my_a = 1;\nlet my_b = 2;\nlet my_c = 3;");
        assert_eq!(places(&e), vec![(0, 7), (1, 7), (2, 7)]);
        assert!(e.single());
        assert_eq!(e.caret_count(), 1);
        assert!(!e.single(), "already one caret");
    }

    #[test]
    fn carets_on_one_line_keep_their_places_while_typing() {
        let mut e = Editor::open("a.txt".into(), "ab ab ab\n");
        at(&mut e, 0, 2);
        e.toggle_caret(Pos::new(0, 5));
        e.toggle_caret(Pos::new(0, 8));
        e.insert("é");
        assert_eq!(e.display_source(), "abé abé abé");
        assert_eq!(places(&e), vec![(0, 3), (0, 7), (0, 11)]);
        e.left(true);
        e.left(true);
        assert_eq!(e.selected_text(), "bé\nbé\nbé");
        e.delete_selected();
        assert_eq!(e.display_source(), "a a a");
        e.paste("1\n2\n3\n");
        assert_eq!(e.display_source(), "a1 a2 a3", "one line of what was copied for each caret");
        e.paste("xy");
        assert_eq!(e.display_source(), "a1xy a2xy a3xy");
        e.end(false);
        assert_eq!(places(&e), vec![(0, 14)], "carets that meet become one");
    }

    #[test]
    fn alt_click_adds_and_takes_away_carets() {
        let mut e = Editor::open("a.txt".into(), "one\ntwo\nthree\n");
        at(&mut e, 0, 1);
        e.toggle_caret(Pos::new(0, 1));
        assert_eq!(e.caret_count(), 1, "the only caret stays");
        e.toggle_caret(Pos::new(2, 3));
        assert_eq!(e.caret, Pos::new(2, 3));
        e.toggle_caret(Pos::new(1, 0));
        assert_eq!(places(&e), vec![(0, 1), (1, 0), (2, 3)]);
        e.toggle_caret(Pos::new(1, 0));
        assert_eq!(places(&e), vec![(0, 1), (2, 3)]);
        e.right(true);
        assert_eq!(e.selected_text(), "n\ne");
        e.toggle_caret(Pos::new(0, 2));
        assert_eq!(places(&e), vec![(2, 4)], "a click inside a selection takes its caret away");
        e.vertical(-1, false);
        e.toggle_caret(Pos::new(2, 0));
        e.backspace();
        assert_eq!(e.display_source(), "one\ntwthree");
        assert_eq!(places(&e), vec![(1, 2)], "two carets that meet become one");
        e.move_to(Pos::new(0, 0), false);
        e.toggle_caret(Pos::new(1, 0));
        e.vertical(1, true);
        assert_eq!(places(&e), vec![(1, 0)], "a selection that reaches another caret takes it in");
        e.insert("_");
        assert_eq!(e.display_source(), "_twthree");
    }

    #[test]
    fn next_occurrence_selects_the_word_then_finds_it_again() {
        let mut e = Editor::open("a.rs".into(), "let value = values(value);\nprint(value);\n");
        at(&mut e, 1, 8);
        assert!(e.add_next_occurrence());
        assert_eq!(e.selected_text(), "value");
        assert_eq!(e.caret_count(), 1);
        assert!(e.add_next_occurrence());
        assert_eq!(e.caret_count(), 2, "after the end of the file it starts again at the top");
        assert_eq!(e.selection(), Some((Pos::new(0, 4), Pos::new(0, 9))));
        assert!(e.add_next_occurrence());
        assert_eq!(e.selection(), Some((Pos::new(0, 19), Pos::new(0, 24))), "values is another word");
        assert!(!e.add_next_occurrence(), "every one is taken");
        e.insert("v");
        assert_eq!(e.display_source(), "let v = values(v);\nprint(v);");
        assert!(e.undo());
        assert_eq!(e.caret_count(), 3);
        e.single();
        at(&mut e, 0, 4);
        e.move_to(Pos::new(0, 7), true);
        assert!(e.select_all_occurrences());
        assert_eq!(e.caret_count(), 4, "a selection made by hand is found inside other words too");
        assert_eq!(e.selection(), Some((Pos::new(0, 4), Pos::new(0, 7))), "the main caret stays where it was");
        e.insert("V");
        assert_eq!(e.display_source(), "let Vue = Vues(Vue);\nprint(Vue);");
        let mut words = Editor::open("b.txt".into(), "x xx x\n");
        assert!(words.select_all_occurrences());
        assert_eq!(words.caret_count(), 2);
    }

    #[test]
    fn someone_elses_edit_moves_every_caret() {
        use crate::share::ot::TextOp;
        let before = "a\nb\nc";
        let mut e = Editor::open("a.txt".into(), before);
        at(&mut e, 0, 1);
        e.add_caret_vertical(1);
        e.add_caret_vertical(1);
        e.apply_remote(&TextOp::between(before, "zz\na\nb\nc"));
        assert_eq!(places(&e), vec![(1, 1), (2, 1), (3, 1)]);
        e.insert("!");
        assert_eq!(e.display_source(), "zz\na!\nb!\nc!");
    }

    #[test]
    fn every_line_keeps_its_own_end_and_new_lines_get_the_one_most_have() {
        for mixed in ["a\nb\nc\r\nd\n", "a\r\nb\r\nc\nd\r\n", "one line", "\u{feff}a\r\nb\n"] {
            assert_eq!(Editor::open("a".into(), mixed).text(), mixed);
        }
        let mut e = Editor::open("a".into(), "\u{feff}a\r\nb\r\nc\nd\r\n");
        assert_eq!(e.display_source(), "a\nb\nc\nd", "the mark at the start of the file is not text");
        at(&mut e, 0, 0);
        e.insert("X");
        at(&mut e, 2, 1);
        e.insert("\nnew");
        assert_eq!(e.text(), "\u{feff}Xa\r\nb\r\nc\r\nnew\nd\r\n");
        at(&mut e, 1, 1);
        e.move_to(Pos::new(2, 0), true);
        e.insert("");
        assert_eq!(e.text(), "\u{feff}Xa\r\nbc\r\nnew\nd\r\n", "two lines joined end as the second did");
    }

    #[test]
    fn replacing_every_place_is_one_step_of_undo() {
        let mut e = Editor::open("a".into(), "one two\nthree\ntwo two\n");
        at(&mut e, 1, 2);
        let places =
            [(Pos::new(0, 4), Pos::new(0, 7)), (Pos::new(2, 0), Pos::new(2, 3)), (Pos::new(2, 4), Pos::new(2, 7))];
        e.replace_each(&places, "2");
        assert_eq!(e.display_source(), "one 2\nthree\n2 2");
        assert_eq!(e.caret, Pos::new(1, 2));
        assert!(e.undo());
        assert_eq!(e.display_source(), "one two\nthree\ntwo two");
        assert!(!e.undo());
    }

    #[test]
    fn typing_and_undo() {
        let mut e = Editor::open("a.rs".into(), "fn main() {\n}\n");
        at(&mut e, 0, 11);
        e.newline();
        e.insert("    let x = 1;");
        assert_eq!(e.text(), "fn main() {\n    let x = 1;\n}\n");
        assert!(e.dirty);
        assert_eq!(e.caret, Pos::new(1, 14));
        e.backspace();
        assert_eq!(e.text(), "fn main() {\n    let x = 1\n}\n");
        assert!(e.undo());
        assert!(e.undo());
        assert!(e.undo());
        assert_eq!(e.text(), "fn main() {\n}\n");
        assert_eq!(e.caret, Pos::new(0, 11));
        assert!(!e.undo());
        assert!(!e.dirty, "undone back to what was read");
        assert!(e.redo());
        assert!(e.redo());
        assert_eq!(e.text(), "fn main() {\n    let x = 1;\n}\n");
        e.mark_saved();
        assert!(!e.dirty);
        assert!(e.undo());
        assert!(e.dirty, "one step before what was saved");
        e.insert("y");
        assert!(e.undo());
        assert!(e.redo());
        assert!(e.dirty, "the saved text cannot be reached again");
    }

    #[test]
    fn newline_keeps_indentation_and_joins_come_back() {
        let mut e = Editor::open("a.py".into(), "def f():\n    return 1\n");
        at(&mut e, 1, 12);
        e.newline();
        assert_eq!(e.caret, Pos::new(2, 4));
        e.insert("pass");
        assert_eq!(e.text(), "def f():\n    return 1\n    pass\n");
        at(&mut e, 2, 0);
        e.backspace();
        assert_eq!(e.text(), "def f():\n    return 1    pass\n");
        at(&mut e, 0, 8);
        e.delete();
        assert_eq!(e.text(), "def f():    return 1    pass\n");
    }

    #[test]
    fn selection_is_replaced() {
        let mut e = Editor::open("a.ts".into(), "const a = 1\nconst b = 2\nconst c = 3\n");
        at(&mut e, 0, 6);
        e.move_to(Pos::new(2, 6), true);
        assert_eq!(e.selected_text(), "a = 1\nconst b = 2\nconst ");
        e.insert("z");
        assert_eq!(e.text(), "const zc = 3\n");
        assert!(e.undo());
        assert_eq!(e.text(), "const a = 1\nconst b = 2\nconst c = 3\n");
        assert_eq!(e.selection(), Some((Pos::new(0, 6), Pos::new(2, 6))));
        at(&mut e, 1, 0);
        e.insert("x\r\ny\n");
        assert_eq!(e.text(), "const a = 1\nx\ny\nconst b = 2\nconst c = 3\n");
        assert_eq!(e.caret, Pos::new(3, 0));
    }

    #[test]
    fn tabs_and_display_columns() {
        let mut e = Editor::open("Makefile".into(), "all:\n\techo hi\n");
        assert_eq!(e.display_col(Pos::new(1, 1)), 4, "a tab reaches the next tab stop");
        assert_eq!(e.display_col(Pos::new(1, 3)), 6);
        assert_eq!(e.pos_at_display(1, 5), Pos::new(1, 2));
        assert_eq!(e.pos_at_display(1, 1), Pos::new(1, 0));
        assert_eq!(e.pos_at_display(1, 3), Pos::new(1, 1));
        assert_eq!(e.pos_at_display(1, 99), Pos::new(1, 8));
        at(&mut e, 1, 0);
        e.tab();
        assert_eq!(e.text(), "all:\n\t\techo hi\n");
        let mut s = Editor::open("a.ts".into(), "if (x) {\n  y()\n}\n");
        at(&mut s, 1, 5);
        s.tab();
        assert_eq!(s.text(), "if (x) {\n  y() \n}\n", "from column 5 to the next multiple of 2");
    }

    #[test]
    fn up_and_down_go_by_the_rows_of_a_wrapped_line() {
        let mut e = Editor::open("a.txt".into(), &format!("ab\n\t{}\ncd\n", "x".repeat(400)));
        let (text, starts, _) = canvas_core::scan::normalise(&e.display_source());
        let rows = canvas_core::Wrap::of(&text, &starts, 160).expect("the long line wraps");
        at(&mut e, 1, 1);
        e.vertical_in(1, false, Some(&rows));
        assert_eq!(e.caret, Pos::new(1, 157), "the next row of the same line, under the caret (a tab is 4 columns)");
        e.vertical_in(1, true, Some(&rows));
        assert_eq!(e.selection(), Some((Pos::new(1, 157), Pos::new(1, 311))));
        e.vertical_in(5, false, Some(&rows));
        assert_eq!(e.caret, Pos::new(2, 2), "past the rows of the line, the next line");
        e.vertical_in(-1, false, Some(&rows));
        assert_eq!(e.caret, Pos::new(1, 311), "back up to the last row, under the same place");
        e.vertical_in(-1, false, None);
        assert_eq!(e.caret.line, 0, "without rows a line is one step");
    }

    #[test]
    fn moving_around() {
        let mut e = Editor::open("a.rs".into(), "    let value = other_name;\nx\n");
        at(&mut e, 0, 27);
        e.vertical(1, false);
        assert_eq!(e.caret, Pos::new(1, 1), "clamped to the shorter line");
        e.vertical(-1, false);
        assert_eq!(e.caret, Pos::new(0, 27), "and back to the column it came from");
        e.home(false);
        assert_eq!(e.caret.col, 4);
        e.home(false);
        assert_eq!(e.caret.col, 0);
        e.word(true, false);
        assert_eq!(e.caret.col, 7);
        e.word(true, true);
        assert_eq!(e.selected_text(), " value");
        e.end(false);
        e.word(false, false);
        assert_eq!(e.caret.col, 16);
        e.select_word(Pos::new(0, 9));
        assert_eq!(e.selected_text(), "value");
        e.select_all();
        assert_eq!(e.selected_text(), "    let value = other_name;\nx");
        e.right(false);
        assert_eq!(e.caret, Pos::new(1, 1));
        e.right(false);
        assert_eq!(e.caret, Pos::new(1, 1), "the end of the text is the end");
    }
}
