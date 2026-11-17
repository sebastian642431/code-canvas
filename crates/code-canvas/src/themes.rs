use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use canvas_render::theme::{self, Theme};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ThemeFile {
    pub theme: String,
    pub colors: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icons: String,
    #[serde(skip)]
    pub from_extension: Option<Theme>,
}

pub const FROM_EXTENSION: &str = "ext:";

impl ThemeFile {
    pub fn base(&self) -> (&str, Theme) {
        if let Some(colors) = self.from_extension.filter(|_| self.theme.starts_with(FROM_EXTENSION)) {
            return (self.theme.as_str(), colors);
        }
        let (key, _, colors) = theme::PRESETS.iter().find(|p| p.0 == self.theme).unwrap_or(&theme::PRESETS[0]);
        (key, *colors)
    }

    pub fn resolve(&self) -> Theme {
        let (_, mut out) = self.base();
        for field in theme::FIELDS {
            if let Some(color) = self.colors.get(field.key).and_then(|text| theme::parse_hex(text)) {
                (field.set)(&mut out, color);
            }
        }
        out
    }

    pub fn set_base(&mut self, key: &str) {
        self.theme = key.to_string();
    }

    pub fn set_color(&mut self, key: &str, color: u32) {
        let Some(field) = theme::FIELDS.iter().find(|f| f.key == key) else { return };
        if (field.get)(&self.base().1) == color {
            self.colors.remove(key);
        } else {
            self.colors.insert(key.to_string(), theme::to_hex(color));
        }
    }

    pub fn is_changed(&self, key: &str) -> bool {
        self.colors.contains_key(key)
    }
}

