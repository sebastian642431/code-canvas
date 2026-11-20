use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use canvas_core::TokenKind;

mod presets;
pub use presets::*;

pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub get: fn(&Theme) -> u32,
    pub set: fn(&mut Theme, u32),
}

macro_rules! theme {
    ($( $group:literal { $( $field:ident : $label:literal; )* } )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Theme { $( $( pub $field: u32, )* )* }

        pub const FIELDS: &[Field] = &[ $( $( Field {
            key: stringify!($field),
            label: $label,
            group: $group,
            get: |t| t.$field,
            set: |t, v| t.$field = v,
        }, )* )* ];
    };
}

theme! {
    "Interface" {
        chrome: "Window and side rails";
        card: "Panels";
        well: "Text fields and insets";
        row: "List rows";
        raised: "Buttons and selected rows";
        hover: "Under the pointer";
        active: "While pressed";
        selection: "Selected text";
        border: "Borders";
        border_strong: "Borders under the pointer";
        text: "Text";
        text_dim: "Secondary text";
        text_faint: "Hints and disabled text";
        accent: "Accent";
        on_accent: "Text on the accent";
    }
    "Canvas" {
        background: "Background";
        card_fill: "File cards";
        card_border: "File card borders";
        card_note_fill: "Cards of skipped files";
        dir_fill: "Folders";
        dir_collapsed_fill: "Collapsed folders";
        dir_border_0: "Folder borders, depth 1";
        dir_border_1: "Folder borders, depth 2";
        dir_border_2: "Folder borders, depth 3";
        dir_border_3: "Folder borders, depth 4";
        label_dir: "Folder names";
        label_file: "File names";
        canvas_hover: "Outline under the pointer";
        select: "Selection outline";
        select_fill: "Selection fill";
        chip_fill: "Labels on lines";
        chip_text: "Label text";
        chip_text_dim: "Label secondary text";
        ui_text: "Text drawn on the canvas";
        ui_text_dim: "Secondary text on the canvas";
        ui_warn: "Warnings on the canvas";
    }
    "Code" {
        tok_keyword: "Keywords";
        tok_type: "Types";
        tok_function: "Functions";
        tok_macro: "Macros";
        tok_string: "Strings";
        tok_number: "Numbers";
        tok_comment: "Comments";
        tok_punct: "Punctuation";
        tok_ident: "Names";
        tok_heading: "Headings";
        tok_text: "Prose";
        caret: "Caret";
        code_selection: "Selected code";
        current_line: "Line with the caret";
        find_match: "Search matches";
        line_mark: "Lines a link points to";
    }
    "Relations" {
        rel_def: "Definitions";
        rel_impl: "Implementations";
        rel_ref: "References";
        rel_in: "Incoming calls";
        rel_out: "Outgoing calls";
        dep_out: "What the selection imports";
        dep_in: "What imports the selection";
    }
    "Status" {
        problem_error: "Errors";
        problem_warning: "Warnings";
        problem_info: "Information";
        git_added: "Git: new";
        git_modified: "Git: changed";
        git_removed: "Git: removed";
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dark,
    Light,
    HighContrast,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Dark, Kind::Light, Kind::HighContrast];

    pub fn title(self) -> &'static str {
        match self {
            Kind::Dark => "Dark",
            Kind::Light => "Light",
            Kind::HighContrast => "High contrast",
        }
    }
}

pub struct Preset {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub about: &'static str,
    pub theme: Theme,
}

const fn preset(key: &'static str, name: &'static str, kind: Kind, about: &'static str, theme: Theme) -> Preset {
    Preset { key, name, kind, about, theme }
}

