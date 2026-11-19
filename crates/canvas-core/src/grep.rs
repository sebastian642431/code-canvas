use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};

use crate::TAB_WIDTH;

const BEFORE: usize = 40;
const AFTER: usize = 160;
const FOUND: usize = 300;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
}

pub struct Matcher {
    re: Regex,
    word: bool,
    regex: bool,
}

impl Matcher {
    pub fn new(query: &Query) -> Result<Matcher, String> {
        if query.text.is_empty() {
            return Err("nothing to look for".to_string());
        }
        let pattern = if query.regex { query.text.clone() } else { regex::escape(&query.text) };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!query.case)
            .multi_line(true)
            .crlf(true)
            .build()
            .map_err(|e| short_error(&e.to_string()))?;
        Ok(Matcher { re, word: query.word, regex: query.regex })
    }

    pub fn may_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    /// Byte ranges of the matches in one line; empty matches are left out.
    pub fn find_in(&self, line: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at <= line.len() {
            let Some(m) = self.re.find_at(line, at) else { break };
            let (start, end) = (m.start(), m.end());
            if start == end || (self.word && !whole_word(line, start, end)) {
                at = line[start..].chars().next().map_or(line.len() + 1, |c| start + c.len_utf8());
                continue;
            }
            out.push((start, end));
            at = end;
        }
        out
    }

    /// What the match starting at `start` of `line` becomes: with a regular expression
    /// `$1`, `$&`, `\n` and `\t` in `with` are filled in.
    pub fn replacement(&self, line: &str, start: usize, with: &str) -> String {
        if !self.regex {
            return with.to_string();
        }
        let Some(caps) = self.re.captures_at(line, start) else { return with.to_string() };
        let mut out = String::new();
        caps.expand(&expansion(with), &mut out);
        out
    }

    pub fn replaced(&self, found: &str, with: &str) -> String {
        self.replacement(found, 0, with)
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn whole_word(line: &str, start: usize, end: usize) -> bool {
    let found = &line[start..end];
    let joined_before =
        line[..start].chars().next_back().is_some_and(is_word) && found.chars().next().is_some_and(is_word);
    let joined_after =
        line[end..].chars().next().is_some_and(is_word) && found.chars().next_back().is_some_and(is_word);
    !joined_before && !joined_after
}

/// A replacement as typed (JavaScript style, like VS Code) in the syntax of the regex crate.
fn expansion(with: &str) -> String {
    let mut out = String::with_capacity(with.len() + 8);
    let mut chars = with.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('$') => out.push_str("$$"),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            '$' => match chars.peek().copied() {
                Some('&') => {
                    chars.next();
                    out.push_str("${0}");
                }
                Some(d) if d.is_ascii_digit() => {
                    let mut group = String::new();
                    while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                        group.push(d);
                        chars.next();
                    }
                    out.push_str(&format!("${{{group}}}"));
                }
                Some('{') => out.push('$'),
                Some('$') => {
                    chars.next();
                    out.push_str("$$");
                }
                _ => out.push_str("$$"),
            },
            c => out.push(c),
        }
    }
    out
}

/// The line of an error of the regex crate that says what is wrong.
pub fn short_error(message: &str) -> String {
    let said = message.lines().rev().find_map(|line| line.trim().strip_prefix("error: "));
    match said {
        Some(said) => said.to_string(),
        None => message.lines().last().unwrap_or(message).trim().to_string(),
    }
}

/// The column a line reaches after `text`, from `col`, with tabs as the canvas shows them.
pub fn advance(col: usize, text: &str) -> usize {
    text.chars().fold(col, |col, c| if c == '\t' { col + TAB_WIDTH - col % TAB_WIDTH } else { col + 1 })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub line: u32,
    /// Which match of its line it is: with `line`, what a replacement looks for again.
    pub nth: u32,
    pub col: u32,
    pub len: u32,
    pub bytes: u32,
    pub before: String,
    pub found: String,
    pub after: String,
}

fn hit(line: &str, at: u32, nth: u32, start: usize, end: usize) -> Hit {
    let col = advance(0, &line[..start]);
    let len = advance(col, &line[start..end]) - col;
    let lead = line[..start].trim_start();
    let count = lead.chars().count();
    let before = match count > BEFORE {
        true => format!("…{}", lead.chars().skip(count - BEFORE).collect::<String>()),
        false => lead.to_string(),
    };
    Hit {
        line: at,
        nth,
        col: col as u32,
        len: len as u32,
        bytes: (end - start) as u32,
        before,
        found: line[start..end].chars().take(FOUND).collect(),
        after: line[end..].chars().take(AFTER).collect(),
    }
}

