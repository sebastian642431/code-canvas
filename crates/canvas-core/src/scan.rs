use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use rayon::prelude::*;

use crate::syntax::{self, Analysis, Lang, Tokens, CHUNK_LINES};
use crate::TAB_WIDTH;

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub max_file_bytes: u64,
    pub max_lines: usize,
    pub max_files: usize,
    pub skip_dirs: Vec<String>,
    pub skip_files: Vec<String>,
    pub cache: Option<PathBuf>,
    pub reuse: bool,
    pub only: bool,
    pub extra: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024,
            max_lines: 100_000,
            max_files: 2_000_000,
            skip_dirs: [".git", "node_modules", "target", ".next", "dist", "build", "__pycache__", ".venv", "venv"]
                .into_iter()
                .map(String::from)
                .collect(),
            skip_files: [
                "pnpm-lock.yaml",
                "package-lock.json",
                "yarn.lock",
                "bun.lockb",
                "Cargo.lock",
                "poetry.lock",
                "composer.lock",
                "Gemfile.lock",
                "go.sum",
                "uv.lock",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            cache: None,
            reuse: true,
            only: false,
            extra: Vec::new(),
        }
    }
}

#[derive(Default, Debug)]
pub struct ScanProgress {
    pub discovered: AtomicUsize,
    pub loaded: AtomicUsize,
}

struct Loaded {
    text: String,
    line_starts: Vec<u32>,
}

static LOADED_BYTES: AtomicUsize = AtomicUsize::new(0);
static CLOCK: AtomicU32 = AtomicU32::new(1);

pub fn loaded_text_bytes() -> usize {
    LOADED_BYTES.load(Ordering::Relaxed)
}

impl Loaded {
    fn new(text: String, line_starts: Vec<u32>) -> Loaded {
        let loaded = Loaded { text, line_starts };
        LOADED_BYTES.fetch_add(loaded.bytes(), Ordering::Relaxed);
        loaded
    }

    fn bytes(&self) -> usize {
        self.text.capacity() + self.line_starts.capacity() * 4
    }

    fn with_lines(mut self, lines: usize) -> Loaded {
        if self.line_starts.len() == lines {
            return self;
        }
        let before = self.bytes();
        if self.line_starts.len() > lines {
            self.text.truncate(self.line_starts[lines] as usize - 1);
            self.line_starts.truncate(lines);
        }
        while self.line_starts.len() < lines {
            self.text.push('\n');
            self.line_starts.push(self.text.len() as u32);
        }
        LOADED_BYTES.fetch_add(self.bytes(), Ordering::Relaxed);
        LOADED_BYTES.fetch_sub(before, Ordering::Relaxed);
        self
    }
}

impl Drop for Loaded {
    fn drop(&mut self) {
        LOADED_BYTES.fetch_sub(self.bytes(), Ordering::Relaxed);
    }
}

pub struct SourceFile {
    pub rel_path: String,
    pub name: String,
    pub lang: Lang,
    loaded: OnceLock<Loaded>,
    root: Option<Arc<Path>>,
    lines: u32,
    used: AtomicU32,
    pub(crate) idents: Box<[u64]>,
    pub(crate) threes: Box<[u64]>,
    pub(crate) names: Box<str>,
    pub(crate) name_at: Box<[u32]>,
    pub(crate) implemented: Box<[(u32, Box<str>)]>,
    pub(crate) stamp: (u64, u64),
    pub analysis: Analysis,
    pub max_cols: u32,
    pub note: Option<String>,
    pub fold: usize,
    pub numbers: Option<Box<[u32]>>,
}