pub const PRESETS: &[Preset] = &[
    preset("dark", "Dark", Kind::Dark, "Neutral graphite, the look Hawk Canvas started with.", DARK),
    preset("midnight", "Midnight", Kind::Dark, "Deep night blue with soft pastel code.", MIDNIGHT),
    preset("ember", "Ember", Kind::Dark, "Warm charcoal and amber, easy on the eyes at night.", EMBER),
    preset("harbor", "Harbor", Kind::Dark, "Cool slate and sea teal.", HARBOR),
    preset("moss", "Moss", Kind::Dark, "Quiet forest greens with earthy code.", MOSS),
    preset("linen", "Linen", Kind::Light, "Warm paper white with ink-dark text.", LINEN),
    preset("daylight", "Daylight", Kind::Light, "Crisp cool white for bright rooms.", DAYLIGHT),
    preset(
        "contrast",
        "High Contrast",
        Kind::HighContrast,
        "Black, white text, strong borders and vivid code.",
        CONTRAST,
    ),
    preset(
        "contrast-light",
        "High Contrast Light",
        Kind::HighContrast,
        "White, black text, strong borders and deep code colours.",
        CONTRAST_LIGHT,
    ),
];

pub fn preset_named(key: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.key == key)
}

impl Default for Theme {
    fn default() -> Self {
        DARK
    }
}

impl Theme {
    pub fn token_colors(&self) -> [u32; 16] {
        let mut out = [0xffffffff; 16];
        out[..11].copy_from_slice(&[
            self.tok_keyword,
            self.tok_type,
            self.tok_function,
            self.tok_macro,
            self.tok_string,
            self.tok_number,
            self.tok_comment,
            self.tok_punct,
            self.tok_ident,
            self.tok_heading,
            self.tok_text,
        ]);
        out
    }

    pub fn dir_border(&self, depth: usize) -> u32 {
        [self.dir_border_0, self.dir_border_1, self.dir_border_2, self.dir_border_3][depth % 4]
    }

    pub fn rel_colors(&self) -> [u32; 5] {
        [self.rel_def, self.rel_impl, self.rel_ref, self.rel_in, self.rel_out]
    }

    pub fn is_light(&self) -> bool {
        luminance(self.card) > 0.4
    }
}

pub fn with_alpha(color: u32, alpha: u32) -> u32 {
    (color & 0xffff_ff00) | (alpha & 0xff)
}

pub fn over(color: u32, under: u32) -> u32 {
    mix(under | 0xff, color | 0xff, (color & 0xff) as f32 / 255.0)
}

pub fn luminance(c: u32) -> f32 {
    let [r, g, b, _] = unpack(c);
    let ch = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
}

