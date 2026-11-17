//! The user's choice of colours: a built-in theme and the colours changed on
//! top of it. Stored as `theme.json` in the data directory, next to the
//! session:
//!
//! ```json
//! { "theme": "dark", "colors": { "card_border": "#4b5362" }, "icons": "symbols" }
//! ```
//!
//! The themes themselves, and the key of every colour, are in
//! `canvas_render::theme`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use canvas_render::theme::{self, Theme};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ThemeFile {
    /// Key of a built-in theme (`theme::PRESETS`); unknown or empty means
    /// the default one.
    pub theme: String,
    /// Colours changed by the user: key → `#rrggbb` or `#rrggbbaa`.
    pub colors: BTreeMap<String, String>,
    /// The file icon theme: the id an extension gives it (`"symbols"`),
    /// `"none"` for the app's own glyphs, or empty to use one that is
    /// installed (see `Extensions::icon_theme`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icons: String,
}

impl ThemeFile {
    /// The built-in theme this choice starts from: `(key, colours)`.
    pub fn base(&self) -> (&'static str, Theme) {
        let (key, _, colors) = theme::PRESETS.iter().find(|p| p.0 == self.theme).unwrap_or(&theme::PRESETS[0]);
        (key, *colors)
    }

    /// The colours to use: the base theme with the user's changes. Keys
    /// that no longer exist and values that are not colours are ignored.
    pub fn resolve(&self) -> Theme {
        let (_, mut out) = self.base();
        for field in theme::FIELDS {
            if let Some(color) = self.colors.get(field.key).and_then(|text| theme::parse_hex(text)) {
                (field.set)(&mut out, color);
            }
        }
        out
    }

    /// Switch to another built-in theme, keeping the colours changed by hand.
    pub fn set_base(&mut self, key: &str) {
        self.theme = key.to_string();
    }

    /// Change one colour. Setting it back to the base theme's value forgets
    /// the change, so the file only lists what really differs.
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

        // Back to the base theme's own value: nothing left to remember.
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
