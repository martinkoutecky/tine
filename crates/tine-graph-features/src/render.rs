//! HTML byte rendering for print and static publication.

use crate::print::PrintOpts;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use tine_core::doc::{self, DocBlock};
use tine_core::lsdoc::ast::{Block, Inline, Url};
use tine_core::model::{BlockDto, BlockPreview, PageKind, RefGroup};
use tine_core::refs::block_id;
use tine_core::{Corpus, CorpusPage};
use tine_store::{Area, QueryDialect, QueryError, QueryResult, Store, WholeGraph};

#[cfg(test)]
#[path = "render_tests.rs"]
mod review_tests;

pub(crate) struct RenderGraph<'a> {
    pub corpus: &'a Corpus,
    pub whole: &'a WholeGraph,
    pub store: &'a Store,
}

impl RenderGraph<'_> {
    fn publish_preview_block(&self, uuid: &str) -> Option<BlockPreview> {
        fn find<'a>(blocks: &'a [DocBlock], uuid: &str) -> Option<&'a DocBlock> {
            for block in blocks {
                if block.uuid == uuid || block.property("id").as_deref() == Some(uuid) {
                    return Some(block);
                }
                if let Some(found) = find(&block.children, uuid) {
                    return Some(found);
                }
            }
            None
        }
        for page in &self.corpus.pages {
            if let Some(block) = find(&page.document.roots, uuid) {
                let total = subtree_node_count(block);
                let mut remaining_nodes = 10_000;
                let mut remaining_bytes = 8 * 1024 * 1024;
                let blocks = bounded_preview_dto(block, &mut remaining_nodes, &mut remaining_bytes)
                    .into_iter()
                    .collect();
                return Some(BlockPreview {
                    group: RefGroup {
                        page: page.name.clone(),
                        kind: page.kind,
                        blocks,
                        evidence: Vec::new(),
                    },
                    truncated: total.saturating_sub(10_000 - remaining_nodes),
                });
            }
        }
        None
    }

    fn with_pages<R>(&self, f: impl FnOnce(&[(&CorpusPage, &Arc<doc::Document>)]) -> R) -> R {
        let pages: Vec<_> = self
            .corpus
            .pages
            .iter()
            .map(|page| (page, &page.document))
            .collect();
        f(&pages)
    }

    fn list_pages(&self) -> Vec<&CorpusPage> {
        self.corpus.pages.iter().collect()
    }

    fn read_asset_limited(
        &self,
        name: &str,
        limit: u64,
    ) -> Result<Vec<u8>, tine_store::StoreError> {
        let id = self.store.file_id(Area::Assets, name)?;
        self.store.read(&id, Some(limit)).map(|(bytes, _)| bytes)
    }

    fn query_bounded(&self, source: &str, advanced: bool) -> BoundedGroups {
        let dialect = if advanced {
            QueryDialect::Advanced
        } else {
            QueryDialect::Simple
        };
        match self.whole.query(source, dialect, None) {
            Ok(QueryResult::Simple(groups)) => BoundedGroups {
                total: groups.iter().map(|group| group.blocks.len()).sum(),
                groups: groups.as_ref().clone(),
                exceeded: false,
            },
            Ok(QueryResult::Advanced(result)) => BoundedGroups {
                total: result.groups.iter().map(|group| group.blocks.len()).sum(),
                groups: result.groups,
                exceeded: false,
            },
            Err(QueryError::ResultTooLarge { count, .. }) => BoundedGroups {
                groups: Vec::new(),
                total: count,
                exceeded: true,
            },
            Err(_) => BoundedGroups {
                groups: Vec::new(),
                total: 0,
                exceeded: false,
            },
        }
    }
}

fn subtree_node_count(root: &DocBlock) -> usize {
    let mut count = 0usize;
    let mut stack = vec![root];
    while let Some(block) = stack.pop() {
        count = count.saturating_add(1);
        stack.extend(block.children.iter());
    }
    count
}

fn bounded_preview_dto(
    block: &DocBlock,
    remaining_nodes: &mut usize,
    remaining_bytes: &mut usize,
) -> Option<BlockDto> {
    if *remaining_nodes == 0 {
        return None;
    }
    let minimum_bytes = block
        .raw()
        .len()
        .saturating_add(if block.uuid.is_empty() {
            36
        } else {
            block.uuid.len()
        })
        .saturating_add(128);
    if minimum_bytes > *remaining_bytes {
        return None;
    }
    let mut dto = tine_core::projection::block_to_shallow_dto(block);
    let dto_bytes = tine_core::model::block_dto_estimated_bytes(&dto);
    if dto_bytes > *remaining_bytes {
        return None;
    }
    *remaining_nodes -= 1;
    *remaining_bytes -= dto_bytes;
    for child in &block.children {
        let Some(child_dto) = bounded_preview_dto(child, remaining_nodes, remaining_bytes) else {
            break;
        };
        dto.children.push(child_dto);
    }
    Some(dto)
}

#[derive(Clone)]
struct BoundedGroups {
    groups: Vec<RefGroup>,
    total: usize,
    exceeded: bool,
}

/// URL/file-safe slug for a page name (links and filenames must match).
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Per-export map from a page's display name (lowercased for case-insensitive
/// lookup, matching Logseq's case-insensitive page identity) to its UNIQUE,
/// GUARANTEED-NONEMPTY output slug. Built once per export (`build_slug_map`) and
/// used as the single source of truth for every filename, cross-page link, and
/// search-index entry — so a link can never diverge from the file it points at.
type SlugMap = std::collections::HashMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum QueryCacheKey {
    Simple(String),
    Advanced(String),
}

impl QueryCacheKey {
    fn source_len(&self) -> usize {
        match self {
            Self::Simple(source) | Self::Advanced(source) => source.len(),
        }
    }
}

const QUERY_CACHE_MAX_ENTRIES: usize = 64;
const QUERY_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
struct QueryCache {
    entries: HashMap<QueryCacheKey, BoundedGroups>,
    bytes: usize,
}

impl QueryCache {
    fn get(&self, key: &QueryCacheKey) -> Option<BoundedGroups> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: QueryCacheKey, groups: BoundedGroups) {
        if self.entries.contains_key(&key) || self.entries.len() >= QUERY_CACHE_MAX_ENTRIES {
            return;
        }
        let bytes = key
            .source_len()
            .saturating_add(tine_core::model::ref_groups_estimated_bytes(&groups.groups))
            .saturating_add(256);
        if bytes > QUERY_CACHE_MAX_BYTES || self.bytes.saturating_add(bytes) > QUERY_CACHE_MAX_BYTES
        {
            return;
        }
        self.bytes += bytes;
        self.entries.insert(key, groups);
    }
}

type SharedQueryCache = RefCell<QueryCache>;

/// FNV-1a 64-bit hash → 8 lowercase hex chars. Deterministic across runs (unlike
/// std's `DefaultHasher`/`RandomState`, which are randomly seeded), so re-exports
/// of the same graph produce identical filenames and diffs stay small.
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (h & 0xffff_ffff) as u32)
}

/// The base slug for a page name, made NONEMPTY: `slug(name)` when it has any
/// ASCII-alnum content, else a stable `page-<hash>` token (a title of only
/// punctuation / non-ASCII would otherwise slug to the empty string).
fn base_slug(name: &str) -> String {
    let s = slug(name);
    if s.is_empty() {
        format!("page-{}", short_hash(name))
    } else {
        s
    }
}

/// Build the per-export name→slug map guaranteeing every page a UNIQUE, NONEMPTY
/// filename. `names` must be in a deterministic order (the caller sorts by name)
/// so the same graph always yields the same assignment. On a collision the loser
/// gets a stable `-<hash>` suffix (hash of its own name, so it's stable across
/// runs — not a mutable counter); a `-<n>` counter is only a last resort if even
/// the hashed slug collides. Returns the map plus the list of `(name, base,
/// chosen)` renames so the caller can warn about them. O(n) over pages.
fn build_slug_map(names: &[&str]) -> (SlugMap, Vec<(String, String, String)>) {
    let mut map = SlugMap::with_capacity(names.len());
    let mut used: HashSet<String> = HashSet::with_capacity(names.len());
    let mut collisions = Vec::new();
    for name in names {
        let base = base_slug(name);
        let mut chosen = base.clone();
        if used.contains(&chosen) {
            chosen = format!("{base}-{}", short_hash(name));
            // Vanishingly unlikely, but stay correct: if the hashed slug also
            // collides, disambiguate with a deterministic counter.
            if used.contains(&chosen) {
                let stem = chosen.clone();
                let mut k = 2u32;
                while used.contains(&chosen) {
                    chosen = format!("{stem}-{k}");
                    k += 1;
                }
            }
            collisions.push((name.to_string(), base, chosen.clone()));
        }
        used.insert(chosen.clone());
        map.insert(name.to_lowercase(), chosen);
    }
    (map, collisions)
}

/// Resolve a page name to its export slug via the per-export map (the single
/// source of truth). Falls back to a raw `slug()` for a name not in the map — a
/// reference to a page that isn't being exported (its link is dead either way),
/// or the single-page print export (`ctx.slugs == None`, no cross-page files).
fn page_slug(ctx: &Ctx, name: &str) -> String {
    ctx.slugs
        .and_then(|m| m.get(&name.to_lowercase()))
        .cloned()
        .unwrap_or_else(|| slug(name))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A print document crosses Rust -> IPC -> WebKit and base64 expands its inputs.
/// Bound both one pathological file and the complete export before any bytes are
/// read. The cumulative input ceiling keeps the resulting HTML below roughly
/// 43 MiB even when many otherwise-valid images are present.
const PRINT_ASSET_MAX_BYTES: u64 = 12 * 1024 * 1024;
const PRINT_ASSETS_TOTAL_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
struct PrintAssetBudget {
    per_asset: u64,
    remaining: u64,
}

impl PrintAssetBudget {
    fn standard() -> Self {
        Self {
            per_asset: PRINT_ASSET_MAX_BYTES,
            remaining: PRINT_ASSETS_TOTAL_MAX_BYTES,
        }
    }
}

/// Read a local `assets/<file>` image referenced by an export `data-asset` path
/// (e.g. `../assets/cat.png`) and return it as a self-contained `data:` URI, for
/// the single-page print/PDF export. Returns `None` for a non-local URL (http/…),
/// no graph, an unreadable/oversized file, an exhausted export budget, or a path
/// that escapes `assets/`. The caller emits an inert omission marker rather than
/// leaving a broken or network-capable image in the privileged export flow.
fn inline_asset_uri(ctx: &Ctx, src: &str) -> Option<String> {
    let graph = ctx.graph?;
    let budget_cell = ctx.print_asset_budget?;
    // Only local asset references; leave remote/data URLs untouched.
    if src.contains("://") || src.starts_with("data:") {
        return None;
    }
    // `read_asset` re-guards against traversal; pass just the file name so a
    // `../assets/x` (or `assets/x`) ref resolves to `<graph>/assets/x`.
    let name = src.rsplit('/').next().unwrap_or(src);
    let mut budget = budget_cell.borrow_mut();
    let admission_limit = budget.per_asset.min(budget.remaining);
    let bytes = graph.read_asset_limited(name, admission_limit).ok()?;
    budget.remaining = budget.remaining.saturating_sub(bytes.len() as u64);
    let mime = match name
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("avif") => "image/avif",
        _ => "application/octet-stream",
    };
    Some(format!("data:{mime};base64,{}", base64_encode(&bytes)))
}

/// Minimal standard base64 (no line wrapping) — used only to inline print-export
/// image assets as `data:` URIs. Dependency-free on purpose (tine-core carries no
/// base64 crate); correctness is covered by `base64_matches_known_vectors`.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Where a block lives — its target page slug + its first content line — keyed by
/// the block's `id::` uuid, built from the exported (public) pages so that
/// `((block refs))` can link to the actual block (`<slug>.html#<uuid>`).
struct RefTarget {
    slug: String,
    text: String,
}
type RefIndex = std::collections::HashMap<String, RefTarget>;

struct Referrer {
    slug: String,
    page: String,
    anchor: String,
    text: String,
}
type ReverseRefIndex = std::collections::HashMap<String, Vec<Referrer>>;

fn collect_block_refs(blocks: &[DocBlock], slug: &str, refs: &mut RefIndex) {
    for b in blocks {
        if let Some(id) = block_id(b.raw()) {
            refs.insert(
                id,
                RefTarget {
                    slug: slug.to_string(),
                    text: ref_target_text(b.raw()),
                },
            );
        }
        collect_block_refs(&b.children, slug, refs);
    }
}

fn collect_reverse_refs(
    blocks: &[DocBlock],
    slug: &str,
    page: &str,
    counter: &mut u32,
    public_targets: &RefIndex,
    reverse: &mut ReverseRefIndex,
) {
    for block in blocks {
        let anchor = block_id(block.raw()).unwrap_or_else(|| {
            let anchor = format!("b{}", *counter);
            *counter += 1;
            anchor
        });
        let mut seen = HashSet::new();
        for target in &block.projection().block_refs {
            if !public_targets.contains_key(target) || !seen.insert(target.as_str()) {
                continue;
            }
            reverse.entry(target.clone()).or_default().push(Referrer {
                slug: slug.to_string(),
                page: page.to_string(),
                anchor: anchor.clone(),
                text: ref_target_text(block.raw()),
            });
        }
        collect_reverse_refs(
            &block.children,
            slug,
            page,
            counter,
            public_targets,
            reverse,
        );
    }
}

