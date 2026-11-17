use std::collections::HashMap;
use std::time::Instant;

use rayon::prelude::*;

use crate::scan::{Repo, SourceFile};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum DepKind {
    Import,
    TypeOnly,
    ReExport,
    Include,
    Module,
}

impl DepKind {
    pub const ALL: [DepKind; 5] =
        [DepKind::Import, DepKind::TypeOnly, DepKind::ReExport, DepKind::Include, DepKind::Module];

    pub fn label(self) -> &'static str {
        match self {
            DepKind::Import => "Imports",
            DepKind::TypeOnly => "Type imports",
            DepKind::ReExport => "Re-exports",
            DepKind::Include => "Includes",
            DepKind::Module => "Modules",
        }
    }

    pub fn index(self) -> usize {
        match self {
            DepKind::Import => 0,
            DepKind::TypeOnly => 1,
            DepKind::ReExport => 2,
            DepKind::Include => 3,
            DepKind::Module => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    pub line: u32,
    pub kind: DepKind,
}

#[derive(Clone, Debug, Default)]
pub struct Deps {
    pub edges: Vec<Edge>,
    pub unresolved: u32,
    pub external: u32,
    pub counts: [u32; 5],
    pub ms: u128,
    out_start: Vec<u32>,
    by_to: Vec<u32>,
    in_start: Vec<u32>,
    pairs: usize,
}

impl Deps {
    fn build(mut edges: Vec<Edge>, files: usize) -> Deps {
        edges.par_sort_unstable_by_key(|e| (e.from, e.to, e.line, e.kind.index()));
        let mut counts = [0u32; 5];
        let mut out_start = vec![0u32; files + 1];
        let mut in_start = vec![0u32; files + 1];
        let mut pairs = 0;
        let mut last = None;
        for e in &edges {
            counts[e.kind.index()] += 1;
            out_start[e.from as usize + 1] += 1;
            in_start[e.to as usize + 1] += 1;
            if last != Some((e.from, e.to)) {
                last = Some((e.from, e.to));
                pairs += 1;
            }
        }
        for f in 0..files {
            out_start[f + 1] += out_start[f];
            in_start[f + 1] += in_start[f];
        }
        let mut next = in_start.clone();
        let mut by_to = vec![0u32; edges.len()];
        for (i, e) in edges.iter().enumerate() {
            let slot = &mut next[e.to as usize];
            by_to[*slot as usize] = i as u32;
            *slot += 1;
        }
        Deps { edges, unresolved: 0, external: 0, counts, ms: 0, out_start, by_to, in_start, pairs }
    }

    pub fn outgoing(&self, file: u32) -> &[Edge] {
        match range(&self.out_start, file) {
            Some(r) => &self.edges[r],
            None => &[],
        }
    }

    pub fn incoming(&self, file: u32) -> impl Iterator<Item = &Edge> + '_ {
        let r = range(&self.in_start, file).unwrap_or(0..0);
        self.by_to[r].iter().map(move |&i| &self.edges[i as usize])
    }

    pub fn pairs(&self) -> usize {
        self.pairs
    }
}

fn range(starts: &[u32], file: u32) -> Option<std::ops::Range<usize>> {
    let f = file as usize;
    Some(*starts.get(f)? as usize..*starts.get(f + 1)? as usize)
}

pub fn analyze(repo: &Repo) -> Deps {
    let started = Instant::now();
    let ix = Index::build(&repo.files);
    let parts: Vec<Scratch> = repo
        .files
        .par_iter()
        .enumerate()
        .fold(Scratch::default, |mut s, (fi, f)| {
            scan_file(&ix, fi as u32, f, &mut s);
            s
        })
        .collect();
    let mut edges = Vec::with_capacity(parts.iter().map(|p| p.edges.len()).sum());
    let (mut unresolved, mut external) = (0, 0);
    for p in parts {
        edges.extend_from_slice(&p.edges);
        unresolved += p.unresolved;
        external += p.external;
    }
    let mut deps = Deps::build(edges, repo.files.len());
    deps.unresolved = unresolved;
    deps.external = external;
    deps.ms = started.elapsed().as_millis();
    deps
}

const MAX_STATEMENT_LINES: usize = 40;
const JAVA_SEGMENTS: usize = 6;
const AMBIGUOUS: u32 = u32::MAX;

struct Index<'r> {
    by_path: HashMap<&'r str, u32>,
    java: HashMap<&'r str, u32>,
}

impl<'r> Index<'r> {
    fn build(files: &'r [SourceFile]) -> Self {
        let mut by_path = HashMap::with_capacity(files.len());
        let mut java: HashMap<&'r str, u32> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            by_path.insert(f.rel_path.as_str(), i as u32);
            if scanner_for(&f.name) != Some(Scanner::Java) {
                continue;
            }
            let stem = &f.rel_path[..f.rel_path.rfind('.').unwrap_or(f.rel_path.len())];
            let mut end = stem.len();
            for _ in 0..JAVA_SEGMENTS {
                let start = stem[..end].rfind('/').map(|p| p + 1).unwrap_or(0);
                java.entry(&stem[start..]).and_modify(|v| *v = AMBIGUOUS).or_insert(i as u32);
                if start == 0 {
                    break;
                }
                end = start - 1;
            }
        }
        Index { by_path, java }
    }

    fn find(&self, path: &str) -> Option<u32> {
        self.by_path.get(path).copied()
    }

    fn find_with(&self, path: &mut String, ext: &str) -> Option<u32> {
        let n = path.len();
        path.push_str(ext);
        let found = self.find(path);
        path.truncate(n);
        found
    }
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

fn ancestors(dir: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(dir), |d| (!d.is_empty()).then(|| dir_of(d)))
}

fn push_path(buf: &mut String, rel: &str) -> bool {
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if buf.is_empty() {
                    return false;
                }
                let cut = buf.rfind('/').unwrap_or(0);
                buf.truncate(cut);
            }
            seg => {
                if !buf.is_empty() {
                    buf.push('/');
                }
                buf.push_str(seg);
            }
        }
    }
    true
}

