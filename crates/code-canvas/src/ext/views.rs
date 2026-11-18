//! Code lenses and tree views: what the app keeps of them and how it asks the host.
//!
//! Lenses are asked for the files whose code is readable on screen (the host's open
//! documents), kept per file until it is edited, changed on disk or the extensions say they
//! changed, and run by an id the host gives each one. Trees are asked row by row: the root
//! when a container's card is open, the children of a row when it is expanded.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{path_key, Container, Event, Extensions, Installed, Level, Pending};

/// The prefix of a rail item that opens a container of tree views.
pub const TREE: &str = "tree:";
/// How long a file being typed in rests before its lenses are asked again.
const LENS_REST_MS: u32 = 600;

#[derive(Clone, Debug, PartialEq)]
pub struct Lens {
    pub id: String,
    pub line: u32,
    pub col: u32,
    pub title: String,
    pub tooltip: String,
    pub runs: bool,
}

pub struct FileLenses {
    pub path: String,
    pub items: Vec<Lens>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeRow {
    pub handle: String,
    pub label: String,
    pub description: String,
    pub tooltip: String,
    pub icon: String,
    pub icon_color: u32,
    pub resource: String,
    /// 0: a leaf, 1: collapsed, 2: expanded at first.
    pub collapsible: u8,
    pub command: bool,
    pub context: String,
}

/// A row as the card shows it: how deep, whether it is open and still loading.
pub struct ShownRow {
    pub depth: u16,
    pub row: TreeRow,
    pub open: bool,
    pub loading: bool,
}

#[derive(Default)]
pub struct TreeState {
    pub registered: bool,
    pub missing: bool,
    pub message: String,
    pub title: String,
    pub description: String,
    children: HashMap<String, Vec<TreeRow>>,
    expanded: HashSet<String>,
    seen: HashSet<String>,
    loading: HashSet<String>,
    stale: HashSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuPlace {
    Title,
    Row,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub place: MenuPlace,
    pub command: String,
    pub title: String,
    pub codicon: Option<String>,
    pub when: String,
    pub group: String,
}

impl MenuItem {
    /// Shown as a button: the title's "navigation" group, a row's "inline" one. The rest is
    /// VS Code's "…" menu and right click, which the cards do not have.
    pub fn is_button(&self) -> bool {
        let group = self.group.split('@').next().unwrap_or_default();
        match self.place {
            MenuPlace::Title => group == "navigation",
            MenuPlace::Row => group == "inline",
        }
    }
}

pub enum Asked {
    Lenses { key: String, path: PathBuf },
    RunLens(String),
    TreeChildren { view: String, parent: String },
    TreeAct(String),
}

pub struct Views {
    pub lenses: HashMap<String, FileLenses>,
    lens_asked: HashMap<String, u64>,
    lens_fresh: HashSet<String>,
    show_lenses: bool,
    /// The id of the lens under the pointer, drawn brighter.
    pub lens_hover: Option<String>,
    pub trees: HashMap<String, TreeState>,
}

impl Default for Views {
    fn default() -> Self {
        Views {
            lenses: HashMap::new(),
            lens_asked: HashMap::new(),
            lens_fresh: HashSet::new(),
            show_lenses: true,
            lens_hover: None,
            trees: HashMap::new(),
        }
    }
}

fn command_looks(v: &Value, nls: &Value) -> HashMap<String, (String, Option<String>)> {
    let list = match v.pointer("/contributes/commands") {
        Some(Value::Array(a)) => a.clone(),
        Some(one @ Value::Object(_)) => vec![one.clone()],
        _ => Vec::new(),
    };
    list.iter()
        .filter_map(|c| {
            let id = c.get("command").and_then(Value::as_str)?;
            let title = c.get("title").map(|t| super::localized(t, nls)).unwrap_or_default();
            let icon = c.get("icon").and_then(Value::as_str).unwrap_or_default();
            let codicon = icon.strip_prefix("$(").and_then(|i| i.strip_suffix(')')).map(str::to_string);
            Some((id.to_string(), (title, codicon)))
        })
        .collect()
}

/// The buttons a manifest puts on its views: `view/title` and `view/item/context`.
pub fn manifest_menus(v: &Value, nls: &Value) -> Vec<MenuItem> {
    let looks = command_looks(v, nls);
    let mut out = Vec::new();
    for (key, place) in [("view/title", MenuPlace::Title), ("view/item/context", MenuPlace::Row)] {
        let Some(list) = v.pointer("/contributes/menus").and_then(|m| m.get(key)).and_then(Value::as_array) else {
            continue;
        };
        for item in list {
            let Some(command) = item.get("command").and_then(Value::as_str) else { continue };
            let (title, codicon) = looks.get(command).cloned().unwrap_or_else(|| (command.to_string(), None));
            let text = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            out.push(MenuItem {
                place,
                command: command.to_string(),
                title,
                codicon,
                when: text("when"),
                group: text("group"),
            });
        }
    }
    out
}

/// Takes the codicons out of a lens's title (`$(play) Run`): the canvas's font has none.
pub fn plain_title(title: &str) -> String {
    let mut out = String::new();
    let mut rest = title;
    while let Some(at) = rest.find("$(") {
        out.push_str(&rest[..at]);
        match rest[at..].find(')') {
            Some(end) => rest = &rest[at + end + 1..],
            None => rest = "",
        }
    }
    out.push_str(rest);
    // "▶︎ Run" (rust-analyzer): the canvas's fonts draw ▶ as a coloured emoji.
    let out = out.replace(['\u{fe0e}', '\u{fe0f}'], "").replace('▶', "▸");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tree_row(v: &Value) -> TreeRow {
    let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    TreeRow {
        handle: s("handle"),
        label: s("label"),
        description: s("description"),
        tooltip: s("tooltip"),
        icon: s("icon"),
        icon_color: super::parse_color(&s("iconColor")),
        resource: s("resource"),
        collapsible: v.get("collapsible").and_then(Value::as_u64).unwrap_or(0).min(2) as u8,
        command: v.get("command").and_then(Value::as_bool).unwrap_or(false),
        context: s("context"),
    }
}

fn walk(state: &TreeState, parent: &str, depth: u16, out: &mut Vec<ShownRow>) {
    let Some(rows) = state.children.get(parent) else { return };
    for row in rows {
        let open = row.collapsible > 0 && state.expanded.contains(&row.handle);
        let loading = open && !state.children.contains_key(&row.handle);
        out.push(ShownRow { depth, row: row.clone(), open, loading });
        if open {
            walk(state, &row.handle, depth + 1, out);
        }
    }
}

impl Extensions {
    fn ask_host(&mut self, method: &str, params: Value, asked: Asked) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, Pending::View(asked));
        self.send(json!({ "type": "request", "id": id, "method": method, "params": params }));
        id
    }

