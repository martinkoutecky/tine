//! Asset names a document mentions: the one collector behind orphan-asset
//! detection and publication asset copying. Pure: no I/O, no store state.

use std::collections::HashSet;

use crate::doc::{DocBlock, Document};
use crate::model::Format;

pub use crate::asset_path::percent_decode;
use crate::asset_path::{asset_rel_path, decoded_local_path, is_pdf_path, pdf_asset_file_decoded};

/// Collect authored attachment targets from one inline run. Orphan inventory
/// also keeps conservative plaintext mentions and the first segment of nested
/// paths. Publication uses only resolved file names. O(text bytes), no I/O.
pub fn collect_asset_refs(text: &str, format: &str, liveness: bool, into: &mut HashSet<String>) {
    use crate::lsdoc::ast::{Inline, Url};
    fn links(
        nodes: &[Inline],
        format: &str,
        liveness: bool,
        mentions: bool,
        into: &mut HashSet<String>,
    ) {
        for node in nodes {
            match node {
                Inline::Link { url, label, .. } => {
                    match url {
                        Url::Search { v } => insert_link_target(v, LinkForm::Search, liveness, into),
                        Url::PageRef { v } if format == "org" => {
                            insert_link_target(v, LinkForm::OrgPageRef, liveness, into);
                        }
                        Url::Complex { protocol, link: Some(v) }
                            if protocol.as_deref().unwrap_or("").is_empty() =>
                        {
                            insert_link_target(v, LinkForm::Link, liveness, into);
                        }
                        _ => (),
                    }
                    // Bare URLs (including web media) can have the URL itself
                    // as their label. Only nested links in labels can name files.
                    links(label, format, liveness, false, into);
                }
                Inline::Macro { name, args, .. } if name.eq_ignore_ascii_case("img") => {
                    let body = if let [arg] = args.as_slice() {
                        std::borrow::Cow::Borrowed(arg.as_str())
                    } else {
                        std::borrow::Cow::Owned(args.join(", "))
                    };
                    if let Some(target) = body.split_whitespace().next() {
                        insert_link_target(target, LinkForm::Image, liveness, into);
                    }
                }
                Inline::Plain { text, .. }
                | Inline::Code { text, .. }
                | Inline::Verbatim { text, .. }
                    if mentions =>
                {
                    conservative_asset_mentions(text, into);
                }
                Inline::Emphasis { children, .. }
                | Inline::Subscript { children, .. }
                | Inline::Superscript { children, .. }
                | Inline::Tag { children, .. } => links(children, format, liveness, mentions, into),
                Inline::Fnref { definition, .. } => links(definition, format, liveness, mentions, into),
                _ => (),
            }
        }
    }
    // Only a written marker can name a file. Escapes and prose percentages
    // alone never warrant a parse (D-20261120-A984).
    if asset_rel_path(text).is_none() {
        return;
    }
    #[cfg(test)]
    PARSE_COUNT.with(|count| count.set(count.get() + 1));
    if let Some(nodes) = crate::render::parse_inline_bounded(text, format) {
        links(&nodes, format, liveness, liveness, into);
    } else if liveness {
        conservative_asset_mentions(text, into);
    }
}

fn insert_asset_path(into: &mut HashSet<String>, name: &str, liveness: bool) {
    if !name.is_empty() {
        into.insert(name.to_owned());
        if liveness {
            if let Some((segment, _)) = name.split_once('/') {
                into.insert(segment.to_owned());
            }
        }
    }
}

enum LinkForm {
    Search,
    OrgPageRef,
    Link,
    Image,
}

