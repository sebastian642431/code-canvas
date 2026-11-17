use std::collections::{HashMap, HashSet};
use std::time::Instant;

use rayon::prelude::*;

use crate::scan::{Repo, SourceFile};
use crate::symbols::{symbol_at_line, Symbol, SymbolKind};
use crate::syntax::{Token, TokenKind, Tokens, CHUNK_LINES};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymRef {
    pub file: u32,
    pub index: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Site {
    pub file: u32,
    pub line: u32,
    pub col: u16,
    pub len: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelKind {
    Definition,
    Reference,
    IncomingCall,
    OutgoingCall,
    Implementation,
}

impl RelKind {
    pub const ALL: [RelKind; 5] = [
        RelKind::Definition,
        RelKind::Implementation,
        RelKind::Reference,
        RelKind::IncomingCall,
        RelKind::OutgoingCall,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            RelKind::Definition => "Definitions",
            RelKind::Reference => "References",
            RelKind::IncomingCall => "Incoming calls",
            RelKind::OutgoingCall => "Outgoing calls",
            RelKind::Implementation => "Implementations",
        }
    }

    pub fn index(&self) -> usize {
        match self {
            RelKind::Definition => 0,
            RelKind::Implementation => 1,
            RelKind::Reference => 2,
            RelKind::IncomingCall => 3,
            RelKind::OutgoingCall => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relation {
    pub kind: RelKind,
    pub from: Site,
    pub to: Site,
    pub other: Option<SymRef>,
}

pub struct Relations {
    pub target: SymRef,
    pub items: Vec<Relation>,
    pub truncated: bool,
    pub search_ms: f32,
}

impl Relations {
    pub fn count(&self, kind: RelKind) -> usize {
        self.items.iter().filter(|r| r.kind == kind).count()
    }
}

pub const MAX_PER_KIND: usize = 1500;

#[derive(Default)]
pub struct SymbolIndex {
    by_name: HashMap<Box<str>, Vec<SymRef>>,
    impls: HashMap<Box<str>, Vec<SymRef>>,
    pub count: usize,
}

impl SymbolIndex {
    pub fn build(files: &[SourceFile]) -> Self {
        let mut idx = SymbolIndex::default();
        for (fi, f) in files.iter().enumerate() {
            for (si, s) in f.analysis.symbols.iter().enumerate() {
                let r = SymRef { file: fi as u32, index: si as u32 };
                let name = f.symbol_name(si);
                if s.kind == SymbolKind::Impl {
                    idx.impls.entry(name.into()).or_default().push(r);
                } else {
                    idx.by_name.entry(name.into()).or_default().push(r);
                }
                idx.count += 1;
            }
            for (sym, name) in f.implemented() {
                idx.impls.entry(name.into()).or_default().push(SymRef { file: fi as u32, index: sym });
            }
        }
        idx
    }

    pub fn defs(&self, name: &str) -> &[SymRef] {
        self.by_name.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn impls(&self, name: &str) -> &[SymRef] {
        self.impls.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
}

pub(crate) fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn find_all(hay: &[u8], needle: &[u8], ignore_case: bool, mut found: impl FnMut(usize) -> bool) {
    let n = needle.len();
    if n == 0 || hay.len() < n {
        return;
    }
    let first = needle[0];
    let first_upper = first.to_ascii_uppercase();
    let mut i = 0;
    let last = hay.len() - n;
    while i <= last {
        let b = hay[i];
        if b == first || (ignore_case && b == first_upper) {
            let window = &hay[i..i + n];
            let hit = if ignore_case { window.eq_ignore_ascii_case(needle) } else { window == needle };
            if hit {
                if !found(i) {
                    return;
                }
                i += n;
                continue;
            }
        }
        i += 1;
    }
}

fn shared_dirs(a: &str, b: &str) -> usize {
    let (a, b) = (a.rsplit_once('/').map(|x| x.0).unwrap_or(""), b.rsplit_once('/').map(|x| x.0).unwrap_or(""));
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    a.split('/').zip(b.split('/')).take_while(|(x, y)| x == y).count()
}

fn usable(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Ident | TokenKind::Function | TokenKind::Type | TokenKind::Macro | TokenKind::Keyword)
}

fn is_call(text: &str, toks: &[Token], k: usize) -> bool {
    let t = &toks[k];
    match t.kind {
        TokenKind::Function | TokenKind::Macro => true,
        TokenKind::Keyword => {
            let qualified = k > 0
                && toks[k - 1].kind == TokenKind::Punct
                && matches!(&text[toks[k - 1].range()], "::" | "." | "->" | "?.");
            let called =
                toks.get(k + 1).is_some_and(|n| n.kind == TokenKind::Punct && text[n.range()].starts_with('('));
            qualified && called
        }
        _ => false,
    }
}

impl Repo {
    pub fn sym(&self, r: SymRef) -> &Symbol {
        &self.files[r.file as usize].analysis.symbols[r.index as usize]
    }

    pub fn sym_name(&self, r: SymRef) -> &str {
        self.files[r.file as usize].symbol_name(r.index as usize)
    }

    pub fn sym_site(&self, r: SymRef) -> Site {
        let s = self.sym(r);
        let len = self.sym_name(r).chars().count().min(u16::MAX as usize) as u16;
        Site { file: r.file, line: s.line, col: s.col, len }
    }

    pub fn sym_line_start(&self, r: SymRef) -> Site {
        let f = &self.files[r.file as usize];
        let s = self.sym(r);
        let indent = f.line(s.line as usize).chars().take_while(|c| *c == ' ').count();
        Site { file: r.file, line: s.line, col: indent.min(u16::MAX as usize) as u16, len: 0 }
    }

    pub fn sym_container(&self, r: SymRef) -> Option<&str> {
        let f = &self.files[r.file as usize];
        let mut p = self.sym(r).parent;
        while p != u32::MAX {
            let s = &f.analysis.symbols[p as usize];
            if s.kind.is_container() || s.kind == SymbolKind::Module {
                return Some(f.symbol_name(p as usize));
            }
            p = s.parent;
        }
        None
    }

    pub fn sym_signature(&self, r: SymRef) -> String {
        let f = &self.files[r.file as usize];
        let s = self.sym(r);
        let mut out = String::new();
        for line in s.line..(s.line + 6).min(s.body_end).min(f.line_count() as u32) {
            let text = f.line(line as usize).trim();
            if !out.is_empty() {
                out.push(' ');
            }
            let cut = text.find('{').or_else(|| text.strip_suffix(':').map(|t| t.len()));
            match cut {
                Some(c) => {
                    out.push_str(text[..c].trim_end());
                    break;
                }
                None => out.push_str(text),
            }
            if text.ends_with(';') || out.len() > 400 {
                break;
            }
        }
        out
    }

    pub fn sym_at(&self, file: u32, line: u32) -> Option<SymRef> {
        symbol_at_line(&self.files[file as usize].analysis.symbols, line)
            .map(|index| SymRef { file, index: index as u32 })
    }

    pub fn identifier_at(&self, file: u32, line: u32, col: u32) -> Option<(Site, &str)> {
        let f = &self.files[file as usize];
        if line as usize >= f.line_count() {
            return None;
        }
        let chunk = line as usize / CHUNK_LINES;
        let tokens = f.tokenize_chunk(chunk);
        let toks = tokens.line(line as usize - chunk * CHUNK_LINES);
        let t = toks.iter().find(|t| (t.col as u32) <= col && col < t.col as u32 + t.cols as u32)?;
        if !usable(t) || t.kind == TokenKind::Keyword {
            return None;
        }
        let mut name = &f.text()[t.range()];
        let mut len = t.cols;
        if let Some(stripped) = name.strip_suffix('!') {
            name = stripped;
            len -= 1;
        }
        Some((Site { file, line, col: t.col, len }, name))
    }

    pub fn search_text(&self, query: &str, max: usize) -> (Vec<Site>, bool) {
        if query.is_empty() {
            return (Vec::new(), false);
        }
        let ignore_case = !query.chars().any(|c| c.is_uppercase());
        let needle = query.as_bytes();
        let cols = query.chars().count().min(u16::MAX as usize) as u16;
        let found = std::sync::atomic::AtomicUsize::new(0);
        let per_file: Vec<Vec<Site>> = self
            .files
            .par_iter()
            .enumerate()
            .filter_map(|(fi, f)| {
                let mut hits = Vec::new();
                let enough = found.load(std::sync::atomic::Ordering::Relaxed) >= max;
                if !f.may_hold(needle) || (enough && !f.has_text()) {
                    return None;
                }
                f.peek(|text, line_starts| {
                    find_all(text.as_bytes(), needle, ignore_case, |off| {
                        let line = line_starts.partition_point(|&s| s as usize <= off) - 1;
                        let start = line_starts[line] as usize;
                        let col = text.get(start..off).map(|t| t.chars().count()).unwrap_or(0);
                        hits.push(Site {
                            file: fi as u32,
                            line: line as u32,
                            col: col.min(u16::MAX as usize) as u16,
                            len: cols,
                        });
                        hits.len() < max
                    })
                });
                found.fetch_add(hits.len(), std::sync::atomic::Ordering::Relaxed);
                (!hits.is_empty()).then_some(hits)
            })
            .collect();
        let total: usize = per_file.iter().map(Vec::len).sum();
        let mut out: Vec<Site> = per_file.into_iter().flatten().take(max).collect();
        out.shrink_to_fit();
        (out, total > max)
    }

    pub fn resolve_at(&self, file: u32, line: u32, col: u32) -> Option<(SymRef, Site)> {
        let f = &self.files[file as usize];
        if line as usize >= f.line_count() {
            return None;
        }
        let chunk = line as usize / CHUNK_LINES;
        let tokens = f.tokenize_chunk(chunk);
        let toks = tokens.line(line as usize - chunk * CHUNK_LINES);
        let k = toks.iter().position(|t| (t.col as u32) <= col && col < t.col as u32 + t.cols as u32)?;
        let t = toks[k];
        if !usable(&t) {
            return None;
        }
        let name = f.text()[t.range()].trim_end_matches('!');
        let site = Site { file, line, col: t.col, len: name.chars().count().min(u16::MAX as usize) as u16 };
        if let Some(index) = f.analysis.symbols.iter().position(|s| s.name_start == t.start) {
            return Some((SymRef { file, index: index as u32 }, site));
        }
        let call = is_call(&f.text(), toks, k);
        if t.kind == TokenKind::Keyword && !call {
            return None;
        }
        let q = self.qualifier(f, file, line, toks, k);
        self.resolve(name, file, q, call).map(|s| (s, site))
    }

    pub fn resolve(&self, name: &str, from_file: u32, qualifier: Option<&str>, call: bool) -> Option<SymRef> {
        let cands = self.symbols.defs(name);
        match cands.len() {
            0 => return None,
            1 => return Some(cands[0]),
            _ => {}
        }
        if let Some(q) = qualifier {
            let scoped: Vec<SymRef> = cands.iter().copied().filter(|&c| self.sym_container(c) == Some(q)).collect();
            if !scoped.is_empty() {
                return self.nearest(&scoped, from_file, call);
            }
        }
        self.nearest(cands, from_file, call)
    }

    fn nearest(&self, cands: &[SymRef], from_file: u32, call: bool) -> Option<SymRef> {
        let from = &self.files[from_file as usize];
        let check_imports = cands.len() <= 16;
        cands.iter().copied().max_by_key(|c| {
            let f = &self.files[c.file as usize];
            let mut score = 0usize;
            if c.file == from_file {
                score += 100_000;
            }
            score += shared_dirs(&from.rel_path, &f.rel_path) * 100;
            if self.sym(*c).kind.is_callable() == call {
                score += 30;
            }
            if check_imports && c.file != from_file {
                let stem = f.name.split('.').next().unwrap_or("");
                if stem.len() >= 3 && !matches!(stem, "mod" | "index" | "lib" | "main") && from.text().contains(stem) {
                    score += 1000;
                }
            }
            (score, std::cmp::Reverse(c.file), std::cmp::Reverse(c.index))
        })
    }

    fn qualifier<'a>(&'a self, f: &'a SourceFile, file: u32, line: u32, toks: &[Token], k: usize) -> Option<&'a str> {
        if k < 2 {
            return None;
        }
        let sep = &f.text()[toks[k - 1].range()];
        if toks[k - 1].kind != TokenKind::Punct || !(sep == "::" || sep == "." || sep == "->" || sep == "?.") {
            return None;
        }
        let q = &toks[k - 2];
        if !usable(q) {
            return None;
        }
        let word = &f.text()[q.range()];
        if matches!(word, "Self" | "self" | "this" | "super" | "cls") {
            let at = self.sym_at(file, line)?;
            return self.sym_container(at).or_else(|| {
                let s = self.sym(at);
                s.kind.is_container().then(|| self.sym_name(at))
            });
        }
        (sep == "::" || word.chars().next().is_some_and(char::is_uppercase)).then_some(word)
    }

    pub fn relations(&self, target: SymRef, clicked: Option<Site>) -> Relations {
        let started = Instant::now();
        let def = self.sym_site(target);
        let name = self.sym_name(target).to_string();
        let container = self.sym_container(target).map(str::to_string);
        let tsym = *self.sym(target);
        let mut items = vec![Relation {
            kind: RelKind::Definition,
            from: clicked.filter(|c| *c != def).unwrap_or(def),
            to: def,
            other: None,
        }];

        let many_defs = self.symbols.defs(&name).len() > 1;
        let found: Vec<Vec<Relation>> = self
            .files
            .par_iter()
            .enumerate()
            .filter_map(|(fi, f)| {
                if !f.may_contain(&name) || f.text().len() < name.len() || !f.text().contains(name.as_str()) {
                    return None;
                }
                let fi = fi as u32;
                let bytes = f.text().as_bytes();
                let mut chunks: HashMap<usize, Tokens> = HashMap::new();
                let mut default_binding: Option<Option<SymRef>> = None;
                let mut out = Vec::new();
                for (off, _) in f.text().match_indices(name.as_str()) {
                    let end = off + name.len();
                    if (off > 0 && ident_byte(bytes[off - 1])) || (end < bytes.len() && ident_byte(bytes[end])) {
                        continue;
                    }
                    let line = f.line_starts().partition_point(|&s| s as usize <= off) - 1;
                    let chunk = line / CHUNK_LINES;
                    let tokens = chunks.entry(chunk).or_insert_with(|| f.tokenize_chunk(chunk));
                    let toks = tokens.line(line - chunk * CHUNK_LINES);
                    let Some(k) = toks.iter().position(|t| t.start as usize == off) else { continue };
                    let t = toks[k];
                    if !usable(&t) {
                        continue;
                    }
                    if f.analysis.symbols.iter().any(|s| s.name_start as usize == off) {
                        continue;
                    }
                    if many_defs {
                        let q = self.qualifier(f, fi, line as u32, toks, k);
                        let bound = match q {
                            Some(q) if self.symbols.defs(&name).iter().any(|&c| self.sym_container(c) == Some(q)) => {
                                (container.as_deref() == Some(q)).then_some(target)
                            }
                            _ => *default_binding
                                .get_or_insert_with(|| self.resolve(&name, fi, None, tsym.kind.is_callable())),
                        };
                        if bound != Some(target) {
                            continue;
                        }
                    }
                    let call = is_call(&f.text(), toks, k);
                    let enclosing = symbol_at_line(&f.analysis.symbols, line as u32)
                        .map(|index| SymRef { file: fi, index: index as u32 });
                    out.push(Relation {
                        kind: if call { RelKind::IncomingCall } else { RelKind::Reference },
                        from: Site { file: fi, line: line as u32, col: t.col, len: def.len },
                        to: def,
                        other: enclosing,
                    });
                }
                (!out.is_empty()).then_some(out)
            })
            .collect();

        let mut truncated = false;
        let mut per_kind = [0usize; 5];
        for r in found.into_iter().flatten() {
            let n = &mut per_kind[r.kind.index()];
            if *n >= MAX_PER_KIND {
                truncated = true;
                continue;
            }
            *n += 1;
            items.push(r);
        }

        let f = &self.files[target.file as usize];
        let mut seen: HashSet<SymRef> = HashSet::new();
        let body_end = tsym.body_end.min(tsym.line + 5000).min(f.line_count() as u32);
        let mut chunk_cache: Option<(usize, Tokens)> = None;
        for line in tsym.line..body_end {
            let chunk = line as usize / CHUNK_LINES;
            if chunk_cache.as_ref().map(|c| c.0) != Some(chunk) {
                chunk_cache = Some((chunk, f.tokenize_chunk(chunk)));
            }
            let toks = chunk_cache.as_ref().unwrap().1.line(line as usize - chunk * CHUNK_LINES);
            for (k, t) in toks.iter().enumerate() {
                if !is_call(&f.text(), toks, k) {
                    continue;
                }
                if f.analysis.symbols.iter().any(|s| s.name_start == t.start) {
                    continue;
                }
                let callee = f.text()[t.range()].trim_end_matches('!');
                let q = self.qualifier(f, target.file, line, toks, k);
                let Some(d) = self.resolve(callee, target.file, q, true) else { continue };
                if d == target || !seen.insert(d) {
                    continue;
                }
                if seen.len() > MAX_PER_KIND {
                    truncated = true;
                    break;
                }
                items.push(Relation {
                    kind: RelKind::OutgoingCall,
                    from: Site { file: target.file, line, col: t.col, len: callee.chars().count() as u16 },
                    to: self.sym_site(d),
                    other: Some(d),
                });
            }
        }

        if !tsym.kind.is_callable() {
            for &imp in self.symbols.impls(&name) {
                if many_defs && self.resolve(&name, imp.file, None, false) != Some(target) {
                    continue;
                }
                items.push(Relation {
                    kind: RelKind::Implementation,
                    from: self.sym_site(imp),
                    to: def,
                    other: Some(imp),
                });
            }
        }

        Relations { target, items, truncated, search_ms: started.elapsed().as_secs_f32() * 1000.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{scan, ScanOptions};

    fn repo(files: &[(&str, &str)]) -> Repo {
        let dir = std::env::temp_dir().join(format!("code-canvas-refs-{}-{}", std::process::id(), files.len()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let repo = scan(&dir, &ScanOptions::default(), None);
        std::fs::remove_dir_all(&dir).ok();
        repo
    }

    fn file(repo: &Repo, name: &str) -> u32 {
        repo.files.iter().position(|f| f.rel_path == name).unwrap() as u32
    }

    #[test]
    fn finds_calls_references_and_callees() {
        let repo = repo(&[
            (
                "src/atlas.rs",
                "pub struct Atlas;\n\nimpl Atlas {\n    pub fn new() -> Self { Atlas }\n}\n\npub fn rasterize(a: &Atlas) -> u32 {\n    helper(1)\n}\n\nfn helper(x: u32) -> u32 { x }\n",
            ),
            (
                "src/pipeline.rs",
                "use crate::atlas::{rasterize, Atlas};\n\npub fn glyph_atlas() -> u32 {\n    let a = Atlas::new();\n    // rasterize in a comment is ignored\n    rasterize(&a)\n}\n",
            ),
            ("other/lib.rs", "pub struct Thing;\nimpl Thing {\n    pub fn new() -> Self { Thing }\n}\nfn make() -> Thing { Thing::new() }\n"),
        ]);
        let atlas = file(&repo, "src/atlas.rs");
        let pipeline = file(&repo, "src/pipeline.rs");

        let (site, name) = repo.identifier_at(pipeline, 5, 6).unwrap();
        assert_eq!(name, "rasterize");
        let target = repo.resolve(name, pipeline, None, true).unwrap();
        assert_eq!(target.file, atlas);
        assert_eq!(repo.sym_name(target), "rasterize");

        let rel = repo.relations(target, Some(site));
        assert_eq!(rel.count(RelKind::Definition), 1);
        assert_eq!(rel.count(RelKind::IncomingCall), 1, "{:?}", rel.items);
        assert_eq!(rel.count(RelKind::Reference), 1, "the `use` line");
        assert_eq!(rel.count(RelKind::OutgoingCall), 1, "calls helper");
        let call = rel.items.iter().find(|r| r.kind == RelKind::IncomingCall).unwrap();
        assert_eq!(repo.sym_name(call.other.unwrap()), "glyph_atlas");
        let out = rel.items.iter().find(|r| r.kind == RelKind::OutgoingCall).unwrap();
        assert_eq!(repo.sym_name(out.other.unwrap()), "helper");

        let news = repo.symbols.defs("new");
        assert_eq!(news.len(), 2);
        let atlas_new = *news.iter().find(|r| r.file == atlas).unwrap();
        let rel = repo.relations(atlas_new, None);
        assert_eq!(rel.count(RelKind::IncomingCall), 1);
        let call = rel.items.iter().find(|r| r.kind == RelKind::IncomingCall).unwrap();
        assert_eq!(call.from.file, pipeline);

        let atlas_struct = repo.resolve("Atlas", pipeline, None, false).unwrap();
        let rel = repo.relations(atlas_struct, None);
        assert_eq!(rel.count(RelKind::Implementation), 1);
        assert!(rel.count(RelKind::Reference) >= 3);
        assert_eq!(repo.sym_signature(target), "pub fn rasterize(a: &Atlas) -> u32");

        assert_eq!(repo.resolve_at(pipeline, 5, 6).map(|r| r.0), Some(target));
        assert_eq!(repo.resolve_at(pipeline, 3, 20).map(|r| r.0), Some(atlas_new), "Atlas::new()");
        assert_eq!(repo.resolve_at(atlas, 6, 8).map(|r| r.0), Some(target), "the definition itself");
        assert_eq!(repo.resolve_at(pipeline, 2, 0), None, "`pub` is a keyword");

        let (hits, more) = repo.search_text("rasterize", 100);
        assert!(!more);
        assert_eq!(hits.len(), 4, "definition, import, comment, call");
        assert!(hits.contains(&Site { file: pipeline, line: 5, col: 4, len: 9 }));
        assert_eq!(repo.search_text("atlas", 100).0.len(), repo.search_text("Atlas", 100).0.len() + 2);
        assert_eq!(repo.search_text("Rasterize", 100).0.len(), 0, "an uppercase letter makes it case-sensitive");
        assert!(repo.search_text("a", 3).1, "limit reached");
    }
}
