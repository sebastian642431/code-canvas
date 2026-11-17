//! Newer versions of a project's dependencies, shown next to each one.
//!
//! `package.json` and `Cargo.toml` files are read for their dependencies;
//! the latest version of each is asked for on background threads (npm's
//! registry, the crates.io index) and comes back as a short text for the end
//! of its line: `✓ latest`, or `↑ 2.1.0` when there is something newer.
//!
//! Nothing but the names of the dependencies leaves the machine, and only
//! when this is switched on (`prefs.rs`). Answers are kept for an hour.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Who knows the versions of a package.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Registry {
    Npm,
    Crates,
}

/// A dependency as a manifest declares it.
#[derive(Clone, Debug, PartialEq)]
pub struct Dependency {
    /// 0-based line of the manifest it is on.
    pub line: u32,
    pub name: String,
    /// What the manifest asks for: `^1.2.0`, `1`, `>=2, <3`.
    pub wanted: String,
    pub registry: Registry,
}

/// How what a manifest asks for compares with the latest version.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    /// It asks for the latest.
    Latest,
    /// Something newer exists that it already accepts (same major).
    Newer,
    /// Something newer exists that it does not accept: an upgrade to do.
    Major,
}

/// The dependencies of a manifest called `file_name`, with the line each is on.
pub fn dependencies(file_name: &str, text: &str) -> Vec<Dependency> {
    match file_name {
        "package.json" => package_json(text),
        "Cargo.toml" => cargo_toml(text),
        _ => Vec::new(),
    }
}

/// `"name": "value"` on a line, with or without a trailing comma.
fn json_pair(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix('"')?;
    let (name, rest) = rest.split_once('"')?;
    let value = rest.trim_start().strip_prefix(':')?.trim().trim_end_matches(',').trim();
    Some((name, value.strip_prefix('"')?.strip_suffix('"')?))
}

fn package_json(text: &str) -> Vec<Dependency> {
    const SECTIONS: [&str; 4] = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"];
    let mut out = Vec::new();
    let mut inside = false;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !inside {
            // `"dependencies": {`
            let opens = trimmed.strip_prefix('"').and_then(|r| r.split_once('"'));
            inside = opens.is_some_and(|(name, rest)| SECTIONS.contains(&name) && rest.trim_end().ends_with('{'));
            continue;
        }
        if trimmed.starts_with('}') {
            inside = false;
            continue;
        }
        let Some((name, wanted)) = json_pair(line) else { continue };
        // Only what a registry can answer for: not files, links, git or tags.
        let from_registry = wanted.chars().next().is_some_and(|c| c.is_ascii_digit() || "^~>=<v*".contains(c));
        if from_registry && !wanted.contains(':') && !wanted.contains('/') {
            out.push(Dependency {
                line: i as u32,
                name: name.to_string(),
                wanted: wanted.to_string(),
                registry: Registry::Npm,
            });
        }
    }
    out
}

/// The value of `key = "value"` inside an inline table: `{ version = "1", features = [...] }`.
fn inline_value<'a>(table: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = table;
    while let Some(at) = rest.find(key) {
        let before_ok = rest[..at].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
        let after = rest[at + key.len()..].trim_start();
        if before_ok {
            if let Some(value) = after.strip_prefix('=') {
                let value = value.trim_start().strip_prefix('"')?;
                return value.split_once('"').map(|(v, _)| v);
            }
        }
        rest = &rest[at + key.len()..];
    }
    None
}

fn cargo_toml(text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    // In a `[...dependencies]` table every line is one; in a
    // `[dependencies.name]` table the `version` line is that crate's.
    let (mut listing, mut single): (bool, Option<String>) = (false, None);
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        if let Some(header) = trimmed.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            let header = header.trim_matches(|c| c == '[' || c == ']');
            listing = header.ends_with("dependencies");
            single = header.rsplit_once("dependencies.").map(|(_, name)| name.trim_matches('"').to_string());
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else { continue };
        let (key, value) = (key.trim().trim_matches('"'), value.trim());
        let found = if listing && !key.contains('.') {
            let plain = value.strip_prefix('"').and_then(|v| v.split_once('"')).map(|(v, _)| v);
            // `name = "1"`, or `name = { version = "1", ... }`. A table with a
            // `package` renames the crate: that is the name to ask for.
            let wanted = plain.or_else(|| inline_value(value, "version"));
            let name = inline_value(value, "package").unwrap_or(key);
            let elsewhere = inline_value(value, "path").is_some() || inline_value(value, "git").is_some();
            wanted.filter(|_| !elsewhere).map(|w| (name.to_string(), w))
        } else if let (Some(name), "version") = (&single, key) {
            value.strip_prefix('"').and_then(|v| v.split_once('"')).map(|(v, _)| (name.clone(), v))
        } else {
            None
        };
        if let Some((name, wanted)) = found {
            out.push(Dependency { line: i as u32, name, wanted: wanted.to_string(), registry: Registry::Crates });
        }
    }
    out
}

