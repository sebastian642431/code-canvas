use std::ops::Range;

use crate::layout::FOLD_LINES;

pub const WRAP_INDENT: u32 = 2;
const MOST_INDENT: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wrap {
    width: u32,
    per: u32,
    first: Box<[u32]>,
    breaks: Box<[[u32; 2]]>,
}

fn breaks_after(c: char) -> bool {
    c.is_whitespace() || matches!(c, ',' | ';')
}

fn breaks_of(line: &str, width: u32, out: &mut Vec<[u32; 2]>) {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len() as u32;
    if n <= width {
        return;
    }
    let lead = chars.iter().take_while(|c| **c == ' ').count() as u32;
    let indent = (lead + WRAP_INDENT).min(width / MOST_INDENT);
    let (mut start, mut room) = (0u32, width);
    while n - start > room {
        let hard = start + room;
        let soft = (start + room / 2 + 1..=hard).rev().find(|&k| breaks_after(chars[k as usize - 1]));
        start = soft.unwrap_or(hard);
        out.push([start, indent]);
        room = width - indent;
    }
}

impl Wrap {
    pub fn of(text: &str, line_starts: &[u32], width: u32) -> Option<Wrap> {
        let width = width.max(1);
        let lines = line_starts.len();
        let mut first = Vec::with_capacity(lines + 1);
        let mut breaks = Vec::new();
        for (i, &start) in line_starts.iter().enumerate() {
            first.push(breaks.len() as u32);
            let s = start as usize;
            let e = line_starts.get(i + 1).map_or(text.len(), |&e| e as usize - 1);
            if e.saturating_sub(s) > width as usize {
                breaks_of(&text[s..e], width, &mut breaks);
            }
        }
        first.push(breaks.len() as u32);
        if breaks.is_empty() {
            return None;
        }
        let mut wrap = Wrap { width, per: 1, first: first.into(), breaks: breaks.into() };
        wrap.per = (0..lines.div_ceil(FOLD_LINES)).map(|fold| wrap.column_rows(fold)).max().unwrap_or(1).max(1);
        Some(wrap)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn per(&self) -> u32 {
        self.per
    }

    pub fn lines(&self) -> usize {
        self.first.len() - 1
    }

    fn own(&self, line: usize) -> &[[u32; 2]] {
        &self.breaks[self.first[line] as usize..self.first[line + 1] as usize]
    }

    pub fn breaks(&self, line: usize) -> &[[u32; 2]] {
        match line < self.lines() {
            true => self.own(line),
            false => &[],
        }
    }

    pub fn rows(&self, line: usize) -> u32 {
        match line < self.lines() {
            true => 1 + self.own(line).len() as u32,
            false => 1,
        }
    }

    fn in_column(&self, line: usize) -> u32 {
        let base = line / FOLD_LINES * FOLD_LINES;
        (line - base) as u32 + self.first[line] - self.first[base]
    }

    pub fn column_rows(&self, fold: usize) -> u32 {
        let base = fold * FOLD_LINES;
        let lines = self.lines();
        if base >= lines {
            return 0;
        }
        let end = (base + FOLD_LINES).min(lines);
        (end - base) as u32 + self.first[end] - self.first[base]
    }

    pub fn row(&self, line: usize) -> u32 {
        let lines = self.lines();
        if line >= lines {
            let last = lines.saturating_sub(1);
            return self.row(last) + self.rows(last) + (line - lines) as u32;
        }
        (line / FOLD_LINES) as u32 * self.per + self.in_column(line)
    }

    pub fn place(&self, line: usize, col: u32) -> (u32, u32) {
        if line >= self.lines() {
            return (self.row(line), col);
        }
        let own = self.own(line);
        match own.partition_point(|b| b[0] <= col) {
            0 => (self.row(line), col),
            k => {
                let [start, indent] = own[k - 1];
                (self.row(line) + k as u32, indent + col - start)
            }
        }
    }

    pub fn pieces(&self, line: usize, col: u32, cols: u32) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        let own: &[[u32; 2]] = if line < self.lines() { self.own(line) } else { &[] };
        let (row, end) = (self.row(line), col.saturating_add(cols));
        let (first, last) = (own.partition_point(|b| b[0] <= col), own.partition_point(|b| b[0] < end));
        (first..=last).filter_map(move |k| {
            let [start, indent] = if k == 0 { [0, 0] } else { own[k - 1] };
            let stop = own.get(k).map_or(u32::MAX, |b| b[0]);
            let (a, b) = (col.max(start), end.min(stop));
            (a < b).then(|| (row + k as u32, indent + a - start, b - a))
        })
    }