/// Append `s` HTML-escaped for an ATTRIBUTE value (`& < > " '`) — matches lsdoc's
/// `esc_attr`, so a re-emitted attribute (asset src, alt) round-trips identically.
fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Encode an opaque block id for the URL-fragment context. The same raw value is
/// HTML-escaped separately when emitted as an `id` attribute; treating these as
/// distinct contexts prevents a user-controlled `id::` from breaking either.
fn fragment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Closed URL policy for navigable content in a static export. External links
/// are limited to ordinary web/contact schemes; graph assets and local export
/// links may use a relative path or fragment. Scheme-like and network-path
/// references do not fall through to the relative case.
fn safe_export_url(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.is_empty()
        || url.bytes().any(|b| b == 0 || b.is_ascii_control())
        || url.starts_with("//")
        || url.starts_with('\\')
        || url.contains('\\')
    {
        return None;
    }
    let first_delimiter = url.find(['/', '?', '#']).unwrap_or(url.len());
    if let Some(colon) = url.find(':').filter(|colon| *colon < first_delimiter) {
        let scheme = &url[..colon];
        if !scheme.bytes().enumerate().all(|(i, b)| {
            if i == 0 {
                b.is_ascii_alphabetic()
            } else {
                b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')
            }
        }) {
            return None;
        }
        return matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "mailto" | "tel"
        )
        .then_some(url);
    }
    Some(url)
}

fn safe_media_url(url: &str) -> Option<&str> {
    let safe = safe_export_url(url)?;
    let lower = safe.to_ascii_lowercase();
    if lower.starts_with("mailto:") || lower.starts_with("tel:") {
        None
    } else {
        Some(safe)
    }
}

/// Reverse lsdoc's HTML escaping for a `data-*` payload we re-emit elsewhere (e.g.
/// `data-tex` → visible KaTeX text, `data-asset` → an `src`). `&amp;` decodes LAST so
/// an escaped `&amp;lt;` becomes `&lt;`, not `<`.
fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The value of attribute `name` in a start-tag's inner text (`a class="x" data-page="y"`).
/// lsdoc always double-quotes and attribute-escapes values, so the value runs to the next
/// `"` (an interior quote is `&quot;`).
fn tag_attr<'a>(inner: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let start = inner.find(&pat)? + pat.len();
    let end = inner[start..].find('"')? + start;
    Some(&inner[start..end])
}

/// True if the tag's `class` attribute (space-separated) contains `cls`.
fn has_class(inner: &str, cls: &str) -> bool {
    tag_attr(inner, "class").is_some_and(|c| c.split_whitespace().any(|x| x == cls))
}

/// Consume `html` from `*i` up to and INCLUDING the next `</tag>`, returning the inner
/// text. For elements whose body lsdoc emits as plain text (block-ref) or leaves empty
/// (math / macro / raw-html / media).
fn take_to_close(html: &str, i: &mut usize, tag: &str) -> String {
    let close = format!("</{tag}>");
    match html[*i..].find(&close) {
        Some(rel) => {
            let body = html[*i..*i + rel].to_string();
            *i += rel + close.len();
            body
        }
        None => {
            let body = html[*i..].to_string();
            *i = html.len();
            body
        }
    }
}

/// Decorate lsdoc's canonical skeleton (`render_html`) for the STATIC export: resolve
/// the `data-*` hooks lsdoc leaves to the consumer. lsdoc owns structure + classes +
/// escaping; the export owns link resolution:
/// - page ref  → `<a class="ref" href="slug.html">name</a>` (brackets dropped)
/// - tag       → `<a class="tag" href="slug.html">#name</a>`
/// - block ref → in-page anchor via `refs` (muted text if the target isn't public)
/// - `data-tex`   → KaTeX `\(..\)` / `\[..\]` delimiters (typeset client-side)
/// - `data-asset` → the asset's path as `src`
/// - `data-lang`  → a `language-X` class for highlight.js
/// - `data-raw`   → the raw HTML, sanitized to the shared allowlist (`html_sanitize`) and emitted live
/// - macros       → EXPANDED when a graph is in context (`ctx.graph`): `query` runs the
///   query engine, `embed` inlines the target block/page, `video` embeds an iframe,
///   `namespace` lists child pages, user macros expand from config; unknown macros
///   render as muted literal `{{name …}}`. With no graph (unit tests of the inline
///   decorator) they drop, as before.
/// Everything else (tags, classes, escaped text, nesting, `td`/`th` `data-align`) passes
/// through verbatim.
///
/// O(n) single pass over the html: lsdoc escapes every text + attribute value, so the only
/// raw `<` in its output opens a tag — a `<`-delimited scan is exact and can't be fooled by
/// content. (`depth` bounds macro-expansion recursion; see `expand_macro`.)
fn decorate(html: &str, ctx: &Ctx, depth: u8) -> String {
    let refs = ctx.refs;
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len() + 64);
    let mut i = 0;
    // After a page-ref open tag, the next text node is the link body: strip a surrounding
    // `[[ ]]` (unlabeled `[[name]]`). A labeled ref's body starts with a tag, so the flag
    // is cleared without stripping.
    let mut strip_brackets = false;
    let mut inert_link_closures = 0usize;
    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            let text = &html[start..i];
            if strip_brackets {
                strip_brackets = false;
                let t = text.trim();
                if let Some(inner) = t.strip_prefix("[[").and_then(|x| x.strip_suffix("]]")) {
                    out.push_str(inner);
                    continue;
                }
            }
            out.push_str(text);
            continue;
        }
        let close = match html[i..].find('>') {
            Some(rel) => i + rel,
            None => {
                out.push_str(&html[i..]); // malformed tail — emit verbatim
                break;
            }
        };
        let inner = &html[i + 1..close];
        i = close + 1;
        // A labeled page-ref body that opened with a tag → not the `[[name]]` form.
        if strip_brackets && !inner.starts_with('/') {
            strip_brackets = false;
        }
        let name = inner.split([' ', '\t', '/']).next().unwrap_or("");

        if inner == "/a" && inert_link_closures > 0 {
            inert_link_closures -= 1;
            out.push_str("</span>");
            continue;
        }

        if name == "a" && has_class(inner, "page-ref") {
            if let Some(page) = tag_attr(inner, "data-page") {
                out.push_str(&format!(
                    "<a class=\"ref\" href=\"{}.html\">",
                    page_slug(ctx, &unescape(page))
                ));
                strip_brackets = true;
                continue;
            }
        }
        if name == "a" && has_class(inner, "tag") {
            if let Some(page) = tag_attr(inner, "data-page") {
                out.push_str(&format!(
                    "<a class=\"tag\" href=\"{}.html\">",
                    page_slug(ctx, &unescape(page))
                ));
                continue;
            }
        }
        if name == "span" && has_class(inner, "block-ref") {
            if let Some(id_esc) = tag_attr(inner, "data-block") {
                let id = unescape(id_esc);
                let body = take_to_close(html, &mut i, "span"); // body is plain text
                let auto = format!("(({}))", id.chars().take(8).collect::<String>());
                match refs.get(&id) {
                    Some(t) => {
                        let text = if body == auto { esc(&t.text) } else { body };
                        out.push_str(&format!(
                            "<a class=\"ref block-ref\" href=\"{}.html#{}\">{}</a>",
                            t.slug,
                            fragment(&id),
                            text
                        ));
                    }
                    None => out.push_str(&format!("<span class=\"block-ref\">{body}</span>")),
                }
                continue;
            }
        }
        if name == "span" && has_class(inner, "math") {
            if let Some(tex_esc) = tag_attr(inner, "data-tex") {
                let _ = take_to_close(html, &mut i, "span"); // empty body
                let tex = esc(&unescape(tex_esc));
                let (l, r, cls) = if has_class(inner, "math-display") {
                    ("\\[", "\\]", "math math-display")
                } else {
                    ("\\(", "\\)", "math")
                };
                out.push_str(&format!("<span class=\"{cls}\">{l}{tex}{r}</span>"));
                continue;
            }
        }
        if name == "span" && has_class(inner, "macro") {
            let _ = take_to_close(html, &mut i, "span"); // empty element; args are in attrs
            let mname = tag_attr(inner, "data-macro")
                .map(unescape)
                .unwrap_or_default();
            let args = macro_args(tag_attr(inner, "data-args"));
            out.push_str(&expand_macro(&mname, &args, ctx, depth));
            continue;
        }
        if name == "span" && has_class(inner, "raw-html") {
            let _ = take_to_close(html, &mut i, "span");
            if let Some(raw_esc) = tag_attr(inner, "data-raw") {
                // Sanitize to the shared allowlist and emit LIVE (mirrors the app's
                // DOMPurify pass — see html_sanitize). Handlers/`style`/`<script>`/
                // `<iframe>` are stripped; the surviving markup is already safe, so it
                // is pushed verbatim (NOT re-escaped).
                out.push_str(&tine_core::html_sanitize::sanitize(&unescape(raw_esc)));
            }
            continue;
        }
        if name == "img" && has_class(inner, "inline-image") {
            if let Some(asset) = tag_attr(inner, "data-asset") {
                let alt = tag_attr(inner, "alt").map(unescape).unwrap_or_default();
                let src = unescape(asset);
                if ctx.inline_assets {
                    if let Some(inlined) = inline_asset_uri(ctx, &src) {
                        out.push_str(&format!(
                            "<img class=\"inline-image\" src=\"{}\" alt=\"{}\">",
                            esc_attr(&inlined),
                            esc_attr(&alt)
                        ));
                    } else {
                        out.push_str(
                            "<span class=\"print-asset-omitted\">[Image omitted from PDF: unavailable or exceeds the print size limit]</span>",
                        );
                    }
                } else if let Some(src) = safe_media_url(&src) {
                    out.push_str(&format!(
                        "<img class=\"inline-image\" src=\"{}\" alt=\"{}\">",
                        esc_attr(&src),
                        esc_attr(&alt)
                    ));
                } else {
                    out.push_str("<span class=\"unsafe-link\">[Unsafe image URL omitted]</span>");
                }
                continue;
            }
        }
        if (name == "video" || name == "audio") && has_class(inner, "media-embed") {
            if let Some(asset) = tag_attr(inner, "data-asset") {
                let _ = take_to_close(html, &mut i, name); // empty element
                let src = unescape(asset);
                if let Some(src) = safe_media_url(&src) {
                    out.push_str(&format!(
                        "<{name} class=\"media-embed\" controls src=\"{}\"></{name}>",
                        esc_attr(src)
                    ));
                } else {
                    out.push_str("<span class=\"unsafe-link\">[Unsafe media URL omitted]</span>");
                }
                continue;
            }
        }
        if name == "a" {
            if let Some(href) = tag_attr(inner, "href").map(unescape) {
                if safe_export_url(&href).is_some() {
                    out.push('<');
                    out.push_str(inner);
                    out.push('>');
                } else {
                    out.push_str("<span class=\"unsafe-link\">");
                    inert_link_closures += 1;
                }
                continue;
            }
        }
        if name == "code" && has_class(inner, "hljs") {
            // data-lang → highlight.js's `language-X` class; body (escaped code) + the
            // `</code>` close pass through as the default text/close-tag.
            let lang = tag_attr(inner, "data-lang")
                .map(unescape)
                .unwrap_or_default();
            if lang.is_empty() {
                out.push_str("<code class=\"hljs\">");
            } else {
                out.push_str(&format!(
                    "<code class=\"hljs language-{}\">",
                    esc_attr(&lang)
                ));
            }
            continue;
        }
        // default: verbatim (incl. `th`/`td` keeping `data-align` for the export CSS)
        out.push('<');
        out.push_str(inner);
        out.push('>');
    }
    out
}

/// The destination string of a link `url` (mirrors the frontend `urlDest`).
fn url_dest(url: &Url) -> String {
    match url {
        Url::PageRef { v }
        | Url::BlockRef { v }
        | Url::Search { v }
        | Url::File { v }
        | Url::EmbedData { v } => v.clone(),
        Url::Complex { protocol, link } => match (protocol, link) {
            (Some(p), Some(l)) => format!("{p}://{l}"),
            (_, l) => l.clone().unwrap_or_default(),
        },
    }
}

/// Flatten an inline run to plain SEARCH text (mirrors lsdoc's `flatten_text` / the
/// frontend `astText`): keep plain/code text, emphasis children, `#tag`, page-ref names,
/// link labels; drop block-ref uuids, timestamps, macros, breaks (noise in an index).
fn flatten_inlines(inlines: &[Inline], out: &mut String) {
    for s in inlines {
        match s {
            Inline::Plain { text, .. }
            | Inline::Code { text, .. }
            | Inline::Verbatim { text, .. } => out.push_str(text),
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. } => flatten_inlines(children, out),
            Inline::Tag { children, .. } => {
                out.push('#');
                flatten_inlines(children, out);
            }
            Inline::Link { url, label, .. } => match url {
                Url::BlockRef { .. } => {} // opaque uuid reads as noise
                _ if label.is_empty() => out.push_str(&url_dest(url)),
                _ => flatten_inlines(label, out),
            },
            Inline::NestedLink { content, .. } => out.push_str(content),
            Inline::Target { text, .. } => out.push_str(text),
            Inline::Entity { unicode, .. } => out.push_str(unicode),
            Inline::Latex { body, .. } => out.push_str(body),
            Inline::Hiccup { v, .. } => out.push_str(v),
            _ => {}
        }
    }
}