/// The numbers of a version, as many as are written: `1.2.3-beta` → `[1, 2, 3]`.
fn numbers(version: &str) -> Vec<u64> {
    let core = version.trim().trim_start_matches(['^', '~', '=', '>', '<', 'v', ' ']);
    let core = core.split(['-', '+', ' ', ',']).next().unwrap_or("");
    core.split('.').map_while(|part| part.parse().ok()).collect()
}

/// Is `version` a pre-release (`2.0.0-rc.1`)? Those are not "the latest".
fn pre_release(version: &str) -> bool {
    version.split('+').next().unwrap_or("").contains('-')
}

/// Compare what a manifest asks for with the latest version.
pub fn standing(wanted: &str, latest: &str) -> Standing {
    let (w, l) = (numbers(wanted), numbers(latest));
    if w.is_empty() || l.is_empty() || wanted.trim() == "*" {
        return Standing::Latest; // nothing to compare: do not cry wolf
    }
    // As far as it is written: `1.2` is `1.2.x`.
    let same = w.iter().zip(&l).all(|(a, b)| a == b);
    let older = w.iter().zip(&l).find(|(a, b)| a != b).is_some_and(|(a, b)| a < b);
    if same || !older {
        return Standing::Latest;
    }
    // What a caret range accepts: the same first number that is not zero.
    let lead = |v: &[u64]| v.iter().position(|n| *n != 0).map(|i| (i, v[i]));
    let accepted = match (lead(&w), lead(&l)) {
        (Some(a), Some(b)) => a == b && w.len() > a.0,
        _ => false,
    };
    // `~1.2` and exact versions do not take a new minor by themselves.
    let pinned = wanted.trim_start().starts_with(['~', '=']) || wanted.contains('<');
    if accepted && !pinned {
        Standing::Newer
    } else {
        Standing::Major
    }
}

/// The highest version that is not a pre-release.
fn highest<'a>(versions: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    versions.filter(|v| !pre_release(v)).max_by(|a, b| numbers(a).cmp(&numbers(b)))
}

/// Where the crates.io index keeps a crate: `se/rd/serde`, `3/s/syn`.
fn crates_index_path(name: &str) -> String {
    let name = name.to_lowercase();
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

/// The latest version in what the crates.io index says about a crate: one
/// JSON object per line, one per version, some of them withdrawn.
fn latest_in_index(body: &str) -> Option<String> {
    let versions: Vec<String> = body
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("yanked").and_then(serde_json::Value::as_bool) != Some(true))
        .filter_map(|v| v.get("vers").and_then(serde_json::Value::as_str).map(str::to_string))
        .collect();
    highest(versions.iter().map(String::as_str)).map(str::to_string)
}

/// Ask the registry. Blocks: runs on a worker thread.
fn fetch(registry: Registry, name: &str) -> Result<String, String> {
    const AGENT: &str = "code-canvas (https://github.com/sebastian642431/code-canvas)";
    let get = |url: &str| -> Result<String, String> {
        let agent: ureq::Agent =
            ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
        let mut response = agent.get(url).header("User-Agent", AGENT).call().map_err(|e| e.to_string())?;
        response.body_mut().read_to_string().map_err(|e| e.to_string())
    };
    match registry {
        Registry::Npm => {
            // A scope's slash is part of the name, not of the path.
            let body = get(&format!("https://registry.npmjs.org/{}/latest", name.replace('/', "%2f")))?;
            let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
            json.get("version").and_then(serde_json::Value::as_str).map(str::to_string).ok_or("no version".to_string())
        }
        Registry::Crates => {
            let body = get(&format!("https://index.crates.io/{}", crates_index_path(name)))?;
            latest_in_index(&body).ok_or("no version".to_string())
        }
    }
}

