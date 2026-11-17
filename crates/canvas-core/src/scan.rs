//! Walk a folder and load every text file into memory, in parallel.
//!
//! The walk respects `.gitignore`, `.ignore` and global git excludes (via the
//! `ignore` crate, the same engine ripgrep uses). File reading and
//! tokenisation run on all cores with `rayon`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rayon::prelude::*;

use crate::syntax::{self, Analysis, Lang, Tokens, CHUNK_LINES};
use crate::TAB_WIDTH;

#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Files bigger than this are shown as an empty card.
    pub max_file_bytes: u64,
    /// Files longer than this are truncated (the card says so).
    pub max_lines: usize,
    /// Hard cap on the number of files loaded.
    pub max_files: usize,
    /// Directory names that are always skipped, even when not git-ignored.
    pub skip_dirs: Vec<String>,
    /// File names that are always skipped (lockfiles: huge and rarely useful).
    pub skip_files: Vec<String>,
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
        }
    }
}

/// Progress counters shared with the UI thread while a scan runs.
#[derive(Default, Debug)]
pub struct ScanProgress {
    pub discovered: AtomicUsize,
    pub loaded: AtomicUsize,
}

pub struct SourceFile {
    /// Path relative to the repository root, always with `/` separators.
    pub rel_path: String,
    pub name: String,
    pub lang: Lang,
    /// Normalised text: `\n` line endings, tabs expanded.
    pub text: String,
    /// Byte offset of every line in `text`.
    pub line_starts: Vec<u32>,
    /// Block summaries and chunk states; tokens themselves are produced on
    /// demand with [`SourceFile::tokenize_chunk`].
    pub analysis: Analysis,
    /// Widest line, in character cells.
    pub max_cols: u32,
    /// Why the file is shown without (all of) its contents, if it is.
    pub note: Option<String>,
}

impl SourceFile {
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn chunk_count(&self) -> usize {
        self.line_count().div_ceil(CHUNK_LINES)
    }

    /// Lines covered by chunk `c`.
    pub fn chunk_lines(&self, c: usize) -> std::ops::Range<usize> {
        let first = c * CHUNK_LINES;
        first..(first + CHUNK_LINES).min(self.line_count())
    }

    /// Tokenize one chunk. Tokens keep absolute byte offsets into `text`.
    pub fn tokenize_chunk(&self, c: usize) -> Tokens {
        let in_comment = self.analysis.chunk_in_comment.get(c).copied().unwrap_or(false);
        syntax::tokenize_range(&self.text, &self.line_starts, self.lang, self.chunk_lines(c), in_comment)
    }

    /// Approximate heap memory used by this file.
    pub fn heap_bytes(&self) -> usize {
        self.text.capacity()
            + self.line_starts.capacity() * 4
            + self.analysis.blocks.capacity() * std::mem::size_of::<syntax::Block>()
            + self.analysis.chunk_in_comment.capacity()
            + self.analysis.symbols.capacity() * std::mem::size_of::<crate::symbols::Symbol>()
            + self.rel_path.capacity()
            + self.name.capacity()
    }

    pub fn line(&self, i: usize) -> &str {
        let s = self.line_starts[i] as usize;
        let e = self.line_starts.get(i + 1).map(|&e| e as usize - 1).unwrap_or(self.text.len());
        &self.text[s..e]
    }

    fn placeholder(rel_path: String, note: String) -> Self {
        let name = file_name(&rel_path);
        SourceFile {
            lang: Lang::from_path(&rel_path),
            rel_path,
            name,
            text: String::new(),
            line_starts: vec![0],
            analysis: Analysis::default(),
            max_cols: 0,
            note: Some(note),
        }
    }
}

#[derive(Default, Debug)]
pub struct Dir {
    pub name: String,
    pub rel_path: String,
    pub dirs: Vec<Dir>,
    /// Indices into [`Repo::files`].
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
    /// Definitions of the whole repository, by name.
    pub symbols: crate::refs::SymbolIndex,
}

impl Repo {
    /// Index in [`Repo::files`] of the file at `rel_path` (`/` separators).
    /// Files are kept sorted by path, so this is a binary search.
    pub fn file_index(&self, rel_path: &str) -> Option<usize> {
        self.files.binary_search_by(|f| Path::new(&f.rel_path).cmp(Path::new(rel_path))).ok()
    }

    /// Show `source` as the contents of file `index` without touching the
    /// disk: an edit that is not saved yet. The symbol index is left as it
    /// was (rebuilding it per keystroke would be slow); call
    /// [`Repo::reindex_symbols`] when the typing pauses.
    pub fn replace_source(&mut self, index: usize, source: &str, opts: &ScanOptions) {
        let rel = self.files[index].rel_path.clone();
        self.files[index] = build_file(rel, source, opts);
        self.total_lines = self.files.iter().map(|f| f.line_count()).sum();
    }

    pub fn reindex_symbols(&mut self) {
        self.symbols = crate::refs::SymbolIndex::build(&self.files);
    }

