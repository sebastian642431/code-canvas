//! The page of one extension, installed or not: VS Code's "Extension: Name" tab. The card
//! draws its head (icon, name, numbers, buttons, whether it works here); the README, the
//! changelog and what it brings are a page of HTML in a webview below it, made here.

use std::sync::Arc;

use serde_json::Value;

use super::gallery::{Got, Manifest};
use super::market::{self, MarketDetails};

/// The webview the page is shown in.
pub const PAGE: &str = "extension-page";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Details,
    Features,
    Changelog,
    /// What it does now it is installed: started or not, its views and commands.
    Runtime,
}

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Details => "Details",
            Tab::Features => "Features",
            Tab::Changelog => "Changelog",
            Tab::Runtime => "Runtime Status",
        }
    }

    pub fn from_word(word: &str) -> Option<Tab> {
        Some(match word.to_ascii_lowercase().as_str() {
            "details" | "readme" => Tab::Details,
            "features" => Tab::Features,
            "changelog" => Tab::Changelog,
            "runtime" | "status" => Tab::Runtime,
            _ => return None,
        })
    }

    /// Whether it is shown as a page of HTML (the others are drawn by the card).
    pub fn in_page(self) -> bool {
        self != Tab::Runtime
    }
}

/// Everything the page of one extension shows, from the registry and from its folder.
#[derive(Clone, Debug)]
pub struct View {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub market: Got<Arc<MarketDetails>>,
    pub manifest: Got<Arc<Manifest>>,
    pub readme: Got<Arc<String>>,
    /// Where the README's relative links start.
    pub readme_base: String,
    pub changelog: Got<Arc<String>>,
    pub changelog_base: String,
    pub first_released: Got<String>,
    /// The version installed, when it is.
    pub installed: Option<String>,
}

impl View {
    /// What tells whether the page must be made again.
    pub fn stamp(&self, tab: Tab) -> String {
        format!(
            "{}|{tab:?}|{}|{}|{}|{}|{}|{:?}|{}",
            self.id,
            self.market.stage(),
            self.manifest.stage(),
            self.readme.stage(),
            self.changelog.stage(),
            self.first_released.stage(),
            self.installed,
            canvas_render::theme::revision()
        )
    }
}

pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn link(url: &str, label: &str) -> String {
    let safe = url.starts_with("https://") || url.starts_with("http://");
    if safe {
        format!("<a href=\"{}\">{}</a>", esc(url), esc(label))
    } else {
        esc(label)
    }
}

/// The folder of an address, for a page's relative links.
pub fn folder_of(url: &str) -> String {
    match url.rfind('/') {
        Some(at) => url[..=at].to_string(),
        None => String::new(),
    }
}

const STYLE: &str = r#"
  body > article { max-width: 1120px; }
  .ext { display: grid; grid-template-columns: minmax(0, 1fr); gap: 18px; }
  .ext > aside { order: -1; display: flex; flex-direction: column; gap: 8px; padding-bottom: 12px;
    border-bottom: 1px solid var(--border); font-size: 12.5px; }
  aside h4 { display: none; margin: 0 0 6px; font-size: 12px; font-weight: 600; color: var(--text);
    text-transform: uppercase; letter-spacing: 0.04em; }
  aside dl { display: flex; flex-wrap: wrap; gap: 2px 18px; margin: 0; }
  aside dl > div { display: flex; gap: 6px; }
  aside dt { color: var(--dim); }
  aside dd { margin: 0; overflow-wrap: anywhere; }
  aside .links { display: flex; flex-wrap: wrap; gap: 2px 16px; }
  aside .tags { display: none; }
  aside p { margin: 4px 0 0; }
  @media (min-width: 720px) {
    .ext { grid-template-columns: minmax(0, 1fr) 220px; gap: 28px; }
    .ext > aside { order: 0; display: block; padding: 0 0 0 18px; border: 0; border-left: 1px solid var(--border); }
    .ext > aside section + section { margin-top: 18px; }
    aside h4, aside .tags { display: block; }
    aside dl { display: grid; grid-template-columns: auto 1fr; gap: 3px 12px; }
    aside dl > div { display: contents; }
    aside .links { display: block; }
    aside .links a { display: block; line-height: 1.7; }
  }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .chips span { padding: 1px 8px; border-radius: 10px; background: var(--well); font-size: 12px; }
  .dim { color: var(--dim); }
  .badge { display: inline-block; margin-left: 8px; padding: 1px 8px; border-radius: 10px; font-size: 11.5px;
    font-weight: 500; vertical-align: middle; border: 1px solid var(--border); }
  .badge.works { color: var(--accent); }
  .badge.partly { color: var(--dim); }
  .badge.not { color: var(--warn); border-color: var(--warn); }
  h3 { margin-top: 28px; }
  h3:first-child { margin-top: 0; }
  td code, li code { white-space: nowrap; }
  .events code { display: inline-block; margin: 0 4px 4px 0; }