/// What to write after a dependency's line.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub line: u32,
    pub text: String,
    pub standing: Standing,
}

/// Asks for latest versions in the background and remembers the answers.
pub struct Checker {
    known: HashMap<(Registry, String), (Option<String>, Instant)>,
    asking: HashSet<(Registry, String)>,
    tx: Sender<(Registry, String, Option<String>)>,
    rx: Receiver<(Registry, String, Option<String>)>,
    /// Names waiting for a free worker.
    queue: Vec<(Registry, String)>,
    waker: fn(),
}

/// How many registries are asked at once, and how long an answer is good for.
const WORKERS: usize = 4;
const FRESH: Duration = Duration::from_secs(3600);

impl Checker {
    pub fn new(waker: fn()) -> Self {
        let (tx, rx) = mpsc::channel();
        Checker { known: HashMap::new(), asking: HashSet::new(), tx, rx, queue: Vec::new(), waker }
    }

    /// Make sure the latest version of each of these is known or on its way.
    pub fn want(&mut self, dependencies: &[Dependency]) {
        for d in dependencies {
            let key = (d.registry, d.name.clone());
            let fresh = self.known.get(&key).is_some_and(|(_, at)| at.elapsed() < FRESH);
            if !fresh && !self.asking.contains(&key) && !self.queue.contains(&key) {
                self.queue.push(key);
            }
        }
        self.start();
    }

    fn start(&mut self) {
        while self.asking.len() < WORKERS {
            let Some((registry, name)) = self.queue.pop() else { break };
            self.asking.insert((registry, name.clone()));
            let (tx, waker) = (self.tx.clone(), self.waker);
            std::thread::spawn(move || {
                let latest = match fetch(registry, &name) {
                    Ok(version) => Some(version),
                    Err(e) => {
                        log::debug!("latest version of {name}: {e}");
                        None
                    }
                };
                let _ = tx.send((registry, name, latest));
                waker();
            });
        }
    }

    /// Take in the answers that arrived. True when something new is known.
    pub fn poll(&mut self) -> bool {
        let mut news = false;
        while let Ok((registry, name, latest)) = self.rx.try_recv() {
            self.asking.remove(&(registry, name.clone()));
            self.known.insert((registry, name), (latest, Instant::now()));
            news = true;
        }
        if news {
            self.start();
        }
        news
    }

    /// Still asking?
    pub fn busy(&self) -> bool {
        !self.asking.is_empty() || !self.queue.is_empty()
    }