fn insert_link_target(target: &str, form: LinkForm, liveness: bool, into: &mut HashSet<String>) {
    let target = target.trim();
    if target.split_once(':').is_some_and(|(prefix, _)| {
        let mut bytes = prefix
            .trim_start_matches(|c: char| c <= ' ')
            .bytes()
            .filter(|b| !matches!(b, b'\t' | b'\n' | b'\r'));
        bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
            && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    }) {
        return;
    }
    let (path, attachment) = decoded_local_path(target, true);
    let Some(start) = attachment else { return };
    // Encoded Windows drive links belong to the system file opener.
    if matches!(path.as_bytes(), [b'a'..=b'z' | b'A'..=b'Z', b':', b'/', ..]) {
        return;
    }
    let name = path[start..].trim_end_matches('/');
    match form {
        LinkForm::Search if !path.contains('.') => return,
        LinkForm::OrgPageRef => {
            // Same excludes as the renderer's bare Org image branch.
            let stem = path.split(['#', '?']).next().unwrap_or_default();
            if stem.rsplit_once('.').is_some_and(|(_, ext)| {
                ["pdf", "mp4", "ogg", "webm"].iter().any(|skip| ext.eq_ignore_ascii_case(skip))
            }) {
                return;
            }
        }
        _ => (),
    }
    if matches!(form, LinkForm::Search | LinkForm::Link) && is_pdf_path(&path) {
        let name = pdf_asset_file_decoded(&path, Some(name));
        insert_asset_path(into, &name, liveness);
    } else {
        insert_asset_path(into, name, liveness);
    }
}

/// Legacy plaintext safety policy for the orphan inventory. Delimiters bound
/// a conservative candidate. Parsed destinations keep their exact identities.
fn conservative_asset_mentions(text: &str, into: &mut HashSet<String>) {
    let mut rest = text;
    while let Some(after) = asset_rel_path(rest) {
        let end = after
            .find(|c: char| {
                matches!(
                    c,
                    ')' | ']' | '"' | '\'' | '<' | '>' | '|' | '\n' | '\r' | '\t'
                )
            })
            .unwrap_or(after.len());
        let name = &after[..end];
        insert_asset_path(into, name, true);
        if name.contains('%') {
            insert_asset_path(into, &percent_decode(name), true);
        }
        rest = &after[end..];
    }
}

fn collect_block_asset_refs(b: &DocBlock, format: &str, liveness: bool, into: &mut HashSet<String>) {
    collect_asset_refs(b.raw(), format, liveness, into);
    for c in &b.children {
        collect_block_asset_refs(c, format, liveness, into);
    }
}

/// [`collect_asset_refs`] over a document's preamble and every block.
/// Cost O(document text bytes).
pub fn collect_document_asset_refs(
    doc: &Document,
    format: Format,
    liveness: bool,
    into: &mut HashSet<String>,
) {
    let format = format.ext();
    if let Some(pre) = &doc.pre_block {
        collect_asset_refs(pre, format, liveness, into);
    }
    for block in &doc.roots {
        collect_block_asset_refs(block, format, liveness, into);
    }
}

#[cfg(test)]
thread_local! {
    static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod renderer_agreement_tests {
    use super::collect_asset_refs;
    use std::collections::HashSet;

    fn refs(text: &str) -> HashSet<String> {
        let mut into = HashSet::new();
        collect_asset_refs(text, "md", true, &mut into);
        into
    }

    #[test]
    fn asset_prefilter_counts_only_authored_markers() {
        for (text, parses) in [
            ("50% complete", 0),
            ("50% of the assets are ready", 0),
            ("[p](../%61ssets/photo.png)", 0),
            ("[p](../assets%2Fphoto.png)", 0),
            ("[p](../assets%5Cphoto.png)", 0),
            ("[p](../%41ssets%5Cphoto.png)", 0),
            ("![p](../ASSETS/photo.png)", 1),
            ("[p](../assets/sub%5Cphoto.png)", 1),
            ("[p](../aSsEtS/my%20file.png)", 1),
        ] {
            super::PARSE_COUNT.with(|count| count.set(0));
            let names = refs(text);
            assert_eq!(super::PARSE_COUNT.with(std::cell::Cell::get), parses, "{text}");
            assert_eq!(names.is_empty(), parses == 0, "{text}");
        }
    }

    #[test]
    fn plaintext_mentions_stay_conservative_and_unrelated_text_skips_parsing() {
        assert!(refs("see \"assets/notes.txt\" here").contains("notes.txt"));
        assert!(refs("no attachment here").is_empty());
    }
}
