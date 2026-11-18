pub mod cache;
pub mod deps;
pub mod geom;
pub mod layout;
pub mod refs;
pub mod scan;
pub mod store;
pub mod symbols;
pub mod syntax;
pub mod wrap;

#[cfg(test)]
mod updates;

pub use cache::{Painted, Painter, Piece, TokenCache};
pub use geom::Rect;
pub use layout::{Layout, LayoutFilter, Node, NodeKind};
pub use refs::{RelKind, Relation, Relations, Site, SymRef, SymbolIndex};
pub use scan::{Dir, Repo, ScanOptions, ScanProgress, SourceFile};
pub use symbols::{Symbol, SymbolKind};
pub use syntax::{Block, Lang, Token, TokenKind, Tokens, BLOCK_LINES, CHUNK_LINES};
pub use wrap::Wrap;

pub const CHAR_W: f32 = 6.0;
pub const LINE_H: f32 = 12.0;
pub const TAB_WIDTH: usize = 4;
