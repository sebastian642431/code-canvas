//! A deliberately small, language-agnostic tokenizer.
//!
//! It is not a parser: it only needs to be good enough to colour code on the
//! canvas, both as readable text and as the zoomed-out "minimap" bars. Precise
//! semantic information (definitions, references, calls) will come from
//! language servers — see the roadmap in the README.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    Type,
    Function,
    Macro,
    String,
    Number,
    Comment,
    Punct,
    Ident,
    Heading,
    Text,
}

/// A coloured run inside one line. 12 bytes: large repositories have tens of
/// millions of these, so keep it small.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    /// Byte offset inside the file's (normalised) text.
    pub start: u32,
    /// Byte length (saturates at 65535; only the colouring is affected).
    pub len: u16,
    /// Column of the first character (tabs are already expanded; saturates).
    pub col: u16,
    /// Width in character cells (saturates).
    pub cols: u16,
    pub kind: TokenKind,
}

impl Token {
    pub fn end(&self) -> u32 {
        self.start + self.len as u32
    }

    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end() as usize
    }
}

/// Tokens of a whole file, stored flat: line `i` owns
/// `tokens[line_start[i]..line_start[i + 1]]`.
#[derive(Clone, Debug, Default)]
pub struct Tokens {
    pub tokens: Vec<Token>,
    pub line_start: Vec<u32>,
}

impl Tokens {
    pub fn line(&self, i: usize) -> &[Token] {
        &self.tokens[self.line_start[i] as usize..self.line_start[i + 1] as usize]
    }

    pub fn lines(&self) -> usize {
        self.line_start.len().saturating_sub(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    CLike,
    Python,
    Shell,
    Toml,
    Yaml,
    Json,
    Markdown,
    Html,
    Css,
    Sql,
    Lua,
    Plain,
}

impl Lang {
    pub fn from_path(path: &str) -> Lang {
        let lower = path.to_ascii_lowercase();
        let file = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
        match file {
            "makefile" | "dockerfile" | "justfile" | ".gitignore" | ".env" => return Lang::Shell,
            "cargo.lock" => return Lang::Toml,
            _ => {}
        }
        let ext = file.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        match ext {
            "rs" => Lang::Rust,
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" | "cs" | "java" | "kt" | "kts" | "scala"
            | "go" | "swift" | "dart" | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "php" | "wgsl" | "glsl"
            | "hlsl" | "vert" | "frag" | "zig" | "proto" | "groovy" | "gradle" | "v" | "odin" => Lang::CLike,
            "py" | "pyi" | "rb" | "r" | "pl" | "ex" | "exs" | "nim" | "cmake" => Lang::Python,
            "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" => Lang::Shell,
            "toml" | "ini" | "cfg" => Lang::Toml,
            "yaml" | "yml" => Lang::Yaml,
            "json" | "jsonc" | "json5" => Lang::Json,
            "md" | "markdown" | "mdx" | "rst" => Lang::Markdown,
            "html" | "htm" | "xml" | "svg" | "vue" | "svelte" | "astro" => Lang::Html,
            "css" | "scss" | "sass" | "less" => Lang::Css,
            "sql" => Lang::Sql,
            "lua" | "hs" => Lang::Lua,
            _ => Lang::Plain,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::CLike => "C-like",
            Lang::Python => "Python-like",
            Lang::Shell => "Shell",
            Lang::Toml => "TOML",
            Lang::Yaml => "YAML",
            Lang::Json => "JSON",
            Lang::Markdown => "Markdown",
            Lang::Html => "Markup",
            Lang::Css => "CSS",
            Lang::Sql => "SQL",
            Lang::Lua => "Lua-like",
            Lang::Plain => "Text",
        }
    }

    fn line_comment(&self) -> Option<&'static str> {
        match self {
            Lang::Rust | Lang::CLike | Lang::Json => Some("//"),
            Lang::Python | Lang::Shell | Lang::Toml | Lang::Yaml => Some("#"),
            Lang::Sql | Lang::Lua => Some("--"),
            Lang::Css | Lang::Markdown | Lang::Html | Lang::Plain => None,
        }
    }

    fn block_comment(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Lang::Rust | Lang::CLike | Lang::Css | Lang::Json | Lang::Sql => Some(("/*", "*/")),
            Lang::Html | Lang::Markdown => Some(("<!--", "-->")),
            Lang::Lua => Some(("--[[", "]]")),
            _ => None,
        }
    }

