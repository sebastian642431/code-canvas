use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Num(u64),
    Text(String),
}

#[derive(Clone, Debug, Eq)]
pub struct Version {
    pub nums: Vec<u64>,
    pub pre: Vec<Part>,
    pub post: u64,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.nums.len().max(other.nums.len());
        for i in 0..len {
            let (a, b) = (self.num(i), other.num(i));
            if a != b {
                return a.cmp(&b);
            }
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => self.pre.cmp(&other.pre).then(self.post.cmp(&other.post)),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

fn ident(text: &str) -> Option<Part> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some(match text.parse() {
        Ok(n) if text.chars().all(|c| c.is_ascii_digit()) => Part::Num(n),
        _ => Part::Text(text.to_ascii_lowercase()),
    })
}

fn digits(text: &str) -> (u64, &str) {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    (text[..end].parse().unwrap_or(0), &text[end..])
}

fn python_suffix(rest: &str) -> Option<(Vec<Part>, u64)> {
    let lower = rest.to_ascii_lowercase();
    let sep = |r: &str| r.trim_start_matches(['.', '_', '-']).to_string();
    let mut rest = sep(&lower);
    let mut pre = Vec::new();
    let mut post = 0;
    const TAGS: [(&str, &str); 8] = [
        ("alpha", "a"),
        ("beta", "b"),
        ("preview", "rc"),
        ("pre", "rc"),
        ("rc", "rc"),
        ("a", "a"),
        ("b", "b"),
        ("c", "rc"),
    ];
    if let Some((r, norm)) = TAGS.iter().find_map(|(tag, norm)| rest.strip_prefix(tag).map(|r| (r.to_string(), norm))) {
        let (n, r) = digits(&r);
        pre = vec![Part::Text(norm.to_string()), Part::Num(n)];
        rest = sep(r);
    }
    if let Some(r) = rest.strip_prefix("post").map(str::to_string) {
        let r = sep(&r);
        let (n, r) = digits(&r);
        post = n + 1;
        rest = sep(r);
    }
    if let Some(r) = rest.strip_prefix("dev").map(str::to_string) {
        let (n, r) = digits(&r);
        if pre.is_empty() && post == 0 {
            pre = vec![Part::Num(0), Part::Num(n)];
        }
        rest = r.to_string();
    }
    rest.is_empty().then_some((pre, post))
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let text = text.split('+').next()?;
        let end = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
        let (mut core, mut rest) = text.split_at(end);
        if let Some(trimmed) = core.strip_suffix('.') {
            (core, rest) = (trimmed, &text[end - 1..]);
        }
        let nums: Vec<u64> = core.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        let (pre, post) = match rest.strip_prefix('-') {
            _ if rest.is_empty() => (Vec::new(), 0),
            Some(pre) if !pre.is_empty() => match pre.split('.').map(ident).collect::<Option<Vec<_>>>() {
                Some(parts) => (parts, 0),
                None => python_suffix(rest)?,
            },
            _ => python_suffix(rest)?,
        };
        Some(Version { nums, pre, post })
    }

    pub fn num(&self, i: usize) -> u64 {
        self.nums.get(i).copied().unwrap_or(0)
    }

    pub fn is_pre(&self) -> bool {
        !self.pre.is_empty()
    }

