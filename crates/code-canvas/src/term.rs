use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const KEPT: usize = 5000;
pub const GIT_PAGE: &str = "https://git-scm.com/downloads";

pub fn bash() -> Option<PathBuf> {
    let name = if cfg!(windows) { "bash.exe" } else { "bash" };
    let mut places: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            let Some(base) = std::env::var_os(base).map(PathBuf::from) else { continue };
            places.push(base.join("Git").join("bin").join(name));
            places.push(base.join("Programs").join("Git").join("bin").join(name));
        }
    } else {
        places.extend(["/bin", "/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"].map(|dir| Path::new(dir).join(name)));
    }
    let not_a_shell = |dir: &Path| {
        let dir = dir.to_string_lossy().to_lowercase();
        dir.contains("\\windows\\system32") || dir.contains("\\windowsapps")
    };
    if let Some(path) = std::env::var_os("PATH") {
        places.extend(std::env::split_paths(&path).filter(|dir| !not_a_shell(dir)).map(|dir| dir.join(name)));
    }
    places.into_iter().find(|place| place.is_file())
}

fn whole(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap_or(NonZeroU16::MIN)
}

/// Counts the rows that fell off the top: of the scrollback when it is full, of the screen itself on the alternate
/// screen (which keeps none). With it a line keeps one number for as long as it is anywhere ([`Spot::line`]).
#[derive(Default)]
pub struct Dropped {
    main: usize,
    alternate: usize,
}

impl vt100::Callbacks for Dropped {
    fn on_scroll(&mut self, _: vt100::capture::RowContents<'_>, alternate: bool) {
        match alternate {
            true => self.alternate += 1,
            false => self.main += 1,
        }
    }
}

pub struct Terminal {
    parser: vt100::Parser<Dropped>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    heard: Receiver<Vec<u8>>,
    size: (u16, u16),
    pub ended: bool,
    /// What the mouse selected. It stays on its lines while more output comes, and goes with a resize, a switch to
    /// or from the alternate screen, a cleared scrollback, or once all of it fell off the top.
    pub selection: Option<Selection>,
}

impl Terminal {
    pub fn open(shell: &Path, dir: &Path, rows: u16, cols: u16) -> Result<Terminal, String> {
        let (rows, cols) = (rows.max(2), cols.max(8));
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;
        let mut command = CommandBuilder::new(shell);
        command.args(["--login", "-i"]);
        if dir.is_dir() {
            command.cwd(dir);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("CHERE_INVOKING", "1");
        let child = pair.slave.spawn_command(command).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let (said, heard) = mpsc::channel();
        let read = move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 || said.send(chunk[..n].to_vec()).is_err() {
                    break;
                }
                crate::ext::wake_ui();
            }
            crate::ext::wake_ui();
        };
        std::thread::Builder::new().name("console".into()).spawn(read).map_err(|e| e.to_string())?;
        log::info!("console: {} in {}", shell.display(), dir.display());
        Ok(Terminal {
            parser: vt100::Parser::new_with_callbacks(whole(rows), whole(cols), KEPT, Dropped::default()),
            writer,
            master: pair.master,
            child,
            heard,
            size: (rows, cols),
            ended: false,
            selection: None,
        })
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let (bottom, alternate) = (self.bottom(), self.screen().alternate_screen());
        while let Ok(bytes) = self.heard.try_recv() {
            self.parser.process(&bytes);
            changed = true;
            let asks = |what: &[u8]| bytes.windows(what.len()).any(|part| part == what);
            if asks(b"\x1b[6n") {
                let (row, col) = self.parser.screen().cursor_position();
                self.answer(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            if asks(b"\x1b[c") || asks(b"\x1b[0c") {
                self.answer(b"\x1b[?1;2c");
            }
        }
        if !self.ended && matches!(self.child.try_wait(), Ok(Some(_))) {
            self.ended = true;
            changed = true;
        }
        let gone = self.selection.is_some_and(|selection| selection.anchor.max(selection.head).line < self.first());
        if self.bottom() < bottom || self.screen().alternate_screen() != alternate || gone {
            self.selection = None;
        }
        changed
    }

    /// The number of the first line still kept (of the scrollback, or of the screen when there is none).
    fn first(&self) -> usize {
        let dropped = self.parser.callbacks();
        match self.screen().alternate_screen() {
            true => dropped.alternate,
            false => dropped.main,
        }
    }

    /// The number of the first line of the screen the shell writes on: lines only ever move up past it.
    fn bottom(&self) -> usize {
        self.first() + self.screen().scrollback_len()
    }

    /// The number of the line shown at the top, scrolled back or not.
    pub fn top(&self) -> usize {
        self.bottom() - self.screen().scrollback()
    }

    pub fn grid(&mut self) -> Lines<'_> {
        let first = self.first();
        Lines { screen: self.parser.screen_mut(), first }
    }

