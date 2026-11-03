//! The prose of a Markdown page preamble (CM step 2, batch 8).

use crate::block_regions::{page_header, Range};

/// The prose of a Markdown page preamble: what follows the
/// [`page_header`](crate::block_regions::page_header) (the whole text when it
/// has none), without the newlines at either edge. `None` when nothing is
/// left. OG reads the text before a page's first block as its first block
/// (graph-parser `block.cljs with-pre-block-if-exists`); Tine shows this part
/// as that block's row (`Page.tsx`) and promotes it to a real block when
/// edited (GH #85), so every reference question reads it as a block. The one
/// answer for both sides (the window asks it through wasm). O(bytes).
pub fn page_preamble_content(raw: &str) -> Option<Range> {
    let header = page_header(raw);
    let start = if header.entries.is_empty() {
        0
    } else {
        header.end
    };
    let rest = &raw[start..];
    let leading = rest.len() - rest.trim_start_matches('\n').len();
    let content = rest.trim_matches('\n');
    (!content.is_empty()).then(|| Range(start + leading, start + leading + content.len()))
}

#[cfg(test)]
mod tests {
    use super::page_preamble_content;

    fn text(raw: &str) -> Option<&str> {
        page_preamble_content(raw).map(|r| &raw[r.0..r.1])
    }

    #[test]
    fn the_prose_after_the_page_header_without_edge_newlines() {
        assert_eq!(text("[[Target]] in prose"), Some("[[Target]] in prose"));
        assert_eq!(
            text("tags:: x\n\n\nsee [[Target]]\n\n"),
            Some("see [[Target]]")
        );
        assert_eq!(text("tags:: x"), None);
        assert_eq!(text(""), None);
    }
}
