use crate::syntax::{Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Type,
    Const,
    Module,
    Impl,
}

impl SymbolKind {
    pub fn badge(&self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Type => "type",
            SymbolKind::Const => "const",
            SymbolKind::Module => "mod",
            SymbolKind::Impl => "impl",
        }
    }

    pub fn is_callable(&self) -> bool {
        matches!(self, SymbolKind::Function | SymbolKind::Method)
    }

    pub fn is_container(&self) -> bool {
        matches!(
            self,
            SymbolKind::Class | SymbolKind::Struct | SymbolKind::Trait | SymbolKind::Interface | SymbolKind::Impl
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub name_start: u32,
    pub name_len: u16,
    pub col: u16,
    pub line: u32,
    pub body_end: u32,
    pub parent: u32,
}

impl Symbol {
    pub fn name<'a>(&self, text: &'a str) -> &'a str {
        &text[self.name_start as usize..self.name_start as usize + self.name_len as usize]
    }

    pub fn contains_line(&self, line: u32) -> bool {
        line >= self.line && line < self.body_end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dialect {
    Rust,
    Js,
    Python,
    Go,
    CFamily,
    None,
}

impl Dialect {
    pub fn from_path(path: &str) -> Dialect {
        let lower = path.to_ascii_lowercase();
        let ext = lower.rsplit(['/', '\\']).next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e).unwrap_or("");
        match ext {
            "rs" => Dialect::Rust,
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" => Dialect::Js,
            "py" | "pyi" => Dialect::Python,
            "go" => Dialect::Go,
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" | "cs" | "java" | "kt" | "kts" | "scala"
            | "swift" | "dart" | "php" | "zig" => Dialect::CFamily,
            _ => Dialect::None,
        }
    }
}

struct Open {
    sym: u32,
    depth: i32,
    indent: u32,
}

struct Pending {
    sym: u32,
    tentative: bool,
    expirable: bool,
}

struct Det {
    kind: SymbolKind,
    name: usize,
    tentative: bool,
    expirable: bool,
    trait_name: Option<usize>,
}

impl Det {
    fn new(kind: SymbolKind, name: usize) -> Det {
        Det { kind, name, tentative: false, expirable: false, trait_name: None }
    }
}

pub struct SymbolScanner {
    dialect: Dialect,
    depth: i32,
    paren: i32,
    open: Vec<Open>,
    pending: Option<Pending>,
    out: Vec<Symbol>,
    impl_traits: Vec<(u32, u32, u16)>,
    last_code_line: u32,
}

fn is_name(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Ident | TokenKind::Function | TokenKind::Type)
}

fn name_like(text: &str, t: &Token) -> bool {
    matches!(t.kind, TokenKind::Ident | TokenKind::Function | TokenKind::Type | TokenKind::Keyword)
        && text[t.range()].chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
}