/// The matches in a text, at most `room` of them, in its first `lines` lines; whether some were left out.
pub fn hits_in(text: &str, lines: u32, matcher: &Matcher, room: usize) -> (Vec<Hit>, bool) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut hits = Vec::new();
    if !matcher.may_match(text) {
        return (hits, false);
    }
    for (i, raw) in text.split('\n').enumerate().take(lines as usize) {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        for (nth, (start, end)) in matcher.find_in(line).into_iter().enumerate() {
            if hits.len() >= room {
                return (hits, true);
            }
            hits.push(hit(line, i as u32, nth as u32, start, end));
        }
    }
    (hits, false)
}

/// The text with the matches of `hits` that are still there replaced; how many were.
pub fn replace_in(text: &str, matcher: &Matcher, with: &str, hits: &[Hit]) -> (String, usize) {
    let mut wanted: Vec<&Hit> = hits.iter().collect();
    wanted.sort_by_key(|hit| (hit.line, hit.nth));
    let marked = text.starts_with('\u{feff}');
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = String::with_capacity(text.len());
    if marked {
        out.push('\u{feff}');
    }
    let (mut next, mut done) = (0, 0);
    for (i, raw) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        while next < wanted.len() && (wanted[next].line as usize) < i {
            next += 1;
        }
        let from = next;
        while next < wanted.len() && wanted[next].line as usize == i {
            next += 1;
        }
        let here = &wanted[from..next];
        if here.is_empty() {
            out.push_str(raw);
            continue;
        }
        let (line, end_of_line) = match raw.strip_suffix('\r') {
            Some(line) => (line, "\r"),
            None => (raw, ""),
        };
        let mut kept = 0;
        for (nth, (start, end)) in matcher.find_in(line).into_iter().enumerate() {
            let Some(hit) = here.iter().find(|hit| hit.nth as usize == nth) else { continue };
            if hit.bytes as usize != end - start || !line[start..end].starts_with(hit.found.as_str()) {
                continue;
            }
            out.push_str(&line[kept..start]);
            out.push_str(&matcher.replacement(line, start, with));
            kept = end;
            done += 1;
        }
        out.push_str(&line[kept..]);
        out.push_str(end_of_line);
    }
    (out, done)
}

/// Files to include or exclude, as typed: glob patterns separated by commas (`src/**/*.ts, *.rs`).
/// A pattern without a slash matches at any depth, and a folder matches everything inside it.
#[derive(Clone, Debug, Default)]
pub struct PathFilter {
    include: Vec<Vec<String>>,
    exclude: Vec<Vec<String>>,
}

impl PathFilter {
    pub fn new(include: &str, exclude: &str) -> PathFilter {
        PathFilter { include: globs(include), exclude: globs(exclude) }
    }

    pub fn allows(&self, rel_path: &str) -> bool {
        if self.include.is_empty() && self.exclude.is_empty() {
            return true;
        }
        let path = rel_path.replace('\\', "/").to_lowercase();
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let hit = |globs: &[Vec<String>]| globs.iter().any(|glob| path_matches(glob, &parts));
        (self.include.is_empty() || hit(&self.include)) && !hit(&self.exclude)
    }
}

fn globs(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for pattern in split_top(text).into_iter().flat_map(expand) {
        let mut pattern = pattern.trim().replace('\\', "/").to_lowercase();
        if let Some(rest) = pattern.strip_prefix("./") {
            pattern = format!("/{rest}");
        }
        let pattern = pattern.trim_end_matches('/');
        if pattern.trim_start_matches('/').is_empty() {
            continue;
        }
        let anchored = pattern.contains('/');
        let mut parts: Vec<String> = Vec::new();
        if !anchored {
            parts.push("**".to_string());
        }
        parts.extend(pattern.split('/').filter(|part| !part.is_empty()).map(String::from));
        out.push(parts);
    }
    out
}

fn split_top(text: &str) -> Vec<&str> {
    let (mut out, mut depth, mut from) = (Vec::new(), 0usize, 0);
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&text[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[from..]);
    out
}