fn join(buf: &mut String, base: &str, rel: &str) -> bool {
    buf.clear();
    buf.push_str(base);
    push_path(buf, rel)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scanner {
    Js,
    C,
    Rust,
    Python,
    Java,
}

fn scanner_for(name: &str) -> Option<Scanner> {
    let ext = name.rsplit_once('.')?.1.as_bytes();
    let mut low = [0u8; 8];
    let low = low.get_mut(..ext.len())?;
    low.copy_from_slice(ext);
    low.make_ascii_lowercase();
    Some(match &*low {
        b"js" | b"jsx" | b"mjs" | b"cjs" | b"ts" | b"tsx" | b"mts" | b"cts" | b"vue" | b"svelte" => Scanner::Js,
        b"c" | b"cc" | b"cpp" | b"cxx" | b"h" | b"hh" | b"hpp" | b"hxx" | b"m" | b"mm" | b"inc" | b"inl" => Scanner::C,
        b"rs" => Scanner::Rust,
        b"py" | b"pyi" => Scanner::Python,
        b"java" | b"kt" => Scanner::Java,
        _ => return None,
    })
}

#[derive(Default)]
struct Scratch {
    edges: Vec<Edge>,
    unresolved: u32,
    external: u32,
    buf: String,
    stmt: String,
    roots: Vec<u32>,
    roots_ready: bool,
    mod_dir: String,
    crate_src: String,
    crate_state: CrateState,
    inline: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum CrateState {
    #[default]
    Unknown,
    Found,
    Missing,
}

enum Res {
    File(u32),
    Unresolved,
    External,
}

impl Scratch {
    fn record(&mut self, from: u32, line: usize, kind: DepKind, res: Res) {
        match res {
            Res::File(to) => self.edge(from, to, line, kind),
            Res::Unresolved => self.unresolved += 1,
            Res::External => self.external += 1,
        }
    }

    fn edge(&mut self, from: u32, to: u32, line: usize, kind: DepKind) {
        if to != from {
            self.edges.push(Edge { from, to, line: line as u32, kind });
        }
    }
}

fn code(f: &SourceFile, i: usize) -> &str {
    f.line(i).trim_start_matches(' ')
}

fn scan_file(ix: &Index, fi: u32, f: &SourceFile, s: &mut Scratch) {
    if f.text().is_empty() {
        return;
    }
    let Some(scanner) = scanner_for(&f.name) else { return };
    let first = s.edges.len();
    match scanner {
        Scanner::Js => scan_js(ix, fi, f, s),
        Scanner::C => scan_c(ix, fi, f, s),
        Scanner::Rust => scan_rust(ix, fi, f, s),
        Scanner::Python => scan_python(ix, fi, f, s),
        Scanner::Java => scan_java(ix, fi, f, s),
    }
    if s.edges.len() - first > 1 {
        s.edges[first..].sort_unstable_by_key(|e| (e.to, e.kind.index(), e.line));
        let mut kept = first + 1;
        for r in first + 1..s.edges.len() {
            let (e, prev) = (s.edges[r], s.edges[kept - 1]);
            if e.to != prev.to || e.kind != prev.kind {
                s.edges[kept] = e;
                kept += 1;
            }
        }
        s.edges.truncate(kept);
        s.edges[first..].sort_unstable_by_key(|e| (e.to, e.line, e.kind.index()));
    }
}

fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn quoted(r: &str) -> Option<&str> {
    let q = *r.as_bytes().first()?;
    if q != b'\'' && q != b'"' {
        return None;
    }
    let end = r[1..].find(q as char)?;
    Some(&r[1..1 + end])
}

const JS_EXTS: [&str; 10] = [".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mjs", ".cjs", ".json", ".vue", ".svelte"];
const JS_SWAPS: [(&str, &[&str]); 5] =
    [(".js", &[".ts", ".tsx"]), (".jsx", &[".tsx"]), (".mjs", &[".mts"]), (".cjs", &[".cts"]), (".ts", &[".d.ts"])];
const JS_ALIASES: [&str; 4] = ["@/", "~/", "src/", "#"];
const PACKAGE_MARKERS: [&str; 2] = ["package.json", "tsconfig.json"];

fn scan_js(ix: &Index, fi: u32, f: &SourceFile, s: &mut Scratch) {
    let dir = dir_of(&f.rel_path);
    let dynamic = f.text().contains("require(") || f.text().contains("import(");
    s.roots_ready = false;
    let n = f.line_count();
    let mut i = 0;
    while i < n {
        let line = i;
        let t = code(f, i);
        i += 1;
        let statement = match t.as_bytes().first() {
            Some(b'i' | b'e') => js_statement(t),
            Some(_) if dynamic => None,
            _ => continue,
        };
        if let Some((kind, rest)) = statement {
            if t.contains(['\'', '"']) {
                let spec = if rest.starts_with(['\'', '"']) { quoted(rest) } else { from_spec(rest) };
                if let Some(spec) = spec {
                    if let Some(res) = resolve_js(ix, dir, spec, s) {
                        s.record(fi, line, kind, res);
                    }
                    continue;
                }
            } else if !t.contains([';', '}']) {
                for j in i..(i + MAX_STATEMENT_LINES).min(n) {
                    let l = code(f, j);
                    if js_statement(l).is_some() {
                        break;
                    }
                    if let Some(spec) = from_spec(l) {
                        if let Some(res) = resolve_js(ix, dir, spec, s) {
                            s.record(fi, line, kind, res);
                        }
                        i = j + 1;
                        break;
                    }
                    if l.contains(';') {
                        break;
                    }
                }
                continue;
            }
        }
        if dynamic {
            dynamic_specs(t, |spec| {
                if let Some(res) = resolve_js(ix, dir, spec, s) {
                    s.record(fi, line, DepKind::Import, res);
                }
            });
        }
    }
}

fn js_statement(t: &str) -> Option<(DepKind, &str)> {
    let (import, r) = match t.strip_prefix("import") {
        Some(r) => (true, r),
        None => (false, t.strip_prefix("export")?),
    };
    if !r.is_empty() && !r.starts_with([' ', '{', '*', '\'', '"']) {
        return None;
    }
    let r = r.trim_start_matches(' ');
    let (type_only, r) = match r.strip_prefix("type") {
        Some(x) if x.starts_with([' ', '{', '*']) => {
            let x = x.trim_start_matches(' ');
            if x.starts_with("from") || x.starts_with([',', '=']) {
                (false, r)
            } else {
                (true, x)
            }
        }
        _ => (false, r),
    };
    if import {
        Some((if type_only { DepKind::TypeOnly } else { DepKind::Import }, r))
    } else if r.starts_with(['{', '*']) {
        Some((DepKind::ReExport, r))
    } else {
        None
    }
}

fn from_spec(l: &str) -> Option<&str> {
    let b = l.as_bytes();
    let mut at = 0;
    while let Some(p) = l[at..].find("from") {
        let p = at + p;
        at = p + 4;
        if p == 0 || !ident_byte(b[p - 1]) {
            if let Some(spec) = quoted(l[at..].trim_start_matches(' ')) {
                return Some(spec);
            }
        }
    }
    None
}

fn dynamic_specs<'t>(t: &'t str, mut found: impl FnMut(&'t str)) {
    if t.starts_with("//") || t.starts_with('*') || t.starts_with("/*") {
        return;
    }
    let b = t.as_bytes();
    let mut at = 0;
    while let Some(p) = t[at..].find('(') {
        let p = at + p;
        at = p + 1;
        let head = &t[..p];
        let start = match (head.ends_with("require"), head.ends_with("import")) {
            (true, _) => p - 7,
            (_, true) => p - 6,
            _ => continue,
        };
        if start > 0 && (ident_byte(b[start - 1]) || b[start - 1] == b'.') {
            continue;
        }
        if head.find("//").is_some_and(|c| c == 0 || b[c - 1] != b':') {
            return;
        }
        let rest = t[at..].trim_start_matches(' ');
        let Some(&q) = rest.as_bytes().first() else { continue };
        if !matches!(q, b'\'' | b'"' | b'`') {
            continue;
        }
        let Some(end) = rest[1..].find(q as char) else { continue };
        let spec = &rest[1..1 + end];
        if !spec.contains("${") {
            found(spec);
        }
    }
}

fn resolve_js(ix: &Index, dir: &str, spec: &str, s: &mut Scratch) -> Option<Res> {
    let spec = spec.split('?').next().unwrap_or(spec);
    if spec.is_empty() {
        return None;
    }
    if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
        let found = if join(&mut s.buf, dir, spec) { find_js(ix, &mut s.buf) } else { None };
        return Some(found.map(Res::File).unwrap_or(Res::Unresolved));
    }
    let Some(rest) = JS_ALIASES.iter().find_map(|alias| spec.strip_prefix(alias)) else {
        return Some(Res::External);
    };
    if !s.roots_ready {
        package_roots(ix, dir, &mut s.buf, &mut s.roots);
        s.roots_ready = true;
    }
    for &root in &s.roots {
        for sub in ["src", ""] {
            if join(&mut s.buf, &dir[..root as usize], sub) && push_path(&mut s.buf, rest) {
                if let Some(i) = find_js(ix, &mut s.buf) {
                    return Some(Res::File(i));
                }
            }
        }
    }
    Some(Res::Unresolved)
}

fn package_roots(ix: &Index, dir: &str, buf: &mut String, roots: &mut Vec<u32>) {
    roots.clear();
    for d in ancestors(dir).filter(|d| !d.is_empty()) {
        buf.clear();
        buf.push_str(d);
        buf.push('/');
        if PACKAGE_MARKERS.iter().any(|m| ix.find_with(buf, m).is_some()) {
            roots.push(d.len() as u32);
        }
    }
    roots.push(0);
}

fn find_js(ix: &Index, buf: &mut String) -> Option<u32> {
    let n = buf.len();
    if n > 0 {
        if let Some(i) = ix.find(buf) {
            return Some(i);
        }
        if let Some(&(old, new)) = JS_SWAPS.iter().find(|(old, _)| buf.ends_with(old)) {
            buf.truncate(n - old.len());
            let found = new.iter().find_map(|ext| ix.find_with(buf, ext));
            buf.push_str(old);
            if found.is_some() {
                return found;
            }
        }
        if let Some(i) = JS_EXTS.iter().find_map(|ext| ix.find_with(buf, ext)) {
            return Some(i);
        }
        buf.push('/');
    }
    buf.push_str("index");
    JS_EXTS.iter().find_map(|ext| ix.find_with(buf, ext))
}

fn scan_c(ix: &Index, fi: u32, f: &SourceFile, s: &mut Scratch) {
    let dir = dir_of(&f.rel_path);
    for i in 0..f.line_count() {
        let Some(r) = code(f, i).strip_prefix('#') else { continue };
        let r = r.trim_start_matches(' ');
        let Some(r) = r.strip_prefix("include").or_else(|| r.strip_prefix("import")) else { continue };
        let r = r.trim_start_matches(' ');
        let (close, is_quoted) = match r.as_bytes().first() {
            Some(b'"') => ('"', true),
            Some(b'<') => ('>', false),
            _ => continue,
        };
        let Some(end) = r[1..].find(close) else { continue };
        let path = &r[1..1 + end];
        if path.is_empty() {
            continue;
        }
        let mut found = None;
        if is_quoted && join(&mut s.buf, dir, path) {
            found = ix.find(&s.buf);
        }
        if found.is_none() && join(&mut s.buf, "", path) {
            found = ix.find(&s.buf);
        }
        let res = match found {
            Some(to) => Res::File(to),
            None if is_quoted => Res::Unresolved,
            None => Res::External,
        };
        s.record(fi, i, DepKind::Include, res);
    }
}

fn scan_rust(ix: &Index, fi: u32, f: &SourceFile, s: &mut Scratch) {
    let dir = dir_of(&f.rel_path);
    let root_like = matches!(f.name.as_str(), "lib.rs" | "main.rs" | "mod.rs");
    s.mod_dir.clear();
    s.mod_dir.push_str(dir);
    if !root_like {
        push_path(&mut s.mod_dir, &f.name[..f.name.len() - ".rs".len()]);
    }
    s.crate_state = CrateState::Unknown;
    s.inline.clear();

    let n = f.line_count();
    let mut i = 0;
    while i < n {
        let line = i;
        let raw = f.line(i);
        let t = raw.trim_start_matches(' ');
        i += 1;
        let indent = (raw.len() - t.len()) as u32;
        match t.as_bytes().first() {
            Some(b'}') => {
                while s.inline.last().is_some_and(|&open| open >= indent) {
                    s.inline.pop();
                }
                continue;
            }
            Some(b'm' | b'u' | b'p') => {}
            _ => continue,
        }
        let (public, r) = rust_visibility(t);
        if let Some(r) = r.strip_prefix("mod ") {
            let r = r.trim_start_matches(' ');
            let end = r.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '#' | '$'))).unwrap_or(r.len());
            let name = r[..end].strip_prefix("r#").unwrap_or(&r[..end]);
            let after = r[end..].trim_start_matches(' ');
            if name.is_empty() {
                continue;
            }
            if after.starts_with('{') {
                let open = after.bytes().filter(|&b| b == b'{').count();
                if open > after.bytes().filter(|&b| b == b'}').count() {
                    s.inline.push(indent);
                }
            } else if after.starts_with(';') && s.inline.is_empty() {
                let res = match path_attribute(f, line) {
                    Some(path) => match join(&mut s.buf, dir, path).then(|| ix.find(&s.buf)).flatten() {
                        Some(to) => Res::File(to),
                        None => Res::Unresolved,
                    },
                    None => rust_mod(ix, dir, root_like, name, s),
                };
                s.record(fi, line, DepKind::Module, res);
            }
        } else if let Some(r) = r.strip_prefix("use ") {
            let kind = if public { DepKind::ReExport } else { DepKind::Import };
            match r.find(';') {
                Some(end) => rust_use(ix, fi, dir, root_like, line, kind, &r[..end], s),
                None => {
                    let mut stmt = std::mem::take(&mut s.stmt);
                    stmt.clear();
                    stmt.push_str(rust_code(r));
                    for j in i..(i + MAX_STATEMENT_LINES).min(n) {
                        let l = rust_code(code(f, j));
                        stmt.push(' ');
                        if let Some(end) = l.find(';') {
                            stmt.push_str(&l[..end]);
                            rust_use(ix, fi, dir, root_like, line, kind, &stmt, s);
                            i = j + 1;
                            break;
                        }
                        stmt.push_str(l);
                    }
                    s.stmt = stmt;
                }
            }
        }
    }
}

