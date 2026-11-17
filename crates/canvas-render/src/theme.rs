use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use canvas_core::TokenKind;

pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub get: fn(&Theme) -> u32,
    pub set: fn(&mut Theme, u32),
}

macro_rules! theme {
    ($( $group:literal { $( $field:ident : $label:literal = $dark:literal, $midnight:literal; )* } )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Theme { $( $( pub $field: u32, )* )* }

        pub const DARK: Theme = Theme { $( $( $field: $dark, )* )* };
        pub const MIDNIGHT: Theme = Theme { $( $( $field: $midnight, )* )* };

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
        chrome: "Window and side rails" = 0x1e2227ff, 0x0a0b0eff;
        card: "Panels" = 0x23272eff, 0x101115ff;
        well: "Text fields and insets" = 0x1d1f23ff, 0x0b0c0fff;
        row: "List rows" = 0x282c34ff, 0x14161bff;
        raised: "Buttons and selected rows" = 0x2c313aff, 0x1b1d24ff;
        hover: "Under the pointer" = 0x323842ff, 0x22252eff;
        active: "While pressed" = 0x3e4452ff, 0x2c313eff;
        selection: "Selected text" = 0x3d4657ff, 0x344058ff;
        border: "Borders" = 0x353b45ff, 0x22252eff;
        border_strong: "Borders under the pointer" = 0x4b5362ff, 0x3a3f4eff;
        text: "Text" = 0xccccccff, 0xe6e7ecff;
        text_dim: "Secondary text" = 0x9da5b4ff, 0x8a8e99ff;
        text_faint: "Hints and disabled text" = 0x6f7683ff, 0x5c606cff;
        accent: "Accent" = 0x61afefff, 0x5a96ffff;
        on_accent: "Text on the accent" = 0x16191dff, 0x080a10ff;
    }
    "Canvas" {
        background: "Background" = 0x1e2227ff, 0x090a0eff;
        card_fill: "File cards" = 0x23272ef0, 0x10162290;
        card_border: "File card borders" = 0x3e4452ff, 0x26304aff;
        card_note_fill: "Cards of skipped files" = 0x2e2529f0, 0x1a141490;
        dir_fill: "Folders" = 0xffffff05, 0xffffff04;
        dir_collapsed_fill: "Collapsed folders" = 0xffffff0e, 0xffffff0c;
        dir_border_0: "Folder borders, depth 1" = 0x4a6a8cff, 0x3d4f6eff;
        dir_border_1: "Folder borders, depth 2" = 0x4f7350ff, 0x2f5c4cff;
        dir_border_2: "Folder borders, depth 3" = 0x7a4f80ff, 0x4e4470ff;
        dir_border_3: "Folder borders, depth 4" = 0x7d6c4eff, 0x5c4c34ff;
        label_dir: "Folder names" = 0x9da5b4ff, 0x8b93abff;
        label_file: "File names" = 0xc5cad3ff, 0xa9b1d6ff;
        canvas_hover: "Outline under the pointer" = 0x8a93a6ff, 0x8aa0d0ff;
        select: "Selection outline" = 0x61afefff, 0x7aa2f7ff;
        select_fill: "Selection fill" = 0x61afef1c, 0x7aa2f714;
        chip_fill: "Labels on lines" = 0x1d1f23f2, 0x0b0e14f2;
        chip_text: "Label text" = 0xd7dae0ff, 0xe6e9f5ff;
        chip_text_dim: "Label secondary text" = 0xabb2bfff, 0xaab0c8ff;
        ui_text: "Text drawn on the canvas" = 0xccccccff, 0xc0caf5ff;
        ui_text_dim: "Secondary text on the canvas" = 0x7f848eff, 0x6b7394ff;
        ui_warn: "Warnings on the canvas" = 0xe5c07bff, 0xe0af68ff;
    }
    "Code" {
        tok_keyword: "Keywords" = 0xd55fdeff, 0xbb9af7ff;
        tok_type: "Types" = 0xe5c07bff, 0x2ac3deff;
        tok_function: "Functions" = 0x61afefff, 0x7aa2f7ff;
        tok_macro: "Macros" = 0x2bbac5ff, 0xff9e64ff;
        tok_string: "Strings" = 0x89ca78ff, 0x9ece6aff;
        tok_number: "Numbers" = 0xd19a66ff, 0xff9e64ff;
        tok_comment: "Comments" = 0x7f848eff, 0x5f6a94ff;
        tok_punct: "Punctuation" = 0xabb2bfc8, 0x89ddffb0;
        tok_ident: "Names" = 0xabb2bfff, 0xc0caf5ff;
        tok_heading: "Headings" = 0xef596fff, 0xe0af68ff;
        tok_text: "Prose" = 0xabb2bfff, 0xa9b1d6ff;
    }
    "Relations" {
        rel_def: "Definitions" = 0xff4fb8ff, 0xff4fb8ff;
        rel_impl: "Implementations" = 0x3ddc84ff, 0x3ddc84ff;
        rel_ref: "References" = 0xffb454ff, 0xffb454ff;
        rel_in: "Incoming calls" = 0xb48cffff, 0xb48cffff;
        rel_out: "Outgoing calls" = 0xe8663cff, 0xe8663cff;
        dep_out: "What the selection imports" = 0x2bbac5ff, 0x2ac3deff;
        dep_in: "What imports the selection" = 0xff79c6ff, 0xff79c6ff;
    }
    "Status" {
        problem_error: "Errors" = 0xef596fff, 0xf7768eff;
        problem_warning: "Warnings" = 0xe5c07bff, 0xe0af68ff;
        problem_info: "Information" = 0x61afefff, 0x7aa2f7ff;
        git_added: "Git: new" = 0x89ca78ff, 0x3ddc84ff;
        git_modified: "Git: changed" = 0xe2c08dff, 0x5a96ffff;
        git_removed: "Git: removed" = 0xef596fff, 0xf7768eff;
    }
}

pub const PRESETS: &[(&str, &str, Theme)] = &[("dark", "Dark", DARK), ("midnight", "Midnight", MIDNIGHT)];

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
        assert_eq!(PRESETS[0].2, Theme::default());
        assert_ne!(DARK, MIDNIGHT);
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