fn push_inlines(inlines: &[Inline], out: &mut String) {
    out.push(' ');
    flatten_inlines(inlines, out);
}

fn flatten_list(items: &[tine_core::lsdoc::ast::ListItem], out: &mut String) {
    for it in items {
        if !it.name.is_empty() {
            push_inlines(&it.name, out);
        }
        flatten_blocks(&it.content, out);
        flatten_list(&it.items, out);
    }
}

/// Walk a block tree, accumulating its displayed text (the recursive analogue of
/// `flatten_inlines`). Properties / standalone-planning blocks are filtered by the
/// caller, so they never reach here.
fn flatten_blocks(blocks: &[Block], out: &mut String) {
    for b in blocks {
        match b {
            Block::Paragraph { inline, .. }
            | Block::Bullet { inline, .. }
            | Block::Heading { inline, .. }
            | Block::FootnoteDef { inline, .. } => push_inlines(inline, out),
            Block::List { items, .. } => flatten_list(items, out),
            Block::Src { code, .. } | Block::Example { code, .. } => {
                out.push(' ');
                out.push_str(code);
            }
            Block::Quote { children, .. } | Block::Custom { children, .. } => {
                flatten_blocks(children, out)
            }
            Block::Table { header, rows, .. } => {
                if let Some(h) = header {
                    for cell in h {
                        push_inlines(cell, out);
                    }
                }
                for row in rows {
                    for cell in row {
                        push_inlines(cell, out);
                    }
                }
            }
            Block::DisplayedMath { text, .. } => {
                out.push(' ');
                out.push_str(text);
            }
            Block::LatexEnv { content, .. } => {
                out.push(' ');
                out.push_str(content);
            }
            _ => {}
        }
    }
}

/// Plain text of a block's (already property/planning-filtered) body, off the same
/// lsdoc AST the renderer uses — the unit indexed for search + snippets. Whitespace is
/// collapsed; empty for a structural-only block. NO second markup stripper.
fn ast_plain_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    flatten_blocks(blocks, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse + property/planning-filter one block body the way `render_block` does — the
/// shared front of the render and search-index paths (one lsdoc parse per call).
fn body_blocks(raw: &str) -> Vec<Block> {
    tine_core::doc::strip_planning_lines(tine_core::render::parse_block(raw, false), raw)
        .into_iter()
        .filter(|b| !matches!(b, Block::Properties { .. }))
        .collect()
}

struct BeginQuery {
    title: Option<String>,
    query: String,
}

enum BeginQueryInspection {
    Supported(BeginQuery),
    Unsupported,
}

/// Return the authored payload only when `raw` is exactly one terminated
/// `#+BEGIN_QUERY` container. This mirrors `WHOLE_BEGIN_QUERY` in the frontend:
/// spaces/tabs are accepted around the delimiters, all three line endings are
/// accepted, and neither a prefix nor a suffix may share the block.
fn whole_begin_query_payload(raw: &str) -> Option<&str> {
    const BEGIN: &str = "#+BEGIN_QUERY";
    const END: &str = "#+END_QUERY";

    let bytes = raw.as_bytes();
    let mut begin = 0;
    while matches!(bytes.get(begin), Some(b' ' | b'\t')) {
        begin += 1;
    }
    let begin_end = begin.checked_add(BEGIN.len())?;
    if !raw.get(begin..begin_end)?.eq_ignore_ascii_case(BEGIN) {
        return None;
    }
    let mut payload_start = begin_end;
    while matches!(bytes.get(payload_start), Some(b' ' | b'\t')) {
        payload_start += 1;
    }
    payload_start += match bytes.get(payload_start) {
        Some(b'\r') if bytes.get(payload_start + 1) == Some(&b'\n') => 2,
        Some(b'\r' | b'\n') => 1,
        _ => return None,
    };

    // JavaScript's terminal `$` accepts one final line ending. Account for it
    // before locating the closing-delimiter line.
    let mut closing_end = raw.len();
    if raw[..closing_end].ends_with("\r\n") {
        closing_end -= 2;
    } else if matches!(bytes.get(closing_end.wrapping_sub(1)), Some(b'\r' | b'\n')) {
        closing_end -= 1;
    }
    while closing_end > payload_start && matches!(bytes[closing_end - 1], b' ' | b'\t') {
        closing_end -= 1;
    }

    let mut newline_start = closing_end;
    while newline_start > payload_start && !matches!(bytes[newline_start - 1], b'\r' | b'\n') {
        newline_start -= 1;
    }
    if newline_start == payload_start {
        return None;
    }
    let closing_line_start = newline_start;
    newline_start -= 1;
    if bytes[newline_start] == b'\n'
        && newline_start > payload_start
        && bytes[newline_start - 1] == b'\r'
    {
        newline_start -= 1;
    }
    let mut delimiter_start = closing_line_start;
    while delimiter_start < closing_end && matches!(bytes[delimiter_start], b' ' | b'\t') {
        delimiter_start += 1;
    }
    if !raw
        .get(delimiter_start..closing_end)?
        .eq_ignore_ascii_case(END)
    {
        return None;
    }
    Some(&raw[payload_start..newline_start])
}

fn skip_edn_trivia(source: &str, mut from: usize) -> usize {
    while from < source.len() {
        let c = source[from..].chars().next().expect("in bounds");
        if c.is_whitespace() || c == ',' {
            from += c.len_utf8();
        } else if c == ';' {
            from += 1;
            while from < source.len() && !matches!(source.as_bytes()[from], b'\n' | b'\r') {
                from += 1;
            }
        } else {
            break;
        }
    }
    from
}

fn edn_string_end(source: &str, from: usize) -> Option<usize> {
    let mut at = from + 1;
    while at < source.len() {
        let c = source[at..].chars().next()?;
        if c == '\\' {
            at += 1;
            let escaped = source.get(at..)?.chars().next()?;
            at += escaped.len_utf8();
        } else if c == '"' {
            return Some(at + 1);
        } else {
            at += c.len_utf8();
        }
    }
    None
}

fn edn_balanced_end(source: &str, from: usize) -> Option<usize> {
    fn closer(c: char) -> Option<char> {
        match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            _ => None,
        }
    }

    let first = source.get(from..)?.chars().next()?;
    let mut stack = vec![closer(first)?];
    let mut at = from + first.len_utf8();
    while at < source.len() {
        let c = source[at..].chars().next()?;
        if c == '"' {
            at = edn_string_end(source, at)?;
            continue;
        }
        if c == ';' {
            while at < source.len() && !matches!(source.as_bytes()[at], b'\n' | b'\r') {
                at += 1;
            }
            continue;
        }
        if let Some(close) = closer(c) {
            stack.push(close);
        } else if matches!(c, ')' | ']' | '}') {
            if stack.pop() != Some(c) {
                return None;
            }
            if stack.is_empty() {
                return Some(at + c.len_utf8());
            }
        }
        at += c.len_utf8();
    }
    None
}

fn edn_token_end(source: &str, mut from: usize) -> usize {
    while from < source.len() {
        let c = source[from..].chars().next().expect("in bounds");
        if c.is_whitespace() || c == ',' || matches!(c, '(' | ')' | '[' | ']' | '{' | '}') {
            break;
        }
        from += c.len_utf8();
    }
    from
}

fn edn_value_end(source: &str, from: usize) -> Option<usize> {
    match source.get(from..)?.chars().next()? {
        '"' => edn_string_end(source, from),
        '(' | '[' | '{' => edn_balanced_end(source, from),
        _ => {
            let end = edn_token_end(source, from);
            (end > from).then_some(end)
        }
    }
}

fn unquote_begin_query_title(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some('\n' | '\r') | None => out.push(c),
                Some(_) => out.push(chars.next().expect("peeked character")),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn has_edn_keyword(source: &str, keyword: &str) -> bool {
    source.match_indices(keyword).any(|(start, _)| {
        source[start + keyword.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}

fn parse_begin_query_map(payload: &str) -> Option<BeginQuery> {
    let source = payload.trim();
    if !source.starts_with('{') {
        return None;
    }
    let map_end = edn_balanced_end(source, 0)?;
    if skip_edn_trivia(source, map_end) != source.len() {
        return None;
    }

    let mut title = None;
    let mut query = None;
    let mut at = 1;
    while at < map_end - 1 {
        at = skip_edn_trivia(source, at);
        if at >= map_end - 1 {
            break;
        }
        if source.as_bytes()[at] != b':' {
            return None;
        }
        let key_end = edn_token_end(source, at);
        let key = &source[at..key_end];
        at = skip_edn_trivia(source, key_end);
        let value_end = edn_value_end(source, at)?;
        if value_end > map_end - 1 {
            return None;
        }
        let value = &source[at..value_end];
        match key {
            ":query" if query.is_none() => query = Some(value.to_string()),
            ":query" => return None,
            ":title" if title.is_none() && value.starts_with('"') => {
                title = Some(unquote_begin_query_title(&value[1..value.len() - 1]));
            }
            ":title" => return None,
            _ => {}
        }
        at = value_end;
    }

    let query = query?;
    if !query.starts_with('[')
        || !has_edn_keyword(&query, ":find")
        || !has_edn_keyword(&query, ":where")
    {
        return None;
    }
    Some(BeginQuery { title, query })
}

/// Inspect raw authored text and use the parsed AST only as a confirmation that
/// the whole container is the one custom/query node the frontend would dispatch.
/// EDN is always sliced from `raw`; rendered/flattened AST text is never rebuilt.
fn inspect_begin_query(raw: &str, blocks: &[Block]) -> Option<BeginQueryInspection> {
    let payload = whole_begin_query_payload(raw)?;
    let body = if matches!(
        blocks.first(),
        Some(Block::Bullet { .. } | Block::Heading { .. })
    ) {
        &blocks[1..]
    } else {
        blocks
    };
    if !matches!(body, [Block::Custom { name, .. }] if name.eq_ignore_ascii_case("query")) {
        return Some(BeginQueryInspection::Unsupported);
    }
    Some(match parse_begin_query_map(payload) {
        Some(query) => BeginQueryInspection::Supported(query),
        None => BeginQueryInspection::Unsupported,
    })
}

/// The first visible block's plain text — a `((block ref))`'s shown label when it has none.
fn ref_target_text(raw: &str) -> String {
    let first: Vec<Block> = body_blocks(raw).into_iter().take(1).collect();
    ast_plain_text(&first)
}

/// Render context threaded through `render_block`/`decorate`: the block-ref index
/// (always) and the graph (present in a real export, absent in inline-decorator unit
/// tests — when absent, macros drop instead of expanding).
struct Ctx<'a> {
    refs: &'a RefIndex,
    reverse_refs: Option<&'a ReverseRefIndex>,
    graph: Option<&'a RenderGraph<'a>>,
    /// The per-export unique/nonempty page-name→slug map (whole-graph site
    /// export). `None` for the single-page print export and inline-decorator unit
    /// tests, where there are no cross-page files — `page_slug` then falls back to
    /// a raw `slug()`.
    slugs: Option<&'a SlugMap>,
    /// When true (the single-page PDF/print export), rewrite each `data-asset`
    /// image `src` to a self-contained `data:` URI by reading the asset bytes,
    /// so the printed document needs no sibling `assets/` folder. The whole-graph
    /// site export keeps the relative `../assets/<file>` links (`false`).
    inline_assets: bool,
    /// Shared admission state for every image in one print document. Present
    /// exactly when `inline_assets` is true; all images therefore consume one
    /// cumulative byte ceiling before base64/IPC/DOM amplification.
    print_asset_budget: Option<&'a RefCell<PrintAssetBudget>>,
    /// Export-local `{{query}}` memo. Whole-graph publish sets this so repeated
    /// macros do one graph scan per distinct source; print export/tests leave it
    /// `None` and keep the old direct call path.
    query_cache: Option<&'a SharedQueryCache>,
    /// Public page documents keyed by Logseq page identity. Page embeds use
    /// this projection; print looks up absent embeds in the full corpus.
    pages: Option<&'a HashMap<String, &'a doc::Document>>,
}

/// lsdoc render options for a Markdown block body (the canonical skeleton the export decorates).
fn md_opts() -> tine_core::lsdoc::RenderOpts {
    tine_core::lsdoc::RenderOpts {
        format: tine_core::lsdoc::Format::Md,
    }
}

/// Parse `data-args` (lsdoc emits a JSON array of strings, attribute-escaped) into its items.
fn macro_args(attr: Option<&str>) -> Vec<String> {
    attr.map(unescape)
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

/// A task marker's checkbox state, mirroring the app's `taskCheckboxState`
/// (`src/markers.ts`): DONE = checked, CANCELED/CANCELLED = no box, any other
/// marker = an empty box.
fn checkbox_state(marker: &str) -> Option<bool> {
    match marker {
        "DONE" => Some(true),
        "CANCELED" | "CANCELLED" => None,
        _ => Some(false),
    }
}

/// The header-line facet chrome that precedes a block's body text: the task
/// checkbox + marker badge and the `[#A]` priority badge (matches the app's Block header).
fn emit_header_facets(marker: Option<&str>, priority: Option<&str>, out: &mut String) {
    if let Some(m) = marker {
        match checkbox_state(m) {
            Some(true) => out.push_str("<span class=\"task-checkbox checked\"></span>"),
            Some(false) => out.push_str("<span class=\"task-checkbox\"></span>"),
            None => {}
        }
        out.push_str(&format!(
            "<span class=\"task-marker m-{}\">{}</span> ",
            m.to_ascii_lowercase(),
            esc(m)
        ));
    }
    if let Some(p) = priority {
        out.push_str(&format!(
            "<span class=\"priority p-{}\">[#{}]</span> ",
            p.to_ascii_lowercase(),
            esc(p)
        ));
    }
}

/// A block property is chrome we hide from the rendered page (the app hides these too):
/// the block `id::`, the collapsed flag, and any `logseq.*` internal key.
fn is_hidden_prop(key: &str) -> bool {
    key == "id" || key == "collapsed" || key.starts_with("logseq.")
}

/// The trailing facet chrome shown BELOW a block's body: SCHEDULED / DEADLINE
/// planning lines and the block's visible `key:: value` properties.
fn emit_trailer_facets(
    scheduled: Option<&str>,
    deadline: Option<&str>,
    props: &[(String, String)],
    out: &mut String,
) {
    if let Some(s) = scheduled {
        out.push_str(&format!(
            "<div class=\"planning scheduled\"><span class=\"pk\">SCHEDULED:</span> {}</div>",
            esc(s)
        ));
    }
    if let Some(d) = deadline {
        out.push_str(&format!(
            "<div class=\"planning deadline\"><span class=\"pk\">DEADLINE:</span> {}</div>",
            esc(d)
        ));
    }
    let visible: Vec<&(String, String)> =
        props.iter().filter(|(k, _)| !is_hidden_prop(k)).collect();
    if !visible.is_empty() {
        out.push_str("<div class=\"block-props\">");
        for (k, v) in visible {
            out.push_str(&format!(
                "<div class=\"prop\"><span class=\"pk\">{}::</span> <span class=\"pv\">{}</span></div>",
                esc(k),
                esc(v)
            ));
        }
        out.push_str("</div>");
    }
}

/// Render one block's inner: header facets + the decorated body + trailer facets.
/// Shared by the top-level renderer and the embedded/query-result renderers so a
/// task in a query result looks exactly like a task on its own page.
fn emit_block_inner(raw: &str, out: &mut String, ctx: &Ctx, depth: u8) {
    let blk = DocBlock::new(raw);
    out.push_str(if blk.marker() == Some("DONE") {
        "<div class=\"b done\">"
    } else {
        "<div class=\"b\">"
    });
    emit_header_facets(blk.marker(), blk.priority(), out);
    let body = decorate(
        &tine_core::lsdoc::render_html(&body_blocks(raw), &md_opts()),
        ctx,
        depth,
    );
    out.push_str(&body);
    out.push_str("</div>");
    emit_trailer_facets(blk.scheduled(), blk.deadline(), &blk.properties(), out);
}

/// Render a query/embed result block (a `BlockDto` from the query engine) as an
/// `<li>` with its facets + children, at `depth` (bounds recursion).
const MAX_RENDER_TREE_DEPTH: usize = 128;

fn flat_dto_text(root: &BlockDto) -> String {
    let mut text = String::new();
    let mut stack = vec![root];
    while let Some(block) = stack.pop() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&block.raw);
        stack.extend(block.children.iter().rev());
    }
    text
}

fn flat_doc_text(root: &DocBlock) -> String {
    let mut text = String::new();
    let mut stack = vec![root];
    while let Some(block) = stack.pop() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(block.raw());
        stack.extend(block.children.iter().rev());
    }
    text
}

fn render_result_block(dto: &BlockDto, out: &mut String, ctx: &Ctx, depth: u8, tree_depth: usize) {
    out.push_str("<li>");
    if tree_depth >= MAX_RENDER_TREE_DEPTH {
        out.push_str("<pre class=\"outline-flat\">");
        out.push_str(&esc(&flat_dto_text(dto)));
        out.push_str("</pre></li>");
        return;
    }
    emit_block_inner(&dto.raw, out, ctx, depth);
    if !dto.children.is_empty() {
        out.push_str("<ul>");
        for c in &dto.children {
            render_result_block(c, out, ctx, depth, tree_depth + 1);
        }
        out.push_str("</ul>");
    }
    out.push_str("</li>");
}

fn collect_wanted_doc_blocks<'a>(
    blocks: &'a [DocBlock],
    wanted: &std::collections::HashSet<&str>,
    found: &mut std::collections::HashMap<&'a str, &'a DocBlock>,
) {
    let mut stack: Vec<_> = blocks.iter().rev().collect();
    while let Some(block) = stack.pop() {
        if wanted.contains(block.uuid.as_str()) {
            found.insert(block.uuid.as_str(), block);
        }
        // OG can retain a matching descendant below a non-matching child of a
        // retained ancestor. Keep walking so both roots hydrate from source.
        stack.extend(block.children.iter().rev());
    }
}