fn rust_code(l: &str) -> &str {
    l.find("//").map(|c| &l[..c]).unwrap_or(l)
}

fn rust_visibility(t: &str) -> (bool, &str) {
    let Some(r) = t.strip_prefix("pub") else { return (false, t) };
    let r = match r.as_bytes().first() {
        Some(b' ') => r,
        Some(b'(') => match r.find(')') {
            Some(end) => &r[end + 1..],
            None => return (false, t),
        },
        _ => return (false, t),
    };
    (true, r.trim_start_matches(' '))
}

fn path_attribute(f: &SourceFile, line: usize) -> Option<&str> {
    (line.saturating_sub(4)..line).rev().map(|i| code(f, i)).take_while(|l| l.starts_with("#[")).find_map(|l| {
        let value = l.strip_prefix("#[path")?.trim_start_matches(' ').strip_prefix('=')?;
        quoted(value.trim_start_matches(' '))
    })
}

fn rust_mod(ix: &Index, dir: &str, root_like: bool, name: &str, s: &mut Scratch) -> Res {
    let bases = [s.mod_dir.as_str(), dir];
    for base in &bases[..if root_like { 1 } else { 2 }] {
        if join(&mut s.buf, base, name) {
            if let Some(i) = ix.find_with(&mut s.buf, ".rs").or_else(|| ix.find_with(&mut s.buf, "/mod.rs")) {
                return Res::File(i);
            }
        }
    }
    Res::Unresolved
}