"#;

/// The whole page of a tab of the view.
pub fn page_html(view: &View, tab: Tab) -> String {
    let (article, base) = match tab {
        Tab::Changelog => (text_part(&view.changelog, "changelog"), view.changelog_base.clone()),
        Tab::Features => (features_html(view), String::new()),
        _ => {
            let readme = text_part(&view.readme, "README");
            (
                format!("<div class=\"ext\"><div class=\"readme\">{readme}</div>{}</div>", aside(view)),
                view.readme_base.clone(),
            )
        }
    };
    crate::markdown::document(&article, &base, &format!("{} {tab:?}", view.id), STYLE)
}

/// Whether the registry said it does not know the extension (or the file).
pub fn not_found(why: &str) -> bool {
    why == "not on the registry" || why.starts_with("Extension not found")
}

fn text_part(got: &Got<Arc<String>>, what: &str) -> String {
    match got {
        Got::Ready(text) if text.trim().is_empty() => format!("<p class=\"dim\">Its {what} is empty.</p>"),
        Got::Ready(text) => crate::markdown::body(text),
        Got::Loading => format!("<p class=\"dim\">Getting its {what} …</p>"),
        Got::Failed(why) if why == "unknown" => {
            format!("<p class=\"dim\">Open VSX does not know this extension: there is no {what} to show.</p>")
        }
        Got::Failed(why) if why == "none" || not_found(why) => format!("<p class=\"dim\">It has no {what}.</p>"),
        Got::Failed(why) => format!("<p class=\"dim\">Its {what} could not be read: {}</p>", esc(why)),
    }
}

fn aside(view: &View) -> String {
    let mut info: Vec<(&str, String)> = vec![("Identifier", esc(&view.id))];
    let market = view.market.ready();
    match (view.installed.as_ref(), market) {
        (Some(here), Some(d)) if *here != d.item.version => info.push(("Installed", esc(here))),
        (Some(here), None) => info.push(("Installed", esc(here))),
        _ => {}
    }
    if let Some(d) = market {
        info.push(("Version", esc(&d.item.version)));
        if let Some(first) = view.first_released.ready().map(|at| market::short_date(at)).filter(|d| !d.is_empty()) {
            info.push(("Published", first));
        }
        let last = market::short_date(&d.released);
        if !last.is_empty() {
            info.push(("Last released", last));
        }
        if let Some(size) = d.size {
            info.push(("Size", market::size_words(size)));
        }
        if !d.license.is_empty() {
            info.push(("License", link(&d.license_file, &d.license)));
        }
    }
    let mut out = String::from("<aside><section><h4>Open VSX</h4><dl>");
    for (term, value) in info {
        out.push_str(&format!("<div><dt>{term}</dt><dd>{value}</dd></div>"));
    }
    out.push_str("</dl>");
    match &view.market {
        Got::Loading => out.push_str("<p class=\"dim\">Asking the registry …</p>"),
        Got::Failed(why) if not_found(why) && view.installed.is_some() => {
            out.push_str("<p class=\"dim\">Not on Open VSX: installed from a file.</p>")
        }
        Got::Failed(why) if not_found(why) => out.push_str("<p class=\"dim\">Open VSX does not know it.</p>"),
        Got::Failed(why) => {
            out.push_str(&format!("<p class=\"dim\">The registry could not be reached: {}</p>", esc(why)))
        }
        Got::Ready(_) => {}
    }
    out.push_str("</section>");
    if let Some(d) = market {
        let chips =
            |list: &[String]| list.iter().take(16).map(|c| format!("<span>{}</span>", esc(c))).collect::<String>();
        if !d.categories.is_empty() {
            out.push_str(&format!(
                "<section><h4>Categories</h4><div class=\"chips\">{}</div></section>",
                chips(&d.categories)
            ));
        }
        if !d.tags.is_empty() {
            let tags = chips(&d.tags);
            out.push_str(&format!("<section class=\"tags\"><h4>Tags</h4><div class=\"chips\">{tags}</div></section>"));
        }
        let mut links = vec![link(&market::site_url(&view.namespace, &view.name), "Open VSX page")];
        for (url, label) in [
            (&d.repository, "Repository"),
            (&d.bugs, "Issues"),
            (&d.homepage, "Homepage"),
            (&d.license_file, "License"),
        ] {
            if url.starts_with("http") {
                links.push(link(url, label));
            }
        }
        out.push_str(&format!("<section><h4>Resources</h4><div class=\"links\">{}</div></section>", links.join("")));
    }
    out.push_str("</aside>");
    out
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Works {
    Yes,
    Partly,
    No,
}

/// One kind of thing an extension brings, as the Features tab shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub title: &'static str,
    pub works: Works,
    pub note: &'static str,
    pub head: Vec<&'static str>,
    pub rows: Vec<Vec<String>>,
}