    /// From where to where the selection goes, when it has anything in it.
    pub fn selected(&mut self) -> Option<(Spot, Spot)> {
        let selection = self.selection?;
        let (from, to) = selection.range(&mut self.grid());
        (from < to).then_some((from, to))
    }

    /// The text of the selection, as it is copied.
    pub fn selected_text(&mut self) -> String {
        match self.selected() {
            Some((from, to)) => text(&mut self.grid(), from, to),
            None => String::new(),
        }
    }

    fn answer(&mut self, bytes: &[u8]) {
        if self.writer.write_all(bytes).and_then(|_| self.writer.flush()).is_err() {
            self.ended = true;
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        if self.ended || bytes.is_empty() {
            return;
        }
        self.parser.screen_mut().set_scrollback(0);
        self.answer(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(8));
        if (rows, cols) == self.size {
            return;
        }
        self.size = (rows, cols);
        self.selection = None;
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.parser.set_size(whole(rows), whole(cols));
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn scroll(&mut self, rows: isize) {
        let now = self.parser.screen().scrollback() as isize;
        self.parser.screen_mut().set_scrollback((now + rows).max(0) as usize);
    }
}

/// A place between two cells: `line` counts every line since the shell started, the ones that fell off the top too,
/// so a place stays on its text while more output pushes it up; `col` is the cells before it on that line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub line: usize,
    pub col: u16,
}

impl Spot {
    pub fn new(line: usize, col: u16) -> Spot {
        Spot { line, col }
    }
}

/// What one, two or three clicks select, and what a drag after them grows by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Cell,
    Word,
    Line,
}

/// From where the button went down (`anchor`) to where the pointer is (`head`). With [`Unit::Cell`] both are places
/// between cells; with the others they are the cell under the pointer, grown to its word or its whole line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Spot,
    pub head: Spot,
    pub unit: Unit,
}

impl Selection {
    pub fn new(at: Spot, unit: Unit) -> Selection {
        Selection { anchor: at, head: at, unit }
    }

    /// Its start and end in order: the place before its first cell and the one after its last.
    pub fn range(&self, grid: &mut impl Grid) -> (Spot, Spot) {
        let (low, high) = (self.anchor.min(self.head), self.anchor.max(self.head));
        match self.unit {
            Unit::Cell => (low, high),
            Unit::Word => {
                let (a, b) = (word_at(grid, self.anchor), word_at(grid, self.head));
                (a.0.min(b.0), a.1.max(b.1))
            }
            Unit::Line => {
                let (start, _) = whole_line(grid, low.line);
                let (_, end) = whole_line(grid, high.line);
                (Spot::new(start, 0), Spot::new(end, grid.cols()))
            }
        }
    }
}

/// One cell of a line of the console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Empty,
    Text(String),
    /// The right half of a wide letter that starts in the cell before.
    Rest,
}

/// One line of the console: its cells, and whether it goes on in the next one (it was too long and wrapped).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