pub fn plain_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut star = false;
                for skipped in chars.by_ref() {
                    if star && skipped == '/' {
                        break;
                    }
                    star = skipped == '*';
                }
            }
            ('}' | ']', _) => {
                let kept = out.trim_end().len();
                if out[..kept].ends_with(',') {
                    out.truncate(kept - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[derive(Default)]
struct Written {
    colors: HashMap<String, u32>,
    tokens: Vec<(String, u32)>,
}

fn written(file: &Path, depth: u8, into: &mut Written) -> Option<()> {
    let text = std::fs::read_to_string(file).ok()?;
    let v: serde_json::Value = serde_json::from_str(&plain_json(&text)).ok()?;
    if let (Some(include), true) = (v.get("include").and_then(|i| i.as_str()), depth < 4) {
        written(&file.parent()?.join(include), depth + 1, into);
    }
    if let Some(colors) = v.get("colors").and_then(|c| c.as_object()) {
        for (key, value) in colors {
            if let Some(color) = value.as_str().and_then(color_of) {
                into.colors.insert(key.clone(), color);
            }
        }
    }
    for rule in v.get("tokenColors").and_then(|t| t.as_array()).map(Vec::as_slice).unwrap_or_default() {
        let Some(color) = rule.pointer("/settings/foreground").and_then(|c| c.as_str()).and_then(color_of) else {
            continue;
        };
        let scopes: Vec<&str> = match rule.get("scope") {
            Some(serde_json::Value::String(all)) => all.split(',').collect(),
            Some(serde_json::Value::Array(list)) => list.iter().filter_map(|s| s.as_str()).collect(),
            _ => vec![""],
        };
        for scope in scopes.iter().filter(|scope| !scope.contains(" - ")) {
            let innermost = scope.split_whitespace().last().unwrap_or("");
            into.tokens.push((innermost.to_string(), color));
        }
    }
    Some(())
}

fn color_of(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    match hex.len() {
        4 => theme::parse_hex(&hex.chars().flat_map(|c| [c, c]).collect::<String>()),
        _ => theme::parse_hex(hex),
    }
}

fn over(color: u32, under: u32) -> u32 {
    theme::mix(under | 0xff, color | 0xff, (color & 0xff) as f32 / 255.0)
}

fn with_alpha(color: u32, alpha: u32) -> u32 {
    (color & 0xffff_ff00) | alpha
}

fn token(tokens: &[(String, u32)], wanted: &[&str]) -> Option<u32> {
    wanted.iter().find_map(|scope| {
        let fits = |rule: &&(String, u32)| {
            let rule = rule.0.as_str();
            !rule.is_empty() && (*scope == rule || scope.strip_prefix(rule).is_some_and(|rest| rest.starts_with('.')))
        };
        let mut best: Option<&(String, u32)> = None;
        for rule in tokens.iter().filter(fits) {
            if best.is_none_or(|b| rule.0.len() >= b.0.len()) {
                best = Some(rule);
            }
        }
        best.map(|rule| rule.1 | 0xff)
    })
}

fn theme_of(written: &Written, light: bool) -> Option<Theme> {
    let asked = |keys: &[&str]| keys.iter().find_map(|key| written.colors.get(*key).copied()).filter(|c| c & 0xff > 0);
    let bg = asked(&["editor.background"])? | 0xff;
    let plain = token(&written.tokens, &[""]);
    let fg = asked(&["editor.foreground", "foreground"]).map(|c| over(c, bg)).or(plain).unwrap_or(if light {
        0x24292eff
    } else {
        0xd4d4d4ff
    });
    let side = asked(&["sideBar.background"]).map_or(bg, |c| over(c, bg));
    let bar = asked(&["activityBar.background", "titleBar.activeBackground"]).map_or(side, |c| over(c, side));
    let shade = |color: u32, by: f32| theme::mix(color, fg, by);
    let on_side = |keys: &[&str], or: f32| asked(keys).map_or(shade(side, or), |c| over(c, side));
    let accent = asked(&["focusBorder", "button.background", "activityBarBadge.background", "textLink.foreground"])
        .map_or(DARK_ACCENT, |c| c | 0xff);
    let text = asked(&["sideBar.foreground", "foreground"]).map_or(fg, |c| over(c, side));
    let dim = asked(&["descriptionForeground"]).map_or(theme::mix(text, side, 0.35), |c| over(c, side));
    let faint = asked(&["disabledForeground", "editorLineNumber.foreground"])
        .map_or(theme::mix(text, side, 0.55), |c| over(c, side));
    let border = asked(&["contrastBorder"]).map_or(shade(side, 0.12), |c| over(c, side));
    let canvas = asked(&["editorPane.background", "editorGroup.emptyBackground"])
        .map(|c| over(c, bg))
        .filter(|c| *c != bg)
        .unwrap_or(theme::mix(bg, 0x000000ff, if light { 0.07 } else { 0.28 }));
    let ansi = |key: &str, or: u32| theme::mix(asked(&[key]).map_or(or, |c| c | 0xff), canvas, 0.45);
    let tokens = &written.tokens;
    let keyword = token(tokens, &["keyword.control", "keyword", "storage.type", "storage"]).unwrap_or(fg);
    let function = token(tokens, &["entity.name.function", "support.function", "meta.function-call"]).unwrap_or(fg);
    let comment = token(tokens, &["comment"]).unwrap_or(theme::mix(fg, bg, 0.5));
    let base = theme::DARK;
    let status = |keys: &[&str], or: u32| asked(keys).map_or(or, |c| c | 0xff);
    Some(Theme {
        chrome: bar,
        card: side,
        well: asked(&["input.background"]).map_or(shade(side, 0.04), |c| over(c, side)),
        row: shade(side, 0.04),
        raised: shade(side, 0.09),
        hover: on_side(&["list.hoverBackground"], 0.08),
        active: on_side(&["list.activeSelectionBackground", "list.inactiveSelectionBackground"], 0.15),
        selection: asked(&["editor.selectionBackground"]).map_or(theme::mix(bg, accent, 0.35), |c| over(c, bg)),
        border,
        border_strong: shade(border, 0.25),
        text,
        text_dim: dim,
        text_faint: faint,
        accent,
        on_accent: asked(&["button.foreground"]).map_or(if light { 0xffffffff } else { 0x101216ff }, |c| c | 0xff),
        background: canvas,
        card_fill: with_alpha(bg, 0xf0),
        card_border: theme::mix(bg, fg, 0.18),
        card_note_fill: with_alpha(theme::mix(bg, 0xe05561ff, 0.08), 0xf0),
        dir_fill: with_alpha(fg, 0x05),
        dir_collapsed_fill: with_alpha(fg, 0x0e),
        dir_border_0: ansi("terminal.ansiBlue", base.dir_border_0),
        dir_border_1: ansi("terminal.ansiGreen", base.dir_border_1),
        dir_border_2: ansi("terminal.ansiMagenta", base.dir_border_2),
        dir_border_3: ansi("terminal.ansiYellow", base.dir_border_3),
        label_dir: theme::mix(fg, canvas, 0.35),
        label_file: theme::mix(fg, canvas, 0.12),
        canvas_hover: theme::mix(fg, canvas, 0.4),
        select: accent,
        select_fill: with_alpha(accent, 0x1c),
        chip_fill: with_alpha(theme::mix(canvas, 0x000000ff, if light { 0.0 } else { 0.15 }), 0xf2),
        chip_text: fg,
        chip_text_dim: theme::mix(fg, canvas, 0.3),
        ui_text: fg,
        ui_text_dim: asked(&["editorLineNumber.foreground"]).map_or(theme::mix(fg, bg, 0.5), |c| over(c, bg)),
        ui_warn: status(&["editorWarning.foreground", "list.warningForeground"], base.ui_warn),
        tok_keyword: keyword,
        tok_type: token(
            tokens,
            &["entity.name.type", "support.type", "entity.name.class", "support.class", "storage.type"],
        )
        .unwrap_or(fg),
        tok_function: function,
        tok_macro: token(
            tokens,
            &["entity.name.function.macro", "support.function.macro", "meta.preprocessor", "support.macro"],
        )
        .unwrap_or(function),
        tok_string: token(tokens, &["string.quoted", "string"]).unwrap_or(fg),
        tok_number: token(tokens, &["constant.numeric", "constant"]).unwrap_or(fg),
        tok_comment: comment,
        tok_punct: token(tokens, &["punctuation"]).map_or(with_alpha(fg, 0xc8), |c| theme::mix(c, fg, 0.3)),
        tok_ident: token(tokens, &["variable.other", "variable"]).unwrap_or(fg),
        tok_heading: token(tokens, &["markup.heading", "entity.name.section"]).unwrap_or(keyword),
        tok_text: fg,
        problem_error: status(
            &["editorError.foreground", "errorForeground", "list.errorForeground"],
            base.problem_error,
        ),
        problem_warning: status(&["editorWarning.foreground", "list.warningForeground"], base.problem_warning),
        problem_info: status(&["editorInfo.foreground"], base.problem_info),
        git_added: status(
            &["gitDecoration.addedResourceForeground", "gitDecoration.untrackedResourceForeground"],
            base.git_added,
        ),
        git_modified: status(&["gitDecoration.modifiedResourceForeground"], base.git_modified),
        git_removed: status(&["gitDecoration.deletedResourceForeground"], base.git_removed),
        ..base
    })
}

const DARK_ACCENT: u32 = theme::DARK.accent;

pub fn from_vscode(file: &Path, light: bool) -> Option<Theme> {
    let mut all = Written::default();
    written(file, 0, &mut all)?;
    theme_of(&all, light)
}

fn path() -> PathBuf {
    crate::session::data_dir().join("theme.json")
}

pub fn load() -> ThemeFile {
    match std::fs::read_to_string(path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            log::warn!("ignoring unreadable theme file: {e}");
            ThemeFile::default()
        }),
        Err(_) => ThemeFile::default(),
    }
}

pub fn save(file: &ThemeFile) {
    let path = path();
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_string_pretty(file)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(&tmp, json))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        log::warn!("could not save the theme to {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_file_with_comments_and_commas_left_over_is_read() {
        let text =
            "{ // a comment\n \"a\": \"x // not one\", /* gone */ \"b\": [1, 2, ], \"c\": { \"d\": \"\\\"}\", },\n}";
        let v: serde_json::Value = serde_json::from_str(&plain_json(text)).unwrap();
        assert_eq!(v["a"], "x // not one");
        assert_eq!(v["b"], serde_json::json!([1, 2]));
        assert_eq!(v["c"]["d"], "\"}");
    }

    #[test]
    fn a_vscode_theme_becomes_the_colours_of_the_canvas() {
        let dir = std::env::temp_dir().join(format!("code-canvas-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("base.json"),
            r##"{ "colors": { "editor.background": "#102030", "sideBar.background": "#0a141e" },
                 "tokenColors": [{ "scope": "comment", "settings": { "foreground": "#556677" } }] }"##,
        )
        .unwrap();
        std::fs::write(
            dir.join("night.json"),
            r##"{ "include": "./base.json", // the rest
                 "colors": { "editor.foreground": "#ddeeff", "focusBorder": "#ff00aa", "list.hoverBackground": "#ffffff20",
                             "gitDecoration.modifiedResourceForeground": "#abcdef", "editor.selectionBackground": "#f0f8" },
                 "tokenColors": [
                    { "settings": { "foreground": "#ddeeff" } },
                    { "scope": "keyword, storage.type", "settings": { "foreground": "#ff8800" } },
                    { "scope": ["source.rust entity.name.function", "string"], "settings": { "foreground": "#00ff88" } },
                    { "scope": "keyword.control", "settings": { "foreground": "#aa00ff" } },
                    { "scope": "variable - variable.parameter", "settings": { "foreground": "#010101" } },
                 ] }"##,
        )
        .unwrap();
        let made = from_vscode(&dir.join("night.json"), false).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(made.card_fill, 0x102030f0);
        assert_eq!(made.card, 0x0a141eff, "panels take the side bar's colour, from the file included");
        assert_eq!(made.ui_text, 0xddeeffff);
        assert_eq!((made.accent, made.select), (0xff00aaff, 0xff00aaff));
        assert_eq!(made.tok_keyword, 0xaa00ffff, "the rule that says more about the scope wins");
        assert_eq!(made.tok_type, 0xff8800ff, "storage.type, for want of a rule about types");
        assert_eq!((made.tok_function, made.tok_string), (0x00ff88ff, 0x00ff88ff));
        assert_eq!(made.tok_comment, 0x556677ff);
        assert_eq!(made.tok_ident, 0xddeeffff, "a rule with an exclusion is not taken");
        assert_eq!(made.git_modified, 0xabcdefff);
        assert_ne!(made.hover, made.card);
        assert_eq!(made.hover & 0xff, 0xff, "what is see-through is laid over what it is on");
        assert_ne!(made.background, made.card_fill | 0xff, "the canvas is not the colour of the cards");
        assert_eq!(made.rel_def, theme::DARK.rel_def);
        assert!(from_vscode(&dir.join("gone.json"), false).is_none());
    }

    #[test]
    fn a_theme_of_an_extension_is_the_base_while_it_is_there() {
        let mut file = ThemeFile { theme: "ext:some.theme:Night".into(), ..Default::default() };
        assert_eq!(file.resolve(), Theme::default(), "not read yet, or gone: the default");
        file.from_extension = Some(theme::MIDNIGHT);
        assert_eq!(file.base(), ("ext:some.theme:Night", theme::MIDNIGHT));
        file.set_color("card", 0x112233ff);
        assert_eq!(file.resolve().card, 0x112233ff);
        file.set_base("dark");
        assert_eq!(file.base().0, "dark");
    }

    #[test]
    fn nothing_chosen_is_the_default_theme() {
        assert_eq!(ThemeFile::default().resolve(), Theme::default());
        let unknown = ThemeFile { theme: "no-such-theme".into(), ..Default::default() };
        assert_eq!(unknown.resolve(), Theme::default());
    }

    #[test]
    fn a_changed_colour_goes_on_top_of_the_base() {
        let mut file = ThemeFile::default();
        file.set_base("midnight");
        file.set_color("card", 0x112233ff);
        let colors = file.resolve();
        assert_eq!(colors.card, 0x112233ff);
        assert_eq!(colors.chrome, theme::MIDNIGHT.chrome);
        assert!(file.is_changed("card"));

        file.set_color("card", theme::MIDNIGHT.card);
        assert!(file.colors.is_empty());
        assert_eq!(file.resolve(), theme::MIDNIGHT);
    }

    #[test]
    fn bad_entries_are_ignored() {
        let text = r##"{ "theme": "dark", "colors": { "card": "not a colour", "gone": "#ffffff", "text": "#fff" } }"##;
        let file: ThemeFile = serde_json::from_str(text).unwrap();
        let colors = file.resolve();
        assert_eq!(colors.card, theme::DARK.card);
        assert_eq!(colors.text, 0xffffffff);
    }

    #[test]
    fn survives_a_round_trip() {
        let mut file = ThemeFile::default();
        file.set_color("select_fill", 0x61afef40);
        let again: ThemeFile = serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap();
        assert_eq!(again, file);
        assert_eq!(again.resolve().select_fill, 0x61afef40);
    }
}
