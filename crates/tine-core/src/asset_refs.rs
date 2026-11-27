//! Asset names a document mentions: the one collector behind orphan-asset
//! detection and publication asset copying. Pure: no I/O, no store state.
//!
//! Liveness (orphan listing, trash recheck) adds two answers, and none may
//! remove a name another gives:
//! - the collector's own link split and loose plaintext mentions
//!   ([`collect_asset_refs`]). It stays the base until the reader asks the
//!   link-target owner too, because until then it is the only answer that
//!   never forgets a spelling the reader may show. Publication copies this
//!   answer alone;
//! - the link-target owner ([`crate::local_target`]), asked for every Markdown
//!   link and embed, Org link and `img` macro the parser reports; only its
//!   attachments are kept ([`collect_live_asset_refs`]);
//! - the widening rule W ([`widen_one`]): one generous rule that holds every
//!   name any reader, opener or exporter can make of a written destination,
//!   compared by [`is_live`], so no file a surface uses is offered for trash.

use std::collections::HashSet;

use crate::doc::{DocBlock, Document};
use crate::local_target::{local_target, Destination, LocalTarget, Origin};

pub use crate::local_target::percent_decode;
use tine_outline::editor_text::{is_js_whitespace, js_trim};
use tine_outline::page_filename::decode_escapes_keeping_malformed as lenient_decode;

#[cfg(test)]
thread_local! {
    /// Bytes the prefilter's closer searches were handed (a cost test's count).
    pub(crate) static PREFILTER_SEARCHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Block parses the `file-path` read made (a cost test's count).
    pub(crate) static PROPERTY_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Adds `n` to a cost test's count; compiled out of every other build.
macro_rules! count {
    ($counter:ident, $n:expr) => {
        #[cfg(test)]
        $counter.with(|c| c.set(c.get() + $n));
    };
}

/// Asset liveness combines parser-accepted targets with conservative plaintext
/// mentions. The latter may retain extra files (including literal code), but
/// cannot shorten or replace an accepted target. Parsing is skipped entirely
/// when there is no asset mention.
pub fn collect_asset_refs(text: &str, into: &mut HashSet<String>) {
    if !text.contains("assets/") {
        return;
    }
    collect(text, Some(into), None, None);
}

/// Liveness (orphan listing, trash recheck): L' = L ∪ W. L is
/// [`collect_asset_refs`] plus the owner's attachments (with the first folder
/// of each nested one), kept as written; W is the widening rule
/// ([`widen_one`]) over every destination the parsers report, every
/// `file-path` pointer and every `assets` mention in the text and its decoded
/// views. `into` receives L, the [`live_key`] of every name in L', and each
/// name whose key is empty as written, so a directory entry is live when
/// [`is_live`] says so. O(text bytes): one inline parse per format serving
/// all three answers, skipped unless the text names an `assets` folder, holds
/// a destination with a dot or an escape, or a Hiccup escape; one block parse
/// per format when the text holds `file-path` in any case.
pub fn collect_live_asset_refs(text: &str, into: &mut HashSet<String>) {
    let has = |needle: &str| memchr::memmem::find(text.as_bytes(), needle.as_bytes()).is_some();
    let folder = names_assets_folder(text);
    let linked = may_have_dotted_destination(text);
    let hiccup = has("[:") && has("\\u");
    let property = text.contains(':') && contains_folded(text, &FILE_PATH);
    let escaped = text.contains('%') || (text.contains('<') && text.contains('&'));
    if !(folder || linked || hiccup || property || escaped) {
        return;
    }
    let mut live = HashSet::new();
    let mut wide = HashSet::new();
    if folder || linked || hiccup {
        let mut targets = HashSet::new();
        // The collector's own answer keeps its own prefilter (`assets/`, which
        // implies the owner's).
        let collected = text.contains("assets/").then_some(&mut live);
        collect(
            text,
            collected,
            folder.then_some(&mut targets),
            Some(&mut wide),
        );
        for name in targets {
            if let Some((folder, _)) = name.split_once('/') {
                live.insert(folder.to_owned());
            }
            live.insert(name);
        }
    }
    widen_text(text, property, &mut wide);
    for name in live.iter().chain(&wide) {
        // A name with an empty key (whitespace only) is matched as written
        // ([`is_live`]).
        match live_key(name) {
            key if key.is_empty() => into.insert(name.clone()),
            key => into.insert(key),
        };
    }
    into.extend(live);
}

/// The parse prefilter for the basename rule: whether some destination span
/// (to its closer, whitespace for `](`, or line end) can name a file by its
/// basename: a `{{…}}` or non-web `](…)` one holding a `.` or `%`, or a
/// `[[…]]` one escaped, naming a PDF or read by the owner as a bare Org image
/// (any other is a page). A necessary condition only. A closer is searched
/// for within its line only, and spans never overlap, so O(text bytes).
pub(crate) fn may_have_dotted_destination(text: &str) -> bool {
    fn until<'a>(tail: &'a [u8], close: &[u8]) -> &'a [u8] {
        let line = &tail[..memchr::memchr(b'\n', tail).unwrap_or(tail.len())];
        count!(PREFILTER_SEARCHED, line.len());
        &line[..memchr::memmem::find(line, close).unwrap_or(line.len())]
    }
    fn web(dest: &[u8]) -> bool {
        let scheme = dest.iter().take_while(|c| c.is_ascii_alphabetic()).count();
        scheme > 1
            && dest[scheme..].starts_with(b"://")
            && !dest[..scheme].eq_ignore_ascii_case(b"file")
    }
    let b = text.as_bytes();
    let mut at = 0;
    while let Some(i) = b
        .get(at..)
        .and_then(|rest| memchr::memchr3(b'[', b'{', b']', rest))
    {
        let start = at + i;
        let rest = &b[start..];
        let span = match rest {
            [b'[', b'[', tail @ ..] => Some(until(tail, b"]]")),
            [b'{', b'{', tail @ ..] => Some(until(tail, b"}}")),
            [b']', b'(', b'<', tail @ ..] => Some(until(tail, b">")),
            [b']', b'(', tail @ ..] if !web(tail) => {
                let end = tail
                    .iter()
                    .position(|c| c.is_ascii_whitespace())
                    .unwrap_or(tail.len());
                Some(&tail[..end])
            }
            _ => None,
        };
        // A `[[…]]` destination is a page unless the owner reads it as a
        // bare Org image, or it names a PDF.
        let named = |span: &[u8]| {
            if rest.starts_with(b"[[") {
                let dest = String::from_utf8_lossy(until(span, b"]["));
                return dest.contains('%')
                    || crate::local_target::is_pdf(before_suffix(&dest))
                    || crate::local_target::org_bare_image(&dest);
            }
            span.iter().any(|&c| c == b'.' || c == b'%')
        };
        match span {
            Some(span) if named(span) => return true,
            Some(span) => at = start + 2 + span.len(),
            None => at = start + 1,
        }
    }
    false
}

