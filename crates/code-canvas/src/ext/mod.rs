pub mod compat;
pub mod details;
pub mod gallery;
pub mod host;
pub mod market;
pub mod views;
pub mod when;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use host::{Host, HostEvent};
pub use market::MarketItem;
pub use views::TREE;

#[derive(Clone)]
pub struct Waker(Arc<dyn Fn() + Send + Sync>);

impl Waker {
    pub fn new(f: impl Fn() + Send + Sync + 'static) -> Self {
        Waker(Arc::new(f))
    }

    pub fn wake(&self) {
        (self.0)()
    }
}

static UI_WAKER: std::sync::OnceLock<Waker> = std::sync::OnceLock::new();

pub fn set_ui_waker(waker: Waker) {
    let _ = UI_WAKER.set(waker);
}

pub fn wake_ui() {
    if let Some(w) = UI_WAKER.get() {
        w.wake();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Idle,
    Activating,
    Active,
    Failed,
    Declarative,
    Disabled,
    Restricted,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Untrusted {
    Supported,
    Limited(Vec<String>),
    Unsupported,
}

const UNTRUSTED_KNOWN: &[&str] = &["usernamehw.errorlens", "yoavbls.pretty-ts-errors"];

pub const ALWAYS_RESTRICTED: &[&str] = &[
    "typescript.tsdk",
    "typescript.tsserver.pluginPaths",
    "typescript.tsserver.nodePath",
    "typescript.npm",
    "js/ts.tsdk.path",
    "eslint",
    "prettier.prettierPath",
    "prettier.configPath",
    "prettier.ignorePath",
    "prettier.resolveGlobalModules",
    "prettier.withNodeModules",
    "prettier.documentSelectors",
    "git.path",
    "npm.packageManager",
    "terminal.integrated",
];

fn manifest_untrusted(v: &Value, id: &str, has_code: bool) -> Untrusted {
    if !has_code {
        return Untrusted::Supported;
    }
    let Some(declared) = v.pointer("/capabilities/untrustedWorkspaces") else {
        return match UNTRUSTED_KNOWN.contains(&id) {
            true => Untrusted::Supported,
            false => Untrusted::Unsupported,
        };
    };
    match declared.get("supported") {
        Some(Value::Bool(true)) => Untrusted::Supported,
        Some(Value::String(s)) if s == "limited" => {
            Untrusted::Limited(strings(declared.get("restrictedConfigurations")))
        }
        Some(Value::String(s)) if s == "true" => Untrusted::Supported,
        _ => Untrusted::Unsupported,
    }
}

pub fn runs_in(e: &Installed, trusted: bool) -> bool {
    e.enabled && (trusted || !e.has_code || e.untrusted != Untrusted::Unsupported)
}

pub fn restricted_keys(installed: &[Installed]) -> Vec<String> {
    let mut keys: BTreeSet<String> = ALWAYS_RESTRICTED.iter().map(|k| k.to_string()).collect();
    for e in installed.iter().filter(|e| e.enabled) {
        if let Untrusted::Limited(list) = &e.untrusted {
            keys.extend(list.iter().filter(|k| !k.trim().is_empty()).cloned());
        }
    }
    keys.into_iter().collect()
}

pub fn held_back(installed: &[Installed]) -> impl Iterator<Item = &Installed> {
    installed.iter().filter(|e| e.enabled && e.has_code && e.untrusted != Untrusted::Supported)
}

pub const DEFAULTS: &[(&str, &str)] = &[
    ("Anthropic", "claude-code"),
    ("esbenp", "prettier-vscode"),
    ("dbaeumer", "vscode-eslint"),
    ("miguelsolorio", "symbols"),
];

fn defaults_to_install(
    defaults: &[(&str, &str)],
    offered: &mut BTreeSet<String>,
    is_installed: &dyn Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let mut wanted = Vec::new();
    for (namespace, name) in defaults {
        let id = format!("{namespace}.{name}").to_lowercase();
        if offered.contains(&id) {
            continue;
        }
        if is_installed(&id) {
            offered.insert(id);
        } else {
            wanted.push((namespace.to_string(), name.to_string()));
        }
    }
    wanted
}

pub struct Installed {
    pub id: String,
    pub display_name: String,
    pub publisher: String,
    pub version: String,
    pub description: String,
    pub dir: PathBuf,
    pub has_code: bool,
    pub enabled: bool,
    pub status: Status,
    pub error: String,
    pub activation_ms: u32,
    /// What it asked the host for while running that is not there yet (`vscode.…`).
    pub unsupported: Vec<String>,
    /// Its package.json, and what of it does not work in Code Canvas (`compat::gaps`).
    pub manifest: Arc<gallery::Manifest>,
    pub untrusted: Untrusted,
    pub icon: Option<PathBuf>,
    pub commands: Vec<(String, String)>,
    pub views: Vec<String>,
    pub containers: Vec<Container>,
    pub menus: Vec<views::MenuItem>,
    pub icon_themes: Vec<IconThemeDecl>,
    pub color_themes: Vec<ColorThemeDecl>,
    pub languages: Vec<LanguageDecl>,
    pub grammars: Vec<GrammarDecl>,
}

impl Installed {
    /// What of it does not work in Code Canvas, one line each: what its manifest brings,
    /// then what it asked for while running.
    pub fn not_available(&self) -> Vec<String> {
        let mut out = compat::lines(&self.manifest.gaps);
        out.extend(self.unsupported.iter().map(|api| format!("vscode.{api} (asked for while running)")));
        out
    }

    /// Its publisher and name as the registry spells them.
    pub fn namespace_and_name(&self) -> (String, String) {
        let v = &self.manifest.json;
        let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        (text("publisher"), text("name"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LanguageDecl {
    pub id: String,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Hash)]
pub struct GrammarDecl {
    pub language: Option<String>,
    pub scope_name: String,
    pub file: PathBuf,
    pub inject_to: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorThemeDecl {
    pub key: String,
    pub label: String,
    pub file: PathBuf,
    pub light: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconThemeDecl {
    pub id: String,
    pub label: String,
    pub file: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub id: String,
    pub title: String,
    pub codicon: Option<String>,
    pub views: Vec<ViewDecl>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewDecl {
    pub id: String,
    pub name: String,
    pub webview: bool,
    pub when: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RailItem {
    pub view: String,
    pub extension: String,
    pub title: String,
    pub codicon: Option<String>,
    pub tabs: usize,
}

fn view_words(id: &str) -> String {
    let mut words = String::new();
    let mut previous_lower = false;
    for ch in id.chars() {
        if ch.is_uppercase() && previous_lower {
            words.push(' ');
        }
        previous_lower = ch.is_lowercase();
        words.extend(ch.to_lowercase());
    }
    words
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub severity: Severity,
    pub message: String,
    pub source: String,
    pub code: String,
}

impl Diagnostic {
    pub fn covers(&self, line: u32, col: u32) -> bool {
        if line < self.line || line > self.end_line {
            return false;
        }
        let after_start = line > self.line || col >= self.col;
        let before_end = line < self.end_line || col < self.end_col.max(self.col + 1);
        after_start && before_end
    }

    pub fn title(&self) -> String {
        match (self.source.is_empty(), self.code.is_empty()) {
            (false, false) => format!("{}({})", self.source, self.code),
            (false, true) => self.source.clone(),
            (true, false) => self.code.clone(),
            (true, true) => "problem".to_string(),
        }
    }
}

pub struct FileDiagnostics {
    pub path: String,
    pub items: Vec<Diagnostic>,
    pub errors: u32,
    pub warnings: u32,
}

#[derive(Clone, Debug)]
pub struct Decoration {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub whole_line: bool,
    pub text: String,
    pub color: u32,
    pub background: u32,
}

pub struct FileDecorations {
    pub path: String,
    pub by_type: HashMap<String, Vec<Decoration>>,
}

fn parse_color(name: &str) -> u32 {
    if let Some(id) = name.strip_prefix("theme:") {
        let id = id.rsplit('.').next().unwrap_or(id).to_lowercase();
        let rgb = if id.contains("error") {
            0xf7768e00
        } else if id.contains("warning") {
            0xe0af6800
        } else if id.contains("info") {
            0x7aa2f700
        } else if id.contains("hint") {
            0x73daca00
        } else {
            0x8a8e9900
        };
        return rgb | if id.contains("background") { 0x1c } else { 0xe6 };
    }
    let hex = name.trim().trim_start_matches('#');
    let digits: Option<Vec<u32>> = hex.chars().map(|c| c.to_digit(16)).collect();
    match (name.trim().starts_with('#'), digits.as_deref()) {
        (true, Some([r, g, b])) => (r * 17) << 24 | (g * 17) << 16 | (b * 17) << 8 | 0xff,
        (true, Some(d)) if d.len() == 6 || d.len() == 8 => {
            let v = d.iter().fold(0u32, |acc, x| acc << 4 | x);
            if d.len() == 6 {
                v << 8 | 0xff
            } else {
                v
            }
        }
        _ => 0,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Warn,
    Error,
}

pub struct Notice {
    pub level: Level,
    pub source: String,
    pub text: String,
    pub at: Instant,
    pub actions: Vec<String>,
    reply_to: Option<u64>,
    own: Option<String>,
}

impl Notice {
    fn is_question(&self) -> bool {
        self.reply_to.is_some() || self.own.is_some()
    }
}

pub enum NodeStatus {
    Checking,
    Ready(String),
    Missing(String),
}

pub enum Event {
    Write {
        path: PathBuf,
        text: String,
    },
    Reveal(PathBuf),
    WebviewHtml {
        handle: String,
        html: String,
    },
    WebviewPost {
        handle: String,
        message: Value,
    },
    WebviewClosed(String),
    ShowWebview(String),
    GoTo {
        path: PathBuf,
        line: u32,
        col: u32,
        len: u32,
    },
    Suggestions(Suggestions),
    Relations {
        page: u32,
        of: (u32, u32),
        found: Found,
    },
    ExtraEdits {
        path: PathBuf,
        edits: Vec<ExtraEdit>,
    },
    /// Select the name at that place, which shows its uses (what "3 references" asks).
    ShowUses {
        path: PathBuf,
        line: u32,
        col: u32,
    },
}

/// A place in a file: path, line of the file and display column.
pub type Place<P = PathBuf> = (P, u32, u32);

/// What a language server knows about a selected name, to put in place of the relations
/// guessed by name.
#[derive(Clone, Debug, PartialEq)]
pub enum Found<P = PathBuf> {
    /// Where it is used: its references and incoming calls.
    Uses(Vec<Place<P>>),
    /// The calls it makes: where each call is, and where what it calls is named. Empty when
    /// the server knew the function and it calls nothing.
    Calls(Vec<(Place<P>, Place<P>)>),
    /// What implements it.
    Implementations(Vec<Place<P>>),
}

impl<P> Found<P> {
    /// The same answer with other paths; a place `path` gives none for is left out, and so
    /// is a call with one end there.
    pub fn with_paths<Q>(&self, path: impl Fn(&P) -> Option<Q>) -> Found<Q> {
        let place = |(p, line, col): &Place<P>| Some((path(p)?, *line, *col));
        match self {
            Found::Uses(list) => Found::Uses(list.iter().filter_map(place).collect()),
            Found::Calls(list) => Found::Calls(list.iter().filter_map(|(a, b)| Some((place(a)?, place(b)?))).collect()),
            Found::Implementations(list) => Found::Implementations(list.iter().filter_map(place).collect()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Asked {
    Uses,
    Calls,
    Implementations,
}

/// How long a language server is given to say what a selected name is related to. Until it
/// answers the guesses are shown; past this the request is cancelled and they stay.
const RELATIONS_WAIT_MS: u64 = 20_000;

fn found_of(asked: Asked, result: Option<&Value>) -> Option<Found> {
    let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    let items = result.and_then(|r| r.get("items")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let place = |v: &Value| Some((PathBuf::from(v.get("path")?.as_str()?), n(v, "line"), n(v, "col")));
    match asked {
        Asked::Uses | Asked::Implementations => {
            let places: Vec<Place> = items.iter().filter_map(place).collect();
            if places.is_empty() {
                return None;
            }
            Some(if asked == Asked::Uses { Found::Uses(places) } else { Found::Implementations(places) })
        }
        Asked::Calls => {
            let known = result.and_then(|r| r.get("known")).and_then(Value::as_bool).unwrap_or(false);
            let call = |v: &Value| {
                let from = (PathBuf::from(v.get("fromPath")?.as_str()?), n(v, "fromLine"), n(v, "fromCol"));
                Some((from, place(v)?))
            };
            let calls: Vec<(Place, Place)> = items.iter().filter_map(call).collect();
            (known || !calls.is_empty()).then_some(Found::Calls(calls))
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub kind: String,
    pub text: String,
    pub caret: Option<usize>,
    pub select: usize,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestions {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
    pub serial: u64,
    pub items: Vec<Suggestion>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtraEdit {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub text: String,
}

fn suggestions_of(result: Option<&Value>, path: PathBuf, line: u32, col: u32) -> Suggestions {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let number = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64);
    let items = result.and_then(|r| r.get("items")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let items = items
        .iter()
        .map(|v| Suggestion {
            label: text(v, "label"),
            detail: text(v, "detail"),
            kind: text(v, "kind"),
            text: text(v, "text"),
            caret: number(v, "caret").map(|n| n as usize),
            select: number(v, "select").unwrap_or(0) as usize,
            col: number(v, "col").unwrap_or(0) as u32,
        })
        .collect();
    let serial = result.and_then(|r| number(r, "serial")).unwrap_or(0);
    Suggestions { path, line, col, serial, items }
}

fn extra_edits_of(result: Option<&Value>) -> Vec<ExtraEdit> {
    let number = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    let edits = result.and_then(|r| r.get("edits")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    edits
        .iter()
        .map(|v| ExtraEdit {
            line: number(v, "line"),
            col: number(v, "col"),
            end_line: number(v, "endLine"),
            end_col: number(v, "endCol"),
            text: v.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
        })
        .collect()
}

pub struct Hover {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
    pub text: String,
}

pub struct WebviewEntry {
    pub key: String,
    pub extension: String,
    pub title: String,
    pub handle: Option<String>,
    resolving: bool,
    pub failed: bool,
    pub tab: bool,
}

pub const TABS: &str = "tabs:";

enum Job {
    Node(Result<String, String>),
    Search(String, Result<Vec<MarketItem>, String>),
    Installed(String, Result<PathBuf, String>),
}

enum Pending {
    Format(PathBuf, bool),
    Command(String),
    Webview(String),
    Hover(PathBuf, u32, u32),
    Definition(PathBuf),
    Complete(PathBuf, u32, u32),
    Relations(u32, (u32, u32), Asked),
    CompletionExtra(PathBuf),
    View(views::Asked),
}

const MAX_OPEN: usize = 48;
const MAX_LOG: usize = 400;
const MAX_WRITE: usize = 64 * 1024 * 1024;
const MAX_HTML: usize = 16 * 1024 * 1024;
const MAX_TEXT: usize = 2000;
const MAX_SHORT: usize = 200;
const MAX_STATUS: usize = 40;
const MAX_FILES: usize = 5000;
const MAX_ITEMS: usize = 2000;
const MAX_KINDS: usize = 64;
const MAX_CONTEXTS: usize = 4000;
const MAX_CONTEXT_VALUE: usize = 16 * 1024;
const MAX_WEBVIEWS: usize = 64;
const MAX_QUESTIONS: usize = 8;
const MAX_ACTIONS: usize = 8;
const MAX_UNSUPPORTED: usize = 64;

pub fn clip(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn real_key(path: &Path) -> Option<String> {
    let real = match path.canonicalize() {
        Ok(real) => real,
        Err(_) if std::fs::symlink_metadata(path).is_ok() => return None,
        Err(_) => path.parent()?.canonicalize().ok()?.join(path.file_name()?),
    };
    Some(path_key(&canvas_core::scan::clean_path(real).to_string_lossy()))
}

pub fn writable(path: &Path, open: &[PathBuf], folders: &[PathBuf], forbidden: &[PathBuf]) -> Result<(), &'static str> {
    use std::path::Component;
    if !path.is_absolute() {
        return Err("it is not a full path");
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("it goes up with ..");
    }
    if path.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(".git")) {
        return Err("it is inside .git");
    }
    let wanted = path_key(&path.to_string_lossy());
    if !open.iter().any(|p| path_key(&p.to_string_lossy()) == wanted) {
        return Err("it is not a document open in the app");
    }
    let real = real_key(path).ok_or("it cannot be found")?;
    let inside = |root: &PathBuf| {
        real_key(root).is_some_and(|root| {
            let root = root.trim_end_matches('/');
            real.len() > root.len() && real.starts_with(root) && real[root.len()..].starts_with('/')
        })
    };
    if !folders.iter().any(inside) {
        return Err("it is outside the open folders");
    }
    if forbidden.iter().any(inside) {
        return Err("it is in the app's own folders");
    }
    Ok(())
}

pub struct Extensions {
    pub installed: Vec<Installed>,
    pub node: NodeStatus,
    pub query: String,
    pub results: Vec<MarketItem>,
    pub searching: bool,
    pub market_error: String,
    pub installing: HashSet<String>,
    /// Icons, pages, READMEs and manifests of the registry, kept for the session.
    pub gallery: gallery::Gallery,
    /// The extension whose page is open (publisher or namespace, name), installed or not.
    pub details_of: Option<(String, String)>,
    /// The README and changelog of installed extensions, with where their links start.
    local_texts: HashMap<String, LocalTexts>,
    pub diagnostics: HashMap<String, FileDiagnostics>,
    front_tab: HashMap<String, String>,
    pub revision: u64,
    pub decorations: HashMap<String, FileDecorations>,
    pub notices: Vec<Notice>,
    pub log: VecDeque<String>,
    pub status_items: Vec<(String, String)>,
    pub webviews: Vec<WebviewEntry>,
    pub contexts: HashMap<String, Value>,
    offered: BTreeSet<String>,
    answers: Vec<(String, String)>,
    own_decorations: HashMap<String, FileDecorations>,
    pub hover: Option<Hover>,
    completing: u64,
    dropped_webviews: Vec<String>,
    pub host_running: bool,
    pub views: views::Views,
    pub installed_revision: u64,

    dir: PathBuf,
    data_dir: PathBuf,
    own_dirs: Vec<PathBuf>,
    disabled: HashSet<String>,
    host: Option<Host>,
    folders: Vec<PathBuf>,
    trusted: bool,
    open: Vec<PathBuf>,
    focused: Option<PathBuf>,
    pending: HashMap<u64, Pending>,
    next_request: u64,
    jobs_tx: Sender<Job>,
    jobs_rx: Receiver<Job>,
    waker: Waker,
}

pub fn path_key(path: &str) -> String {
    let unified = path.replace('\\', "/");
    if cfg!(windows) {
        unified.to_lowercase()
    } else {
        unified
    }
}

fn manifest_text(v: &Value, nls: &Value, key: &str) -> String {
    let raw = v.get(key).and_then(Value::as_str).unwrap_or_default();
    match raw.strip_prefix('%').and_then(|r| r.strip_suffix('%')) {
        Some(k) => match nls.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(o) => o.get("message").and_then(Value::as_str).unwrap_or(raw).to_string(),
            None => raw.to_string(),
        },
        None => raw.to_string(),
    }
}

fn localized(value: &Value, nls: &Value) -> String {
    let raw = match value {
        Value::String(s) => s.as_str(),
        other => other.get("value").and_then(Value::as_str).unwrap_or_default(),
    };
    match raw.strip_prefix('%').and_then(|r| r.strip_suffix('%')).and_then(|k| nls.get(k)) {
        Some(Value::String(s)) => s.clone(),
        Some(o) => o.get("message").and_then(Value::as_str).unwrap_or(raw).to_string(),
        None => raw.to_string(),
    }
}

fn manifest_commands(v: &Value, nls: &Value) -> Vec<(String, String)> {
    let hidden: HashSet<&str> = v
        .pointer("/contributes/menus/commandPalette")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|m| m.get("when").and_then(Value::as_str) == Some("false"))
                .filter_map(|m| m.get("command").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    let list = match v.pointer("/contributes/commands") {
        Some(Value::Array(a)) => a.clone(),
        Some(one @ Value::Object(_)) => vec![one.clone()],
        _ => Vec::new(),
    };
    let mut out: Vec<(String, String)> = list
        .iter()
        .filter_map(|c| {
            let id = c.get("command").and_then(Value::as_str)?;
            let title = localized(c.get("title")?, nls);
            if hidden.contains(id) || title.is_empty() {
                return None;
            }
            let category = c.get("category").map(|v| localized(v, nls)).unwrap_or_default();
            Some((id.to_string(), if category.is_empty() { title } else { format!("{category}: {title}") }))
        })
        .collect();
    out.sort_by_key(|a| a.1.to_lowercase());
    out
}

fn manifest_views(v: &Value, nls: &Value) -> Vec<String> {
    let Some(Value::Object(containers)) = v.pointer("/contributes/views") else { return Vec::new() };
    containers
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|view| view.get("name").map(|n| localized(n, nls)))
        .filter(|n| !n.is_empty())
        .collect()
}

fn manifest_containers(v: &Value, nls: &Value) -> Vec<Container> {
    let text = |item: &Value, key: &str| item.get(key).map(|t| localized(t, nls)).unwrap_or_default();
    let views_in = |place: &str| -> Vec<ViewDecl> {
        v.pointer("/contributes/views")
            .and_then(|views| views.get(place))
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|view| {
                        let id = text(view, "id");
                        (!id.is_empty()).then(|| ViewDecl {
                            id,
                            name: text(view, "name"),
                            webview: view.get("type").and_then(Value::as_str) == Some("webview"),
                            when: view.get("when").and_then(Value::as_str).unwrap_or_default().to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    if let Some(Value::Object(bars)) = v.pointer("/contributes/viewsContainers") {
        for item in bars.values().filter_map(Value::as_array).flatten() {
            let id = text(item, "id");
            if id.is_empty() {
                continue;
            }
            let icon = item.get("icon").and_then(Value::as_str).unwrap_or_default();
            let codicon = icon.strip_prefix("$(").and_then(|i| i.strip_suffix(')')).map(str::to_string);
            let views = views_in(&id);
            out.push(Container { id, title: text(item, "title"), codicon, views });
        }
    }
    // Trees an extension adds to VS Code's own containers (the explorer, source control,
    // testing) are put together in one container of the extension's, with no title: its
    // name stands in.
    let Some(Value::Object(places)) = v.pointer("/contributes/views") else { return out };
    let elsewhere: Vec<ViewDecl> = places
        .keys()
        .filter(|place| !out.iter().any(|c| c.id == **place))
        .flat_map(|place| views_in(place))
        .filter(|view| !view.webview)
        .collect();
    if !elsewhere.is_empty() {
        let id =
            format!("{}.{}:views", manifest_text(v, nls, "publisher"), manifest_text(v, nls, "name")).to_lowercase();
        out.push(Container { id, title: String::new(), codicon: None, views: elsewhere });
    }
    out
}

fn rail_items_of(
    installed: &[Installed],
    webviews: &[WebviewEntry],
    context: &dyn Fn(&str) -> Option<Value>,
) -> Vec<RailItem> {
    let mut out = Vec::new();
    for e in installed.iter().filter(|e| e.enabled && e.has_code) {
        let before = out.len();
        let mut trees = Vec::new();
        for container in &e.containers {
            let shown = container.views.iter().find(|v| v.webview && when::eval(&v.when, context));
            let tree = container.views.iter().any(|v| !v.webview && when::eval(&v.when, context));
            if shown.is_none() && tree {
                let title = [&container.title, &e.display_name].into_iter().find(|t| !t.is_empty());
                trees.push(RailItem {
                    view: format!("{TREE}{}", container.id),
                    extension: e.id.clone(),
                    title: title.cloned().unwrap_or_default(),
                    codicon: container.codicon.clone(),
                    tabs: 0,
                });
            }
            if let Some(view) = shown {
                let title = [&container.title, &view.name, &e.display_name].into_iter().find(|t| !t.is_empty());
                out.push(RailItem {
                    view: view.id.clone(),
                    extension: e.id.clone(),
                    title: title.cloned().unwrap_or_default(),
                    codicon: container.codicon.clone(),
                    tabs: 0,
                });
            }
        }
        let same_title = out.len() > before + 1 && out[before + 1..].iter().all(|item| item.title == out[before].title);
        if same_title {
            for item in &mut out[before..] {
                item.title = format!("{} · {}", item.title, view_words(&item.view));
            }
        }
        let tabs = webviews.iter().filter(|w| w.tab && w.extension == e.id).count();
        if out.len() > before && tabs > 0 {
            out.push(RailItem {
                view: format!("{TABS}{}", e.id),
                extension: e.id.clone(),
                title: format!("{} · what is open ({tabs})", e.display_name),
                codicon: None,
                tabs,
            });
        }
        if out.len() == before {
            if let Some(w) = webviews.iter().find(|w| w.extension == e.id) {
                out.push(RailItem {
                    view: w.key.clone(),
                    extension: e.id.clone(),
                    title: w.title.clone(),
                    codicon: None,
                    tabs: 0,
                });
            }
        }
        out.extend(trees);
    }
    out
}

fn read_installed(dir: &Path, disabled: &HashSet<String>) -> Option<Installed> {
    let read = |name: &str| -> Option<Value> {
        serde_json::from_str(std::fs::read_to_string(dir.join(name)).ok()?.trim_start_matches('\u{feff}')).ok()
    };
    let v = read("package.json")?;
    let nls = read("package.nls.json").unwrap_or(Value::Null);
    let publisher = manifest_text(&v, &nls, "publisher");
    let name = manifest_text(&v, &nls, "name");
    if publisher.is_empty() || name.is_empty() {
        return None;
    }
    let id = format!("{publisher}.{name}").to_lowercase();
    let display_name = Some(manifest_text(&v, &nls, "displayName")).filter(|s| !s.is_empty()).unwrap_or(name);
    let enabled = !disabled.contains(&id);
    let has_code = v.get("main").and_then(Value::as_str).is_some_and(|m| !m.is_empty());
    let theme_key = format!("{}{id}:", crate::themes::FROM_EXTENSION);
    let untrusted = manifest_untrusted(&v, &id, has_code);
    Some(Installed {
        id,
        display_name,
        publisher,
        version: manifest_text(&v, &nls, "version"),
        description: manifest_text(&v, &nls, "description"),
        dir: dir.to_path_buf(),
        has_code,
        enabled,
        status: match (enabled, has_code) {
            (false, _) => Status::Disabled,
            (true, false) => Status::Declarative,
            (true, true) => Status::Idle,
        },
        error: String::new(),
        activation_ms: 0,
        unsupported: Vec::new(),
        manifest: Arc::new(gallery::Manifest::new(v.clone())),
        untrusted,
        icon: v.get("icon").and_then(Value::as_str).map(|i| dir.join(i)).filter(|p| p.is_file()),
        commands: manifest_commands(&v, &nls),
        views: manifest_views(&v, &nls),
        containers: manifest_containers(&v, &nls),
        menus: views::manifest_menus(&v, &nls),
        color_themes: v
            .pointer("/contributes/themes")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|t| {
                        let file = dir.join(t.get("path").and_then(Value::as_str)?);
                        let named =
                            t.get("label").or(t.get("id")).map(|l| localized(l, &nls)).filter(|l| !l.is_empty());
                        let label = named.or_else(|| Some(file.file_stem()?.to_string_lossy().into_owned()))?;
                        let light =
                            t.get("uiTheme").and_then(Value::as_str).is_some_and(|ui| ui == "vs" || ui == "hc-light");
                        Some(ColorThemeDecl { key: format!("{theme_key}{label}"), label, file, light })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        icon_themes: v
            .pointer("/contributes/iconThemes")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|t| {
                        let id = t.get("id").and_then(Value::as_str)?;
                        let file = dir.join(t.get("path").and_then(Value::as_str)?);
                        let label = t.get("label").map(|l| localized(l, &nls)).filter(|l| !l.is_empty());
                        Some(IconThemeDecl { id: id.to_string(), label: label.unwrap_or_else(|| id.to_string()), file })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        languages: manifest_languages(&v),
        grammars: manifest_grammars(&v, dir),
    })
}

type LocalText = Option<(String, String)>;
type LocalTexts = (LocalText, LocalText);

/// The README and the changelog in an extension's folder, each with where its relative
/// links start.
fn local_texts(dir: &Path) -> LocalTexts {
    let files: Vec<PathBuf> =
        std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    let read = |stem: &str| -> LocalText {
        let found = files.iter().find(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
            p.is_file()
                && (name == stem
                    || name.strip_prefix(stem).is_some_and(|rest| [".md", ".markdown", ".txt"].contains(&rest)))
        })?;
        let text = std::fs::read_to_string(found).ok()?;
        Some((text, format!("{}/", crate::media::file_url(dir))))
    };
    (read("readme"), read("changelog"))
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|s| Some(s.as_str()?.to_string())).collect())
        .unwrap_or_default()
}

fn manifest_languages(v: &Value) -> Vec<LanguageDecl> {
    let Some(list) = v.pointer("/contributes/languages").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|l| {
            Some(LanguageDecl {
                id: l.get("id")?.as_str()?.to_string(),
                extensions: strings(l.get("extensions")).into_iter().map(|e| e.to_lowercase()).collect(),
                filenames: strings(l.get("filenames")).into_iter().map(|f| f.to_lowercase()).collect(),
            })
        })
        .collect()
}

fn manifest_grammars(v: &Value, dir: &Path) -> Vec<GrammarDecl> {
    let Some(list) = v.pointer("/contributes/grammars").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|g| {
            Some(GrammarDecl {
                language: g.get("language").and_then(Value::as_str).map(str::to_string),
                scope_name: g.get("scopeName")?.as_str()?.to_string(),
                file: dir.join(g.get("path")?.as_str()?),
                inject_to: strings(g.get("injectTo")),
            })
        })
        .collect()
}

impl Extensions {
    pub fn new(data_dir: PathBuf, waker: Waker) -> Self {
        let dir = data_dir.join("extensions");
        let disabled: HashSet<String> = std::fs::read_to_string(dir.join("disabled.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let (jobs_tx, jobs_rx) = mpsc::channel();
        let mut me = Extensions {
            installed: Vec::new(),
            node: NodeStatus::Checking,
            query: String::new(),
            results: Vec::new(),
            searching: false,
            market_error: String::new(),
            installing: HashSet::new(),
            gallery: gallery::Gallery::new(waker.clone()),
            details_of: None,
            local_texts: HashMap::new(),
            diagnostics: HashMap::new(),
            front_tab: HashMap::new(),
            revision: 0,
            decorations: HashMap::new(),
            notices: Vec::new(),
            log: VecDeque::new(),
            status_items: Vec::new(),
            webviews: Vec::new(),
            contexts: HashMap::new(),
            offered: BTreeSet::new(),
            answers: Vec::new(),
            own_decorations: HashMap::new(),
            hover: None,
            completing: 0,
            dropped_webviews: Vec::new(),
            host_running: false,
            views: views::Views::default(),
            installed_revision: 0,
            dir,
            own_dirs: [
                Some(data_dir.clone()),
                std::env::current_exe().ok().and_then(|e| Some(e.parent()?.to_path_buf())),
            ]
            .into_iter()
            .flatten()
            .collect(),
            data_dir,
            disabled,
            host: None,
            folders: Vec::new(),
            trusted: true,
            open: Vec::new(),
            focused: None,
            pending: HashMap::new(),
            next_request: 1,
            jobs_tx,
            jobs_rx,
            waker,
        };
        me.reload_installed();
        me.job(|| Job::Node(host::detect_node()));
        me
    }

    fn job(&self, work: impl FnOnce() -> Job + Send + 'static) {
        let (tx, waker) = (self.jobs_tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work());
            waker.wake();
        });
    }

    fn reload_installed(&mut self) {
        self.installed_revision += 1;
        self.local_texts.clear();
        self.installed =
            market::installed_dirs(&self.dir).iter().filter_map(|d| read_installed(d, &self.disabled)).collect();
        self.installed.sort_by_key(|e| e.display_name.to_lowercase());
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.installed.iter().any(|e| e.id == id)
    }

    pub fn notice(&mut self, level: Level, source: &str, text: impl Into<String>) {
        let text = text.into();
        self.log_line(&format!("[{source}] {text}"));
        self.notices.push(Notice {
            level,
            source: source.to_string(),
            text,
            at: Instant::now(),
            actions: Vec::new(),
            reply_to: None,
            own: None,
        });
        while self.notices.iter().filter(|n| !n.is_question()).count() > 5 {
            let oldest = self.notices.iter().position(|n| !n.is_question()).unwrap_or(0);
            self.notices.remove(oldest);
        }
    }

    pub fn ask(&mut self, source: &str, text: impl Into<String>, choices: &[&str], about: &str) {
        let text = text.into();
        self.log_line(&format!("[{source}] {text}"));
        self.notices.retain(|n| n.own.as_deref() != Some(about));
        self.notices.push(Notice {
            level: Level::Info,
            source: source.to_string(),
            text,
            at: Instant::now(),
            actions: choices.iter().map(|c| c.to_string()).collect(),
            reply_to: None,
            own: Some(about.to_string()),
        });
    }

    pub fn asking(&self, about: &str) -> bool {
        self.notices.iter().any(|n| n.own.as_deref() == Some(about))
    }

    pub fn take_answers(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.answers)
    }

    pub fn answer_with(&mut self, choice: &str) -> bool {
        match self.notices.iter().position(|n| n.actions.iter().any(|a| a == choice)) {
            Some(index) => {
                self.answer_notice(index, Some(choice));
                true
            }
            None => false,
        }
    }

    fn log_line(&mut self, line: &str) {
        let mut short: String = line.chars().take(600).collect();
        if short.len() < line.len() {
            short.push('…');
        }
        log::debug!(target: "extensions", "{short}");
        self.log.push_back(short);
        if self.log.len() > MAX_LOG {
            self.log.pop_front();
        }
    }

    pub fn search(&mut self) {
        let query = self.query.clone();
        self.searching = true;
        self.market_error.clear();
        self.gallery.forget_failures();
        self.job(move || {
            let found = market::search(&query);
            Job::Search(query, found)
        });
    }

    /// Opens the page of an extension of the registry or installed: what the card of
    /// `Panel::Extension` shows.
    pub fn open_details(&mut self, namespace: &str, name: &str) {
        self.details_of = Some((namespace.to_string(), name.to_string()));
    }

    /// Opens the page of an installed extension, by id.
    pub fn open_installed(&mut self, id: &str) -> bool {
        let Some((namespace, name)) = self.installed.iter().find(|e| e.id == id).map(Installed::namespace_and_name)
        else {
            return false;
        };
        self.open_details(&namespace, &name);
        true
    }

    pub fn details_id(&self) -> Option<String> {
        self.details_of.as_ref().map(|(namespace, name)| format!("{namespace}.{name}").to_lowercase())
    }

    /// Everything the open page shows, asking the registry for what is missing. An installed
    /// extension's README, changelog and manifest are those of its folder.
    pub fn details_view(&mut self) -> Option<details::View> {
        use gallery::Got;
        let (namespace, name) = self.details_of.clone()?;
        let id = format!("{namespace}.{name}").to_lowercase();
        let market = self.gallery.details(&namespace, &name);
        let installed = self.installed.iter().find(|e| e.id == id);
        let (readme_here, changelog_here) = match installed {
            Some(e) => self.local_texts.entry(e.id.clone()).or_insert_with(|| local_texts(&e.dir)).clone(),
            None => (None, None),
        };
        let installed_version = installed.map(|e| e.version.clone());
        let manifest_here = installed.map(|e| e.manifest.clone());
        let from_registry = |gallery: &mut gallery::Gallery, pick: fn(&market::MarketDetails) -> &String| match &market
        {
            Got::Ready(d) => (gallery.text(pick(d)), details::folder_of(pick(d))),
            Got::Loading => (Got::Loading, String::new()),
            Got::Failed(why) if details::not_found(why) => (Got::Failed("unknown".into()), String::new()),
            Got::Failed(why) => (Got::Failed(why.clone()), String::new()),
        };
        // An installed one's own README, whose pictures beside it are looked for where the
        // registry keeps its files (the folder's own are not served to pages).
        let registry_folder = |pick: fn(&market::MarketDetails) -> &String| {
            market.ready().map(|d| details::folder_of(pick(d))).filter(|folder| !folder.is_empty())
        };
        let (readme, readme_base) = match readme_here {
            Some((text, base)) => (Got::Ready(Arc::new(text)), registry_folder(|d| &d.readme).unwrap_or(base)),
            None => from_registry(&mut self.gallery, |d| &d.readme),
        };
        let (changelog, changelog_base) = match changelog_here {
            Some((text, base)) => (Got::Ready(Arc::new(text)), registry_folder(|d| &d.changelog).unwrap_or(base)),
            None => from_registry(&mut self.gallery, |d| &d.changelog),
        };
        let manifest = match (manifest_here, &market) {
            (Some(m), _) => Got::Ready(m),
            (None, Got::Ready(d)) => self.gallery.manifest_of(&namespace, &name, &d.item.version),
            (None, Got::Loading) => Got::Loading,
            (None, Got::Failed(why)) => Got::Failed(why.clone()),
        };
        let first_released = match &market {
            Got::Ready(_) => self.gallery.first_released(&namespace, &name),
            _ => Got::Loading,
        };
        Some(details::View {
            id,
            namespace,
            name,
            market,
            manifest,
            readme,
            readme_base,
            changelog,
            changelog_base,
            first_released,
            installed: installed_version,
        })
    }

    pub fn install_defaults(&mut self) {
        self.offered = std::fs::read_to_string(self.dir.join("defaults.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let before = self.offered.len();
        let installed: HashSet<&str> = self.installed.iter().map(|e| e.id.as_str()).collect();
        let wanted = defaults_to_install(DEFAULTS, &mut self.offered, &|id| installed.contains(id));
        if self.offered.len() != before {
            self.save_offered();
        }
        if wanted.is_empty() {
            return;
        }
        let names: Vec<&str> = wanted.iter().map(|(_, name)| name.as_str()).collect();
        self.notice(Level::Info, "Extensions", format!("getting the ones the app comes with: {}", names.join(", ")));
        for (namespace, name) in &wanted {
            self.install(namespace, name);
        }
    }

    fn save_offered(&self) {
        let _ = std::fs::create_dir_all(&self.dir);
        let _ =
            std::fs::write(self.dir.join("defaults.json"), serde_json::to_string(&self.offered).unwrap_or_default());
    }

    pub fn install(&mut self, namespace: &str, name: &str) {
        let id = format!("{namespace}.{name}").to_lowercase();
        if !self.installing.insert(id.clone()) {
            return;
        }
        let (namespace, name, dir) = (namespace.to_string(), name.to_string(), self.dir.clone());
        self.job(move || Job::Installed(id, market::install(&namespace, &name, &dir)));
    }

    pub fn install_vsix(&mut self, file: PathBuf) {
        let label = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !self.installing.insert(label.clone()) {
            return;
        }
        let dir = self.dir.clone();
        self.job(move || Job::Installed(label, market::install_vsix(&file, &dir)));
    }

    pub fn uninstall(&mut self, id: &str) {
        let Some(at) = self.installed.iter().position(|e| e.id == id) else { return };
        self.stop_host();
        let dir = self.installed[at].dir.clone();
        if let Err(e) = remove_dir_retrying(&dir) {
            self.notice(Level::Error, id, format!("could not be removed: {e}"));
        }
        self.reload_installed();
        self.start_host();
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.disabled.remove(id);
        } else {
            self.disabled.insert(id.to_string());
        }
        let _ = std::fs::create_dir_all(&self.dir);
        let list: Vec<&String> = self.disabled.iter().collect();
        let _ = std::fs::write(self.dir.join("disabled.json"), serde_json::to_string(&list).unwrap_or_default());
        self.restart_host();
    }

    fn wants_host(&self) -> bool {
        matches!(self.node, NodeStatus::Ready(_))
            && !self.folders.is_empty()
            && self.installed.iter().any(|e| e.has_code && runs_in(e, self.trusted))
    }

    pub fn restricted_mode(&self) -> bool {
        !self.trusted && !self.folders.is_empty()
    }

    pub fn held_back(&self) -> Vec<String> {
        held_back(&self.installed).map(|e| e.display_name.clone()).collect()
    }

    fn mark_restricted(&mut self) {
        let trusted = self.trusted || self.folders.is_empty();
        for e in &mut self.installed {
            match (runs_in(e, trusted), e.status) {
                (false, Status::Idle) => e.status = Status::Restricted,
                (true, Status::Restricted) => e.status = Status::Idle,
                _ => {}
            }
        }
    }

    fn stop_host(&mut self) {
        if let Some(host) = self.host.take() {
            host.stop();
        }
        self.host_running = false;
        self.pending.clear();
        self.status_items.clear();
        self.dropped_webviews.extend(self.webviews.drain(..).filter_map(|w| w.handle));
        self.contexts.clear();
        self.decorations.clear();
        self.clear_views();
        if !self.diagnostics.is_empty() {
            self.diagnostics.clear();
            self.revision += 1;
        }
    }

    fn start_host(&mut self) {
        self.mark_restricted();
        if self.host.is_some() || !self.wants_host() {
            return;
        }
        match Host::start(&self.data_dir.join("exthost"), self.waker.clone()) {
            Ok(mut host) => {
                let trusted = self.trusted;
                let extensions: Vec<Value> = self
                    .installed
                    .iter()
                    .filter(|e| runs_in(e, trusted))
                    .map(|e| json!({ "id": e.id, "dir": e.dir }))
                    .collect();
                let restricted = if trusted { Vec::new() } else { restricted_keys(&self.installed) };
                if !trusted {
                    log::info!("extensions run in restricted mode: the folders are not trusted");
                }
                host.send(&json!({
                    "type": "init",
                    "tabWidth": canvas_core::TAB_WIDTH,
                    "storageDir": self.data_dir.join("extension-storage"),
                    "settingsFile": self.data_dir.join("settings.json"),
                    "folders": self.folders,
                    "extensions": extensions,
                    "trusted": trusted,
                    "restricted": restricted,
                }));
                for path in &self.open {
                    host.send(&json!({ "type": "open", "path": path, "focus": self.focused.as_ref() == Some(path) }));
                }
                self.host = Some(host);
                self.host_running = true;
            }
            Err(e) => self.notice(Level::Error, "extensions", format!("the extension host did not start: {e}")),
        }
    }

    fn restart_host(&mut self) {
        self.stop_host();
        self.reload_installed();
        self.start_host();
    }

    fn send(&mut self, message: Value) {
        if let Some(host) = self.host.as_mut() {
            host.send(&message);
        }
    }

    pub fn set_folders(&mut self, folders: Vec<PathBuf>, trusted: bool) {
        let trusted = trusted || folders.is_empty();
        if folders == self.folders && trusted == self.trusted {
            return;
        }
        if trusted != self.trusted {
            log::info!("the open folders are {}", if trusted { "trusted" } else { "not trusted" });
            self.trusted = trusted;
            self.folders = folders;
            self.restart_host();
            return;
        }
        self.folders = folders;
        if self.folders.is_empty() {
            self.stop_host();
        } else if self.host.is_some() {
            let folders = self.folders.clone();
            self.send(json!({ "type": "folders", "folders": folders }));
        } else {
            self.start_host();
        }
    }

    pub fn show_documents(&mut self, visible: &[PathBuf], focus: Option<&Path>) {
        for path in visible {
            match self.open.iter().position(|p| p == path) {
                Some(at) => {
                    let p = self.open.remove(at);
                    self.open.push(p);
                }
                None => {
                    self.open.push(path.clone());
                    self.send(json!({ "type": "open", "path": path, "focus": false }));
                }
            }
        }
        self.lenses_for(visible);
        while self.open.len() > MAX_OPEN {
            let old = self.open.remove(0);
            self.decorations.remove(&path_key(&old.to_string_lossy()));
            self.forget_lenses(&old);
            self.send(json!({ "type": "close", "path": old }));
        }
        if let Some(f) = focus {
            if self.focused.as_deref() != Some(f) && self.open.iter().any(|p| p == f) {
                self.focused = Some(f.to_path_buf());
                self.send(json!({ "type": "open", "path": f, "focus": true }));
            }
        }
    }

    pub fn file_edited(&mut self, path: &Path, text: &str) {
        if !self.open.iter().any(|p| p == path) {
            self.open.push(path.to_path_buf());
        }
        self.send(json!({ "type": "edit", "path": path, "text": text }));
        self.lenses_edited(path);
    }

    pub fn file_changed(&mut self, path: &Path) {
        self.send(json!({ "type": "changed", "path": path }));
        self.lenses_stale(path);
    }

    pub fn format(&mut self, path: &Path) {
        if self.host.is_none() {
            let why = match &self.node {
                NodeStatus::Missing(_) => "Node.js is needed to run extensions and was not found",
                _ if !self.installed.iter().any(|e| e.enabled && e.has_code) => {
                    "no formatter is installed: install one (Prettier, for example) from the Extensions panel"
                }
                _ => "extensions are still starting",
            };
            self.notice(Level::Warn, "Format", why);
            return;
        }
        if !self.open.iter().any(|p| p == path) {
            self.open.push(path.to_path_buf());
        }
        self.focused = Some(path.to_path_buf());
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, Pending::Format(path.to_path_buf(), false));
        self.send(json!({ "type": "request", "id": id, "method": "format", "params": { "path": path } }));
    }

    pub fn format_on_save(&mut self, path: &Path) {
        if self.host.is_none() {
            return;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, Pending::Format(path.to_path_buf(), true));
        self.send(
            json!({ "type": "request", "id": id, "method": "format", "params": { "path": path, "onSave": true } }),
        );
    }

    pub fn answer_notice(&mut self, index: usize, action: Option<&str>) {
        if index >= self.notices.len() {
            return;
        }
        let notice = self.notices.remove(index);
        if let Some(id) = notice.reply_to {
            self.send(json!({ "type": "messageReply", "id": id, "action": action }));
        }
        if let (Some(about), Some(choice)) = (notice.own, action) {
            self.answers.push((about, choice.to_string()));
        }
    }

    pub fn request_hover(&mut self, path: &Path, line: u32, col: u32) {
        if self.host.is_none() {
            return;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, Pending::Hover(path.to_path_buf(), line, col));
        self.send(json!({ "type": "request", "id": id, "method": "hover", "params": { "path": path, "line": line, "col": col } }));
    }

    pub fn request_completion(&mut self, path: &Path, line: u32, col: u32, trigger: Option<char>) {
        if self.host.is_none() {
            return;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.completing = id;
        self.pending.insert(id, Pending::Complete(path.to_path_buf(), line, col));
        let params = json!({ "path": path, "line": line, "col": col, "trigger": trigger.map(String::from) });
        self.send(json!({ "type": "request", "id": id, "method": "complete", "params": params }));
    }

    pub fn request_completion_extra(&mut self, path: &Path, serial: u64, index: usize) {
        if self.host.is_none() {
            return;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, Pending::CompletionExtra(path.to_path_buf()));
        let params = json!({ "serial": serial, "index": index });
        self.send(json!({ "type": "request", "id": id, "method": "completionExtra", "params": params }));
    }

    /// Asks the language servers what the name defined at the place is related to: where it
    /// is used, the calls it makes and what implements it. Each answer comes on its own, as
    /// an `Event::Relations`.
    pub fn request_relations(&mut self, path: &Path, line: u32, col: u32, page: u32, of: (u32, u32)) {
        if self.host.is_none() {
            return;
        }
        let at = json!({ "path": path, "line": line, "col": col, "deadline": RELATIONS_WAIT_MS });
        let asks = [
            (Asked::Uses, "locations", Some("Reference")),
            (Asked::Calls, "outgoingCalls", None),
            (Asked::Implementations, "locations", Some("Implementation")),
        ];
        for (asked, method, kind) in asks {
            let id = self.next_request;
            self.next_request += 1;
            self.pending.insert(id, Pending::Relations(page, of, asked));
            let mut params = at.clone();
            if let Some(kind) = kind {
                params["kind"] = json!(kind);
            }
            self.send(json!({ "type": "request", "id": id, "method": method, "params": params }));
        }
    }

    pub fn request_definition(&mut self, path: &Path, line: u32, col: u32) -> bool {
        if self.host.is_none() {
            return false;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, Pending::Definition(path.to_path_buf()));
        let params = json!({ "path": path, "line": line, "col": col, "kind": "Definition" });
        self.send(json!({ "type": "request", "id": id, "method": "locations", "params": params }));
        true
    }

    pub fn extensions_dir(&self) -> &Path {
        &self.dir
    }

    pub fn rail_items(&self) -> Vec<RailItem> {
        rail_items_of(&self.installed, &self.webviews, &|key| self.contexts.get(key).cloned())
    }

    pub fn color_themes(&self) -> impl Iterator<Item = &ColorThemeDecl> {
        self.installed.iter().filter(|e| e.enabled).flat_map(|e| e.color_themes.iter())
    }

    pub fn languages(&self) -> impl Iterator<Item = &LanguageDecl> {
        self.installed.iter().filter(|e| e.enabled).flat_map(|e| e.languages.iter())
    }

    pub fn grammars(&self) -> impl Iterator<Item = &GrammarDecl> {
        self.installed.iter().filter(|e| e.enabled).flat_map(|e| e.grammars.iter())
    }

    pub fn icon_themes(&self) -> impl Iterator<Item = &IconThemeDecl> {
        self.installed.iter().filter(|e| e.enabled).flat_map(|e| e.icon_themes.iter())
    }

    pub fn icon_theme(&self, wanted: &str) -> Option<&IconThemeDecl> {
        if wanted == "none" {
            return None;
        }
        self.icon_themes()
            .find(|t| t.id == wanted)
            .or_else(|| self.icon_themes().find(|t| t.id == "symbols"))
            .or_else(|| self.icon_themes().next())
    }

    pub fn ensure_view(&mut self, key: &str) -> bool {
        if self.webviews.iter().any(|w| w.key == key) {
            return true;
        }
        let declared = self.installed.iter().filter(|e| e.enabled && e.has_code).find_map(|e| {
            let container = e.containers.iter().find(|c| c.views.iter().any(|v| v.webview && v.id == key))?;
            let view = container.views.iter().find(|v| v.id == key)?;
            let title = [&container.title, &view.name, &e.display_name].into_iter().find(|t| !t.is_empty())?;
            Some((e.id.clone(), title.clone()))
        });
        let Some((extension, title)) = declared else { return false };
        self.webviews.push(WebviewEntry {
            key: key.to_string(),
            extension,
            title,
            handle: None,
            resolving: false,
            failed: false,
            tab: false,
        });
        true
    }

    fn has_own_views(&self, extension: &str) -> bool {
        let installed = self.installed.iter().find(|e| e.id == extension);
        installed.is_some_and(|e| e.containers.iter().any(|c| c.views.iter().any(|v| v.webview)))
    }

    pub fn panel_key(&self, handle: &str) -> String {
        match self.webviews.iter().find(|w| w.tab && w.key == handle) {
            Some(w) if self.has_own_views(&w.extension) => format!("{TABS}{}", w.extension),
            _ => handle.to_string(),
        }
    }

    pub fn tabs_of<'a>(&'a self, extension: &'a str) -> impl Iterator<Item = &'a WebviewEntry> + 'a {
        self.webviews.iter().filter(move |w| w.tab && w.extension == extension)
    }

    pub fn shown(&self, view: &str) -> Option<&WebviewEntry> {
        let Some(extension) = view.strip_prefix(TABS) else { return self.webviews.iter().find(|w| w.key == view) };
        let front = self.front_tab.get(extension);
        let tabs = self.webviews.iter().filter(|w| w.tab && w.extension == extension);
        tabs.clone().find(|w| Some(&w.key) == front).or_else(|| tabs.clone().next_back())
    }

    pub fn show_tab(&mut self, extension: &str, handle: &str) {
        self.front_tab.insert(extension.to_string(), handle.to_string());
    }

    pub fn close_tab(&mut self, handle: &str) {
        self.send(json!({ "type": "closeWebview", "handle": handle }));
    }

    pub fn resolve_webview(&mut self, key: &str) {
        if self.host.is_none() {
            return;
        }
        let Some(entry) = self.webviews.iter_mut().find(|w| w.key == key) else { return };
        if entry.handle.is_some() || entry.resolving || entry.failed {
            return;
        }
        entry.resolving = true;
        let request = self.next_request;
        self.next_request += 1;
        self.pending.insert(request, Pending::Webview(key.to_string()));
        self.send(json!({ "type": "request", "id": request, "method": "resolveWebview", "params": { "view": key } }));
    }

    pub fn webview_message(&mut self, handle: &str, message: Value) {
        self.send(json!({ "type": "webviewMessage", "handle": handle, "message": message }));
    }

    pub fn run_command(&mut self, id: &str, title: &str) {
        if self.host.is_none() {
            self.notice(Level::Warn, title, "extensions are not running (open a folder first)");
            return;
        }
        let request = self.next_request;
        self.next_request += 1;
        self.pending.insert(request, Pending::Command(title.to_string()));
        self.send(json!({ "type": "request", "id": request, "method": "command", "params": { "command": id } }));
    }

    pub fn shutdown(&mut self) {
        if let Some(host) = self.host.take() {
            host.stop_and_wait(30);
        }
        self.stop_host();
    }

    pub fn poll(&mut self) -> (Vec<Event>, bool) {
        let mut events: Vec<Event> = self.dropped_webviews.drain(..).map(Event::WebviewClosed).collect();
        let mut changed = !events.is_empty() | self.gallery.poll();

        while let Ok(job) = self.jobs_rx.try_recv() {
            changed = true;
            match job {
                Job::Node(Ok(version)) => {
                    self.node = NodeStatus::Ready(version);
                    self.start_host();
                }
                Job::Node(Err(e)) => self.node = NodeStatus::Missing(e),
                Job::Search(query, found) => {
                    if query == self.query {
                        self.searching = false;
                        match found {
                            Ok(items) => self.results = items,
                            Err(e) => self.market_error = e,
                        }
                    }
                }
                Job::Installed(label, done) => {
                    self.installing.remove(&label);
                    match done {
                        Ok(dir) => {
                            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            self.notice(Level::Info, "Extensions", format!("installed {name}"));
                            let ours = DEFAULTS.iter().any(|(ns, n)| format!("{ns}.{n}").to_lowercase() == label);
                            if ours && self.offered.insert(label.clone()) {
                                self.save_offered();
                            }
                            self.restart_host();
                        }
                        Err(e) => self.notice(Level::Error, "Extensions", format!("{label} was not installed: {e}")),
                    }
                }
            }
        }

        let mut incoming = Vec::new();
        if let Some(host) = &self.host {
            while let Ok(event) = host.rx.try_recv() {
                incoming.push(event);
            }
        }
        for event in incoming {
            changed = true;
            match event {
                HostEvent::Message(m) => self.on_message(&m, &mut events),
                HostEvent::Stderr(line) => self.log_line(&format!("[host] {line}")),
                HostEvent::Exited => {
                    if self.host.take().is_some() {
                        self.host_running = false;
                        self.notice(Level::Error, "Extensions", "the extension host stopped unexpectedly");
                        for e in &mut self.installed {
                            if matches!(e.status, Status::Active | Status::Activating) {
                                e.status = Status::Idle;
                            }
                        }
                    }
                }
            }
        }

        let before = self.notices.len();
        let expired: Vec<usize> = (0..self.notices.len())
            .rev()
            .filter(|&i| {
                self.notices[i].at.elapsed().as_secs_f32()
                    > match (&self.notices[i].own, self.notices[i].reply_to) {
                        (Some(_), _) => 3600.0,
                        (None, Some(_)) => 300.0,
                        (None, None) => 9.0,
                    }
            })
            .collect();
        for i in expired {
            self.answer_notice(i, None);
        }
        changed |= before != self.notices.len();
        (events, changed)
    }

    fn on_message(&mut self, m: &Value, events: &mut Vec<Event>) {
        let s = |key: &str| m.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        if self.on_view_message(&s("type"), m, events) {
            return;
        }
        match s("type").as_str() {
            "ready" => self.log_line("[host] ready"),
            "extension" => {
                let (id, status) = (s("id"), s("status"));
                if let Some(e) = self.installed.iter_mut().find(|e| e.id == id) {
                    e.status = match status.as_str() {
                        "activating" => Status::Activating,
                        "active" => Status::Active,
                        "failed" => Status::Failed,
                        "declarative" => Status::Declarative,
                        _ => e.status,
                    };
                    e.error = s("error").lines().next().unwrap_or_default().to_string();
                    match e.status {
                        Status::Failed => log::warn!("extension {id} failed to start: {}", e.error),
                        Status::Active => log::info!("extension {id} started"),
                        _ => {}
                    }
                    e.activation_ms = m.get("ms").and_then(Value::as_u64).unwrap_or(0) as u32;
                }
            }
            "unsupported" => {
                let (id, api) = (s("extension"), clip(&s("api"), MAX_SHORT));
                if let Some(e) = self.installed.iter_mut().find(|e| e.id == id) {
                    if !e.unsupported.contains(&api) && e.unsupported.len() < MAX_UNSUPPORTED {
                        e.unsupported.push(api.clone());
                    }
                }
                log::info!("extension {id} asked for vscode.{api}, which is not available yet");
                self.log_line(&format!("[{id}] asked for vscode.{api}, which is not available yet"));
            }
            "diagnostics" => {
                let path = s("path").replace('\\', "/");
                let items: Vec<Diagnostic> = m
                    .get("items")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().take(MAX_ITEMS).map(diagnostic).collect())
                    .unwrap_or_default();
                let key = path_key(&path);
                if !items.is_empty() && !self.diagnostics.contains_key(&key) && self.diagnostics.len() >= MAX_FILES {
                    return;
                }
                if items.is_empty() {
                    self.diagnostics.remove(&key);
                } else {
                    let count = |s: Severity| items.iter().filter(|d| d.severity == s).count() as u32;
                    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
                    self.diagnostics.insert(key, FileDiagnostics { path, items, errors, warnings });
                }
                self.revision += 1;
            }
            "context" => {
                let key = s("key");
                match m.get("value") {
                    None | Some(Value::Null) => {
                        self.contexts.remove(&key);
                    }
                    Some(value) => {
                        let fits = key.len() <= MAX_SHORT && value.to_string().len() <= MAX_CONTEXT_VALUE;
                        if fits && (self.contexts.contains_key(&key) || self.contexts.len() < MAX_CONTEXTS) {
                            self.contexts.insert(key, value.clone());
                        }
                    }
                }
            }
            "webviewView" => {
                let key = s("view");
                if !self.webviews.iter().any(|w| w.key == key) && self.webviews.len() < MAX_WEBVIEWS {
                    self.webviews.push(WebviewEntry {
                        key,
                        extension: s("extension"),
                        title: clip(&s("title"), MAX_SHORT),
                        handle: None,
                        resolving: false,
                        failed: false,
                        tab: false,
                    });
                }
            }
            "webviewPanel" => {
                let handle = s("handle");
                if self.webviews.len() >= MAX_WEBVIEWS && !self.webviews.iter().any(|w| w.key == handle) {
                    self.send(json!({ "type": "closeWebview", "handle": handle }));
                    return;
                }
                match self.webviews.iter_mut().find(|w| w.key == handle) {
                    Some(entry) => entry.title = clip(&s("title"), MAX_SHORT),
                    None => self.webviews.push(WebviewEntry {
                        key: handle.clone(),
                        extension: s("extension"),
                        title: clip(&s("title"), MAX_SHORT),
                        handle: Some(handle.clone()),
                        resolving: false,
                        failed: false,
                        tab: true,
                    }),
                }
                if m.get("reveal").and_then(Value::as_bool).unwrap_or(false) {
                    self.front_tab.insert(s("extension"), handle.clone());
                    events.push(Event::ShowWebview(self.panel_key(&handle)));
                }
            }
            "webviewHtml" => {
                let html = s("html");
                if html.len() <= MAX_HTML {
                    events.push(Event::WebviewHtml { handle: s("handle"), html });
                } else {
                    log::warn!("a page of {} bytes for {} was not shown", html.len(), s("handle"));
                }
            }
            "webviewPost" => events.push(Event::WebviewPost {
                handle: s("handle"),
                message: m.get("message").cloned().unwrap_or(Value::Null),
            }),
            "webviewClosed" => {
                let handle = s("handle");
                self.webviews.retain(|w| w.handle.as_deref() != Some(handle.as_str()));
                self.front_tab.retain(|_, front| *front != handle);
                events.push(Event::WebviewClosed(handle));
            }
            "decorations" => {
                let path = s("path").replace('\\', "/");
                let key = path_key(&path);
                let items: Vec<Decoration> = m
                    .get("items")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().take(MAX_ITEMS).map(decoration).collect())
                    .unwrap_or_default();
                let room = match self.decorations.get(&key) {
                    Some(file) => file.by_type.contains_key(&s("key")) || file.by_type.len() < MAX_KINDS,
                    None => self.decorations.len() < MAX_FILES,
                };
                if !items.is_empty() && !room {
                    return;
                }
                let file = self
                    .decorations
                    .entry(key.clone())
                    .or_insert_with(|| FileDecorations { path, by_type: HashMap::new() });
                if items.is_empty() {
                    file.by_type.remove(&s("key"));
                } else {
                    file.by_type.insert(s("key"), items);
                }
                if file.by_type.is_empty() {
                    self.decorations.remove(&key);
                }
            }
            "log" => {
                let line = format!("[{}] {}", s("source"), s("text"));
                self.log_line(&line);
            }
            "message" => {
                let level = match s("level").as_str() {
                    "error" => Level::Error,
                    "warn" => Level::Warn,
                    _ => Level::Info,
                };
                let source = self.display_name(&s("extension"));
                let actions: Vec<String> = m
                    .get("actions")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).take(MAX_ACTIONS).map(|a| clip(a, MAX_SHORT)).collect())
                    .unwrap_or_default();
                if !actions.is_empty() {
                    let theirs: Vec<usize> =
                        (0..self.notices.len()).filter(|&i| self.notices[i].reply_to.is_some()).collect();
                    if theirs.len() >= MAX_QUESTIONS {
                        self.answer_notice(theirs[0], None);
                    }
                }
                self.notice(level, &source, clip(&s("text"), MAX_TEXT));
                if let (Some(id), Some(last), false) =
                    (m.get("id").and_then(Value::as_u64), self.notices.last_mut(), actions.is_empty())
                {
                    last.actions = actions;
                    last.reply_to = Some(id);
                }
            }
            "status" => {
                let id = s("id");
                self.status_items.retain(|(i, _)| *i != id);
                let visible = m.get("visible").and_then(Value::as_bool).unwrap_or(false);
                let text = clip(crate::codicons::render(&s("text")).trim(), MAX_SHORT);
                if visible && !text.is_empty() && self.status_items.len() < MAX_STATUS {
                    self.status_items.push((id, text));
                }
            }
            "write" => {
                let (path, text) = (PathBuf::from(s("path")), s("text"));
                let by = self.display_name(&s("extension"));
                let refused = match writable(&path, &self.open, &self.folders, &self.own_dirs) {
                    Ok(()) if text.len() > MAX_WRITE => Some("it is too big"),
                    Ok(()) => None,
                    Err(why) => Some(why),
                };
                match refused {
                    None => events.push(Event::Write { path, text }),
                    Some(why) => {
                        log::warn!("refused a write of {by} to {}: {why}", path.display());
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        self.notice(Level::Warn, &by, format!("was not let change {}: {why}", clip(&name, MAX_SHORT)));
                    }
                }
            }
            "reveal" => {
                let n = |key: &str| m.get(key).and_then(Value::as_u64).map(|n| n as u32);
                let path = PathBuf::from(s("path"));
                events.push(match n("line") {
                    Some(line) => Event::GoTo { path, line, col: n("col").unwrap_or(0), len: n("len").unwrap_or(1) },
                    None => Event::Reveal(path),
                });
            }
            "openExternal" => self.log_line(&format!("[host] link not opened: {}", s("url"))),
            "reply" => {
                let id = m.get("id").and_then(Value::as_u64).unwrap_or(0);
                let (path, quiet) = match self.pending.remove(&id) {
                    Some(Pending::Format(path, quiet)) => (path, quiet),
                    Some(Pending::View(asked)) => {
                        self.on_view_reply(id, asked, m);
                        return;
                    }
                    Some(Pending::Webview(key)) => {
                        let handle = m.pointer("/result/handle").and_then(Value::as_str).map(str::to_string);
                        let error = m.get("error").and_then(Value::as_str).unwrap_or("it gave no page").to_string();
                        let mut failed = None;
                        if let Some(entry) = self.webviews.iter_mut().find(|w| w.key == key) {
                            entry.resolving = false;
                            match handle {
                                Some(h) => entry.handle = Some(h),
                                None => {
                                    entry.failed = true;
                                    failed = Some(entry.title.clone());
                                }
                            }
                        }
                        if let Some(title) = failed {
                            self.notice(Level::Error, &title, error);
                        }
                        return;
                    }
                    Some(Pending::Hover(path, line, col)) => {
                        let text = m.pointer("/result/text").and_then(Value::as_str).unwrap_or_default().to_string();
                        self.hover = Some(Hover { path, line, col, text });
                        return;
                    }
                    Some(Pending::Relations(page, of, asked)) => {
                        if let Some(found) = found_of(asked, m.get("result")) {
                            events.push(Event::Relations { page, of, found });
                        }
                        return;
                    }
                    Some(Pending::Complete(path, line, col)) => {
                        if id == self.completing {
                            events.push(Event::Suggestions(suggestions_of(m.get("result"), path, line, col)));
                        }
                        return;
                    }
                    Some(Pending::CompletionExtra(path)) => {
                        let edits = extra_edits_of(m.get("result"));
                        if !edits.is_empty() {
                            events.push(Event::ExtraEdits { path, edits });
                        }
                        return;
                    }
                    Some(Pending::Definition(from)) => {
                        let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
                        match m.pointer("/result/items/0") {
                            Some(first) => {
                                let len = if n(first, "endLine") == n(first, "line") {
                                    n(first, "endCol").saturating_sub(n(first, "col"))
                                } else {
                                    1
                                };
                                events.push(Event::GoTo {
                                    path: PathBuf::from(first.get("path").and_then(Value::as_str).unwrap_or_default()),
                                    line: n(first, "line"),
                                    col: n(first, "col"),
                                    len: len.max(1),
                                });
                            }
                            None => {
                                let name =
                                    from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                self.notice(
                                    Level::Info,
                                    "Go to definition",
                                    format!("no extension knows a definition for that name in {name}"),
                                );
                            }
                        }
                        return;
                    }
                    Some(Pending::Command(title)) => {
                        match m.get("error").and_then(Value::as_str) {
                            Some(error) => self.notice(Level::Warn, &title, error.to_string()),
                            None => self.notice(Level::Info, &title, "done"),
                        }
                        return;
                    }
                    None => return,
                };
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if let Some(error) = m.get("error").and_then(Value::as_str) {
                    self.notice(Level::Warn, "Format", format!("{name}: {error}"));
                    return;
                }
                if quiet {
                    let result = m.get("result").cloned().unwrap_or(Value::Null);
                    if let (Some(true), Some(text)) =
                        (result.get("changed").and_then(Value::as_bool), result.get("text").and_then(Value::as_str))
                    {
                        events.push(Event::Write { path, text: text.to_string() });
                    }
                    return;
                }
                let result = m.get("result").cloned().unwrap_or(Value::Null);
                let by = self.display_name(result.get("extension").and_then(Value::as_str).unwrap_or_default());
                match (result.get("changed").and_then(Value::as_bool), result.get("text").and_then(Value::as_str)) {
                    (Some(true), Some(text)) => {
                        self.notice(Level::Info, &by, format!("formatted {name}"));
                        events.push(Event::Write { path, text: text.to_string() });
                    }
                    _ => self.notice(Level::Info, &by, format!("{name} is already formatted")),
                }
            }
            _ => {}
        }
    }

    fn display_name(&self, id: &str) -> String {
        self.installed.iter().find(|e| e.id == id).map(|e| e.display_name.clone()).unwrap_or_else(|| id.to_string())
    }

    pub fn diagnostics_under<'a>(&'a self, root: &Path) -> impl Iterator<Item = (&'a str, &'a FileDiagnostics)> + 'a {
        let prefix = folder_prefix(root);
        let len = prefix.len();
        let under = move |key: &String, path: &String| key.starts_with(&prefix) && path.len() == key.len();
        self.diagnostics.iter().filter(move |(key, d)| under(key, &d.path)).map(move |(_, d)| (&d.path[len..], d))
    }

    pub fn decorations_under<'a>(&'a self, root: &Path) -> impl Iterator<Item = (&'a str, &'a FileDecorations)> + 'a {
        let prefix = folder_prefix(root);
        let len = prefix.len();
        let under = move |key: &String, path: &String| key.starts_with(&prefix) && path.len() == key.len();
        self.decorations
            .iter()
            .chain(self.own_decorations.iter())
            .filter(move |(key, d)| under(key, &d.path))
            .map(move |(_, d)| (&d.path[len..], d))
    }

    pub fn set_own_decorations(&mut self, path: &str, key: &str, items: Vec<Decoration>) {
        let path = path.replace('\\', "/");
        let at = path_key(&path);
        let file =
            self.own_decorations.entry(at.clone()).or_insert_with(|| FileDecorations { path, by_type: HashMap::new() });
        if items.is_empty() {
            file.by_type.remove(key);
        } else {
            file.by_type.insert(key.to_string(), items);
        }
        if file.by_type.is_empty() {
            self.own_decorations.remove(&at);
        }
    }

    pub fn totals(&self) -> (u32, u32) {
        self.diagnostics.values().fold((0, 0), |(e, w), d| (e + d.errors, w + d.warnings))
    }
}

fn folder_prefix(root: &Path) -> String {
    let mut prefix = path_key(&root.to_string_lossy());
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    prefix
}

fn decoration(v: &Value) -> Decoration {
    let n = |key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default();
    Decoration {
        line: n("line"),
        col: n("col"),
        end_line: n("endLine"),
        end_col: n("endCol"),
        whole_line: v.get("wholeLine").and_then(Value::as_bool).unwrap_or(false),
        text: clip(s("text"), MAX_SHORT),
        color: parse_color(s("color")),
        background: parse_color(s("background")),
    }
}

fn diagnostic(v: &Value) -> Diagnostic {
    let n = |key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    Diagnostic {
        line: n("line"),
        col: n("col"),
        end_line: n("endLine"),
        end_col: n("endCol"),
        severity: match n("severity") {
            0 => Severity::Error,
            1 => Severity::Warning,
            2 => Severity::Info,
            _ => Severity::Hint,
        },
        message: clip(&s("message"), MAX_TEXT),
        source: clip(&s("source"), MAX_SHORT),
        code: clip(&s("code"), MAX_SHORT),
    }
}

fn remove_dir_retrying(dir: &Path) -> std::io::Result<()> {
    let mut last = Ok(());
    for _ in 0..20 {
        last = std::fs::remove_dir_all(dir);
        if last.is_ok() || !dir.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_found_by_folder() {
        let mut ext = Extensions::new(std::env::temp_dir().join("code-canvas-test-ext-none"), Waker::new(|| {}));
        let mut events = Vec::new();
        let message = json!({
            "type": "diagnostics",
            "path": "C:\\Work\\App\\src\\a.ts",
            "items": [
                { "line": 2, "col": 4, "endLine": 2, "endCol": 9, "severity": 0, "message": "boom", "source": "eslint", "code": "no-undef" },
                { "line": 5, "col": 0, "endLine": 5, "endCol": 1, "severity": 1, "message": "hm", "source": "eslint", "code": "" }
            ]
        });
        ext.on_message(&message, &mut events);
        assert_eq!(ext.totals(), (1, 1));
        let root = if cfg!(windows) { "c:\\work\\app" } else { "C:/Work/App" };
        let found: Vec<_> = ext.diagnostics_under(Path::new(root)).collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "src/a.ts");
        assert_eq!(found[0].1.items[0].message, "boom");
        assert_eq!(ext.diagnostics_under(Path::new("C:/Work/Other")).count(), 0);

        ext.on_message(&json!({ "type": "diagnostics", "path": "C:\\Work\\App\\src\\a.ts", "items": [] }), &mut events);
        assert_eq!(ext.totals(), (0, 0));
    }

    #[test]
    fn answers_about_relations_become_events() {
        let mut ext = Extensions::new(std::env::temp_dir().join("code-canvas-test-ext-none"), Waker::new(|| {}));
        let mut events = Vec::new();
        let asked =
            [(1, Asked::Uses), (2, Asked::Calls), (3, Asked::Implementations), (4, Asked::Calls), (5, Asked::Uses)];
        for (id, what) in asked {
            ext.pending.insert(id, Pending::Relations(9, (1, 2), what));
        }
        let reply = |id: u64, result: Value| json!({ "type": "reply", "id": id, "result": result });
        let at = |line: u32, col: u32| (PathBuf::from("/w/a.rs"), line, col);
        ext.on_message(
            &reply(1, json!({ "items": [{ "path": "/w/a.rs", "line": 3, "col": 4, "endLine": 3, "endCol": 7 }] })),
            &mut events,
        );
        let call = json!({ "path": "/w/a.rs", "line": 8, "col": 3, "name": "helper", "fromPath": "/w/a.rs", "fromLine": 5, "fromCol": 4 });
        ext.on_message(&reply(2, json!({ "items": [call], "known": true })), &mut events);
        ext.on_message(&reply(3, json!({ "items": [{ "path": "/w/a.rs", "line": 2, "col": 5 }] })), &mut events);
        ext.on_message(&reply(4, json!({ "items": [], "known": true })), &mut events);
        ext.on_message(&reply(5, json!({ "items": [] })), &mut events);
        ext.on_message(&json!({ "type": "reply", "id": 6, "result": { "items": [], "known": true } }), &mut events);
        let found: Vec<Found> = events
            .into_iter()
            .map(|e| match e {
                Event::Relations { page: 9, of: (1, 2), found } => found,
                _ => panic!("another event"),
            })
            .collect();
        assert_eq!(
            found,
            [
                Found::Uses(vec![at(3, 4)]),
                Found::Calls(vec![(at(5, 4), at(8, 3))]),
                Found::Implementations(vec![at(2, 5)]),
                Found::Calls(Vec::new()),
            ],
            "no uses is no answer, but a function the server knows may call nothing"
        );
        assert!(ext.pending.is_empty());
        assert_eq!(found_of(Asked::Calls, Some(&json!({ "items": [], "known": false }))), None);
        let outside = Found::Calls(vec![(at(5, 4), at(8, 3)), (at(6, 0), (PathBuf::from("/std/x.rs"), 1, 1))]);
        let inside = |p: &PathBuf| p.strip_prefix("/w").ok().map(|r| r.to_string_lossy().into_owned());
        assert_eq!(
            outside.with_paths(inside),
            Found::Calls(vec![(("a.rs".to_string(), 5, 4), ("a.rs".to_string(), 8, 3))]),
            "a call to what is outside the folder is left out"
        );
    }

    #[test]
    fn questions_wait_for_an_answer() {
        let mut ext = Extensions::new(std::env::temp_dir().join("code-canvas-test-ext-none"), Waker::new(|| {}));
        let mut events = Vec::new();
        let ask = json!({ "type": "message", "id": 7, "level": "info", "extension": "x.y", "text": "Trust?", "actions": ["Yes", "No"] });
        ext.on_message(&ask, &mut events);
        ext.on_message(
            &json!({ "type": "message", "level": "info", "extension": "x.y", "text": "fyi", "actions": [] }),
            &mut events,
        );
        assert_eq!(ext.notices.len(), 2);
        assert_eq!(ext.notices[0].actions, vec!["Yes", "No"]);
        assert!(ext.notices[1].actions.is_empty());
        ext.answer_notice(0, Some("Yes"));
        assert_eq!(ext.notices.len(), 1);
        assert_eq!(ext.notices[0].text, "fyi");
    }

    #[test]
    fn commands_and_views_of_a_manifest() {
        let nls = json!({ "cmd.fix": "Fix all problems", "view.prs": { "message": "Pull Requests" } });
        let manifest = json!({ "contributes": {
            "commands": [
                { "command": "x.fix", "title": "%cmd.fix%", "category": "Lint" },
                { "command": "x.internal", "title": "Internal" },
                { "command": "x.about", "title": { "value": "About" } }
            ],
            "menus": { "commandPalette": [{ "command": "x.internal", "when": "false" }] },
            "views": { "github": [{ "id": "prs", "name": "%view.prs%" }, { "id": "issues", "name": "Issues" }] }
        }});
        let commands = manifest_commands(&manifest, &nls);
        assert_eq!(
            commands,
            vec![
                ("x.about".to_string(), "About".to_string()),
                ("x.fix".to_string(), "Lint: Fix all problems".to_string())
            ]
        );
        assert_eq!(manifest_views(&manifest, &nls), vec!["Pull Requests", "Issues"]);
        assert!(manifest_commands(&json!({}), &nls).is_empty());
    }

    #[test]
    fn default_extensions_are_offered_once() {
        let defaults = [("Acme", "one"), ("acme", "two"), ("acme", "three")];
        let mut offered = BTreeSet::new();
        let wanted = defaults_to_install(&defaults, &mut offered, &|id| id == "acme.two");
        assert_eq!(wanted, vec![("Acme".to_string(), "one".to_string()), ("acme".to_string(), "three".to_string())]);
        assert_eq!(offered.iter().collect::<Vec<_>>(), vec!["acme.two"]);

        offered.insert("acme.one".into());
        let wanted = defaults_to_install(&defaults, &mut offered, &|id| id != "acme.three");
        assert_eq!(wanted, vec![("acme".to_string(), "three".to_string())]);

        offered.insert("acme.three".into());
        assert!(defaults_to_install(&defaults, &mut offered, &|_| false).is_empty());
    }

    #[test]
    fn containers_of_a_manifest() {
        let nls = json!({ "box.title": "My Box" });
        let manifest = json!({ "contributes": {
            "viewsContainers": {
                "activitybar": [
                    { "id": "box", "title": "%box.title%", "icon": "media/box.svg" },
                    { "id": "prs", "title": "Pull Requests", "icon": "$(github)" }
                ],
                "secondarySidebar": [{ "id": "side", "title": "Side", "icon": "media/side.svg" }]
            },
            "views": {
                "box": [
                    { "id": "box.page", "name": "Page", "type": "webview", "when": "box.ready" },
                    { "id": "box.tree", "name": "Tree" }
                ],
                "prs": [{ "id": "prs.list", "name": "List" }],
                "explorer": [{ "id": "x.outline", "name": "Outline" }]
            }
        }});
        let containers = manifest_containers(&manifest, &nls);
        let by_id = |id: &str| containers.iter().find(|c| c.id == id).unwrap();
        assert_eq!(containers.len(), 4);
        let elsewhere = by_id(".:views");
        assert_eq!((elsewhere.title.as_str(), elsewhere.views[0].id.as_str()), ("", "x.outline"));
        assert_eq!(by_id("box").title, "My Box");
        assert_eq!(by_id("box").codicon, None);
        assert_eq!(by_id("prs").codicon.as_deref(), Some("github"));
        let page = &by_id("box").views[0];
        assert!(page.webview && page.when == "box.ready" && page.id == "box.page");
        assert!(!by_id("box").views[1].webview);
        assert!(by_id("side").views.is_empty());
        assert!(manifest_containers(&json!({}), &nls).is_empty());
    }

    fn installed_with(name: &str, manifest: Value) -> Installed {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-rail-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut manifest = manifest;
        manifest["publisher"] = json!("test");
        manifest["name"] = json!(name);
        manifest["main"] = json!("./main.js");
        std::fs::write(dir.join("package.json"), manifest.to_string()).unwrap();
        let read = read_installed(&dir, &HashSet::new()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        read
    }

    #[test]
    fn the_rail_follows_manifests_and_context() {
        let boxed = installed_with(
            "box",
            json!({ "contributes": {
                "viewsContainers": { "activitybar": [
                    { "id": "a", "title": "Box", "icon": "$(github)" },
                    { "id": "b", "title": "Box, elsewhere", "icon": "b.svg" },
                    { "id": "trees", "title": "Only trees", "icon": "t.svg" }
                ] },
                "views": {
                    "a": [{ "id": "box.a", "name": "A", "type": "webview", "when": "box.narrow" }],
                    "b": [{ "id": "box.b", "name": "B", "type": "webview", "when": "!box.narrow" }],
                    "trees": [{ "id": "box.tree", "name": "Tree" }]
                }
            }}),
        );
        let plain = installed_with("plain", json!({}));
        let installed = vec![boxed, plain];
        let views = |context: Value, webviews: &[WebviewEntry]| -> Vec<String> {
            rail_items_of(&installed, webviews, &|key| context.get(key).cloned()).into_iter().map(|i| i.view).collect()
        };

        assert_eq!(views(json!({}), &[]), vec!["box.b", "tree:trees"]);
        assert_eq!(views(json!({ "box.narrow": true }), &[]), vec!["box.a", "tree:trees"]);

        let panel = WebviewEntry {
            key: "panel-1".into(),
            extension: "test.plain".into(),
            title: "Graph".into(),
            handle: Some("panel-1".into()),
            resolving: false,
            failed: false,
            tab: true,
        };
        assert_eq!(views(json!({}), std::slice::from_ref(&panel)), vec!["box.b", "tree:trees", "panel-1"]);

        let item = rail_items_of(&installed, &[], &|_| None).remove(0);
        assert_eq!((item.title.as_str(), item.codicon, item.extension.as_str()), ("Box, elsewhere", None, "test.box"));

        let mut off = installed;
        off[0].enabled = false;
        assert!(rail_items_of(&off, &[], &|_| None).is_empty());
    }

    #[test]
    #[ignore]
    fn rail_of_a_real_extensions_folder() {
        let dir = PathBuf::from(std::env::var("EXT_RAIL_TEST").expect("set EXT_RAIL_TEST to an extensions folder"));
        let installed: Vec<Installed> =
            market::installed_dirs(&dir).iter().filter_map(|d| read_installed(d, &HashSet::new())).collect();
        println!("{} extensions", installed.len());
        for (label, context) in
            [("nothing set", json!({})), ("narrow host", json!({ "claude-code:doesNotSupportSecondarySidebar": true }))]
        {
            println!("-- {label}");
            for item in rail_items_of(&installed, &[], &|key| context.get(key).cloned()) {
                println!("   {:38} {:34} {:?} {:?}", item.extension, item.view, item.title, item.codicon);
            }
        }
        for e in &installed {
            let trees: Vec<&str> =
                e.containers.iter().filter(|c| !c.views.iter().any(|v| v.webview)).map(|c| c.title.as_str()).collect();
            if !trees.is_empty() {
                println!("   (no icon, only trees) {}: {trees:?}", e.id);
            }
        }
    }

    #[test]
    fn what_a_language_server_offers_is_read() {
        let reply = json!({ "serial": 7, "items": [
            { "label": "title()", "detail": "fn(&self) -> String", "kind": "Method", "text": "title()", "caret": null, "select": 0, "col": 6 },
            { "label": "new(…)", "detail": "", "kind": "Function", "text": "new(cap)", "caret": 4, "select": 3, "col": 12 },
        ] });
        let found = suggestions_of(Some(&reply), PathBuf::from("a.rs"), 3, 8);
        assert_eq!((found.serial, found.line, found.col, found.items.len()), (7, 3, 8, 2));
        assert_eq!((found.items[0].label.as_str(), found.items[0].caret, found.items[0].col), ("title()", None, 6));
        assert_eq!(
            (found.items[1].caret, found.items[1].select, found.items[1].kind.as_str()),
            (Some(4), 3, "Function")
        );
        assert!(suggestions_of(None, PathBuf::new(), 0, 0).items.is_empty());

        let extra = json!({ "edits": [{ "line": 0, "endLine": 0, "col": 0, "endCol": 0, "text": "use a::B;\n" }] });
        let edits = extra_edits_of(Some(&extra));
        assert_eq!(edits, [ExtraEdit { line: 0, col: 0, end_line: 0, end_col: 0, text: "use a::B;\n".into() }]);
        assert!(extra_edits_of(None).is_empty());
    }

    #[test]
    fn colours_from_extensions() {
        assert_eq!(parse_color("#fff"), 0xffffffff);
        assert_eq!(parse_color("#102030"), 0x102030ff);
        assert_eq!(parse_color("#10203040"), 0x10203040);
        assert_eq!(parse_color("rgba(1,2,3,1)"), 0, "not understood: left unset");
        assert_eq!(parse_color(""), 0);
        assert_eq!(parse_color("theme:errorLens.errorForeground") >> 8, 0xf7768e);
        assert_eq!(parse_color("theme:errorLens.warningForeground") >> 8, 0xe0af68);
        assert!(parse_color("theme:errorLens.warningBackground") & 0xff < 0x40, "backgrounds are faint");
    }

    #[test]
    fn decorations_are_kept_per_type() {
        let mut ext = Extensions::new(std::env::temp_dir().join("code-canvas-test-ext-none"), Waker::new(|| {}));
        let mut events = Vec::new();
        let item = json!({ "line": 3, "col": 0, "endLine": 3, "endCol": 0, "wholeLine": true, "text": "boom", "color": "#ff0000", "background": "" });
        ext.on_message(
            &json!({ "type": "decorations", "path": "C:\\Work\\App\\a.ts", "key": "k1", "items": [item] }),
            &mut events,
        );
        let root = if cfg!(windows) { "c:\\work\\app" } else { "C:/Work/App" };
        let found: Vec<_> = ext.decorations_under(Path::new(root)).collect();
        assert_eq!(found[0].0, "a.ts");
        assert_eq!(found[0].1.by_type["k1"][0].text, "boom");
        ext.on_message(
            &json!({ "type": "decorations", "path": "C:\\Work\\App\\a.ts", "key": "k1", "items": [] }),
            &mut events,
        );
        assert!(ext.decorations.is_empty());
    }

    #[test]
    fn status_text_shows_icons() {
        let mut ext = Extensions::new(std::env::temp_dir().join("code-canvas-test-ext-none"), Waker::new(|| {}));
        let mut events = Vec::new();
        ext.on_message(
            &json!({ "type": "status", "id": "p", "visible": true, "text": "$(check) Prettier" }),
            &mut events,
        );
        let check = crate::codicons::glyph("check").unwrap();
        assert_eq!(ext.status_items, vec![("p".to_string(), format!("{check} Prettier"))]);
        ext.on_message(&json!({ "type": "status", "id": "p", "visible": false, "text": "" }), &mut events);
        assert!(ext.status_items.is_empty());
    }

    #[test]
    fn what_runs_in_a_folder_that_is_not_trusted_follows_each_manifest() {
        let no = installed_with("no", json!({ "capabilities": { "untrustedWorkspaces": { "supported": false } } }));
        let yes = installed_with("yes", json!({ "capabilities": { "untrustedWorkspaces": { "supported": true } } }));
        let limited = installed_with(
            "limited",
            json!({ "capabilities": { "untrustedWorkspaces": {
                "supported": "limited", "restrictedConfigurations": ["limited.path", "limited.path", ""]
            } } }),
        );
        let silent = installed_with("silent", json!({}));
        assert_eq!(no.untrusted, Untrusted::Unsupported);
        assert_eq!(yes.untrusted, Untrusted::Supported);
        assert_eq!(
            limited.untrusted,
            Untrusted::Limited(vec!["limited.path".into(), "limited.path".into(), "".into()])
        );
        assert_eq!(silent.untrusted, Untrusted::Unsupported, "saying nothing is as VS Code takes it: not supported");
        assert!(!runs_in(&no, false) && runs_in(&no, true));
        assert!(runs_in(&yes, false) && runs_in(&limited, false));
        assert!(!runs_in(&silent, false));
        let mut off = installed_with("off", json!({}));
        off.enabled = false;
        assert!(!runs_in(&off, true), "a disabled one never runs");

        let known = manifest_untrusted(&json!({}), "usernamehw.errorlens", true);
        assert_eq!(known, Untrusted::Supported, "one that only shows what others report");
        assert_eq!(manifest_untrusted(&json!({}), "x.themes", false), Untrusted::Supported, "no code to run");

        let all = [no, yes, limited, silent];
        let keys = restricted_keys(&all);
        assert!(keys.contains(&"limited.path".to_string()) && keys.contains(&"typescript.tsdk".to_string()));
        assert_eq!(keys.iter().filter(|k| *k == "limited.path").count(), 1);
        assert!(!keys.iter().any(String::is_empty));
        let held: Vec<&str> = held_back(&all).map(|e| e.id.as_str()).collect();
        assert_eq!(held, vec!["test.no", "test.limited", "test.silent"]);
    }

    #[test]
    fn a_folder_that_is_not_trusted_marks_what_is_held_back() {
        let mut ext = Extensions::new(std::env::temp_dir().join("code-canvas-test-ext-none"), Waker::new(|| {}));
        ext.installed = vec![installed_with("held", json!({}))];
        ext.folders = vec![PathBuf::from("/a")];
        ext.trusted = false;
        ext.mark_restricted();
        assert_eq!(ext.installed[0].status, Status::Restricted);
        assert!(ext.restricted_mode());
        ext.trusted = true;
        ext.mark_restricted();
        assert_eq!(ext.installed[0].status, Status::Idle);
        assert!(!ext.restricted_mode());
    }

    fn link_dir(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_dir(target, link).is_ok() {
                return true;
            }
            let made = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .is_ok_and(|o| o.status.success());
            made && link.exists()
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    #[test]
    fn an_extension_writes_only_open_documents_inside_the_open_folders() {
        let root = std::env::temp_dir().join(format!("code-canvas-test-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (folder, outside, data) = (root.join("project"), root.join("outside"), root.join("project").join("data"));
        for dir in [&folder.join("src"), &outside, &data, &folder.join(".git").join("hooks")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let file = folder.join("src").join("a.ts");
        let away = outside.join("b.ts");
        let hook = folder.join(".git").join("hooks").join("pre-commit");
        let own = data.join("prefs.json");
        let up = folder.join("src").join("..").join("..").join("outside").join("b.ts");
        let fresh = folder.join("src").join("new.ts");
        for f in [&file, &away, &hook, &own] {
            std::fs::write(f, "x").unwrap();
        }
        let folders = vec![folder.clone()];
        let forbidden = vec![data.clone()];
        let open = vec![file.clone(), away.clone(), hook.clone(), own.clone(), up.clone(), fresh.clone()];
        let check = |path: &Path| writable(path, &open, &folders, &forbidden);
        assert_eq!(check(&file), Ok(()));
        assert_eq!(check(&fresh), Ok(()), "an open document not written yet");
        let spelled = PathBuf::from(file.to_string_lossy().replace('\\', "/"));
        if cfg!(windows) {
            assert_eq!(writable(&spelled, &open, &folders, &forbidden), Ok(()), "the same file spelled another way");
        }
        assert_eq!(check(&folder.join("src").join("closed.ts")), Err("it is not a document open in the app"));
        assert_eq!(check(&away), Err("it is outside the open folders"));
        assert_eq!(check(&up), Err("it goes up with .."));
        assert_eq!(check(&hook), Err("it is inside .git"));
        assert_eq!(check(&own), Err("it is in the app's own folders"));
        assert_eq!(check(Path::new("src/a.ts")), Err("it is not a full path"));
        assert_eq!(writable(&file, &open, &[], &forbidden), Err("it is outside the open folders"));
        let link = folder.join("linked");
        if link_dir(&link, &outside) {
            let through = link.join("b.ts");
            let open = vec![through.clone()];
            assert_eq!(
                writable(&through, &open, &folders, &forbidden),
                Err("it is outside the open folders"),
                "a link out"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_the_host_says_is_kept_small() {
        let mut ext = Extensions::new(std::env::temp_dir().join("code-canvas-test-ext-none"), Waker::new(|| {}));
        let mut events = Vec::new();
        let long = "y".repeat(10_000);
        for i in 0..100 {
            ext.on_message(
                &json!({ "type": "status", "id": format!("s{i}"), "visible": true, "text": long }),
                &mut events,
            );
        }
        assert_eq!(ext.status_items.len(), MAX_STATUS);
        assert!(ext.status_items[0].1.chars().count() <= MAX_SHORT + 1);
        let items: Vec<Value> =
            (0..5000).map(|i| json!({ "line": i, "col": 0, "endLine": i, "endCol": 1, "message": long })).collect();
        ext.on_message(&json!({ "type": "diagnostics", "path": "C:/w/a.ts", "items": items }), &mut events);
        let file = ext.diagnostics.values().next().unwrap();
        assert_eq!(file.items.len(), MAX_ITEMS);
        assert!(file.items[0].message.chars().count() <= MAX_TEXT + 1);
        for i in 0..100 {
            let message = json!({ "type": "message", "id": i, "level": "info", "text": long, "actions": ["a", "b"] });
            ext.on_message(&message, &mut events);
        }
        assert_eq!(ext.notices.iter().filter(|n| n.reply_to.is_some()).count(), MAX_QUESTIONS);
        assert!(ext.notices.iter().all(|n| n.text.chars().count() <= MAX_TEXT + 1));
        for i in 0..100 {
            let panel = json!({ "type": "webviewPanel", "handle": format!("w{i}"), "extension": "x.y", "title": long });
            ext.on_message(&panel, &mut events);
        }
        assert_eq!(ext.webviews.len(), MAX_WEBVIEWS);
        for i in 0..5000 {
            ext.on_message(&json!({ "type": "context", "key": format!("k{i}"), "value": true }), &mut events);
        }
        ext.on_message(&json!({ "type": "context", "key": "big", "value": long.repeat(2) }), &mut events);
        assert_eq!(ext.contexts.len(), MAX_CONTEXTS);
        assert!(!ext.contexts.contains_key("big"));
        ext.on_message(&json!({ "type": "write", "path": "C:/w/a.ts", "text": "x", "extension": "x.y" }), &mut events);
        assert!(!events.iter().any(|e| matches!(e, Event::Write { .. })), "not open, no folders");
        assert_eq!(clip("añb", 2), "añ…");
        assert_eq!(clip("ab", 2), "ab");
    }
}