    fn last_at_or_before(&self, base: usize, end: usize, row: u32) -> usize {
        let (mut lo, mut hi) = (base, end);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.in_column(mid) <= row {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.saturating_sub(1).max(base)
    }

    pub fn line_at(&self, vrow: u32) -> Option<(usize, u32)> {
        let (fold, row) = ((vrow / self.per) as usize, vrow % self.per);
        let (base, lines) = (fold * FOLD_LINES, self.lines());
        if base >= lines {
            return None;
        }
        let line = self.last_at_or_before(base, (base + FOLD_LINES).min(lines), row);
        let sub = row - self.in_column(line);
        (sub < self.rows(line)).then_some((line, sub))
    }

    pub fn col_at(&self, line: usize, sub: u32, vcol: u32) -> u32 {
        if line >= self.lines() {
            return vcol;
        }
        let own = self.own(line);
        let [start, indent] = match own.get((sub as usize).wrapping_sub(1)).or(own.last()) {
            Some(&row) if sub > 0 => row,
            _ => [0, 0],
        };
        let col = start + vcol.saturating_sub(indent);
        own.get(sub as usize).map_or(col, |next| col.min(next[0]))
    }

    pub fn lines_in(&self, fold: usize, rows: Range<u32>) -> Range<usize> {
        let (base, lines) = (fold * FOLD_LINES, self.lines());
        if base >= lines || rows.is_empty() {
            return 0..0;
        }
        let end = (base + FOLD_LINES).min(lines);
        let first = self.last_at_or_before(base, end, rows.start);
        let (mut lo, mut hi) = (first, end);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.in_column(mid) < rows.end {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        first..lo
    }

    pub fn step(&self, line: usize, col: u32, goal: Option<u32>, delta: isize) -> (usize, u32, u32) {
        let lines = self.lines();
        if lines == 0 {
            return (line, col, goal.unwrap_or(col));
        }
        let line = line.min(lines - 1);
        let own = self.own(line);
        let mut sub = own.partition_point(|b| b[0] <= col) as u32;
        let goal = goal.unwrap_or_else(|| self.place(line, col).1);
        let mut line = line;
        for _ in 0..delta.unsigned_abs() {
            if delta > 0 {
                if sub + 1 < self.rows(line) {
                    sub += 1;
                } else if line + 1 < lines {
                    (line, sub) = (line + 1, 0);
                } else {
                    break;
                }
            } else if sub > 0 {
                sub -= 1;
            } else if line > 0 {
                line -= 1;
                sub = self.rows(line) - 1;
            } else {
                break;
            }
        }
        (line, self.col_at(line, sub, goal), goal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(text: &str, width: u32) -> Option<Wrap> {
        let (text, starts, _) = crate::scan::normalise(text);
        Wrap::of(&text, &starts, width)
    }

    #[test]
    fn short_lines_need_no_wrap() {
        assert_eq!(of("abc\ndef\n", 10), None);
        assert_eq!(of("", 10), None);
        assert_eq!(of("ñandú ñandú", 11), None, "columns are characters, not bytes");
    }

    #[test]
    fn a_long_line_goes_on_in_rows_under_it() {
        let w = of("short\n0123456789abcdefghijklmnopqrst\nend\n", 10).unwrap();
        assert_eq!(w.lines(), 3);
        assert_eq!((w.rows(0), w.rows(1), w.rows(2)), (1, 4, 1), "10 columns, then 8 a row after the indent");
        assert_eq!((w.row(0), w.row(1), w.row(2)), (0, 1, 5));
        assert_eq!(w.per(), 6);
        assert_eq!(w.place(1, 0), (1, 0));
        assert_eq!(w.place(1, 9), (1, 9));
        assert_eq!(w.place(1, 10), (2, 2), "continues indented");
        assert_eq!(w.place(1, 17), (2, 9));
        assert_eq!(w.place(1, 18), (3, 2));
        assert_eq!(w.place(1, 30), (4, 6), "the end of the line, after its last character");
        assert_eq!(w.place(2, 1), (5, 1));
    }

    #[test]
    fn rows_go_back_to_lines_and_columns() {
        let w = of("short\n0123456789abcdefghijklmnopqrst\nend\n", 10).unwrap();
        assert_eq!(w.line_at(0), Some((0, 0)));
        assert_eq!(w.line_at(1), Some((1, 0)));
        assert_eq!(w.line_at(3), Some((1, 2)));
        assert_eq!(w.line_at(5), Some((2, 0)));
        assert_eq!(w.line_at(6), None);
        assert_eq!(w.col_at(1, 0, 4), 4);
        assert_eq!(w.col_at(1, 0, 15), 10, "past a row's end is the start of the next");
        assert_eq!(w.col_at(1, 1, 0), 10, "the indent belongs to the first character of the row");
        assert_eq!(w.col_at(1, 1, 5), 13);
        assert_eq!(w.col_at(1, 3, 40), 64, "the last row is as long as the line wants");
        for col in 0..=30 {
            let (row, vcol) = w.place(1, col);
            let (line, sub) = w.line_at(row).unwrap();
            assert_eq!((line, w.col_at(line, sub, vcol)), (1, col), "column {col} there and back");
        }
    }

    #[test]
    fn rows_break_after_a_blank_when_there_is_one() {
        let w = of("aaaa bbbb cccc dddd eeee", 12).unwrap();
        assert_eq!(w.rows(0), 3);
        assert_eq!(w.place(0, 10), (1, 2), "\"cccc\" is not cut in two");
        assert_eq!(w.place(0, 20), (2, 2));
        let indented = of("    aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 40).unwrap();
        assert_eq!(indented.place(0, 40), (1, 6), "under the line's own indent, two further in");
    }

    #[test]
    fn spans_are_cut_into_one_piece_per_row() {
        let w = of("0123456789abcdefghijklmnopqrst", 10).unwrap();
        let pieces = |col, cols| w.pieces(0, col, cols).collect::<Vec<_>>();
        assert_eq!(pieces(2, 3), vec![(0, 2, 3)]);
        assert_eq!(pieces(8, 4), vec![(0, 8, 2), (1, 2, 2)]);
        assert_eq!(pieces(0, 31), vec![(0, 0, 10), (1, 2, 8), (2, 2, 8), (3, 2, 5)]);
        assert_eq!(pieces(12, 3), vec![(1, 4, 3)]);
        assert_eq!(pieces(30, 1), vec![(3, 6, 1)], "the cell after the end of the line");
        assert_eq!(pieces(5, 0), vec![]);
        assert_eq!(w.pieces(7, 1, 2).collect::<Vec<_>>(), vec![(w.row(7), 1, 2)], "past the last line");
    }

    #[test]
    fn rows_seen_give_the_lines_drawn() {
        let w = of("a\n0123456789abcdefghijklmnopqrst\nb\nc\n", 10).unwrap();
        assert_eq!(w.lines_in(0, 0..1), 0..1);
        assert_eq!(w.lines_in(0, 2..4), 1..2, "rows inside a wrapped line draw that line");
        assert_eq!(w.lines_in(0, 3..7), 1..4);
        assert_eq!(w.lines_in(0, 0..100), 0..4);
        assert_eq!(w.lines_in(1, 0..10), 0..0);
    }

    #[test]
    fn up_and_down_walk_the_rows() {
        let w = of("short\n0123456789abcdefghijklmnopqrst\nend\n", 10).unwrap();
        let (line, col, goal) = w.step(0, 3, None, 1);
        assert_eq!((line, col, goal), (1, 3, 3));
        let (line, col, goal) = w.step(line, col, Some(goal), 1);
        assert_eq!((line, col), (1, 11), "the next row of the same line, same place on screen");
        let (line, col, _) = w.step(line, col, Some(goal), 3);
        assert_eq!((line, col), (2, 3));
        let (line, col, _) = w.step(line, col, Some(goal), -1);
        assert_eq!((line, col), (1, 27));
        assert_eq!(w.step(0, 2, None, -1), (0, 2, 2), "nothing above the first row");
    }

    #[test]
    fn folded_columns_count_rows_of_their_own_lines() {
        let mut text = String::new();
        for i in 0..FOLD_LINES + 3 {
            text.push_str(if i == 1 || i == FOLD_LINES + 1 { "0123456789abcdefghij\n" } else { "x\n" });
        }
        let w = of(&text, 10).unwrap();
        assert_eq!(w.column_rows(0), FOLD_LINES as u32 + 2);
        assert_eq!(w.column_rows(1), 3 + 2);
        assert_eq!(w.per(), FOLD_LINES as u32 + 2);
        assert_eq!(w.row(FOLD_LINES), w.per(), "the second column starts at its own top");
        assert_eq!(w.row(FOLD_LINES + 2), w.per() + 4);
        assert_eq!(w.line_at(w.per() + 4), Some((FOLD_LINES + 2, 0)));
        assert_eq!(w.line_at(w.per() + 6), None, "below the shorter column is nothing");
    }
}
