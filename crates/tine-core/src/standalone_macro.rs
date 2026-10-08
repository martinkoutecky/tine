//! Sole visible macro policy over lsdoc's accepted block AST. Properties and
//! whitespace do not contribute visible content; any other node does. O(nodes),
//! borrowed result, no parsing, allocation or I/O. Callers choose macro names.
use lsdoc::ast::{Block, Inline};

/// Return the sole macro node, refusing body text, literal containers, headers
/// with task/priority/heading facets, and multiple macros.
pub fn sole_macro(blocks: &[Block]) -> Option<&Inline> {
    let mut found = None;
    for block in blocks {
        let inline = match block {
            Block::Properties { .. } => continue,
            Block::Paragraph { inline, .. } => inline,
            Block::Bullet {
                inline,
                marker,
                priority,
                size,
                ..
            } if marker.is_none() && priority.is_none() && size.is_none() => inline,
            _ => return None,
        };
        for node in inline {
            match node {
                Inline::Plain { text, .. } if text.trim().is_empty() => {}
                Inline::Break { .. } => {}
                Inline::Macro { .. } if found.is_none() => found = Some(node),
                _ => return None,
            }
        }
    }
    found
}
