use std::path::Path;
use std::sync::Arc;

use rayon::prelude::*;

use crate::deps::{DepKind, FileDeps};
use crate::scan::{ident_filter, three_filter, ScanOptions, SourceFile};
use crate::symbols::{Symbol, SymbolKind};
use crate::syntax::{Analysis, Block, TokenKind};

const MAGIC: &[u8; 4] = b"CCSC";
const FORMAT: u32 = 2;
const BATCH: usize = 4096;

const SYMBOL_KINDS: [SymbolKind; 11] = [
    SymbolKind::Function,
    SymbolKind::Method,
    SymbolKind::Class,
    SymbolKind::Struct,
    SymbolKind::Enum,
    SymbolKind::Trait,
    SymbolKind::Interface,
    SymbolKind::Type,
    SymbolKind::Const,
    SymbolKind::Module,
    SymbolKind::Impl,
];

const TOKEN_KINDS: [TokenKind; 11] = [
    TokenKind::Keyword,
    TokenKind::Type,
    TokenKind::Function,
    TokenKind::Macro,
    TokenKind::String,
    TokenKind::Number,
    TokenKind::Comment,
    TokenKind::Punct,
    TokenKind::Ident,
    TokenKind::Heading,
    TokenKind::Text,
];

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }

    fn words(&mut self, v: &[u64]) {
        self.u32(v.len() as u32);
        for word in v {
            self.u64(*word);
        }
    }
}

struct In<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let part = self.bytes.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(part)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    fn text(&mut self) -> Option<&'a str> {
        std::str::from_utf8(self.bytes()?).ok()
    }

    fn words(&mut self) -> Option<Box<[u64]>> {
        let n = self.u32()? as usize;
        let raw = self.take(n.checked_mul(8)?)?;
        Some(raw.chunks_exact(8).map(|w| u64::from_le_bytes(w.try_into().unwrap())).collect())
    }
}

fn made_with(opts: &ScanOptions) -> Vec<u8> {
    let mut out = Out::default();
    out.u32(FORMAT);
    out.bytes(env!("CARGO_PKG_VERSION").as_bytes());
    out.u32(crate::TAB_WIDTH as u32);
    out.u64(opts.max_file_bytes);
    out.u64(opts.max_lines as u64);
    out.0
}

