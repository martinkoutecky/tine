//! Asset names a document mentions: the one collector behind orphan-asset
//! detection, the trash recheck and publication asset copying. Which file a
//! destination names is not decided here: every destination the parser
//! reports (Markdown links and embeds, Org links, the `img` macro, raw HTML
//! and Hiccup resources) is answered by the link-target owner
//! ([`crate::local_target`]), and only its attachments are kept. Pure: no
//! I/O, no store state.

use std::collections::HashSet;

use crate::doc::{DocBlock, Document};
use crate::local_target::{local_target, AssetName, Destination, LocalTarget, Origin};

/// Liveness (orphan listing, trash recheck): the attachments [`asset_targets`]
/// finds, with the first folder of each nested one, plus the loose plaintext
/// mentions. The mentions may keep extra files alive (literal code, a web URL
/// with an `assets/` folder) but never shorten or replace a target. O(text
/// bytes); no parse when the text names no `assets` folder.
pub fn collect_asset_refs(text: &str, into: &mut HashSet<String>) {
    if !names_assets_folder(text) {
        return;
    }
    each_target(text, &mut |name| {
        let name = name.as_str();
        into.insert(name.to_owned());
        if let Some((folder, _)) = name.split_once('/') {
            into.insert(folder.to_owned());
        }
    });
    conservative_asset_mentions(text, into);
}

/// Export: exactly the attachments the reader resolves `text`'s destinations
/// to, as the owner names them (decoded once; the folder itself is no file).
/// O(text bytes); no parse when the text names no `assets` folder.
pub fn collect_asset_targets(text: &str, into: &mut HashSet<String>) {
    if !names_assets_folder(text) {
        return;
    }
    each_target(text, &mut |name| {
        into.insert(name.as_str().to_owned());
    });
}

/// The prefilter: the owner makes an attachment only through a folder
/// literally written `assets`, in any case, so a text without those six
/// letters (any case) names none, unless raw HTML spells a letter as a
/// character reference (`&#97;ssets`), which the reader decodes first.
/// O(text bytes), no allocation.
fn names_assets_folder(text: &str) -> bool {
    let bytes = text.as_bytes();
    memchr::memchr2_iter(b'a', b'A', bytes).any(|i| {
        bytes
            .get(i + 1..i + 6)
            .is_some_and(|rest| rest.eq_ignore_ascii_case(b"ssets"))
    }) || memchr::memmem::find(bytes, b"&#").is_some()
}

/// Every non-empty attachment the owner answers for a destination in `text`.
/// Preambles carry no format argument, so both parsers read the text; each
/// may only add targets.
fn each_target(text: &str, f: &mut dyn FnMut(&AssetName)) {
    use crate::lsdoc::ast::Inline;
    fn keep(target: LocalTarget, f: &mut dyn FnMut(&AssetName)) {
        if let Some(name) = target.attachment().filter(|n| !n.as_str().is_empty()) {
            f(name);
        }
    }
    fn html(urls: Vec<String>, f: &mut dyn FnMut(&AssetName)) {
        for url in urls {
            keep(
                local_target(Destination::Text(&url), Origin::HtmlResource),
                f,
            );
        }
    }
    fn walk(nodes: &[Inline], md: bool, f: &mut dyn FnMut(&AssetName)) {
        for node in nodes {
            match node {
                Inline::Link {
                    url, label, image, ..
                } => {
                    let origin = match (md, *image) {
                        (true, true) => Origin::MdEmbed,
                        (true, false) => Origin::MdLink,
                        // `[[x]]` has no label; `[[x][label]]` has one.
                        (false, _) if label.is_empty() => Origin::OrgBare,
                        (false, _) => Origin::OrgLink,
                    };
                    keep(local_target(Destination::Link { url }, origin), f);
                    walk(label, md, f);
                }
                // The macro's URL is the first token of the parser's first argument.
                Inline::Macro { name, args, .. } if crate::render::builtin_macro(name) == "img" => {
                    let url = args
                        .first()
                        .and_then(|a| a.split_whitespace().next())
                        .unwrap_or("");
                    keep(local_target(Destination::Text(url), Origin::ImgMacro), f);
                }
                Inline::InlineHtml { text, .. } => {
                    html(crate::html_sanitize::resource_urls(text), f)
                }
                Inline::Hiccup { v, .. } => html(crate::html_sanitize::hiccup_resource_urls(v), f),
                Inline::Emphasis { children, .. }
                | Inline::Subscript { children, .. }
                | Inline::Superscript { children, .. }
                | Inline::Tag { children, .. } => walk(children, md, f),
                Inline::Fnref { definition, .. } => walk(definition, md, f),
                _ => (),
            }
        }
    }
    for format in ["md", "org"] {
        if let Some(nodes) = crate::render::parse_inline_bounded(text, format) {
            walk(&nodes, format == "md", f);
        }
    }
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
/// conservative candidate only; link targets above always come from the owner.
/// Liveness only: export never copies a mention.
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

/// Record a plaintext mention under BOTH its raw form AND its percent-decoded
/// form: a mention of `my%20file.png` may mean the file `my file.png` or a
/// file literally named with the escape, and liveness must keep both.
fn insert_asset_ref(into: &mut HashSet<String>, raw: &str) {
    let decoded = crate::local_target::percent_decode(raw);
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
    each_text(doc, &mut |text| collect_asset_refs(text, into));
}

/// [`collect_asset_targets`] over a document's preamble and every block.
/// Cost O(document text bytes).
pub fn collect_document_asset_targets(doc: &Document, into: &mut HashSet<String>) {
    each_text(doc, &mut |text| collect_asset_targets(text, into));
}

fn each_text(doc: &Document, f: &mut dyn FnMut(&str)) {
    fn block(b: &DocBlock, f: &mut dyn FnMut(&str)) {
        f(b.raw());
        for c in &b.children {
            block(c, f);
        }
    }
    if let Some(pre) = &doc.pre_block {
        f(pre.as_str());
    }
    for b in &doc.roots {
        block(b, f);
    }
}