fn ident_bits(ident: &[u8], bits: usize) -> [usize; 3] {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in ident {
        hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    [hash as usize & (bits - 1), (hash >> 21) as usize & (bits - 1), (hash >> 42) as usize & (bits - 1)]
}

fn three_bit(a: u8, b: u8, c: u8, bits: usize) -> usize {
    let three =
        a.to_ascii_lowercase() as u32 | (b.to_ascii_lowercase() as u32) << 8 | (c.to_ascii_lowercase() as u32) << 16;
    (three.wrapping_mul(0x9e37_79b1) >> 7) as usize & (bits - 1)
}

pub(crate) fn three_filter(text: &str) -> Box<[u64]> {
    let bits = (text.len() / 2).clamp(64, 1 << 23).next_power_of_two();
    let mut filter = vec![0u64; bits / 64];
    for three in text.as_bytes().windows(3) {
        let bit = three_bit(three[0], three[1], three[2], bits);
        filter[bit / 64] |= 1 << (bit % 64);
    }
    filter.into_boxed_slice()
}

pub(crate) fn ident_filter(text: &str) -> Box<[u64]> {
    let bits = (text.len() / 2).clamp(64, 1 << 22).next_power_of_two();
    let mut filter = vec![0u64; bits / 64];
    for ident in text.as_bytes().split(|b| !crate::refs::ident_byte(*b)).filter(|run| !run.is_empty()) {
        for bit in ident_bits(ident, bits) {
            filter[bit / 64] |= 1 << (bit % 64);
        }
    }
    filter.into_boxed_slice()
}

impl SourceFile {
    fn loaded(&self) -> &Loaded {
        self.used.store(CLOCK.load(Ordering::Relaxed), Ordering::Relaxed);
        self.loaded.get_or_init(|| self.read())
    }

    fn read(&self) -> Loaded {
        let raw = self.root.as_ref().and_then(|root| std::fs::read(root.join(&self.rel_path)).ok());
        let (text, line_starts, _) = normalise(&String::from_utf8_lossy(&raw.unwrap_or_default()));
        Loaded::new(text, line_starts).with_lines(self.lines as usize)
    }

    pub fn text(&self) -> &str {
        &self.loaded().text
    }

    pub fn line_starts(&self) -> &[u32] {
        &self.loaded().line_starts
    }

    pub fn has_text(&self) -> bool {
        self.loaded.get().is_some()
    }

    pub fn peek<R>(&self, look: impl FnOnce(&str, &[u32]) -> R) -> R {
        match self.loaded.get() {
            Some(loaded) => look(&loaded.text, &loaded.line_starts),
            None => {
                let read = self.read();
                look(&read.text, &read.line_starts)
            }
        }
    }

    pub fn symbol_name(&self, index: usize) -> &str {
        let start = self.name_at[index] as usize;
        &self.names[start..start + self.analysis.symbols[index].name_len as usize]
    }

    pub fn implemented(&self) -> impl Iterator<Item = (u32, &str)> {
        self.implemented.iter().map(|(symbol, name)| (*symbol, &**name))
    }

    fn keep_names(&mut self, text: &str) {
        let mut names = String::new();
        let mut name_at = Vec::with_capacity(self.analysis.symbols.len());
        for symbol in &self.analysis.symbols {
            name_at.push(names.len() as u32);
            names.push_str(symbol.name(text));
        }
        let traits = self.analysis.impl_traits.iter();
        self.implemented = traits
            .map(|&(symbol, start, len)| (symbol, text[start as usize..start as usize + len as usize].into()))
            .collect();
        (self.names, self.name_at) = (names.into(), name_at.into());
    }

    pub fn may_hold(&self, needle: &[u8]) -> bool {
        let bits = self.threes.len() * 64;
        bits == 0
            || needle.windows(3).all(|t| {
                let bit = three_bit(t[0], t[1], t[2], bits);
                self.threes[bit / 64] >> (bit % 64) & 1 == 1
            })
    }

    pub fn may_contain(&self, ident: &str) -> bool {
        let bits = self.idents.len() * 64;
        let whole = !ident.is_empty() && ident.bytes().all(crate::refs::ident_byte);
        !whole
            || bits == 0
            || ident_bits(ident.as_bytes(), bits).iter().all(|bit| self.idents[bit / 64] >> (bit % 64) & 1 == 1)
    }

    fn forget_text(&mut self) -> usize {
        match (&self.root, self.loaded.get()) {
            (Some(_), Some(loaded)) => {
                let bytes = loaded.bytes();
                if self.idents.is_empty() {
                    (self.idents, self.threes) = (ident_filter(&loaded.text), three_filter(&loaded.text));
                }
                self.loaded = OnceLock::new();
                bytes
            }
            _ => 0,
        }
    }

    pub fn line_count(&self) -> usize {
        self.lines as usize
    }

    pub fn chunk_count(&self) -> usize {
        self.line_count().div_ceil(CHUNK_LINES)
    }

    pub fn chunk_lines(&self, c: usize) -> std::ops::Range<usize> {
        let first = c * CHUNK_LINES;
        first..(first + CHUNK_LINES).min(self.line_count())
    }

    pub fn tokenize_chunk(&self, c: usize) -> Tokens {
        let in_comment = self.analysis.chunk_in_comment.get(c).copied().unwrap_or(false);
        let loaded = self.loaded();
        syntax::tokenize_range(&loaded.text, &loaded.line_starts, self.lang, self.chunk_lines(c), in_comment)
    }

    pub fn heap_bytes(&self) -> usize {
        self.loaded.get().map_or(0, Loaded::bytes)
            + self.idents.len() * 8
            + self.threes.len() * 8
            + self.names.len()
            + self.name_at.len() * 4
            + self.analysis.blocks.capacity() * std::mem::size_of::<syntax::Block>()
            + self.analysis.chunk_in_comment.capacity()
            + self.analysis.symbols.capacity() * std::mem::size_of::<crate::symbols::Symbol>()
            + self.rel_path.capacity()
            + self.name.capacity()
    }

    pub fn line(&self, i: usize) -> &str {
        let loaded = self.loaded();
        let Some(&start) = loaded.line_starts.get(i) else { return "" };
        let end = loaded.line_starts.get(i + 1).map(|&e| e as usize - 1).unwrap_or(loaded.text.len());
        &loaded.text[start as usize..end]
    }

    fn with_text(rel_path: String, text: String, line_starts: Vec<u32>, max_cols: u32) -> Self {
        SourceFile {
            name: file_name(&rel_path),
            lang: Lang::from_path(&rel_path),
            rel_path,
            lines: line_starts.len() as u32,
            idents: Box::default(),
            threes: Box::default(),
            names: Box::default(),
            name_at: Box::default(),
            implemented: Box::default(),
            stamp: (0, 0),
            loaded: OnceLock::from(Loaded::new(text, line_starts)),
            root: None,
            used: AtomicU32::new(0),
            analysis: Analysis::default(),
            max_cols,
            note: None,
            fold: 0,
            numbers: None,
        }
    }

    pub(crate) fn kept(
        rel_path: String,
        root: Arc<Path>,
        lines: u32,
        max_cols: u32,
        note: Option<String>,
        analysis: Analysis,
    ) -> Self {
        let mut file = SourceFile::with_text(rel_path, String::new(), vec![0], max_cols);
        if note.as_ref().is_none_or(|note| note.starts_with("truncated")) {
            (file.root, file.loaded, file.lines) = (Some(root), OnceLock::new(), lines);
        } else {
            file = file.tall_as(lines);
        }
        (file.note, file.analysis) = (note, analysis);
        file
    }

    fn tall_as(mut self, lines: u32) -> Self {
        let empty = Loaded::new(String::new(), vec![0]).with_lines(lines as usize);
        (self.lines, self.loaded) = (lines, OnceLock::from(empty));
        self
    }

    fn media(rel_path: String, what: &str) -> Self {
        let mut file = SourceFile::placeholder(rel_path, what.to_string()).tall_as(MEDIA_LINES);
        file.max_cols = MEDIA_COLS;
        file
    }

    fn placeholder(rel_path: String, note: String) -> Self {
        let mut file = SourceFile::with_text(rel_path, String::new(), vec![0], 0);
        file.note = Some(note);
        file
    }
}

#[derive(Default, Debug)]
pub struct Dir {
    pub name: String,
    pub rel_path: String,
    pub dirs: Vec<Dir>,
    pub files: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScanTimings {
    pub walk_ms: u128,
    pub load_ms: u128,
    pub tree_ms: u128,
    pub index_ms: u128,
}

pub struct Repo {
    pub root: PathBuf,
    pub name: String,
    pub tree: Dir,
    pub files: Vec<SourceFile>,
    pub total_lines: usize,
    pub scan_ms: u128,
    pub timings: ScanTimings,
    pub symbols: crate::refs::SymbolIndex,
    pub kept_deps: Vec<Option<crate::deps::FileDeps>>,
    pub reused: usize,
    pub text_bytes: u64,
}

impl Repo {
    pub fn make_filters(&mut self) {
        self.files.par_iter_mut().for_each(|f| {
            let made = match (f.idents.is_empty() && f.root.is_some(), f.loaded.get()) {
                (true, Some(loaded)) => Some((ident_filter(&loaded.text), three_filter(&loaded.text))),
                _ => None,
            };
            if let Some((idents, threes)) = made {
                (f.idents, f.threes) = (idents, threes);
            }
        });
    }

    pub fn warm(&self, from: usize, count: usize) -> usize {
        let end = (from + count).min(self.files.len());
        self.files[from..end].par_iter().filter(|f| f.root.is_some() && !f.has_text()).for_each(|f| {
            f.loaded.get_or_init(|| f.read());
        });
        end
    }

    pub fn trim_text(&mut self, keep: usize) -> usize {
        let mut held: Vec<(u32, usize, usize)> = Vec::new();
        for (index, f) in self.files.iter().enumerate() {
            if let (Some(_), Some(loaded)) = (&f.root, f.loaded.get()) {
                held.push((f.used.load(Ordering::Relaxed), index, loaded.bytes()));
            }
        }
        let mut total: usize = held.iter().map(|(_, _, bytes)| bytes).sum();
        CLOCK.fetch_add(1, Ordering::Relaxed);
        if total <= keep {
            return 0;
        }
        held.sort_unstable();
        let mut going = vec![false; self.files.len()];
        for (_, index, bytes) in held {
            if total <= keep / 4 * 3 {
                break;
            }
            total -= bytes;
            going[index] = true;
        }
        let let_go = |(index, file): (usize, &mut SourceFile)| if going[index] { file.forget_text() } else { 0 };
        self.files.par_iter_mut().enumerate().map(let_go).sum()
    }

    pub fn file_index(&self, rel_path: &str) -> Option<usize> {
        self.files.binary_search_by(|f| Path::new(&f.rel_path).cmp(Path::new(rel_path))).ok()
    }

    pub fn replace_source(&mut self, index: usize, source: &str, opts: &ScanOptions) {
        let rel = self.files[index].rel_path.clone();
        self.files[index] = build_file(rel, source, opts);
        self.total_lines = self.files.iter().map(|f| f.line_count()).sum();
    }

    pub fn replace_shown(
        &mut self,
        index: usize,
        shown: &str,
        real: &str,
        row_of: impl Fn(u32) -> (u32, bool),
        opts: &ScanOptions,
    ) -> Vec<(u32, u32)> {
        let rel = self.files[index].rel_path.clone();
        let mut file = build_file(rel.clone(), shown, opts);
        let SourceFile { analysis, names, name_at, implemented, loaded, .. } = build_file(rel, real, opts);
        let starts =
            |loaded: &OnceLock<Loaded>| loaded.get().map(|loaded| loaded.line_starts.clone()).unwrap_or_default();
        let (of_real, of_shown) = (starts(&loaded), starts(&file.loaded));
        let start = |starts: &[u32], line: u32| starts.get(line as usize).copied().unwrap_or(0);
        let mut symbols = analysis.symbols;
        let places: Vec<(u32, u32)> = symbols.iter().map(|symbol| (symbol.line, symbol.body_end)).collect();
        let rows = file.lines.max(1);
        for symbol in &mut symbols {
            let (row, seen) = row_of(symbol.line);
            let (last, _) = row_of(symbol.body_end.saturating_sub(1).max(symbol.line));
            let (row, last, seen) = (row.min(rows - 1), last.min(rows - 1), seen && row < rows);
            let within = symbol.name_start.saturating_sub(start(&of_real, symbol.line));
            symbol.name_start = if seen { start(&of_shown, row) + within } else { u32::MAX };
            (symbol.line, symbol.body_end) = (row, last.max(row) + 1);
        }
        (file.analysis.symbols, file.analysis.impl_traits) = (symbols, Vec::new());
        (file.names, file.name_at, file.implemented) = (names, name_at, implemented);
        self.files[index] = file;
        self.total_lines = self.files.iter().map(|f| f.line_count()).sum();
        places
    }

    pub fn reindex_symbols(&mut self) {
        self.symbols = crate::refs::SymbolIndex::build(&self.files);
    }

    pub fn reload_file(&mut self, index: usize, opts: &ScanOptions) {
        let rel = self.files[index].rel_path.clone();
        let path = self.root.join(&rel);
        self.files[index] = load_file(&path, rel, None, opts);
        self.files[index].root = Some(Arc::from(self.root.as_path()));
        self.total_lines = self.files.iter().map(|f| f.line_count()).sum();
        self.symbols = crate::refs::SymbolIndex::build(&self.files);
    }
}

pub fn clean_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

fn file_name(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

pub fn normalise(raw: &str) -> (String, Vec<u32>, u32) {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut text = String::with_capacity(raw.len());
    let mut starts = vec![0u32];
    let mut col = 0usize;
    let mut max_cols = 0usize;
    for ch in raw.chars() {
        match ch {
            '\r' => {}
            '\n' => {
                max_cols = max_cols.max(col);
                col = 0;
                text.push('\n');
                starts.push(text.len() as u32);
            }
            '\t' => {
                let spaces = TAB_WIDTH - (col % TAB_WIDTH);
                for _ in 0..spaces {
                    text.push(' ');
                }
                col += spaces;
            }
            c => {
                text.push(c);
                col += 1;
            }
        }
    }
    max_cols = max_cols.max(col);
    if starts.len() > 1 && *starts.last().unwrap() as usize == text.len() {
        starts.pop();
        text.pop();
    }
    (text, starts, max_cols as u32)
}

pub static LOAD_PROFILE: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

fn lap(step: usize, since: &mut Instant) {
    let now = Instant::now();
    LOAD_PROFILE[step].fetch_add(now.duration_since(*since).as_nanos() as u64, Ordering::Relaxed);
    *since = now;
}

fn read_sized(path: &Path, size: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::with_capacity(size as usize + 1);
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn load_file(path: &Path, rel_path: String, size: Option<u64>, opts: &ScanOptions) -> SourceFile {
    let mut t = Instant::now();
    if let Some(what) = media_kind(&rel_path) {
        return SourceFile::media(rel_path, what);
    }
    if is_binary_name(&rel_path) {
        return SourceFile::placeholder(rel_path, "binary".into());
    }
    let size = match size.map(Ok).unwrap_or_else(|| std::fs::metadata(path).map(|m| m.len())) {
        Ok(size) => size,
        Err(e) => return SourceFile::placeholder(rel_path, format!("unreadable: {e}")),
    };
    if size > opts.max_file_bytes {
        return SourceFile::placeholder(rel_path, format!("too large ({} KB)", size / 1024));
    }
    lap(0, &mut t);
    let bytes = match read_sized(path, size) {
        Ok(b) => b,
        Err(e) => return SourceFile::placeholder(rel_path, format!("unreadable: {e}")),
    };
    lap(1, &mut t);
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return SourceFile::placeholder(rel_path, "binary".into());
    }
    let file = build_file(rel_path, &String::from_utf8_lossy(&bytes), opts);
    lap(3, &mut t);
    file
}

fn build_file(rel_path: String, raw: &str, opts: &ScanOptions) -> SourceFile {
    let mut t = Instant::now();
    let (mut text, mut line_starts, max_cols) = normalise(raw);
    let mut note = None;
    if line_starts.len() > opts.max_lines {
        note = Some(format!("truncated at {} of {} lines", opts.max_lines, line_starts.len()));
        let cut = line_starts[opts.max_lines] as usize - 1;
        text.truncate(cut);
        line_starts.truncate(opts.max_lines);
    }
    lap(2, &mut t);
    let lang = Lang::from_path(&rel_path);
    let analysis = syntax::analyze(&text, &line_starts, lang, crate::symbols::Dialect::from_path(&rel_path));
    text.shrink_to_fit();
    let mut file = SourceFile::with_text(rel_path, String::new(), Vec::new(), max_cols);
    (file.analysis, file.note) = (analysis, note);
    file.keep_names(&text);
    file.lines = line_starts.len() as u32;
    file.loaded = OnceLock::from(Loaded::new(text, line_starts));
    file
}

type Found = (PathBuf, String, Option<u64>, Option<u64>);

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "icns", "cur", "webp", "avif", "tif", "tiff", "psd", "woff", "woff2",
    "ttf", "otf", "eot", "pdf", "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "jar", "war", "apk", "aab", "crx",
    "exe", "dll", "so", "dylib", "a", "lib", "o", "obj", "pdb", "class", "wasm", "pyc", "mp3", "mp4", "m4a", "aac",
    "wav", "ogg", "opus", "flac", "webm", "mov", "avi", "mkv", "sqlite", "db", "pak", "nib", "keystore",
];

pub const MEDIA_LINES: u32 = 24;
pub const MEDIA_COLS: u32 = 72;

pub fn media_kind(rel_path: &str) -> Option<&'static str> {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "avif" | "tif" | "tiff" => "picture",
        "mp4" | "m4v" | "webm" | "mov" | "mkv" | "avi" | "ogv" => "video",
        "mp3" | "m4a" | "aac" | "wav" | "ogg" | "opus" | "flac" => "sound",
        "pdf" => "document",
        _ => return None,
    })
}

