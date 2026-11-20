//! What of an extension Hawk Canvas cannot do yet, read from its manifest (package.json)
//! alone: the same answer before it is installed (from the registry's copy of the manifest)
//! and after (from the one in its folder). What it asks for while running and is missing
//! comes on top, from the host (`Installed::unsupported`).

use serde_json::Value;

/// One thing an extension brings that does not work in Hawk Canvas yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Gap {
    /// What it is, in a few words: "Custom editors".
    pub what: String,
    /// Which of them, or what it means: "Bruno Request Editor".
    pub detail: String,
}

/// The menus of VS Code whose entries Hawk Canvas shows: the buttons of a view's title and
/// of its rows. The command palette's entries are what the extension's page lists.
const SHOWN_MENUS: [&str; 3] = ["view/title", "view/item/context", "commandPalette"];

/// Contribution points that do nothing in Hawk Canvas yet, and what to call them.
const NOT_YET: [(&str, &str); 17] = [
    ("customEditors", "Custom editors"),
    ("keybindings", "Keyboard shortcuts"),
    ("snippets", "Snippets"),
    ("debuggers", "Debugging"),
    ("breakpoints", "Debugging"),
    ("taskDefinitions", "Tasks"),
    ("problemMatchers", "Tasks"),
    ("notebooks", "Notebooks"),
    ("notebookRenderer", "Notebooks"),
    ("terminal", "Terminal profiles"),
    ("authentication", "Signing in to its service"),
    ("chatParticipants", "VS Code's chat"),
    ("languageModelTools", "VS Code's chat"),
    ("languageModelChatProviders", "VS Code's chat"),
    ("mcpServerDefinitionProviders", "VS Code's chat"),
    ("productIconThemes", "Product icon themes"),
    ("typescriptServerPlugins", "TypeScript server plugins"),
];

fn plain(raw: &str) -> String {
    raw.strip_prefix('%').and_then(|r| r.strip_suffix('%')).unwrap_or(raw).to_string()
}

fn names(list: &Value, keys: &[&str]) -> Vec<String> {
    let items = list.as_array().map(Vec::as_slice).unwrap_or_default();
    let mut out: Vec<String> = items
        .iter()
        .filter_map(|item| keys.iter().find_map(|k| item.get(*k)).and_then(Value::as_str).map(plain))
        .filter(|n| !n.is_empty())
        .collect();
    out.dedup();
    out
}

fn count(list: &Value) -> usize {
    match list {
        Value::Array(items) => items.len(),
        Value::Object(map) => map.len(),
        _ => 0,
    }
}