    // ---- Code lenses ----

    pub fn set_show_lenses(&mut self, on: bool) {
        if self.views.show_lenses == on {
            return;
        }
        self.views.show_lenses = on;
        self.views.lenses.clear();
        self.views.lens_asked.clear();
        self.views.lens_fresh.clear();
        self.revision += 1;
    }

    fn ask_lenses(&mut self, path: &Path, wait: u32) {
        if !self.views.show_lenses || self.host.is_none() {
            return;
        }
        let key = path_key(&path.to_string_lossy());
        self.views.lens_fresh.insert(key.clone());
        let asked = Asked::Lenses { key: key.clone(), path: path.to_path_buf() };
        let id = self.ask_host("codeLenses", json!({ "path": path, "wait": wait }), asked);
        self.views.lens_asked.insert(key, id);
    }

    /// The files readable on screen: those with no answer, or an old one, are asked.
    pub(super) fn lenses_for(&mut self, visible: &[PathBuf]) {
        for path in visible {
            if !self.views.lens_fresh.contains(&path_key(&path.to_string_lossy())) {
                self.ask_lenses(path, 0);
            }
        }
    }

    pub(super) fn lenses_edited(&mut self, path: &Path) {
        self.ask_lenses(path, LENS_REST_MS);
    }

    pub(super) fn lenses_stale(&mut self, path: &Path) {
        self.views.lens_fresh.remove(&path_key(&path.to_string_lossy()));
    }

    pub(super) fn forget_lenses(&mut self, path: &Path) {
        let key = path_key(&path.to_string_lossy());
        self.views.lenses.remove(&key);
        self.views.lens_fresh.remove(&key);
        self.views.lens_asked.remove(&key);
    }