#[allow(clippy::too_many_arguments)]
fn rust_use(ix: &Index, fi: u32, dir: &str, root_like: bool, line: usize, kind: DepKind, tree: &str, s: &mut Scratch) {
    let (mut unresolved, mut external) = (false, false);
    use_leaves(tree, |segs| match rust_path(ix, dir, root_like, segs, s) {
        Some(Res::File(to)) => s.edge(fi, to, line, kind),
        Some(Res::Unresolved) => unresolved = true,
        Some(Res::External) => external = true,
        None => {}
    });
    s.unresolved += unresolved as u32;
    s.external += external as u32;
}

fn use_leaves<'t>(tree: &'t str, mut leaf: impl FnMut(&[&'t str])) {
    const MAX_SEGMENTS: usize = 24;
    const MAX_DEPTH: usize = 8;
    let word_byte = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'#' || b >= 0x80;
    let b = tree.as_bytes();
    let mut segs = [""; MAX_SEGMENTS];
    let mut n = 0;
    let mut marks = [0usize; MAX_DEPTH];
    let mut depth = 0;
    let mut pending = false;
    let mut alias = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if word_byte(c) && c != b'#' {
            let start = i;
            while i < b.len() && word_byte(b[i]) {
                i += 1;
            }
            let word = &tree[start..i];
            let word = word.strip_prefix("r#").unwrap_or(word);
            if alias {
                alias = false;
            } else if word == "as" {
                alias = true;
            } else {
                if n == MAX_SEGMENTS {
                    return;
                }
                segs[n] = word;
                n += 1;
                pending = true;
            }
            continue;
        }
        match c {
            b'{' => {
                if depth == MAX_DEPTH {
                    return;
                }
                marks[depth] = n;
                depth += 1;
                pending = false;
            }
            b'*' => pending = true,
            b',' | b'}' | b';' => {
                if pending {
                    leaf(&segs[..n]);
                    pending = false;
                }
                match c {
                    b',' => n = if depth > 0 { marks[depth - 1] } else { 0 },
                    b'}' if depth > 0 => {
                        depth -= 1;
                        n = marks[depth];
                    }
                    _ => return,
                }
            }
            _ => {}
        }
        i += 1;
    }
    if pending {
        leaf(&segs[..n]);
    }
}

fn rust_path(ix: &Index, dir: &str, root_like: bool, segs: &[&str], s: &mut Scratch) -> Option<Res> {
    let segs = match segs {
        [rest @ .., "self"] if !rest.is_empty() => rest,
        _ => segs,
    };
    let inline = s.inline.len();
    match *segs.first()? {
        "crate" => {
            if s.crate_state == CrateState::Unknown {
                let found = crate_src(ix, dir, &mut s.buf, &mut s.crate_src);
                s.crate_state = if found { CrateState::Found } else { CrateState::Missing };
            }
            if s.crate_state == CrateState::Missing {
                return Some(Res::Unresolved);
            }
            if let Some(i) = find_rust(ix, &s.crate_src, &segs[1..], &mut s.buf) {
                return Some(Res::File(i));
            }
            if root_like && dir == s.crate_src {
                return None;
            }
            Some(module_file(ix, &s.crate_src, &mut s.buf).map(Res::File).unwrap_or(Res::Unresolved))
        }
        "self" => {
            if inline > 0 {
                return None;
            }
            find_rust(ix, &s.mod_dir, &segs[1..], &mut s.buf).map(Res::File)
        }
        "super" => {
            let supers = segs.iter().take_while(|seg| **seg == "super").count();
            if supers <= inline {
                return None;
            }
            let mut base = s.mod_dir.as_str();
            for _ in inline..supers {
                if base.is_empty() {
                    return Some(Res::Unresolved);
                }
                base = dir_of(base);
            }
            let found = find_rust(ix, base, &segs[supers..], &mut s.buf).or_else(|| module_file(ix, base, &mut s.buf));
            Some(found.map(Res::File).unwrap_or(Res::Unresolved))
        }
        _ => Some(Res::External),
    }
}

fn find_rust(ix: &Index, base: &str, segs: &[&str], buf: &mut String) -> Option<u32> {
    (1..=segs.len()).rev().find_map(|k| {
        buf.clear();
        buf.push_str(base);
        for seg in &segs[..k] {
            if !buf.is_empty() {
                buf.push('/');
            }
            buf.push_str(seg);
        }
        ix.find_with(buf, ".rs").or_else(|| ix.find_with(buf, "/mod.rs"))
    })
}

fn module_file(ix: &Index, dir: &str, buf: &mut String) -> Option<u32> {
    buf.clear();
    buf.push_str(dir);
    let own = if dir.is_empty() { None } else { ix.find_with(buf, ".rs") };
    own.or_else(|| {
        if !dir.is_empty() {
            buf.push('/');
        }
        ["mod.rs", "lib.rs", "main.rs"].iter().find_map(|name| ix.find_with(buf, name))
    })
}

