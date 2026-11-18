use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

mod manifests;
pub mod npmrc;
pub mod range;
mod registries;

pub use manifests::{dependencies, is_manifest};
use npmrc::Secret;
use range::{Flavor, Requirement, Version};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Registry {
    Npm,
    Crates,
    PyPI,
    NuGet,
    Go,
    Packagist,
    Pub,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Source {
    #[default]
    Public,
    Mirror(String),
    Private(String, Secret),
    NoToken(String),
}

impl Source {
    fn base(&self) -> Option<&str> {
        match self {
            Source::Public => None,
            Source::Mirror(base) | Source::Private(base, _) | Source::NoToken(base) => Some(base),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dependency {
    pub line: u32,
    pub col: u32,
    pub name: String,
    pub wanted: String,
    pub registry: Registry,
    pub flavor: Flavor,
    pub source: Source,
}

type Key = (Registry, Option<String>, String);

impl Dependency {
    fn key(&self) -> Key {
        (self.registry, self.source.base().map(str::to_string), self.name.clone())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub text: String,
    pub v: Version,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Facts {
    pub releases: Vec<Release>,
    pub latest: Option<Release>,
}

impl Facts {
    pub fn new(texts: impl Iterator<Item = String>, hint: Option<String>) -> Facts {
        let mut releases: Vec<Release> =
            texts.filter_map(|text| Some(Release { v: Version::parse(&text)?, text })).collect();
        releases.sort_by(|a, b| a.v.cmp(&b.v));
        releases.dedup_by(|a, b| a.v == b.v);
        let hinted = hint.and_then(|text| Some(Release { v: Version::parse(&text)?, text })).filter(|r| !r.v.is_pre());
        let stable = || releases.iter().rev().find(|r| !r.v.is_pre()).cloned();
        let latest = hinted.or_else(stable).or_else(|| releases.last().cloned());
        Facts { releases, latest }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Heard {
    Facts(Facts),
    NotFound,
    Refused,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    Latest,
    InRange,
    Behind,
    NoMatch,
    Private,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub label: String,
    pub version: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub line: u32,
    pub name: String,
    pub wanted: String,
    pub text: String,
    pub standing: Standing,
    pub choices: Vec<Choice>,
    pub latest: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bulk {
    Latest,
    Major,
    Minor,
    Patch,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Set { path: std::path::PathBuf, line: u32, name: String, text: String },
    Update(Bulk, Option<std::path::PathBuf>),
    Hints(bool),
    Prereleases(bool),
    Recheck,
    Install(Option<std::path::PathBuf>),
}

pub struct Judged {
    pub standing: Standing,
    pub text: String,
    pub choices: Vec<Choice>,
    pub latest: String,
    floor: Option<Version>,
}

pub const LATEST: &str = "latest";
pub const PRERELEASE: &str = "prerelease";

pub fn judge(wanted: &str, flavor: Flavor, facts: &Facts, prereleases: bool) -> Option<Judged> {
    let req = Requirement::parse(wanted, flavor)?;
    let latest = facts.latest.as_ref()?;
    let floor = req.floor();
    let show_pre = prereleases || req.mentions_pre();
    let mut choices: Vec<Choice> = Vec::new();
    let mut offer = |label: String, r: &Release| {
        if floor.as_ref().is_some_and(|f| r.v <= *f) {
            return;
        }
        if let Some(text) = range::rewrite(wanted, &r.text) {
            if !choices.iter().any(|c| c.text == text) {
                choices.push(Choice { label, version: r.text.clone(), text });
            }
        }
    };
    if !latest.v.is_pre() || show_pre {
        offer(LATEST.to_string(), latest);
    }
    if let Some(f) = &floor {
        let mut stable = facts.releases.iter().rev().filter(|r| !r.v.is_pre());
        if let Some(r) = stable.clone().find(|r| r.v.num(0) == f.num(0)) {
            offer(format!("latest {}.x", f.num(0)), r);
        }
        if let Some(r) = stable.find(|r| r.v.num(0) == f.num(0) && r.v.num(1) == f.num(1)) {
            offer(format!("latest {}.{}.x", f.num(0), f.num(1)), r);
        }
    }
    let newest_pre = facts.releases.iter().rev().find(|r| r.v.is_pre()).filter(|r| r.v > latest.v);
    if let (true, Some(pre)) = (show_pre, newest_pre) {
        offer(PRERELEASE.to_string(), pre);
    }
    let matched = facts.releases.iter().any(|r| req.matches(&r.v));
    let bump = choices.iter().any(|c| c.label == LATEST);
    let ahead = floor.as_ref().is_some_and(|f| *f >= latest.v);
    let standing = match () {
        _ if !facts.releases.is_empty() && !matched => Standing::NoMatch,
        _ if req.matches(&latest.v) && bump => Standing::InRange,
        _ if req.matches(&latest.v) || ahead => Standing::Latest,
        _ => Standing::Behind,
    };
    let mut text = match standing {
        Standing::Latest => "✓ latest".to_string(),
        Standing::InRange => format!("↑ {} in range", latest.text),
        Standing::NoMatch => format!("✗ no version matches · latest {}", latest.text),
        _ => format!("↑ {}", latest.text),
    };
    if let Some(pre) = choices.iter().find(|c| c.label == PRERELEASE) {
        text.push_str(&format!("  ◇ {}", pre.version));
    }
    Some(Judged { standing, text, choices, latest: latest.text.clone(), floor })
}

pub fn bulk_text(judged: &Judged, bulk: Bulk) -> Option<String> {
    let floor = judged.floor.as_ref()?;
    let target = |c: &&Choice| Version::parse(&c.version);
    let mut stable = judged.choices.iter().filter(|c| c.label != PRERELEASE);
    let chosen = match bulk {
        Bulk::Latest => judged
            .choices
            .iter()
            .find(|c| c.label == LATEST)
            .or_else(|| judged.choices.iter().find(|c| c.label == PRERELEASE).filter(|_| floor.is_pre())),
        Bulk::Major => judged
            .choices
            .iter()
            .find(|c| c.label == LATEST)
            .filter(|c| target(c).is_some_and(|v| v.num(0) > floor.num(0))),
        Bulk::Minor => stable.find(|c| target(c).is_some_and(|v| v.num(0) == floor.num(0))),
        Bulk::Patch => stable.find(|c| target(c).is_some_and(|v| v.num(0) == floor.num(0) && v.num(1) == floor.num(1))),
    };
    chosen.map(|c| c.text.clone())
}

pub fn install_for(file_name: &str, locks: &[&str], text: &str) -> Option<String> {
    let has = |name: &str| locks.contains(&name);
    let lower = file_name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "package.json" if has("pnpm-lock.yaml") => "pnpm install".into(),
        "package.json" if has("yarn.lock") => "yarn install".into(),
        "package.json" if has("bun.lockb") || has("bun.lock") => "bun install".into(),
        "package.json" => "npm install".into(),
        "cargo.toml" => "cargo fetch".into(),
        "pyproject.toml" if has("uv.lock") => "uv sync".into(),
        "pyproject.toml" if has("poetry.lock") => "poetry lock && poetry install".into(),
        "pyproject.toml" => return None,
        "go.mod" => "go mod tidy".into(),
        "composer.json" => "composer update".into(),
        "pubspec.yaml" if text.contains("sdk: flutter") => "flutter pub get".into(),
        "pubspec.yaml" => "dart pub get".into(),
        _ if lower.starts_with("requirements") && lower.ends_with(".txt") => format!("pip install -r {file_name}"),
        _ if is_manifest(&lower) => "dotnet restore".into(),
        _ => return None,
    })
}

pub fn install_command(manifest: &Path, root: &Path, text: &str) -> Option<String> {
    const LOCKS: [&str; 6] = ["pnpm-lock.yaml", "yarn.lock", "bun.lockb", "bun.lock", "uv.lock", "poetry.lock"];
    let name = manifest.file_name()?.to_string_lossy().into_owned();
    let dir = manifest.parent()?;
    let mut found: Vec<&str> = Vec::new();
    for folder in dir.ancestors().take_while(|d| d.starts_with(root)) {
        found.extend(LOCKS.iter().filter(|lock| folder.join(lock).is_file()));
        if !found.is_empty() {
            break;
        }
    }
    install_for(&name, &found, text)
}

pub struct Checker {
    known: HashMap<Key, (Heard, Instant)>,
    asking: HashSet<Key>,
    tx: Sender<(Key, Heard)>,
    rx: Receiver<(Key, Heard)>,
    queue: Vec<(Key, Option<Secret>)>,
    waker: fn(),
}

const WORKERS: usize = 4;
const FRESH: Duration = Duration::from_secs(3600);

impl Checker {
    pub fn new(waker: fn()) -> Self {
        let (tx, rx) = mpsc::channel();
        Checker { known: HashMap::new(), asking: HashSet::new(), tx, rx, queue: Vec::new(), waker }
    }

    pub fn want(&mut self, dependencies: &[Dependency]) {
        for d in dependencies {
            if matches!(d.source, Source::NoToken(_)) {
                continue;
            }
            let key = d.key();
            let fresh = self.known.get(&key).is_some_and(|(_, at)| at.elapsed() < FRESH);
            if !fresh && !self.asking.contains(&key) && !self.queue.iter().any(|(k, _)| *k == key) {
                let secret = match &d.source {
                    Source::Private(_, secret) => Some(secret.clone()),
                    _ => None,
                };
                self.queue.push((key, secret));
            }
        }
        self.start();
    }

    fn start(&mut self) {
        while self.asking.len() < WORKERS {
            let Some((key, secret)) = self.queue.pop() else { break };
            self.asking.insert(key.clone());
            let (tx, waker) = (self.tx.clone(), self.waker);
            std::thread::spawn(move || {
                let (registry, base, name) = &key;
                let found = registries::fetch(*registry, base.as_deref(), name, secret.as_ref());
                let _ = tx.send((key, found));
                waker();
            });
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut news = false;
        while let Ok((key, found)) = self.rx.try_recv() {
            self.asking.remove(&key);
            self.known.insert(key, (found, Instant::now()));
            news = true;
        }
        if news {
            self.start();
        }
        news
    }

    pub fn busy(&self) -> bool {
        !self.asking.is_empty() || !self.queue.is_empty()
    }

    pub fn forget(&mut self) {
        self.known.clear();
        self.queue.clear();
    }

    fn judged(&self, d: &Dependency, prereleases: bool) -> Option<Judged> {
        match &self.known.get(&d.key())?.0 {
            Heard::Facts(facts) => judge(&d.wanted, d.flavor, facts, prereleases),
            _ => None,
        }
    }

    pub fn notes(&self, dependencies: &[Dependency], prereleases: bool) -> Vec<Note> {
        dependencies
            .iter()
            .filter_map(|d| {
                let plain = |standing: Standing, text: &str| (standing, text.to_string(), Vec::new(), None);
                let (standing, text, choices, latest) =
                    match (&d.source, self.known.get(&d.key()).map(|(found, _)| found)) {
                        (Source::NoToken(_), _) => plain(Standing::Private, "private registry"),
                        (_, Some(Heard::Refused)) => plain(Standing::Private, "private registry: no access"),
                        (_, Some(Heard::NotFound)) => plain(Standing::Unknown, "not found in the registry"),
                        (_, Some(Heard::Facts(_))) => {
                            let judged = self.judged(d, prereleases)?;
                            (judged.standing, judged.text, judged.choices, Some(judged.latest))
                        }
                        _ => return None,
                    };
                let (line, name, wanted) = (d.line, d.name.clone(), d.wanted.clone());
                Some(Note { line, name, wanted, text, standing, choices, latest })
            })
            .collect()
    }

    pub fn updates(&self, dependencies: &[Dependency], bulk: Bulk, prereleases: bool) -> Vec<(u32, String, String)> {
        dependencies
            .iter()
            .filter_map(|d| {
                let text = bulk_text(&self.judged(d, prereleases)?, bulk)?;
                Some((d.line, d.name.clone(), text))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(versions: &[&str], latest: Option<&str>) -> Facts {
        Facts::new(versions.iter().map(|v| v.to_string()), latest.map(str::to_string))
    }

    const REACT: [&str; 9] =
        ["17.0.2", "18.2.0", "18.2.5", "18.3.0", "18.3.1", "19.0.0", "19.1.0", "19.2.0-rc.1", "20.0.0-beta.1"];

    #[test]
    fn how_a_requirement_stands() {
        use Standing::*;
        let react = facts(&REACT, Some("19.1.0"));
        let serde = facts(&["0.9.0", "1.0.0", "1.0.228"], None);
        let log = facts(&["0.3.9", "0.4.0", "0.4.22", "0.5.0-rc.1"], None);
        for (wanted, flavor, found, standing, text) in [
            ("^19.1.0", Flavor::Npm, &react, Latest, "✓ latest"),
            ("^19.0.0", Flavor::Npm, &react, InRange, "↑ 19.1.0 in range"),
            ("^18.2.0", Flavor::Npm, &react, Behind, "↑ 19.1.0"),
            ("18.2.0", Flavor::Npm, &react, Behind, "↑ 19.1.0"),
            (">=17", Flavor::Npm, &react, InRange, "↑ 19.1.0 in range"),
            (">=17 <20", Flavor::Npm, &react, Latest, "✓ latest"),
            ("*", Flavor::Npm, &react, Latest, "✓ latest"),
            ("^99.0.0", Flavor::Npm, &react, NoMatch, "✗ no version matches · latest 19.1.0"),
            ("20.0.0-beta.1", Flavor::Npm, &react, Latest, "✓ latest"),
            ("1", Flavor::Cargo, &serde, Latest, "✓ latest"),
            ("1.0.100", Flavor::Cargo, &serde, InRange, "↑ 1.0.228 in range"),
            ("0.4", Flavor::Cargo, &log, Latest, "✓ latest"),
            ("0.3", Flavor::Cargo, &log, Behind, "↑ 0.4.22"),
            ("=0.4.0", Flavor::Cargo, &log, Behind, "↑ 0.4.22"),
        ] {
            let judged = judge(wanted, flavor, found, false).unwrap_or_else(|| panic!("{wanted} is judged"));
            assert_eq!((judged.standing, judged.text.as_str()), (standing, text), "{wanted}");
        }
        assert!(judge("latest", Flavor::Npm, &react, false).is_none());
        assert!(judge("^1.0.0", Flavor::Npm, &Facts::default(), false).is_none());
    }

    fn offered(judged: &Judged) -> Vec<(&str, &str)> {
        judged.choices.iter().map(|c| (c.label.as_str(), c.text.as_str())).collect()
    }

    #[test]
    fn the_choices_offered_for_a_dependency() {
        let react = facts(&REACT, Some("19.1.0"));
        let judged = judge("^18.2.0", Flavor::Npm, &react, false).unwrap();
        assert_eq!(
            offered(&judged),
            vec![("latest", "^19.1.0"), ("latest 18.x", "^18.3.1"), ("latest 18.2.x", "^18.2.5")]
        );
        let with_pre = judge("^18.2.0", Flavor::Npm, &react, true).unwrap();
        assert_eq!(
            with_pre.choices.last().map(|c| (c.label.as_str(), c.text.as_str())),
            Some(("prerelease", "^20.0.0-beta.1"))
        );
        assert_eq!(with_pre.text, "↑ 19.1.0  ◇ 20.0.0-beta.1");
        let pre = judge("^19.2.0-rc.0", Flavor::Npm, &react, false).unwrap();
        assert_eq!(offered(&pre), vec![("prerelease", "^20.0.0-beta.1")]);
        assert_eq!((pre.standing, pre.text.as_str()), (Standing::Latest, "✓ latest  ◇ 20.0.0-beta.1"));
        let tilde = judge("~18.2.0", Flavor::Npm, &react, false).unwrap();
        assert_eq!(
            offered(&tilde),
            vec![("latest", "~19.1.0"), ("latest 18.x", "~18.3.1"), ("latest 18.2.x", "~18.2.5")]
        );
        let pinned = judge("==2.28.1", Flavor::Exact, &facts(&["2.28.1", "2.28.2", "2.31.0"], None), false).unwrap();
        assert_eq!(offered(&pinned), vec![("latest", "==2.31.0"), ("latest 2.28.x", "==2.28.2")]);
        let ranged = judge(">=1.2 <2", Flavor::Npm, &react, false).unwrap();
        assert!(ranged.choices.is_empty());
    }

    #[test]
    fn bulk_updates_pick_their_kind_of_version() {
        let react = facts(&REACT, Some("19.1.0"));
        let old = judge("^18.2.0", Flavor::Npm, &react, false).unwrap();
        assert_eq!(bulk_text(&old, Bulk::Latest).as_deref(), Some("^19.1.0"));
        assert_eq!(bulk_text(&old, Bulk::Major).as_deref(), Some("^19.1.0"));
        assert_eq!(bulk_text(&old, Bulk::Minor).as_deref(), Some("^18.3.1"));
        assert_eq!(bulk_text(&old, Bulk::Patch).as_deref(), Some("^18.2.5"));
        let recent = judge("^19.0.0", Flavor::Npm, &react, false).unwrap();
        assert_eq!(bulk_text(&recent, Bulk::Major), None);
        assert_eq!(bulk_text(&recent, Bulk::Minor).as_deref(), Some("^19.1.0"));
        assert_eq!(bulk_text(&recent, Bulk::Patch), None);
        let current = judge("^19.1.0", Flavor::Npm, &react, false).unwrap();
        assert!([Bulk::Latest, Bulk::Major, Bulk::Minor, Bulk::Patch]
            .iter()
            .all(|b| bulk_text(&current, *b).is_none()));
        let pre = judge("^20.0.0-alpha.1", Flavor::Npm, &react, false).unwrap();
        assert_eq!(bulk_text(&pre, Bulk::Latest).as_deref(), Some("^20.0.0-beta.1"));
    }

    #[test]
    fn notes_for_what_is_known() {
        let mut checker = Checker::new(|| {});
        let mut deps =
            dependencies("Cargo.toml", "[dependencies]\nserde = \"1\"\nlog = \"0.3\"\nunknown = \"1\"\ngone = \"1\"\n");
        deps.push(Dependency {
            line: 9,
            col: 0,
            name: "@corp/lib".into(),
            wanted: "^1.0.0".into(),
            registry: Registry::Npm,
            flavor: Flavor::Npm,
            source: Source::NoToken("https://npm.pkg.github.com/".into()),
        });
        let now = Instant::now();
        let key = |name: &str| (Registry::Crates, None, name.to_string());
        checker.known.insert(key("serde"), (Heard::Facts(facts(&["1.0.228"], None)), now));
        checker.known.insert(key("log"), (Heard::Facts(facts(&["0.3.9", "0.4.22"], None)), now));
        checker.known.insert(key("unknown"), (Heard::Failed, now));
        checker.known.insert(key("gone"), (Heard::NotFound, now));
        let notes = checker.notes(&deps, false);
        let seen: Vec<(u32, &str, Standing)> = notes.iter().map(|n| (n.line, n.text.as_str(), n.standing)).collect();
        assert_eq!(
            seen,
            vec![
                (1, "✓ latest", Standing::Latest),
                (2, "↑ 0.4.22", Standing::Behind),
                (4, "not found in the registry", Standing::Unknown),
                (9, "private registry", Standing::Private),
            ]
        );
        assert_eq!(checker.updates(&deps, Bulk::Latest, false), vec![(2, "log".to_string(), "0.4".to_string())]);
        checker.want(&deps[3..]);
        assert!(!checker.queue.iter().any(|(k, _)| k.2 == "@corp/lib"));
    }

    #[test]
    fn the_install_of_each_kind_of_project() {
        for (file, locks, text, expected) in [
            ("package.json", &[][..], "", Some("npm install")),
            ("package.json", &["pnpm-lock.yaml"], "", Some("pnpm install")),
            ("package.json", &["yarn.lock"], "", Some("yarn install")),
            ("Cargo.toml", &[], "", Some("cargo fetch")),
            ("pyproject.toml", &["uv.lock"], "", Some("uv sync")),
            ("pyproject.toml", &[], "", None),
            ("requirements-dev.txt", &[], "", Some("pip install -r requirements-dev.txt")),
            ("App.csproj", &[], "", Some("dotnet restore")),
            ("go.mod", &[], "", Some("go mod tidy")),
            ("pubspec.yaml", &[], "  flutter:\n    sdk: flutter\n", Some("flutter pub get")),
            ("pubspec.yaml", &[], "", Some("dart pub get")),
            ("README.md", &[], "", None),
        ] {
            assert_eq!(install_for(file, locks, text).as_deref(), expected, "{file}");
        }
    }

    #[test]
    #[ignore]
    fn the_real_registries_answer() {
        for (registry, name) in [
            (Registry::Npm, "react"),
            (Registry::Npm, "@types/node"),
            (Registry::Crates, "serde"),
            (Registry::Crates, "syn"),
            (Registry::PyPI, "requests"),
            (Registry::NuGet, "Newtonsoft.Json"),
            (Registry::Go, "github.com/spf13/cobra"),
            (Registry::Packagist, "monolog/monolog"),
            (Registry::Pub, "http"),
        ] {
            match registries::fetch(registry, None, name, None) {
                Heard::Facts(facts) => {
                    let latest = facts.latest.unwrap_or_else(|| panic!("{name}: no latest"));
                    println!("{registry:?} {name}: {} of {} versions", latest.text, facts.releases.len());
                    assert!(latest.v.nums.len() >= 2);
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }
}
