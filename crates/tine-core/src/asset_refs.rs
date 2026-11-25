//! Asset names a document mentions: the one collector behind orphan-asset
//! detection, the trash recheck and publication asset copying. Pure: no I/O,
//! no store state.
//!
//! Two answers are combined, and neither may remove a name the other gives:
//! - the collector's own link split and loose plaintext mentions (what this
//!   module answered before the link-target owner existed). It stays the
//!   base until the reader asks the owner too, because until then it is the
//!   only answer that never forgets a spelling the reader may show;
//! - the link-target owner ([`crate::local_target`]), asked for every Markdown
//!   link and embed, Org link and `img` macro the parser reports; only its
//!   attachments are kept.

use std::collections::HashSet;

use crate::doc::{DocBlock, Document};
use crate::local_target::{local_target, Destination, LocalTarget, Origin};

pub use crate::local_target::percent_decode;

/// Liveness (orphan listing, trash recheck): parser-accepted link targets and
/// conservative plaintext mentions, plus the owner's attachments (with the
/// first folder of each nested one). The mentions may retain extra files
/// (including literal code) but cannot shorten or replace an accepted target.
/// O(text bytes); parsing is skipped when the text names no `assets` folder.
pub fn collect_asset_refs(text: &str, into: &mut HashSet<String>) {
    let mut targets = HashSet::new();
    collect(text, Some(&mut *into), &mut targets);
    for name in targets {
        if let Some((folder, _)) = name.split_once('/') {
            into.insert(folder.to_owned());
        }
        into.insert(name);
    }
}

/// The owner's attachments alone: exactly the files the owner says `text`'s
/// destinations name (decoded once; the folder itself is no file).
/// O(text bytes); no parse when the text names no `assets` folder.
pub fn collect_asset_targets(text: &str, into: &mut HashSet<String>) {
    collect(text, None, into);
}

/// One parse per format serves both answers. `collected`, when given,
/// receives the collector's own answer: parser-accepted targets split after
/// `assets/` under raw and decoded spellings, and the loose plaintext
/// mentions. `targets` receives the owner's attachments.
fn collect(text: &str, collected: Option<&mut HashSet<String>>, targets: &mut HashSet<String>) {
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
    fn keep(target: LocalTarget, into: &mut HashSet<String>) {
        if let Some(name) = target.attachment().filter(|n| !n.as_str().is_empty()) {
            into.insert(name.as_str().to_owned());
        }
    }
    fn owner(nodes: &[Inline], md: bool, into: &mut HashSet<String>) {
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
                    keep(local_target(Destination::Link { url }, origin), into);
                    owner(label, md, into);
                }
                // The macro's URL is the first token of the parser's first argument.
                Inline::Macro { name, args, .. } if crate::render::builtin_macro(name) == "img" => {
                    let url = args
                        .first()
                        .and_then(|a| a.split_whitespace().next())
                        .unwrap_or("");
                    keep(local_target(Destination::Text(url), Origin::ImgMacro), into);
                }
                Inline::Emphasis { children, .. }
                | Inline::Subscript { children, .. }
                | Inline::Superscript { children, .. }
                | Inline::Tag { children, .. } => owner(children, md, into),
                Inline::Fnref { definition, .. } => owner(definition, md, into),
                _ => (),
            }
        }
    }
    if !names_assets_folder(text) {
        return;
    }
    // The collector's own answer keeps its own prefilter (`assets/`, which
    // implies the owner's).
    let mut collected = collected.filter(|_| text.contains("assets/"));
    // Preambles carry no format argument. Both parsers may conservatively add
    // accepted targets; neither removes a target recognized by the other.
    for format in ["md", "org"] {
        if let Some(nodes) = crate::render::parse_inline_bounded(text, format) {
            if let Some(into) = collected.as_deref_mut() {
                links(&nodes, into);
            }
            owner(&nodes, format == "md", targets);
        }
    }
    if let Some(into) = collected {
        conservative_asset_mentions(text, into);
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

/// The owner's prefilter: the owner makes an attachment only through a folder
/// literally written `assets`, in any case, so a text without those six
/// letters (any case) names none. One vectorized search per window of a
/// folded stack copy: `b | 0x20` is `a` exactly for `a` and `A` (likewise
/// `s`, `e`, `t`), so the fold finds every case and nothing else. Windows
/// overlap by five bytes, so no occurrence straddles two. O(text bytes), no
/// allocation.
fn names_assets_folder(text: &str) -> bool {
    const WINDOW: usize = 256;
    const OVERLAP: usize = b"assets".len() - 1;
    static FINDER: std::sync::OnceLock<memchr::memmem::Finder<'static>> =
        std::sync::OnceLock::new();
    let finder = FINDER.get_or_init(|| memchr::memmem::Finder::new(b"assets"));
    let bytes = text.as_bytes();
    let mut folded = [0u8; WINDOW + OVERLAP];
    let mut start = 0;
    while start + OVERLAP < bytes.len() {
        let window = &bytes[start..bytes.len().min(start + WINDOW + OVERLAP)];
        let folded = &mut folded[..window.len()];
        for (to, from) in folded.iter_mut().zip(window) {
            *to = from | 0x20;
        }
        if finder.find(folded).is_some() {
            return true;
        }
        start += WINDOW;
    }
    false
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
    each_text(doc, &mut |text| collect_asset_refs(text, into));
}

/// The names export starts from, kept as two answers so neither can narrow
/// the other: `collected` is the collector's own answer (raw and decoded
/// spellings, mentions, folder markers), `targets` the owner's attachments.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExportNames {
    pub collected: HashSet<String>,
    pub targets: HashSet<String>,
}

/// [`ExportNames`] of one text, with one parse per format. O(text bytes).
pub fn collect_export_names(text: &str, into: &mut ExportNames) {
    collect(text, Some(&mut into.collected), &mut into.targets);
}

/// [`collect_export_names`] over a document's preamble and every block.
/// Cost O(document text bytes).
pub fn collect_document_export_names(doc: &Document, into: &mut ExportNames) {
    each_text(doc, &mut |text| collect_export_names(text, into));
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
