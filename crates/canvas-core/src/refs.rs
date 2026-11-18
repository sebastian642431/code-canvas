use std::borrow::Cow;
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

/// A file, a line and a column: where a call is made or where what it calls is.
pub type CallPlace = (u32, u32, u16);

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
    /// The kinds a language server gave, by `RelKind::index`; the others are guessed by name.
    pub exact: [bool; 5],
}

impl Relations {
    pub fn count(&self, kind: RelKind) -> usize {
        self.items.iter().filter(|r| r.kind == kind).count()
    }

    pub fn is_exact(&self, kind: RelKind) -> bool {
        self.exact[kind.index()]
    }
}

pub const MAX_PER_KIND: usize = 1500;

const COMPACT_AT: usize = 1 << 14;

type Key = (u64, u32, u32);

fn key(hash: u64, at: SymRef) -> Key {
    (hash, at.file, at.index)
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Named {
    hashes: Vec<u64>,
    refs: Vec<SymRef>,
    clashing: HashMap<Box<str>, Vec<SymRef>>,
    added: Vec<(u64, SymRef)>,
    gone: Vec<u32>,
}

fn name_hash(name: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in name.as_bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl Named {
    fn build(mut found: Vec<(u64, SymRef, &str)>) -> Named {
        found.par_sort_unstable_by_key(|(hash, at, _)| (*hash, at.file, at.index));
        let mut named = Named {
            hashes: Vec::with_capacity(found.len()),
            refs: Vec::with_capacity(found.len()),
            ..Default::default()
        };
        let mut first = 0;
        while first < found.len() {
            let (hash, _, name) = found[first];
            let run = found[first..].iter().take_while(|entry| entry.0 == hash).count();
            let same = &found[first..first + run];
            if same.iter().all(|entry| entry.2 == name) {
                named.hashes.extend(same.iter().map(|entry| entry.0));
                named.refs.extend(same.iter().map(|entry| entry.1));
            } else {
                for entry in same {
                    named.clashing.entry(entry.2.into()).or_default().push(entry.1);
                }
            }
            first += run;
        }
        named
    }

    fn run(&self, hash: u64) -> std::ops::Range<usize> {
        let first = self.hashes.partition_point(|h| *h < hash);
        first..first + self.hashes[first..].partition_point(|h| *h == hash)
    }

    fn added_run(&self, hash: u64) -> std::ops::Range<usize> {
        let first = self.added.partition_point(|e| e.0 < hash);
        first..first + self.added[first..].partition_point(|e| e.0 == hash)
    }

    fn is_gone(&self, at: usize) -> bool {
        self.gone.binary_search(&(at as u32)).is_ok()
    }

    fn get(&self, name: &str) -> Cow<'_, [SymRef]> {
        if let Some(found) = self.clashing.get(name) {
            return Cow::Borrowed(found);
        }
        let hash = name_hash(name);
        let run = self.run(hash);
        let added = self.added_run(hash);
        let gone_from = self.gone.partition_point(|&g| (g as usize) < run.start);
        let gone_to = self.gone.partition_point(|&g| (g as usize) < run.end);
        if added.is_empty() && gone_from == gone_to {
            return Cow::Borrowed(&self.refs[run]);
        }
        let mut out: Vec<SymRef> = run.filter(|&at| !self.is_gone(at)).map(|at| self.refs[at]).collect();
        out.extend(self.added[added].iter().map(|e| e.1));
        out.sort_unstable_by_key(|at| (at.file, at.index));
        Cow::Owned(out)
    }

    fn clashes(&self, hash: u64) -> bool {
        !self.clashing.is_empty() && self.clashing.keys().any(|name| name_hash(name) == hash)
    }

    fn named_here(&self, hash: u64, mut same: impl FnMut(SymRef) -> bool) -> bool {
        if let Some(at) = self.run(hash).find(|&at| !self.is_gone(at)) {
            return same(self.refs[at]);
        }
        self.added[self.added_run(hash)].first().is_none_or(|e| same(e.1))
    }

    fn take(&mut self, hash: u64, at: SymRef) -> bool {
        if self.clashes(hash) {
            return false;
        }
        let wanted = key(hash, at);
        if let Ok(i) = self.added.binary_search_by_key(&wanted, |e| key(e.0, e.1)) {
            self.added.remove(i);
            return true;
        }
        let (mut lo, mut hi) = (0, self.hashes.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if key(self.hashes[mid], self.refs[mid]) < wanted {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == self.hashes.len() || key(self.hashes[lo], self.refs[lo]) != wanted {
            return false;
        }
        match self.gone.binary_search(&(lo as u32)) {
            Ok(_) => false,
            Err(i) => {
                self.gone.insert(i, lo as u32);
                true
            }
        }
    }

    fn put(&mut self, hash: u64, at: SymRef, same: impl FnMut(SymRef) -> bool) -> bool {
        if self.clashes(hash) || !self.named_here(hash, same) {
            return false;
        }
        let wanted = key(hash, at);
        match self.added.binary_search_by_key(&wanted, |e| key(e.0, e.1)) {
            Ok(_) => false,
            Err(i) => {
                self.added.insert(i, (hash, at));
                true
            }
        }
    }

    fn untidy(&self) -> usize {
        self.added.len() + self.gone.len()
    }

    fn compact(&mut self) {
        if self.untidy() == 0 {
            return;
        }
        let (hashes, refs) = (std::mem::take(&mut self.hashes), std::mem::take(&mut self.refs));
        let (added, gone) = (std::mem::take(&mut self.added), std::mem::take(&mut self.gone));
        let len = hashes.len() + added.len() - gone.len();
        let (mut out_h, mut out_r) = (Vec::with_capacity(len), Vec::with_capacity(len));
        let (mut g, mut a) = (0, 0);
        for (at, (hash, sym)) in hashes.into_iter().zip(refs).enumerate() {
            if g < gone.len() && gone[g] as usize == at {
                g += 1;
                continue;
            }
            while a < added.len() && key(added[a].0, added[a].1) < key(hash, sym) {
                out_h.push(added[a].0);
                out_r.push(added[a].1);
                a += 1;
            }
            out_h.push(hash);
            out_r.push(sym);
        }
        for (hash, sym) in &added[a..] {
            out_h.push(*hash);
            out_r.push(*sym);
        }
        (self.hashes, self.refs) = (out_h, out_r);
    }

    fn shift_files(&mut self, from: u32, up: bool) {
        self.compact();
        let moved = |at: &mut SymRef| {
            if at.file >= from {
                at.file = if up { at.file + 1 } else { at.file - 1 };
            }
        };
        self.refs.iter_mut().for_each(moved);
        self.clashing.values_mut().flat_map(|found| found.iter_mut()).for_each(moved);
    }
}

type Entry<'f> = (u64, SymRef, &'f str);

fn own_names(fi: u32, f: &SourceFile, implementations: bool) -> impl Iterator<Item = Entry<'_>> {
    let symbols = f.analysis.symbols.iter().enumerate();
    let own = symbols.filter(move |(_, s)| (s.kind == SymbolKind::Impl) == implementations);
    let own = own.map(move |(si, _)| (SymRef { file: fi, index: si as u32 }, f.symbol_name(si)));
    let of_traits = f.implemented().filter(move |_| implementations);
    let of_traits = of_traits.map(move |(sym, name)| (SymRef { file: fi, index: sym }, name));
    own.chain(of_traits).map(|(at, name)| (name_hash(name), at, name))
}

fn carries(files: &[SourceFile], at: SymRef, name: &str, implementations: bool) -> bool {
    let Some(f) = files.get(at.file as usize) else { return false };
    let symbol = f.analysis.symbols.get(at.index as usize);
    let own = symbol.is_some_and(|s| (s.kind == SymbolKind::Impl) == implementations);
    (own && f.symbol_name(at.index as usize) == name)
        || (implementations && f.implemented().any(|(sym, of)| sym == at.index && of == name))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SymbolIndex {
    by_name: Named,
    impls: Named,
    pub count: usize,
}

impl SymbolIndex {
    pub fn build(files: &[SourceFile]) -> Self {
        let of_kind = |implementations: bool| -> Vec<Entry<'_>> {
            files
                .par_iter()
                .enumerate()
                .flat_map_iter(move |(fi, f)| own_names(fi as u32, f, implementations))
                .collect()
        };
        let (by_name, impls) = rayon::join(|| Named::build(of_kind(false)), || Named::build(of_kind(true)));
        SymbolIndex { by_name, impls, count: files.iter().map(|f| f.analysis.symbols.len()).sum() }
    }

    pub fn defs(&self, name: &str) -> Cow<'_, [SymRef]> {
        self.by_name.get(name)
    }

    pub fn impls(&self, name: &str) -> Cow<'_, [SymRef]> {
        self.impls.get(name)
    }

    fn swap_names(&mut self, files: &[SourceFile], file: u32, old: Option<&SourceFile>) -> bool {
        let new = files.get(file as usize);
        for implementations in [false, true] {
            let before: Vec<Entry> = old.into_iter().flat_map(|f| own_names(file, f, implementations)).collect();
            let after: Vec<Entry> = new.into_iter().flat_map(|f| own_names(file, f, implementations)).collect();
            if before == after {
                continue;
            }
            let named = if implementations { &mut self.impls } else { &mut self.by_name };
            for &(hash, at, _) in &before {
                if !named.take(hash, at) {
                    return false;
                }
            }
            for &(hash, at, name) in &after {
                let same = |other: SymRef| match other.file == file {
                    true => after.iter().any(|e| e.1 == other && e.2 == name),
                    false => carries(files, other, name, implementations),
                };
                if !named.put(hash, at, same) {
                    return false;
                }
            }
            if named.untidy() > COMPACT_AT {
                named.compact();
            }
        }
        true
    }

    pub fn replace_file(&mut self, files: &[SourceFile], file: u32, old: &SourceFile) {
        if !self.swap_names(files, file, Some(old)) {
            *self = SymbolIndex::build(files);
            return;
        }
        self.count = self.count + files[file as usize].analysis.symbols.len() - old.analysis.symbols.len();
    }

    pub fn insert_file(&mut self, files: &[SourceFile], file: u32) {
        self.by_name.shift_files(file, true);
        self.impls.shift_files(file, true);
        if !self.swap_names(files, file, None) {
            *self = SymbolIndex::build(files);
            return;
        }
        self.count += files[file as usize].analysis.symbols.len();
    }

    pub fn remove_file(&mut self, files: &[SourceFile], file: u32, old: &SourceFile) {
        for implementations in [false, true] {
            let named = if implementations { &mut self.impls } else { &mut self.by_name };
            for (hash, at, _) in own_names(file, old, implementations) {
                if !named.take(hash, at) {
                    *self = SymbolIndex::build(files);
                    return;
                }
            }
        }
        self.by_name.shift_files(file + 1, false);
        self.impls.shift_files(file + 1, false);
        self.count -= old.analysis.symbols.len();
    }

    pub fn tidied(&self) -> SymbolIndex {
        let mut out = self.clone();
        out.by_name.compact();
        out.impls.compact();
        out
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

pub fn signature<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for text in lines.take(6) {
        let text = text.trim();
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
        let lines = s.line..(s.line + 6).min(s.body_end).min(f.line_count() as u32);
        signature(lines.map(|line| f.line(line as usize)))
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
        let call = is_call(f.text(), toks, k);
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
        self.nearest(&cands, from_file, call)
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
                    let call = is_call(f.text(), toks, k);
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
                if !is_call(f.text(), toks, k) {
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
            for &imp in self.symbols.impls(&name).iter() {
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

        Relations { target, items, truncated, search_ms: started.elapsed().as_secs_f32() * 1000.0, exact: [false; 5] }
    }

    pub fn refine(&self, rel: &mut Relations, uses: &[(u32, u32, u16)]) {
        let def = self.sym_site(rel.target);
        let wanted: HashSet<(u32, u32, u16)> = uses.iter().copied().collect();
        let place = |site: &Site| (site.file, site.line, site.col);
        let used = |r: &Relation| !matches!(r.kind, RelKind::Reference | RelKind::IncomingCall);
        rel.items.retain(|r| used(r) || wanted.contains(&place(&r.from)));
        let known: HashSet<(u32, u32, u16)> = rel.items.iter().filter(|r| !used(r)).map(|r| place(&r.from)).collect();
        let mut missing: Vec<(u32, u32, u16)> =
            wanted.into_iter().filter(|at| !known.contains(at) && *at != place(&def)).collect();
        missing.sort_unstable();
        let mut tokens: Option<((u32, usize), Tokens)> = None;
        for (file, line, col) in missing {
            let Some(f) = self.files.get(file as usize).filter(|f| (line as usize) < f.line_count()) else { continue };
            if rel.items.len() >= MAX_PER_KIND * 2 {
                rel.truncated = true;
                break;
            }
            let chunk = line as usize / CHUNK_LINES;
            if tokens.as_ref().map(|t| t.0) != Some((file, chunk)) {
                tokens = Some(((file, chunk), f.tokenize_chunk(chunk)));
            }
            let toks = tokens.as_ref().unwrap().1.line(line as usize - chunk * CHUNK_LINES);
            let call = toks.iter().position(|t| t.col == col).is_some_and(|k| is_call(f.text(), toks, k));
            let enclosing = symbol_at_line(&f.analysis.symbols, line).map(|index| SymRef { file, index: index as u32 });
            rel.items.push(Relation {
                kind: if call { RelKind::IncomingCall } else { RelKind::Reference },
                from: Site { file, line, col, len: def.len },
                to: def,
                other: enclosing.filter(|sym| *sym != rel.target || line != def.line),
            });
        }
        rel.exact[RelKind::Reference.index()] = true;
        rel.exact[RelKind::IncomingCall.index()] = true;
    }

    /// The symbol named at a place: the one whose name starts there, else the first one
    /// defined on that line (a server may point at another part of an `impl` line).
    pub fn symbol_named_at(&self, file: u32, line: u32, col: u16) -> Option<SymRef> {
        let symbols = &self.files.get(file as usize)?.analysis.symbols;
        let start = symbols.partition_point(|s| s.line < line);
        let count = symbols[start..].iter().take_while(|s| s.line == line).count();
        let on_line = &symbols[start..start + count];
        let at = on_line.iter().position(|s| s.col == col).unwrap_or(0);
        on_line.get(at)?;
        Some(SymRef { file, index: (start + at) as u32 })
    }

    /// Puts the calls a language server says the target makes in place of the guessed ones.
    /// Each is `(where the call is, where what it calls is named)`. As with the guess, one
    /// relation per callee, at its first call, and the target calling itself is left out.
    pub fn refine_calls(&self, rel: &mut Relations, calls: &[(CallPlace, CallPlace)]) {
        rel.items.retain(|r| r.kind != RelKind::OutgoingCall);
        let mut calls = calls.to_vec();
        calls.sort_unstable();
        let has_line =
            |file: u32, line: u32| self.files.get(file as usize).is_some_and(|f| (line as usize) < f.line_count());
        let mut seen: HashSet<(u32, u32, u16)> = HashSet::new();
        for ((file, line, col), (to_file, to_line, to_col)) in calls {
            if !has_line(file, line) || !has_line(to_file, to_line) {
                continue;
            }
            let callee = self.symbol_named_at(to_file, to_line, to_col);
            if callee == Some(rel.target) {
                continue;
            }
            let to = match callee {
                Some(sym) => self.sym_site(sym),
                None => Site { file: to_file, line: to_line, col: to_col, len: 1 },
            };
            if !seen.insert((to.file, to.line, to.col)) {
                continue;
            }
            if seen.len() > MAX_PER_KIND {
                rel.truncated = true;
                break;
            }
            rel.items.push(Relation {
                kind: RelKind::OutgoingCall,
                from: Site { file, line, col, len: to.len },
                to,
                other: callee,
            });
        }
        rel.exact[RelKind::OutgoingCall.index()] = true;
    }

    /// Puts what a language server says implements the target in place of the guessed
    /// implementations. A place is where the implementation is named.
    pub fn refine_implementations(&self, rel: &mut Relations, places: &[(u32, u32, u16)]) {
        rel.items.retain(|r| r.kind != RelKind::Implementation);
        let def = self.sym_site(rel.target);
        let mut places = places.to_vec();
        places.sort_unstable();
        let mut seen: HashSet<(u32, u32, u16)> = HashSet::new();
        for (file, line, col) in places {
            let Some(f) = self.files.get(file as usize).filter(|f| (line as usize) < f.line_count()) else { continue };
            if (file, line, col) == (def.file, def.line, def.col) {
                continue;
            }
            let named = self.symbol_named_at(file, line, col);
            if named == Some(rel.target) {
                continue;
            }
            let from = named.map_or(Site { file, line, col, len: def.len }, |sym| self.sym_site(sym));
            if !seen.insert((from.file, from.line, from.col)) {
                continue;
            }
            if seen.len() > MAX_PER_KIND {
                rel.truncated = true;
                break;
            }
            let enclosing =
                || symbol_at_line(&f.analysis.symbols, line).map(|index| SymRef { file, index: index as u32 });
            rel.items.push(Relation { kind: RelKind::Implementation, from, to: def, other: named.or_else(enclosing) });
        }
        rel.exact[RelKind::Implementation.index()] = true;
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
    fn exact_uses_drop_the_guesses_and_add_what_was_missed() {
        let repo = repo(&[
            ("a.rs", "pub fn run() -> u32 {\n    1\n}\n\nfn first() {\n    run();\n}\n"),
            ("b.rs", "fn other() {\n    let run = 2;\n    let again = run;\n    crate::a::run();\n}\n"),
        ]);
        let a = repo.file_index("a.rs").unwrap() as u32;
        let b = repo.file_index("b.rs").unwrap() as u32;
        let run = repo.symbols.defs("run")[0];
        let mut rel = repo.relations(run, None);
        let guessed = rel.items.iter().filter(|r| r.from.file == b).count();
        assert!(guessed >= 2, "by name, the local of b.rs looks like a use: {guessed}");
        assert!(!rel.is_exact(RelKind::Reference));
        repo.refine(&mut rel, &[(a, 0, 7), (a, 5, 4), (b, 3, 14)]);
        assert!(rel.is_exact(RelKind::Reference) && rel.is_exact(RelKind::IncomingCall));
        assert!(!rel.is_exact(RelKind::OutgoingCall) && !rel.is_exact(RelKind::Implementation));
        let uses: Vec<_> = rel
            .items
            .iter()
            .filter(|r| r.kind != RelKind::Definition)
            .map(|r| (r.from.file, r.from.line, r.kind))
            .collect();
        assert_eq!(uses.len(), 2, "{uses:?}");
        assert!(uses.contains(&(a, 5, RelKind::IncomingCall)));
        assert!(uses.contains(&(b, 3, RelKind::IncomingCall)));
        let mut rel = repo.relations(run, None);
        rel.items.retain(|r| r.kind == RelKind::Definition);
        repo.refine(&mut rel, &[(b, 3, 14), (b, 99, 0)]);
        assert_eq!(rel.count(RelKind::IncomingCall), 1, "a use the names missed is added, one past the file is not");
        assert_eq!(
            rel.items.last().unwrap().other.map(|sym| repo.sym_name(sym).to_string()),
            Some("other".to_string())
        );
    }

    #[test]
    fn exact_calls_and_implementations_take_the_place_of_the_guessed_ones() {
        let repo = repo(&[
            (
                "src/shape.rs",
                "pub trait Shape {\n    fn area(&self) -> u32;\n}\n\npub fn total(s: &dyn Shape) -> u32 {\n    helper(s.area()) + helper(1) + total(s)\n}\n\nfn helper(n: u32) -> u32 {\n    n\n}\n",
            ),
            ("src/square.rs", "pub struct Square;\n\nimpl Shape for Square {\n    fn area(&self) -> u32 {\n        4\n    }\n}\n"),
            ("src/circle.rs", "pub struct Circle;\n\nimpl Shape for Circle {\n    fn area(&self) -> u32 {\n        3\n    }\n}\n"),
            ("src/other.rs", "pub fn unrelated() -> u32 {\n    7\n}\n"),
        ]);
        let shape = file(&repo, "src/shape.rs");
        let square = file(&repo, "src/square.rs");
        let circle = file(&repo, "src/circle.rs");
        let other = file(&repo, "src/other.rs");
        let name = |sym: Option<SymRef>| sym.map(|s| repo.sym_name(s).to_string()).unwrap_or_default();

        let total = repo.symbols.defs("total")[0];
        let mut rel = repo.relations(total, None);
        let definitions = rel.count(RelKind::Definition);
        let calls = [
            ((shape, 5, 23), (shape, 8, 3)),
            ((shape, 5, 4), (shape, 8, 3)),
            ((shape, 5, 13), (shape, 1, 7)),
            ((shape, 5, 35), (shape, 4, 7)),
            ((shape, 5, 30), (other, 0, 7)),
            ((shape, 99, 0), (other, 0, 7)),
        ];
        repo.refine_calls(&mut rel, &calls);
        assert!(rel.is_exact(RelKind::OutgoingCall));
        assert!(!rel.is_exact(RelKind::Reference), "the uses are still the guessed ones");
        assert_eq!(rel.count(RelKind::Definition), definitions);
        let out: Vec<_> = rel
            .items
            .iter()
            .filter(|r| r.kind == RelKind::OutgoingCall)
            .map(|r| ((r.from.line, r.from.col), (r.to.file, r.to.line), name(r.other)))
            .collect();
        assert_eq!(out.len(), 3, "one per callee, not itself, not past the end of the file: {out:?}");
        assert!(out.contains(&((5, 4), (shape, 8), "helper".to_string())), "a callee at its first call: {out:?}");
        assert!(out.iter().any(|(_, to, _)| *to == (shape, 1)), "the method of the trait: {out:?}");
        assert!(out.contains(&((5, 30), (other, 0), "unrelated".to_string())), "what the names missed: {out:?}");
        repo.refine_calls(&mut rel, &[]);
        assert_eq!(rel.count(RelKind::OutgoingCall), 0, "a function that calls nothing");

        let trait_ = repo.symbols.defs("Shape")[0];
        let mut rel = repo.relations(trait_, None);
        assert!(!rel.is_exact(RelKind::Implementation));
        repo.refine_implementations(&mut rel, &[(square, 2, 5), (circle, 2, 15), (circle, 2, 15), (shape, 0, 10)]);
        assert!(rel.is_exact(RelKind::Implementation));
        let found: Vec<_> = rel
            .items
            .iter()
            .filter(|r| r.kind == RelKind::Implementation)
            .map(|r| (r.from.file, r.from.line, r.other.is_some()))
            .collect();
        assert_eq!(found.len(), 2, "once each, and not the trait itself: {found:?}");
        assert!(found.contains(&(square, 2, true)) && found.contains(&(circle, 2, true)), "{found:?}");
        repo.refine_implementations(&mut rel, &[(square, 2, 5)]);
        assert_eq!(rel.count(RelKind::Implementation), 1, "what the server does not name goes");

        let area = repo.symbols.defs("area").iter().copied().find(|s| s.file == shape).unwrap();
        let mut rel = repo.relations(area, None);
        repo.refine_implementations(&mut rel, &[(square, 3, 7), (circle, 3, 7)]);
        let methods: Vec<_> =
            rel.items.iter().filter(|r| r.kind == RelKind::Implementation).map(|r| name(r.other)).collect();
        assert_eq!(methods, ["area", "area"], "the methods that implement the one of the trait");
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