/// The lines of a console by their number ([`Spot::line`]), scrollback and screen together.
pub trait Grid {
    fn cols(&self) -> u16;
    /// `None` for a line that fell off the top or is not there yet.
    fn row(&mut self, line: usize) -> Option<Row>;
}

pub struct Lines<'a> {
    screen: &'a mut vt100::Screen,
    first: usize,
}

impl Grid for Lines<'_> {
    fn cols(&self) -> u16 {
        self.screen.size().1.get()
    }

    fn row(&mut self, line: usize) -> Option<Row> {
        let index = line.checked_sub(self.first)?;
        let (kept, rows, cols) = (self.screen.scrollback_len(), self.screen.size().0.get(), self.cols());
        let (back, at) = match index.checked_sub(kept) {
            None => (kept - index, 0),
            Some(on_screen) if on_screen < usize::from(rows) => (0, on_screen as u16),
            Some(_) => return None,
        };
        // vt100 reads the scrollback only through what is in view: look there for a moment.
        let was = self.screen.scrollback();
        self.screen.set_scrollback(back);
        let cells = (0..cols)
            .map(|col| match self.screen.cell(at, col) {
                Some(cell) if cell.is_wide_continuation() => Cell::Rest,
                Some(cell) if cell.has_contents() => Cell::Text(cell.contents().to_string()),
                _ => Cell::Empty,
            })
            .collect();
        let row = Row { cells, wrapped: self.screen.row_wrapped(at) };
        self.screen.set_scrollback(was);
        Some(row)
    }
}

/// The text between two places as it is copied: a line that wrapped goes on without a break, the spaces at the end
/// of each line are left out, and so are the empty lines at the end.
pub fn text(grid: &mut impl Grid, from: Spot, to: Spot) -> String {
    let cols = grid.cols();
    let mut said = String::new();
    for line in from.line..=to.line {
        let Some(row) = grid.row(line) else { continue };
        let start = if line == from.line { from.col } else { 0 };
        let end = if line == to.line { to.col } else { cols };
        let mut piece = String::new();
        for cell in row.cells.iter().take(usize::from(end)).skip(usize::from(start)) {
            match cell {
                Cell::Empty => piece.push(' '),
                Cell::Text(text) => piece.push_str(text),
                Cell::Rest => {}
            }
        }
        if line != to.line && row.wrapped {
            said.push_str(&piece);
        } else {
            said.push_str(piece.trim_end());
            if line != to.line {
                said.push('\n');
            }
        }
    }
    said.truncate(said.trim_end().len());
    said
}

/// What a double click takes together: letters, digits and what paths and addresses are made of (`/`, `.`, `-`,
/// `:`...) are a word (1); spaces go with spaces (0); a bracket, a quote or a comma is one alone (2).
fn kind(row: &Row, col: usize) -> u8 {
    const ALONE: &str = "()[]{}<>'\"`,;|│";
    match row.cells.get(col) {
        None | Some(Cell::Empty) => 0,
        Some(Cell::Rest) if col == 0 => 1,
        Some(Cell::Rest) => kind(row, col - 1),
        Some(Cell::Text(text)) if text.chars().all(char::is_whitespace) => 0,
        Some(Cell::Text(text)) if text.chars().count() == 1 && ALONE.contains(text.as_str()) => 2,
        Some(Cell::Text(_)) => 1,
    }
}

