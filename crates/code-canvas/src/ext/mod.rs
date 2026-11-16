//! VS Code extensions.
//!
//! Installed extensions live unpacked in `<data dir>/extensions`. Those with
//! code run in the extension host (`host.rs`, a Node process); what they
//! report comes back here: diagnostics for the canvas and the Problems
//! panel, formatted text, messages. See docs/EXTENSIONS.md.

pub mod host;
pub mod market;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use host::{Host, HostEvent};
pub use market::MarketItem;

/// Wakes the UI thread when background work has something for it.
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

/// Register the waker of the UI thread (once, at start-up).
pub fn set_ui_waker(waker: Waker) {
    let _ = UI_WAKER.set(waker);
}

/// Wake the UI thread: a background thread finished something.
pub fn wake_ui() {
    if let Some(w) = UI_WAKER.get() {
        w.wake();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Installed; the host has not reported on it (yet).
    Idle,
    Activating,
    Active,
    Failed,
    /// Contributes data only (themes, grammars…): nothing to run.
    Declarative,
    Disabled,
}

pub struct Installed {
    /// `publisher.name`, lower case.
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
    /// VS Code API this extension asked for that the host does not have yet.
    pub unsupported: Vec<String>,
    /// The extension's own icon (a PNG inside its folder).
    pub icon: Option<PathBuf>,
    /// Commands it offers, as `(id, title)`.
    pub commands: Vec<(String, String)>,
    /// Names of the views it adds to VS Code's side bar.
    pub views: Vec<String>,
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
    /// Display columns (tabs expanded), like everything on the canvas.
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub severity: Severity,
    pub message: String,
    pub source: String,
    pub code: String,
}

impl Diagnostic {
    /// Is the cell at `(line, col)` inside this diagnostic's range?
    pub fn covers(&self, line: u32, col: u32) -> bool {
        if line < self.line || line > self.end_line {
            return false;
        }
        let after_start = line > self.line || col >= self.col;
        let before_end = line < self.end_line || col < self.end_col.max(self.col + 1);
        after_start && before_end
    }

    /// `eslint(no-undef)`, or whatever of that is known.
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
    /// Absolute path with `/` separators.
    pub path: String,
    pub items: Vec<Diagnostic>,
    pub errors: u32,
    pub warnings: u32,
}

/// Something an extension draws on a file: a wash over a range and/or
/// text after the end of a line (VS Code's "text editor decorations").
#[derive(Clone, Debug)]
pub struct Decoration {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub whole_line: bool,
    /// Text shown after the line's code.
    pub text: String,
    /// `0xRRGGBBAA`; zero alpha means "not set".
    pub color: u32,
    pub background: u32,
}

pub struct FileDecorations {
    /// Absolute path with `/` separators.
    pub path: String,
    /// By decoration type, as the extension set them.
    pub by_type: HashMap<String, Vec<Decoration>>,
}

/// A colour sent by the host: `#rgb`, `#rrggbb`, `#rrggbbaa`, or
/// `theme:<id>` for a colour of VS Code's theme, which is guessed from its
/// name until themes are supported.
fn parse_color(name: &str) -> u32 {
    if let Some(id) = name.strip_prefix("theme:") {
        // `errorLens.warningForeground`: the last part says what it is.
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

/// A message for the user (from an extension, or about one).
pub struct Notice {
    pub level: Level,
    pub source: String,
    pub text: String,
    pub at: Instant,
    /// Choices the extension offers; it waits for one (or a dismissal).
    pub actions: Vec<String>,
    /// The host's id of a message with choices.
    reply_to: Option<u64>,
}

pub enum NodeStatus {
    Checking,
    Ready(String),
    Missing(String),
}

/// Things the app has to do for the extensions.
pub enum Event {
    /// Replace a file's contents (a formatter or a fix produced them).
    Write {
        path: PathBuf,
        text: String,
    },
    /// Show this file.
    Reveal(PathBuf),
    /// An extension set the page of a webview.
    WebviewHtml {
        handle: String,
        html: String,
    },
    /// An extension sent a message to a webview's page.
    WebviewPost {
        handle: String,
        message: Value,
    },
    WebviewClosed(String),
    /// Bring this webview (by key) in front of the user.
    ShowWebview(String),
}

/// A webview an extension offers: a view it registered (shown when the user
/// asks for it) or a panel it opened itself.
pub struct WebviewEntry {
    /// The view's id, or the handle of a panel.
    pub key: String,
    pub extension: String,
    pub title: String,
    /// Set once the extension has been asked to fill it.
    pub handle: Option<String>,
    resolving: bool,
}

enum Job {
    Node(Result<String, String>),
    Search(String, Result<Vec<MarketItem>, String>),
    Installed(String, Result<PathBuf, String>),
}

enum Pending {
    Format(PathBuf),
    /// A command, by its title.
    Command(String),
    /// A webview view being filled, by key.
    Webview(String),
}

/// Documents kept open in the host at most (least recently seen go first).
const MAX_OPEN: usize = 48;
const MAX_LOG: usize = 400;

pub struct Extensions {
    pub installed: Vec<Installed>,
    pub node: NodeStatus,
    pub query: String,
    pub results: Vec<MarketItem>,
    pub searching: bool,
    pub market_error: String,
    pub installing: HashSet<String>,
    /// By [`path_key`].
    pub diagnostics: HashMap<String, FileDiagnostics>,
    /// Bumped whenever `diagnostics` changes.
    pub revision: u64,
    /// By [`path_key`].
    pub decorations: HashMap<String, FileDecorations>,
    pub notices: Vec<Notice>,
    pub log: VecDeque<String>,
    /// Status bar items extensions are showing, `(id, text)`.
    pub status_items: Vec<(String, String)>,
    pub webviews: Vec<WebviewEntry>,
    /// Webviews that went away with their host; reported by `poll`.
    dropped_webviews: Vec<String>,
    pub host_running: bool,

    dir: PathBuf,
    data_dir: PathBuf,
    disabled: HashSet<String>,
    host: Option<Host>,
    folders: Vec<PathBuf>,
    /// Open documents, least recently wanted first.
    open: Vec<PathBuf>,
    focused: Option<PathBuf>,
    pending: HashMap<u64, Pending>,
    next_request: u64,
    jobs_tx: Sender<Job>,
    jobs_rx: Receiver<Job>,
    waker: Waker,
}

/// Key for a path that is the same however it is spelled.
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
    // `%key%` points into package.nls.json.
    match raw.strip_prefix('%').and_then(|r| r.strip_suffix('%')) {
        Some(k) => match nls.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(o) => o.get("message").and_then(Value::as_str).unwrap_or(raw).to_string(),
            None => raw.to_string(),
        },
        None => raw.to_string(),
    }
}

/// A manifest string, which may be `%key%` pointing into package.nls.json.
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

/// The commands a manifest offers to the user, as `(id, "Category: Title")`.
/// Those the extension hides from the command palette are left out.
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
        icon: v.get("icon").and_then(Value::as_str).map(|i| dir.join(i)).filter(|p| p.is_file()),
        commands: manifest_commands(&v, &nls),
        views: manifest_views(&v, &nls),
    })
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
            diagnostics: HashMap::new(),
            revision: 0,
            decorations: HashMap::new(),
            notices: Vec::new(),
            log: VecDeque::new(),
            status_items: Vec::new(),
            webviews: Vec::new(),
            dropped_webviews: Vec::new(),
            host_running: false,
            dir,
            data_dir,
            disabled,
            host: None,
            folders: Vec::new(),
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
        self.installed =
            market::installed_dirs(&self.dir).iter().filter_map(|d| read_installed(d, &self.disabled)).collect();
        self.installed.sort_by_key(|e| e.display_name.to_lowercase());
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.installed.iter().any(|e| e.id == id)
    }

    fn notice(&mut self, level: Level, source: &str, text: impl Into<String>) {
        let text = text.into();
        self.log_line(&format!("[{source}] {text}"));
        self.notices.push(Notice {
            level,
            source: source.to_string(),
            text,
            at: Instant::now(),
            actions: Vec::new(),
            reply_to: None,
        });
        // Keep the corner small; questions stay until they are answered.
        while self.notices.iter().filter(|n| n.reply_to.is_none()).count() > 5 {
            let oldest = self.notices.iter().position(|n| n.reply_to.is_none()).unwrap_or(0);
            self.notices.remove(oldest);
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

    // ----- marketplace ------------------------------------------------------

    pub fn search(&mut self) {
        let query = self.query.clone();
        self.searching = true;
        self.market_error.clear();
        self.job(move || {
            let found = market::search(&query);
            Job::Search(query, found)
        });
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
        // The host may hold the files open: stop it first.
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

    // ----- host --------------------------------------------------------------

    fn wants_host(&self) -> bool {
        matches!(self.node, NodeStatus::Ready(_))
            && !self.folders.is_empty()
            && self.installed.iter().any(|e| e.enabled && e.has_code)
    }

    fn stop_host(&mut self) {
        if let Some(host) = self.host.take() {
            host.stop();
        }
        self.host_running = false;
        self.pending.clear();
        self.status_items.clear();
        self.dropped_webviews.extend(self.webviews.drain(..).filter_map(|w| w.handle));
        self.decorations.clear();
        if !self.diagnostics.is_empty() {
            self.diagnostics.clear();
            self.revision += 1;
        }
    }

    fn start_host(&mut self) {
        if self.host.is_some() || !self.wants_host() {
            return;
        }
        match Host::start(&self.data_dir.join("exthost"), self.waker.clone()) {
            Ok(mut host) => {
                let extensions: Vec<Value> =
                    self.installed.iter().filter(|e| e.enabled).map(|e| json!({ "id": e.id, "dir": e.dir })).collect();
                host.send(&json!({
                    "type": "init",
                    "tabWidth": canvas_core::TAB_WIDTH,
                    "storageDir": self.data_dir.join("extension-storage"),
                    "settingsFile": self.data_dir.join("settings.json"),
                    // VS Code's own settings, read as a base when it is installed.
                    "vscodeSettingsFile": dirs::config_dir().map(|d| d.join("Code").join("User").join("settings.json")),
                    "folders": self.folders,
                    "extensions": extensions,
                }));
                // Documents wanted before the (re)start are opened again.
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

    /// The folders open in the app: the extensions' workspace.
    pub fn set_folders(&mut self, folders: Vec<PathBuf>) {
        if folders == self.folders {
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

    /// Files whose code is readable on screen right now: extensions get them
    /// as open documents (that is what they lint). `focus` is the one the
    /// user is pointing at.
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
        while self.open.len() > MAX_OPEN {
            let old = self.open.remove(0);
            self.decorations.remove(&path_key(&old.to_string_lossy()));
            self.send(json!({ "type": "close", "path": old }));
        }
        if let Some(f) = focus {
            if self.focused.as_deref() != Some(f) && self.open.iter().any(|p| p == f) {
                self.focused = Some(f.to_path_buf());
                self.send(json!({ "type": "open", "path": f, "focus": true }));
            }
        }
    }

    /// The file's contents changed on disk.
    pub fn file_changed(&mut self, path: &Path) {
        self.send(json!({ "type": "changed", "path": path }));
    }

    /// Ask the extensions to format `path`; the result arrives as
    /// [`Event::Write`] (or a notice when nothing can format it).
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
        self.pending.insert(id, Pending::Format(path.to_path_buf()));
        self.send(json!({ "type": "request", "id": id, "method": "format", "params": { "path": path } }));
    }

    /// Answer the question in notice `index`: one of its actions, or
    /// `None` to dismiss it. Plain notices are just removed.
    pub fn answer_notice(&mut self, index: usize, action: Option<&str>) {
        if index >= self.notices.len() {
            return;
        }
        let notice = self.notices.remove(index);
        if let Some(id) = notice.reply_to {
            self.send(json!({ "type": "messageReply", "id": id, "action": action }));
        }
    }

    /// Folder the installed extensions live in.
    pub fn extensions_dir(&self) -> &Path {
        &self.dir
    }

    /// Ask the extension to fill the webview view `key` (once).
    pub fn resolve_webview(&mut self, key: &str) {
        let Some(entry) = self.webviews.iter_mut().find(|w| w.key == key) else { return };
        if entry.handle.is_some() || entry.resolving {
            return;
        }
        entry.resolving = true;
        let request = self.next_request;
        self.next_request += 1;
        self.pending.insert(request, Pending::Webview(key.to_string()));
        self.send(json!({ "type": "request", "id": request, "method": "resolveWebview", "params": { "view": key } }));
    }

    /// The page of a webview sent a message to its extension.
    pub fn webview_message(&mut self, handle: &str, message: Value) {
        self.send(json!({ "type": "webviewMessage", "handle": handle, "message": message }));
    }

    /// Run one of an extension's commands (`title` is for messages).
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
        self.stop_host();
    }

    // ----- incoming ----------------------------------------------------------

    /// Take in what background work and the host produced. Returns what the
    /// app must do, and whether anything visible changed.
    pub fn poll(&mut self) -> (Vec<Event>, bool) {
        let mut events: Vec<Event> = self.dropped_webviews.drain(..).map(Event::WebviewClosed).collect();
        let mut changed = !events.is_empty();

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

        // Notices fade after a while; questions wait much longer, and an
        // unanswered one counts as dismissed.
        let before = self.notices.len();
        let expired: Vec<usize> = (0..self.notices.len())
            .rev()
            .filter(|&i| {
                self.notices[i].at.elapsed().as_secs_f32()
                    > if self.notices[i].reply_to.is_some() { 300.0 } else { 9.0 }
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
                let (id, api) = (s("extension"), s("api"));
                if let Some(e) = self.installed.iter_mut().find(|e| e.id == id) {
                    if !e.unsupported.contains(&api) {
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
                    .map(|l| l.iter().map(diagnostic).collect())
                    .unwrap_or_default();
                let key = path_key(&path);
                if items.is_empty() {
                    self.diagnostics.remove(&key);
                } else {
                    let count = |s: Severity| items.iter().filter(|d| d.severity == s).count() as u32;
                    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
                    self.diagnostics.insert(key, FileDiagnostics { path, items, errors, warnings });
                }
                self.revision += 1;
            }
            "webviewView" => {
                let key = s("view");
                if !self.webviews.iter().any(|w| w.key == key) {
                    self.webviews.push(WebviewEntry {
                        key,
                        extension: s("extension"),
                        title: s("title"),
                        handle: None,
                        resolving: false,
                    });
                }
            }
            "webviewPanel" => {
                let handle = s("handle");
                match self.webviews.iter_mut().find(|w| w.key == handle) {
                    Some(entry) => entry.title = s("title"),
                    None => self.webviews.push(WebviewEntry {
                        key: handle.clone(),
                        extension: s("extension"),
                        title: s("title"),
                        handle: Some(handle.clone()),
                        resolving: false,
                    }),
                }
                if m.get("reveal").and_then(Value::as_bool).unwrap_or(false) {
                    events.push(Event::ShowWebview(handle));
                }
            }
            "webviewHtml" => events.push(Event::WebviewHtml { handle: s("handle"), html: s("html") }),
            "webviewPost" => events.push(Event::WebviewPost {
                handle: s("handle"),
                message: m.get("message").cloned().unwrap_or(Value::Null),
            }),
            "webviewClosed" => {
                let handle = s("handle");
                self.webviews.retain(|w| w.handle.as_deref() != Some(handle.as_str()));
                events.push(Event::WebviewClosed(handle));
            }
            "decorations" => {
                let path = s("path").replace('\\', "/");
                let key = path_key(&path);
                let items: Vec<Decoration> = m
                    .get("items")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().map(decoration).collect())
                    .unwrap_or_default();
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
                self.notice(level, &source, s("text"));
                let actions: Vec<String> = m
                    .get("actions")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
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
                // `$(check) Prettier`: the icon names are codicons.
                let text = crate::codicons::render(&s("text")).trim().to_string();
                if visible && !text.is_empty() {
                    self.status_items.push((id, text));
                }
            }
            "write" => events.push(Event::Write { path: PathBuf::from(s("path")), text: s("text") }),
            "reveal" => events.push(Event::Reveal(PathBuf::from(s("path")))),
            "openExternal" => self.log_line(&format!("[host] link not opened: {}", s("url"))),
            "reply" => {
                let id = m.get("id").and_then(Value::as_u64).unwrap_or(0);
                let path = match self.pending.remove(&id) {
                    Some(Pending::Format(path)) => path,
                    Some(Pending::Webview(key)) => {
                        let handle = m.pointer("/result/handle").and_then(Value::as_str).map(str::to_string);
                        let error = m.get("error").and_then(Value::as_str).unwrap_or("it gave no page").to_string();
                        let mut failed = None;
                        if let Some(entry) = self.webviews.iter_mut().find(|w| w.key == key) {
                            entry.resolving = false;
                            match handle {
                                Some(h) => entry.handle = Some(h),
                                None => failed = Some(entry.title.clone()),
                            }
                        }
                        if let Some(title) = failed {
                            self.notice(Level::Error, &title, error);
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

    /// Diagnostics of the files under `root`, as `(path relative to root, diagnostics)`.
    pub fn diagnostics_under<'a>(&'a self, root: &Path) -> impl Iterator<Item = (&'a str, &'a FileDiagnostics)> + 'a {
        let prefix = folder_prefix(root);
        let len = prefix.len();
        // Same length in both spellings: the key only differs in case.
        let under = move |key: &String, path: &String| key.starts_with(&prefix) && path.len() == key.len();
        self.diagnostics.iter().filter(move |(key, d)| under(key, &d.path)).map(move |(_, d)| (&d.path[len..], d))
    }

    /// Decorations of the files under `root`, as `(path relative to root, decorations)`.
    pub fn decorations_under<'a>(&'a self, root: &Path) -> impl Iterator<Item = (&'a str, &'a FileDecorations)> + 'a {
        let prefix = folder_prefix(root);
        let len = prefix.len();
        // Same length in both spellings: the key only differs in case.
        let under = move |key: &String, path: &String| key.starts_with(&prefix) && path.len() == key.len();
        self.decorations.iter().filter(move |(key, d)| under(key, &d.path)).map(move |(_, d)| (&d.path[len..], d))
    }

    /// `(errors, warnings)` over every file.
    pub fn totals(&self) -> (u32, u32) {
        self.diagnostics.values().fold((0, 0), |(e, w), d| (e + d.errors, w + d.warnings))
    }
}

/// [`path_key`] of a folder, ending in `/`: what its files' keys start with.
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
        text: s("text").to_string(),
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
        message: s("message"),
        source: s("source"),
        code: s("code"),
    }
}

/// Windows keeps files locked for a moment after the process using them ends.
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

        // An empty list clears the file.
        ext.on_message(&json!({ "type": "diagnostics", "path": "C:\\Work\\App\\src\\a.ts", "items": [] }), &mut events);
        assert_eq!(ext.totals(), (0, 0));
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
    fn colours_from_extensions() {
        assert_eq!(parse_color("#fff"), 0xffffffff);
        assert_eq!(parse_color("#102030"), 0x102030ff);
        assert_eq!(parse_color("#10203040"), 0x10203040);
        assert_eq!(parse_color("rgba(1,2,3,1)"), 0, "not understood: left unset");
        assert_eq!(parse_color(""), 0);
        // Theme colours are guessed from their name.
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
}