fn expand(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else { return vec![pattern.to_string()] };
    let mut depth = 0;
    let mut close = None;
    for (i, c) in pattern[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else { return vec![pattern.to_string()] };
    let (head, body, tail) = (&pattern[..open], &pattern[open + 1..close], &pattern[close + 1..]);
    split_top(body).into_iter().flat_map(|alt| expand(&format!("{head}{alt}{tail}"))).take(256).collect()
}

fn path_matches(glob: &[String], parts: &[&str]) -> bool {
    (1..=parts.len()).any(|n| parts_match(glob, &parts[..n]))
}

fn parts_match(glob: &[String], parts: &[&str]) -> bool {
    match glob.split_first() {
        None => parts.is_empty(),
        Some((first, rest)) if first == "**" => (0..=parts.len()).any(|skip| parts_match(rest, &parts[skip..])),
        Some((first, rest)) => parts.split_first().is_some_and(|(part, more)| {
            let (p, t): (Vec<char>, Vec<char>) = (first.chars().collect(), part.chars().collect());
            wild(&p, &t) && parts_match(rest, more)
        }),
    }
}

fn wild(p: &[char], t: &[char]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some('*') => {
            let rest = &p[p.iter().take_while(|c| **c == '*').count()..];
            (0..=t.len()).any(|skip| wild(rest, &t[skip..]))
        }
        Some('?') => !t.is_empty() && wild(&p[1..], &t[1..]),
        Some('[') => match class(p) {
            Some((len, ranges, negated)) => t.first().is_some_and(|c| {
                let inside = ranges.iter().any(|(a, b)| (*a..=*b).contains(c));
                inside != negated && wild(&p[len..], &t[1..])
            }),
            None => t.first() == Some(&'[') && wild(&p[1..], &t[1..]),
        },
        Some(c) => t.first() == Some(c) && wild(&p[1..], &t[1..]),
    }
}

type Class = (usize, Vec<(char, char)>, bool);

fn class(p: &[char]) -> Option<Class> {
    let mut i = 1;
    let negated = matches!(p.get(1), Some('!' | '^'));
    if negated {
        i += 1;
    }
    let mut ranges = Vec::new();
    while let Some(&c) = p.get(i) {
        if c == ']' && !ranges.is_empty() {
            return Some((i + 1, ranges, negated));
        }
        match (p.get(i + 1), p.get(i + 2)) {
            (Some('-'), Some(&end)) if end != ']' => {
                ranges.push((c, end));
                i += 3;
            }
            _ => {
                ranges.push((c, c));
                i += 1;
            }
        }
    }
    None
}

/// A file to look in: its text when the canvas does not show what is on disk
/// (unsaved changes, a commit), and how many of its lines the canvas has.
pub struct Source {
    pub rel: String,
    pub lines: u32,
    pub text: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FileHits {
    pub rel: String,
    pub hits: Vec<Hit>,
}

pub struct Found {
    pub files: Vec<FileHits>,
    pub truncated: bool,
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return None;
    }
    Some(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    })
}