/// The word of the cell at `at`, from the place before it to the one after; a word goes on across a wrapped line.
pub fn word_at(grid: &mut impl Grid, at: Spot) -> (Spot, Spot) {
    let cols = usize::from(grid.cols());
    let Some(row) = grid.row(at.line) else { return (at, at) };
    let col = usize::from(at.col).min(cols.saturating_sub(1));
    let wanted = kind(&row, col);
    if wanted == 2 {
        return (Spot::new(at.line, col as u16), Spot::new(at.line, col as u16 + 1));
    }
    let crosses = wanted == 1;
    let (mut start, mut line, mut here) = (col, at.line, row.clone());
    loop {
        while start > 0 && kind(&here, start - 1) == wanted {
            start -= 1;
        }
        let up = match start == 0 && crosses {
            true => line.checked_sub(1).and_then(|up| grid.row(up)),
            false => None,
        };
        match up {
            Some(up) if up.wrapped && kind(&up, cols - 1) == wanted => (line, start, here) = (line - 1, cols, up),
            _ => break,
        }
    }
    let from = Spot::new(line, start as u16);
    let (mut end, mut line, mut here) = (col + 1, at.line, row);
    loop {
        while end < cols && kind(&here, end) == wanted {
            end += 1;
        }
        let down = match end == cols && crosses && here.wrapped {
            true => grid.row(line + 1),
            false => None,
        };
        match down {
            Some(down) if kind(&down, 0) == wanted => (line, end, here) = (line + 1, 0, down),
            _ => break,
        }
    }
    (from, Spot::new(line, end as u16))
}

/// The first and the last line of the line `line` is part of, when a long one wrapped over several.
pub fn whole_line(grid: &mut impl Grid, line: usize) -> (usize, usize) {
    let mut start = line;
    while start > 0 && grid.row(start - 1).is_some_and(|up| up.wrapped) {
        start -= 1;
    }
    let mut end = line;
    while grid.row(end).is_some_and(|row| row.wrapped) && grid.row(end + 1).is_some() {
        end += 1;
    }
    (start, end)
}

/// The clipboard. A scripted run (the scenarios of `qa/`) keeps one of its own, so that a test never reads or
/// replaces what the person at the computer copied.
static OWN_CLIPBOARD: Mutex<Option<String>> = Mutex::new(None);

fn scripted() -> bool {
    std::env::var_os("CODE_CANVAS_SCRIPT").is_some()
}

pub fn copy(text: &str) {
    if scripted() {
        *OWN_CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
        return;
    }
    if let Err(e) = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.to_string())) {
        log::warn!("console: the selection could not be copied: {e}");
    }
}

pub fn clipboard() -> Option<String> {
    if scripted() {
        return OWN_CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()).clone();
    }
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()).ok()
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Default)]
pub struct Console {
    pub term: Option<Terminal>,
    pub shell: Option<PathBuf>,
    pub looked: bool,
    pub failed: Option<String>,
    pub focus: bool,
    pub wheel: f32,
    pub mouse: Mouse,
    installing: Option<Receiver<Result<(), String>>>,
}

/// The pointer over the console's screen, from one frame to the next.
#[derive(Default)]
pub struct Mouse {
    /// The left button went down on the screen and is not up yet.
    pub dragging: bool,
    /// One, two or three clicks in a row on the same cell.
    pub clicks: u8,
    pub last: f64,
    pub at: Option<Spot>,
    /// Where the first cell of the screen was drawn and the size of a cell, in points: what the script's `term`
    /// steps aim at.
    pub place: Option<(egui::Pos2, egui::Vec2)>,
}

impl Console {
    pub fn look(&mut self) {
        self.shell = bash();
        self.looked = true;
    }

    pub fn start(&mut self, dir: &Path) {
        if !self.looked || self.shell.is_none() {
            self.look();
        }
        let Some(shell) = self.shell.clone() else { return };
        let (rows, cols) = self.term.as_ref().map_or((24, 80), Terminal::size);
        self.term = None;
        match Terminal::open(&shell, dir, rows, cols) {
            Ok(term) => {
                self.term = Some(term);
                self.failed = None;
                self.focus = true;
            }
            Err(why) => {
                log::warn!("console: {} did not start: {why}", shell.display());
                self.failed = Some(why);
            }
        }
    }

    pub fn installing(&self) -> bool {
        self.installing.is_some()
    }