fn is_binary_name(rel_path: &str) -> bool {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    match name.rsplit_once('.') {
        Some((_, ext)) => BINARY_EXTENSIONS.iter().any(|b| ext.eq_ignore_ascii_case(b)),
        None => false,
    }
}

struct Collector<'a> {
    root: &'a Path,
    opts: &'a ScanOptions,
    progress: Option<&'a ScanProgress>,
    shared: &'a Mutex<Vec<Found>>,
}

struct Visitor<'a> {
    c: Collector<'a>,
    local: Vec<Found>,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for Collector<'s> {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        let c = Collector { root: self.root, opts: self.opts, progress: self.progress, shared: self.shared };
        Box::new(Visitor { c, local: Vec::new() })
    }
}

impl Visitor<'_> {
    fn flush(&mut self) -> usize {
        let mut all = self.c.shared.lock().unwrap();
        all.append(&mut self.local);
        all.len()
    }
}

impl ignore::ParallelVisitor for Visitor<'_> {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        let Ok(entry) = entry else { return ignore::WalkState::Continue };
        if !entry.file_type().is_some_and(|t| t.is_file())
            || self.c.opts.skip_files.iter().any(|f| entry.file_name() == f.as_str())
        {
            return ignore::WalkState::Continue;
        }
        let rel = entry.path().strip_prefix(self.c.root).unwrap_or(entry.path()).to_string_lossy().replace('\\', "/");
        let known = if cfg!(windows) || self.c.opts.cache.is_some() { entry.metadata().ok() } else { None };
        let size = known.as_ref().map(|m| m.len());
        let changed =
            known.and_then(|m| m.modified().ok()).and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok());
        self.local.push((entry.into_path(), rel, size, changed.map(|since| since.as_nanos() as u64)));
        if let Some(p) = self.c.progress {
            p.discovered.fetch_add(1, Ordering::Relaxed);
        }
        if self.local.len() >= 512 && self.flush() >= self.c.opts.max_files {
            return ignore::WalkState::Quit;
        }
        ignore::WalkState::Continue
    }
}