    fn is_prose(&self) -> bool {
        matches!(self, Lang::Markdown | Lang::Plain)
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        // Rust
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
            | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
            | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self" | "Self"
            | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe" | "use"
            | "where" | "while" | "yield"
            // C family / JS / TS / Java / Go / Swift / C#
            | "auto" | "case" | "catch" | "class" | "default" | "delete" | "do" | "export"
            | "extends" | "final" | "finally" | "from" | "func" | "function" | "go" | "goto"
            | "implements" | "import" | "include" | "interface" | "instanceof" | "namespace"
            | "new" | "null" | "nil" | "override" | "package" | "private" | "protected"
            | "public" | "sizeof" | "switch" | "template" | "this" | "throw" | "throws" | "try"
            | "typedef" | "typename" | "typeof" | "undefined" | "union" | "var" | "virtual"
            | "void" | "volatile" | "defer" | "chan" | "select" | "guard" | "readonly"
            | "#include" | "#define" | "#ifdef" | "#ifndef" | "#endif" | "#if" | "#pragma"
            // Python / Ruby / Lua / shell
            | "and" | "def" | "del" | "elif" | "except" | "global" | "is" | "lambda" | "not"
            | "or" | "pass" | "raise" | "with" | "None" | "True" | "False" | "end" | "then"
            | "local" | "elseif" | "require" | "module" | "begin" | "unless" | "until" | "fi"
            | "done" | "esac" | "echo" | "export_"
    )
}

fn is_primitive_type(word: &str) -> bool {
    matches!(
        word,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "int"
            | "float"
            | "double"
            | "long"
            | "short"
            | "unsigned"
            | "signed"
            | "string"
            | "boolean"
            | "number"
            | "vec2"
            | "vec3"
            | "vec4"
            | "mat4"
            | "mat3"
            | "byte"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    BlockComment,
}

/// Tokens are produced lazily, [`CHUNK_LINES`] lines at a time, and cached by
/// whoever needs them (see [`crate::TokenCache`]).
pub const CHUNK_LINES: usize = 128;
/// Lines summarised by one [`Block`].
pub const BLOCK_LINES: usize = 8;

/// Summary of [`BLOCK_LINES`] lines, enough to draw the far-away minimap
/// without keeping tokens in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// First and last column covered by code (`hi <= lo` means empty).
    pub lo: u16,
    pub hi: u16,
    pub kind: TokenKind,
}

/// What is kept per file after loading: block summaries for the minimap and
/// the tokenizer state at the start of every chunk, so any chunk can be
/// tokenized on demand without starting from the top of the file.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub blocks: Vec<Block>,
    /// `true` when chunk `i` starts inside a block comment.
    pub chunk_in_comment: Vec<bool>,
    /// Definitions found in the file, in source order.
    pub symbols: Vec<crate::symbols::Symbol>,
    /// For Rust `impl Trait for Type`: `(impl symbol, trait name start, len)`.
    pub impl_traits: Vec<(u32, u32, u16)>,
}

fn line_bounds(text: &str, line_starts: &[u32], i: usize) -> (usize, usize) {
    let start = line_starts[i] as usize;
    let end = line_starts
        .get(i + 1)
        .map(|&s| s as usize - 1) // drop the '\n'
        .unwrap_or(text.len());
    (start, end)
}