/// Every match in the files, at most `max`, read in parallel; gives up early when `stop` is set.
pub fn search(
    root: &Path,
    sources: Vec<Source>,
    matcher: &Matcher,
    paths: &PathFilter,
    max: usize,
    stop: &AtomicBool,
) -> Found {
    let count = AtomicUsize::new(0);
    let cut = AtomicBool::new(false);
    let files: Vec<FileHits> = sources
        .into_par_iter()
        .filter(|source| paths.allows(&source.rel))
        .filter_map(|source| {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let room = max.saturating_sub(count.load(Ordering::Relaxed));
            if room == 0 {
                cut.store(true, Ordering::Relaxed);
                return None;
            }
            let text = match source.text {
                Some(text) => text,
                None => read_text(&root.join(&source.rel))?,
            };
            let (hits, more) = hits_in(&text, source.lines, matcher, room);
            if more {
                cut.store(true, Ordering::Relaxed);
            }
            count.fetch_add(hits.len(), Ordering::Relaxed);
            (!hits.is_empty()).then_some(FileHits { rel: source.rel, hits })
        })
        .collect();
    let mut kept = Vec::with_capacity(files.len());
    let mut room = max;
    for mut file in files {
        if room == 0 {
            cut.store(true, Ordering::Relaxed);
            break;
        }
        if file.hits.len() > room {
            file.hits.truncate(room);
            cut.store(true, Ordering::Relaxed);
        }
        room -= file.hits.len();
        kept.push(file);
    }
    Found { files: kept, truncated: cut.load(Ordering::Relaxed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(text: &str, case: bool, word: bool, regex: bool) -> Matcher {
        Matcher::new(&Query { text: text.to_string(), case, word, regex }).unwrap()
    }

    fn found<'a>(m: &Matcher, line: &'a str) -> Vec<&'a str> {
        m.find_in(line).into_iter().map(|(a, b)| &line[a..b]).collect()
    }

    #[test]
    fn plain_text_ignores_case_unless_asked() {
        let line = "Foo foo FOO";
        assert_eq!(found(&matcher("foo", false, false, false), line), ["Foo", "foo", "FOO"]);
        assert_eq!(found(&matcher("foo", true, false, false), line), ["foo"]);
    }

    #[test]
    fn plain_text_is_not_a_regular_expression() {
        assert_eq!(found(&matcher("a.b(", false, false, false), "axb( a.b("), ["a.b("]);
    }

    #[test]
    fn whole_words_only() {
        let m = matcher("step", false, true, false);
        assert_eq!(found(&m, "step steps step_1 (step) misstep"), ["step", "step"]);
        let call = matcher("(step", false, true, false);
        assert_eq!(found(&call, "f(step) f(steps)"), ["(step"]);
    }

    #[test]
    fn regular_expressions_and_empty_matches() {
        assert_eq!(found(&matcher(r"step_\d+", false, false, true), "step_1 step_x step_22"), ["step_1", "step_22"]);
        assert!(found(&matcher("x*", false, false, true), "abc").is_empty());
        assert_eq!(found(&matcher("^fn", false, false, true), "fn a() { fn b() }"), ["fn"]);
    }

    #[test]
    fn a_bad_regular_expression_says_why() {
        let err = Matcher::new(&Query { text: "(".into(), regex: true, ..Default::default() }).err().unwrap();
        assert_eq!(err, "unclosed group");
        assert_eq!(short_error("one line"), "one line");
    }

    #[test]
    fn replacement_fills_in_groups() {
        let m = matcher(r"(\w+)=(\d)", false, false, true);
        assert_eq!(m.replaced("a=1", "$2=$1"), "1=a");
        assert_eq!(m.replaced("a=1", "$1x"), "ax");
        assert_eq!(m.replaced("a=1", r"[$&]\n"), "[a=1]\n");
        assert_eq!(m.replaced("a=1", "$$"), "$");
        assert_eq!(matcher("a", false, false, false).replaced("a", "$1"), "$1");
    }

    #[test]
    fn hits_have_canvas_columns_and_a_preview() {
        let text = "\tlet x = step;\r\nstep step\n";
        let (hits, more) = hits_in(text, 100, &matcher("step", false, false, false), 100);
        assert!(!more);
        assert_eq!(hits.len(), 3);
        assert_eq!((hits[0].line, hits[0].col, hits[0].len, hits[0].nth), (0, 12, 4, 0));
        assert_eq!((hits[0].before.as_str(), hits[0].after.as_str()), ("let x = ", ";"));
        assert_eq!((hits[2].line, hits[2].col, hits[2].nth), (1, 5, 1));
        let (hits, more) = hits_in(text, 1, &matcher("step", false, false, false), 100);
        assert_eq!((hits.len(), more), (1, false));
        let (hits, more) = hits_in(text, 100, &matcher("step", false, false, false), 2);
        assert_eq!((hits.len(), more), (2, true));
    }

    #[test]
    fn long_text_before_a_match_is_cut() {
        let line = format!("{}step", "x".repeat(100));
        let (hits, _) = hits_in(&line, 1, &matcher("step", false, false, false), 10);
        assert!(hits[0].before.starts_with('…'));
        assert_eq!(hits[0].before.chars().count(), BEFORE + 1);
    }

    #[test]
    fn replacing_keeps_line_ends_and_skips_what_moved() {
        let m = matcher("step", false, false, false);
        let text = "\u{feff}step a\r\nb step step\nc";
        let (hits, _) = hits_in(text, 100, &m, 100);
        let (out, n) = replace_in(text, &m, "go", &hits);
        assert_eq!((out.as_str(), n), ("\u{feff}go a\r\nb go go\nc", 3));
        let (out, n) = replace_in(text, &m, "go", &hits[2..]);
        assert_eq!((out.as_str(), n), ("\u{feff}step a\r\nb step go\nc", 1));
        let (out, n) = replace_in("nothing here", &m, "go", &hits);
        assert_eq!((out.as_str(), n), ("nothing here", 0));
    }

    #[test]
    fn replacing_with_groups() {
        let m = matcher(r"(\w+)\(\)", false, false, true);
        let text = "a(); b();";
        let (hits, _) = hits_in(text, 1, &m, 10);
        assert_eq!(replace_in(text, &m, "$1(x)", &hits).0, "a(x); b(x);");
    }

    #[test]
    fn globs_like_vs_code() {
        let f = PathFilter::new("src/**/*.ts, *.rs", "");
        assert!(f.allows("src/a/b.ts"));
        assert!(f.allows("src/b.ts"));
        assert!(!f.allows("lib/b.ts"));
        assert!(f.allows("crates/x/main.rs"));
        let f = PathFilter::new("", "node_modules, *.{md,txt}");
        assert!(!f.allows("web/node_modules/x/index.js"));
        assert!(!f.allows("README.MD"));
        assert!(!f.allows("docs/notes.txt"));
        assert!(f.allows("src/main.rs"));
        let f = PathFilter::new("src/ui", "");
        assert!(f.allows("src/ui/panel.rs"));
        assert!(!f.allows("src/uix.rs"));
        let f = PathFilter::new("./main.rs, ?.c, [ab]*.h", "");
        assert!(f.allows("main.rs"));
        assert!(!f.allows("src/main.rs"));
        assert!(f.allows("x/y.c"));
        assert!(f.allows("b_thing.h"));
        assert!(!f.allows("c_thing.h"));
        assert!(PathFilter::new(" , ", "").allows("anything"));
    }
}