fn plain(raw: &str) -> String {
    raw.strip_prefix('%').and_then(|r| r.strip_suffix('%')).unwrap_or(raw).to_string()
}

fn words(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => plain(s),
        Some(Value::Object(o)) => o.get("value").and_then(Value::as_str).map(plain).unwrap_or_default(),
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(plain).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    }
}

fn list(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// What a manifest contributes, kind by kind, and whether each works in Hawk Canvas.
pub fn features(manifest: &Value) -> Vec<Feature> {
    let c = manifest.get("contributes").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();
    let mut push = |title, works, note, head: Vec<&'static str>, rows: Vec<Vec<String>>| {
        if !rows.is_empty() {
            out.push(Feature { title, works, note, head, rows });
        }
    };
    let commands = match c.get("commands") {
        Some(Value::Object(one)) => vec![Value::Object(one.clone())],
        other => list(other).to_vec(),
    };
    let rows = commands
        .iter()
        .filter_map(|cmd| {
            let id = cmd.get("command")?.as_str()?.to_string();
            let title = words(cmd.get("title"));
            let category = words(cmd.get("category"));
            Some(vec![if category.is_empty() { title } else { format!("{category}: {title}") }, id])
        })
        .collect();
    push(
        "Commands",
        Works::Yes,
        "They run from its page and from the canvas's command list.",
        vec!["Title", "ID"],
        rows,
    );

    let sections = match c.get("configuration") {
        Some(Value::Array(all)) => all.clone(),
        Some(one) => vec![one.clone()],
        None => Vec::new(),
    };
    let mut rows = Vec::new();
    for section in &sections {
        let Some(Value::Object(props)) = section.get("properties") else { continue };
        for (key, schema) in props {
            let about = words(schema.get("markdownDescription").or(schema.get("description")));
            let about: String = about.chars().take(220).collect();
            let default = schema.get("default").map(|d| d.to_string()).unwrap_or_default();
            rows.push(vec![key.clone(), about, default]);
        }
    }
    push(
        "Settings",
        Works::Partly,
        "Their defaults hold; to change one, write it in settings.json in the app's data folder (no page for them yet).",
        vec!["ID", "Description", "Default"],
        rows,
    );

    let rows = list(c.get("languages"))
        .iter()
        .filter_map(|l| {
            let id = l.get("id")?.as_str()?.to_string();
            let alias = list(l.get("aliases")).first().and_then(Value::as_str).unwrap_or_default().to_string();
            let mut files: Vec<String> =
                list(l.get("extensions")).iter().filter_map(Value::as_str).map(str::to_string).collect();
            files.extend(list(l.get("filenames")).iter().filter_map(Value::as_str).map(str::to_string));
            Some(vec![id, alias, files.join(" ")])
        })
        .collect();
    push("Languages", Works::Yes, "", vec!["ID", "Name", "Files"], rows);

    let rows = list(c.get("grammars"))
        .iter()
        .filter_map(|g| {
            let scope = g.get("scopeName")?.as_str()?.to_string();
            Some(vec![g.get("language").and_then(Value::as_str).unwrap_or("(injected)").to_string(), scope])
        })
        .collect();
    push("Grammars", Works::Yes, "They colour the code on the canvas.", vec!["Language", "Scope"], rows);

    let mut rows = Vec::new();
    if let Some(Value::Object(places)) = c.get("views") {
        for (place, views) in places {
            for v in list(Some(views)) {
                let kind = if v.get("type").and_then(Value::as_str) == Some("webview") { "page" } else { "tree" };
                rows.push(vec![words(v.get("name")), place.clone(), kind.to_string()]);
            }
        }
    }
    push("Views", Works::Yes, "Each opens from its icon in the right rail.", vec!["Name", "Container", "Kind"], rows);

    let rows = [("themes", "colour theme"), ("iconThemes", "file icon theme")]
        .iter()
        .flat_map(|(key, kind)| {
            list(c.get(*key)).iter().map(move |t| vec![words(t.get("label").or(t.get("id"))), kind.to_string()])
        })
        .collect();
    push("Themes", Works::Yes, "Chosen in the settings.", vec!["Name", "Kind"], rows);

    let rows = list(c.get("keybindings"))
        .iter()
        .map(|k| {
            let key = k.get("win").or(k.get("key")).and_then(Value::as_str).unwrap_or_default().to_string();
            vec![key, words(k.get("command")), words(k.get("when"))]
        })
        .collect();
    push(
        "Keyboard shortcuts",
        Works::No,
        "Its commands run from its page instead.",
        vec!["Key", "Command", "When"],
        rows,
    );

    let rows = list(c.get("customEditors"))
        .iter()
        .map(|e| {
            let files: Vec<String> = list(e.get("selector"))
                .iter()
                .filter_map(|s| s.get("filenamePattern")?.as_str().map(str::to_string))
                .collect();
            vec![words(e.get("displayName")), words(e.get("viewType")), files.join(" ")]
        })
        .collect();
    push("Custom editors", Works::No, "The files open as text on the canvas.", vec!["Name", "ID", "Files"], rows);

    let rows = list(c.get("snippets")).iter().map(|s| vec![words(s.get("language")), words(s.get("path"))]).collect();
    push("Snippets", Works::No, "", vec!["Language", "File"], rows);

    let rows = list(c.get("debuggers")).iter().map(|d| vec![words(d.get("label")), words(d.get("type"))]).collect();
    push("Debuggers", Works::No, "", vec!["Name", "Type"], rows);

    let rows = list(c.get("taskDefinitions")).iter().map(|t| vec![words(t.get("type"))]).collect();
    push("Tasks", Works::No, "", vec!["Type"], rows);

    let rows = match c.get("menus") {
        Some(Value::Object(menus)) => menus
            .iter()
            .filter(|(place, items)| {
                !["view/title", "view/item/context", "commandPalette"].contains(&place.as_str())
                    && !list(Some(items)).is_empty()
            })
            .map(|(place, items)| vec![place.clone(), list(Some(items)).len().to_string()])
            .collect(),
        _ => Vec::new(),
    };
    push(
        "Menu entries",
        Works::No,
        "Hawk Canvas shows the buttons of its views' titles and rows; these places are not there yet.",
        vec!["Place", "Entries"],
        rows,
    );

    let rows =
        ["notebooks", "notebookRenderer", "terminal", "authentication", "chatParticipants", "languageModelTools"]
            .iter()
            .flat_map(|key| {
                list(c.get(*key)).iter().map(move |item| {
                    let name =
                        words(item.get("displayName").or(item.get("label")).or(item.get("name")).or(item.get("id")));
                    vec![key.to_string(), name]
                })
            })
            .collect();
    push("Other", Works::No, "", vec!["Kind", "Name"], rows);
    out
}

