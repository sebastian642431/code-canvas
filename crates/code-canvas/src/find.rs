//! What the find bar looks for: a text in the lines of a file, or files and folders by name.

use std::cmp::Reverse;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Options {
    pub case: bool,
    pub word: bool,
    pub regex: bool,
}

pub struct Matcher {
    how: How,
    word: bool,
}

enum How {
    Text { wanted: Vec<char>, fold: bool },
    Regex(regex::Regex),
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn folded(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

fn alone(before: Option<char>, after: Option<char>) -> bool {
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

impl Matcher {
    /// Without Match Case a query in lowercase ignores case and one with a capital is exact
    /// (a regular expression just ignores case).
    pub fn new(query: &str, options: Options) -> Result<Matcher, String> {
        let how = if options.regex {
            let built = regex::RegexBuilder::new(query).case_insensitive(!options.case).build();
            How::Regex(built.map_err(|e| canvas_core::grep::short_error(&e.to_string()))?)
        } else {
            let fold = !options.case && !query.chars().any(char::is_uppercase);
            let wanted = query.chars().map(|c| if fold { folded(c) } else { c }).collect();
            How::Text { wanted, fold }
        };
        Ok(Matcher { how, word: options.word })
    }

    /// Every match in `line`, as (column, length) in characters, until `found` says stop.
    pub fn each(&self, line: &str, mut found: impl FnMut(usize, usize) -> bool) {
        match &self.how {
            How::Text { wanted, fold } => {
                let n = wanted.len();
                if n == 0 {
                    return;
                }
                let text: Vec<char> = line.chars().map(|c| if *fold { folded(c) } else { c }).collect();
                let mut from = 0;
                while from + n <= text.len() {
                    let fits = text[from..from + n] == wanted[..];
                    let whole = !self.word || alone(from.checked_sub(1).map(|i| text[i]), text.get(from + n).copied());
                    if !(fits && whole) {
                        from += 1;
                        continue;
                    }
                    if !found(from, n) {
                        return;
                    }
                    from += n;
                }
            }
            How::Regex(re) => {
                let (mut col, mut counted) = (0, 0);
                for m in re.find_iter(line).filter(|m| !m.is_empty()) {
                    if self.word && !alone(line[..m.start()].chars().next_back(), line[m.end()..].chars().next()) {
                        continue;
                    }
                    col += line[counted..m.start()].chars().count();
                    counted = m.start();
                    if !found(col, m.as_str().chars().count()) {
                        return;
                    }
                }
            }
        }
    }

    pub fn first(&self, text: &str) -> Option<(usize, usize)> {
        let mut hit = None;
        self.each(text, |col, len| {
            hit = Some((col, len));
            false
        });
        hit
    }

    /// What replaces the match at `col` (in characters) of `line`: with a regular expression `$1`
    /// and `${name}` take its groups; Preserve Case gives it the case of the text it replaces.
    pub fn replacement(&self, line: &str, col: usize, len: usize, with: &str, preserve: bool) -> String {
        let start = line.char_indices().nth(col).map_or(line.len(), |(at, _)| at);
        let mut out = String::new();
        match &self.how {
            How::Regex(re) => {
                match re.captures_at(line, start).filter(|caps| caps.get(0).is_some_and(|m| m.start() == start)) {
                    Some(caps) => caps.expand(&canvas_core::grep::expansion(with), &mut out),
                    None => out.push_str(with),
                }
            }
            How::Text { .. } => out.push_str(with),
        }
        if !preserve {
            return out;
        }
        let found: String = line[start..].chars().take(len).collect();
        preserve_case(&found, &out)
    }
}

pub fn preserve_case(found: &str, with: &str) -> String {
    let letters = || found.chars().filter(|c| c.is_alphabetic());
    if letters().next().is_none() {
        return with.to_string();
    }
    if letters().all(char::is_uppercase) {
        return with.to_uppercase();
    }
    if letters().all(char::is_lowercase) {
        return with.to_lowercase();
    }
    let mut chars = with.chars();
    match (found.chars().next().is_some_and(char::is_uppercase), chars.next()) {
        (true, Some(first)) => first.to_uppercase().chain(chars).collect(),
        _ => with.to_string(),
    }
}

/// The paths whose name or path holds `query`, best first: the whole name, then the start of the
/// name, then anywhere in the name, then anywhere in the path. Only when none holds it (and it is
/// plain text) the ones that hold its letters in order, the tightest first.
pub fn names(paths: &[&str], query: &str, options: Options) -> Result<Vec<usize>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let matcher = Matcher::new(query, options)?;
    let tier = |path: &str| {
        let name = path.rsplit('/').next().unwrap_or(path);
        match matcher.first(name) {
            Some((0, len)) if len == name.chars().count() => Some(4),
            Some((0, _)) => Some(3),
            Some(_) => Some(2),
            None => matcher.first(path).map(|_| 1),
        }
    };
    let mut ranked: Vec<(u8, usize, usize)> =
        paths.iter().enumerate().filter_map(|(i, path)| Some((tier(path)?, 0, i))).collect();
    if ranked.is_empty() && !options.regex && !options.word {
        let exact = options.case || query.chars().any(char::is_uppercase);
        ranked = paths.iter().enumerate().filter_map(|(i, path)| Some((0, spread(path, query, exact)?, i))).collect();
    }
    ranked.sort_by_key(|&(tier, span, i)| (Reverse(tier), span, paths[i].len(), paths[i]));
    Ok(ranked.into_iter().map(|(.., i)| i).collect())
}

/// How far apart the letters of `query` lie in `path`, taken in order; `None` when they are not all there.
fn spread(path: &str, query: &str, exact: bool) -> Option<usize> {
    let same = |c: char| if exact { c } else { folded(c) };
    let mut wanted = query.chars().filter(|c| !c.is_whitespace()).map(same).peekable();
    let (mut first, mut last) = (None, 0);
    for (at, c) in path.chars().map(same).enumerate() {
        if wanted.peek() == Some(&c) {
            wanted.next();
            first.get_or_insert(at);
            last = at;
        }
    }
    wanted.peek().is_none().then(|| last - first.unwrap_or(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(query: &str, options: Options, line: &str) -> Vec<(usize, usize)> {
        let matcher = Matcher::new(query, options).unwrap();
        let mut out = Vec::new();
        matcher.each(line, |col, len| {
            out.push((col, len));
            true
        });
        out
    }

    const PLAIN: Options = Options { case: false, word: false, regex: false };

    #[test]
    fn lowercase_ignores_case_and_a_capital_is_exact() {
        assert_eq!(hits("alpha", PLAIN, "Alpha alpha"), vec![(0, 5), (6, 5)]);
        assert_eq!(hits("Alpha", PLAIN, "Alpha alpha"), vec![(0, 5)]);
        assert_eq!(hits("alpha", Options { case: true, ..PLAIN }, "Alpha alpha"), vec![(6, 5)]);
    }

    #[test]
    fn plain_text_takes_every_character_literally() {
        assert_eq!(hits("a.b", PLAIN, "axb a.b"), vec![(4, 3)]);
        assert_eq!(hits("$1", PLAIN, "x $1"), vec![(2, 2)]);
        assert_eq!(hits("aa", PLAIN, "aaaa"), vec![(0, 2), (2, 2)]);
        assert!(hits("", PLAIN, "abc").is_empty());
    }

    #[test]
    fn whole_word_skips_matches_inside_words() {
        let word = Options { word: true, ..PLAIN };
        assert_eq!(hits("foo", word, "foofoo food foo foo_bar (foo)"), vec![(12, 3), (25, 3)]);
        let both = Options { word: true, regex: true, ..PLAIN };
        assert_eq!(hits("f.o", both, "food foo"), vec![(5, 3)]);
    }

    #[test]
    fn regex_columns_count_characters() {
        let re = Options { regex: true, ..PLAIN };
        assert_eq!(hits("a.b", re, "ñé axb"), vec![(3, 3)]);
        assert_eq!(hits("^start", re, "start start"), vec![(0, 5)]);
        assert_eq!(hits("x*", re, "ab"), vec![]);
        assert_eq!(hits("ALPHA", re, "alpha"), vec![(0, 5)]);
        assert_eq!(hits("ALPHA", Options { case: true, ..re }, "alpha"), vec![]);
        assert!(Matcher::new("(unclosed", re).is_err());
    }

    #[test]
    fn regex_replacements_take_groups() {
        let matcher = Matcher::new(r"(\w+)@(\w+)", Options { regex: true, ..PLAIN }).unwrap();
        assert_eq!(matcher.replacement("ñ me@home", 2, 7, "$2 of $1", false), "home of me");
        assert_eq!(matcher.replacement("me@home", 0, 7, "$1x\\t$2", false), "mex\thome", "as the Search panel does");
        let plain = Matcher::new("x", PLAIN).unwrap();
        assert_eq!(plain.replacement("x", 0, 1, "$1", false), "$1");
    }

    #[test]
    fn preserve_case_follows_the_text_replaced() {
        assert_eq!(preserve_case("FOO", "bar"), "BAR");
        assert_eq!(preserve_case("foo", "Bar"), "bar");
        assert_eq!(preserve_case("Foo", "bar"), "Bar");
        assert_eq!(preserve_case("fOO", "bar"), "bar");
        assert_eq!(preserve_case("123", "Bar"), "Bar");
        let matcher = Matcher::new("foo", PLAIN).unwrap();
        assert_eq!(matcher.replacement("a Foo", 2, 3, "bar", true), "Bar");
    }

    #[test]
    fn names_rank_the_name_before_the_path() {
        let paths = ["src/app.rs", "src/app", "src/application.rs", "app/main.rs", "src/my_app.rs", "docs/x.md"];
        let found = names(&paths, "app", PLAIN).unwrap();
        let order: Vec<&str> = found.iter().map(|&i| paths[i]).collect();
        assert_eq!(order, ["src/app", "src/app.rs", "src/application.rs", "src/my_app.rs", "app/main.rs"]);
        assert!(names(&paths, "  ", PLAIN).unwrap().is_empty());
    }

    #[test]
    fn names_fall_back_to_letters_in_order_only_when_nothing_holds_the_text() {
        let paths = ["src/main.rs", "lib/m/data/l.rs", "src/model.rs"];
        let found = names(&paths, "mdl", PLAIN).unwrap();
        assert_eq!(found.iter().map(|&i| paths[i]).collect::<Vec<_>>(), ["src/model.rs", "lib/m/data/l.rs"]);
        assert_eq!(names(&paths, "main", PLAIN).unwrap(), vec![0]);
        assert!(names(&paths, "mdl", Options { word: true, ..PLAIN }).unwrap().is_empty());
    }

    #[test]
    fn names_take_case_word_and_regex() {
        let paths = ["src/App.rs", "src/app.rs", "src/apps.rs"];
        assert_eq!(names(&paths, "App", PLAIN).unwrap(), vec![0]);
        assert_eq!(names(&paths, "app", Options { case: true, ..PLAIN }).unwrap(), vec![1, 2]);
        assert_eq!(names(&paths, "app", Options { word: true, ..PLAIN }).unwrap(), vec![0, 1]);
        assert_eq!(names(&paths, r"^apps?\.rs$", Options { regex: true, ..PLAIN }).unwrap(), vec![0, 1, 2]);
        assert!(names(&paths, "[", Options { regex: true, ..PLAIN }).is_err());
    }
}