    /// What is known about these dependencies, one note per answered line.
    pub fn notes(&self, dependencies: &[Dependency]) -> Vec<Note> {
        dependencies
            .iter()
            .filter_map(|d| {
                let (latest, _) = self.known.get(&(d.registry, d.name.clone()))?;
                let latest = latest.as_deref()?;
                let standing = standing(&d.wanted, latest);
                let text = match standing {
                    Standing::Latest => "✓ latest".to_string(),
                    Standing::Newer | Standing::Major => format!("↑ {latest}"),
                };
                Some(Note { line: d.line, text, standing })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependencies_of_a_package_json() {
        let text = r#"{
  "name": "demo",
  "version": "1.0.0",
  "scripts": { "build": "vite build" },
  "dependencies": {
    "react": "^18.2.0",
    "@scope/pkg": "~1.4",
    "local": "file:../local",
    "forked": "github:me/forked",
    "tagged": "latest"
  },
  "devDependencies": {
    "vite": "5.0.0"
  }
}"#;
        let deps = dependencies("package.json", text);
        let seen: Vec<(u32, &str, &str)> = deps.iter().map(|d| (d.line, d.name.as_str(), d.wanted.as_str())).collect();
        assert_eq!(seen, vec![(5, "react", "^18.2.0"), (6, "@scope/pkg", "~1.4"), (12, "vite", "5.0.0")]);
        assert!(deps.iter().all(|d| d.registry == Registry::Npm));
        assert!(dependencies("other.json", text).is_empty());
    }

    #[test]
    fn dependencies_of_a_cargo_toml() {
        let text = r#"[package]
name = "demo"
version = "0.1.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
log = "0.4" # logging
local = { path = "../local" }
shared.workspace = true
renamed = { package = "real-name", version = "2.1" }

[dev-dependencies]
tempfile = "3"

[target.'cfg(windows)'.dependencies]
wry = "0.57"

[dependencies.tokio]
version = "1.35"
features = ["full"]

[features]
default = []
"#;
        let deps = dependencies("Cargo.toml", text);
        let seen: Vec<(u32, &str, &str)> = deps.iter().map(|d| (d.line, d.name.as_str(), d.wanted.as_str())).collect();
        assert_eq!(
            seen,
            vec![
                (5, "serde", "1"),
                (6, "log", "0.4"),
                (9, "real-name", "2.1"),
                (12, "tempfile", "3"),
                (15, "wry", "0.57"),
                (18, "tokio", "1.35"),
            ]
        );
        assert!(deps.iter().all(|d| d.registry == Registry::Crates));
    }

    #[test]
    fn how_a_requirement_stands() {
        use Standing::*;
        for (wanted, latest, expected) in [
            ("^18.2.0", "18.2.0", Latest),
            ("^18.2.0", "18.3.1", Newer),
            ("^18.2.0", "19.0.0", Major),
            ("1", "1.0.228", Latest),
            ("1", "2.0.0", Major),
            ("0.4", "0.4.22", Latest),
            ("0.4", "0.5.0", Major),
            ("0.57", "0.57.2", Latest),
            ("~1.4", "1.5.0", Major),
            ("=1.4.0", "1.4.2", Major),
            ("5.0.0", "5.4.1", Newer),
            (">=2, <3", "3.1.0", Major),
            ("*", "9.9.9", Latest),
            ("3.0.0", "2.9.0", Latest),
            ("workspace", "1.0.0", Latest),
        ] {
            assert_eq!(standing(wanted, latest), expected, "{wanted} against {latest}");
        }
    }

    #[test]
    fn the_latest_of_an_index_entry() {
        let body = r#"{"name":"demo","vers":"0.9.0","yanked":false}
{"name":"demo","vers":"1.0.0","yanked":false}
{"name":"demo","vers":"1.10.0","yanked":false}
{"name":"demo","vers":"1.9.3","yanked":false}
{"name":"demo","vers":"2.0.0","yanked":true}
{"name":"demo","vers":"2.0.0-rc.1","yanked":false}
"#;
        assert_eq!(latest_in_index(body).as_deref(), Some("1.10.0"));
        assert_eq!(latest_in_index(""), None);
        assert_eq!(crates_index_path("a"), "1/a");
        assert_eq!(crates_index_path("io"), "2/io");
        assert_eq!(crates_index_path("syn"), "3/s/syn");
        assert_eq!(crates_index_path("Serde"), "se/rd/serde");
    }

    #[test]
    fn notes_for_what_is_known() {
        let mut checker = Checker::new(|| {});
        let deps = dependencies("Cargo.toml", "[dependencies]\nserde = \"1\"\nlog = \"0.3\"\nunknown = \"1\"\n");
        let now = Instant::now();
        checker.known.insert((Registry::Crates, "serde".into()), (Some("1.0.228".into()), now));
        checker.known.insert((Registry::Crates, "log".into()), (Some("0.4.22".into()), now));
        // Asked and not found (no network): nothing is said about it.
        checker.known.insert((Registry::Crates, "unknown".into()), (None, now));
        let notes = checker.notes(&deps);
        assert_eq!(
            notes,
            vec![
                Note { line: 1, text: "✓ latest".into(), standing: Standing::Latest },
                Note { line: 2, text: "↑ 0.4.22".into(), standing: Standing::Major },
            ]
        );
    }

    /// Asks the real registries: `cargo test -p code-canvas real_registries -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn the_real_registries_answer() {
        for (registry, name) in [
            (Registry::Npm, "react"),
            (Registry::Npm, "@types/node"),
            (Registry::Crates, "serde"),
            (Registry::Crates, "syn"),
        ] {
            let latest = fetch(registry, name).unwrap_or_else(|e| panic!("{name}: {e}"));
            println!("{registry:?} {name}: {latest}");
            assert!(numbers(&latest).len() >= 2);
        }
    }
}