    pub(super) fn clear_views(&mut self) {
        self.views.lenses.clear();
        self.views.lens_asked.clear();
        self.views.lens_fresh.clear();
        self.views.lens_hover = None;
        self.views.trees.clear();
    }

    pub fn lenses_under<'a>(&'a self, root: &Path) -> impl Iterator<Item = (&'a str, &'a FileLenses)> + 'a {
        let prefix = super::folder_prefix(root);
        let len = prefix.len();
        let show = self.views.show_lenses;
        self.views
            .lenses
            .iter()
            .filter(move |(key, l)| show && key.starts_with(&prefix) && l.path.len() == key.len())
            .map(move |(_, l)| (&l.path[len..], l))
    }

    /// The lenses of a line of a file (0-based), in the order they are drawn.
    pub fn lenses_at(&self, path: &Path, line: u32) -> Vec<Lens> {
        let key = path_key(&path.to_string_lossy());
        let Some(file) = self.views.lenses.get(&key) else { return Vec::new() };
        let mut found: Vec<Lens> = file.items.iter().filter(|l| l.line == line).cloned().collect();
        found.sort_by_key(|l| l.col);
        found
    }

    /// The lines of a file that have lenses (0-based).
    pub fn lens_lines(&self, path: &Path) -> Vec<u32> {
        let key = path_key(&path.to_string_lossy());
        let mut lines: Vec<u32> =
            self.views.lenses.get(&key).map_or_else(Vec::new, |f| f.items.iter().map(|l| l.line).collect());
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    pub fn run_lens(&mut self, lens: &Lens) {
        if self.host.is_none() {
            return;
        }
        let title = plain_title(&lens.title);
        log::info!("running the code lens '{title}'");
        self.ask_host("runLens", json!({ "id": lens.id }), Asked::RunLens(title));
    }

    // ---- Tree views ----

    /// The container a rail item `tree:<id>` opens, with the extension it is of.
    pub fn tree_container(&self, id: &str) -> Option<(&Installed, &Container)> {
        self.installed
            .iter()
            .filter(|e| e.enabled && e.has_code)
            .find_map(|e| Some((e, e.containers.iter().find(|c| c.id == id)?)))
    }

    /// The tree views of a container whose `when` holds.
    pub fn tree_views(&self, container: &Container) -> Vec<super::ViewDecl> {
        let get = |key: &str| self.contexts.get(key).cloned();
        container.views.iter().filter(|v| !v.webview && super::when::eval(&v.when, &get)).cloned().collect()
    }

    /// The buttons of a view's title, or of one of its rows (`row` is its context value).
    pub fn view_buttons(&self, extension: &str, view: &str, row: Option<&str>) -> Vec<MenuItem> {
        let Some(e) = self.installed.iter().find(|e| e.id == extension) else { return Vec::new() };
        let place = if row.is_some() { MenuPlace::Row } else { MenuPlace::Title };
        let get = |key: &str| match (key, row) {
            ("view", _) => Some(Value::String(view.to_string())),
            ("viewItem", Some(context)) => Some(Value::String(context.to_string())),
            _ => self.contexts.get(key).cloned(),
        };
        e.menus
            .iter()
            .filter(|m| m.place == place && m.is_button() && super::when::eval(&m.when, &get))
            .cloned()
            .collect()
    }

    fn ask_tree(&mut self, view: &str, parent: &str) {
        let state = self.views.trees.entry(view.to_string()).or_default();
        if state.loading.contains(parent) {
            return;
        }
        state.loading.insert(parent.to_string());
        state.stale.remove(parent);
        let params = match parent {
            "" => json!({ "view": view }),
            _ => json!({ "view": view, "parent": parent }),
        };
        self.ask_host(
            "treeChildren",
            params,
            Asked::TreeChildren { view: view.to_string(), parent: parent.to_string() },
        );
    }

    /// What a view's card shows now. Asks for what is missing or old: the root, and the
    /// children of every open row.
    pub fn tree_rows(&mut self, view: &str) -> Vec<ShownRow> {
        if self.host.is_none() {
            return Vec::new();
        }
        let state = self.views.trees.entry(view.to_string()).or_default();
        let mut wanted: Vec<String> = Vec::new();
        if !state.missing && (!state.children.contains_key("") || state.stale.contains("")) {
            wanted.push(String::new());
        }
        let mut shown = Vec::new();
        walk(state, "", 0, &mut shown);
        for s in shown.iter().filter(|s| s.open) {
            if s.loading || state.stale.contains(&s.row.handle) {
                wanted.push(s.row.handle.clone());
            }
        }
        for parent in wanted {
            self.ask_tree(view, &parent);
        }
        shown
    }

    pub fn tree_state(&self, view: &str) -> Option<&TreeState> {
        self.views.trees.get(view)
    }

    pub fn toggle_tree_row(&mut self, view: &str, handle: &str) {
        let state = self.views.trees.entry(view.to_string()).or_default();
        let open = !state.expanded.remove(handle);
        if open {
            state.expanded.insert(handle.to_string());
        }
        log::info!("{} the row {handle} of {view}", if open { "expanded" } else { "collapsed" });
        let params = json!({ "view": view, "handle": handle, "expanded": open });
        self.ask_host("treeExpand", params, Asked::TreeAct(view.to_string()));
    }

    pub fn click_tree_row(&mut self, view: &str, row: &TreeRow) {
        log::info!("clicked the row '{}' of {view}", row.label);
        let params = json!({ "view": view, "handle": row.handle });
        self.ask_host("treeClick", params, Asked::TreeAct(row.label.clone()));
    }

    pub fn tree_command(&mut self, view: &str, handle: Option<&str>, item: &MenuItem) {
        log::info!("the button '{}' of {view} runs {}", item.title, item.command);
        let mut params = json!({ "view": view, "command": item.command });
        if let Some(handle) = handle {
            params["handle"] = json!(handle);
        }
        self.ask_host("treeCommand", params, Asked::TreeAct(item.title.clone()));
    }

    /// Every row loaded of a view, depth first, as `label (description)`: for the log.
    pub fn tree_report(&self, view: &str) -> Vec<String> {
        let Some(state) = self.views.trees.get(view) else { return Vec::new() };
        let mut shown = Vec::new();
        walk(state, "", 0, &mut shown);
        shown
            .iter()
            .map(|s| {
                let mark = match (s.row.collapsible, s.open) {
                    (0, _) => "",
                    (_, true) => "v ",
                    (_, false) => "> ",
                };
                format!("{}{mark}{}", "  ".repeat(s.depth as usize), s.row.label)
            })
            .collect()
    }

    // ---- Messages ----

    pub(super) fn on_view_message(&mut self, kind: &str, m: &Value, events: &mut Vec<Event>) -> bool {
        let s = |key: &str| m.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        match kind {
            "lensesChanged" => {
                self.views.lens_fresh.clear();
            }
            "showUses" => {
                let n = |key: &str| m.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
                events.push(Event::ShowUses { path: PathBuf::from(s("path")), line: n("line"), col: n("col") });
            }
            "treeView" => {
                let state = self.views.trees.entry(s("view")).or_default();
                state.registered = true;
                state.missing = false;
                state.stale.extend(state.children.keys().cloned());
                state.stale.insert(String::new());
            }
            "treeChanged" => {
                if let Some(state) = self.views.trees.get_mut(&s("view")) {
                    let loaded: Vec<String> = state.children.keys().chain(state.loading.iter()).cloned().collect();
                    state.stale.extend(loaded);
                }
            }
            "treeInfo" => {
                let state = self.views.trees.entry(s("view")).or_default();
                state.title = s("title");
                state.description = s("description");
                state.message = s("message");
            }
            "treeGone" => {
                if let Some(state) = self.views.trees.get_mut(&s("view")) {
                    state.registered = false;
                    state.children.clear();
                    state.stale.clear();
                    state.missing = true;
                }
            }
            _ => return false,
        }
        true
    }

    pub(super) fn on_view_reply(&mut self, id: u64, asked: Asked, m: &Value) {
        let error = m.get("error").and_then(Value::as_str).map(str::to_string);
        let result = m.get("result").cloned().unwrap_or(Value::Null);
        match asked {
            Asked::Lenses { key, path } => {
                if self.views.lens_asked.get(&key) != Some(&id) || result.get("superseded").is_some() {
                    return;
                }
                self.views.lens_asked.remove(&key);
                if let Some(error) = error {
                    log::warn!("the code lenses of {} could not be asked: {error}", path.display());
                    return;
                }
                let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
                let t = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                let items: Vec<Lens> = result
                    .get("items")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|v| Lens {
                                id: t(v, "id"),
                                line: n(v, "line"),
                                col: n(v, "col"),
                                title: t(v, "title"),
                                tooltip: t(v, "tooltip"),
                                runs: v.get("runs").and_then(Value::as_bool).unwrap_or(false),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !items.is_empty() {
                    log::info!("{} code lenses for {name}", items.len());
                }
                let shown = path.to_string_lossy().replace('\\', "/");
                self.views.lenses.insert(key, FileLenses { path: shown, items });
                self.revision += 1;
            }
            Asked::RunLens(title) => {
                if let Some(error) = error {
                    self.notice(Level::Warn, &title, error);
                }
            }
            Asked::TreeChildren { view, parent } => {
                let state = self.views.trees.entry(view.clone()).or_default();
                state.loading.remove(&parent);
                if let Some(error) = error {
                    log::warn!("the tree {view} could not be asked: {error}");
                    state.children.insert(parent, Vec::new());
                    return;
                }
                if result.get("missing").is_some() {
                    state.missing = true;
                    return;
                }
                if result.get("gone").is_some() {
                    state.children.remove(&parent);
                    state.expanded.remove(&parent);
                    return;
                }
                let rows: Vec<TreeRow> = result
                    .get("items")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().map(tree_row).collect())
                    .unwrap_or_default();
                for row in &rows {
                    if state.seen.insert(row.handle.clone()) && row.collapsible == 2 {
                        state.expanded.insert(row.handle.clone());
                    }
                }
                if parent.is_empty() {
                    let text = |k: &str| result.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                    state.message = text("message");
                    state.title = text("title");
                    state.description = text("description");
                    log::info!("the tree {view} shows {} rows at its root", rows.len());
                }
                state.children.insert(parent, rows);
            }
            Asked::TreeAct(title) => {
                if let Some(error) = error {
                    self.notice(Level::Warn, &title, error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lens_titles_lose_their_codicons() {
        assert_eq!(plain_title("$(play) Run"), "Run");
        assert_eq!(plain_title("▶\u{fe0e} Run Test"), "▸ Run Test");
        assert_eq!(plain_title("3 references $(x"), "3 references");
    }

    #[test]
    fn menus_of_a_manifest() {
        let nls = json!({ "refresh": "Refresh" });
        let manifest = json!({ "contributes": {
            "commands": [
                { "command": "x.refresh", "title": "%refresh%", "icon": "$(refresh)" },
                { "command": "x.drop", "title": "Drop", "icon": { "light": "a.svg", "dark": "b.svg" } }
            ],
            "menus": {
                "view/title": [
                    { "command": "x.refresh", "when": "view == x.list", "group": "navigation@1" },
                    { "command": "x.drop", "when": "view == x.list", "group": "2_more" }
                ],
                "view/item/context": [{ "command": "x.drop", "when": "viewItem == leaf", "group": "inline" }],
                "commandPalette": [{ "command": "x.drop", "when": "false" }]
            }
        }});
        let menus = manifest_menus(&manifest, &nls);
        assert_eq!(menus.len(), 3);
        assert_eq!((menus[0].title.as_str(), menus[0].codicon.as_deref()), ("Refresh", Some("refresh")));
        assert!(menus[0].is_button() && !menus[1].is_button());
        assert_eq!((menus[2].place, menus[2].codicon.clone(), menus[2].is_button()), (MenuPlace::Row, None, true));
    }

    #[test]
    fn rows_follow_what_is_expanded() {
        let mut state = TreeState::default();
        let row = |handle: &str, collapsible: u8| TreeRow {
            handle: handle.into(),
            label: handle.rsplit('/').next().unwrap().into(),
            collapsible,
            ..Default::default()
        };
        state.children.insert(String::new(), vec![row("/a", 1), row("/b", 2), row("/c", 0)]);
        state.children.insert("/b".into(), vec![row("/b/x", 0)]);
        state.expanded.insert("/b".into());
        state.expanded.insert("/a".into());
        let mut shown = Vec::new();
        walk(&state, "", 0, &mut shown);
        let seen: Vec<(u16, &str, bool, bool)> =
            shown.iter().map(|s| (s.depth, s.row.label.as_str(), s.open, s.loading)).collect();
        assert_eq!(
            seen,
            vec![(0, "a", true, true), (0, "b", true, false), (1, "x", false, false), (0, "c", false, false)]
        );
    }
}
