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

struct Change {
    at: Pos,
    removed: String,
    inserted: String,
    caret_before: Pos,
    anchor_before: Option<Pos>,
}

pub struct Editor {
    pub rel_path: String,
    lines: Vec<String>,
    eol: &'static str,
    final_newline: bool,
    pub caret: Pos,
    pub anchor: Option<Pos>,
    goal: Option<usize>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    saved_at: Option<usize>,
    pub dirty: bool,
}

fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

impl Editor {
    pub fn open(rel_path: String, source: &str) -> Editor {
        let eol = if source.contains("\r\n") { "\r\n" } else { "\n" };
        let final_newline = source.ends_with('\n');
        let body = source.strip_suffix('\n').unwrap_or(source);
        let lines: Vec<String> = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
        Editor {
            rel_path,
            lines,
            eol,
            final_newline,
            caret: Pos::default(),
            anchor: None,
            goal: None,
            undo: Vec::new(),
            redo: Vec::new(),
            saved_at: Some(0),
            dirty: false,
        }
    }

    pub fn text(&self) -> String {
        let mut out = self.lines.join(self.eol);
        if self.final_newline {
            out.push_str(self.eol);
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
        self.selection().map(|(a, b)| self.range_text(a, b)).unwrap_or_default()
    }

    fn splice(&mut self, start: Pos, end: Pos, text: &str) -> Pos {
        let head = self.lines[start.line][..byte_at(&self.lines[start.line], start.col)].to_string();
        let tail = self.lines[end.line][byte_at(&self.lines[end.line], end.col)..].to_string();
        let mut new: Vec<String> = text.split('\n').map(str::to_string).collect();
        let last = new.len() - 1;
        let end_pos = Pos::new(start.line + last, if last == 0 { start.col } else { 0 } + new[last].chars().count());
        new[0].insert_str(0, &head);
        new[last].push_str(&tail);
        self.lines.splice(start.line..=end.line, new);
        end_pos
    }

    fn replace(&mut self, start: Pos, end: Pos, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let removed = self.range_text(start, end);
        if removed.is_empty() && text.is_empty() {
            return;
        }
        let change =
            Change { at: start, removed, inserted: text, caret_before: self.caret, anchor_before: self.anchor };
        self.caret = self.splice(start, end, &change.inserted);
        self.anchor = None;
        self.goal = None;
        if self.saved_at.is_some_and(|depth| depth > self.undo.len()) {
            self.saved_at = None;
        }
        self.undo.push(change);
        self.redo.clear();
        self.touch();
    }

    fn touch(&mut self) {
        self.dirty = self.saved_at != Some(self.undo.len());
    }

    fn end_of(at: Pos, text: &str) -> Pos {
        match text.rsplit_once('\n') {
            Some((before, last)) => Pos::new(at.line + before.matches('\n').count() + 1, last.chars().count()),
            None => Pos::new(at.line, at.col + text.chars().count()),
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(change) = self.undo.pop() else { return false };
        self.splice(change.at, Self::end_of(change.at, &change.inserted), &change.removed);
        self.caret = change.caret_before;
        self.anchor = change.anchor_before;
        self.goal = None;
        self.redo.push(change);
        self.touch();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(change) = self.redo.pop() else { return false };
        self.caret = self.splice(change.at, Self::end_of(change.at, &change.removed), &change.inserted);
        self.anchor = None;
        self.goal = None;
        self.undo.push(change);
        self.touch();
        true
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    pub fn complete(&mut self, from: usize, text: &str) {
        let end = self.caret;
        self.replace(Pos::new(end.line, from.min(end.col)), end, text);
    }

    pub fn insert(&mut self, text: &str) {
        let (start, end) = self.selection().unwrap_or((self.caret, self.caret));
        self.replace(start, end, text);
    }

    pub fn newline(&mut self) {
        let line = &self.lines[self.caret.line];
        let indent: String = line.chars().take(self.caret.col).take_while(|c| *c == ' ' || *c == '\t').collect();
        self.insert(&format!("\n{indent}"));
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
        let col = self.display_col(self.selection().map_or(self.caret, |(a, _)| a));
        self.insert(&" ".repeat(step - col % step));
    }

    pub fn backspace(&mut self) {
        if let Some((a, b)) = self.selection() {
            return self.replace(a, b, "");
        }
        let c = self.caret;
        if c.col > 0 {
            self.replace(Pos::new(c.line, c.col - 1), c, "");
        } else if c.line > 0 {
            self.replace(Pos::new(c.line - 1, self.line_len(c.line - 1)), c, "");
        }
    }

    pub fn delete(&mut self) {
        if let Some((a, b)) = self.selection() {
            return self.replace(a, b, "");
        }
        let c = self.caret;
        if c.col < self.line_len(c.line) {
            self.replace(c, Pos::new(c.line, c.col + 1), "");
        } else if c.line + 1 < self.lines.len() {
            self.replace(c, Pos::new(c.line + 1, 0), "");
        }
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
    }

    pub fn left(&mut self, select: bool) {
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
        let goal = self.goal.unwrap_or_else(|| self.display_col(self.caret));
        let line = (self.caret.line as isize + delta).clamp(0, self.lines.len() as isize - 1) as usize;
        let p = self.pos_at_display(line, goal);
        self.move_to(p, select);
        self.goal = Some(goal);
    }

    pub fn home(&mut self, select: bool) {
        let line = &self.lines[self.caret.line];
        let first = line.chars().take_while(|c| c.is_whitespace()).count();
        let col = if self.caret.col == first { 0 } else { first };
        self.move_to(Pos::new(self.caret.line, col), select);
    }

    pub fn end(&mut self, select: bool) {
        self.move_to(Pos::new(self.caret.line, self.line_len(self.caret.line)), select);
    }

    pub fn word(&mut self, forward: bool, select: bool) {
        let chars: Vec<char> = self.lines[self.caret.line].chars().collect();
        let mut col = self.caret.col;
        if forward {
            if col >= chars.len() {
                return self.right(select);
            }
            while col < chars.len() && !is_word(chars[col]) {
                col += 1;
            }
            while col < chars.len() && is_word(chars[col]) {
                col += 1;
            }
        } else {
            if col == 0 {
                return self.left(select);
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
        self.anchor = Some(Pos::default());
        self.caret = Pos::new(last, self.line_len(last));
    }

    pub fn select_word(&mut self, p: Pos) {
        let p = self.clamp(p);
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        let (mut a, mut b) = (p.col.min(chars.len()), p.col.min(chars.len()));
        while a > 0 && is_word(chars[a - 1]) {
            a -= 1;
        }
        while b < chars.len() && is_word(chars[b]) {
            b += 1;
        }
        self.anchor = Some(Pos::new(p.line, a));
        self.caret = Pos::new(p.line, b);
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo.len());
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(e: &mut Editor, line: usize, col: usize) {
        e.move_to(Pos::new(line, col), false);
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