impl SymbolScanner {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            depth: 0,
            paren: 0,
            open: Vec::new(),
            pending: None,
            out: Vec::new(),
            impl_traits: Vec::new(),
            last_code_line: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.dialect != Dialect::None
    }

    fn parent(&self) -> Option<&Symbol> {
        self.open.last().map(|o| &self.out[o.sym as usize])
    }

    fn push(&mut self, det: &Det, line: u32, tokens: &[Token]) -> u32 {
        let t = &tokens[det.name];
        let mut kind = det.kind;
        if kind == SymbolKind::Function && self.parent().is_some_and(|p| p.kind.is_container()) {
            kind = SymbolKind::Method;
        }
        let len = if t.kind == TokenKind::Macro { t.len.saturating_sub(1) } else { t.len };
        self.out.push(Symbol {
            kind,
            name_start: t.start,
            name_len: len,
            col: t.col,
            line,
            body_end: line + 1,
            parent: self.open.last().map(|o| o.sym).unwrap_or(u32::MAX),
        });
        (self.out.len() - 1) as u32
    }

    pub fn feed(&mut self, line: u32, text: &str, tokens: &[Token]) {
        if self.dialect == Dialect::None || tokens.is_empty() {
            return;
        }
        if tokens.iter().all(|t| t.kind == TokenKind::Comment) {
            return;
        }
        if self.dialect == Dialect::Python {
            self.feed_python(line, text, tokens);
        } else {
            self.feed_braces(line, text, tokens);
        }
        self.last_code_line = line;
    }

    fn feed_python(&mut self, line: u32, text: &str, tokens: &[Token]) {
        let indent = tokens[0].col as u32;
        if self.paren == 0 {
            while let Some(top) = self.open.last() {
                if indent <= top.indent {
                    self.out[top.sym as usize].body_end = self.last_code_line + 1;
                    self.open.pop();
                } else {
                    break;
                }
            }
        }
        let s = |i: usize| tokens.get(i).map(|t| &text[t.range()]);
        let mut i = 0;
        if s(i) == Some("async") {
            i += 1;
        }
        let kind = match s(i) {
            Some("def") => Some(SymbolKind::Function),
            Some("class") => Some(SymbolKind::Class),
            _ => None,
        };
        if let (Some(kind), Some(name)) = (kind, tokens.get(i + 1)) {
            if name_like(text, name) && self.paren == 0 {
                let sym = self.push(&Det::new(kind, i + 1), line, tokens);
                self.open.push(Open { sym, depth: 0, indent });
            }
        }
        for t in tokens.iter().filter(|t| t.kind == TokenKind::Punct) {
            for ch in text[t.range()].chars() {
                match ch {
                    '(' | '[' | '{' => self.paren += 1,
                    ')' | ']' | '}' => self.paren = (self.paren - 1).max(0),
                    _ => {}
                }
            }
        }
    }

    fn feed_braces(&mut self, line: u32, text: &str, tokens: &[Token]) {
        let det = match self.dialect {
            Dialect::Rust => detect_rust(text, tokens),
            Dialect::Js => self.detect_js(text, tokens),
            Dialect::Go => detect_go(text, tokens),
            Dialect::CFamily => self.detect_c(text, tokens),
            _ => None,
        };
        let name_idx = det.as_ref().map(|d| d.name);
        for (i, t) in tokens.iter().enumerate() {
            if Some(i) == name_idx {
                let d = det.as_ref().unwrap();
                let sym = self.push(d, line, tokens);
                if let Some(ti) = d.trait_name {
                    self.impl_traits.push((sym, tokens[ti].start, tokens[ti].len));
                }
                self.pending = Some(Pending { sym, tentative: d.tentative, expirable: d.expirable });
                self.paren = 0;
            }
            if t.kind != TokenKind::Punct {
                continue;
            }
            for ch in text[t.range()].chars() {
                match ch {
                    '(' | '[' => self.paren += 1,
                    ')' | ']' => self.paren = (self.paren - 1).max(0),
                    '{' => {
                        if let Some(p) = self.pending.take() {
                            self.open.push(Open { sym: p.sym, depth: self.depth, indent: 0 });
                        }
                        self.depth += 1;
                    }
                    '}' => {
                        self.depth = (self.depth - 1).max(0);
                        while let Some(top) = self.open.last() {
                            if self.depth <= top.depth {
                                self.out[top.sym as usize].body_end = line + 1;
                                self.open.pop();
                            } else {
                                break;
                            }
                        }
                    }
                    ';' if self.paren == 0 => {
                        if let Some(p) = self.pending.take() {
                            if p.tentative && p.sym as usize == self.out.len() - 1 {
                                self.out.pop();
                                self.impl_traits.retain(|t| t.0 != p.sym);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if self.paren == 0 && self.pending.as_ref().is_some_and(|p| p.expirable) {
            let last = &text[tokens[tokens.len() - 1].range()];
            let continues = [",", "(", "[", "=", "=>", "|", "&", "+", "-", "*", ":", "?", "<", "."]
                .iter()
                .any(|c| last.ends_with(c))
                || matches!(last, "extends" | "implements");
            if !continues {
                self.pending = None;
            }
        }
    }

    fn detect_js(&self, text: &str, tokens: &[Token]) -> Option<Det> {
        let s = |i: usize| tokens.get(i).map(|t| &text[t.range()]);
        let mut i = 0;
        while let Some(w) = s(i) {
            let modifier = matches!(
                w,
                "export"
                    | "default"
                    | "declare"
                    | "abstract"
                    | "async"
                    | "public"
                    | "private"
                    | "protected"
                    | "static"
                    | "readonly"
                    | "override"
            ) || (matches!(w, "get" | "set") && tokens[i].kind != TokenKind::Function);
            if !modifier {
                break;
            }
            i += 1;
        }
        let word = s(i)?;
        let named =
            |kind: SymbolKind, at: usize| tokens.get(at).filter(|t| name_like(text, t)).map(|_| Det::new(kind, at));
        match word {
            "function" => {
                let at = if s(i + 1) == Some("*") { i + 2 } else { i + 1 };
                return named(SymbolKind::Function, at);
            }
            "class" => return named(SymbolKind::Class, i + 1),
            "interface" => return named(SymbolKind::Interface, i + 1),
            "enum" => return named(SymbolKind::Enum, i + 1),
            "namespace" | "module" => return named(SymbolKind::Module, i + 1),
            "type" => {
                let next = s(i + 2)?;
                if next.starts_with('=') || next.starts_with('<') {
                    let mut d = named(SymbolKind::Type, i + 1)?;
                    d.expirable = true;
                    return Some(d);
                }
                return None;
            }
            "const" | "let" | "var" => {
                tokens.get(i + 1).filter(|t| is_name(t))?;
                let rest = &tokens[i + 2..];
                let has_eq = rest.iter().any(|t| t.kind == TokenKind::Punct && text[t.range()].contains('='));
                if !has_eq {
                    return None;
                }
                let arrow = rest.iter().any(|t| t.kind == TokenKind::Punct && text[t.range()].contains("=>"))
                    || rest.iter().any(|t| &text[t.range()] == "function");
                let open_call = rest.last().is_some_and(|t| text[t.range()].ends_with('('));
                let kind = if arrow || open_call {
                    SymbolKind::Function
                } else if self.depth == 0 {
                    SymbolKind::Const
                } else {
                    return None;
                };
                let mut d = Det::new(kind, i + 1);
                d.expirable = true;
                return Some(d);
            }
            _ => {}
        }
        let top = self.open.last()?;
        let parent = &self.out[top.sym as usize];
        if !matches!(parent.kind, SymbolKind::Class | SymbolKind::Interface) || self.depth != top.depth + 1 {
            return None;
        }
        let t = tokens.get(i)?;
        let control = matches!(
            word,
            "if" | "for"
                | "while"
                | "switch"
                | "catch"
                | "return"
                | "super"
                | "throw"
                | "else"
                | "do"
                | "try"
                | "case"
                | "await"
                | "yield"
                | "typeof"
                | "import"
                | "from"
        );
        if control || !name_like(text, t) {
            return None;
        }
        let rest = &tokens[i + 1..];
        let called = rest.first().is_some_and(|n| n.kind == TokenKind::Punct && text[n.range()].starts_with('('));
        let is_method = t.kind == TokenKind::Function
            || (t.kind == TokenKind::Keyword && called)
            || rest.iter().any(|t| t.kind == TokenKind::Punct && text[t.range()].contains("=>"));
        if !is_method {
            return None;
        }
        let mut d = Det::new(SymbolKind::Method, i);
        d.expirable = true;
        Some(d)
    }

    fn detect_c(&self, text: &str, tokens: &[Token]) -> Option<Det> {
        let s = |i: usize| tokens.get(i).map(|t| &text[t.range()]);
        let mut i = 0;
        while let Some(w) = s(i) {
            if !matches!(
                w,
                "public"
                    | "private"
                    | "protected"
                    | "internal"
                    | "static"
                    | "final"
                    | "abstract"
                    | "virtual"
                    | "override"
                    | "async"
                    | "sealed"
                    | "partial"
                    | "inline"
                    | "extern"
                    | "unsafe"
                    | "export"
                    | "open"
                    | "data"
                    | "suspend"
                    | "synchronized"
                    | "native"
                    | "constexpr"
                    | "explicit"
                    | "friend"
                    | "template"
            ) {
                break;
            }
            i += 1;
        }
        let word = s(i)?;
        let kind = match word {
            "class" | "object" | "extension" | "record" => Some(SymbolKind::Class),
            "struct" | "union" => Some(SymbolKind::Struct),
            "interface" | "protocol" => Some(SymbolKind::Interface),
            "enum" => Some(SymbolKind::Enum),
            "trait" => Some(SymbolKind::Trait),
            "namespace" | "package" if self.dialect == Dialect::CFamily => Some(SymbolKind::Module),
            _ => None,
        };
        if let Some(kind) = kind {
            let at = if matches!(s(i + 1), Some("class" | "struct")) { i + 2 } else { i + 1 };
            let t = tokens.get(at).filter(|t| is_name(t))?;
            if t.kind == TokenKind::Function {
                return None;
            }
            let mut d = Det::new(kind, at);
            d.tentative = true;
            return Some(d);
        }
        if matches!(word, "fun" | "func" | "function" | "def" | "fn") {
            let at = (i + 1..tokens.len())
                .find(|&j| tokens[j].kind == TokenKind::Function)
                .or_else(|| tokens.get(i + 1).filter(|t| name_like(text, t)).map(|_| i + 1))?;
            return Some(Det::new(SymbolKind::Function, at));
        }
        if let Some(top) = self.open.last() {
            let p = &self.out[top.sym as usize];
            if !(p.kind.is_container() || matches!(p.kind, SymbolKind::Module | SymbolKind::Enum))
                || self.depth != top.depth + 1
            {
                return None;
            }
        } else if self.depth != 0 {
            return None;
        }
        if tokens[i].kind == TokenKind::Keyword
            && matches!(
                word,
                "if" | "for"
                    | "while"
                    | "switch"
                    | "return"
                    | "else"
                    | "catch"
                    | "do"
                    | "new"
                    | "throw"
                    | "case"
                    | "using"
                    | "delete"
                    | "await"
                    | "yield"
                    | "typedef"
                    | "import"
                    | "goto"
            )
        {
            return None;
        }
        let j = (i..tokens.len()).find(|&j| tokens[j].kind == TokenKind::Function)?;
        let before = &tokens[i..j];
        if before.iter().any(|t| {
            let w = &text[t.range()];
            (t.kind == TokenKind::Punct && (w.contains('=') || w.contains('(') || w.contains('.')))
                || matches!(w, "return" | "new" | "throw" | "await")
        }) {
            return None;
        }
        if j == i {
            let ctor = self.parent().is_some_and(|p| p.name(text) == word);
            if !ctor {
                return None;
            }
        }
        let mut d = Det::new(SymbolKind::Function, j);
        d.tentative = true;
        Some(d)
    }

    pub fn finish(mut self, line_count: u32) -> (Vec<Symbol>, Vec<(u32, u32, u16)>) {
        let end = if self.dialect == Dialect::Python { self.last_code_line + 1 } else { line_count };
        for o in self.open.drain(..) {
            self.out[o.sym as usize].body_end = end.max(self.out[o.sym as usize].line + 1);
        }
        self.out.shrink_to_fit();
        (self.out, self.impl_traits)
    }
}

fn detect_rust(text: &str, tokens: &[Token]) -> Option<Det> {
    let s = |i: usize| tokens.get(i).map(|t| &text[t.range()]);
    let mut i = 0;
    while let Some(w) = s(i) {
        let t = &tokens[i];
        let skip = match w {
            "pub" | "async" | "unsafe" | "extern" | "default" | "crate" | "super" | "self" | "in" => true,
            "const" => matches!(s(i + 1), Some("fn" | "unsafe" | "async" | "extern")),
            _ => t.kind == TokenKind::String || (t.kind == TokenKind::Punct && w.chars().all(|c| "():".contains(c))),
        };
        if !skip {
            break;
        }
        i += 1;
    }
    let word = s(i)?;
    let named = |kind: SymbolKind, at: usize| tokens.get(at).filter(|t| name_like(text, t)).map(|_| Det::new(kind, at));
    match word {
        "fn" => named(SymbolKind::Function, i + 1),
        "struct" | "union" => named(SymbolKind::Struct, i + 1),
        "enum" => named(SymbolKind::Enum, i + 1),
        "trait" => named(SymbolKind::Trait, i + 1),
        "type" => named(SymbolKind::Type, i + 1),
        "mod" => named(SymbolKind::Module, i + 1),
        "const" | "static" => {
            let at = if s(i + 1) == Some("mut") { i + 2 } else { i + 1 };
            named(SymbolKind::Const, at)
        }
        "macro_rules!" => named(SymbolKind::Function, i + 1),
        "impl" => {
            let mut j = i + 1;
            if s(j).is_some_and(|w| w.starts_with('<')) {
                let mut angle = 0i32;
                while let Some(w) = s(j) {
                    if tokens[j].kind == TokenKind::Punct {
                        angle += w.matches('<').count() as i32 - w.matches('>').count() as i32;
                    }
                    j += 1;
                    if angle <= 0 {
                        break;
                    }
                }
            }
            let path_end = |from: usize| -> (Option<usize>, usize) {
                let mut last = None;
                let mut k = from;
                while let Some(w) = s(k) {
                    let t = &tokens[k];
                    if matches!(w, "for" | "where")
                        || (t.kind == TokenKind::Punct && (w.contains('<') || w.contains('{')))
                    {
                        break;
                    }
                    if is_name(t) {
                        last = Some(k);
                    }
                    k += 1;
                }
                (last, k)
            };
            let (first, mut k) = path_end(j);
            let mut angle = 0i32;
            while let Some(w) = s(k) {
                if w == "for" && angle <= 0 {
                    break;
                }
                if tokens[k].kind == TokenKind::Punct {
                    if w.contains('{') {
                        break;
                    }
                    angle += w.matches('<').count() as i32 - w.matches('>').count() as i32;
                }
                k += 1;
            }
            if s(k) == Some("for") {
                let (ty, _) = path_end(k + 1);
                let mut d = Det::new(SymbolKind::Impl, ty?);
                d.trait_name = first;
                Some(d)
            } else {
                first.map(|f| Det::new(SymbolKind::Impl, f))
            }
        }
        _ => None,
    }
}

fn detect_go(text: &str, tokens: &[Token]) -> Option<Det> {
    let s = |i: usize| tokens.get(i).map(|t| &text[t.range()]);
    match s(0)? {
        "func" => {
            if s(1).is_some_and(|w| w.starts_with('(')) {
                let close = (1..tokens.len())
                    .find(|&j| tokens[j].kind == TokenKind::Punct && text[tokens[j].range()].contains(')'))?;
                tokens.get(close + 1).filter(|t| name_like(text, t))?;
                Some(Det::new(SymbolKind::Method, close + 1))
            } else {
                tokens.get(1).filter(|t| name_like(text, t)).map(|_| Det::new(SymbolKind::Function, 1))
            }
        }
        "type" => {
            tokens.get(1).filter(|t| name_like(text, t))?;
            let kind = match s(2) {
                Some("struct") => SymbolKind::Struct,
                Some("interface") => SymbolKind::Interface,
                _ => SymbolKind::Type,
            };
            let mut d = Det::new(kind, 1);
            d.expirable = true;
            Some(d)
        }
        _ => None,
    }
}

pub fn symbol_at_line(symbols: &[Symbol], line: u32) -> Option<usize> {
    let mut i = symbols.partition_point(|s| s.line <= line).checked_sub(1)?;
    loop {
        if symbols[i].contains_line(line) {
            return Some(i);
        }
        let parent = symbols[i].parent;
        if parent == u32::MAX {
            return None;
        }
        i = parent as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{self, Lang};

    fn scan(src: &str, path: &str) -> (String, Vec<Symbol>) {
        let (text, starts, _) = crate::scan::normalise(src);
        let a = syntax::analyze(&text, &starts, Lang::from_path(path), Dialect::from_path(path));
        (text, a.symbols)
    }

    fn names(text: &str, syms: &[Symbol]) -> Vec<(String, SymbolKind, u32, u32)> {
        syms.iter().map(|s| (s.name(text).to_string(), s.kind, s.line, s.body_end)).collect()
    }

    #[test]
    fn rust_symbols_and_bodies() {
        let src = "\
pub struct Atlas {
    size: u32,
}

impl Atlas {
    pub(crate) fn new(size: u32) -> Self {
        Atlas { size }
    }

    fn draw(&self);
}

impl<T: Clone> Default for Atlas {
    fn default() -> Self { Self::new(1) }
}

pub fn rasterize(a: &Atlas) -> u32 {
    a.size
}
";
        let (text, syms) = scan(src, "atlas.rs");
        let n = names(&text, &syms);
        assert_eq!(n[0], ("Atlas".into(), SymbolKind::Struct, 0, 3));
        assert_eq!(n[1], ("Atlas".into(), SymbolKind::Impl, 4, 11));
        assert_eq!(n[2], ("new".into(), SymbolKind::Method, 5, 8));
        assert_eq!(n[3], ("draw".into(), SymbolKind::Method, 9, 10));
        assert_eq!(n[4], ("Atlas".into(), SymbolKind::Impl, 12, 15));
        assert_eq!(n[5], ("default".into(), SymbolKind::Method, 13, 14));
        assert_eq!(n[6], ("rasterize".into(), SymbolKind::Function, 16, 19));
        assert_eq!(syms[2].parent, 1);
        assert_eq!(symbol_at_line(&syms, 6), Some(2));
        assert_eq!(symbol_at_line(&syms, 9), Some(3));
        assert_eq!(symbol_at_line(&syms, 4), Some(1));
        assert_eq!(symbol_at_line(&syms, 3), None);
        assert_eq!(symbol_at_line(&syms, 17), Some(6));
    }

    #[test]
    fn typescript_symbols() {
        let src = "\
import { x } from './x'

export const API_URL = 'http://localhost'

export function formatNumber(value: number) {
  return value.toFixed(2)
}

export const useThing = (a: number) => {
  const inner = () => a + 1
  if (a) {
    return inner()
  }
  return 0
}

export class Service extends Base {
  private cache = new Map()

  async load(id: string): Promise<void> {
    await this.fetch(id)
  }

  handle = (e: Event) => {
    console.log(e)
  }
}

interface Props {
  title: string
}

type Id = string | number
";
        let (text, syms) = scan(src, "a.ts");
        let n = names(&text, &syms);
        let find =
            |name: &str| n.iter().find(|s| s.0 == name).unwrap_or_else(|| panic!("{name} missing: {n:?}")).clone();
        assert_eq!(find("API_URL"), ("API_URL".into(), SymbolKind::Const, 2, 3));
        assert_eq!(find("formatNumber"), ("formatNumber".into(), SymbolKind::Function, 4, 7));
        assert_eq!(find("useThing"), ("useThing".into(), SymbolKind::Function, 8, 15));
        assert_eq!(find("inner"), ("inner".into(), SymbolKind::Function, 9, 10));
        assert_eq!(find("Service"), ("Service".into(), SymbolKind::Class, 16, 27));
        assert_eq!(find("load"), ("load".into(), SymbolKind::Method, 19, 22));
        assert_eq!(find("handle"), ("handle".into(), SymbolKind::Method, 23, 26));
        assert_eq!(find("Props"), ("Props".into(), SymbolKind::Interface, 28, 31));
        assert_eq!(find("Id"), ("Id".into(), SymbolKind::Type, 32, 33));
        assert!(!n.iter().any(|s| s.0 == "cache" || s.0 == "fetch" || s.0 == "log"));
    }

    #[test]
    fn python_symbols() {
        let src = "\
class Repo:
    def scan(self, root):
        for p in root:
            yield p

    async def load(self):
        pass

def main():
    Repo().scan('.')
";
        let (text, syms) = scan(src, "a.py");
        let n = names(&text, &syms);
        assert_eq!(n[0], ("Repo".into(), SymbolKind::Class, 0, 7));
        assert_eq!(n[1], ("scan".into(), SymbolKind::Method, 1, 4));
        assert_eq!(n[2], ("load".into(), SymbolKind::Method, 5, 7));
        assert_eq!(n[3], ("main".into(), SymbolKind::Function, 8, 10));
    }

    #[test]
    fn go_and_c_symbols() {
        let (text, syms) = scan("type Server struct {\n\tport int\n}\n\nfunc (s *Server) Run(a int) error {\n\treturn nil\n}\n\nfunc main() {\n}\n", "a.go");
        let n = names(&text, &syms);
        assert_eq!(n[0], ("Server".into(), SymbolKind::Struct, 0, 3));
        assert_eq!(n[1], ("Run".into(), SymbolKind::Method, 4, 7));
        assert_eq!(n[2], ("main".into(), SymbolKind::Function, 8, 10));

        let (text, syms) = scan("int add(int a, int b);\n\nstatic int add(int a, int b) {\n    return helper(a) + b;\n}\n\nclass Foo {\n  public void bar() {\n    baz();\n  }\n}\n", "a.cpp");
        let n = names(&text, &syms);
        assert_eq!(n[0], ("add".into(), SymbolKind::Function, 2, 5));
        assert_eq!(n[1], ("Foo".into(), SymbolKind::Class, 6, 11));
        assert_eq!(n[2], ("bar".into(), SymbolKind::Method, 7, 10));
        assert_eq!(n.len(), 3);
    }
}