pub fn names_hash<'a>(paths: impl Iterator<Item = &'a str>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for path in paths {
        for &byte in path.as_bytes().iter().chain(&[0u8]) {
            hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

fn encode(f: &SourceFile, deps: &FileDeps) -> Vec<u8> {
    let mut out = Out::default();
    out.u32(f.line_count() as u32);
    out.u32(f.max_cols);
    match &f.note {
        Some(note) => {
            out.u8(1);
            out.bytes(note.as_bytes());
        }
        None => out.u8(0),
    }
    out.u32(f.analysis.blocks.len() as u32);
    for block in &f.analysis.blocks {
        out.u16(block.lo);
        out.u16(block.hi);
        out.u8(block.kind as u8);
    }
    out.u32(f.analysis.chunk_in_comment.len() as u32);
    for inside in &f.analysis.chunk_in_comment {
        out.u8(*inside as u8);
    }
    out.u32(f.analysis.symbols.len() as u32);
    for (symbol, at) in f.analysis.symbols.iter().zip(f.name_at.iter()) {
        out.u8(symbol.kind as u8);
        out.u32(symbol.name_start);
        out.u16(symbol.name_len);
        out.u16(symbol.col);
        out.u32(symbol.line);
        out.u32(symbol.body_end);
        out.u32(symbol.parent);
        out.u32(*at);
    }
    out.u32(f.analysis.impl_traits.len() as u32);
    for &(symbol, start, len) in &f.analysis.impl_traits {
        out.u32(symbol);
        out.u32(start);
        out.u16(len);
    }
    out.bytes(f.names.as_bytes());
    out.u32(f.implemented.len() as u32);
    for (symbol, name) in f.implemented.iter() {
        out.u32(*symbol);
        out.bytes(name.as_bytes());
    }
    match (f.idents.is_empty(), f.has_text()) {
        (true, true) => {
            out.words(&ident_filter(f.text()));
            out.words(&three_filter(f.text()));
        }
        _ => {
            out.words(&f.idents);
            out.words(&f.threes);
        }
    }
    out.u32(deps.edges.len() as u32);
    for &(to, line, kind) in &deps.edges {
        out.u32(to);
        out.u32(line);
        out.u8(kind.index() as u8);
    }
    out.u32(deps.unresolved);
    out.u32(deps.external);
    out.0
}

fn decode(record: &[u8], rel_path: &str, root: &Arc<Path>, stamp: (u64, u64)) -> Option<(SourceFile, FileDeps)> {
    let mut r = In { bytes: record, at: 0 };
    let lines = r.u32()?;
    let max_cols = r.u32()?;
    let note = match r.u8()? {
        0 => None,
        _ => Some(r.text()?.to_string()),
    };
    let mut analysis = Analysis::default();
    for _ in 0..r.u32()? {
        let (lo, hi) = (r.u16()?, r.u16()?);
        analysis.blocks.push(Block { lo, hi, kind: *TOKEN_KINDS.get(r.u8()? as usize)? });
    }
    for _ in 0..r.u32()? {
        analysis.chunk_in_comment.push(r.u8()? != 0);
    }
    let symbols = r.u32()? as usize;
    let mut name_at = Vec::with_capacity(symbols.min(1 << 20));
    for _ in 0..symbols {
        analysis.symbols.push(Symbol {
            kind: *SYMBOL_KINDS.get(r.u8()? as usize)?,
            name_start: r.u32()?,
            name_len: r.u16()?,
            col: r.u16()?,
            line: r.u32()?,
            body_end: r.u32()?,
            parent: r.u32()?,
        });
        name_at.push(r.u32()?);
    }
    for _ in 0..r.u32()? {
        analysis.impl_traits.push((r.u32()?, r.u32()?, r.u16()?));
    }
    let names: Box<str> = r.text()?.into();
    let whole = analysis.symbols.iter().zip(&name_at).all(|(symbol, at)| {
        let end = *at as usize + symbol.name_len as usize;
        names.is_char_boundary(*at as usize) && names.is_char_boundary(end) && end <= names.len()
    });
    if !whole {
        return None;
    }
    let mut implemented = Vec::new();
    for _ in 0..r.u32()? {
        implemented.push((r.u32()?, Box::<str>::from(r.text()?)));
    }
    let (idents, threes) = (r.words()?, r.words()?);
    if !idents.len().is_power_of_two() && !idents.is_empty() || !threes.len().is_power_of_two() && !threes.is_empty() {
        return None;
    }
    let mut deps = FileDeps::default();
    for _ in 0..r.u32()? {
        let (to, line) = (r.u32()?, r.u32()?);
        deps.edges.push((to, line, *DepKind::ALL.get(r.u8()? as usize)?));
    }
    (deps.unresolved, deps.external) = (r.u32()?, r.u32()?);
    let mut file = SourceFile::kept(rel_path.to_string(), root.clone(), lines, max_cols, note, analysis);
    (file.names, file.name_at, file.implemented) = (names, name_at.into(), implemented.into());
    (file.idents, file.threes, file.stamp) = (idents, threes, stamp);
    Some((file, deps))
}

fn wanted(f: &SourceFile) -> bool {
    f.stamp != (0, 0) && f.note.as_ref().is_none_or(|note| !note.starts_with("unreadable"))
}

fn write(path: &Path, files: &[SourceFile], deps: &[FileDeps], opts: &ScanOptions) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let unfinished = path.with_extension("unfinished");
    let mut file = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&unfinished)?);
    let mut head = Out::default();
    head.0.extend_from_slice(MAGIC);
    head.bytes(&made_with(opts));
    head.u64(names_hash(files.iter().map(|f| f.rel_path.as_str())));
    head.u32(files.len() as u32);
    file.write_all(&head.0)?;
    let nothing = FileDeps::default();
    let mut at = head.0.len() as u64;
    let mut table = Out::default();
    let mut count = 0u32;
    for (batch, some) in files.chunks(BATCH).enumerate() {
        let first = batch * BATCH;
        let records: Vec<Option<Vec<u8>>> = some
            .par_iter()
            .enumerate()
            .map(|(i, f)| wanted(f).then(|| encode(f, deps.get(first + i).unwrap_or(&nothing))))
            .collect();
        for (f, record) in some.iter().zip(&records) {
            let Some(record) = record else { continue };
            file.write_all(record)?;
            table.bytes(f.rel_path.as_bytes());
            table.u64(f.stamp.0);
            table.u64(f.stamp.1);
            table.u64(at);
            table.u32(record.len() as u32);
            at += record.len() as u64;
            count += 1;
        }
    }
    file.write_all(&count.to_le_bytes())?;
    file.write_all(&table.0)?;
    file.write_all(&at.to_le_bytes())?;
    file.into_inner().map_err(|e| e.into_error())?.sync_data().ok();
    std::fs::rename(&unfinished, path)
}

fn read_at(file: &std::fs::File, buf: &mut [u8], at: u64) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            match file.seek_read(&mut buf[done..], at + done as u64)? {
                0 => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                n => done += n,
            }
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, at)
    }
}

struct Entry {
    path: Box<str>,
    stamp: (u64, u64),
    at: u64,
    len: u32,
}

pub struct Kept {
    file: std::fs::File,
    entries: Vec<Entry>,
    names: u64,
    files: usize,
}