impl Drop for Visitor<'_> {
    fn drop(&mut self) {
        if !self.local.is_empty() {
            self.flush();
        }
    }
}

pub fn scan(root: &Path, opts: &ScanOptions, progress: Option<Arc<ScanProgress>>) -> Repo {
    let started = Instant::now();
    let root = clean_path(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).thread_name(|i| format!("scan {i}")).build();

    let skip = opts.skip_dirs.clone();
    let walker = ignore::WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(move |e| {
            !(e.file_type().is_some_and(|t| t.is_dir()) && skip.iter().any(|s| e.file_name() == s.as_str()))
        })
        .threads(workers)
        .build_parallel();

    let shared = Mutex::new(Vec::<Found>::new());
    let mut builder = Collector { root: &root, opts, progress: progress.as_deref(), shared: &shared };
    if !opts.only {
        walker.visit(&mut builder);
    }
    let mut paths = shared.into_inner().unwrap();
    for rel in &opts.extra {
        let path = root.join(rel);
        let Some(known) = std::fs::metadata(&path).ok().filter(|m| m.is_file()) else { continue };
        if paths.iter().all(|found| found.1 != *rel) {
            let changed = known.modified().ok().and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok());
            paths.push((path, rel.clone(), Some(known.len()), changed.map(|since| since.as_nanos() as u64)));
        }
    }
    paths.sort_unstable_by(|a, b| Path::new(&a.1).cmp(Path::new(&b.1)));
    if paths.len() > opts.max_files {
        log::warn!("file limit {} reached, the rest is skipped", opts.max_files);
        paths.truncate(opts.max_files);
    }

    let walk_ms = started.elapsed().as_millis();
    let t_load = Instant::now();
    let dir: Arc<Path> = Arc::from(root.as_path());
    let kept = opts.cache.as_deref().filter(|_| opts.reuse).and_then(|file| crate::store::Kept::read(file, opts));
    let same_files =
        kept.as_ref().is_some_and(|k| k.same_files(paths.len(), paths.iter().map(|found| found.1.as_str())));
    let hits = AtomicUsize::new(0);
    let load = || -> Vec<(SourceFile, Option<crate::deps::FileDeps>)> {
        paths
            .par_iter()
            .map(|(path, rel, size, changed)| {
                let stamp = size.zip(*changed);
                let had = kept.as_ref().zip(stamp).and_then(|(kept, stamp)| kept.file(rel, stamp, &dir));
                let hit = had.is_some();
                hits.fetch_add(hit as usize, Ordering::Relaxed);
                let loaded = had.unwrap_or_else(|| {
                    let mut f = load_file(path, rel.clone(), *size, opts);
                    if f.note.as_ref().is_none_or(|note| note.starts_with("truncated")) {
                        f.root = Some(dir.clone());
                    }
                    f.stamp = stamp.unwrap_or_default();
                    (f, crate::deps::FileDeps::default())
                });
                if let Some(p) = &progress {
                    p.loaded.fetch_add(1, Ordering::Relaxed);
                }
                (loaded.0, (hit && same_files).then_some(loaded.1))
            })
            .collect()
    };
    let (files, kept_deps): (Vec<SourceFile>, Vec<Option<crate::deps::FileDeps>>) = match &pool {
        Ok(pool) => pool.install(load),
        Err(_) => load(),
    }
    .into_iter()
    .unzip();
    drop(kept);
    let reused = hits.load(Ordering::Relaxed);
    let text_bytes = files.iter().filter(|f| f.root.is_some()).map(|f| f.stamp.0).sum();

    let load_ms = t_load.elapsed().as_millis();
    let t_tree = Instant::now();
    let mut tree = Dir { name: name.clone(), rel_path: String::new(), ..Default::default() };
    for (idx, f) in files.iter().enumerate() {
        let mut dir = &mut tree;
        let mut parts: Vec<&str> = f.rel_path.split('/').collect();
        parts.pop();
        let mut rel = String::new();
        for part in parts {
            if !rel.is_empty() {
                rel.push('/');
            }
            rel.push_str(part);
            let pos = match dir.dirs.iter().position(|d| d.name == part) {
                Some(p) => p,
                None => {
                    dir.dirs.push(Dir { name: part.to_string(), rel_path: rel.clone(), ..Default::default() });
                    dir.dirs.len() - 1
                }
            };
            dir = &mut dir.dirs[pos];
        }
        dir.files.push(idx);
    }

    let tree_ms = t_tree.elapsed().as_millis();
    let t_index = Instant::now();
    let symbols = crate::refs::SymbolIndex::build(&files);
    let index_ms = t_index.elapsed().as_millis();
    let total_lines = files.iter().map(|f| f.line_count()).sum();
    Repo {
        root,
        name,
        tree,
        files,
        total_lines,
        scan_ms: started.elapsed().as_millis(),
        timings: ScanTimings { walk_ms, load_ms, tree_ms, index_ms },
        symbols,
        kept_deps,
        reused,
        text_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_tabs_and_crlf() {
        let (t, starts, cols) = normalise("a\tb\r\nxy\n");
        assert_eq!(t, "a   b\nxy");
        assert_eq!(starts, vec![0, 6]);
        assert_eq!(cols, 5);
    }

    #[test]
    fn scans_this_crate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo = scan(root, &ScanOptions::default(), None);
        assert!(repo.files.iter().any(|f| f.rel_path == "src/scan.rs"));
        assert!(repo.tree.dirs.iter().any(|d| d.name == "src"));
        for (i, f) in repo.files.iter().enumerate() {
            assert_eq!(repo.file_index(&f.rel_path), Some(i), "{}", f.rel_path);
        }
        assert_eq!(repo.file_index("src/nope.rs"), None);
        let f = repo.files.iter().find(|f| f.name == "lib.rs").unwrap();
        assert_eq!(f.analysis.blocks.len(), f.line_count().div_ceil(syntax::BLOCK_LINES));
        let all: usize = (0..f.chunk_count()).map(|c| f.tokenize_chunk(c).lines()).sum();
        assert_eq!(all, f.line_count());
    }

    #[test]
    fn text_is_let_go_and_read_again() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut repo = scan(root, &ScanOptions::default(), None);
        let at = repo.file_index("src/scan.rs").unwrap();
        let (text, lines) = (repo.files[at].text().to_string(), repo.files[at].line_count());
        let absent = ["never", "written", "anywhere"].join("_");
        assert!(repo.files[at].may_contain(&absent), "nothing is ruled out while the text is at hand");
        assert!(repo.trim_text(0) > 0);
        assert!(repo.files[at].may_contain("trim_text") && !repo.files[at].may_contain(&absent));
        let unseen = absent.replace('_', "#~");
        assert!(repo.files[at].may_hold(b"pub fn trim_text(&mut self") && !repo.files[at].may_hold(unseen.as_bytes()));
        assert!(repo.files.iter().all(|f| !f.has_text() || f.note.is_some()));
        assert_eq!(repo.files[at].peek(|text, starts| (text.len(), starts.len())), (text.len(), lines));
        assert!(!repo.files[at].has_text(), "a look does not keep the text");
        assert_eq!(repo.files[at].text(), text);
        assert!(repo.files[at].has_text());
        assert_eq!(repo.files[at].line(lines + 5), "");

        repo.replace_source(at, "only this\nand this", &ScanOptions::default());
        assert_eq!(repo.trim_text(0), repo.trim_text(0), "nothing left to let go");
        assert_eq!(repo.files[at].line(1), "and this", "text given by hand is all there is: it stays");
    }
}
