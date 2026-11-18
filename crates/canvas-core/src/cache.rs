use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use crate::scan::SourceFile;
use crate::syntax::{dominant_kind, Block, Token, TokenKind, Tokens, BLOCK_LINES, CHUNK_LINES};

struct Entry {
    tokens: Tokens,
    last_used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub start: u32,
    pub len: u32,
    pub kind: TokenKind,
}

#[derive(Clone, Debug, Default)]
pub struct Painted {
    pub serial: u64,
    pub total: usize,
    pub lines: Vec<Arc<[Piece]>>,
}

impl Painted {
    pub fn covers(&self, lines: &Range<usize>) -> bool {
        lines.end.min(self.total) <= self.lines.len()
    }

    fn tokens(&self, file: &SourceFile, lines: Range<usize>) -> Option<Tokens> {
        if !self.covers(&lines) || lines.end > self.total {
            return None;
        }
        let (text, starts) = (file.text(), file.line_starts());
        let mut out = Tokens { tokens: Vec::new(), line_start: Vec::with_capacity(lines.len() + 1) };
        for line in lines {
            out.line_start.push(out.tokens.len() as u32);
            let start = *starts.get(line)? as usize;
            let end = starts.get(line + 1).map_or(text.len(), |&next| next as usize - 1);
            let body = text.get(start..end)?;
            let (mut col, mut at) = (0usize, 0usize);
            for piece in self.lines[line].iter() {
                let (s, e) = (piece.start as usize, piece.start as usize + piece.len as usize);
                if s < at || e > body.len() || !body.is_char_boundary(s) || !body.is_char_boundary(e) {
                    return None;
                }
                col += body[at..s].chars().count();
                let cols = body[s..e].chars().count();
                out.tokens.push(Token {
                    start: (start + s) as u32,
                    len: (e - s).min(u16::MAX as usize) as u16,
                    col: col.min(u16::MAX as usize) as u16,
                    cols: cols.min(u16::MAX as usize) as u16,
                    kind: piece.kind,
                });
                (col, at) = (col + cols, e);
            }
        }
        out.line_start.push(out.tokens.len() as u32);
        Some(out)
    }

    /// The far-away bars of the file (one per `BLOCK_LINES` lines: the columns its
    /// pieces cover and the kind most of them are), as `syntax::analyze` makes them
    /// from the app's own tokens. Only for the whole file, painted for its text.
    pub fn blocks(&self, file: &SourceFile) -> Option<Vec<Block>> {
        let n = file.line_count();
        if self.serial != file.serial() || self.total != n || self.lines.len() < n {
            return None;
        }
        let (text, starts) = (file.text(), file.line_starts());
        let mut out = Vec::with_capacity(n.div_ceil(BLOCK_LINES));
        let mut block = (u16::MAX, 0u16, [0u32; 11]);
        for line in 0..n {
            let start = *starts.get(line)? as usize;
            let end = starts.get(line + 1).map_or(text.len(), |&next| next as usize - 1);
            let body = text.get(start..end)?;
            let (mut col, mut at) = (0usize, 0usize);
            for piece in self.lines[line].iter() {
                let (s, e) = (piece.start as usize, piece.start as usize + piece.len as usize);
                if s < at || e > body.len() || !body.is_char_boundary(s) || !body.is_char_boundary(e) {
                    return None;
                }
                col += body[at..s].chars().count();
                let cols = body[s..e].chars().count();
                let (lo, hi) = (col.min(u16::MAX as usize) as u16, (col + cols).min(u16::MAX as usize) as u16);
                (block.0, block.1) = (block.0.min(lo), block.1.max(hi));
                block.2[piece.kind as usize] += cols as u32;
                (col, at) = (col + cols, e);
            }
            if line % BLOCK_LINES == BLOCK_LINES - 1 || line + 1 == n {
                out.push(Block { lo: block.0, hi: block.1, kind: dominant_kind(&block.2) });
                block = (u16::MAX, 0, [0; 11]);
            }
        }
        Some(out)
    }
}

pub type Painter = Arc<dyn Fn(&SourceFile) -> bool + Send + Sync>;

const PAINTED_LINES: usize = 1_000_000;