impl Kept {
    pub fn read(path: &Path, opts: &ScanOptions) -> Option<Kept> {
        let file = std::fs::File::open(path).ok()?;
        let size = file.metadata().ok()?.len();
        let made = made_with(opts);
        let mut head = vec![0u8; MAGIC.len() + 4 + made.len() + 8 + 4];
        read_at(&file, &mut head, 0).ok()?;
        let mut r = In { bytes: &head, at: 0 };
        if r.take(4)? != MAGIC || r.bytes()? != made {
            return None;
        }
        let names = r.u64()?;
        let files = r.u32()? as usize;
        let mut end = [0u8; 8];
        read_at(&file, &mut end, size.checked_sub(8)?).ok()?;
        let table_at = u64::from_le_bytes(end);
        let mut table = vec![0u8; usize::try_from((size - 8).checked_sub(table_at)?).ok()?];
        read_at(&file, &mut table, table_at).ok()?;
        let mut r = In { bytes: &table, at: 0 };
        let count = r.u32()? as usize;
        let mut entries = Vec::with_capacity(count.min(1 << 22));
        for _ in 0..count {
            let path: Box<str> = r.text()?.into();
            let stamp = (r.u64()?, r.u64()?);
            let (at, len) = (r.u64()?, r.u32()?);
            if at + len as u64 > table_at {
                return None;
            }
            entries.push(Entry { path, stamp, at, len });
        }
        Some(Kept { file, entries, names, files })
    }

    pub fn same_files<'a>(&self, count: usize, paths: impl Iterator<Item = &'a str>) -> bool {
        count == self.files && names_hash(paths) == self.names
    }

    pub fn file(&self, rel_path: &str, stamp: (u64, u64), root: &Arc<Path>) -> Option<(SourceFile, FileDeps)> {
        let at = self.entries.binary_search_by(|e| Path::new(&*e.path).cmp(Path::new(rel_path))).ok()?;
        let entry = &self.entries[at];
        if entry.stamp != stamp {
            return None;
        }
        let mut record = vec![0u8; entry.len as usize];
        read_at(&self.file, &mut record, entry.at).ok()?;
        decode(&record, rel_path, root, stamp)
    }
}

pub fn keep(repo: &mut crate::scan::Repo, deps: &[FileDeps], opts: &ScanOptions) -> std::io::Result<bool> {
    let nothing_new = !repo.kept_deps.is_empty() && repo.kept_deps.iter().all(Option::is_some);
    repo.kept_deps = Vec::new();
    let Some(path) = &opts.cache else { return Ok(false) };
    if nothing_new {
        let kept = std::fs::File::options().write(true).open(path)?;
        kept.set_modified(std::time::SystemTime::now())?;
        return Ok(false);
    }
    repo.make_filters();
    write(path, &repo.files, deps, opts)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan;

    #[test]
    fn a_folder_opened_again_reads_only_what_changed() {
        let folder = std::env::temp_dir().join(format!("code-canvas-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("src")).unwrap();
        let put = |name: &str, text: &str| std::fs::write(folder.join("src").join(name), text).unwrap();
        put("a.rs", "mod b;\n\nfn main() {\n    b::go();\n}\n");
        put("b.rs", "pub fn go() {}\n");
        put("notes.txt", "nothing here\n");
        let opts = ScanOptions { cache: Some(folder.join("kept.bin")), ..Default::default() };
        let open = |opts: &ScanOptions| {
            let mut repo = scan(&folder.join("src"), opts, None);
            let reused_deps = repo.kept_deps.iter().filter(|kept| kept.is_some()).count();
            let (deps, per_file) = crate::deps::analyze_keeping(&repo);
            keep(&mut repo, &per_file, opts).unwrap();
            (repo, deps, reused_deps)
        };

        let (first, first_deps, _) = open(&opts);
        assert_eq!((first.reused, first.files.len()), (0, 3));
        assert_eq!(first_deps.edges.len(), 1, "a.rs declares the module b.rs is");

        let (again, again_deps, reused_deps) = open(&opts);
        assert_eq!((again.reused, reused_deps), (3, 3));
        assert!(again.files.iter().all(|f| !f.has_text()), "no file was opened");
        assert_eq!(again_deps.edges, first_deps.edges);
        assert_eq!(again.symbols.count, first.symbols.count);
        assert_eq!(again.symbols.defs("go").len(), 1);
        for (was, is) in first.files.iter().zip(&again.files) {
            assert_eq!((was.line_count(), was.max_cols), (is.line_count(), is.max_cols));
            assert_eq!(was.analysis.blocks, is.analysis.blocks);
            assert_eq!(was.analysis.symbols, is.analysis.symbols);
            assert_eq!(was.text(), is.text(), "and the text is there when it is asked for");
        }
        assert!(again.files[1].may_contain("go") && again.files[1].may_hold(b"pub fn go"));

        put("b.rs", "pub fn go() {}\n\npub fn stop() {}\n");
        let (changed, _, reused_deps) = open(&opts);
        assert_eq!((changed.reused, reused_deps), (2, 2), "only the file that changed was read");
        assert_eq!(changed.symbols.defs("stop").len(), 1);

        put("c.rs", "pub fn more() {}\n");
        let (grown, grown_deps, reused_deps) = open(&opts);
        assert_eq!((grown.reused, reused_deps), (3, 0), "with another set of files every import is resolved again");
        assert_eq!(grown_deps.edges.len(), 1);

        let fresh = ScanOptions { reuse: false, ..opts.clone() };
        assert_eq!(open(&fresh).0.reused, 0, "asked to read everything again");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