    /// Read one file again (it changed on disk) and refresh what is derived
    /// from the files: line totals and the symbol index.
    pub fn reload_file(&mut self, index: usize, opts: &ScanOptions) {
        let rel = self.files[index].rel_path.clone();
        let path = self.root.join(&rel);
        self.files[index] = load_file(&path, rel, None, opts);
        self.total_lines = self.files.iter().map(|f| f.line_count()).sum();
        self.symbols = crate::refs::SymbolIndex::build(&self.files);
    }
}

/// `canonicalize` on Windows returns `\\?\C:\...`; strip that prefix.
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

/// Expand tabs, drop `\r`, and compute line starts.
pub fn normalise(raw: &str) -> (String, Vec<u32>, u32) {
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
    // A trailing newline should not create an extra empty line.
    if starts.len() > 1 && *starts.last().unwrap() as usize == text.len() {
        starts.pop();
        text.pop();
    }
    (text, starts, max_cols as u32)
}

/// CPU time spent per loading step, summed over all threads, in
/// nanoseconds: `[stat, read, normalise, analyse]`. For `scan_bench`.
pub static LOAD_PROFILE: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

fn lap(step: usize, since: &mut Instant) {
    let now = Instant::now();
    LOAD_PROFILE[step].fetch_add(now.duration_since(*since).as_nanos() as u64, Ordering::Relaxed);
    *since = now;
}

/// Read a whole file whose size is known: one open, no question about its size.
fn read_sized(path: &Path, size: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::with_capacity(size as usize + 1);
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn load_file(path: &Path, rel_path: String, size: Option<u64>, opts: &ScanOptions) -> SourceFile {
    let mut t = Instant::now();
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

/// Normalise and analyse a file's contents.
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
    SourceFile { name: file_name(&rel_path), rel_path, lang, text, line_starts, analysis, max_cols, note }
}

/// Parallel directory walk: every walker thread gets a [`Visitor`] that
/// collects paths locally and hands them over in batches.
/// A file found by the walk: its path, its path relative to the root, and
/// its size when the directory listing gave it for free (it does on Windows;
/// asking again costs one system call per file, which adds up to seconds).
type Found = (PathBuf, String, Option<u64>);

/// Extensions of files that are never text: they become a card without the
/// file being opened at all.
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "icns", "cur", "webp", "avif", "tif", "tiff", "psd", "woff", "woff2",
    "ttf", "otf", "eot", "pdf", "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "jar", "war", "apk", "aab", "crx",
    "exe", "dll", "so", "dylib", "a", "lib", "o", "obj", "pdb", "class", "wasm", "pyc", "mp3", "mp4", "m4a", "aac",
    "wav", "ogg", "opus", "flac", "webm", "mov", "avi", "mkv", "sqlite", "db", "pak", "nib", "keystore",
];

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
        // Free where the listing carries it; never an extra system call.
        let size = if cfg!(windows) { entry.metadata().ok().map(|m| m.len()) } else { None };
        self.local.push((entry.into_path(), rel, size));
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

/// Scan `root`. Safe to call from a background thread; `progress` lets the UI
/// show counters while it runs.
pub fn scan(root: &Path, opts: &ScanOptions, progress: Option<Arc<ScanProgress>>) -> Repo {
    let started = Instant::now();
    let root = clean_path(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    // Two cores stay free: a scan of a huge folder must not starve the
    // window that is showing its progress (or anything else on the machine).
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).thread_name(|i| format!("scan {i}")).build();

    // 1. Discover paths (fast, single-threaded, sorted for a stable layout).
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
    walker.visit(&mut builder);
    let mut paths = shared.into_inner().unwrap();
    // Component-wise order (like a file explorer), independent of thread timing.
    paths.sort_unstable_by(|a, b| Path::new(&a.1).cmp(Path::new(&b.1)));
    if paths.len() > opts.max_files {
        log::warn!("file limit {} reached, the rest is skipped", opts.max_files);
        paths.truncate(opts.max_files);
    }

    let walk_ms = started.elapsed().as_millis();
    let t_load = Instant::now();
    // 2. Read + tokenise in parallel, keeping the original order.
    let load = || -> Vec<SourceFile> {
        paths
            .par_iter()
            .map(|(path, rel, size)| {
                let f = load_file(path, rel.clone(), *size, opts);
                if let Some(p) = &progress {
                    p.loaded.fetch_add(1, Ordering::Relaxed);
                }
                f
            })
            .collect()
    };
    let files = match &pool {
        Ok(pool) => pool.install(load),
        Err(_) => load(),
    };

    let load_ms = t_load.elapsed().as_millis();
    let t_tree = Instant::now();
    // 3. Build the directory tree.
    let mut tree = Dir { name: name.clone(), rel_path: String::new(), ..Default::default() };
    for (idx, f) in files.iter().enumerate() {
        let mut dir = &mut tree;
        let mut parts: Vec<&str> = f.rel_path.split('/').collect();
        parts.pop(); // the file name
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
}