/// Everything of the manifest that does not work in Hawk Canvas yet; empty when all of it does.
pub fn gaps(manifest: &Value) -> Vec<Gap> {
    let mut out: Vec<Gap> = Vec::new();
    let mut add = |what: &str, detail: String| match out.iter_mut().find(|g| g.what == what) {
        Some(g) if !detail.is_empty() && !g.detail.contains(&detail) => {
            g.detail = if g.detail.is_empty() { detail } else { format!("{}, {detail}", g.detail) };
        }
        Some(_) => {}
        None => out.push(Gap { what: what.to_string(), detail }),
    };
    let has_main = manifest.get("main").and_then(Value::as_str).is_some_and(|m| !m.trim().is_empty());
    let has_browser = manifest.get("browser").and_then(Value::as_str).is_some_and(|m| !m.trim().is_empty());
    if has_browser && !has_main {
        add("Its code", "it is written only for VS Code in a browser".into());
    }
    let contributes = manifest.get("contributes").cloned().unwrap_or(Value::Null);
    for (key, what) in NOT_YET {
        let Some(list) = contributes.get(key) else { continue };
        let n = count(list);
        if n == 0 {
            continue;
        }
        let detail = match key {
            "customEditors" | "notebooks" => names(list, &["displayName", "viewType", "type"]).join(", "),
            "debuggers" => names(list, &["label", "type"]).join(", "),
            "keybindings" | "snippets" => format!("{n}"),
            "taskDefinitions" => names(list, &["type"]).join(", "),
            "authentication" => names(list, &["label", "id"]).join(", "),
            "chatParticipants" | "languageModelTools" => {
                names(list, &["fullName", "displayName", "name", "id"]).join(", ")
            }
            _ => String::new(),
        };
        add(what, detail);
    }
    if let Some(Value::Object(menus)) = contributes.get("menus") {
        let places: Vec<&str> = menus
            .iter()
            .filter(|(place, items)| !SHOWN_MENUS.contains(&place.as_str()) && count(items) > 0)
            .map(|(place, _)| place.as_str())
            .collect();
        if !places.is_empty() {
            add("Menu entries", format!("in {} (its commands still run from its page)", places.join(", ")));
        }
    }
    let wanted = |key: &str| -> Vec<String> {
        let list = manifest.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        list.iter().filter_map(Value::as_str).map(str::to_string).collect()
    };
    let pack = wanted("extensionPack");
    if !pack.is_empty() {
        add("Extension packs", format!("the {} extensions it gathers are not installed with it", pack.len()));
    }
    let needs = wanted("extensionDependencies");
    if !needs.is_empty() {
        add("Extensions it needs", format!("not installed with it: {}", needs.join(", ")));
    }
    // A count shows up as "Keyboard shortcuts (3)" rather than "Keyboard shortcuts: 3".
    for g in &mut out {
        if !g.detail.is_empty() && g.detail.chars().all(|c| c.is_ascii_digit()) {
            g.what = format!("{} ({})", g.what, g.detail);
            g.detail.clear();
        }
    }
    out
}

/// The gaps in one line each: "Custom editors: Bruno Request Editor".
pub fn lines(gaps: &[Gap]) -> Vec<String> {
    gaps.iter()
        .map(|g| if g.detail.is_empty() { g.what.clone() } else { format!("{}: {}", g.what, g.detail) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_manifest_with_what_works_has_no_gaps() {
        let manifest = json!({
            "main": "./out/extension.js",
            "activationEvents": ["onLanguage:rust"],
            "contributes": {
                "commands": [{ "command": "x.run", "title": "Run" }],
                "languages": [{ "id": "x", "extensions": [".x"] }],
                "grammars": [{ "language": "x", "scopeName": "source.x", "path": "x.json" }],
                "configuration": { "properties": { "x.on": { "type": "boolean" } } },
                "views": { "explorer": [{ "id": "x.tree", "name": "X" }] },
                "menus": { "view/title": [{ "command": "x.run" }], "commandPalette": [{ "command": "x.run" }] },
                "themes": [{ "label": "X", "path": "t.json" }]
            }
        });
        assert!(gaps(&manifest).is_empty());
    }

    #[test]
    fn what_does_not_work_is_named() {
        let manifest = json!({
            "main": "./dist/extension.js",
            "extensionDependencies": ["vscode.git"],
            "contributes": {
                "customEditors": [{ "viewType": "bruno.requestEditor", "displayName": "Bruno Request Editor" }],
                "keybindings": [{ "command": "a", "key": "ctrl+s" }, { "command": "b", "key": "ctrl+r" }],
                "debuggers": [{ "type": "node", "label": "Node.js" }],
                "breakpoints": [{ "language": "javascript" }],
                "menus": { "editor/title": [{ "command": "a" }], "view/title": [{ "command": "b" }], "explorer/context": [] }
            }
        });
        let found = lines(&gaps(&manifest));
        assert_eq!(
            found,
            vec![
                "Custom editors: Bruno Request Editor",
                "Keyboard shortcuts (2)",
                "Debugging: Node.js",
                "Menu entries: in editor/title (its commands still run from its page)",
                "Extensions it needs: not installed with it: vscode.git",
            ]
        );
    }

    #[test]
    fn code_only_for_the_browser() {
        let manifest = json!({ "browser": "./dist/web.js" });
        assert_eq!(gaps(&manifest)[0].what, "Its code");
        assert!(gaps(&json!({ "browser": "./web.js", "main": "./node.js" })).is_empty());
    }
}