/// Query DTOs intentionally carry shallow membership rows. Static publishing
/// has the source graph in-process, so hydrate each result subtree directly from
/// its page once instead of shipping/caching overlapping owned DTO trees.
fn render_query_groups(
    graph: &RenderGraph<'_>,
    groups: &[RefGroup],
    out: &mut String,
    ctx: &Ctx,
    depth: u8,
) {
    graph.with_pages(|pages| {
        // One lookup index for the complete query avoids O(pages * groups)
        // source-page scans during static/print export.
        let page_by_key = pages
            .iter()
            .map(|(entry, doc)| ((entry.name.as_str(), entry.kind), doc.as_ref()))
            .collect::<std::collections::HashMap<_, _>>();
        for group in groups {
            let Some(doc) = page_by_key.get(&(group.page.as_str(), group.kind)) else {
                // A result without a source in this exact projection has no
                // publication capability. Never fall back to cached DTO bytes.
                continue;
            };
            let wanted = group
                .blocks
                .iter()
                .map(|block| block.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            let mut found = std::collections::HashMap::with_capacity(wanted.len());
            collect_wanted_doc_blocks(&doc.roots, &wanted, &mut found);
            for block in &group.blocks {
                if let Some(source) = found.get(block.id.as_str()) {
                    render_embedded_block(source, out, ctx, depth, 0);
                }
            }
        }
    });
}

/// Render an embedded page's block (a `DocBlock`) as an `<li>`, mirroring `render_result_block`.
fn render_embedded_block(b: &DocBlock, out: &mut String, ctx: &Ctx, depth: u8, tree_depth: usize) {
    out.push_str("<li>");
    if tree_depth >= MAX_RENDER_TREE_DEPTH {
        out.push_str("<pre class=\"outline-flat\">");
        out.push_str(&esc(&flat_doc_text(b)));
        out.push_str("</pre></li>");
        return;
    }
    emit_block_inner(b.raw(), out, ctx, depth);
    if !b.children.is_empty() {
        out.push_str("<ul>");
        for c in &b.children {
            render_embedded_block(c, out, ctx, depth, tree_depth + 1);
        }
        out.push_str("</ul>");
    }
    out.push_str("</li>");
}

/// Expand one `{{macro …}}`. Bounded by `depth` (a page can embed a block that embeds
/// a page …; a circular embed would otherwise loop). With no graph in context, macros drop.
fn expand_macro(name: &str, args: &[String], ctx: &Ctx, depth: u8) -> String {
    let Some(graph) = ctx.graph else {
        return String::new();
    };
    if depth >= 4 {
        return format!("<span class=\"macro-raw\">{{{{{} …}}}}</span>", esc(name));
    }
    let arg0 = args.first().map(|s| s.as_str()).unwrap_or("").trim();
    match name {
        "query" => render_query(graph, arg0, ctx, depth + 1),
        "embed" => render_embed(graph, arg0, ctx, depth + 1),
        "video" => render_video(arg0),
        "namespace" => render_namespace(graph, arg0, ctx),
        // Unknown / can't-render-statically macro → muted literal (better than a blank).
        _ => format!(
            "<span class=\"macro-raw\">{{{{{} {}}}}}</span>",
            esc(name),
            esc(&args.join(" "))
        ),
    }
}

/// Run a `{{query …}}` against the graph and render its results as a bordered block.
fn render_query(graph: &RenderGraph<'_>, src: &str, ctx: &Ctx, depth: u8) -> String {
    render_query_with_title(graph, src, None, ctx, depth)
}

fn render_query_with_title(
    graph: &RenderGraph<'_>,
    src: &str,
    title: Option<&str>,
    ctx: &Ctx,
    depth: u8,
) -> String {
    if !tine_core::query::query_source_within_limit(src) {
        return format!(
            "<div class=\"query query-too-large\">Query source exceeds the {} KiB publication limit.</div>",
            tine_core::query::QUERY_SOURCE_MAX_BYTES / 1024
        );
    }
    if !tine_core::query::query_nesting_within_limit(src) {
        return "<div class=\"query query-too-large\">Query nesting is too deep to publish safely.</div>".to_string();
    }
    let is_advanced = tine_core::query::is_advanced(src);
    let bounded = if let Some(cache) = ctx.query_cache {
        let key = if is_advanced {
            QueryCacheKey::Advanced(src.to_string())
        } else {
            QueryCacheKey::Simple(src.to_string())
        };
        let cached = cache.borrow().get(&key);
        if let Some(groups) = cached {
            groups
        } else {
            let groups = graph.query_bounded(src, is_advanced);
            cache.borrow_mut().insert(key, groups.clone());
            groups
        }
    } else {
        graph.query_bounded(src, is_advanced)
    };
    if bounded.exceeded {
        return format!(
            "<div class=\"query query-too-large\">Query has {} matches; narrow it before publishing.</div>",
            bounded.total
        );
    }
    // A site export is a projection of the public page set, not an alternate
    // frontend over the live graph. Query execution still reuses the ordinary
    // engine, but results from pages outside the pass-1 public capability must
    // never cross into generated HTML. Print export has no page capability and
    // deliberately retains its existing whole-graph behavior.
    let pre_filter_total: usize = bounded.groups.iter().map(|group| group.blocks.len()).sum();
    let groups: Vec<RefGroup> = bounded
        .groups
        .into_iter()
        .filter(|group| publish_page_allowed(ctx, &group.page))
        .collect();
    let total: usize = groups.iter().map(|g| g.blocks.len()).sum();
    let omitted = pre_filter_total.saturating_sub(total);
    let mut out = format!(
        "<div class=\"query\"><div class=\"query-head\">{} <span class=\"query-count\">{}</span></div>",
        esc(title.unwrap_or("Query")),
        total
    );
    if total == 0 {
        out.push_str("<div class=\"query-empty\">No matching blocks.</div>");
    } else {
        out.push_str("<ul class=\"query-results\">");
        render_query_groups(graph, &groups, &mut out, ctx, depth);
        out.push_str("</ul>");
    }
    if omitted > 0 {
        out.push_str(&format!(
            "<div class=\"query-omitted\">{} result{} on non-public pages omitted.</div>",
            omitted,
            if omitted == 1 { "" } else { "s" }
        ));
    }
    out.push_str("</div>");
    out
}

/// Inline an `{{embed ((uuid))}}` or `{{embed [[Page]]}}`.
fn render_embed(graph: &RenderGraph<'_>, arg: &str, ctx: &Ctx, depth: u8) -> String {
    if let Some(uuid) = arg.strip_prefix("((").and_then(|s| s.strip_suffix("))")) {
        let uuid = uuid.trim();
        if ctx.pages.is_some() && !ctx.refs.contains_key(uuid) {
            return "<div class=\"embed embed-missing\">Embedded content is not public.</div>"
                .into();
        }
        return match graph.publish_preview_block(uuid) {
            Some(preview) if publish_page_allowed(ctx, &preview.group.page) => {
                let mut out = String::from(
                    "<div class=\"embed block-embed single-root\"><ul class=\"embed-outline\">",
                );
                for blk in &preview.group.blocks {
                    render_result_block(blk, &mut out, ctx, depth, 0);
                }
                if preview.truncated > 0 {
                    out.push_str(&format!(
                        "<li class=\"query-truncated\">{} more blocks omitted</li>",
                        preview.truncated
                    ));
                }
                out.push_str("</ul></div>");
                out
            }
            Some(_) => {
                "<div class=\"embed embed-missing\">Embedded content is not public.</div>".into()
            }
            None => "<div class=\"embed embed-missing\">Embedded block not found.</div>".into(),
        };
    }
    if let Some(page) = arg.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        let page = page.trim();
        if let Some(doc) = ctx
            .pages
            .and_then(|pages| pages.get(&tine_core::refs::page_key(page)).copied())
        {
            return render_page_embed_doc(page, doc, ctx, depth);
        }
        if ctx.pages.is_some() {
            return "<div class=\"embed embed-missing\">Embedded content is not public.</div>"
                .into();
        }
        return match load_page_doc(graph, page) {
            Some(doc) => render_page_embed_doc(page, &doc, ctx, depth),
            None => "<div class=\"embed embed-missing\">Embedded page not found.</div>".into(),
        };
    }
    format!(
        "<span class=\"macro-raw\">{{{{embed {}}}}}</span>",
        esc(arg)
    )
}

/// Embed a video: a YouTube/Vimeo URL becomes an iframe; anything else, a link.
fn render_video(url: &str) -> String {
    if let Some(id) = youtube_id(url) {
        return format!(
            "<div class=\"video-embed\"><iframe src=\"https://www.youtube.com/embed/{}\" \
             allowfullscreen loading=\"lazy\" frameborder=\"0\"></iframe></div>",
            esc_attr(&id)
        );
    }
    match safe_media_url(url) {
        Some(url) => format!(
            "<div class=\"video-embed\"><a href=\"{}\">{}</a></div>",
            esc_attr(url),
            esc(url)
        ),
        None => format!("<div class=\"video-embed unsafe-link\">{}</div>", esc(url)),
    }
}