    fn same_core(&self, other: &Version) -> bool {
        (0..3).all(|i| self.num(i) == other.num(i))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flavor {
    Npm,
    Cargo,
    Composer,
    Exact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Debug, PartialEq)]
struct Comparator {
    op: Op,
    v: Version,
}

impl Comparator {
    fn allows(&self, v: &Version) -> bool {
        let order = v.cmp(&self.v);
        match self.op {
            Op::Lt => order == Ordering::Less,
            Op::Le => order != Ordering::Greater,
            Op::Gt => order == Ordering::Greater,
            Op::Ge => order != Ordering::Less,
            Op::Eq => order == Ordering::Equal,
            Op::Ne => order != Ordering::Equal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Requirement {
    sets: Vec<Vec<Comparator>>,
}

struct Partial {
    nums: Vec<u64>,
    wild: bool,
    v: Version,
}

fn is_wild(part: &str) -> bool {
    matches!(part, "*" | "x" | "X")
}

fn partial(text: &str) -> Option<Partial> {
    let text = text.trim();
    let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
    let empty = || Version { nums: Vec::new(), pre: Vec::new(), post: 0 };
    if text.is_empty() || is_wild(text) {
        return Some(Partial { nums: Vec::new(), wild: true, v: empty() });
    }
    let parts: Vec<&str> = text.split('.').collect();
    if let Some(at) = parts.iter().position(|p| is_wild(p)) {
        let nums: Vec<u64> = parts[..at].iter().map(|p| p.parse().ok()).collect::<Option<_>>()?;
        return Some(Partial { v: Version { nums: nums.clone(), pre: Vec::new(), post: 0 }, nums, wild: true });
    }
    let v = Version::parse(text)?;
    Some(Partial { nums: v.nums.clone(), wild: false, v })
}

fn padded(nums: &[u64]) -> Vec<u64> {
    let mut out = nums.to_vec();
    while out.len() < 3 {
        out.push(0);
    }
    out
}

fn upper(nums: &[u64], keep: usize) -> Version {
    let mut out: Vec<u64> = nums[..keep].to_vec();
    if let Some(last) = out.last_mut() {
        *last += 1;
    }
    Version { nums: padded(&out), pre: Vec::new(), post: 0 }
}

const OPS: [&str; 12] = ["===", "==", "!=", "~=", "~>", ">=", "<=", ">", "<", "=", "^", "~"];

fn split_op(token: &str) -> (&'static str, &str) {
    OPS.iter().find_map(|op| token.strip_prefix(op).map(|rest| (*op, rest.trim()))).unwrap_or(("", token.trim()))
}

fn term(token: &str, flavor: Flavor) -> Option<Vec<Comparator>> {
    let (op, rest) = split_op(token);
    let p = partial(rest)?;
    let k = p.nums.len();
    let lo = Version { nums: padded(&p.v.nums), pre: p.v.pre.clone(), post: p.v.post };
    let at = |op: Op, v: Version| Comparator { op, v };
    let range = |keep: usize| match keep {
        0 => Vec::new(),
        keep => vec![at(Op::Ge, lo.clone()), at(Op::Lt, upper(&p.nums, keep))],
    };
    let caret = || range(p.nums.iter().position(|n| *n != 0).map_or(k, |i| (i + 1).min(k)));
    let loose = matches!(flavor, Flavor::Npm | Flavor::Cargo | Flavor::Composer) && (p.wild || k < 3);
    Some(match op {
        "^" => caret(),
        "~>" => range(if k <= 2 { k.min(1) } else { k - 1 }),
        "~" if flavor == Flavor::Composer => range(if k <= 2 { k.min(1) } else { k - 1 }),
        "~" => range(k.min(2)),
        "~=" if k >= 2 => range(k - 1),
        "~=" => return None,
        ">=" => vec![at(Op::Ge, lo)],
        ">" if k == 0 => vec![at(Op::Lt, Version { nums: vec![0], pre: Vec::new(), post: 0 })],
        ">" if loose => vec![at(Op::Ge, upper(&p.nums, k))],
        ">" => vec![at(Op::Gt, lo)],
        "<" => vec![at(Op::Lt, lo)],
        "<=" if loose && k > 0 => vec![at(Op::Lt, upper(&p.nums, k))],
        "<=" => vec![at(Op::Le, lo)],
        "!=" if p.wild => Vec::new(),
        "!=" => vec![at(Op::Ne, lo)],
        "" if flavor == Flavor::Cargo => caret(),
        _ if p.wild => range(k),
        _ if k < 3 && matches!(flavor, Flavor::Npm | Flavor::Cargo) => range(k),
        _ => vec![at(Op::Eq, lo)],
    })
}

fn tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut pending = String::new();
    for word in text.split([',', ' ', '\t']).filter(|w| !w.is_empty()) {
        if word.chars().all(|c| "<>=~^!".contains(c)) {
            pending.push_str(word);
            continue;
        }
        out.push(format!("{}{word}", std::mem::take(&mut pending)));
    }
    out
}

fn interval(text: &str) -> Option<Requirement> {
    let (open, close) = (text.chars().next()?, text.chars().last()?);
    if !"])".contains(close) || text.len() < 2 {
        return None;
    }
    let inner = &text[1..text.len() - 1];
    let version = |t: &str| Version::parse(t.trim());
    let set = match inner.split_once(',') {
        None if open == '[' && close == ']' => vec![Comparator { op: Op::Eq, v: version(inner)? }],
        None => return None,
        Some((a, b)) => {
            let mut set = Vec::new();
            if !a.trim().is_empty() {
                set.push(Comparator { op: if open == '[' { Op::Ge } else { Op::Gt }, v: version(a)? });
            }
            if !b.trim().is_empty() {
                set.push(Comparator { op: if close == ']' { Op::Le } else { Op::Lt }, v: version(b)? });
            }
            set
        }
    };
    Some(Requirement { sets: vec![set] })
}

impl Requirement {
    pub fn parse(text: &str, flavor: Flavor) -> Option<Requirement> {
        let text = text.trim();
        let text = if flavor == Flavor::Composer { text.split('@').next().unwrap_or("").trim() } else { text };
        if text.starts_with(['[', '(']) {
            return interval(text);
        }
        let mut sets = Vec::new();
        let alternatives: Vec<&str> = match flavor {
            Flavor::Composer => text.split("||").flat_map(|a| a.split('|')).collect(),
            _ => text.split("||").collect(),
        };
        for alternative in alternatives {
            let alternative = alternative.trim();
            if let Some((from, to)) = alternative.split_once(" - ") {
                let (from, to) = (partial(from)?, partial(to)?);
                let lo = Version { nums: padded(&from.v.nums), pre: from.v.pre.clone(), post: from.v.post };
                let mut set = vec![Comparator { op: Op::Ge, v: lo }];
                match (to.wild || to.nums.len() < 3, to.nums.len()) {
                    (_, 0) => {}
                    (true, k) => set.push(Comparator { op: Op::Lt, v: upper(&to.nums, k) }),
                    (false, _) => set.push(Comparator { op: Op::Le, v: to.v.clone() }),
                }
                sets.push(set);
                continue;
            }
            let mut set = Vec::new();
            for token in tokens(alternative) {
                set.extend(term(&token, flavor)?);
            }
            sets.push(set);
        }
        Some(Requirement { sets })
    }

    pub fn matches(&self, v: &Version) -> bool {
        self.sets.iter().any(|set| {
            set.iter().all(|c| c.allows(v)) && (!v.is_pre() || set.iter().any(|c| c.v.is_pre() && c.v.same_core(v)))
        })
    }

    pub fn mentions_pre(&self) -> bool {
        self.sets.iter().flatten().any(|c| c.v.is_pre())
    }

    pub fn floor(&self) -> Option<Version> {
        let lows = self.sets.iter().flatten().filter(|c| matches!(c.op, Op::Ge | Op::Gt | Op::Eq));
        lows.map(|c| c.v.clone()).min()
    }
}

const REWRITABLE: [&str; 9] = ["", "^", "~", "~=", "~>", "=", "==", "===", ">="];

pub fn rewrite(text: &str, to: &str) -> Option<String> {
    let (op, rest) = split_op(text);
    let op_text = &text[..text.len() - text.trim_start_matches(|c: char| "<>=~^!".contains(c)).len()];
    let spacing = &text[op_text.len()..text.len() - rest.len()];
    if !REWRITABLE.contains(&op) || rest.is_empty() || rest.contains(|c: char| c.is_whitespace() || ",|<>".contains(c))
    {
        return None;
    }
    let v_prefix = if rest.starts_with(['v', 'V']) { &rest[..1] } else { "" };
    let body = &rest[v_prefix.len()..];
    let target = Version::parse(to)?;
    let to_body = to.trim().trim_start_matches(['v', 'V']);
    let parts: Vec<&str> = body.split('.').collect();
    let written = if let Some(at) = parts.iter().position(|p| is_wild(p)) {
        if at == 0 || target.is_pre() {
            return None;
        }
        let mut out: Vec<String> = (0..at).map(|i| target.num(i).to_string()).collect();
        out.extend(parts[at..].iter().map(|p| p.to_string()));
        out.join(".")
    } else {
        let current = Version::parse(body)?;
        let k = current.nums.len();
        let whole = target.is_pre() || target.post > 0 || current.is_pre() || current.post > 0 || to_body.contains('+');
        if whole || k > target.nums.len() {
            to_body.to_string()
        } else {
            (0..k.max(1)).map(|i| target.num(i).to_string()).collect::<Vec<_>>().join(".")
        }
    };
    let out = format!("{op_text}{spacing}{v_prefix}{written}");
    (out != text).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text} is a version"))
    }