fn crate_src(ix: &Index, dir: &str, buf: &mut String, out: &mut String) -> bool {
    const ROOTS: [&str; 2] = ["lib.rs", "main.rs"];
    let mut holds = |d: &str, names: &[&str]| {
        buf.clear();
        buf.push_str(d);
        if !d.is_empty() {
            buf.push('/');
        }
        names.iter().any(|name| ix.find_with(buf, name).is_some())
    };
    if let Some(d) = ancestors(dir).find(|d| holds(d, &["Cargo.toml"])) {
        let flat = !holds(d, &["src/lib.rs", "src/main.rs"]) && holds(d, &ROOTS);
        return join(out, d, if flat { "" } else { "src" });
    }
    match ancestors(dir).find(|d| holds(d, &ROOTS)) {
        Some(d) => join(out, d, ""),
        None => false,
    }
}

fn scan_python(ix: &Index, fi: u32, f: &SourceFile, s: &mut Scratch) {
    let dir = dir_of(&f.rel_path);
    let n = f.line_count();
    for i in 0..n {
        let t = code(f, i);
        if !matches!(t.as_bytes().first(), Some(b'i' | b'f')) {
            continue;
        }
        if let Some(r) = t.strip_prefix("import ") {
            for part in python_code(r).split(',') {
                let module = part.trim_matches(' ').split(' ').next().unwrap_or("");
                if python_module(module) && !module.starts_with('.') {
                    python_import(ix, fi, dir, i, 0, module, "", s);
                }
            }
        } else if let Some(r) = t.strip_prefix("from ") {
            let r = r.trim_start_matches(' ');
            let Some(p) = r.find(" import") else { continue };
            let module = r[..p].trim_end_matches(' ');
            let after = &r[p + " import".len()..];
            if !(after.is_empty() || after.starts_with([' ', '('])) || !python_module(module) {
                continue;
            }
            let dots = module.bytes().take_while(|&b| b == b'.').count();
            let names = python_code(after);
            if names.starts_with('(') && !names.contains(')') {
                let mut stmt = std::mem::take(&mut s.stmt);
                stmt.clear();
                stmt.push_str(names);
                for j in i + 1..(i + 1 + MAX_STATEMENT_LINES).min(n) {
                    let l = python_code(code(f, j));
                    stmt.push(',');
                    stmt.push_str(l);
                    if l.contains(')') {
                        break;
                    }
                }
                python_import(ix, fi, dir, i, dots, &module[dots..], &stmt, s);
                s.stmt = stmt;
            } else {
                python_import(ix, fi, dir, i, dots, &module[dots..], names, s);
            }
        }
    }
}

fn python_code(l: &str) -> &str {
    l.find('#').map(|c| &l[..c]).unwrap_or(l).trim_matches(' ')
}

fn python_module(m: &str) -> bool {
    !m.is_empty() && m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b >= 0x80)
}

#[allow(clippy::too_many_arguments)]
fn python_import(ix: &Index, fi: u32, dir: &str, line: usize, dots: usize, module: &str, names: &str, s: &mut Scratch) {
    if dots > 0 {
        let mut base = Some(dir);
        for _ in 1..dots {
            base = base.filter(|b| !b.is_empty()).map(dir_of);
        }
        if !base.is_some_and(|base| python_try(ix, fi, base, line, module, names, s)) {
            s.unresolved += 1;
        }
        return;
    }
    let mut bases = std::iter::once("").chain(ancestors(dir).filter(|d| !d.is_empty()));
    if !bases.any(|base| python_try(ix, fi, base, line, module, names, s)) {
        s.external += 1;
    }
}

fn python_try(ix: &Index, fi: u32, base: &str, line: usize, module: &str, names: &str, s: &mut Scratch) -> bool {
    s.buf.clear();
    s.buf.push_str(base);
    for seg in module.split('.').filter(|seg| !seg.is_empty()) {
        if !s.buf.is_empty() {
            s.buf.push('/');
        }
        s.buf.push_str(seg);
    }
    let n = s.buf.len();
    let mut found = false;
    let (mut any_name, mut plain_name) = (false, false);
    for name in names.split(',') {
        let name = name.trim_matches(['(', ')', ' ', '\\']).split(' ').next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        any_name = true;
        let mut hit = None;
        if name.bytes().all(ident_byte) {
            s.buf.truncate(n);
            if n > 0 {
                s.buf.push('/');
            }
            s.buf.push_str(name);
            hit = ix.find_with(&mut s.buf, ".py").or_else(|| ix.find_with(&mut s.buf, "/__init__.py"));
        }
        match hit {
            Some(to) => {
                found = true;
                s.edge(fi, to, line, DepKind::Import);
            }
            None => plain_name = true,
        }
    }
    if plain_name || !any_name {
        s.buf.truncate(n);
        let hit = if module.is_empty() {
            if n > 0 {
                s.buf.push('/');
            }
            ix.find_with(&mut s.buf, "__init__.py")
        } else {
            ix.find_with(&mut s.buf, ".py").or_else(|| ix.find_with(&mut s.buf, "/__init__.py"))
        };
        if let Some(to) = hit {
            found = true;
            s.edge(fi, to, line, DepKind::Import);
        }
    }
    found
}

fn scan_java(ix: &Index, fi: u32, f: &SourceFile, s: &mut Scratch) {
    for i in 0..f.line_count() {
        let Some(r) = code(f, i).strip_prefix("import ") else { continue };
        let r = r.trim_start_matches(' ');
        let r = r.strip_prefix("static ").map(|r| r.trim_start_matches(' ')).unwrap_or(r);
        let end = r.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '.')).unwrap_or(r.len());
        let path = r[..end].trim_end_matches('.');
        if path.is_empty() {
            continue;
        }
        let res = find_java(ix, path, &mut s.buf).map(Res::File).unwrap_or(Res::External);
        s.record(fi, i, DepKind::Import, res);
    }
}