/// Extract a YouTube video id from a watch/short/embed URL, if this is one.
fn youtube_id(url: &str) -> Option<String> {
    let u = url.trim();
    if let Some(rest) = u.split("v=").nth(1) {
        if u.contains("youtube.com") {
            return Some(rest.split(['&', '#']).next().unwrap_or(rest).to_string());
        }
    }
    if let Some(rest) = u.split("youtu.be/").nth(1) {
        return Some(
            rest.split(['?', '&', '#'])
                .next()
                .unwrap_or(rest)
                .to_string(),
        );
    }
    if let Some(rest) = u.split("youtube.com/embed/").nth(1) {
        return Some(
            rest.split(['?', '&', '#'])
                .next()
                .unwrap_or(rest)
                .to_string(),
        );
    }
    None
}

/// List the pages directly under a `{{namespace X}}` prefix as links.
fn render_namespace(graph: &RenderGraph<'_>, ns: &str, ctx: &Ctx) -> String {
    let prefix = format!("{}/", ns.trim());
    let mut children: Vec<String> = graph
        .list_pages()
        .into_iter()
        .map(|e| e.name.clone())
        .filter(|n| n.starts_with(&prefix) && publish_page_allowed(ctx, n))
        .collect();
    children.sort();
    children.dedup();
    if children.is_empty() {
        return format!(
            "<div class=\"namespace-macro\">No pages under {}.</div>",
            esc(ns)
        );
    }
    let mut out = format!(
        "<div class=\"namespace-macro\"><div class=\"ns-head\">{}</div><ul>",
        esc(ns)
    );
    for c in &children {
        out.push_str(&format!(
            "<li><a class=\"ref\" href=\"{}.html\">{}</a></li>",
            page_slug(ctx, c),
            esc(c)
        ));
    }
    out.push_str("</ul></div>");
    out
}

fn publish_page_allowed(ctx: &Ctx, page: &str) -> bool {
    ctx.pages
        .is_none_or(|pages| pages.contains_key(&tine_core::refs::page_key(page)))
}

fn render_page_embed_doc(page: &str, doc: &doc::Document, ctx: &Ctx, depth: u8) -> String {
    let mut out = format!(
        "<div class=\"embed page-embed\"><a class=\"embed-title ref\" href=\"{}.html\">{}</a><ul>",
        page_slug(ctx, page),
        esc(page)
    );
    for b in &doc.roots {
        render_embedded_block(b, &mut out, ctx, depth, 0);
    }
    out.push_str("</ul></div>");
    out
}

/// Find a parsed page by name in the supplied corpus, case-insensitively.
fn load_page_doc(graph: &RenderGraph<'_>, name: &str) -> Option<doc::Document> {
    graph.with_pages(|pages| {
        pages
            .iter()
            .find(|(entry, _)| entry.name.eq_ignore_ascii_case(name))
            .map(|(_, document)| document.as_ref().clone())
    })
}

fn render_block(
    b: &DocBlock,
    out: &mut String,
    ctx: &Ctx,
    slug: &str,
    title: &str,
    counter: &mut u32,
    index: &mut Vec<serde_json::Value>,
    opts: PrintOpts,
    tree_depth: usize,
) {
    if tree_depth >= MAX_RENDER_TREE_DEPTH {
        out.push_str("<li><pre class=\"outline-flat\">");
        out.push_str(&esc(&flat_doc_text(b)));
        out.push_str("</pre></li>");
        return;
    }
    // ONE lsdoc parse → the canonical body skeleton (M3), property/planning-filtered like
    // the app's `bodyBlocks`. No second hand-rolled inline parser (the old `render_inline`).
    let blocks = body_blocks(b.raw());
    // BEGIN_QUERY is a static-site feature. The print context deliberately has
    // no public-page capability (`pages: None`) and retains its prior rendering
    // and whole-graph query behavior.
    let begin_query = ctx
        .pages
        .and_then(|_| inspect_begin_query(b.raw(), &blocks));

    // Every block gets a stable anchor so a search hit can deep-link straight to it: its
    // `id::` uuid when present, else a generated per-page `b{n}` (never collides with a
    // 36-char uuid). Emitting the `<li id>` and the search-index entry in the SAME place
    // keeps the HTML anchor and the index in lock-step.
    let anchor = match block_id(b.raw()) {
        Some(id) => id,
        None => {
            let a = format!("b{}", *counter);
            *counter += 1;
            a
        }
    };
    out.push_str(&format!("<li id=\"{}\">", esc_attr(&anchor)));
    // The container payload is executable/configuration source, not visible
    // page prose. In particular, malformed payload bytes must not be copied to
    // the publication search index after the visible block fails closed.
    let text = if begin_query.is_some() {
        String::new()
    } else {
        ast_plain_text(&blocks)
    };
    if !text.is_empty() {
        index.push(json!({"slug": slug, "title": title, "anchor": anchor, "text": text}));
    }

    // Header facets (task checkbox + marker, priority) → the decorated body (lsdoc's
    // canonical render_html; a `# heading` block is wrapped in `<span class="heading-text
    // h{n}">` by render_html itself, so the export markup matches the app) → trailer facets
    // (SCHEDULED/DEADLINE, block properties). Macros in the body are expanded via `ctx`.
    out.push_str(if b.marker() == Some("DONE") {
        "<div class=\"b done\">"
    } else {
        "<div class=\"b\">"
    });
    emit_header_facets(b.marker(), b.priority(), out);
    match &begin_query {
        Some(BeginQueryInspection::Supported(begin)) => {
            if let Some(graph) = ctx.graph {
                out.push_str(&render_query_with_title(
                    graph,
                    &begin.query,
                    begin.title.as_deref(),
                    ctx,
                    0,
                ));
            } else {
                out.push_str("<div class=\"query-unsupported begin-query-unsupported\" role=\"alert\">Unsupported BEGIN_QUERY.</div>");
            }
        }
        Some(BeginQueryInspection::Unsupported) => out.push_str(
            "<div class=\"query-unsupported begin-query-unsupported\" role=\"alert\">Unsupported BEGIN_QUERY.</div>",
        ),
        None => out.push_str(&decorate(&tine_core::lsdoc::render_html(&blocks, &md_opts()), ctx, 0)),
    }
    out.push_str("</div>");
    emit_trailer_facets(b.scheduled(), b.deadline(), &b.properties(), out);
    if let (Some(id), Some(reverse)) = (block_id(b.raw()), ctx.reverse_refs) {
        if let Some(referrers) = reverse.get(&id).filter(|items| !items.is_empty()) {
            let count = referrers.len();
            out.push_str(&format!(
                "<details class=\"block-referrers\"><summary class=\"ref-count\" aria-label=\"{count} block reference{}\">{count}</summary><ul>",
                if count == 1 { "" } else { "s" }
            ));
            for referrer in referrers {
                let text = if referrer.text.is_empty() {
                    "Referenced block"
                } else {
                    &referrer.text
                };
                out.push_str(&format!(
                    "<li><a href=\"{}.html#{}\"><span class=\"referrer-page\">{}</span>: {}</a></li>",
                    referrer.slug,
                    fragment(&referrer.anchor),
                    esc(&referrer.page),
                    esc(text)
                ));
            }
            out.push_str("</ul></details>");
        }
    }

    // A collapsed block hides its children on screen; the print export expands them
    // by default (a PDF usually wants the whole page), but the dialog can keep them
    // folded to match what's visible.
    if !b.children.is_empty() && (opts.expand_collapsed || !b.collapsed()) {
        out.push_str("<ul>");
        for c in &b.children {
            render_block(
                c,
                out,
                ctx,
                slug,
                title,
                counter,
                index,
                opts,
                tree_depth + 1,
            );
        }
        out.push_str("</ul>");
    }
    out.push_str("</li>");
}

fn page_html(
    title: &str,
    slug: &str,
    doc: &doc::Document,
    kind: PageKind,
    ctx: &Ctx,
    blocks: &mut Vec<serde_json::Value>,
    home_href: &str,
) -> String {
    let mut body = String::new();
    body.push_str("<ul class=\"outline\">");
    let mut counter = 0u32;
    for b in &doc.roots {
        // The whole-graph site export always expands (no fold state on paper).
        render_block(
            b,
            &mut body,
            ctx,
            slug,
            title,
            &mut counter,
            blocks,
            PrintOpts::default(),
            0,
        );
    }
    body.push_str("</ul>");
    // Journal titles get a leading calendar glyph, like Logseq.
    let cal = "<svg class=\"cal\" viewBox=\"0 0 24 24\" width=\"18\" height=\"18\" fill=\"none\" \
stroke=\"currentColor\" stroke-width=\"1.7\"><rect x=\"4\" y=\"5\" width=\"16\" height=\"16\" rx=\"2\"/>\
<line x1=\"4\" y1=\"9.5\" x2=\"20\" y2=\"9.5\"/><line x1=\"8.5\" y1=\"3\" x2=\"8.5\" y2=\"7\"/>\
<line x1=\"15.5\" y1=\"3\" x2=\"15.5\" y2=\"7\"/></svg>";
    let heading = if kind == PageKind::Journal {
        format!("<h1 class=\"page\">{}{}</h1>", cal, esc(title))
    } else {
        format!("<h1 class=\"page\">{}</h1>", esc(title))
    };
    shell(title, &format!("{heading}{body}"), home_href)
}

/// The shared two-column document shell used by every generated page: `<head>` +
/// the persistent sidebar (home link, search box, and a `#tine-pages` list filled
/// by `app.js`) + the page's `<main>` + the export scripts. The sidebar markup is
/// identical on every page; `app.js` reads the embedded `search-index.js` globals
/// (`window.__tinePages` / `__tineBlocks`) — read as `<script>` globals, never
/// `fetch`ed — so navigation and search work offline / opened straight off disk
/// (`file://`, where `fetch` of a sibling file is blocked but `<script src>` is not).
fn shell(title: &str, main: &str, home_href: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; base-uri 'none'; object-src 'none'; form-action 'none'; script-src 'self' https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; img-src 'self' data: https: http:; media-src 'self' blob: https: http:; frame-src https://www.youtube.com https://player.vimeo.com\">\
<title>{title}</title>\
<link rel=\"stylesheet\" href=\"style.css\">{katex}{hljs}</head><body>\
<aside class=\"sidebar\">\
<a class=\"home\" href=\"{home_href}\">\u{2302} Home</a>\
<a class=\"home pages-link\" href=\"pages.html\">All pages</a>\
<input id=\"tine-search\" type=\"search\" placeholder=\"Search\u{2026}\" autocomplete=\"off\" spellcheck=\"false\">\
<div id=\"tine-results\" hidden></div>\
<nav id=\"tine-pages\"></nav>\
</aside><main>{main}</main>\
<script src=\"search-index.js\"></script><script src=\"fuse.min.js\"></script><script src=\"app.js\"></script><script defer src=\"enhance.js\"></script>\
</body></html>",
        title = esc(title),
        katex = KATEX_HEAD,
        hljs = HLJS_HEAD,
        main = main,
        home_href = esc_attr(home_href),
    )
}

/// A **self-contained single page** for the print-to-PDF export: the same block
/// render as a published page, but with the stylesheet + print rules inlined and
/// no sidebar / search / scripts — so the returned document cannot execute in the
/// privileged Tauri origin. Assets are inlined as `data:` URIs upstream
/// (`inline_assets`). The frontend upgrades math/code with its locally bundled
/// KaTeX/highlight.js before placing this static document in a script-disabled
/// sandbox.
// Inter faces bundled INTO the print document as `@font-face` data URIs. The print
// doc is a separate document from the app, so it does NOT inherit the app's Inter
// `@font-face` rules; without this it falls back to a system font, and if that font
// lacks an italic face WebKitGTK *synthesizes* one — which its PDF (Cairo) backend
// renders garbled (emphasis in particular). Embedding real normal/italic/bold faces
// fixes that and makes the PDF self-contained + Inter-faithful. Latin subset only
// (~120 KB); non-latin falls back to the system font, same as before.
const INTER_FACES: &[(&[u8], u32, &str)] = &[
    (
        include_bytes!("../assets/fonts/inter-400-normal.woff2"),
        400,
        "normal",
    ),
    (
        include_bytes!("../assets/fonts/inter-400-italic.woff2"),
        400,
        "italic",
    ),
    (
        include_bytes!("../assets/fonts/inter-600-normal.woff2"),
        600,
        "normal",
    ),
    (
        include_bytes!("../assets/fonts/inter-700-normal.woff2"),
        700,
        "normal",
    ),
    (
        include_bytes!("../assets/fonts/inter-700-italic.woff2"),
        700,
        "italic",
    ),
];

fn print_fontface() -> String {
    let mut css = String::new();
    for (bytes, weight, style) in INTER_FACES {
        css.push_str(&format!(
            "@font-face{{font-family:'Inter';font-weight:{weight};font-style:{style};font-display:swap;\
src:url(data:font/woff2;base64,{}) format('woff2')}}\n",
            base64_encode(bytes),
        ));
    }
    css
}