    #[test]
    fn versions_parse_and_order() {
        let ordered = [
            "0.9.0",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1",
            "1.10.0",
            "2.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(v("1.2"), v("1.2.0"));
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert_eq!(v("1.2.3+build.5"), v("1.2.3"));
        assert_eq!(v("v2.0.0+incompatible"), v("2.0.0"));
        assert_eq!(v("13.0.3.0"), v("13.0.3"));
        assert!(v("1.0.0-beta").is_pre());
        assert!(!v("1.0.0").is_pre());
        for bad in ["", "abc", "1..2", "latest", "1.0.Final", "workspace:*"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn python_versions_order_like_pep_440() {
        let ordered = ["1.0.dev1", "1.0a1", "1.0a2", "1.0b1", "1.0rc1", "1.0", "1.0.post1", "1.0.1", "1.1.dev0"];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert!(v("2.0rc1").is_pre());
        assert!(v("2.0.dev3").is_pre());
        assert!(!v("2.0.post2").is_pre());
        assert_eq!(v("1.0.0alpha3"), v("1.0a3"));
    }

    fn satisfied(req: &str, flavor: Flavor, version: &str) -> bool {
        Requirement::parse(req, flavor).unwrap_or_else(|| panic!("{req} parses")).matches(&v(version))
    }

    #[test]
    fn npm_ranges() {
        use Flavor::Npm;
        for (req, yes, no) in [
            ("^1.2.3", &["1.2.3", "1.9.0", "1.2.4"][..], &["2.0.0", "1.2.2", "2.0.0-rc.1", "1.3.0-beta"][..]),
            ("^0.2.3", &["0.2.3", "0.2.9"], &["0.3.0", "0.2.2"]),
            ("^0.0.3", &["0.0.3"], &["0.0.4"]),
            ("^0.0", &["0.0.9"], &["0.1.0"]),
            ("^1.2", &["1.2.0", "1.9.9"], &["2.0.0", "1.1.9"]),
            ("^1.x", &["1.0.0", "1.9.0"], &["2.0.0"]),
            ("~1.2.3", &["1.2.3", "1.2.9"], &["1.3.0"]),
            ("~1.2", &["1.2.0", "1.2.9"], &["1.3.0"]),
            ("~1", &["1.0.0", "1.9.0"], &["2.0.0"]),
            ("1.2.x", &["1.2.0", "1.2.7"], &["1.3.0"]),
            ("1.2.*", &["1.2.7"], &["1.3.0"]),
            ("1.2", &["1.2.0", "1.2.7"], &["1.3.0"]),
            ("1", &["1.0.0", "1.99.0"], &["2.0.0"]),
            ("*", &["0.0.1", "99.0.0"], &["1.0.0-beta"]),
            ("", &["3.0.0"], &[]),
            ("x", &["3.0.0"], &[]),
            ("1.2.3", &["1.2.3"], &["1.2.4"]),
            ("=1.2.3", &["1.2.3"], &["1.2.4"]),
            ("v1.2.3", &["1.2.3"], &["1.2.4"]),
            (">=1.2.3", &["1.2.3", "5.0.0"], &["1.2.2"]),
            (">1.2.3", &["1.2.4"], &["1.2.3"]),
            (">1.2", &["1.3.0"], &["1.2.9"]),
            ("<1.2.3", &["1.2.2"], &["1.2.3"]),
            ("<=1.2", &["1.2.9"], &["1.3.0"]),
            (">=1.2.3 <2", &["1.5.0"], &["2.0.0", "1.2.0"]),
            (">= 1.2.3 < 2.0.0", &["1.5.0"], &["2.0.0"]),
            ("1.2.3 - 2.3.4", &["1.2.3", "2.3.4"], &["2.3.5", "1.2.2"]),
            ("1.2 - 2.3", &["1.2.0", "2.3.9"], &["2.4.0"]),
            ("^1.2.3 || ^2.0.0", &["1.5.0", "2.5.0"], &["3.0.0", "1.0.0"]),
            ("<1.0.0 || >=3", &["0.5.0", "3.1.0"], &["2.0.0"]),
            ("^1.2.3-beta.2", &["1.2.3-beta.4", "1.2.3", "1.5.0"], &["1.2.3-beta.1", "1.2.4-beta.1", "2.0.0"]),
            ("~1.2.3-rc.1", &["1.2.3-rc.2", "1.2.5"], &["1.3.0"]),
        ] {
            for version in yes {
                assert!(satisfied(req, Npm, version), "{version} satisfies {req}");
            }
            for version in no {
                assert!(!satisfied(req, Npm, version), "{version} does not satisfy {req}");
            }
        }
    }

    #[test]
    fn cargo_requirements() {
        use Flavor::Cargo;
        for (req, yes, no) in [
            ("1.2.3", &["1.2.3", "1.9.0"][..], &["2.0.0", "1.2.2"][..]),
            ("0.4", &["0.4.0", "0.4.22"], &["0.5.0"]),
            ("0.0.3", &["0.0.3"], &["0.0.4"]),
            ("1", &["1.0.228"], &["2.0.0"]),
            ("=1.4.0", &["1.4.0"], &["1.4.2"]),
            ("=1.4", &["1.4.2"], &["1.5.0"]),
            ("~1.4", &["1.4.9"], &["1.5.0"]),
            (">=2, <3", &["2.9.0"], &["3.1.0"]),
            ("*", &["7.0.0"], &[]),
            ("1.*", &["1.7.0"], &["2.0.0"]),
            ("1.0.0-rc.1", &["1.0.0-rc.2", "1.0.0", "1.5.0"], &["2.0.0"]),
        ] {
            for version in yes {
                assert!(satisfied(req, Cargo, version), "{version} satisfies {req}");
            }
            for version in no {
                assert!(!satisfied(req, Cargo, version), "{version} does not satisfy {req}");
            }
        }
    }

    #[test]
    fn python_composer_and_nuget_requirements() {
        for (req, flavor, yes, no) in [
            ("==2.28.1", Flavor::Exact, &["2.28.1"][..], &["2.28.2"][..]),
            ("==2.28", Flavor::Exact, &["2.28.0"], &["2.28.1"]),
            ("==2.*", Flavor::Exact, &["2.31.0"], &["3.0.0"]),
            ("~=1.4.2", Flavor::Exact, &["1.4.2", "1.4.9"], &["1.5.0"]),
            ("~=2.2", Flavor::Exact, &["2.2.0", "2.9.0"], &["3.0.0"]),
            (">=2.28,<3", Flavor::Exact, &["2.31.0"], &["3.0.0", "2.27.0"]),
            (">=1.0,!=1.5.0", Flavor::Exact, &["1.4.0", "1.6.0"], &["1.5.0"]),
            (">=2.0", Flavor::Exact, &["2.0.0"], &["2.1.0rc1"]),
            ("^1.2", Flavor::Exact, &["1.9.0"], &["2.0.0"]),
            ("1.2.3", Flavor::Exact, &["1.2.3"], &["1.2.4"]),
            ("~1.2", Flavor::Composer, &["1.2.0", "1.9.0"], &["2.0.0"]),
            ("~1.2.3", Flavor::Composer, &["1.2.9"], &["1.3.0"]),
            ("^7.4 | ^8.0", Flavor::Composer, &["7.4.1", "8.2.0"], &["9.0.0", "7.3.0"]),
            ("^7.4 || ^8.0", Flavor::Composer, &["8.2.0"], &["9.0.0"]),
            ("1.0.*", Flavor::Composer, &["1.0.5"], &["1.1.0"]),
            ("v2.1.0", Flavor::Composer, &["2.1.0"], &["2.1.1"]),
            ("^2.0@dev", Flavor::Composer, &["2.3.0"], &["3.0.0"]),
            (">=1.0 <2.0", Flavor::Composer, &["1.5.0"], &["2.0.0"]),
            ("13.0.1", Flavor::Exact, &["13.0.1"], &["13.0.3"]),
            ("[1.0,2.0)", Flavor::Exact, &["1.0.0", "1.9.9"], &["2.0.0", "0.9.0"]),
            ("(1.0,]", Flavor::Exact, &["1.0.1", "99.0.0"], &["1.0.0"]),
            ("[1.2.3]", Flavor::Exact, &["1.2.3"], &["1.2.4"]),
            ("(,2.0]", Flavor::Exact, &["0.1.0", "2.0.0"], &["2.0.1"]),
            ("v1.9.1", Flavor::Exact, &["1.9.1"], &["1.9.2"]),
        ] {
            for version in yes {
                assert!(satisfied(req, flavor, version), "{version} satisfies {req} ({flavor:?})");
            }
            for version in no {
                assert!(!satisfied(req, flavor, version), "{version} does not satisfy {req} ({flavor:?})");
            }
        }
    }

    #[test]
    fn requirements_that_do_not_parse() {
        for (req, flavor) in [
            ("latest", Flavor::Npm),
            ("workspace:*", Flavor::Npm),
            ("~=1", Flavor::Exact),
            ("[1.0", Flavor::Exact),
            ("^abc", Flavor::Cargo),
        ] {
            assert_eq!(Requirement::parse(req, flavor), None, "{req}");
        }
    }

    #[test]
    fn the_floor_of_a_requirement() {
        for (req, flavor, floor) in [
            ("^1.2.3", Flavor::Npm, Some("1.2.3")),
            ("1.2", Flavor::Npm, Some("1.2.0")),
            (">=2, <3", Flavor::Cargo, Some("2.0.0")),
            ("<3", Flavor::Npm, None),
            ("*", Flavor::Npm, None),
            ("^1.0.0 || ^2.0.0", Flavor::Npm, Some("1.0.0")),
            ("==4.2.1", Flavor::Exact, Some("4.2.1")),
        ] {
            let found = Requirement::parse(req, flavor).and_then(|r| r.floor());
            assert_eq!(found, floor.map(v), "{req}");
        }
    }

    #[test]
    fn rewriting_keeps_the_operator_the_precision_and_the_quotes_outside() {
        for (text, to, expected) in [
            ("^18.2.0", "19.1.0", Some("^19.1.0")),
            ("~1.4.2", "1.4.9", Some("~1.4.9")),
            ("~1.4", "1.6.2", Some("~1.6")),
            (">=2.0.0", "3.1.0", Some(">=3.1.0")),
            ("=1.4.0", "1.5.0", Some("=1.5.0")),
            ("5.0.0", "5.4.1", Some("5.4.1")),
            ("1", "2.0.3", Some("2")),
            ("1", "1.0.228", None),
            ("0.4", "0.5.1", Some("0.5")),
            ("1.x", "2.3.0", Some("2.x")),
            ("1.2.*", "1.3.4", Some("1.3.*")),
            ("*", "2.0.0", None),
            ("==2.28.1", "2.31.0", Some("==2.31.0")),
            ("~=1.4", "1.6.0", Some("~=1.6")),
            ("v1.9.1", "v1.10.0", Some("v1.10.0")),
            ("v1.2.3", "v2.0.0+incompatible", Some("v2.0.0+incompatible")),
            ("^1.2.3", "2.0.0-rc.1", Some("^2.0.0-rc.1")),
            ("^2.0.0-beta.1", "2.0.0-beta.3", Some("^2.0.0-beta.3")),
            ("^2.0.0-beta.1", "2.0.0", Some("^2.0.0")),
            ("13.0.1", "13.0.3", Some("13.0.3")),
            ("1.0.0.0", "1.2.0", Some("1.2.0")),
            (">=1.2 <2", "2.1.0", None),
            (">=1.0,<2", "2.1.0", None),
            ("^1 || ^2", "3.0.0", None),
            ("<2.0.0", "3.0.0", None),
            ("[1.0,2.0)", "3.0.0", None),
            ("^ 1.2.0", "1.3.0", Some("^ 1.3.0")),
            ("^18.2.0", "18.2.0", None),
        ] {
            assert_eq!(rewrite(text, to).as_deref(), expected, "{text} to {to}");
        }
    }
}