/// The kind that best represents a run: most character cells wins, with
/// comments and punctuation only winning when little else is there.
pub fn dominant_kind(cells: &[u32; 11]) -> TokenKind {
    const KINDS: [TokenKind; 11] = [
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
    let mut best = TokenKind::Ident;
    let mut best_n = 0;
    for (k, &n) in cells.iter().enumerate() {
        let n = if KINDS[k] == TokenKind::Comment || KINDS[k] == TokenKind::Punct { n / 3 } else { n };
        if n > best_n {
            best_n = n;
            best = KINDS[k];
        }
    }
    best
}

/// One pass over a whole file: block summaries, chunk start states and symbols.
pub fn analyze(text: &str, line_starts: &[u32], lang: Lang, dialect: crate::symbols::Dialect) -> Analysis {
    let n = line_starts.len();
    let mut out = Analysis {
        blocks: Vec::with_capacity(n.div_ceil(BLOCK_LINES)),
        chunk_in_comment: Vec::with_capacity(n.div_ceil(CHUNK_LINES)),
        ..Default::default()
    };
    let mut scanner = crate::symbols::SymbolScanner::new(dialect);
    let mut state = State::Code;
    let mut toks = Vec::new();
    let mut scratch = Vec::new();
    let mut block = (u16::MAX, 0u16, [0u32; 11]);
    for i in 0..n {
        if i % CHUNK_LINES == 0 {
            out.chunk_in_comment.push(state == State::BlockComment);
        }
        let (s, e) = line_bounds(text, line_starts, i);
        toks.clear();
        tokenize_line(&text[s..e], s, lang, &mut state, &mut toks, &mut scratch);
        scanner.feed(i as u32, text, &toks);
        for t in &toks {
            block.0 = block.0.min(t.col);
            block.1 = block.1.max(t.col.saturating_add(t.cols));
            block.2[t.kind as usize] += t.cols as u32;
        }
        if i % BLOCK_LINES == BLOCK_LINES - 1 || i + 1 == n {
            out.blocks.push(Block { lo: block.0, hi: block.1, kind: dominant_kind(&block.2) });
            block = (u16::MAX, 0, [0; 11]);
        }
    }
    (out.symbols, out.impl_traits) = scanner.finish(n as u32);
    out
}

/// Tokenize `lines` of normalised text (no `\r`, no tabs). `line_starts[i]`
/// is the byte offset of line `i`; `in_comment` is the state at the first
/// line (see [`Analysis::chunk_in_comment`]).
pub fn tokenize_range(text: &str, line_starts: &[u32], lang: Lang, lines: Range<usize>, in_comment: bool) -> Tokens {
    let mut out =
        Tokens { tokens: Vec::with_capacity(lines.len() * 6), line_start: Vec::with_capacity(lines.len() + 1) };
    let mut state = if in_comment { State::BlockComment } else { State::Code };
    let mut scratch = Vec::new();
    for i in lines {
        let (s, e) = line_bounds(text, line_starts, i);
        out.line_start.push(out.tokens.len() as u32);
        tokenize_line(&text[s..e], s, lang, &mut state, &mut out.tokens, &mut scratch);
    }
    out.line_start.push(out.tokens.len() as u32);
    out
}

/// Tokenize a whole text (convenience for tests and tools).
pub fn tokenize(text: &str, line_starts: &[u32], lang: Lang) -> Tokens {
    tokenize_range(text, line_starts, lang, 0..line_starts.len(), false)
}

fn tokenize_line(
    line: &str,
    base: usize,
    lang: Lang,
    state: &mut State,
    tokens: &mut Vec<Token>,
    chars: &mut Vec<(usize, char)>,
) {
    if line.is_empty() {
        return;
    }
    // Collect (byte_offset, char) once so we can look ahead cheaply.
    chars.clear();
    chars.extend(line.char_indices());
    let chars = &*chars;
    let n = chars.len();
    let byte_at = |ci: usize| if ci < n { chars[ci].0 } else { line.len() };
    let rest_at = |ci: usize| &line[byte_at(ci)..];
    let push = |tokens: &mut Vec<Token>, from: usize, to: usize, kind: TokenKind| {
        if to > from {
            tokens.push(Token {
                start: (base + byte_at(from)) as u32,
                len: (byte_at(to) - byte_at(from)).min(u16::MAX as usize) as u16,
                col: from.min(u16::MAX as usize) as u16,
                cols: (to - from).min(u16::MAX as usize) as u16,
                kind,
            });
        }
    };

    if lang.is_prose() {
        let heading = lang == Lang::Markdown && line.trim_start().starts_with('#');
        let mut i = 0;
        while i < n {
            if chars[i].1.is_whitespace() {
                i += 1;
                continue;
            }
            let s = i;
            while i < n && !chars[i].1.is_whitespace() {
                i += 1;
            }
            let kind = if heading {
                TokenKind::Heading
            } else if chars[s].1 == '`' {
                TokenKind::String
            } else {
                TokenKind::Text
            };
            push(tokens, s, i, kind);
        }
        return;
    }

    let line_comment = lang.line_comment();
    let block = lang.block_comment();
    let mut i = 0;
    while i < n {
        if *state == State::BlockComment {
            let (_, close) = block.expect("block comment state without block syntax");
            let s = i;
            match rest_at(i).find(close) {
                Some(off) => {
                    let close_byte = byte_at(i) + off + close.len();
                    while i < n && chars[i].0 < close_byte {
                        i += 1;
                    }
                    *state = State::Code;
                }
                None => i = n,
            }
            push(tokens, s, i, TokenKind::Comment);
            continue;
        }

        let c = chars[i].1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let rest = rest_at(i);
        if let Some(lc) = line_comment {
            if rest.starts_with(lc) {
                push(tokens, i, n, TokenKind::Comment);
                break;
            }
        }
        if let Some((open, _)) = block {
            if rest.starts_with(open) {
                *state = State::BlockComment;
                // Consume the opener so `/*/` isn't treated as open+close.
                i += open.chars().count();
                let s = i - open.chars().count();
                // Continue scanning for the close on the same line.
                let (_, close) = block.unwrap();
                match rest_at(i).find(close) {
                    Some(off) => {
                        let close_byte = byte_at(i) + off + close.len();
                        while i < n && chars[i].0 < close_byte {
                            i += 1;
                        }
                        *state = State::Code;
                    }
                    None => i = n,
                }
                push(tokens, s, i, TokenKind::Comment);
                continue;
            }
        }

        // Rust lifetimes ('a) vs char literals ('a').
        if c == '\'' && lang == Lang::Rust {
            let is_char_lit = (i + 2 < n && chars[i + 2].1 == '\'') || (i + 1 < n && chars[i + 1].1 == '\\');
            if !is_char_lit {
                let s = i;
                i += 1;
                while i < n && (chars[i].1.is_alphanumeric() || chars[i].1 == '_') {
                    i += 1;
                }
                push(tokens, s, i, TokenKind::Type);
                continue;
            }
        }

        if c == '"' || c == '\'' || c == '`' {
            let s = i;
            i += 1;
            while i < n {
                let ch = chars[i].1;
                if ch == '\\' {
                    i += 2;
                    continue;
                }
                i += 1;
                if ch == c {
                    break;
                }
            }
            let i2 = i.min(n);
            push(tokens, s, i2, TokenKind::String);
            i = i2;
            continue;
        }

        if c.is_ascii_digit() {
            let s = i;
            while i < n && (chars[i].1.is_alphanumeric() || chars[i].1 == '_' || chars[i].1 == '.') {
                i += 1;
            }
            push(tokens, s, i, TokenKind::Number);
            continue;
        }

        let starts_ident = c.is_alphabetic()
            || c == '_'
            || (c == '#' && lang == Lang::CLike && i + 1 < n && chars[i + 1].1.is_alphabetic())
            || (c == '@' && i + 1 < n && chars[i + 1].1.is_alphabetic())
            || (c == '$' && lang == Lang::Shell);
        if starts_ident {
            let s = i;
            i += 1;
            while i < n && (chars[i].1.is_alphanumeric() || chars[i].1 == '_' || chars[i].1 == '-' && lang == Lang::Css)
            {
                i += 1;
            }
            let word = &line[byte_at(s)..byte_at(i)];
            let next = chars[i..].iter().map(|c| c.1).find(|c| !c.is_whitespace());
            let kind = if c == '@' || c == '#' {
                TokenKind::Macro
            } else if lang == Lang::Rust && next == Some('!') && i < n && chars[i].1 == '!' {
                i += 1; // include the bang: println!
                TokenKind::Macro
            } else if is_keyword(word) {
                TokenKind::Keyword
            } else if is_primitive_type(word) {
                TokenKind::Type
            } else if next == Some('(') {
                TokenKind::Function
            } else if word.chars().next().is_some_and(|ch| ch.is_uppercase()) {
                TokenKind::Type
            } else if matches!(lang, Lang::Toml | Lang::Yaml | Lang::Json) && next == Some('=')
                || matches!(lang, Lang::Yaml) && next == Some(':')
            {
                TokenKind::Function
            } else {
                TokenKind::Ident
            };
            push(tokens, s, i, kind);
            continue;
        }

        // Punctuation run.
        let s = i;
        while i < n {
            let ch = chars[i].1;
            if ch.is_alphanumeric() || ch.is_whitespace() || ch == '_' || ch == '"' || ch == '\'' || ch == '`' {
                break;
            }
            if i > s {
                let r = rest_at(i);
                if line_comment.is_some_and(|lc| r.starts_with(lc)) || block.is_some_and(|(o, _)| r.starts_with(o)) {
                    break;
                }
            }
            i += 1;
        }
        if i == s {
            i += 1;
        }
        push(tokens, s, i, TokenKind::Punct);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str, lang: Lang) -> Vec<(String, TokenKind)> {
        let mut starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i as u32 + 1);
            }
        }
        tokenize(src, &starts, lang).tokens.into_iter().map(|t| (src[t.range()].to_string(), t.kind)).collect()
    }

    #[test]
    fn rust_basics() {
        let k = kinds("pub fn main() { println!(\"hi\"); } // done", Lang::Rust);
        assert!(k.contains(&("pub".into(), TokenKind::Keyword)));
        assert!(k.contains(&("main".into(), TokenKind::Function)));
        assert!(k.contains(&("println!".into(), TokenKind::Macro)));
        assert!(k.contains(&("\"hi\"".into(), TokenKind::String)));
        assert!(k.contains(&("// done".into(), TokenKind::Comment)));
    }

    #[test]
    fn rust_lifetime_vs_char() {
        let k = kinds("fn f<'a>(c: char) { let x = 'z'; }", Lang::Rust);
        assert!(k.contains(&("'a".into(), TokenKind::Type)));
        assert!(k.contains(&("'z'".into(), TokenKind::String)));
    }

    #[test]
    fn block_comment_spans_lines() {
        let k = kinds("a /* one\ntwo */ b", Lang::CLike);
        assert_eq!(k[0], ("a".into(), TokenKind::Ident));
        assert_eq!(k[1], ("/* one".into(), TokenKind::Comment));
        assert_eq!(k[2], ("two */".into(), TokenKind::Comment));
        assert_eq!(k[3], ("b".into(), TokenKind::Ident));
    }

    #[test]
    fn columns_track_chars_not_bytes() {
        let src = "é = 1";
        let mut starts = vec![0u32];
        starts.truncate(1);
        let t = tokenize(src, &starts, Lang::Python);
        let one = t.line(0).iter().find(|t| t.kind == TokenKind::Number).unwrap();
        assert_eq!(one.col, 4);
    }

    #[test]
    fn token_is_small() {
        assert_eq!(std::mem::size_of::<Token>(), 12);
    }

    #[test]
    fn lang_detection() {
        assert_eq!(Lang::from_path("src/main.rs"), Lang::Rust);
        assert_eq!(Lang::from_path("a/b/App.TSX"), Lang::CLike);
        assert_eq!(Lang::from_path("Makefile"), Lang::Shell);
        assert_eq!(Lang::from_path("README"), Lang::Plain);
    }
}
