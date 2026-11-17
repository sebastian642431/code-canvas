//! `canvas-core`: everything Code Canvas knows about a repository that does
//! not need a GPU.
//!
//! * [`scan`] walks a directory (respecting `.gitignore`) and loads source files.
//! * [`syntax`] is a tiny, dependency-free tokenizer used to colour code.
//! * [`layout`] turns the directory tree into nested boxes on an infinite canvas.
//!
//! All canvas coordinates are expressed in *world units*. One monospace
//! character cell is [`CHAR_W`] × [`LINE_H`] world units, so at zoom `1.0` the
//! code is drawn at roughly a 10px font.

pub mod cache;
pub mod deps;
pub mod geom;
pub mod layout;
pub mod refs;
pub mod scan;
pub mod symbols;
pub mod syntax;

pub use cache::TokenCache;
pub use geom::Rect;
pub use layout::{Layout, LayoutFilter, Node, NodeKind};
pub use refs::{RelKind, Relation, Relations, Site, SymRef, SymbolIndex};
pub use scan::{Dir, Repo, ScanOptions, ScanProgress, SourceFile};
pub use symbols::{Symbol, SymbolKind};
pub use syntax::{Block, Lang, Token, TokenKind, Tokens, BLOCK_LINES, CHUNK_LINES};

/// Width of one monospace character cell, in world units.
pub const CHAR_W: f32 = 6.0;
/// Height of one line of code, in world units.
pub const LINE_H: f32 = 12.0;
/// Tabs are expanded to this many spaces when files are loaded.
pub const TAB_WIDTH: usize = 4;
