//! Asset names a document mentions: the one collector behind orphan-asset
//! detection and publication asset copying. Pure: no I/O, no store state.

use std::collections::HashSet;

use crate::doc::{DocBlock, Document};

/// Decode `%XX` percent-escapes (UTF-8 aware, like JS `decodeURIComponent`). An
/// invalid or truncated escape is left literal rather than dropped.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex_nibble(b[i + 1]), hex_nibble(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Asset liveness combines parser-accepted targets with conservative plaintext
/// mentions. The latter may retain extra files (including literal code), but
/// cannot shorten or replace an accepted target. Parsing is skipped entirely
/// when there is no asset mention.
pub fn collect_asset_refs(text: &str, into: &mut HashSet<String>) {
    use crate::lsdoc::ast::{Inline, Url};
    fn links(nodes: &[Inline], into: &mut HashSet<String>) {
        for node in nodes {
            match node {
                Inline::Link { url, label, .. } => {
                    let target = match url {
                        Url::Search { v } | Url::File { v } => Some(v.as_str()),
                        Url::Complex { link, .. } => link.as_deref(),
                        _ => None,
                    };
                    if let Some((_, name)) = target.and_then(|t| t.split_once("assets/")) {
                        insert_asset_path(into, name);
                    }
                    links(label, into);
                }
                Inline::Emphasis { children, .. }
                | Inline::Subscript { children, .. }
                | Inline::Superscript { children, .. }
                | Inline::Tag { children, .. } => links(children, into),
                Inline::Fnref { definition, .. } => links(definition, into),
                _ => (),
            }
        }
    }
    if !text.contains("assets/") {
        return;
    }
    // Preambles carry no format argument. Both parsers may conservatively add
    // accepted targets; neither removes a target recognized by the other.
    for format in ["md", "org"] {
        if let Some(nodes) = crate::render::parse_inline_bounded(text, format) {
            links(&nodes, into);
        }
    }
    conservative_asset_mentions(text, into);
}

fn insert_asset_path(into: &mut HashSet<String>, name: &str) {
    if name.is_empty() {
        return;
    }
    insert_asset_ref(into, name);
    if let Some((segment, _)) = name.split_once('/') {
        insert_asset_ref(into, segment);
    }
}

/// Legacy plaintext safety policy, NOT link recognition: any assets/ mention
/// can keep a file alive even outside accepted links. Delimiters bound an extra
/// conservative candidate only; link targets above always come from lsdoc.
fn conservative_asset_mentions(text: &str, into: &mut HashSet<String>) {
    let mut rest = text;
    while let Some(i) = rest.find("assets/") {
        let after = &rest[i + "assets/".len()..];
        let end = after
            .find(|c: char| {
                matches!(
                    c,
                    ')' | ']' | '"' | '\'' | '<' | '>' | '|' | '\n' | '\r' | '\t'
                )
            })
            .unwrap_or(after.len());
        let name = &after[..end];
        insert_asset_path(into, name);
        rest = &after[end..];
    }
}

/// Record an asset reference under BOTH its raw form AND its percent-decoded form.
/// A link like `../assets/my%20file.png` names the on-disk file `my file.png`, so
/// comparing the raw URL substring against directory entries would miss the real
/// file and let `orphan_assets` offer an IN-USE asset for trashing (DS Codex#7).
/// Keeping the raw form too covers a file literally named with a `%` escape.
fn insert_asset_ref(into: &mut HashSet<String>, raw: &str) {
    let decoded = percent_decode(raw);
    if decoded != raw {
        into.insert(decoded);
    }
    into.insert(raw.to_string());
}

/// [`collect_asset_refs`] over one block and its whole subtree.
pub fn collect_block_asset_refs(b: &DocBlock, into: &mut HashSet<String>) {
    collect_asset_refs(b.raw(), into);
    for c in &b.children {
        collect_block_asset_refs(c, into);
    }
}

/// [`collect_asset_refs`] over a document's preamble and every block.
/// Cost O(document text bytes).
pub fn collect_document_asset_refs(doc: &Document, into: &mut HashSet<String>) {
    if let Some(pre) = &doc.pre_block {
        collect_asset_refs(pre, into);
    }
    for block in &doc.roots {
        collect_block_asset_refs(block, into);
    }
}
