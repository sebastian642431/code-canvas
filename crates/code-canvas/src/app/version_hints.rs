use super::*;
use crate::versions::{self, npmrc::NpmConfig, Bulk, Op, Standing};

pub(super) const INSTALL: &str = "versions-install:";

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn status(note: &versions::Note) -> String {
    let latest = note.latest.as_deref().unwrap_or("");
    let mut text = match note.standing {
        Standing::Latest => format!("It takes the latest version, {latest}."),
        Standing::InRange => format!("The latest version, {latest}, is within what it asks for."),
        Standing::Behind => format!("The latest version, {latest}, is outside what it asks for."),
        Standing::NoMatch => format!("No published version matches it. The latest is {latest}."),
        Standing::Private => "Its scope goes to a private registry, which was not asked: no token for it.".to_string(),
        Standing::Unknown => "The registry does not know it.".to_string(),
    };
    if note.text.contains("no access") {
        text = "Its private registry refused the token.".to_string();
    }
    if let Some(pre) = note.choices.iter().find(|c| c.label == versions::PRERELEASE) {
        text.push_str(&format!(" Newest prerelease: {}.", pre.version));
    }
    text
}

fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl App {
    pub(super) fn read_dependencies(&mut self, page_id: u32) {
        const MAX_MANIFESTS: usize = 40;
        if !self.ui.prefs.check_versions {
            return;
        }
        let Some(page) = self.pages.iter().find(|p| p.id == page_id) else { return };
        let Some(r) = page.ready() else { return };
        let root = page.root.clone();
        for gone in self.manifests.keys().filter(|path| path.starts_with(&root)).cloned().collect::<Vec<_>>() {
            self.manifests.remove(&gone);
            self.ext.set_own_decorations(&gone.to_string_lossy(), "versions", Vec::new());
        }
        let mut all = Vec::new();
        let mut npm: HashMap<PathBuf, NpmConfig> = HashMap::new();
        let manifests = r.repo.files.iter().filter(|f| versions::is_manifest(&f.name));
        for f in manifests.take(MAX_MANIFESTS) {
            let mut found = versions::dependencies(&f.name, f.text());
            if found.is_empty() {
                continue;
            }
            let path = root.join(&f.rel_path);
            if f.name == "package.json" {
                let dir = path.parent().map_or_else(|| root.clone(), Path::to_path_buf);
                let config = npm.entry(dir.clone()).or_insert_with(|| NpmConfig::read(&dir, &root));
                for d in &mut found {
                    d.source = config.source(&d.name);
                }
            }
            all.extend(found.iter().cloned());
            self.manifests.insert(path, found);
        }
        self.versions.want(&all);
        self.show_versions();
    }

    pub(super) fn show_versions(&mut self) {
        let t = canvas_render::theme::get();
        let pre = self.ui.prefs.version_prereleases;
        for (path, wanted) in &self.manifests {
            let notes = self.versions.notes(wanted, pre);
            let items = notes
                .into_iter()
                .map(|n| ext::Decoration {
                    line: n.line,
                    col: 0,
                    end_line: n.line,
                    end_col: 0,
                    whole_line: false,
                    text: n.text,
                    color: match n.standing {
                        Standing::Latest => t.git_added,
                        Standing::InRange => t.problem_info,
                        Standing::Behind | Standing::NoMatch => t.problem_warning,
                        Standing::Private | Standing::Unknown => 0,
                    },
                    background: 0,
                })
                .collect();
            self.ext.set_own_decorations(&path.to_string_lossy(), "versions", items);
        }
    }

    fn version_target(&self, asked: Option<PathBuf>) -> Option<PathBuf> {
        if asked.is_some() {
            return asked;
        }
        let page = self.pages.get(self.active)?;
        let editing = page.editing.as_ref().map(|rel| page.root.join(rel));
        let mut candidates = editing.into_iter().chain(self.current_file());
        if let Some(path) = candidates.find(|p| versions::is_manifest(&file_name(p))) {
            return Some(path);
        }
        let mut here = self.manifests.keys().filter(|p| p.starts_with(&page.root));
        match (here.next(), here.next()) {
            (Some(only), None) => Some(only.clone()),
            _ => None,
        }
    }

    pub(super) fn versions_op(&mut self, op: Op) {
        const PICK: &str = "Select a manifest first: click its card or its name in the explorer, or edit it.";
        match op {
            Op::Set { path, line, name, text } => {
                if self.rewrite_manifest(&path, &[(line, name, text)]) > 0 {
                    self.offer_install(&path);
                }
            }
            Op::Update(_, _) if !self.ui.prefs.check_versions => {
                let text = "The versions of dependencies are not asked for: turn them on first (Versions: Show the Latest Versions in Manifests).";
                self.ext.notice(ext::Level::Info, "Versions", text.to_string());
            }
            Op::Update(bulk, asked) => {
                let Some(path) = self.version_target(asked) else {
                    return self.ext.notice(ext::Level::Info, "Versions", PICK.to_string());
                };
                let name = file_name(&path);
                let edits = match self.manifests.get(&path) {
                    Some(deps) => self.versions.updates(deps, bulk, self.ui.prefs.version_prereleases),
                    None => Vec::new(),
                };
                if edits.is_empty() {
                    let what = match bulk {
                        Bulk::Latest => "has a newer version to take",
                        Bulk::Major => "has a new major version",
                        Bulk::Minor => "has a newer version within its major version",
                        Bulk::Patch => "has a newer patch",
                    };
                    let text = format!("No dependency of {name} {what}.");
                    return self.ext.notice(ext::Level::Info, "Versions", text);
                }
                let changed = self.rewrite_manifest(&path, &edits);
                if changed > 0 {
                    let text = format!("{changed} dependencies of {name} now ask for newer versions. Undo takes them all back at once.");
                    self.ext.notice(ext::Level::Info, "Versions", text);
                    self.offer_install(&path);
                }
            }
            Op::Hints(on) => {
                self.apply(Action::SetPrefs(crate::prefs::Prefs { check_versions: on, ..self.ui.prefs.clone() }))
            }
            Op::Prereleases(on) => {
                self.apply(Action::SetPrefs(crate::prefs::Prefs { version_prereleases: on, ..self.ui.prefs.clone() }))
            }
            Op::Recheck => {
                self.versions.forget();
                self.versions_due = self.pages.iter().map(|p| p.id).collect();
                if !self.ui.prefs.check_versions {
                    let text = "Versions are not shown: turn them on in Settings, Help while editing.";
                    self.ext.notice(ext::Level::Info, "Versions", text.to_string());
                }
            }
            Op::Install(asked) => match self.version_target(asked) {
                Some(path) => self.run_install(&path),
                None => self.ext.notice(ext::Level::Info, "Versions", PICK.to_string()),
            },
        }
        self.request_redraw();
    }

    fn rewrite_manifest(&mut self, path: &Path, edits: &[(u32, String, String)]) -> usize {
        let Some((index, rel)) = self.locate_file(path) else { return 0 };
        let name = file_name(path);
        if index != self.active {
            self.switch_to(index);
        }
        let page = &mut self.pages[index];
        if !page.begin_edit(&rel) {
            let text = format!("{name} cannot be changed here.");
            self.ext.notice(ext::Level::Warn, "Versions", text);
            return 0;
        }
        let page_id = page.id;
        let Some(editor) = page.editor_mut() else { return 0 };
        let found = versions::dependencies(&name, &editor.display_source());
        let spans: Vec<_> = edits
            .iter()
            .filter_map(|(line, dep, text)| {
                let d = found.iter().find(|d| d.line == *line && d.name == *dep)?;
                let (row, col) = (d.line as usize, d.col as usize);
                let to = crate::editor::Pos::new(row, col + d.wanted.chars().count());
                Some((crate::editor::Pos::new(row, col), to, text.clone()))
            })
            .collect();
        if spans.is_empty() {
            return 0;
        }
        editor.replace_spans(&spans);
        log::info!("versions: {} requirements of {rel} rewritten", spans.len());
        if index == self.active {
            self.after_edit();
        }
        self.versions_due.push(page_id);
        spans.len()
    }

    fn offer_install(&mut self, path: &Path) {
        let Some(page) = self.pages.iter().find(|p| path.starts_with(&p.root)) else { return };
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let Some(command) = versions::install_command(path, &page.root, &text) else { return };
        let run = format!("Run {command}");
        let text = format!("{} asks for other versions now. {run} in the console to install them?", file_name(path));
        self.ext.ask("Versions", text, &[run.as_str(), "Not now"], &format!("{INSTALL}{}", path.display()));
    }

    pub(super) fn install_answered(&mut self, path: &str, choice: &str) {
        if choice.starts_with("Run ") {
            self.run_install(Path::new(path));
        }
    }

    fn run_install(&mut self, path: &Path) {
        let Some(page) = self.pages.iter().find(|p| path.starts_with(&p.root)) else { return };
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let Some(command) = versions::install_command(path, &page.root, &text) else {
            let text = format!("No install is known for {}.", file_name(path));
            return self.ext.notice(ext::Level::Info, "Versions", text);
        };
        let Some(dir) = path.parent().map(Path::to_path_buf) else { return };
        log::info!("versions: {command} in {}", dir.display());
        self.ui.show(Panel::Console);
        match self.ui.console.term.as_mut() {
            Some(term) => term.send(format!("cd {} && {command}\r", quoted(&dir)).as_bytes()),
            None => {
                self.console(ui::ConsoleOp::Start, Some(dir));
                if let Some(term) = self.ui.console.term.as_mut() {
                    term.send(format!("{command}\r").as_bytes());
                }
            }
        }
        self.request_redraw();
    }

    pub(super) fn version_click(&mut self, rel: &str, line: u32, point: [f32; 2]) {
        let Some(page) = self.pages.get(self.active) else { return };
        let path = page.root.join(rel);
        let root = page.root.clone();
        let Some(deps) = self.manifests.get(&path) else { return };
        let notes = self.versions.notes(deps, self.ui.prefs.version_prereleases);
        let Some(note) = notes.into_iter().find(|n| n.line == line) else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (scale, status) = (self.scale(), status(&note));
        self.ui.card_menu = None;
        self.ui.version_menu = Some(ui::VersionMenu {
            at: egui::Pos2::new(point[0] / scale, point[1] / scale + 6.0),
            install: versions::install_command(&path, &root, &text),
            path,
            line,
            name: note.name,
            wanted: note.wanted,
            status,
            choices: note.choices,
            place: None,
        });
        self.request_redraw();
    }

    pub(super) fn version_hit(&self, at: [f32; 2]) -> Option<(String, u32)> {
        let on_canvas = !self.over_ui(at);
        let hit = self.version_hits.iter().find(|(r, ..)| r.contains(at)).filter(|_| on_canvas);
        hit.map(|(_, rel, line)| (rel.clone(), *line))
    }

    pub(super) fn script_versions(&mut self, arg: &str) {
        let words: Vec<&str> = arg.split_whitespace().collect();
        let root = self.pages.get(self.active).map(|p| p.root.clone());
        let full = |rel: &str| root.as_ref().map(|root| root.join(rel));
        let on = |word: Option<&&str>| word.is_none_or(|w| *w == "on");
        match words.as_slice() {
            ["report", rel] => {
                let Some(deps) = full(rel).and_then(|path| self.manifests.get(&path)) else {
                    return log::warn!("script: no dependencies read from {rel}");
                };
                for n in self.versions.notes(deps, self.ui.prefs.version_prereleases) {
                    let choices: Vec<String> = n.choices.iter().map(|c| format!("{} {}", c.label, c.text)).collect();
                    log::info!(
                        "script: versions {rel} line {} {} {} -> {} ({:?}) [{}]",
                        n.line + 1,
                        n.name,
                        n.wanted,
                        n.text,
                        n.standing,
                        choices.join(" | ")
                    );
                }
                log::info!("script: versions {rel} reported, still asking: {}", self.versions.busy());
            }
            ["menu", rel, line] => {
                let line = line.parse::<u32>().unwrap_or(1).saturating_sub(1);
                let hit = self.version_hits.iter().find(|(_, r, l)| r == rel && *l == line).map(|(r, ..)| *r);
                match hit {
                    Some(rect) => self.version_click(rel, line, [rect.x + 2.0, rect.bottom()]),
                    None => log::warn!("script: no version is drawn on line {} of {rel}", line + 1),
                }
            }
            ["pick", n] => {
                let index = n.parse::<usize>().unwrap_or(1).saturating_sub(1);
                let picked = self.ui.version_menu.take().and_then(|menu| {
                    let choice = menu.choices.get(index)?;
                    Some(Op::Set { path: menu.path, line: menu.line, name: menu.name, text: choice.text.clone() })
                });
                match picked {
                    Some(op) => self.versions_op(op),
                    None => log::warn!("script: the version menu has no choice {n}"),
                }
            }
            ["update", kind, rel] => {
                let bulk = match *kind {
                    "major" => Bulk::Major,
                    "minor" => Bulk::Minor,
                    "patch" => Bulk::Patch,
                    _ => Bulk::Latest,
                };
                self.versions_op(Op::Update(bulk, full(rel)));
            }
            ["prereleases", rest @ ..] => self.versions_op(Op::Prereleases(on(rest.first()))),
            ["hints", rest @ ..] => self.versions_op(Op::Hints(on(rest.first()))),
            ["recheck"] => self.versions_op(Op::Recheck),
            _ => log::warn!("script: versions report|menu|pick|update|prereleases|hints|recheck, not '{arg}'"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_quoted_for_the_console() {
        assert_eq!(quoted(Path::new("C:\\work\\it's")), "'C:/work/it'\\''s'");
    }
}