pub struct TokenCache {
    map: HashMap<(u32, u32), Entry>,
    frame: u64,
    pub max_chunks: usize,
    pub tokenized_this_frame: usize,
    painted: HashMap<u32, Arc<Painted>>,
    painter: Option<Painter>,
    asked: HashSet<u32>,
}

impl Default for TokenCache {
    fn default() -> Self {
        Self::new(16_384)
    }
}

impl TokenCache {
    pub fn new(max_chunks: usize) -> Self {
        Self {
            map: HashMap::new(),
            frame: 0,
            max_chunks,
            tokenized_this_frame: 0,
            painted: HashMap::new(),
            painter: None,
            asked: HashSet::new(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.tokenized_this_frame = 0;
        if self.map.len() > self.max_chunks {
            let mut ages: Vec<u64> = self.map.values().map(|e| e.last_used).collect();
            ages.sort_unstable();
            let cut = ages[self.map.len() - self.max_chunks * 3 / 4];
            let frame = self.frame;
            self.map.retain(|_, e| e.last_used >= cut || e.last_used + 2 >= frame);
        }
        if self.frame.is_multiple_of(256) && self.painted.values().map(|p| p.lines.len()).sum::<usize>() > PAINTED_LINES
        {
            let seen: HashSet<u32> = self.map.keys().map(|&(file, _)| file).collect();
            self.painted.retain(|file, _| seen.contains(file));
        }
    }

    pub fn chunk(&mut self, file_index: usize, file: &SourceFile, chunk: usize) -> &Tokens {
        let frame = self.frame;
        let mut made = false;
        let TokenCache { map, painted, painter, asked, .. } = self;
        let e = map.entry((file_index as u32, chunk as u32)).or_insert_with(|| {
            made = true;
            let wanted = painter.as_ref().is_some_and(|painter| painter(file));
            let painted = painted.get(&(file_index as u32)).filter(|p| wanted && p.serial == file.serial());
            if wanted && painted.is_none_or(|p| p.lines.len() < p.total) {
                asked.insert(file_index as u32);
            }
            let own = painted.and_then(|p| p.tokens(file, file.chunk_lines(chunk)));
            Entry { tokens: own.unwrap_or_else(|| file.tokenize_chunk(chunk)), last_used: frame }
        });
        e.last_used = frame;
        if made {
            self.tokenized_this_frame += 1;
        }
        &e.tokens
    }

    pub fn set_painter(&mut self, painter: Option<Painter>) {
        self.painter = painter;
        self.painted.clear();
        self.asked.clear();
        self.map.clear();
    }

    pub fn has_painter(&self) -> bool {
        self.painter.is_some()
    }

    pub fn paint(&mut self, file_index: usize, painted: Arc<Painted>) -> Vec<u32> {
        let file = file_index as u32;
        let covered = |p: &Painted| match p.lines.len() >= p.total {
            true => p.total.div_ceil(CHUNK_LINES),
            false => p.lines.len() / CHUNK_LINES,
        };
        let had = self.painted.get(&file).filter(|p| p.serial == painted.serial).map_or(0, |p| covered(p));
        let new = had..covered(&painted).max(had);
        self.map.retain(|&(f, c), _| f != file || !new.contains(&(c as usize)));
        self.painted.insert(file, painted);
        new.map(|c| c as u32).collect()
    }

    pub fn take_asked(&mut self) -> Vec<u32> {
        let mut asked: Vec<u32> = self.asked.drain().collect();
        asked.sort_unstable();
        asked
    }

    pub fn reset(&mut self) {
        self.map.clear();
        self.asked.clear();
    }

    pub fn forget_file(&mut self, file_index: usize) {
        self.map.retain(|&(file, _), _| file != file_index as u32);
        self.asked.remove(&(file_index as u32));
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes() -> SourceFile {
        crate::scan::build_file("notes.zzz".into(), "alpha beta\ngamma é x\n", &crate::ScanOptions::default())
    }

    fn piece(start: u32, len: u32, kind: TokenKind) -> Piece {
        Piece { start, len, kind }
    }

    fn painted(file: &SourceFile, lines: Vec<Vec<Piece>>) -> Arc<Painted> {
        let lines = lines.into_iter().map(Arc::from).collect();
        Arc::new(Painted { serial: file.serial(), total: file.line_count(), lines })
    }

    #[test]
    fn painted_pieces_replace_the_own_tokens_once_they_come() {
        let file = &notes();
        let mut cache = TokenCache::default();
        cache.set_painter(Some(Arc::new(|_: &SourceFile| true)));
        assert!(cache.chunk(0, file, 0).tokens.iter().all(|t| t.kind == TokenKind::Text));
        assert_eq!(cache.take_asked(), vec![0]);
        let lines = vec![
            vec![piece(0, 5, TokenKind::Keyword)],
            vec![piece(0, 5, TokenKind::Function), piece(9, 1, TokenKind::Number)],
            vec![],
        ];
        assert_eq!(cache.paint(0, painted(file, lines)), vec![0]);
        let tokens = cache.chunk(0, file, 0).clone();
        assert_eq!(tokens.line(0)[0].kind, TokenKind::Keyword);
        let x = tokens.line(1)[1];
        assert_eq!((x.kind, x.start, x.col, x.cols), (TokenKind::Number, 20, 8, 1));
        assert!(cache.take_asked().is_empty());
    }

    #[test]
    fn stale_partial_or_misfit_pieces_are_left_out() {
        let file = &notes();
        let mut cache = TokenCache::default();
        cache.set_painter(Some(Arc::new(|_: &SourceFile| true)));
        let wrong = vec![vec![piece(3, 40, TokenKind::Keyword)], vec![], vec![]];
        cache.paint(0, painted(file, wrong));
        assert_ne!(cache.chunk(0, file, 0).tokens.first().map(|t| t.kind), Some(TokenKind::Keyword));
        cache.reset();
        let inside = vec![vec![], vec![piece(7, 1, TokenKind::Keyword)], vec![]];
        cache.paint(0, painted(file, inside));
        assert!(cache.chunk(0, file, 0).tokens.iter().all(|t| t.kind != TokenKind::Keyword));
        cache.reset();
        let mut stale = Painted::clone(&painted(file, vec![vec![piece(0, 5, TokenKind::Keyword)], vec![], vec![]]));
        stale.serial += 1000;
        cache.paint(0, Arc::new(stale));
        assert_ne!(cache.chunk(0, file, 0).tokens.first().map(|t| t.kind), Some(TokenKind::Keyword));
        assert_eq!(cache.take_asked(), vec![0]);
        cache.reset();
        let partial = Painted { serial: file.serial(), total: 3, lines: vec![Arc::from(vec![])] };
        assert!(cache.paint(0, Arc::new(partial)).is_empty());
        cache.chunk(0, file, 0);
        assert_eq!(cache.take_asked(), vec![0]);
    }

    #[test]
    fn the_far_away_blocks_come_from_the_whole_painted_file() {
        let file = &notes();
        let lines = vec![
            vec![piece(0, 5, TokenKind::Keyword), piece(6, 4, TokenKind::Keyword)],
            vec![piece(0, 5, TokenKind::Function), piece(9, 1, TokenKind::Number)],
            vec![],
        ];
        let whole = painted(file, lines);
        assert_eq!(whole.blocks(file), Some(vec![Block { lo: 0, hi: 10, kind: TokenKind::Keyword }]));
        let partial = Painted { lines: whole.lines[..1].to_vec(), ..Painted::clone(&whole) };
        assert_eq!(partial.blocks(file), None, "only once the whole file is painted");
        let stale = Painted { serial: whole.serial + 1000, ..Painted::clone(&whole) };
        assert_eq!(stale.blocks(file), None, "only for the text it was painted for");
        let misfit = painted(file, vec![vec![piece(3, 40, TokenKind::Keyword)], vec![], vec![]]);
        assert_eq!(misfit.blocks(file), None);
    }

    #[test]
    fn without_a_painter_nothing_is_asked() {
        let file = &notes();
        let mut cache = TokenCache::default();
        cache.paint(0, painted(file, vec![vec![piece(0, 5, TokenKind::Keyword)], vec![], vec![]]));
        assert_ne!(cache.chunk(0, file, 0).tokens[0].kind, TokenKind::Keyword);
        assert!(cache.take_asked().is_empty());
    }
}