fn find_java(ix: &Index, path: &str, buf: &mut String) -> Option<u32> {
    let mut path = path;
    for _ in 0..3 {
        let key = match path.rmatch_indices('.').nth(JAVA_SEGMENTS - 1) {
            Some((dot, _)) => &path[dot + 1..],
            None => path,
        };
        buf.clear();
        buf.extend(key.chars().map(|c| if c == '.' { '/' } else { c }));
        match ix.java.get(buf.as_str()) {
            Some(&i) if i != AMBIGUOUS => return Some(i),
            _ => {}
        }
        path = &path[..path.rfind('.')?];
        if !path.contains('.') {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{scan, ScanOptions};

    fn repo(tag: &str, files: &[(&str, &str)]) -> Repo {
        let dir = std::env::temp_dir().join(format!("code-canvas-deps-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let repo = scan(&dir, &ScanOptions::default(), None);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(repo.files.len(), files.len(), "every test file is scanned");
        repo
    }

    fn file(repo: &Repo, path: &str) -> u32 {
        repo.file_index(path).unwrap_or_else(|| panic!("no file {path}")) as u32
    }

    fn out<'a>(repo: &'a Repo, deps: &Deps, from: &str) -> Vec<(&'a str, u32, DepKind)> {
        deps.outgoing(file(repo, from))
            .iter()
            .map(|e| (repo.files[e.to as usize].rel_path.as_str(), e.line, e.kind))
            .collect()
    }

    fn incoming<'a>(repo: &'a Repo, deps: &Deps, to: &str) -> Vec<&'a str> {
        deps.incoming(file(repo, to)).map(|e| repo.files[e.from as usize].rel_path.as_str()).collect()
    }

    fn check(repo: &Repo, deps: &Deps) {
        let key = |e: &Edge| (e.from, e.to, e.line);
        assert!(deps.edges.windows(2).all(|w| key(&w[0]) <= key(&w[1])), "sorted by (from, to, line)");
        let mut seen = std::collections::HashSet::new();
        let mut pairs = std::collections::HashSet::new();
        let mut counts = [0u32; 5];
        for e in &deps.edges {
            assert_ne!(e.from, e.to, "no self edges");
            assert!((e.to as usize) < repo.files.len());
            assert!((e.line as usize) < repo.files[e.from as usize].line_count());
            assert!(seen.insert((e.from, e.to, e.kind)), "one edge per (from, to, kind)");
            pairs.insert((e.from, e.to));
            counts[e.kind.index()] += 1;
        }
        assert_eq!(deps.pairs(), pairs.len());
        assert_eq!(deps.counts, counts);
        assert_eq!(deps.counts.iter().sum::<u32>() as usize, deps.edges.len());
        let (mut outgoing, mut arriving) = (0, 0);
        for f in 0..repo.files.len() as u32 {
            assert!(deps.outgoing(f).iter().all(|e| e.from == f));
            assert!(deps.incoming(f).all(|e| e.to == f));
            outgoing += deps.outgoing(f).len();
            arriving += deps.incoming(f).count();
        }
        assert_eq!((outgoing, arriving), (deps.edges.len(), deps.edges.len()));
        assert!(deps.outgoing(repo.files.len() as u32 + 7).is_empty());
        assert_eq!(deps.incoming(u32::MAX).count(), 0);
    }

    #[test]
    fn kinds_have_stable_indices() {
        for (i, k) in DepKind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i);
            assert!(!k.label().is_empty());
        }
    }

    #[test]
    fn paths_are_normalised_without_the_disk() {
        let mut buf = String::new();
        assert!(join(&mut buf, "a/b", "./c/../d//e.h"));
        assert_eq!(buf, "a/b/d/e.h");
        assert!(join(&mut buf, "a/b", "../../x"));
        assert_eq!(buf, "x");
        assert!(join(&mut buf, "", "a/./b"));
        assert_eq!(buf, "a/b");
        assert!(!join(&mut buf, "a", "../../x"), "above the root");
        assert_eq!(dir_of("a/b/c.rs"), "a/b");
        assert_eq!(dir_of("c.rs"), "");
    }

    #[test]
    fn use_trees_are_flattened() {
        let leaves = |tree: &str| {
            let mut out = Vec::new();
            use_leaves(tree, |segs| out.push(segs.join("::")));
            out
        };
        assert_eq!(leaves("crate::a::b::C"), ["crate::a::b::C"]);
        assert_eq!(
            leaves("crate::{a::X, b::{Y, Z as W}, c::*}"),
            ["crate::a::X", "crate::b::Y", "crate::b::Z", "crate::c"]
        );
        assert_eq!(leaves("super::{self, r#type}"), ["super::self", "super::type"]);
        assert_eq!(leaves("::std::fmt"), ["std::fmt"]);
    }

    #[test]
    fn typescript_imports() {
        let app = "\
import { a } from './util';
import widgets from './widgets';
import type { T } from './types';
export * from './util';
import {
  one,
  two,
} from './multi';
const legacy = require('./legacy');
const lazy = () => import('./lazy');
import { b } from '@/lib/b';
import React from 'react';
import missing from './missing';
// import hidden from './hidden';
import './app';
import { c } from './esm.js';
import { again } from './util';
export { one } from \"./multi\";
import data from '../data.json';
/* require('./hidden') */
  * import('./hidden')
const fs = require('fs'); // require('./hidden')
export const sql = \"select x from './hidden'\";
export type { T } from './types';
";
        let repo = repo(
            "ts",
            &[
                ("data.json", "{}\n"),
                ("src/app.ts", app),
                ("src/util.ts", "export const a = 1;\n"),
                ("src/widgets/index.tsx", "export default 1;\n"),
                ("src/types.d.ts", "export type T = number;\n"),
                ("src/multi.js", "export const one = 1, two = 2;\n"),
                ("src/legacy.cjs", "module.exports = {};\n"),
                ("src/lazy.vue", "<script>\nimport { a } from './util'\n</script>\n"),
                ("src/lib/b.ts", "export const b = 1;\n"),
                ("src/esm.ts", "export const c = 1;\n"),
                ("src/hidden.ts", "export default 1;\n"),
            ],
        );
        let deps = analyze(&repo);
        check(&repo, &deps);
        assert_eq!(
            out(&repo, &deps, "src/app.ts"),
            [
                ("data.json", 18, DepKind::Import),
                ("src/esm.ts", 15, DepKind::Import),
                ("src/lazy.vue", 9, DepKind::Import),
                ("src/legacy.cjs", 8, DepKind::Import),
                ("src/lib/b.ts", 10, DepKind::Import),
                ("src/multi.js", 4, DepKind::Import),
                ("src/multi.js", 17, DepKind::ReExport),
                ("src/types.d.ts", 2, DepKind::TypeOnly),
                ("src/types.d.ts", 23, DepKind::ReExport),
                ("src/util.ts", 0, DepKind::Import),
                ("src/util.ts", 3, DepKind::ReExport),
                ("src/widgets/index.tsx", 1, DepKind::Import),
            ]
        );
        assert_eq!(out(&repo, &deps, "src/lazy.vue"), [("src/util.ts", 1, DepKind::Import)]);
        assert!(incoming(&repo, &deps, "src/hidden.ts").is_empty(), "comments and strings import nothing");
        assert_eq!(incoming(&repo, &deps, "src/util.ts"), ["src/app.ts", "src/app.ts", "src/lazy.vue"]);
        assert_eq!(deps.external, 2, "react and fs");
        assert_eq!(deps.unresolved, 1, "./missing");
        assert_eq!(deps.pairs(), 10);
    }

    #[test]
    fn aliases_resolve_from_the_nearest_package() {
        let repo = repo(
            "alias",
            &[
                ("src/y/b.ts", "export const root = 1;\n"),
                ("packages/web/package.json", "{}\n"),
                ("packages/web/src/y/b.ts", "export const b = 1;\n"),
                ("packages/web/src/x/a.ts", "import { b } from '@/y/b';\nimport '~/y/b';\nimport 'src/y/b';\n"),
                ("packages/api/tsconfig.json", "{}\n"),
                ("packages/api/lib/c.ts", "export const c = 1;\n"),
                ("packages/api/lib/d.d.ts", "export type D = number;\n"),
                (
                    "packages/api/routes/r.ts",
                    "import { c } from '@/lib/c';\nimport { n } from '@/nope';\nimport type { D } from '#lib/d.ts';\n",
                ),
                ("tools/t.ts", "import { root } from '@/y/b';\nimport { s } from '@scope/pkg';\n"),
            ],
        );
        let deps = analyze(&repo);
        check(&repo, &deps);
        assert_eq!(out(&repo, &deps, "packages/web/src/x/a.ts"), [("packages/web/src/y/b.ts", 0, DepKind::Import)]);
        assert_eq!(
            out(&repo, &deps, "packages/api/routes/r.ts"),
            [("packages/api/lib/c.ts", 0, DepKind::Import), ("packages/api/lib/d.d.ts", 2, DepKind::TypeOnly)]
        );
        assert_eq!(out(&repo, &deps, "tools/t.ts"), [("src/y/b.ts", 0, DepKind::Import)]);
        assert_eq!((deps.unresolved, deps.external), (1, 1), "@/nope; @scope/pkg");
    }

    #[test]
    fn c_includes() {
        let view = "\
#include \"view.h\"
#include \"base/logging.h\"
#include <vector>
#include <base/macros.h>
#include \"nope.h\"
// #include \"ui/hidden.h\"
#  include \"../base/./macros.h\"
#include \"ui/view.cc\"
#include GENERATED_HEADER
";
        let repo = repo(
            "cpp",
            &[
                ("base/logging.h", "#pragma once\n"),
                ("base/macros.h", "#define X 1\n"),
                ("ui/hidden.h", "\n"),
                ("ui/view.cc", view),
                ("ui/view.h", "#include \"base/logging.h\"\n"),
                ("ui/view_mac.mm", "#import \"view.h\"\n#import <Cocoa/Cocoa.h>\n"),
            ],
        );
        let deps = analyze(&repo);
        check(&repo, &deps);
        assert_eq!(
            out(&repo, &deps, "ui/view.cc"),
            [
                ("base/logging.h", 1, DepKind::Include),
                ("base/macros.h", 3, DepKind::Include),
                ("ui/view.h", 0, DepKind::Include),
            ]
        );
        assert_eq!(out(&repo, &deps, "ui/view_mac.mm"), [("ui/view.h", 0, DepKind::Include)]);
        assert_eq!(incoming(&repo, &deps, "base/logging.h"), ["ui/view.cc", "ui/view.h"]);
        assert!(incoming(&repo, &deps, "ui/hidden.h").is_empty());
        assert_eq!(deps.external, 2, "<vector> and <Cocoa/Cocoa.h>");
        assert_eq!(deps.unresolved, 1, "nope.h");
        assert_eq!(deps.counts[DepKind::Include.index()] as usize, deps.edges.len());
    }

    #[test]
    fn rust_modules_and_uses() {
        let lib = "\
mod scan;
pub mod syntax;
mod layout;
mod missing;
mod inline {
    mod nested;
}
pub use crate::scan::Repo;
use crate::Thing;
#[cfg(windows)]
#[path = \"sys/windows.rs\"]
mod sys;
#[path = \"sys/nope.rs\"]
mod sys;
macro_rules! cases {
    ($name:ident) => {
        mod $name {
            use super::*;
        }
    };
}
";
        let scan_rs = "\
use crate::syntax::Lang;
use std::fmt;
mod walk;
use self::walk::Walker;
use crate::{
    layout::Layout, // a comment
    syntax::{Token, Tokens},
};

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::layout::Layout;
}
";
        let repo = repo(
            "rust",
            &[
                ("Cargo.toml", "[package]\n"),
                ("src/lib.rs", lib),
                ("src/scan.rs", scan_rs),
                ("src/scan/walk.rs", "use super::Repo;\nuse super::super::syntax::{Lang, Token};\nuse crate::Thing;\n"),
                ("src/syntax.rs", "pub struct Lang;\n"),
                ("src/layout/mod.rs", "use crate::{scan::Repo, syntax::Lang};\nuse super::scan;\nmod grid;\n"),
                ("src/layout/grid.rs", "use super::Layout;\n// use crate::syntax::Lang;\n"),
                ("src/bin/tool.rs", "mod helper;\n"),
                ("src/bin/helper.rs", "pub fn help() {}\n"),
                ("src/sys/windows.rs", "use crate::scan::Repo;\n"),
                ("flat/Cargo.toml", "[package]\n"),
                ("flat/lib.rs", "mod sink;\n"),
                ("flat/sink.rs", "pub struct Push;\n"),
                ("flat/parser.rs", "use crate::sink::Push;\nuse crate::Thing;\n"),
            ],
        );
        let deps = analyze(&repo);
        check(&repo, &deps);
        assert_eq!(
            out(&repo, &deps, "src/lib.rs"),
            [
                ("src/layout/mod.rs", 2, DepKind::Module),
                ("src/scan.rs", 0, DepKind::Module),
                ("src/scan.rs", 7, DepKind::ReExport),
                ("src/syntax.rs", 1, DepKind::Module),
                ("src/sys/windows.rs", 11, DepKind::Module),
            ],
            "nothing for `mod nested;` in an inline module nor for `use super::*` in a macro's module"
        );
        assert_eq!(
            out(&repo, &deps, "src/scan.rs"),
            [
                ("src/layout/mod.rs", 4, DepKind::Import),
                ("src/scan/walk.rs", 2, DepKind::Module),
                ("src/scan/walk.rs", 3, DepKind::Import),
                ("src/syntax.rs", 0, DepKind::Import),
            ],
            "`use super::*` inside `mod tests` stays in the file"
        );
        assert_eq!(
            out(&repo, &deps, "src/scan/walk.rs"),
            [
                ("src/lib.rs", 2, DepKind::Import),
                ("src/scan.rs", 0, DepKind::Import),
                ("src/syntax.rs", 1, DepKind::Import)
            ]
        );
        assert_eq!(
            out(&repo, &deps, "src/layout/mod.rs"),
            [
                ("src/layout/grid.rs", 2, DepKind::Module),
                ("src/scan.rs", 0, DepKind::Import),
                ("src/syntax.rs", 0, DepKind::Import),
            ]
        );
        assert_eq!(out(&repo, &deps, "src/layout/grid.rs"), [("src/layout/mod.rs", 0, DepKind::Import)]);
        assert_eq!(out(&repo, &deps, "src/bin/tool.rs"), [("src/bin/helper.rs", 0, DepKind::Module)]);
        assert_eq!(out(&repo, &deps, "src/sys/windows.rs"), [("src/scan.rs", 0, DepKind::Import)]);
        assert_eq!(
            out(&repo, &deps, "flat/parser.rs"),
            [("flat/lib.rs", 1, DepKind::Import), ("flat/sink.rs", 0, DepKind::Import)],
            "the crate root sits next to the manifest"
        );
        assert_eq!(deps.external, 1, "std::fmt");
        assert_eq!(deps.unresolved, 2, "mod missing and the wrong #[path]");
    }

    #[test]
    fn python_imports() {
        let a = "\
import os
import pkg.b
import pkg.sub.c as c, sys
from pkg.sub import c
from . import b
from .sub import c, thing
from .nope import x
from pkg import (
    b,
    VALUE,
)
# import pkg.hidden
from . import a
";
        let repo = repo(
            "python",
            &[
                ("pkg/__init__.py", "VALUE = 1\n"),
                ("pkg/a.py", a),
                ("pkg/b.py", "\n"),
                ("pkg/hidden.py", "\n"),
                ("pkg/sub/__init__.py", "thing = 1\n"),
                (
                    "pkg/sub/c.py",
                    "from .. import b\nfrom ..b import thing\nfrom ... import top\nfrom .... import nothing\n",
                ),
                ("top.py", "\n"),
                ("tools/scripts/helper.py", "\n"),
                ("tools/scripts/run.py", "import helper\nfrom helper import go\n    import json\n"),
            ],
        );
        let deps = analyze(&repo);
        check(&repo, &deps);
        assert_eq!(
            out(&repo, &deps, "pkg/a.py"),
            [
                ("pkg/__init__.py", 7, DepKind::Import),
                ("pkg/b.py", 1, DepKind::Import),
                ("pkg/sub/__init__.py", 5, DepKind::Import),
                ("pkg/sub/c.py", 2, DepKind::Import),
            ]
        );
        assert_eq!(
            out(&repo, &deps, "pkg/sub/c.py"),
            [("pkg/b.py", 0, DepKind::Import), ("top.py", 2, DepKind::Import)]
        );
        assert_eq!(out(&repo, &deps, "tools/scripts/run.py"), [("tools/scripts/helper.py", 0, DepKind::Import)]);
        assert!(incoming(&repo, &deps, "pkg/hidden.py").is_empty());
        assert_eq!(deps.external, 3, "os, sys, json");
        assert_eq!(deps.unresolved, 2, ".nope and the four dots");
    }

    #[test]
    fn java_and_kotlin_imports() {
        let main = "\
package com.acme;

import com.acme.util.Strings;
import static com.acme.util.Strings.join;
import com.acme.model.*;
import com.acme.model.User.Role;
import java.util.List;
import com.acme.dup.Twice;
// import com.acme.model.Hidden;
";
        let repo = repo(
            "java",
            &[
                ("app/src/main/java/com/acme/Main.java", main),
                ("app/src/main/java/com/acme/util/Strings.java", "package com.acme.util;\n"),
                ("app/src/main/java/com/acme/model/User.java", "package com.acme.model;\n"),
                ("app/src/main/java/com/acme/model/Hidden.java", "package com.acme.model;\n"),
                ("app/src/main/java/com/acme/dup/Twice.java", "package com.acme.dup;\n"),
                ("lib/src/main/java/com/acme/dup/Twice.java", "package com.acme.dup;\n"),
                (
                    "app/src/main/kotlin/com/acme/App.kt",
                    "import com.acme.Main\nimport com.acme.util.Strings as S\nimport kotlin.math.max\n",
                ),
            ],
        );
        let deps = analyze(&repo);
        check(&repo, &deps);
        assert_eq!(
            out(&repo, &deps, "app/src/main/java/com/acme/Main.java"),
            [
                ("app/src/main/java/com/acme/model/User.java", 5, DepKind::Import),
                ("app/src/main/java/com/acme/util/Strings.java", 2, DepKind::Import),
            ]
        );
        assert_eq!(
            out(&repo, &deps, "app/src/main/kotlin/com/acme/App.kt"),
            [
                ("app/src/main/java/com/acme/Main.java", 0, DepKind::Import),
                ("app/src/main/java/com/acme/util/Strings.java", 1, DepKind::Import),
            ]
        );
        assert!(incoming(&repo, &deps, "app/src/main/java/com/acme/model/Hidden.java").is_empty());
        assert_eq!(deps.external, 4, "the wildcard, java.util, the ambiguous class, kotlin.math");
        assert_eq!(deps.unresolved, 0);
    }

    #[test]
    fn other_languages_and_empty_repositories_give_nothing() {
        let repo = repo(
            "other",
            &[
                ("main.go", "package main\nimport \"./util\"\n"),
                ("Program.cs", "using System;\n"),
                ("notes.md", "import x from './main.go'\n#include \"main.go\"\n"),
                ("a.ts", "import './a';\nimport './b';\n"),
                ("b.ts", "import './a';\nimport './a.ts';\n"),
            ],
        );
        let deps = analyze(&repo);
        check(&repo, &deps);
        assert_eq!(out(&repo, &deps, "a.ts"), [("b.ts", 1, DepKind::Import)], "no edge to itself");
        assert_eq!(out(&repo, &deps, "b.ts"), [("a.ts", 0, DepKind::Import)], "the first line of a repeated import");
        assert_eq!((deps.edges.len(), deps.pairs(), deps.unresolved, deps.external), (2, 2, 0, 0));

        let empty = Deps::default();
        assert!(empty.outgoing(0).is_empty());
        assert_eq!((empty.incoming(0).count(), empty.pairs()), (0, 0));
    }

    #[test]
    #[ignore]
    fn bench_repo_from_env() {
        let Ok(root) = std::env::var("CODE_CANVAS_DEPS_REPO") else { return };
        let repo = scan(std::path::Path::new(&root), &ScanOptions::default(), None);
        println!(
            "{}: {} files, {} lines, scanned in {} ms",
            repo.name,
            repo.files.len(),
            repo.total_lines,
            repo.scan_ms
        );
        let deps = (0..3).map(|_| analyze(&repo)).min_by_key(|d| d.ms).unwrap();
        check(&repo, &deps);
        println!(
            "analyze: {} ms, {} edges, {} pairs, {} unresolved, {} external",
            deps.ms,
            deps.edges.len(),
            deps.pairs(),
            deps.unresolved,
            deps.external
        );
        for k in DepKind::ALL {
            println!("  {:<13}{}", k.label(), deps.counts[k.index()]);
        }
        let t = Instant::now();
        let ix = Index::build(&repo.files);
        let build = t.elapsed();
        let t = Instant::now();
        let hits = repo.files.iter().filter(|f| ix.find(&f.rel_path).is_some()).count();
        let hash = t.elapsed();
        let t = Instant::now();
        let same = repo.files.iter().filter(|f| repo.file_index(&f.rel_path).is_some()).count();
        let search = t.elapsed();
        assert_eq!(hits, same);
        let per = |d: std::time::Duration| d.as_nanos() / repo.files.len().max(1) as u128;
        println!("index built in {build:?}; per lookup: hash {} ns, Repo::file_index {} ns", per(hash), per(search));
    }
}