fn print_shell(title: &str, main: &str, opts: PrintOpts) -> String {
    // Dialog-driven knobs (font size + page margin) are appended AFTER PRINT_STYLE so
    // they win over its `@page`/font defaults. Clamped to sane bounds.
    let font = opts.font_px.clamp(8, 40);
    let margin = opts.margin_mm.clamp(0, 50);
    let tuned = format!("@page{{margin:{margin}mm}}\nbody.print{{font-size:{font}px}}");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title>\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'none'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' data:; media-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'\">\
<style>{fonts}{style}\n{print}\n{tuned}</style></head><body class=\"print\">\
<main>{main}</main></body></html>",
        title = esc(title),
        fonts = print_fontface(),
        style = STYLE,
        print = PRINT_STYLE,
        tuned = tuned,
        main = main,
    )
}

/// Print/PDF-only overrides layered on top of `STYLE`. The published-site `STYLE`
/// lays out a two-column app shell (flex body + sticky sidebar); with no sidebar
/// here we drop the flex row, widen `main` to the page, set page margins, and add
/// print niceties (avoid breaking a block across pages, un-style links to plain
/// text so a printed link isn't a mystery blue word).
const PRINT_STYLE: &str = r#"
@page{margin:16mm 14mm}
/* Force a LIGHT, printable document even if the OS/webview is in dark mode — a PDF
   should never be white-on-black. These come AFTER STYLE in the same <style>, so
   they override STYLE's own prefers-color-scheme:dark block (both the default and
   the dark media query are pinned to the light palette). */