pub fn contrast(a: u32, b: u32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

static CURRENT: RwLock<Theme> = RwLock::new(DARK);
static REVISION: AtomicU64 = AtomicU64::new(0);

pub fn get() -> Theme {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

pub fn set(theme: Theme) {
    let mut current = CURRENT.write().unwrap_or_else(|e| e.into_inner());
    if *current != theme {
        *current = theme;
        REVISION.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

pub fn kind_index(kind: TokenKind) -> u32 {
    match kind {
        TokenKind::Keyword => 0,
        TokenKind::Type => 1,
        TokenKind::Function => 2,
        TokenKind::Macro => 3,
        TokenKind::String => 4,
        TokenKind::Number => 5,
        TokenKind::Comment => 6,
        TokenKind::Punct => 7,
        TokenKind::Ident => 8,
        TokenKind::Heading => 9,
        TokenKind::Text => 10,
    }
}

pub fn unpack(c: u32) -> [u8; 4] {
    [(c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8]
}

pub fn to_f32(c: u32) -> [f32; 4] {
    let [r, g, b, a] = unpack(c);
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
}

pub fn glyph_color(c: u32) -> glyphon::Color {
    let [r, g, b, a] = unpack(c);
    glyphon::Color::rgba(r, g, b, a)
}

pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let (a, b) = (unpack(a), unpack(b));
    let ch = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t.clamp(0.0, 1.0)).round() as u32;
    (ch(0) << 24) | (ch(1) << 16) | (ch(2) << 8) | ch(3)
}

pub fn to_hex(c: u32) -> String {
    if c & 0xff == 0xff {
        format!("#{:06x}", c >> 8)
    } else {
        format!("#{c:08x}")
    }
}

pub fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        3 => {
            let (r, g, b) = ((value >> 8) & 0xf, (value >> 4) & 0xf, value & 0xf);
            Some((r * 17) << 24 | (g * 17) << 16 | (b * 17) << 8 | 0xff)
        }
        6 => Some(value << 8 | 0xff),
        8 => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        for c in [0x23272eff, 0x61afef1c, 0x00000000, 0xffffffff] {
            assert_eq!(parse_hex(&to_hex(c)), Some(c));
        }
        assert_eq!(to_hex(0x23272eff), "#23272e");
        assert_eq!(to_hex(0x61afef1c), "#61afef1c");
        assert_eq!(parse_hex("#fff"), Some(0xffffffff));
        assert_eq!(parse_hex("1e2227"), Some(0x1e2227ff));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
        assert_eq!(parse_hex("+23272e"), None);
    }

    #[test]
    fn fields_cover_the_theme_once() {
        let mut keys: Vec<&str> = FIELDS.iter().map(|f| f.key).collect();
        keys.sort_unstable();
        let total = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), total);
        let mut copy = MIDNIGHT;
        for f in FIELDS {
            (f.set)(&mut copy, (f.get)(&DARK));
        }
        assert_eq!(copy, DARK);
    }

    #[test]
    fn the_default_is_the_first_preset() {
        assert_eq!(PRESETS[0].theme, Theme::default());
        let mut keys: Vec<&str> = PRESETS.iter().map(|p| p.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PRESETS.len(), "every preset has a key of its own");
        for (i, a) in PRESETS.iter().enumerate() {
            for b in &PRESETS[i + 1..] {
                assert_ne!(a.theme, b.theme, "{} and {} are the same colours", a.name, b.name);
            }
        }
        assert_eq!(preset_named("linen").map(|p| p.name), Some("Linen"));
    }

    #[test]
    fn every_kind_has_themes_and_light_ones_are_light() {
        for kind in Kind::ALL {
            assert!(PRESETS.iter().filter(|p| p.kind == kind).count() >= 2, "{kind:?}");
        }
        for p in PRESETS {
            let light = matches!(p.key, "linen" | "daylight" | "contrast-light");
            assert_eq!(p.theme.is_light(), light, "{}", p.name);
        }
    }

    fn readable_pairs(t: &Theme, high: bool) -> Vec<(&'static str, &'static str, u32, u32, f32)> {
        let (main, second) = if high { (7.0, 4.5) } else { (4.5, 3.0) };
        let canvas = t.background;
        let card = over(t.card_fill, canvas);
        let chip = over(t.chip_fill, canvas);
        let on_panel = |c: u32| over(c, t.card);
        let mut pairs = vec![
            ("text", "card", t.text, t.card, main),
            ("text", "chrome", t.text, on_panel(t.chrome), main),
            ("text", "well", t.text, on_panel(t.well), main),
            ("text", "row", t.text, on_panel(t.row), main),
            ("text", "raised", t.text, on_panel(t.raised), main),
            ("text", "hover", t.text, on_panel(t.hover), main),
            ("text", "active", t.text, on_panel(t.active), main),
            ("text", "selection", t.text, on_panel(t.selection), main),
            ("text_dim", "card", t.text_dim, t.card, second),
            ("text_dim", "chrome", t.text_dim, on_panel(t.chrome), second),
            ("text_dim", "well", t.text_dim, on_panel(t.well), second),
            ("text_dim", "row", t.text_dim, on_panel(t.row), second),
            ("text_faint", "card", t.text_faint, t.card, if high { 3.0 } else { 2.0 }),
            ("accent", "card", t.accent, t.card, second),
            ("on_accent", "accent", t.on_accent, t.accent, main),
            ("ui_text", "background", t.ui_text, canvas, main),
            ("ui_text", "card_fill", t.ui_text, card, main),
            ("ui_text_dim", "background", t.ui_text_dim, canvas, second),
            ("label_file", "background", t.label_file, canvas, main),
            ("label_dir", "background", t.label_dir, canvas, second),
            ("chip_text", "chip_fill", t.chip_text, chip, main),
            ("chip_text_dim", "chip_fill", t.chip_text_dim, chip, second),
            ("ui_warn", "background", t.ui_warn, canvas, second),
            ("select", "background", t.select, canvas, second),
            ("caret", "card_fill", t.caret, card, second),
            ("tok_comment", "card_fill", over(t.tok_comment, card), card, second),
            ("tok_punct", "card_fill", over(t.tok_punct, card), card, second),
        ];
        let code = [
            ("tok_keyword", t.tok_keyword),
            ("tok_type", t.tok_type),
            ("tok_function", t.tok_function),
            ("tok_macro", t.tok_macro),
            ("tok_string", t.tok_string),
            ("tok_number", t.tok_number),
            ("tok_ident", t.tok_ident),
            ("tok_heading", t.tok_heading),
            ("tok_text", t.tok_text),
        ];
        for (name, color) in code {
            pairs.push((name, "card_fill", over(color, card), card, main));
            pairs.push((name, "code_selection", over(color, card), over(t.code_selection, card), second));
            pairs.push((name, "line_mark", over(color, card), over(t.line_mark, card), second));
        }
        let status = [
            ("problem_error", t.problem_error),
            ("problem_warning", t.problem_warning),
            ("problem_info", t.problem_info),
            ("git_added", t.git_added),
            ("git_modified", t.git_modified),
            ("git_removed", t.git_removed),
        ];
        for (name, color) in status {
            pairs.push((name, "card", color, t.card, second));
        }
        pairs
    }

    #[test]
    fn every_preset_is_readable() {
        let mut bad = Vec::new();
        for p in PRESETS {
            for (fore, back, a, b, wanted) in readable_pairs(&p.theme, p.kind == Kind::HighContrast) {
                let got = contrast(a, b);
                if got < wanted {
                    bad.push(format!("{}: {fore} on {back} is {got:.2}:1, wants {wanted}:1", p.name));
                }
            }
        }
        assert!(bad.is_empty(), "\n{}", bad.join("\n"));
    }

    #[test]
    fn syntax_colours_are_told_apart() {
        for p in PRESETS {
            let t = p.theme;
            let main = [t.tok_keyword, t.tok_type, t.tok_function, t.tok_string, t.tok_number, t.tok_comment];
            for (i, a) in main.iter().enumerate() {
                for b in &main[i + 1..] {
                    let (a, b) = (unpack(*a), unpack(*b));
                    let apart: f32 = (0..3).map(|c| (a[c] as f32 - b[c] as f32).powi(2)).sum::<f32>().sqrt();
                    assert!(apart >= 45.0, "{}: two kinds of code look alike ({apart:.0})", p.name);
                }
            }
        }
    }

    #[test]
    fn contrast_of_black_and_white() {
        assert!((contrast(0x000000ff, 0xffffffff) - 21.0).abs() < 0.01);
        assert!((contrast(0x777777ff, 0x777777ff) - 1.0).abs() < 0.001);
        assert_eq!(over(0xffffff80, 0x000000ff), 0x808080ff);
        assert_eq!(with_alpha(0x11223344, 0x55), 0x11223355);
    }

    #[test]
    fn token_colours_follow_kind_index() {
        let t = DARK;
        let colors = t.token_colors();
        assert_eq!(colors[kind_index(TokenKind::Keyword) as usize], t.tok_keyword);
        assert_eq!(colors[kind_index(TokenKind::Comment) as usize], t.tok_comment);
        assert_eq!(colors[kind_index(TokenKind::Text) as usize], t.tok_text);
    }

    #[test]
    fn mix_ends() {
        assert_eq!(mix(0x000000ff, 0xffffffff, 0.0), 0x000000ff);
        assert_eq!(mix(0x000000ff, 0xffffffff, 1.0), 0xffffffff);
        assert_eq!(mix(0x000000ff, 0xffffffff, 0.5), 0x808080ff);
    }
}