    pub fn install(&mut self) {
        if self.installing.is_some() {
            return;
        }
        let (done, heard) = mpsc::channel();
        self.installing = Some(heard);
        self.failed = None;
        std::thread::spawn(move || {
            let mut winget = std::process::Command::new("winget");
            winget.args(["install", "--id", "Git.Git", "-e", "--source", "winget"]);
            winget.args(["--accept-source-agreements", "--accept-package-agreements"]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                winget.creation_flags(0x0800_0000);
            }
            let ran = winget.stdin(std::process::Stdio::null()).output();
            let result = match ran {
                Ok(out) if out.status.success() => Ok(()),
                Ok(out) => {
                    let said = String::from_utf8_lossy(&out.stdout);
                    let last = said.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("it did not say why");
                    Err(format!("winget did not install Git: {}", last.trim()))
                }
                Err(e) => Err(format!("winget could not be run ({e})")),
            };
            let _ = done.send(result);
            crate::ext::wake_ui();
        });
    }

    pub fn poll(&mut self) -> Option<Result<(), String>> {
        if let Some(term) = self.term.as_mut() {
            term.poll();
        }
        let done = self.installing.as_ref().and_then(|heard| heard.try_recv().ok())?;
        self.installing = None;
        self.look();
        Some(match (&done, &self.shell) {
            (Ok(()), None) => {
                Err("Git was installed but bash is not where it is looked for: start the app again.".into())
            }
            _ => done,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A console of `rows` × `cols` keeping `kept` lines of scrollback that was sent `said`.
    fn console(rows: u16, cols: u16, kept: usize, said: &str) -> vt100::Parser<Dropped> {
        let mut parser = vt100::Parser::new_with_callbacks(whole(rows), whole(cols), kept, Dropped::default());
        parser.process(said.as_bytes());
        parser
    }

    fn lines(parser: &mut vt100::Parser<Dropped>) -> Lines<'_> {
        let first = parser.callbacks().main;
        Lines { screen: parser.screen_mut(), first }
    }

    fn chosen(parser: &mut vt100::Parser<Dropped>, selection: Selection) -> String {
        let mut grid = lines(parser);
        let (from, to) = selection.range(&mut grid);
        text(&mut grid, from, to)
    }

    fn dragged(from: (usize, u16), to: (usize, u16)) -> Selection {
        Selection { anchor: Spot::new(from.0, from.1), head: Spot::new(to.0, to.1), unit: Unit::Cell }
    }

    fn clicked(at: (usize, u16), unit: Unit) -> Selection {
        Selection::new(Spot::new(at.0, at.1), unit)
    }

    #[test]
    fn a_selection_copies_its_lines_with_wrapped_ones_joined_and_no_spaces_at_the_ends() {
        // Row 1 is too long for ten columns and goes on in row 2.
        let mut parser = console(6, 10, 100, "hello   \r\nabcdefghijklmno\r\nx  y\r\n");
        assert!(parser.screen().row_wrapped(1) && !parser.screen().row_wrapped(2));
        assert_eq!(chosen(&mut parser, dragged((0, 0), (3, 10))), "hello\nabcdefghijklmno\nx  y");
        assert_eq!(chosen(&mut parser, dragged((0, 2), (1, 3))), "llo\nabc");
        assert_eq!(chosen(&mut parser, dragged((1, 3), (0, 2))), "llo\nabc", "dragged upwards");
        assert_eq!(chosen(&mut parser, dragged((1, 8), (2, 2))), "ijkl", "across the wrap, no line break");
        assert_eq!(chosen(&mut parser, dragged((0, 5), (0, 8))), "", "only spaces");
        assert_eq!(chosen(&mut parser, dragged((2, 0), (5, 10))), "klmno\nx  y", "empty lines at the end left out");
        assert_eq!(chosen(&mut parser, dragged((3, 1), (3, 1))), "", "a click alone selects nothing");
    }

    #[test]
    fn a_double_click_takes_a_word_with_its_path_and_a_bracket_alone() {
        let mut parser = console(4, 40, 100, "see (src/term.rs:42), ok\r\n");
        assert_eq!(chosen(&mut parser, clicked((0, 8), Unit::Word)), "src/term.rs:42");
        assert_eq!(chosen(&mut parser, clicked((0, 4), Unit::Word)), "(");
        assert_eq!(chosen(&mut parser, clicked((0, 1), Unit::Word)), "see");
        assert_eq!(chosen(&mut parser, clicked((0, 23), Unit::Word)), "ok");
        let both = Selection { anchor: Spot::new(0, 1), head: Spot::new(0, 8), unit: Unit::Word };
        assert_eq!(chosen(&mut parser, both), "see (src/term.rs:42", "dragging after a double click grows by words");
        // A word longer than the line goes on in the next one.
        let mut parser = console(4, 10, 100, "aaa bbbbbbbbbbbb cc\r\n");
        let mut grid = lines(&mut parser);
        assert_eq!(word_at(&mut grid, Spot::new(1, 2)), (Spot::new(0, 4), Spot::new(1, 6)));
        assert_eq!(chosen(&mut parser, clicked((0, 6), Unit::Word)), "bbbbbbbbbbbb");
    }

    #[test]
    fn a_triple_click_takes_the_whole_line_even_when_it_wrapped() {
        let mut parser = console(5, 10, 100, "first\r\nabcdefghijklmno\r\nlast\r\n");
        assert_eq!(chosen(&mut parser, clicked((2, 1), Unit::Line)), "abcdefghijklmno");
        assert_eq!(chosen(&mut parser, clicked((0, 3), Unit::Line)), "first");
        let down = Selection { anchor: Spot::new(0, 3), head: Spot::new(3, 0), unit: Unit::Line };
        assert_eq!(chosen(&mut parser, down), "first\nabcdefghijklmno\nlast");
    }

    #[test]
    fn a_selection_reaches_into_the_scrollback_and_keeps_its_lines_as_they_fall_off() {
        let said: String = (0..10).map(|n| format!("line{n}\r\n")).collect();
        let mut parser = console(3, 10, 100, &said);
        assert_eq!(parser.screen().scrollback_len(), 8);
        parser.screen_mut().set_scrollback(2);
        assert_eq!(
            chosen(&mut parser, dragged((1, 0), (8, 10))),
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8"
        );
        assert_eq!(parser.screen().scrollback(), 2, "reading the scrollback leaves the view where it was");

        // Only four lines are kept above the screen: the first ones fall off, the others keep their numbers.
        let mut parser = console(3, 10, 4, &said);
        assert_eq!(parser.callbacks().main, 4);
        let mut grid = lines(&mut parser);
        assert_eq!(grid.row(3), None);
        assert_eq!(text(&mut grid, Spot::new(4, 0), Spot::new(6, 10)), "line4\nline5\nline6");
        assert_eq!(text(&mut grid, Spot::new(2, 0), Spot::new(5, 10)), "line4\nline5", "what fell off is left out");
        parser.process(b"line10\r\nline11\r\n");
        assert_eq!(chosen(&mut parser, dragged((6, 0), (10, 10))), "line6\nline7\nline8\nline9\nline10");
    }

    #[test]
    fn a_shell_runs_and_what_it_prints_is_on_the_screen() {
        let Some(shell) = bash() else { return };
        let dir = std::env::temp_dir();
        let mut term = Terminal::open(&shell, &dir, 24, 80).expect("the shell starts");
        let started = std::time::Instant::now();
        let seen = |term: &mut Terminal, wanted: &str| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while std::time::Instant::now() < until {
                term.poll();
                if term.screen().contents().contains(wanted) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            false
        };
        term.send(b"echo can$((20+22))vas\r");
        assert!(seen(&mut term, "can42vas"), "{:?} after {:?}", term.screen().contents(), started.elapsed());
        term.resize(30, 100);
        assert_eq!(term.size(), (30, 100));
        term.send(b"exit\r");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !term.ended && std::time::Instant::now() < until {
            term.poll();
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        assert!(term.ended, "the shell ends when told to");
    }
}
