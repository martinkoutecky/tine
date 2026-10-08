//! Block/page relation vocabulary shared by every query consumer.

use serde::{Deserialize, Serialize};

/// One relation of the anchor row. Bare identifiers inside a relation predicate
/// bind to the ELEMENT, never to the outer row (SPEC §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rel {
    /// OG `:block/path-refs`: this block's refs, every ancestor's, and its page.
    Refs,
    /// The block's own inline `#tag` / Org headline tags (Tine-only leaf, Q2).
    Tags,
    /// Property elements of the owner (block or page).
    Props,
    /// Direct children of a block (A1).
    Children,
    /// The direct parent block, empty for roots (never the owning page).
    Parent,
    /// All strict ancestor blocks, excluding this block and its page.
    Ancestors,
    /// All strict descendant blocks at any depth, excluding this block.
    Descendants,
    /// Every block of a page (`@page` anchor).
    Blocks,
    /// The owning page of a block (to-one).
    Page,
}

impl Rel {
    pub fn tql_name(self) -> &'static str {
        match self {
            Rel::Refs => "refs",
            Rel::Tags => "tags",
            Rel::Props => "props",
            Rel::Children => "children",
            Rel::Parent => "parent",
            Rel::Ancestors => "ancestors",
            Rel::Descendants => "descendants",
            Rel::Blocks => "blocks",
            Rel::Page => "page",
        }
    }
}