:root{color-scheme:light;
  --bg:#fff;--fg:#1c1d1e;--muted:#8a8f98;--line:#e4e4e8;--accent:#10b981;--link:#0b5cad;--code:#f4f5f7;}
@media (prefers-color-scheme:dark){:root{
  --bg:#fff;--fg:#1c1d1e;--muted:#8a8f98;--line:#e4e4e8;--accent:#10b981;--link:#0b5cad;--code:#f4f5f7;}}
body.print{display:block;background:#fff;color:var(--fg);
  /* WebKitGTK renders Inter's `->` / `--` / `-->` ligatures as arrow/dash glyphs
     that look garbled in the export (the editor disables ligatures for the same
     reason). Keep the literal characters in the PDF. */
  font-variant-ligatures:none;font-feature-settings:"liga" 0,"calt" 0;
  /* Only ever use the REAL embedded Inter faces (normal/italic/bold above) — never
     a synthesized oblique/bold, which WebKitGTK's PDF backend renders garbled. */
  font-synthesis:none;}
body.print main{max-width:none;margin:0;padding:0 4mm 8mm}
body.print h1.page{margin-top:0}
/* No bullet guide-rails on paper: the connecting lines are a screen-navigation
   affordance and misalign against the bullet dots when printed — dots alone read
   cleaner. */
body.print ul.outline ul{border-left:none}
.print-asset-omitted{display:inline-block;color:#777;font-style:italic;margin:.3rem 0}
@media print{
  body.print main{padding:0}
  a.ref,a.tag,a.block-ref{color:inherit;text-decoration:none}
  li,pre,table,.video-embed,img{break-inside:avoid}
  h1,h2,h3,h4,h5,h6,.heading-text{break-after:avoid}
  /* Print the accent colors (task-checkbox fills, callout tints) instead of
     dropping them to grey. */
  *{-webkit-print-color-adjust:exact;print-color-adjust:exact;}
}
"#;

/// Render ONE page to a self-contained HTML document for print-to-PDF. `name` is
/// the page's display name (as shown in the app / `PageEntry.name`). Returns
/// `Ok(None)` if no such page file exists. Block references resolve within this
/// page (in-page `((ref))` → an in-document anchor); a ref to a block on another
/// page degrades to its label / muted text (there's no second file to link to),
/// which is correct for a single-page export.
pub fn page_print_html(
    graph: &RenderGraph<'_>,
    name: &str,
    opts: PrintOpts,
    org_document: Option<&doc::Document>,
) -> io::Result<Option<String>> {
    let Some(entry) = graph.list_pages().into_iter().find(|e| e.name == name) else {
        return Ok(None);
    };
    let parsed = org_document.unwrap_or(entry.document.as_ref());
    let slug = slug(&entry.name);
    let mut refs = RefIndex::new();
    collect_block_refs(&parsed.roots, &slug, &mut refs);
    let print_asset_budget = RefCell::new(PrintAssetBudget::standard());
    let ctx = Ctx {
        refs: &refs,
        reverse_refs: None,
        graph: Some(graph),
        slugs: None,
        inline_assets: true,
        print_asset_budget: Some(&print_asset_budget),
        query_cache: None,
        pages: None,
    };
    // `page_html` builds the heading + outline and wraps it in `shell`; we want the
    // same body but the print shell, so mirror its body build here.
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut body = String::new();
    body.push_str("<ul class=\"outline\">");
    let mut counter = 0u32;
    for b in &parsed.roots {
        render_block(
            b,
            &mut body,
            &ctx,
            &slug,
            &entry.name,
            &mut counter,
            &mut blocks,
            opts,
            0,
        );
    }
    body.push_str("</ul>");
    let heading = format!("<h1 class=\"page\">{}</h1>", esc(&entry.name));
    Ok(Some(print_shell(
        &entry.name,
        &format!("{heading}{body}"),
        opts,
    )))
}

// KaTeX (from CDN) typesets the `\(..\)` / `\[..\]` math the decorator emits from
// lsdoc's `data-tex` hook, client-side in the published pages. mhchem (\ce{…}) must
// register before auto-render runs; `defer` preserves script order, so auto-render's
// onload fires only after katex.min.js and mhchem have executed. Math therefore
// typesets when the page is viewed online; an offline viewer shows the raw TeX.
const KATEX_HEAD: &str = r#"<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/katex.min.css"><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/katex.min.js"></script><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/contrib/mhchem.min.js"></script><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/contrib/auto-render.min.js"></script>"#;

// highlight.js (from CDN) syntax-highlights the `<pre class="code-block"><code
// class="hljs language-X">` blocks lsdoc emits (the export's `data-lang` → `language-X`).
// `highlightAll()` reads the `language-X` class; `defer` + onload runs it after the body
// parses. Offline / no network → plain (already-escaped) code, never broken.
const HLJS_HEAD: &str = r#"<link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11.11.1/build/styles/github.min.css"><script defer src="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11.11.1/build/highlight.min.js"></script>"#;

const ENHANCE_JS: &str = r#"(function () {
  'use strict';
  if (window.renderMathInElement) {
    window.renderMathInElement(document.body, {
      delimiters: [
        {left: '\\[', right: '\\]', display: true},
        {left: '\\(', right: '\\)', display: false}
      ],
      throwOnError: false
    });
  }
  if (window.hljs) window.hljs.highlightAll();
})();
"#;

const STYLE: &str = r#":root{
  --bg:#fff;--fg:#2e2e2e;--muted:#8a8f98;--line:#e9e9ec;--accent:#10b981;--link:#0b6ec9;--code:#f4f5f7;
}
@media (prefers-color-scheme:dark){:root{--bg:#1b1c1d;--fg:#d8dadd;--muted:#7a7f87;--line:#2d2f31;--link:#5aa9ef;--code:#26282a;}}
*{box-sizing:border-box}
body{font-family:'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;
  background:var(--bg);color:var(--fg);margin:0;line-height:1.6;
  -webkit-font-smoothing:antialiased;font-size:16px;display:flex;align-items:flex-start}
main{flex:1 1 0;min-width:0;max-width:740px;margin:0 auto;padding:48px 24px 96px}
/* sidebar */
.sidebar{flex:0 0 260px;width:260px;position:sticky;top:0;align-self:stretch;height:100vh;overflow-y:auto;
  border-right:1px solid var(--line);padding:22px 14px;font-size:.9rem}
.sidebar a.home{display:block;color:var(--fg);font-size:.95rem;font-weight:650;text-decoration:none;margin:0 4px 12px}
.sidebar a.home:hover{color:var(--link)}
#tine-search{width:100%;padding:7px 10px;border:1px solid var(--line);border-radius:7px;background:var(--bg);
  color:var(--fg);font-size:.9rem;outline:none;font-family:inherit}
#tine-search:focus{border-color:var(--link)}
#tine-results{margin-top:10px}
#tine-results .res{display:block;padding:6px 8px;border-radius:6px;text-decoration:none;color:var(--fg)}
#tine-results .res:hover{background:var(--code)}
#tine-results .res-title{display:block;font-weight:650;font-size:.84rem;color:var(--link)}
#tine-results .res-snip{display:block;font-size:.8rem;color:var(--muted);line-height:1.4;margin-top:1px}
#tine-results mark{background:rgba(245,196,66,.38);color:inherit;border-radius:2px;padding:0 1px}
#tine-results .empty{color:var(--muted);font-size:.85rem;padding:6px 8px}
#tine-pages{margin-top:14px}
#tine-pages .sec{margin-bottom:14px}
#tine-pages h3{font-size:.68rem;text-transform:uppercase;letter-spacing:.06em;color:var(--muted);
  margin:0 0 4px 8px;font-weight:700}
#tine-pages ul{list-style:none;margin:0;padding:0}
#tine-pages li{margin:0;position:static}
#tine-pages li::before{display:none}
#tine-pages a{display:block;padding:3px 8px;border-radius:5px;text-decoration:none;color:var(--fg);
  white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
#tine-pages a:hover{background:var(--code)}
#tine-pages a.active{background:var(--code);font-weight:650;color:var(--link)}
@media (max-width:720px){
  body{flex-direction:column}
  .sidebar{position:static;height:auto;width:100%;flex-basis:auto;align-self:auto;
    border-right:none;border-bottom:1px solid var(--line)}
  main{padding:24px 18px 64px}
}
h1.page{font-size:1.9rem;font-weight:700;letter-spacing:-.02em;margin:.4rem 0 1.4rem}
h1.page .cal{color:var(--muted);margin-right:.45rem;vertical-align:-3px;opacity:.7}
ul.outline,ul.outline ul{list-style:none}
ul.outline{padding-left:0;margin:0}
ul.outline ul{padding-left:1.25rem;margin:.1rem 0;border-left:0;position:relative}
/* Put each connector through the child bullet's center. The old border sat on
   the ul box edge, roughly 7px left of every bullet. */
ul.outline ul::before{content:"";position:absolute;left:.45rem;top:0;bottom:0;border-left:1px solid var(--line)}
li{margin:1px 0;position:relative}
li::before{content:"";position:absolute;left:-0.95rem;top:.62em;width:5px;height:5px;border-radius:50%;
  background:var(--muted);opacity:.45}
ul.outline>li::before{display:none}
.b{padding:1px 0}
h1,h2,h3,h4,h5,h6{line-height:1.3;margin:.5rem 0 .2rem;letter-spacing:-.01em}
h2{font-size:1.4rem}h3{font-size:1.18rem}h4{font-size:1.04rem}
/* block-level `# heading` bodies render as `.heading-text.h{n}` spans (lsdoc render_html). */
.heading-text{display:block;font-weight:600;line-height:1.3;letter-spacing:-.01em;margin:.4rem 0 .15rem}
.heading-text.h1{font-size:1.7em}.heading-text.h2{font-size:1.4em}.heading-text.h3{font-size:1.2em}
.heading-text.h4{font-size:1.1em}.heading-text.h5{font-size:1em}.heading-text.h6{font-size:.9em}
a.ref,a.tag{color:var(--link);text-decoration:none}
a.ref:hover,a.tag:hover{text-decoration:underline}
a.block-ref,span.block-ref{background:var(--code);border-radius:4px;padding:0 .28em;font-size:.95em}
a.block-ref{color:var(--link);text-decoration:none}
a.block-ref:hover{text-decoration:underline}
span.block-ref{color:var(--muted)}
.block-referrers{display:inline-block;margin-left:.4rem;vertical-align:middle}
.block-referrers summary.ref-count{display:inline-flex;align-items:center;justify-content:center;min-width:1.45em;height:1.35em;
  padding:0 .35em;border:1px solid var(--line);border-radius:999px;color:var(--muted);font-size:.72em;cursor:pointer;list-style:none}
.block-referrers summary.ref-count::-webkit-details-marker{display:none}
.block-referrers[open]{display:block;margin:.3rem 0 .55rem .2rem}
.block-referrers ul{margin:.3rem 0 0;padding-left:1.2rem;border-left:1px solid var(--line)}
.block-referrers li{font-size:.86em;color:var(--muted)}
.block-referrers a{color:var(--link);text-decoration:none}.block-referrers a:hover{text-decoration:underline}
.referrer-page{font-weight:600}
a.tag{font-size:.92em}
a[href^="http"]{color:var(--link)}
code,.inline-code{background:var(--code);border-radius:4px;padding:.05em .35em;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.9em}
/* fenced code blocks (highlight.js) — not the inline pill */
pre.code-block{background:var(--code);border:1px solid var(--line);border-radius:8px;padding:.7em .9em;overflow:auto;margin:.4rem 0}
pre.code-block code,pre.code-block code.hljs{background:none;border:0;padding:0;font-size:.86em;display:block}
img,.inline-image{max-width:100%;border-radius:6px;margin:.3rem 0}
.inline-image-wrap{display:inline-block;max-width:100%}
.media-embed{max-width:100%;border-radius:6px;margin:.3rem 0}
.math-display{display:block;text-align:center;margin:.5rem 0}
strong{font-weight:650}
/* in-block markdown lists (.b-scoped so they win over the outline ul rules) */
.b ul.md-list,.b ol.md-list{margin:.2rem 0;padding-left:1.3rem;border-left:none}
.b ul.md-list{list-style:disc}.b ol.md-list{list-style:decimal}
.b li.md-list-item{margin:.1rem 0;position:static}
.b li.md-list-item::before{display:none}
.md-list-term{font-weight:650}
.block-checkbox{display:inline-block;width:.95em;height:.95em;border:1.5px solid var(--muted);border-radius:3px;vertical-align:-2px;margin-right:.15em}
.block-checkbox.checked{background:var(--accent);border-color:var(--accent)}
/* task facets: checkbox + marker badge, priority badge (match the app header) */
.task-checkbox{display:inline-block;width:.95em;height:.95em;border:1.5px solid var(--muted);border-radius:3px;vertical-align:-2px;margin-right:.35em;position:relative}
.task-checkbox.checked{background:var(--accent);border-color:var(--accent)}
.task-checkbox.checked::after{content:"";position:absolute;left:.28em;top:.08em;width:.2em;height:.42em;border:solid #fff;border-width:0 .12em .12em 0;transform:rotate(45deg)}
.task-marker{font-size:.68rem;font-weight:700;letter-spacing:.03em;padding:.05em .35em;border-radius:4px;background:var(--code);color:var(--muted);vertical-align:.05em}
.task-marker.m-doing,.task-marker.m-now{color:#c2410c;background:#fff2e8}
.task-marker.m-done{color:var(--accent);background:#e7f7f0}
.task-marker.m-waiting{color:#a16207;background:#fdf6e3}
.task-marker.m-canceled,.task-marker.m-cancelled{color:var(--muted);text-decoration:line-through}
.b.done>.heading-text,.b.done{color:var(--muted)}
.priority{font-size:.72rem;font-weight:700;padding:.02em .3em;border-radius:4px;background:var(--code);color:var(--muted)}
.priority.p-a{color:#b91c1c;background:#fdeaea}.priority.p-b{color:#c2410c;background:#fff2e8}
/* planning (SCHEDULED/DEADLINE) + block properties */
.planning{font-size:.85em;color:var(--muted);margin:.05rem 0}
.planning.deadline .pk{color:#b91c1c}
.planning .pk,.block-props .pk{font-weight:650;letter-spacing:.02em}
.block-props{font-size:.85em;color:var(--muted);margin:.1rem 0;display:flex;flex-wrap:wrap;gap:.1rem .8rem}
.block-props .pv{color:var(--fg)}
/* query results + embeds + video + namespace macro */
.query{border:1px solid var(--line);border-radius:8px;margin:.4rem 0;overflow:hidden}
.query-head{font-size:.72rem;text-transform:uppercase;letter-spacing:.05em;color:var(--muted);background:var(--code);padding:.3em .7em}
.query-count{background:var(--muted);color:var(--bg);border-radius:8px;padding:0 .45em;margin-left:.3em;font-size:.9em}
.query-results{padding:.3rem .7rem}
.query-empty{padding:.5rem .7rem;color:var(--muted);font-size:.9em}
.query-omitted{padding:.35rem .7rem;color:var(--muted);font-size:.82em;border-top:1px solid var(--line)}
.query-unsupported{border:1px solid #b91c1c;border-radius:8px;margin:.4rem 0;padding:.5rem .7rem;color:#b91c1c;background:#fdeaea;font-size:.9em}
.embed{border-left:3px solid var(--line);padding:.1rem 0 .1rem .8rem;margin:.35rem 0}
/* A block embed is already hosted by one outline li. Remove the embedded ul's
   second bullet/connector and the generic embed border, leaving one root marker. */
.block-embed.single-root{border-left:0;padding-left:0}
.block-embed.single-root>ul.embed-outline{padding-left:0;margin:0}
.block-embed.single-root>ul.embed-outline::before,
.block-embed.single-root>ul.embed-outline>li::before{content:none}
.embed-title{display:inline-block;font-size:.78rem;color:var(--muted);margin-bottom:.1rem}
.embed-missing{color:var(--muted);font-style:italic;font-size:.9em}
.video-embed{position:relative;width:100%;max-width:560px;aspect-ratio:16/9;margin:.4rem 0}
.video-embed iframe{position:absolute;inset:0;width:100%;height:100%;border:0;border-radius:8px}
.namespace-macro{margin:.3rem 0}
.namespace-macro .ns-head{font-size:.72rem;text-transform:uppercase;letter-spacing:.05em;color:var(--muted)}
.macro-raw{color:var(--muted);font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.9em}
/* tables (data-align is the export's beyond-OG column alignment) */
table.md-table{border-collapse:collapse;margin:.5rem 0;font-size:.94em}
table.md-table th,table.md-table td{border:1px solid var(--line);padding:.3em .6em;text-align:left}
table.md-table th{background:var(--code);font-weight:650}
table.md-table [data-align="center"]{text-align:center}
table.md-table [data-align="right"]{text-align:right}
/* blockquote + callouts */
blockquote.md-quote{margin:.5rem 0;padding:.2rem 0 .2rem .9rem;border-left:3px solid var(--line);color:var(--muted)}
.callout{margin:.5rem 0;padding:.5rem .8rem;border-radius:8px;border-left:3px solid var(--accent);background:var(--code)}
.callout-title{font-weight:700;font-size:.82rem;text-transform:uppercase;letter-spacing:.04em;color:var(--accent);margin-bottom:.2rem}
/* org timestamps + footnotes */
.org-timestamp{color:var(--muted);font-size:.92em;font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
.org-timestamp.inactive{opacity:.6}
.footnote-def{font-size:.9em;color:var(--muted);margin:.2rem 0}
.footnote-ref{color:var(--link);font-size:.85em}
.index-list li{margin:.15rem 0}
.index-list .k{color:var(--muted);font-size:.8rem;margin-left:.4rem}
.md-hr,hr{border:none;border-top:1px solid var(--line);margin:1.2rem 0}
footer{margin-top:64px;color:var(--muted);font-size:.78rem;border-top:1px solid var(--line);padding-top:12px}
"#;

// Sidebar + search behaviour for the published site. Vanilla JS, no build step; the
// only dependency is the vendored Fuse.js (loaded separately). Reads the embedded
// `window.__tinePages` / `__tineBlocks` globals (never `fetch`ed) so it works offline
// and over `file://`. Fuse is configured to mirror OG's published block search
// (threshold 0.35, block-level content). Search hits deep-link to `slug.html#anchor`.
const APP_JS: &str = r#"(function () {
  'use strict';
  var pages = window.__tinePages || [];
  var blocks = window.__tineBlocks || [];

  function esc(s) {
    return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
  function basename(p) {
    var parts = String(p).split('/');
    return decodeURIComponent(parts[parts.length - 1] || '');
  }
  var here = basename(location.pathname);

  var input = document.getElementById('tine-search');
  var results = document.getElementById('tine-results');
  var nav = document.getElementById('tine-pages');

  // ---- sidebar page list ----
  function section(title, items) {
    if (!items.length) return '';
    var lis = items.map(function (p) {
      var file = p.slug + '.html';
      var cls = basename(file) === here ? ' class="active"' : '';
      return '<li><a href="' + file + '"' + cls + '>' + esc(p.title) + '</a></li>';
    }).join('');
    return '<div class="sec"><h3>' + esc(title) + '</h3><ul>' + lis + '</ul></div>';
  }
  function byTitleAsc(a, b) { return a.title < b.title ? -1 : a.title > b.title ? 1 : 0; }
  function byTitleDesc(a, b) { return a.title < b.title ? 1 : a.title > b.title ? -1 : 0; }
  function renderPages() {
    if (!nav) return;
    var favs = pages.filter(function (p) { return p.favorite; });
    var journals = pages.filter(function (p) { return p.journal; }).slice().sort(byTitleDesc);
    var plain = pages.filter(function (p) { return !p.journal; }).slice().sort(byTitleAsc);
    nav.innerHTML = section('Favorites', favs) + section('Journals', journals) + section('Pages', plain);
  }

  // ---- fuzzy search (Fuse, OG params) ----
  var fuse = window.Fuse ? new window.Fuse(blocks, {
    keys: ['text', 'title'],
    threshold: 0.35,
    ignoreLocation: true,
    minMatchCharLength: 1,
    includeMatches: true
  }) : null;

  function snippet(entry, matches) {
    var text = entry.text || '';
    var at = -1, len = 0;
    if (matches) {
      for (var i = 0; i < matches.length; i++) {
        var m = matches[i];
        if (m.key === 'text' && m.indices && m.indices.length) {
          at = m.indices[0][0];
          len = m.indices[0][1] - at + 1;
          break;
        }
      }
    }
    if (at < 0) {
      return esc(text.slice(0, 100)) + (text.length > 100 ? '…' : '');
    }
    var start = Math.max(0, at - 28);
    var pre = (start > 0 ? '…' : '') + text.slice(start, at);
    var hit = text.slice(at, at + len);
    var rest = at + len;
    var post = text.slice(rest, rest + 52) + (text.length > rest + 52 ? '…' : '');
    return esc(pre) + '<mark>' + esc(hit) + '</mark>' + esc(post);
  }

  function showList() {
    if (results) { results.hidden = true; results.innerHTML = ''; }
    if (nav) nav.hidden = false;
  }
  function run(q) {
    q = (q || '').trim();
    if (!fuse || !q) { showList(); return; }
    var hits = fuse.search(q, { limit: 20 });
    if (!results) return;
    if (!hits.length) {
      results.innerHTML = '<div class="empty">No matches</div>';
    } else {
      results.innerHTML = hits.map(function (h) {
        var e = h.item;
        var href = e.slug + '.html#' + encodeURIComponent(String(e.anchor));
        return '<a class="res" href="' + href + '">' +
          '<span class="res-title">' + esc(e.title) + '</span>' +
          '<span class="res-snip">' + snippet(e, h.matches) + '</span></a>';
      }).join('');
    }
    results.hidden = false;
    if (nav) nav.hidden = true;
  }

  if (input) {
    input.addEventListener('input', function () { run(input.value); });
    input.addEventListener('keydown', function (ev) {
      if (ev.key === 'Escape') { input.value = ''; showList(); input.blur(); }
      else if (ev.key === 'Enter') {
        var first = results && results.querySelector('a.res');
        if (first) { ev.preventDefault(); location.href = first.getAttribute('href'); }
      }
    });
  }

  renderPages();
})();
"#;

/// True if a page's property pre-block marks it `public:: true`.
fn page_is_public(pre_block: Option<&str>) -> bool {
    let Some(pre) = pre_block else { return false };
    pre.lines().any(|l| {
        let t = l.trim();
        t.starts_with("public::") && t["public::".len()..].trim() == "true"
    })
}
/// Emit the complete site's files and return the public page count.
pub fn publish_graph(
    graph: &RenderGraph<'_>,
    all_public: bool,
    favorites: &[String],
    emit: &mut dyn FnMut(&str, &[u8]) -> io::Result<()>,
) -> io::Result<usize> {
    emit("style.css", STYLE.as_bytes())?;
    // Sidebar + fuzzy search are JS-driven: Fuse (vendored, OG's version) + our tiny
    // app.js, both loaded as `<script src>` so they work offline / over file://.
    emit(
        "fuse.min.js",
        include_str!("../assets/fuse.min.js").as_bytes(),
    )?;
    emit("app.js", APP_JS.as_bytes())?;
    emit("enhance.js", ENHANCE_JS.as_bytes())?;
    let favorites: HashSet<&str> = favorites.iter().map(|s| s.as_str()).collect();

    let pages = graph.list_pages();
    // Query/reference DTOs currently identify their source by logical page name.
    // If two physical files claim that identity, a name-only authorization check
    // cannot prove which file produced a result. Fail closed for that identity:
    // publish neither twin rather than let a private twin borrow the public
    // capability. Ordinary unique pages retain the exact one-file capability.
    let mut source_identity_counts: HashMap<String, usize> = HashMap::new();
    for page in &pages {
        *source_identity_counts
            .entry(tine_core::refs::page_key(&page.name))
            .or_default() += 1;
    }
    let mut entries: Vec<_> = pages.iter().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    // Project authorized pages from the caller's corpus. `entries` is sorted by
    // name, so slug assignment is deterministic. Query hydration stays within
    // this public projection.
    let mut public: Vec<(&str, PageKind, Arc<doc::Document>)> = Vec::new();
    for e in entries {
        let mut parsed = e.document.as_ref().clone();
        let is_public = all_public || page_is_public(parsed.pre_block.as_deref());
        tine_core::projection::assign_doc_runtime_ids(&mut parsed.roots, e.id.as_str());
        let parsed = Arc::new(parsed);
        if !is_public {
            continue;
        }
        if source_identity_counts
            .get(&tine_core::refs::page_key(&e.name))
            .copied()
            .unwrap_or(0)
            != 1
        {
            eprintln!(
                "tine export: refusing ambiguous public page identity {:?}; more than one source file claims it",
                e.name
            );
            continue;
        }
        public.push((e.name.as_str(), e.kind, Arc::clone(&parsed)));
    }

    // ONE source of truth: a unique, nonempty name→slug map for the exported set.
    // Every filename, cross-page link, block-ref target, and search-index entry is
    // driven from this map, so a link can never point at a file that a later page
    // overwrote (DS#4). `slug(name)` is never recomputed independently downstream.
    let names: Vec<&str> = public.iter().map(|(n, _, _)| *n).collect();
    let (slugs, collisions) = build_slug_map(&names);
    for (name, base, chosen) in &collisions {
        eprintln!(
            "tine export: page {name:?} slug {base:?} collides with another page; \
             exporting it as {chosen:?}.html instead"
        );
    }
    let slug_of = |name: &str| -> String {
        slugs
            .get(&name.to_lowercase())
            .cloned()
            .unwrap_or_else(|| slug(name))
    };
    let welcome_slug = slugs.get("welcome to tine").cloned();
    let home_file = welcome_slug
        .as_ref()
        .map(|slug| format!("{slug}.html"))
        .unwrap_or_else(|| "index.html".to_string());

    // Build the block-ref index from the public pages, keyed to their final slugs
    // (a `((ref))` only resolves to a block that's actually exported).
    let mut refs = RefIndex::new();
    for (name, _, parsed) in &public {
        collect_block_refs(&parsed.roots, &slug_of(name), &mut refs);
    }

    let page_docs: HashMap<String, &doc::Document> = public
        .iter()
        .map(|(name, _, parsed)| (tine_core::refs::page_key(name), parsed.as_ref()))
        .collect();
    let mut reverse_refs = ReverseRefIndex::new();
    for (name, _, parsed) in &public {
        let mut counter = 0;
        collect_reverse_refs(
            &parsed.roots,
            &slug_of(name),
            name,
            &mut counter,
            &refs,
            &mut reverse_refs,
        );
    }
    let query_cache: SharedQueryCache = RefCell::new(QueryCache::default());

    // Pass 2: render each public page (collecting the per-block search index along
    // the way), accumulate the sidebar page index (`__tinePages`) and the static
    // no-JS all-pages list shown in the index page's <main>.
    let mut index_list = String::new();
    let mut all_blocks: Vec<serde_json::Value> = Vec::new();
    let mut sidebar_pages: Vec<serde_json::Value> = Vec::new();
    let mut welcome_html: Option<String> = None;
    let mut count = 0;
    // The render context: the block-ref index + the graph (so `{{query}}`/`{{embed}}`/
    // `{{namespace}}` macros can resolve against real data at publish time) + the
    // slug map (so cross-page links resolve to the actual written files).
    let ctx = Ctx {
        refs: &refs,
        reverse_refs: Some(&reverse_refs),
        graph: Some(graph),
        slugs: Some(&slugs),
        inline_assets: false,
        print_asset_budget: None,
        query_cache: Some(&query_cache),
        pages: Some(&page_docs),
    };
    for (name, kind, parsed) in &public {
        let slug = slug_of(name);
        let file = format!("{slug}.html");
        let html = page_html(
            name,
            &slug,
            parsed,
            *kind,
            &ctx,
            &mut all_blocks,
            &home_file,
        );
        if name.eq_ignore_ascii_case("Welcome to Tine") {
            welcome_html = Some(html.clone());
        }
        emit(&file, html.as_bytes())?;
        let journal = *kind == PageKind::Journal;
        let tag = if journal {
            "<span class=\"k\">journal</span>"
        } else {
            ""
        };
        index_list.push_str(&format!(
            "<li><a class=\"ref\" href=\"{}\">{}</a>{}</li>",
            file,
            esc(name),
            tag
        ));
        sidebar_pages.push(json!({
            "title": *name,
            "slug": slug,
            "journal": journal,
            "favorite": favorites.contains(*name),
        }));
        count += 1;
    }

    // Embedded search data, read by app.js as `<script>` globals (never fetched, so
    // the site works offline / over file://). External .js ⇒ serde escaping +
    // no `</script>`-in-content break.
    let data = format!(
        "window.__tinePages={};\nwindow.__tineBlocks={};\n",
        serde_json::to_string(&sidebar_pages).unwrap_or_else(|_| "[]".into()),
        serde_json::to_string(&all_blocks).unwrap_or_else(|_| "[]".into()),
    );
    emit("search-index.js", data.as_bytes())?;

    // Keep the alphabetical page list separately discoverable. When the public
    // set contains Welcome to Tine, index.html is that actual rendered page and
    // every persistent Home link targets its slug. Without it, retain the old
    // page-list index as a safe fallback.
    let main = format!(
        "<h1 class=\"page\">Pages</h1><ul class=\"outline index-list\">{}</ul>\
<footer>Published with Tine</footer>",
        index_list
    );
    let pages_html = shell("Pages", &main, &home_file);
    emit("pages.html", pages_html.as_bytes())?;
    let entry_html = welcome_html.unwrap_or(pages_html);
    emit("index.html", entry_html.as_bytes())?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn no_refs() -> RefIndex {
        RefIndex::new()
    }

    #[test]
    fn slugify() {
        assert_eq!(slug("Foo Bar"), "foo-bar");
        assert_eq!(slug("n-fold IP"), "n-fold-ip");
    }

    /// Render a block body the way `render_block` does: one lsdoc parse → canonical
    /// skeleton (`render_html`) → export decoration. The unit the decorator tests drive.
    fn render_body(raw: &str, refs: &RefIndex) -> String {
        // Graph-less context: the inline decorator under test; macros drop (no graph).
        let ctx = Ctx {
            refs,
            reverse_refs: None,
            graph: None,
            slugs: None,
            inline_assets: false,
            print_asset_budget: None,
            query_cache: None,
            pages: None,
        };
        decorate(
            &tine_core::lsdoc::render_html(&body_blocks(raw), &md_opts()),
            &ctx,
            0,
        )
    }
    fn search_text(raw: &str) -> String {
        ast_plain_text(&body_blocks(raw))
    }

    #[test]
    fn inline_html() {
        let h = render_body("see [[Foo Bar]] and **bold** and `x` and #tag", &no_refs());
        // page ref → `.html` link, brackets dropped (the decorator); #tag → page link.
        assert!(
            h.contains("<a class=\"ref\" href=\"foo-bar.html\">Foo Bar</a>"),
            "{h}"
        );
        assert!(h.contains("<strong>bold</strong>"), "{h}");
        assert!(h.contains("class=\"inline-code\">x</code>"), "{h}");
        assert!(
            h.contains("<a class=\"tag\" href=\"tag.html\">#tag</a>"),
            "{h}"
        );
    }

    #[test]
    fn escapes_html() {
        // lsdoc owns text escaping; the decorator never un-escapes body text.
        assert!(render_body("a < b & c", &no_refs()).contains("a &lt; b &amp; c"));
    }

    #[test]
    fn raw_html_is_sanitized_live() {
        // Raw inline/block HTML now renders LIVE in the export, through the shared
        // sanitizer — allowlisted tags survive, handlers/scripts are stripped.
        let ok = render_body("press <kbd>Ctrl</kbd> and <ins>added</ins>", &no_refs());
        assert!(ok.contains("<kbd>Ctrl</kbd>"), "{ok}");
        assert!(ok.contains("<ins>added</ins>"), "{ok}");
        // NB: mldoc only classifies a SELF-CLOSED `<img/>` as raw HTML; a bare
        // `<img>` is Plain in mldoc/OG too (parity, stays literal). Sanitizer strips
        // the handler, keeps the src.
        let bad = render_body(
            r#"<img src="https://e.com/a.png" onerror="steal()"/>"#,
            &no_refs(),
        );
        assert!(bad.contains("https://e.com/a.png"), "{bad}");
        assert!(!bad.contains("onerror"), "{bad}");
        // A paired <script> IS raw HTML to mldoc; the sanitizer drops it.
        assert!(!render_body("<script>steal()</script>", &no_refs()).contains("steal()"));
    }

    #[test]
    fn math_decorates_katex_delimiters() {
        // The decorator turns lsdoc's `data-tex` hook into KaTeX `\(..\)` / `\[..\]`.
        let h = render_body(r"Euler $e^{i\pi}+1=0$ and $$\int_0^1 x\,dx$$", &no_refs());
        assert!(
            h.contains(r#"<span class="math">\(e^{i\pi}+1=0\)</span>"#),
            "{h}"
        );
        assert!(
            h.contains(r#"<span class="math math-display">\[\int_0^1 x\,dx\]</span>"#),
            "{h}"
        );
        // Underscores inside math must NOT become italics.
        assert!(!render_body(r"$a_1 + b_2$", &no_refs()).contains("<em>"));
    }

    #[test]
    fn block_refs_resolve_via_decoration() {
        let mut refs = RefIndex::new();
        refs.insert(
            "5cfb2cc4-2f18-4b6e-b4c0-dcf657179204".into(),
            RefTarget {
                slug: "related-work".into(),
                text: "Related Work section".into(),
            },
        );
        // Labeled block ref → a link to the target block's anchor, showing the label.
        let h = render_body(
            "see [Related Work](((5cfb2cc4-2f18-4b6e-b4c0-dcf657179204)))",
            &refs,
        );
        assert!(
            h.contains(r#"<a class="ref block-ref" href="related-work.html#5cfb2cc4-2f18-4b6e-b4c0-dcf657179204">Related Work</a>"#),
            "{h}"
        );
        // Bare block ref → the target's text, linked.
        let b = render_body("((5cfb2cc4-2f18-4b6e-b4c0-dcf657179204))", &refs);
        assert!(
            b.contains("related-work.html#5cfb2cc4-2f18-4b6e-b4c0-dcf657179204"),
            "{b}"
        );
        assert!(b.contains("Related Work section"), "{b}");
        // Unresolved ref → muted text, no broken link / no stray `))`.
        let u = render_body("[X](((deadbeef-0000-0000-0000-000000000000)))", &refs);
        assert!(u.contains(r#"<span class="block-ref">X</span>"#), "{u}");
        assert!(!u.contains("((deadbeef"), "{u}");
        // A real URL with parentheses is captured whole (no truncation at first ')').
        let w = render_body(
            "[wiki](https://en.wikipedia.org/wiki/Foo_(bar))",
            &no_refs(),
        );
        assert!(
            w.contains(r#"href="https://en.wikipedia.org/wiki/Foo_(bar)""#),
            "{w}"
        );
    }

    #[test]
    fn ordinary_links_and_video_macros_use_the_closed_url_policy() {
        let js = render_body("[click](javascript:alert(1))", &no_refs());
        assert!(!js.contains("href="), "{js}");
        assert!(js.contains("unsafe-link"), "{js}");
        let data = render_body("[click](data:text/html,boom)", &no_refs());
        assert!(!data.contains("href="), "{data}");
        let web = render_body("[safe](https://example.com/x)", &no_refs());
        assert!(web.contains("href=\"https://example.com/x\""), "{web}");
        let local = render_body("[safe](../assets/report.pdf)", &no_refs());
        assert!(local.contains("href=\"../assets/report.pdf\""), "{local}");
        assert!(render_video("javascript:alert(1)").contains("unsafe-link"));
        assert!(!render_video("javascript:alert(1)").contains("href="));
    }

    #[test]
    fn user_block_ids_are_contextualized_for_attributes_and_fragments() {
        let id = "bad\" onmouseover=\"alert(1) #/%";
        let mut refs = RefIndex::new();
        refs.insert(
            id.into(),
            RefTarget {
                slug: "safe".into(),
                text: "target".into(),
            },
        );
        let link = decorate(
            &format!(
                "<span class=\"block-ref\" data-block=\"{}\">label</span>",
                esc_attr(id)
            ),
            &Ctx {
                refs: &refs,
                reverse_refs: None,
                graph: None,
                slugs: None,
                inline_assets: false,
                print_asset_budget: None,
                query_cache: None,
                pages: None,
            },
            0,
        );
        assert!(
            link.contains("#bad%22%20onmouseover%3D%22alert%281%29%20%23%2F%25"),
            "{link}"
        );
        assert!(!link.contains(" onmouseover="), "{link}");
    }

    #[test]
    fn decorates_image_and_code_block() {
        // image: `data-asset` → src; the inline-image skeleton survives.
        let h = render_body("![cat](../assets/cat.png)", &no_refs());
        assert!(
            h.contains(r#"<img class="inline-image" src="../assets/cat.png" alt="cat">"#),
            "{h}"
        );
        // fenced code: data-lang → highlight.js `language-X` class, body escaped (not the
        // old per-line `<div class="b">` that leaked the ``` fences).
        let c = render_body("```rust\nlet x = 1 < 2;\n```", &no_refs());
        assert!(
            c.contains(r#"<pre class="code-block"><code class="hljs language-rust">"#),
            "{c}"
        );
        assert!(c.contains("1 &lt; 2"), "code body escaped: {c}");
        assert!(!c.contains("```"), "no raw fence in output: {c}");
    }

    #[test]
    fn search_text_off_the_ast() {
        // headings, emphasis/code, [[wiki]], [label](url), ![alt](url) → readable text.
        assert_eq!(
            search_text("## Heading **bold** _it_ `c`"),
            "Heading bold it c"
        );
        assert_eq!(
            search_text("see [[Foo Bar]] and [lbl](http://x)"),
            "see Foo Bar and lbl"
        );
        assert_eq!(search_text("img ![cat](cat.png) end"), "img cat end");
        // a bare block ref → dropped from the index (an opaque uuid reads as noise).
        assert_eq!(
            search_text("ref ((5cfb2cc4-2f18-4b6e-b4c0-dcf657179204)) gone"),
            "ref gone"
        );
    }

    #[test]
    fn search_text_drops_props_and_scheduling() {
        // Property / SCHEDULED / DEADLINE lines are chrome, not searchable content.
        let raw = "task **important** [[Page]]\nSCHEDULED: <2026-01-01 Thu>\nid:: 1111\nkey:: val\ncontinued bit";
        assert_eq!(search_text(raw), "task important Page continued bit");
        // structural-only block → empty (won't be indexed)
        assert_eq!(search_text("id:: abc\ncollapsed:: true"), "");
    }
}