/// The key liveness compares a directory entry by: the file-name fold
/// ([`crate::refs::fold_file_name`]), coarser than any volume's equivalence,
/// so two spellings one volume opens as the same file meet.
pub fn live_key(name: &str) -> String {
    crate::refs::fold_file_name(name)
}

/// Whether directory entry `name` is live in a set [`collect_live_asset_refs`]
/// filled, asked through `holds`: the set holds the name as written (develop's
/// exact match) or its non-empty [`live_key`]. Membership only widens: a name
/// whose key is empty (whitespace only) keeps the exact match. O(name bytes)
/// plus at most two `holds` calls.
pub fn is_live(name: &str, holds: impl Fn(&str) -> bool) -> bool {
    holds(name) || {
        let key = live_key(name);
        !key.is_empty() && holds(&key)
    }
}

/// The owner's attachments alone: exactly the files the owner says `text`'s
/// destinations name (decoded once; the folder itself is no file).
#[cfg(test)]
pub(crate) fn collect_asset_targets(text: &str, into: &mut HashSet<String>) {
    if names_assets_folder(text) {
        collect(text, None, Some(into), None);
    }
}

/// One parse per format serves both answers. `collected`, when given,
/// receives the collector's own answer: parser-accepted targets split after
/// `assets/` under raw and decoded spellings, and the loose plaintext
/// mentions. `targets`, when given, receives the owner's attachments.
/// `widened`, when given, receives W of every destination the parser
/// reports: each link's, each macro argument and its whitespace-separated
/// pieces, and the `assets` mentions of each Hiccup vector's strings
/// when it holds an escape. The callers apply the prefilters.
fn collect(
    text: &str,
    mut collected: Option<&mut HashSet<String>>,
    mut targets: Option<&mut HashSet<String>>,
    mut widened: Option<&mut HashSet<String>>,
) {
    use crate::lsdoc::ast::{Inline, Url};
    fn destinations(nodes: &[Inline], into: &mut HashSet<String>) {
        for node in nodes {
            match node {
                Inline::Link { url, label, .. } => {
                    match url {
                        Url::Search { v } | Url::File { v } | Url::PageRef { v } => {
                            widen_one(v, true, true, into)
                        }
                        Url::Complex { protocol, link } => {
                            let file = protocol
                                .as_deref()
                                .is_some_and(|p| p.eq_ignore_ascii_case("file"));
                            if let Some(link) = link {
                                widen_one(link, file, true, into);
                            }
                        }
                        Url::BlockRef { .. } | Url::EmbedData { .. } => (),
                    }
                    destinations(label, into);
                }
                Inline::Macro { args, .. } => {
                    // The reader joins the arguments with `, ` and splits
                    // the first piece off at ECMAScript's `\s` (U+FEFF
                    // too); the owner splits at Rust's whitespace (U+0085
                    // too).
                    let joined = args.join(", ");
                    for arg in args.iter().map(String::as_str).chain([joined.as_str()]) {
                        widen_one(arg.trim(), true, true, into);
                        widen_one(js_trim(arg), true, true, into);
                        let reader = arg.split(is_js_whitespace).filter(|p| !p.is_empty());
                        for piece in arg.split_whitespace().chain(reader) {
                            widen_one(piece, true, true, into);
                        }
                    }
                }
                // The live reader reads a vector's strings with their
                // escapes (`a`); the EDN reader decodes them alike.
                Inline::Hiccup { v, .. } if v.contains("\\u") => {
                    if let Some(form) = crate::edn::parse(v) {
                        hiccup_strings(&form, into);
                    }
                }
                Inline::Emphasis { children, .. }
                | Inline::Subscript { children, .. }
                | Inline::Superscript { children, .. }
                | Inline::Tag { children, .. } => destinations(children, into),
                Inline::Fnref { definition, .. } => destinations(definition, into),
                _ => (),
            }
        }
    }
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
    // Preambles carry no format argument. Both parsers may conservatively add
    // accepted targets; neither removes a target recognized by the other.
    for format in ["md", "org"] {
        if let Some(nodes) = crate::render::parse_inline_bounded(text, format) {
            if let Some(into) = collected.as_deref_mut() {
                links(&nodes, into);
            }
            if let Some(into) = targets.as_deref_mut() {
                owner(&nodes, format == "md", into);
            }
            if let Some(into) = widened.as_deref_mut() {
                destinations(&nodes, into);
            }
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
/// letters (any case) names none ([`contains_folded`]).
fn names_assets_folder(text: &str) -> bool {
    contains_folded(text, &ASSETS)
}

type Needle = std::sync::LazyLock<memchr::memmem::Finder<'static>>;
static ASSETS: Needle = Needle::new(|| memchr::memmem::Finder::new(b"assets"));
static FILE_PATH: Needle = Needle::new(|| memchr::memmem::Finder::new(b"file-path"));

/// Whether `text` holds the lowercase `needle` in any ASCII case (a
/// necessary condition: a few other bytes fold onto `-`). One vectorized
/// search per window of a folded stack copy: `b | 0x20` is `a` exactly for
/// `a` and `A` (likewise every letter), so the fold finds every case.
/// Windows overlap by the needle's length less one, so no occurrence
/// straddles two. O(text bytes), no allocation.
fn contains_folded(text: &str, finder: &memchr::memmem::Finder<'static>) -> bool {
    const WINDOW: usize = 256;
    const MAX_OVERLAP: usize = 15;
    let overlap = finder.needle().len() - 1;
    debug_assert!(overlap <= MAX_OVERLAP);
    let bytes = text.as_bytes();
    let mut folded = [0u8; WINDOW + MAX_OVERLAP];
    let mut start = 0;
    while start + overlap < bytes.len() {
        let window = &bytes[start..bytes.len().min(start + WINDOW + overlap)];
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
    if let Some(pre) = &doc.pre_block {
        collect_asset_refs(pre, into);
    }
    for block in &doc.roots {
        collect_block_asset_refs(block, into);
    }
}

/// [`collect_live_asset_refs`] over a document's preamble and every block:
/// the names the orphan list and the trash recheck keep alive.
/// Cost O(document text bytes).
pub fn collect_document_live_asset_refs(doc: &Document, into: &mut HashSet<String>) {
    fn block(b: &DocBlock, into: &mut HashSet<String>) {
        collect_live_asset_refs(b.raw(), into);
        for c in &b.children {
            block(c, into);
        }
    }
    if let Some(pre) = &doc.pre_block {
        collect_live_asset_refs(pre, into);
    }
    for b in &doc.roots {
        block(b, into);
    }
}

/// The widening rule W for one written destination `candidate`: every name a
/// reader, opener or exporter can make of it. Spellings: as written and cut
/// before its first `?` or `#`, each raw and, with `decode` (a candidate not
/// already read from a decoded view), decoded once (invalid UTF-8 as U+FFFD)
/// and decoded leniently (invalid UTF-8 escapes kept literal), each cut
/// again. Never decoded twice: no surface does. Each spelling, with `\` and
/// then also `%5C` read as `/`, gives the text after its first `assets/` (any
/// case, inside a segment too), after its first and its last whole `assets`
/// segment, and after the first `assets` segment of the whole path with `..`
/// steps applied (as the openers resolve it); each tail as written, without
/// empty and `.` segments, with `..` steps applied, and each one's first
/// folder. With `basename`, also its last segment, dotted or not (a bare
/// `x.pdf`, a relative image the reader opens in `assets/`, an extensionless
/// file). Over-retaining is safe: an extra name only keeps a file off the
/// unused list. A constant number of tails, so O(candidate bytes).
fn widen_one(candidate: &str, basename: bool, decode: bool, into: &mut HashSet<String>) {
    use std::borrow::Cow;
    fn push<'a>(spellings: &mut Vec<Cow<'a, str>>, s: Cow<'a, str>) {
        if !spellings.iter().any(|known| *known == s) {
            spellings.push(s);
        }
    }
    let mut spellings: Vec<Cow<str>> = Vec::new();
    for whole in [candidate, before_suffix(candidate)] {
        push(&mut spellings, Cow::Borrowed(whole));
        if decode && whole.contains('%') {
            for decoded in [percent_decode(whole), lenient_decode(whole)] {
                push(
                    &mut spellings,
                    Cow::Owned(before_suffix(&decoded).to_owned()),
                );
                push(&mut spellings, Cow::Owned(decoded));
            }
        }
    }
    let mut paths: Vec<Cow<str>> = Vec::new();
    for s in spellings {
        let slashed = match s.contains('\\') {
            true => Cow::Owned(s.replace('\\', "/")),
            false => s,
        };
        if slashed.contains("%5") {
            push(&mut paths, Cow::Owned(replace_escaped_backslash(&slashed)));
        }
        push(&mut paths, slashed);
    }
    for path in &paths {
        let inside = (0..path.len())
            .find(|&i| {
                path.as_bytes()
                    .get(i..i + 7)
                    .is_some_and(|w| w.eq_ignore_ascii_case(b"assets/"))
            })
            .map(|i| i + "assets/".len());
        let mut segments = path
            .split('/')
            .scan(0, |at, part| {
                let start = *at;
                *at += part.len() + 1;
                Some((start, part))
            })
            .filter(|(_, part)| part.eq_ignore_ascii_case("assets"))
            .map(|(start, _)| start + "assets/".len());
        let first = segments.next();
        let last = segments.last();
        for start in [inside, first, last].into_iter().flatten() {
            if let Some(tail) = path.get(start..) {
                insert_tail(tail, into);
            }
        }
        // The openers apply `..` to the whole path before they look for the
        // graph's `assets` folder (`assets/../assets/x` opens `assets/x`).
        let mut resolved = Vec::new();
        crate::local_target::push_segments(&mut resolved, path, true);
        let graph_assets = resolved
            .iter()
            .position(|s| s.eq_ignore_ascii_case("assets"));
        if let Some(at) = graph_assets {
            insert_tail(&resolved[at + 1..].join("/"), into);
        }
        if basename {
            if let Some(last) = path.split('/').rev().find(|part| !part.is_empty()) {
                if last != "." && last != ".." {
                    insert(last, into);
                }
            }
        }
    }
}

/// Insert a non-empty name, allocating only when it is new.
fn insert(name: &str, into: &mut HashSet<String>) {
    if !name.is_empty() && !into.contains(name) {
        into.insert(name.to_owned());
    }
}

/// A name after an `assets` folder: as written, without empty and `.`
/// segments, with `..` steps applied, and the first folder of each.
fn insert_tail(tail: &str, into: &mut HashSet<String>) {
    insert(tail, into);
    if let Some((first, _)) = tail.split_once('/') {
        insert(first, into);
    }
    for parents in [false, true] {
        let mut segments = Vec::new();
        crate::local_target::push_segments(&mut segments, tail, parents);
        match segments.as_slice() {
            [] => (),
            [only] => insert(only, into),
            [first, ..] => {
                insert(first, into);
                insert(&segments.join("/"), into);
            }
        }
    }
}

/// The text before the first `?` or `#`.
fn before_suffix(s: &str) -> &str {
    s.find(['?', '#']).map_or(s, |i| &s[..i])
}

/// `%5C` (any case) read as `/`.
fn replace_escaped_backslash(s: &str) -> String {
    s.replace("%5C", "/").replace("%5c", "/")
}

/// W over the text itself: every `assets` mention in the text and in its
/// decoded views (percent-decoded both ways; the attribute values the HTML
/// sanitizer keeps, entities decoded), and with `property`, every `file-path`
/// pointer value as a destination. O(text bytes).
fn widen_text(text: &str, property: bool, into: &mut HashSet<String>) {
    mentions(text, true, into);
    if text.contains('%') {
        for view in [percent_decode(text), lenient_decode(text)] {
            if view != text {
                mentions(&view, false, into);
            }
        }
    }
    if text.contains('<') && text.contains('&') {
        mentions(&sanitized_view(text), true, into);
    }
    // The PDF pane reads an `hls__` page's `file-path` pointer through the
    // parser's property values (Markdown properties, Org drawers and
    // directives, any key case) and opens its basename when it names no
    // `assets` folder. Every such property in either format is a pointer.
    if property {
        for org in [false, true] {
            count!(PROPERTY_PARSES, 1);
            let regions = crate::block_regions::parse_document(text, org);
            for p in &regions.properties {
                if p.key.eq_ignore_ascii_case("file-path") {
                    widen_one(p.value.trim(), true, true, into);
                }
            }
        }
    }
}

/// The `assets` mentions of every string in a Hiccup vector. O(form size).
fn hiccup_strings(form: &crate::edn::Edn, into: &mut HashSet<String>) {
    use crate::edn::Edn;
    match form {
        Edn::Str(text) => mentions(text, true, into),
        Edn::Vec(items) | Edn::Set(items) | Edn::List(items) => {
            items.iter().for_each(|item| hiccup_strings(item, into))
        }
        Edn::Map(pairs) => pairs.iter().for_each(|(key, value)| {
            hiccup_strings(key, into);
            hiccup_strings(value, into);
        }),
        Edn::Tagged(_, inner) => hiccup_strings(inner, into),
        _ => (),
    }
}

/// The HTML sanitizer's output with the entities it writes into attribute
/// values decoded back: the values a browser loads, as the sanitizer read them.
fn sanitized_view(html: &str) -> String {
    crate::html_sanitize::sanitize(html)
        .replace("&quot;", "\"")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

/// Every `assets` folder mention (any case, then `/`, `\`, `%2F` or `%5C`):
/// the mention to its end at the first of whitespace and `()[]{}<>"'|,`, to
/// the first whitespace, to the first of `"'<>` and line end, and to line
/// end. The three wider ends are taken only for the first mention past the
/// previous one of the same kind, and the scan resumes after the narrowest
/// end, so each byte is read a bounded number of times. O(text bytes).
fn mentions(text: &str, decode: bool, into: &mut HashSet<String>) {
    let b = text.as_bytes();
    let narrow = |c: char| c.is_whitespace() || "()[]{}<>\"'|,".contains(c);
    let wider: [fn(char) -> bool; 3] = [
        char::is_whitespace,
        |c| "\"'<>\n\r".contains(c),
        |c| c == '\n' || c == '\r',
    ];
    let mut reached = [0usize; 3];
    let mut at = 0;
    while let Some(found) = find_assets(b, at) {
        let sep = match b.get(found + 6..) {
            Some([b'/' | b'\\', ..]) => 1,
            Some([b'%', b'2', b'F' | b'f', ..]) | Some([b'%', b'5', b'C' | b'c', ..]) => 3,
            _ => {
                at = found + 6;
                continue;
            }
        };
        let name = found + 6 + sep;
        let end_at =
            |stop: &dyn Fn(char) -> bool| text[name..].find(stop).map_or(text.len(), |i| name + i);
        let end = end_at(&narrow);
        let mut candidates = vec![&text[found..end]];
        for (stop, reached) in wider.iter().zip(&mut reached) {
            if found >= *reached {
                let end = end_at(stop);
                *reached = end;
                candidates.push(text[found..end].trim_end());
            }
        }
        candidates.dedup();
        for candidate in candidates {
            widen_one(candidate, false, decode, into);
        }
        at = end.max(name);
    }
}

/// The next `assets` at or after `from`, in any ASCII case. O(bytes).
fn find_assets(b: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(i) = memchr::memchr2(b'a', b'A', b.get(at..)?) {
        let start = at + i;
        if b.get(start..start + 6)
            .is_some_and(|w| w.eq_ignore_ascii_case(b"assets"))
        {
            return Some(start);
        }
        at = start + 1;
    }
    None
}
