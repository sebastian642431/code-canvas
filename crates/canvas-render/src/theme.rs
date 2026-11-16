//! Colours. Everything is `0xRRGGBBAA` in sRGB; the surface is non-sRGB so
//! values go to the screen untouched (same blending as most UI toolkits).

use canvas_core::TokenKind;

pub const BACKGROUND: [f64; 3] = [0.035, 0.040, 0.055];

pub const DIR_FILL: u32 = 0xffffff04;
pub const DIR_COLLAPSED_FILL: u32 = 0xffffff0c;
/// Directory borders cycle through these by depth.
pub const DIR_BORDERS: [u32; 4] = [0x3d4f6eff, 0x2f5c4cff, 0x4e4470ff, 0x5c4c34ff];
pub const CARD_FILL: u32 = 0x10162290;
pub const CARD_BORDER: u32 = 0x26304aff;
pub const CARD_NOTE_FILL: u32 = 0x1a141490;

/// Relation colours, indexed by `RelKind::index()`:
/// definitions, implementations, references, incoming calls, outgoing calls.
pub const REL_COLORS: [u32; 5] = [0xff4fb8ff, 0x3ddc84ff, 0xffb454ff, 0xb48cffff, 0xe8663cff];
pub const CHIP_FILL: u32 = 0x0b0e14f2;
pub const CHIP_TEXT: u32 = 0xe6e9f5ff;
pub const CHIP_TEXT_DIM: u32 = 0xaab0c8ff;

pub const HOVER: u32 = 0x8aa0d0ff;
pub const SELECT: u32 = 0x7aa2f7ff;
pub const SELECT_FILL: u32 = 0x7aa2f714;

/// Problems reported by extensions (linters, language servers).
pub const PROBLEM_ERROR: u32 = 0xf7768eff;
pub const PROBLEM_WARNING: u32 = 0xe0af68ff;
pub const PROBLEM_INFO: u32 = 0x7aa2f7ff;

/// Source control: new, changed and removed.
pub const GIT_ADDED: u32 = 0x3ddc84ff;
pub const GIT_MODIFIED: u32 = 0x5a96ffff;
pub const GIT_REMOVED: u32 = 0xf7768eff;

pub const LABEL_DIR: u32 = 0x8b93abff;
pub const LABEL_FILE: u32 = 0xa9b1d6ff;

pub const UI_BAR: u32 = 0x0c0f15ff;
pub const UI_BAR_BORDER: u32 = 0x1f2533ff;
pub const UI_TAB: u32 = 0x00000000;
pub const UI_TAB_HOVER: u32 = 0x161b26ff;
pub const UI_TAB_ACTIVE: u32 = 0x1b2232ff;
pub const UI_ACCENT: u32 = 0x7aa2f7ff;
pub const UI_TEXT: u32 = 0xc0caf5ff;
pub const UI_TEXT_DIM: u32 = 0x6b7394ff;
pub const UI_WARN: u32 = 0xe0af68ff;

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

/// Syntax colours, indexed by [`kind_index`].
pub const TOKEN_COLORS: [u32; 16] = [
    0xbb9af7ff, // keyword
    0x2ac3deff, // type
    0x7aa2f7ff, // function
    0xff9e64ff, // macro
    0x9ece6aff, // string
    0xff9e64ff, // number
    0x5f6a94ff, // comment
    0x89ddffb0, // punctuation
    0xc0caf5ff, // identifier
    0xe0af68ff, // heading
    0xa9b1d6ff, // prose
    0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
];

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
