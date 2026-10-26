//! tine-outline: the outline kernel's crate (outline-kernel DESIGN §2).
//!
//! It holds the leaf text rules Tine's outline edits need: Tine's one-block
//! lsdoc boundary, parser-owned block regions and their structural editor, the
//! logbook, the Markdown property-line grammar, the page-filename codec and the
//! closed edit-kind vocabulary. Each compiles to wasm32 with only `lsdoc` and
//! `serde`, so one source serves the native backend and the window's wasm.
//! `tine-core`, `tine-store` and `lsdoc-wasm` re-export these modules; nothing
//! includes these files by `#[path]`.

pub mod block_regions;
pub mod edit_kind;
pub mod editor_text;
pub mod logbook;
pub mod lsdoc_block_parse;
pub mod outline;
pub mod page_filename;
pub mod property_line;
pub mod property_refs;
mod swallowed_rows;

/// The parse entry `block_regions` asks through (`crate::render::…`), the same
/// names `tine-core`'s render bridge exposes.
pub(crate) mod render {
    pub(crate) use crate::lsdoc_block_parse::parse_block;

    pub(crate) fn parse_text_bounded(text: &str, is_org: bool) -> Option<lsdoc::ast::Projection> {
        crate::lsdoc_block_parse::parse_text_bounded(text, if is_org { "org" } else { "md" })
    }
}