fn badge(works: Works) -> &'static str {
    match works {
        Works::Yes => "<span class=\"badge works\">Works</span>",
        Works::Partly => "<span class=\"badge partly\">Partly</span>",
        Works::No => "<span class=\"badge not\">Not in Hawk Canvas yet</span>",
    }
}

fn features_html(view: &View) -> String {
    let manifest = match &view.manifest {
        Got::Ready(m) => m,
        Got::Loading => return "<p class=\"dim\">Reading its manifest …</p>".into(),
        Got::Failed(why) if not_found(why) => {
            return "<p class=\"dim\">Open VSX does not know this extension.</p>".into()
        }
        Got::Failed(why) => return format!("<p class=\"dim\">Its manifest could not be read: {}</p>", esc(why)),
    };
    let mut out = String::new();
    let json = &manifest.json;
    let main = json.get("main").and_then(Value::as_str).is_some_and(|m| !m.is_empty());
    let events: Vec<&str> = list(json.get("activationEvents")).iter().filter_map(Value::as_str).collect();
    out.push_str("<h3>Activation</h3>");
    if !main {
        out.push_str("<p>It has no code of its own: what it brings works without Node.js.</p>");
    } else if events.is_empty() {
        out.push_str("<p>Its code starts when one of its commands or views is used (Node.js runs it).</p>");
    } else {
        let shown: String = events.iter().take(40).map(|e| format!("<code>{}</code>", esc(e))).collect();
        out.push_str(&format!("<p>Its code (run by Node.js) starts on:</p><p class=\"events\">{shown}</p>"));
    }
    let all = features(json);
    if all.is_empty() {
        out.push_str("<p class=\"dim\">It contributes nothing VS Code lists as a feature.</p>");
    }
    for f in all {
        out.push_str(&format!("<h3>{} <span class=\"dim\">({})</span>{}</h3>", f.title, f.rows.len(), badge(f.works)));
        if !f.note.is_empty() {
            out.push_str(&format!("<p class=\"dim\">{}</p>", esc(f.note)));
        }
        out.push_str("<table><thead><tr>");
        for h in &f.head {
            out.push_str(&format!("<th>{h}</th>"));
        }
        out.push_str("</tr></thead><tbody>");
        for row in f.rows.iter().take(400) {
            out.push_str("<tr>");
            for (i, cell) in row.iter().enumerate() {
                let code = f.head.get(i).is_some_and(|h| matches!(*h, "ID" | "Key" | "Scope" | "Type"));
                if code && !cell.is_empty() {
                    out.push_str(&format!("<td><code>{}</code></td>", esc(cell)));
                } else {
                    out.push_str(&format!("<td>{}</td>", esc(cell)));
                }
            }
            out.push_str("</tr>");
        }
        out.push_str("</tbody></table>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(manifest: Value) -> View {
        View {
            id: "acme.demo".into(),
            namespace: "acme".into(),
            name: "demo".into(),
            market: Got::Loading,
            manifest: Got::Ready(Arc::new(Manifest::new(manifest))),
            readme: Got::Ready(Arc::new("# Demo\n\n<script>alert(1)</script>\n".into())),
            readme_base: "https://r/file/".into(),
            changelog: Got::Failed("none".into()),
            changelog_base: String::new(),
            first_released: Got::Loading,
            installed: None,
        }
    }

    #[test]
    fn features_say_what_works() {
        let manifest = json!({ "contributes": {
            "commands": [{ "command": "demo.run", "title": "%run.title%", "category": "Demo" }],
            "keybindings": [{ "command": "demo.run", "key": "ctrl+r", "win": "ctrl+alt+r" }],
            "configuration": { "properties": { "demo.on": { "type": "boolean", "default": true, "description": "On" } } },
            "menus": { "editor/title": [{ "command": "demo.run" }], "view/title": [{ "command": "demo.run" }] }
        }});
        let found = features(&manifest);
        let titles: Vec<&str> = found.iter().map(|f| f.title).collect();
        assert_eq!(titles, vec!["Commands", "Settings", "Keyboard shortcuts", "Menu entries"]);
        assert_eq!(found[0].rows, vec![vec!["Demo: run.title".to_string(), "demo.run".to_string()]]);
        assert_eq!(found[1].rows[0], vec!["demo.on", "On", "true"]);
        assert_eq!(found[2].works, Works::No);
        assert_eq!(found[2].rows[0][0], "ctrl+alt+r");
        assert_eq!(found[3].rows, vec![vec!["editor/title".to_string(), "1".to_string()]]);
    }

    #[test]
    fn pages_escape_and_wait() {
        let v = view(json!({ "main": "x.js", "activationEvents": ["onLanguage:<x>"] }));
        let details = page_html(&v, Tab::Details);
        assert!(details.contains("<h1 id=\"demo\">Demo</h1>"));
        assert!(!details.contains("<script>alert"));
        assert!(details.contains("<base href=\"https://r/file/\">"));
        assert!(details.contains("Asking the registry"));
        let features = page_html(&v, Tab::Features);
        assert!(features.contains("<code>onLanguage:&lt;x&gt;</code>"));
        assert!(page_html(&v, Tab::Changelog).contains("It has no changelog."));
        assert_eq!(folder_of("https://r/api/a/b/1.0.0/file/readme.md"), "https://r/api/a/b/1.0.0/file/");
    }
}
