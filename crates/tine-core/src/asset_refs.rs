//! Asset names a document mentions: the one collector behind orphan-asset
//! detection and publication asset copying. Pure: no I/O, no store state.

use std::collections::HashSet;

use crate::doc::{DocBlock, Document};

pub use crate::asset_path::percent_decode;
use crate::asset_path::{attachment_path, slash_normalized};

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
                    if let Some(target) = target {
                        // The renderer's answer (GH #984): one decode, slash
                        // normalization, then the shared attachment lookup.
                        let decoded = percent_decode(target);
                        if let Some(name) = attachment_path(&slash_normalized(&decoded)) {
                            insert_asset_path(into, name);
                        }
                        // The authored spelling stays a conservative extra.
                        if let Some((_, name)) = target.split_once("assets/") {
                            insert_asset_path(into, name);
                        }
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
    // Cheap prefilter: an attachment link spells `assets` literally (any case)
    // or hides part of it behind an escape (`%61ssets`, `assets%2F`).
    if !text.contains('%')
        && !text
            .as_bytes()
            .windows(6)
            .any(|w| w.eq_ignore_ascii_case(b"assets"))
    {
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

#[cfg(test)]
mod renderer_agreement_tests {
    use super::collect_asset_refs;
    use std::collections::HashSet;

    fn refs(text: &str) -> HashSet<String> {
        let mut into = HashSet::new();
        collect_asset_refs(text, &mut into);
        into
    }

    /// GH #984 review 1102600ff F1: a link the renderer opens as an attachment
    /// keeps that file alive, however its `assets` directory or separator is
    /// escaped, so the orphan list never offers it for trash.
    #[test]
    fn an_escaped_attachment_link_keeps_the_file_the_renderer_opens() {
        for (text, name) in [
            ("[p](../%61ssets/photo.png)", "photo.png"),
            ("[p](../assets%2Fphoto.png)", "photo.png"),
            ("[p](../assets%5Cphoto.png)", "photo.png"),
            ("[p](../%41ssets%5Cphoto.png)", "photo.png"),
            ("![p](../ASSETS/photo.png)", "photo.png"),
            ("[p](../assets/sub%5Cphoto.png)", "sub/photo.png"),
            ("[[../%61ssets/photo.png][p]]", "photo.png"),
            ("[p](../assets/my%20file.png)", "my file.png"),
        ] {
            assert!(
                refs(text).contains(name),
                "{text} must keep {name}: {:?}",
                refs(text)
            );
        }
    }

    #[test]
    fn plaintext_mentions_stay_conservative_and_unrelated_text_skips_parsing() {
        assert!(refs("see \"assets/notes.txt\" here").contains("notes.txt"));
        assert!(refs("no attachment here").is_empty());
    }
}
