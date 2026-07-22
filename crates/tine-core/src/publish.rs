//! Static HTML export: render the whole graph to a folder of linked HTML pages
//! plus an index. Inline formatting, nested block lists, and `[[page]]` links
//! become real anchors between the generated files.

use crate::doc::{self, DocBlock};
use crate::model::{BlockDto, Graph, PageKind, RefGroup};
use crate::refs::block_id;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use lsdoc::ast::{Block, Inline, Url};
#[cfg(not(target_os = "windows"))]
use same_file::Handle as FileIdentity;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// `same_file::Handle` keeps its Windows handle open without FILE_SHARE_DELETE,
// which makes MoveFileW reject the final stage rename. Keep a separately-opened
// identity handle that does share deletion instead. We first compare it against
// the bound capability while both are open, so an ambient path swap cannot make
// the identity refer to a different directory; the live handle then prevents
// file-ID reuse through the move and supports ReFS's full 128-bit identities.
#[cfg(target_os = "windows")]
#[derive(Debug)]
struct FileIdentity {
    _file: fs::File,
    volume: u64,
    id: [u8; 16],
}

#[cfg(target_os = "windows")]
impl PartialEq for FileIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.volume == other.volume && self.id == other.id
    }
}

#[cfg(target_os = "windows")]
impl Eq for FileIdentity {}

#[cfg(target_os = "windows")]
fn identity_from_file(file: fs::File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let mut information = FILE_ID_INFO::default();
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileIdentity {
        _file: file,
        volume: information.VolumeSerialNumber,
        id: information.FileId.Identifier,
    })
}

#[cfg(not(target_os = "windows"))]
fn identity_from_file(file: fs::File) -> io::Result<FileIdentity> {
    FileIdentity::from_file(file)
}

#[cfg(target_os = "windows")]
fn identity_from_path(path: &Path) -> io::Result<FileIdentity> {
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    identity_from_file(unsafe { fs::File::from_raw_handle(handle) })
}

#[cfg(not(target_os = "windows"))]
fn identity_from_path(path: &Path) -> io::Result<FileIdentity> {
    FileIdentity::from_path(path)
}

/// URL/file-safe slug for a page name (links and filenames must match).
pub fn slug(name: &str) -> String {
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
    entries: HashMap<QueryCacheKey, crate::query::BoundedGroups>,
    bytes: usize,
}

impl QueryCache {
    fn get(&self, key: &QueryCacheKey) -> Option<crate::query::BoundedGroups> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: QueryCacheKey, groups: crate::query::BoundedGroups) {
        if self.entries.contains_key(&key) || self.entries.len() >= QUERY_CACHE_MAX_ENTRIES {
            return;
        }
        let bytes = key
            .source_len()
            .saturating_add(crate::model::ref_groups_estimated_bytes(&groups.groups))
            .saturating_add(256);
        if bytes > QUERY_CACHE_MAX_BYTES
            || self.bytes.saturating_add(bytes) > QUERY_CACHE_MAX_BYTES
        {
            return;
        }
        self.bytes += bytes;
        self.entries.insert(key, groups);
    }
}

type SharedQueryCache = RefCell<QueryCache>;

#[cfg(test)]
mod publish_test_counts {
    use crate::model::Graph;
    use std::cell::Cell;
    use std::path::Path;

    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static QUERY_RUNS: Cell<usize> = const { Cell::new(0) };
        static PAGE_DOC_LOADS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) struct Guard;

    pub(super) fn count_for(_root: &Path) -> Guard {
        ACTIVE.set(true);
        QUERY_RUNS.set(0);
        PAGE_DOC_LOADS.set(0);
        Guard
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.set(false);
        }
    }

    pub(super) fn bump_query_run(_graph: &Graph) {
        if ACTIVE.get() {
            QUERY_RUNS.set(QUERY_RUNS.get() + 1);
        }
    }

    pub(super) fn bump_page_doc_load(_graph: &Graph) {
        if ACTIVE.get() {
            PAGE_DOC_LOADS.set(PAGE_DOC_LOADS.get() + 1);
        }
    }

    pub(super) fn query_runs() -> usize {
        QUERY_RUNS.get()
    }

    pub(super) fn page_doc_loads() -> usize {
        PAGE_DOC_LOADS.get()
    }
}

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
        if let Some(id) = block_id(&b.raw) {
            refs.insert(
                id,
                RefTarget {
                    slug: slug.to_string(),
                    text: ref_target_text(&b.raw),
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
        let anchor = block_id(&block.raw).unwrap_or_else(|| {
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
                text: ref_target_text(&block.raw),
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
                out.push_str(&crate::html_sanitize::sanitize(&unescape(raw_esc)));
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

fn flatten_list(items: &[lsdoc::ast::ListItem], out: &mut String) {
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
    crate::doc::strip_planning_lines(crate::render::parse_block(raw, false), raw)
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
    graph: Option<&'a Graph>,
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
    /// Pass-1 parsed public page documents, keyed by Logseq page identity. Page
    /// embeds use this before falling back to disk for non-public/unseen pages.
    pages: Option<&'a HashMap<String, &'a doc::Document>>,
}

/// lsdoc render options for a Markdown block body (the canonical skeleton the export decorates).
fn md_opts() -> lsdoc::RenderOpts {
    lsdoc::RenderOpts {
        format: lsdoc::Format::Md,
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
        &lsdoc::render_html(&body_blocks(raw), &md_opts()),
        ctx,
        depth,
    );
    out.push_str(&body);
    out.push_str("</div>");
    emit_trailer_facets(blk.scheduled(), blk.deadline(), &blk.properties(), out);
}

/// Render a query/embed result block (a `BlockDto` from the query engine) as an
/// `<li>` with its facets + children, at `depth` (bounds recursion).
fn render_result_block(dto: &BlockDto, out: &mut String, ctx: &Ctx, depth: u8) {
    out.push_str("<li>");
    emit_block_inner(&dto.raw, out, ctx, depth);
    if !dto.children.is_empty() {
        out.push_str("<ul>");
        for c in &dto.children {
            render_result_block(c, out, ctx, depth);
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
    for block in blocks {
        if wanted.contains(block.uuid.as_str()) {
            found.insert(block.uuid.as_str(), block);
        }
        // OG can retain a matching descendant below a non-matching child of a
        // retained ancestor. Keep walking so both roots hydrate from source.
        collect_wanted_doc_blocks(&block.children, wanted, found);
    }
}

/// Query DTOs intentionally carry shallow membership rows. Static publishing
/// has the source graph in-process, so hydrate each result subtree directly from
/// its page once instead of shipping/caching overlapping owned DTO trees.
fn render_query_groups(graph: &Graph, groups: &[RefGroup], out: &mut String, ctx: &Ctx, depth: u8) {
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
                    render_embedded_block(source, out, ctx, depth);
                }
            }
        }
    });
}

/// Render an embedded page's block (a `DocBlock`) as an `<li>`, mirroring `render_result_block`.
fn render_embedded_block(b: &DocBlock, out: &mut String, ctx: &Ctx, depth: u8) {
    out.push_str("<li>");
    emit_block_inner(&b.raw, out, ctx, depth);
    if !b.children.is_empty() {
        out.push_str("<ul>");
        for c in &b.children {
            render_embedded_block(c, out, ctx, depth);
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
fn render_query(graph: &Graph, src: &str, ctx: &Ctx, depth: u8) -> String {
    render_query_with_title(graph, src, None, ctx, depth)
}

fn render_query_with_title(
    graph: &Graph,
    src: &str,
    title: Option<&str>,
    ctx: &Ctx,
    depth: u8,
) -> String {
    const STATIC_QUERY_MAX_ROWS: usize = 20_000;
    const STATIC_QUERY_MAX_BYTES: usize = 32 * 1024 * 1024;
    if !crate::query::query_source_within_limit(src) {
        return format!(
            "<div class=\"query query-too-large\">Query source exceeds the {} KiB publication limit.</div>",
            crate::query::QUERY_SOURCE_MAX_BYTES / 1024
        );
    }
    if !crate::query::query_nesting_within_limit(src) {
        return "<div class=\"query query-too-large\">Query nesting is too deep to publish safely.</div>".to_string();
    }
    let is_advanced = crate::query::is_advanced(src);
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
            #[cfg(test)]
            publish_test_counts::bump_query_run(graph);
            let groups = if is_advanced {
                let (result, exceeded, total) = crate::query::run_advanced_query_bounded(
                    graph,
                    src,
                    None,
                    STATIC_QUERY_MAX_ROWS,
                    STATIC_QUERY_MAX_BYTES,
                );
                crate::query::BoundedGroups {
                    groups: result.groups,
                    total,
                    exceeded,
                }
            } else {
                crate::query::run_query_bounded(
                    graph,
                    src,
                    STATIC_QUERY_MAX_ROWS,
                    STATIC_QUERY_MAX_BYTES,
                )
            };
            cache.borrow_mut().insert(key, groups.clone());
            groups
        }
    } else if is_advanced {
        #[cfg(test)]
        publish_test_counts::bump_query_run(graph);
        let (result, exceeded, total) = crate::query::run_advanced_query_bounded(
            graph,
            src,
            None,
            STATIC_QUERY_MAX_ROWS,
            STATIC_QUERY_MAX_BYTES,
        );
        crate::query::BoundedGroups {
            groups: result.groups,
            total,
            exceeded,
        }
    } else {
        #[cfg(test)]
        publish_test_counts::bump_query_run(graph);
        crate::query::run_query_bounded(graph, src, STATIC_QUERY_MAX_ROWS, STATIC_QUERY_MAX_BYTES)
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
fn render_embed(graph: &Graph, arg: &str, ctx: &Ctx, depth: u8) -> String {
    if let Some(uuid) = arg.strip_prefix("((").and_then(|s| s.strip_suffix("))")) {
        let uuid = uuid.trim();
        if ctx.pages.is_some() && !ctx.refs.contains_key(uuid) {
            return "<div class=\"embed embed-missing\">Embedded content is not public.</div>"
                .into();
        }
        const STATIC_EMBED_BLOCK_LIMIT: usize = 10_000;
        const STATIC_EMBED_BYTE_LIMIT: usize = 8 * 1024 * 1024;
        return match crate::query::preview_block_with_budget(
            graph,
            uuid,
            STATIC_EMBED_BLOCK_LIMIT,
            STATIC_EMBED_BYTE_LIMIT,
        ) {
            Some(preview) if publish_page_allowed(ctx, &preview.group.page) => {
                let mut out = String::from(
                    "<div class=\"embed block-embed single-root\"><ul class=\"embed-outline\">",
                );
                for blk in &preview.group.blocks {
                    render_result_block(blk, &mut out, ctx, depth);
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
            .and_then(|pages| pages.get(&crate::refs::page_key(page)).copied())
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
fn render_namespace(graph: &Graph, ns: &str, ctx: &Ctx) -> String {
    let prefix = format!("{}/", ns.trim());
    let mut children: Vec<String> = graph
        .list_pages()
        .into_iter()
        .map(|e| e.name)
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
        .is_none_or(|pages| pages.contains_key(&crate::refs::page_key(page)))
}

fn render_page_embed_doc(page: &str, doc: &doc::Document, ctx: &Ctx, depth: u8) -> String {
    let mut out = format!(
        "<div class=\"embed page-embed\"><a class=\"embed-title ref\" href=\"{}.html\">{}</a><ul>",
        page_slug(ctx, page),
        esc(page)
    );
    for b in &doc.roots {
        render_embedded_block(b, &mut out, ctx, depth);
    }
    out.push_str("</ul></div>");
    out
}

/// Read + parse a page's file by name (for `{{embed [[Page]]}}`), case-insensitively.
fn load_page_doc(graph: &Graph, name: &str) -> Option<doc::Document> {
    #[cfg(test)]
    publish_test_counts::bump_page_doc_load(graph);
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
) {
    // ONE lsdoc parse → the canonical body skeleton (M3), property/planning-filtered like
    // the app's `bodyBlocks`. No second hand-rolled inline parser (the old `render_inline`).
    let blocks = body_blocks(&b.raw);
    // BEGIN_QUERY is a static-site feature. The print context deliberately has
    // no public-page capability (`pages: None`) and retains its prior rendering
    // and whole-graph query behavior.
    let begin_query = ctx.pages.and_then(|_| inspect_begin_query(&b.raw, &blocks));

    // Every block gets a stable anchor so a search hit can deep-link straight to it: its
    // `id::` uuid when present, else a generated per-page `b{n}` (never collides with a
    // 36-char uuid). Emitting the `<li id>` and the search-index entry in the SAME place
    // keeps the HTML anchor and the index in lock-step.
    let anchor = match block_id(&b.raw) {
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
        None => out.push_str(&decorate(&lsdoc::render_html(&blocks, &md_opts()), ctx, 0)),
    }
    out.push_str("</div>");
    emit_trailer_facets(b.scheduled(), b.deadline(), &b.properties(), out);
    if let (Some(id), Some(reverse)) = (block_id(&b.raw), ctx.reverse_refs) {
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
            render_block(c, out, ctx, slug, title, counter, index, opts);
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
pub fn page_print_html(graph: &Graph, name: &str, opts: PrintOpts) -> io::Result<Option<String>> {
    let Some(entry) = graph.list_pages().into_iter().find(|e| e.name == name) else {
        return Ok(None);
    };
    let content = fs::read_to_string(&entry.path)?;
    let parsed = doc::parse(&content);
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

/// Options for the single-page print/PDF export, chosen in the pre-export dialog.
/// `#[serde(default)]` so a partial object from the frontend fills the rest.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(default)]
pub struct PrintOpts {
    /// Render the children of a `collapsed:: true` block anyway (true = expand the
    /// whole page, the usual PDF want; false = print it folded as on screen).
    pub expand_collapsed: bool,
    /// Base body font size in px.
    pub font_px: u32,
    /// Page margin in mm (all four sides).
    pub margin_mm: u32,
}

impl Default for PrintOpts {
    fn default() -> Self {
        Self {
            expand_collapsed: true,
            font_px: 16,
            margin_mm: 16,
        }
    }
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

struct PublishStage {
    path: PathBuf,
    root: Dir,
    dir: Dir,
    identity: FileIdentity,
}

struct PublicationGraphSnapshot {
    graph: Graph,
    root: PathBuf,
}

impl PublicationGraphSnapshot {
    fn new(pages: Vec<(crate::model::PageEntry, Arc<doc::Document>)>) -> io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let temp = std::env::temp_dir();
        for _ in 0..128 {
            let root = temp.join(format!(
                "tine-publication-snapshot-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    return Ok(Self {
                        graph: Graph::from_page_snapshot(&root, pages),
                        root,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve an immutable publication snapshot root",
        ))
    }
}

impl Drop for PublicationGraphSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct PublishRecovery {
    #[cfg(test)]
    path: PathBuf,
    dir: Dir,
}

#[cfg(target_os = "windows")]
fn dir_identity(dir: &Dir, path: &Path) -> io::Result<FileIdentity> {
    let capability = identity_from_file(dir.try_clone()?.into_std_file())?;
    let share_delete = identity_from_path(path)?;
    if capability != share_delete {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static-publish staging path changed while binding its identity",
        ));
    }
    Ok(share_delete)
}

#[cfg(not(target_os = "windows"))]
fn dir_identity(dir: &Dir, _path: &Path) -> io::Result<FileIdentity> {
    identity_from_file(dir.try_clone()?.into_std_file())
}

fn reserve_publish_stage(graph: &Graph) -> io::Result<PublishStage> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = Dir::open_ambient_dir(&graph.root, ambient_authority())?;
    for _ in 0..128 {
        let name = format!(
            ".tine-publish-stage-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let path = graph.root.join(&name);
        graph.ensure_write_target(&path)?;
        match root.create_dir(&name) {
            Ok(()) => {
                let dir = root.open_dir(&name)?;
                let identity = dir_identity(&dir, &path)?;
                return Ok(PublishStage {
                    path,
                    root,
                    dir,
                    identity,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique static-publish staging directory",
    ))
}

fn write_publish_stage_file(stage: &PublishStage, name: &str, bytes: &[u8]) -> io::Result<()> {
    let relative = Path::new(name);
    if relative.file_name().is_none_or(|value| value != name)
        || relative
            .parent()
            .is_some_and(|parent| parent != Path::new(""))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static-publish output name must be one file",
        ));
    }
    publish_stage_write_race_hook(stage)?;
    // All generation is relative to the directory handle reserved above. A
    // rename plus symlink/junction replacement of the ambient stage pathname
    // therefore cannot redirect an open or truncate outside the graph.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = stage.dir.open_with(relative, &options)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
thread_local! {
    static PUBLISH_STAGE_WRITE_SWAP: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static PUBLISH_RECOVERY_SWAP: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn replace_bound_dir_path(path: &Path, outside: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let displaced = path.with_file_name(format!(
            "{}.displaced",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("bound")
        ));
        fs::rename(path, &displaced)?;
        symlink(outside, path)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, outside);
        Ok(())
    }
}

#[cfg(test)]
fn publish_stage_write_race_hook(stage: &PublishStage) -> io::Result<()> {
    PUBLISH_STAGE_WRITE_SWAP.with(|outside| match outside.borrow_mut().take() {
        Some(outside) => replace_bound_dir_path(&stage.path, &outside),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn publish_stage_write_race_hook(_stage: &PublishStage) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn publish_recovery_race_hook(recovery: &PublishRecovery) -> io::Result<()> {
    PUBLISH_RECOVERY_SWAP.with(|outside| match outside.borrow_mut().take() {
        Some(outside) => replace_bound_dir_path(&recovery.path, &outside),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn publish_recovery_race_hook(_recovery: &PublishRecovery) -> io::Result<()> {
    Ok(())
}

fn reserve_publish_recovery(graph: &Graph, root: &Dir) -> io::Result<PublishRecovery> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let recovery_rel = Path::new("logseq").join(".tine-trash").join("conflicts");
    let recovery = graph.root.join(&recovery_rel);
    graph.ensure_write_target(&recovery)?;
    root.create_dir_all(&recovery_rel)?;
    let recovery_root = root.open_dir(&recovery_rel)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    for _ in 0..128 {
        let name = format!(
            "{stamp}-{}__previous-publish",
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        match recovery_root.create_dir(&name) {
            Ok(()) => {
                return Ok(PublishRecovery {
                    #[cfg(test)]
                    path: recovery.join(&name),
                    dir: recovery_root.open_dir(&name)?,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve static-publish recovery directory",
    ))
}

fn commit_publish_stage(graph: &Graph, stage: PublishStage, out: &Path) -> io::Result<()> {
    graph.ensure_write_target(out)?;
    // cap-std may represent a directory capability with an O_PATH descriptor on
    // Linux, which cannot itself be fsynced. Every generated file is fsynced;
    // directory durability remains best-effort, matching the other atomic paths.
    let _ = stage.dir.try_clone()?.into_std_file().sync_all();
    let PublishStage {
        path,
        root,
        dir,
        identity,
    } = stage;
    // Windows refuses to rename a directory while this capability is open.
    // Every file is already synced and the stable identity above survives the
    // close for the post-move replacement check.
    drop(dir);

    // Reject a pre-existing alias without touching it. A replacement racing the
    // check is moved as an inode into bound recovery and rejected there; it is
    // never followed for a write.
    let old_recovery = match root.symlink_metadata("publish") {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "static-publish output is not a real directory",
                ));
            }
            let recovery = reserve_publish_recovery(graph, &root)?;
            publish_recovery_race_hook(&recovery)?;
            root.rename("publish", &recovery.dir, "previous")?;
            let retired = recovery.dir.symlink_metadata("previous")?;
            if !retired.is_dir() || retired.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "static-publish output changed during retirement",
                ));
            }
            Some(recovery)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    if let Err(error) = crate::model::move_file_noreplace(&path, out) {
        // The previous site stays complete in conflict recovery. Avoid a
        // compare-then-replace restoration that could clobber a late winner.
        let _ = old_recovery;
        return Err(error);
    }
    let out_meta = fs::symlink_metadata(out)?;
    let same_stage = out_meta.is_dir()
        && !out_meta.file_type().is_symlink()
        && identity_from_path(out).is_ok_and(|live| live == identity);
    if same_stage {
        return Ok(());
    }

    // A replaced stage must never remain live. Move it through the bound graph
    // and recovery directory handles; the previous complete site is already
    // retained separately and is not overwritten during automatic recovery.
    let bad = reserve_publish_recovery(graph, &root)?;
    let _ = root.rename("publish", &bad.dir, "invalid-stage");
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "static-publish staging directory changed during commit",
    ))
}

/// Export public pages to `<root>/publish/`. Returns (output dir, page count).
/// Only pages with `public:: true` are published, unless
/// `:publishing/all-pages-public?` is set in config (matching Logseq).
pub fn publish_graph(graph: &Graph) -> io::Result<(String, usize)> {
    let out = graph.root.join("publish");
    graph.ensure_write_target(&out)?;
    let stage = reserve_publish_stage(graph)?;
    write_publish_stage_file(&stage, "style.css", STYLE.as_bytes())?;
    // Sidebar + fuzzy search are JS-driven: Fuse (vendored, OG's version) + our tiny
    // app.js, both loaded as `<script src>` so they work offline / over file://.
    write_publish_stage_file(
        &stage,
        "fuse.min.js",
        include_str!("../assets/fuse.min.js").as_bytes(),
    )?;
    write_publish_stage_file(&stage, "app.js", APP_JS.as_bytes())?;
    write_publish_stage_file(&stage, "enhance.js", ENHANCE_JS.as_bytes())?;
    let all_public = graph.config.all_pages_public;
    let favorites: HashSet<&str> = graph.config.favorites.iter().map(|s| s.as_str()).collect();

    let pages = graph.list_pages();
    // Query/reference DTOs currently identify their source by logical page name.
    // If two physical files claim that identity, a name-only authorization check
    // cannot prove which file produced a result. Fail closed for that identity:
    // publish neither twin rather than let a private twin borrow the public
    // capability. Ordinary unique pages retain the exact one-file capability.
    let mut source_identity_counts: HashMap<String, usize> = HashMap::new();
    for page in &pages {
        *source_identity_counts
            .entry(crate::refs::page_key(&page.name))
            .or_default() += 1;
    }
    let mut entries: Vec<_> = pages.iter().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    // Pass 1: parse every page into one immutable query snapshot, while keeping
    // only authorized pages in the publication projection. `entries` is already
    // sorted by name, so `public` (and hence the slug assignment below) is
    // deterministic across runs. Queries need the complete fresh snapshot so
    // the renderer can honestly count matches omitted by the public capability;
    // result hydration still comes exclusively from `public` below.
    let mut public: Vec<(&str, PageKind, Arc<doc::Document>)> = Vec::new();
    let mut snapshot_pages = Vec::new();
    for e in entries {
        let content = fs::read_to_string(&e.path)?;
        let mut parsed = if e
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("org"))
        {
            crate::org::parse_org(&content)
        } else {
            doc::parse(&content)
        };
        let is_public = all_public || page_is_public(parsed.pre_block.as_deref());
        crate::model::assign_doc_runtime_ids(&mut parsed.roots, &e.rel_path);
        let parsed = Arc::new(parsed);
        snapshot_pages.push((e.clone(), Arc::clone(&parsed)));
        if !is_public {
            continue;
        }
        if source_identity_counts
            .get(&crate::refs::page_key(&e.name))
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

    // Every downstream resolver gets the same exact document revision as the
    // visibility pass. The snapshot graph has a fully preinstalled cache/page
    // list, so a query cannot fall through to the live graph or a stale
    // pre-export cache. The render context's public-page map remains the sole
    // capability for hydrating any query/embed/namespace result into HTML.
    let snapshot = PublicationGraphSnapshot::new(snapshot_pages)?;

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
        .map(|(name, _, parsed)| (crate::refs::page_key(name), parsed.as_ref()))
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
        graph: Some(&snapshot.graph),
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
        write_publish_stage_file(&stage, &file, html.as_bytes())?;
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
    write_publish_stage_file(&stage, "search-index.js", data.as_bytes())?;

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
    write_publish_stage_file(&stage, "pages.html", pages_html.as_bytes())?;
    let entry_html = welcome_html.unwrap_or(pages_html);
    write_publish_stage_file(&stage, "index.html", entry_html.as_bytes())?;
    commit_publish_stage(graph, stage, &out)?;
    Ok((out.display().to_string(), count))
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
        decorate(&lsdoc::render_html(&body_blocks(raw), &md_opts()), &ctx, 0)
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

    #[test]
    fn publish_emits_sidebar_search_and_block_anchors() {
        let dir = std::env::temp_dir().join(format!("tine-publish-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:favorites [\"Alpha\"]}\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Alpha.md"),
            "public:: true\n- # Intro to [[Beta]] and **bold** text\n  id:: 11111111-1111-1111-1111-111111111111\n- a unique searchwidget term\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Beta.md"),
            "public:: true\n- linking back to [[Alpha]]\n",
        )
        .unwrap();
        // A non-public page must NOT be exported.
        fs::write(dir.join("pages").join("Secret.md"), "- private stuff\n").unwrap();

        let g = Graph::open(&dir);
        let (outdir, count) = publish_graph(&g).unwrap();
        assert_eq!(count, 2, "only the two public pages");
        let out = std::path::Path::new(&outdir);

        // assets emitted alongside the pages
        assert!(out.join("fuse.min.js").exists(), "vendored Fuse shipped");
        assert!(out.join("app.js").exists(), "app.js shipped");
        assert!(
            out.join("enhance.js").exists(),
            "script-free enhancement shipped"
        );

        // embedded search data: pages + blocks globals, favorite flag, stripped text
        let sidx = fs::read_to_string(out.join("search-index.js")).unwrap();
        assert!(
            sidx.starts_with("window.__tinePages="),
            "{}",
            &sidx[..60.min(sidx.len())]
        );
        let alpha = fs::read_to_string(out.join("alpha.html")).unwrap();
        assert!(alpha.contains("Content-Security-Policy"), "{alpha}");
        assert!(!alpha.contains(" onload="), "{alpha}");
        assert!(sidx.contains("window.__tineBlocks="));
        assert!(
            sidx.contains("\"favorite\":true"),
            "Alpha is a favorite: {sidx}"
        );
        assert!(sidx.contains("searchwidget"), "block content indexed");
        assert!(
            sidx.contains("Intro to Beta and bold text"),
            "markup stripped in index: {sidx}"
        );
        assert!(!sidx.contains("[[Beta]]"), "no raw wiki brackets in index");

        // page html: EVERY block carries an anchor (id:: uuid or generated b{n}); the
        // sidebar + scripts are present; no anchorless <li>.
        let alpha = fs::read_to_string(out.join("alpha.html")).unwrap();
        assert!(
            alpha.contains("id=\"11111111-1111-1111-1111-111111111111\""),
            "id:: anchor kept"
        );
        assert!(
            alpha.contains("id=\"b0\""),
            "id-less block got a generated anchor: {alpha}"
        );
        assert!(!alpha.contains("<li>"), "no anchorless <li>");
        assert!(
            alpha.contains("<aside class=\"sidebar\">"),
            "sidebar present"
        );
        assert!(alpha.contains("id=\"tine-search\""), "search box present");
        assert!(alpha.contains("src=\"app.js\""), "app.js linked");

        // index lists public pages, excludes the private one, and uses the shell.
        let index = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(index.contains("alpha.html") && index.contains("beta.html"));
        assert!(!index.contains("secret.html"), "private page excluded");
        assert!(
            index.contains("<aside class=\"sidebar\">"),
            "index uses the sidebar shell"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_fails_closed_when_public_and_private_files_claim_one_page_identity() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tine-publish-ambiguous-{unique}"));
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages/Twin.md"),
            "public:: true\n- public sentinel\n- {{query (page Twin)}}\n",
        )
        .unwrap();
        fs::write(dir.join("journals/Twin.md"), "- PRIVATE SENTINEL\n").unwrap();
        fs::write(dir.join("pages/Visible.md"), "public:: true\n- visible\n").unwrap();

        let graph = Graph::open(&dir);
        let (outdir, count) = publish_graph(&graph).unwrap();
        let out = Path::new(&outdir);
        assert_eq!(count, 1);
        assert!(!out.join("twin.html").exists());
        let all = fs::read_to_string(out.join("search-index.js")).unwrap();
        assert!(!all.contains("PRIVATE SENTINEL"), "{all}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_macros_never_expand_private_graph_content() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-private-macros-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        let private_id = "99999999-9999-4999-8999-999999999999";
        fs::write(
            dir.join("pages/Dashboard.md"),
            format!(
                "public:: true\n- {{{{query (task TODO)}}}}\n- {{{{embed [[Secret]]}}}}\n- {{{{embed (({private_id}))}}}}\n- {{{{namespace PrivateNS}}}}\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Secret.md"),
            format!("- TODO PRIVATE_QUERY_AND_EMBED_TOKEN\n  id:: {private_id}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/PrivateNS___Child.md"),
            "- PRIVATE_NAMESPACE_TOKEN\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let (outdir, count) = publish_graph(&graph).unwrap();
        assert_eq!(count, 1);
        let dashboard =
            fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();
        assert!(!dashboard.contains("PRIVATE_QUERY_AND_EMBED_TOKEN"));
        assert!(!dashboard.contains("PRIVATE_NAMESPACE_TOKEN"));
        assert!(!dashboard.contains("PrivateNS/Child"));
        assert!(
            dashboard.contains("No matching blocks")
                || dashboard.contains("Embedded content is not public"),
            "private macro targets should fail closed: {dashboard}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_uses_one_fresh_snapshot_after_external_visibility_rewrite() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-external-visibility-snapshot-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        let source_id = "99999999-9999-4999-8999-999999999998";
        fs::write(
            dir.join("pages/Dashboard.md"),
            format!(
                "public:: true\n- {{{{query (task TODO)}}}}\n- {{{{query [:find (pull ?b [*]) :where (task ?b \"TODO\")]}}}}\n- {{{{embed (({source_id}))}}}}\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Source.md"),
            format!("- TODO PRIVATE_STALE_TOKEN\n  id:: {source_id}\n"),
        )
        .unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();

        // Simulate a sync/editor outside Tine changing both visibility and body
        // after the live graph cache was populated. Publication must not combine
        // the fresh public classification with stale cached query/embed DTOs.
        fs::write(
            dir.join("pages/Source.md"),
            format!("public:: true\n- harmless current body\n  id:: {source_id}\n"),
        )
        .unwrap();

        let (outdir, count) = publish_graph(&graph).unwrap();
        assert_eq!(count, 2);
        let out = Path::new(&outdir);
        let dashboard = fs::read_to_string(out.join("dashboard.html")).unwrap();
        let source = fs::read_to_string(out.join("source.html")).unwrap();
        let search = fs::read_to_string(out.join("search-index.js")).unwrap();
        let published = format!("{dashboard}\n{source}\n{search}");
        assert!(published.contains("harmless current body"), "{published}");
        assert!(
            !published.contains("PRIVATE_STALE_TOKEN"),
            "a stale live-graph query/embed result crossed the publication snapshot: {published}"
        );
        assert!(
            dashboard.contains("harmless current body"),
            "the block embed must resolve from the same fresh snapshot: {dashboard}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publication_snapshot_uses_live_file_runtime_identity() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-runtime-identity-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(dir.join("pages/Target.md"), "- target\n").unwrap();
        fs::write(
            dir.join("pages/Source.md"),
            "- [[Target]] from source\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let live_id = graph.backlinks("Target")[0].blocks[0].id.clone();
        let mut snapshot_pages = Vec::new();
        for entry in graph.list_pages() {
            let content = fs::read_to_string(&entry.path).unwrap();
            let mut parsed = doc::parse(&content);
            crate::model::assign_doc_runtime_ids(&mut parsed.roots, &entry.rel_path);
            snapshot_pages.push((entry, Arc::new(parsed)));
        }
        let snapshot = PublicationGraphSnapshot::new(snapshot_pages).unwrap();
        assert_eq!(snapshot.graph.backlinks("Target")[0].blocks[0].id, live_id);

        drop(snapshot);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn republish_retires_pages_that_are_no_longer_public() {
        let dir =
            std::env::temp_dir().join(format!("tine-publish-retire-stale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages/Visible.md"),
            "public:: true\n- visible body\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Secret.md"),
            "public:: true\n- stale private token\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let (outdir, count) = publish_graph(&graph).unwrap();
        assert_eq!(count, 2);
        let out = std::path::Path::new(&outdir);
        assert!(out.join("secret.html").exists());

        fs::write(dir.join("pages/Secret.md"), "- stale private token\n").unwrap();
        let graph = Graph::open(&dir);
        let (outdir, count) = publish_graph(&graph).unwrap();
        assert_eq!(count, 1);
        let out = std::path::Path::new(&outdir);
        assert!(out.join("visible.html").exists());
        assert!(
            !out.join("secret.html").exists(),
            "a formerly public page must not remain deployable after republish"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn publish_commit_never_writes_through_a_replaced_output_symlink() {
        use std::os::unix::fs::symlink;

        let dir =
            std::env::temp_dir().join(format!("tine-publish-output-swap-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!(
            "tine-publish-output-swap-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("index.html"), "outside sentinel").unwrap();
        let graph = Graph::open(&dir);
        let stage = reserve_publish_stage(&graph).unwrap();
        write_publish_stage_file(&stage, "index.html", b"generated site").unwrap();
        symlink(&outside, dir.join("publish")).unwrap();

        assert!(commit_publish_stage(&graph, stage, &dir.join("publish")).is_err());

        assert_eq!(
            fs::read_to_string(outside.join("index.html")).unwrap(),
            "outside sentinel"
        );
        assert!(fs::symlink_metadata(dir.join("publish"))
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn publish_stage_handle_survives_ambient_symlink_swap_without_outside_write() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-stage-capability-{}",
            std::process::id()
        ));
        let outside = std::env::temp_dir().join(format!(
            "tine-publish-stage-capability-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            dir.join("pages/Public.md"),
            "public:: true\n- generated sentinel\n",
        )
        .unwrap();
        fs::write(outside.join("style.css"), "outside sentinel").unwrap();
        PUBLISH_STAGE_WRITE_SWAP.with(|slot| *slot.borrow_mut() = Some(outside.clone()));

        let graph = Graph::open(&dir);
        assert!(publish_graph(&graph).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("style.css")).unwrap(),
            "outside sentinel"
        );
        assert!(!outside.join("public.html").exists());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn publish_recovery_handle_survives_ambient_symlink_swap_without_outside_move() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-recovery-capability-{}",
            std::process::id()
        ));
        let outside = std::env::temp_dir().join(format!(
            "tine-publish-recovery-capability-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("publish")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(dir.join("publish/index.html"), "previous site").unwrap();
        fs::write(
            dir.join("pages/Public.md"),
            "public:: true\n- generated sentinel\n",
        )
        .unwrap();
        fs::write(outside.join("previous"), "outside sentinel").unwrap();
        PUBLISH_RECOVERY_SWAP.with(|slot| *slot.borrow_mut() = Some(outside.clone()));

        let graph = Graph::open(&dir);
        let (out, count) = publish_graph(&graph).unwrap();
        assert_eq!(count, 1);
        assert!(Path::new(&out).join("public.html").exists());
        assert_eq!(
            fs::read_to_string(outside.join("previous")).unwrap(),
            "outside sentinel"
        );
        let conflicts = dir.join("logseq/.tine-trash/conflicts");
        assert!(fs::read_dir(conflicts)
            .unwrap()
            .flatten()
            .any(|entry| entry.path().join("previous/index.html").exists()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn publish_uses_welcome_home_and_public_reverse_block_refs() {
        let dir = std::env::temp_dir().join(format!("tine-publish-welcome-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        let id = "11111111-1111-1111-1111-111111111111";
        fs::write(
            dir.join("pages/Welcome to Tine.md"),
            format!("public:: true\n- Welcome target\n  id:: {id}\n- same page (({id}))\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Other.md"),
            format!("public:: true\n- cross page (({id})) and (({id}))\n- missing ((22222222-2222-2222-2222-222222222222))\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Private.md"),
            format!("- private ref (({id}))\n"),
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let (outdir, count) = publish_graph(&graph).unwrap();
        assert_eq!(count, 2);
        let out = std::path::Path::new(&outdir);
        let entry = fs::read_to_string(out.join("index.html")).unwrap();
        let welcome = fs::read_to_string(out.join("welcome-to-tine.html")).unwrap();
        let pages = fs::read_to_string(out.join("pages.html")).unwrap();

        assert!(entry.contains("<h1 class=\"page\">Welcome to Tine</h1>"));
        assert!(entry.contains("href=\"welcome-to-tine.html\">⌂ Home</a>"));
        assert!(welcome.contains("aria-label=\"2 block references\">2</summary>"));
        assert!(welcome.contains("href=\"welcome-to-tine.html#b0\""));
        assert!(welcome.contains("href=\"other.html#b0\""));
        assert!(
            !welcome.contains("Private"),
            "private referrer must not leak"
        );
        assert!(pages.contains("welcome-to-tine.html") && pages.contains("other.html"));
        assert!(pages.contains("href=\"welcome-to-tine.html\">⌂ Home</a>"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_gives_distinct_nonempty_files_on_slug_collision() {
        // DS#4: two titles that collapse to the same ASCII slug ("foo"), plus a
        // title with NO ASCII-alnum chars (empty slug pre-fix). Pre-fix, `Foo!`
        // and `Foo#` both write `foo.html` (the second silently overwrites the
        // first) and `日本語` writes a degenerate `.html`. Post-fix every page must
        // get a distinct, nonempty file, and every cross-page link must point at
        // the file its target was actually written to.
        let dir = std::env::temp_dir().join(format!("tine-publish-collide-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:publishing/all-pages-public? true}\n",
        )
        .unwrap();
        // Each page links to the next so we can check the link map matches files.
        fs::write(
            dir.join("pages").join("Foo!.md"),
            "- alpha body linking [[Foo#]]\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Foo#.md"),
            "- bravo body linking [[日本語]]\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("日本語.md"), "- charlie body\n").unwrap();

        let g = Graph::open(&dir);
        let (outdir, count) = publish_graph(&g).unwrap();
        assert_eq!(count, 3, "all three public pages exported");
        let out = std::path::Path::new(&outdir);

        // The embedded page index (`__tinePages`) is the single source of truth
        // name -> slug; parse it back out.
        let sidx = fs::read_to_string(out.join("search-index.js")).unwrap();
        let after = sidx.strip_prefix("window.__tinePages=").unwrap();
        let json = &after[..after.find(";\n").unwrap()];
        let pages: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert_eq!(pages.len(), 3, "three pages in the index");

        // Every slug is nonempty + distinct, and names an existing, nonempty file.
        let mut seen = std::collections::HashSet::new();
        for p in &pages {
            let s = p["slug"].as_str().unwrap();
            assert!(!s.is_empty(), "slug must be nonempty: {p}");
            assert!(seen.insert(s.to_string()), "slugs must be distinct: {s}");
            let f = out.join(format!("{s}.html"));
            assert!(f.exists(), "file for slug {s} exists");
            assert!(
                fs::metadata(&f).unwrap().len() > 0,
                "file {s}.html nonempty"
            );
        }
        // No page landed in a degenerate empty-slug file.
        assert!(!out.join(".html").exists(), "no empty-slug .html file");

        // Cross-page links point at the file each target was actually written to.
        let name_slug = |name: &str| -> String {
            pages
                .iter()
                .find(|p| p["title"] == name)
                .unwrap_or_else(|| panic!("page {name} in index"))["slug"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let foo_bang = name_slug("Foo!");
        let foo_hash = name_slug("Foo#");
        let cjk = name_slug("日本語");
        let bang_html = fs::read_to_string(out.join(format!("{foo_bang}.html"))).unwrap();
        assert!(
            bang_html.contains(&format!("href=\"{foo_hash}.html\"")),
            "Foo! links to Foo#'s real file ({foo_hash}.html): {bang_html}"
        );
        let hash_html = fs::read_to_string(out.join(format!("{foo_hash}.html"))).unwrap();
        assert!(
            hash_html.contains(&format!("href=\"{cjk}.html\"")),
            "Foo# links to 日本語's real file ({cjk}.html): {hash_html}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_matches_known_vectors() {
        // RFC 4648 test vectors + a binary triple that exercises all 6-bit lanes.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(&[0xff, 0xef, 0xbf]), "/++/");
    }

    #[test]
    fn print_asset_inlining_enforces_per_file_and_shared_export_budgets() {
        let dir = std::env::temp_dir().join(format!("tine-print-budget-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(dir.join("assets/one.png"), b"1234").unwrap();
        fs::write(dir.join("assets/two.png"), b"5678").unwrap();
        fs::write(dir.join("assets/large.png"), b"123456").unwrap();
        let graph = Graph::open(&dir);
        let refs = no_refs();
        let cumulative = RefCell::new(PrintAssetBudget {
            per_asset: 5,
            remaining: 7,
        });
        let cumulative_ctx = Ctx {
            refs: &refs,
            reverse_refs: None,
            graph: Some(&graph),
            slugs: None,
            inline_assets: true,
            print_asset_budget: Some(&cumulative),
            query_cache: None,
            pages: None,
        };

        assert!(inline_asset_uri(&cumulative_ctx, "../assets/one.png").is_some());
        assert_eq!(cumulative.borrow().remaining, 3);
        assert!(
            inline_asset_uri(&cumulative_ctx, "../assets/two.png").is_none(),
            "the second valid file must not cross the shared export ceiling"
        );
        assert_eq!(
            cumulative.borrow().remaining,
            3,
            "a rejection consumes no budget"
        );

        let per_file = RefCell::new(PrintAssetBudget {
            per_asset: 5,
            remaining: 20,
        });
        let per_file_ctx = Ctx {
            print_asset_budget: Some(&per_file),
            ..cumulative_ctx
        };
        assert!(
            inline_asset_uri(&per_file_ctx, "../assets/large.png").is_none(),
            "one oversized file must be rejected before it is returned"
        );
        assert_eq!(per_file.borrow().remaining, 20);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_print_html_is_self_contained_with_inlined_image() {
        let dir = std::env::temp_dir().join(format!("tine-print-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        // A 1x1 PNG (real bytes) so the inliner produces a valid data: URI.
        let png: [u8; 67] = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        fs::write(dir.join("assets").join("pic.png"), png).unwrap();
        let oversized = fs::File::create(dir.join("assets").join("oversized.png")).unwrap();
        oversized.set_len(PRINT_ASSET_MAX_BYTES + 1).unwrap();
        fs::write(
            dir.join("pages").join("Report.md"),
            "- # Report\n- Some **bold** text and a [[Welcome]] link.\n- ![shot](../assets/pic.png)\n- ![large](../assets/oversized.png)\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        let html = g
            .page_print_html("Report", PrintOpts::default())
            .unwrap()
            .expect("page exists");

        // Self-contained: inlined stylesheet + inlined image, no sidebar / app scripts /
        // external style.css.
        assert!(html.contains("<style>"), "stylesheet inlined");
        assert!(
            html.contains("data:image/png;base64,"),
            "image inlined as data URI: {}",
            &html[..0]
        );
        assert!(
            !html.contains("../assets/pic.png"),
            "no relative asset link left"
        );
        assert!(
            html.contains("class=\"print-asset-omitted\""),
            "an oversized image becomes an explicit inert marker"
        );
        assert!(
            !html.contains("oversized.png"),
            "the rejected source path is not retained in the print document"
        );
        assert!(
            !html.contains("<aside class=\"sidebar\">"),
            "no sidebar in print doc"
        );
        assert!(!html.contains("src=\"app.js\""), "no app.js in print doc");
        assert!(!html.contains("<script"), "print doc executes no scripts");
        assert!(
            !html.contains("cdn.jsdelivr.net"),
            "print doc has no CDN resources"
        );
        assert!(
            html.contains("script-src 'none'"),
            "print doc denies script execution even if markup regresses"
        );
        assert!(
            !html.contains("href=\"style.css\""),
            "no external stylesheet link"
        );
        // Content actually rendered.
        assert!(
            html.contains("<h1 class=\"page\">Report</h1>"),
            "page heading"
        );
        assert!(
            html.contains("<strong>bold</strong>"),
            "inline markup rendered"
        );
        assert!(html.contains("@media print"), "print CSS present");

        // Missing page → None (not an error).
        assert!(g
            .page_print_html("No Such Page", PrintOpts::default())
            .unwrap()
            .is_none());

        // Collapsed handling: a collapsed parent's children are expanded by default,
        // and hidden when expand_collapsed is off.
        fs::write(
            dir.join("pages").join("Folded.md"),
            "- Parent\n  collapsed:: true\n\t- hidden child text\n",
        )
        .unwrap();
        let g2 = Graph::open(&dir);
        let expanded = g2
            .page_print_html("Folded", PrintOpts::default())
            .unwrap()
            .unwrap();
        assert!(
            expanded.contains("hidden child text"),
            "default expands collapsed"
        );
        let folded = g2
            .page_print_html(
                "Folded",
                PrintOpts {
                    expand_collapsed: false,
                    ..PrintOpts::default()
                },
            )
            .unwrap()
            .unwrap();
        assert!(
            !folded.contains("hidden child text"),
            "folded hides collapsed children"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_begin_query_renders_authored_title_and_results() {
        let dir =
            std::env::temp_dir().join(format!("tine-publish-begin-query-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:publishing/all-pages-public? true}\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Tasks.md"),
            "- TODO BEGIN_QUERY_PUBLIC_RESULT\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Dashboard.md"),
            "- #+BEGIN_QUERY\n  {:title \"Open work\"\n   :query [:find (pull ?b [*]) :where (task ?b \"TODO\")]}\n  #+END_QUERY\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let (outdir, _) = publish_graph(&graph).unwrap();
        let dashboard =
            fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

        assert!(
            dashboard.contains("class=\"query-head\">Open work"),
            "{dashboard}"
        );
        assert!(
            dashboard.contains("BEGIN_QUERY_PUBLIC_RESULT"),
            "{dashboard}"
        );
        assert!(!dashboard.contains("class=\"query-omitted\""), "{dashboard}");
        assert!(!dashboard.contains("#+BEGIN_QUERY"), "{dashboard}");
        assert!(!dashboard.contains("#+END_QUERY"), "{dashboard}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_begin_query_reports_private_rows_without_leaking_them() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-begin-query-private-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages/Dashboard.md"),
            "public:: true\n- #+BEGIN_QUERY\n  {:title \"Private-aware work\"\n   :query [:find (pull ?b [*]) :where (task ?b \"TODO\")]}\n  #+END_QUERY\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Secret.md"),
            "- TODO PRIVATE_BEGIN_QUERY_RESULT_MUST_NOT_LEAK\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let (outdir, count) = publish_graph(&graph).unwrap();
        assert_eq!(count, 1);
        let dashboard =
            fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

        assert!(dashboard.contains("class=\"query-omitted\""), "{dashboard}");
        assert!(
            dashboard.contains("1 result on non-public pages omitted."),
            "{dashboard}"
        );
        assert!(
            !dashboard.contains("PRIVATE_BEGIN_QUERY_RESULT_MUST_NOT_LEAK"),
            "{dashboard}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_malformed_begin_query_is_inert_and_hides_its_payload() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-begin-query-malformed-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages/Dashboard.md"),
            "public:: true\n- #+BEGIN_QUERY\n  {:title \"MALFORMED_BEGIN_QUERY_PAYLOAD\" :query (task TODO)}\n  #+END_QUERY\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let (outdir, _) = publish_graph(&graph).unwrap();
        let dashboard =
            fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

        assert!(
            dashboard.contains("class=\"query-unsupported begin-query-unsupported\""),
            "{dashboard}"
        );
        assert!(
            !dashboard.contains("MALFORMED_BEGIN_QUERY_PAYLOAD"),
            "{dashboard}"
        );
        assert!(!dashboard.contains("#+BEGIN_QUERY"), "{dashboard}");
        assert!(!dashboard.contains("#+END_QUERY"), "{dashboard}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_renders_facets_queries_and_embeds() {
        let dir = std::env::temp_dir().join(format!("tine-publish-facets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:publishing/all-pages-public? true}\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Target.md"),
            "- an embeddable target\n  id:: 22222222-2222-2222-2222-222222222222\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Main.md"),
            "- TODO [#A] do the thing\n  SCHEDULED: <2026-07-10 Fri>\n  notes after the schedule\n\
             - DONE finished it\n\
             - a note\n  status:: open\n\
             - {{query (task TODO)}}\n\
             - {{embed ((22222222-2222-2222-2222-222222222222))}}\n\
             - {{video https://www.youtube.com/watch?v=abc123}}\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        let (outdir, _) = publish_graph(&g).unwrap();
        let main = fs::read_to_string(std::path::Path::new(&outdir).join("main.html")).unwrap();

        // task facets: checkbox + marker badge, priority, planning date
        assert!(
            main.contains("class=\"task-checkbox\""),
            "open task checkbox: {main}"
        );
        assert!(
            main.contains("class=\"task-marker m-todo\">TODO"),
            "TODO badge"
        );
        assert!(
            main.contains("class=\"priority p-a\">[#A]"),
            "priority badge"
        );
        assert!(main.contains("SCHEDULED:"), "scheduled line");
        assert!(
            main.contains("notes after the schedule"),
            "body after schedule"
        );
        let scheduled_trailers = main.matches("class=\"planning scheduled\"").count();
        assert!(scheduled_trailers > 0, "scheduled trailer rendered");
        assert_eq!(
            main.matches("2026-07-10 Fri").count(),
            scheduled_trailers,
            "each planning date renders only in trailer chrome, not again in the body"
        );
        assert!(
            main.contains("class=\"task-checkbox checked\"") && main.contains("class=\"b done\""),
            "DONE checked + muted"
        );
        // block property shown
        assert!(
            main.contains("status") && main.contains("open"),
            "block property rendered"
        );
        // query ran and rendered the TODO result (not empty, not the literal macro)
        assert!(main.contains("class=\"query\""), "query block rendered");
        assert!(
            !main.contains("{{query"),
            "query macro expanded, not literal"
        );
        // embed inlined the target block's text
        assert!(main.contains("class=\"embed"), "embed rendered");
        assert!(
            main.contains("class=\"embed block-embed single-root\"><ul class=\"embed-outline\">"),
            "block embed exposes one CSS-scoped root without generic outline connectors: {main}"
        );
        assert!(
            main.contains("an embeddable target"),
            "embed inlined target content: {main}"
        );
        // video → youtube iframe
        assert!(main.contains("youtube.com/embed/abc123"), "video iframe");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_memoizes_repeated_query_macros() {
        let dir =
            std::env::temp_dir().join(format!("tine-publish-query-memo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:publishing/all-pages-public? true}\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Tasks.md"),
            "- TODO repeated query memo target\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Dashboard.md"),
            "- {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        let _guard = publish_test_counts::count_for(&dir);
        let (outdir, _) = publish_graph(&g).unwrap();

        assert_eq!(
            publish_test_counts::query_runs(),
            1,
            "same query source should be evaluated once per export"
        );
        let dash =
            fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();
        assert_eq!(
            dash.matches("class=\"query\"").count(),
            5,
            "each macro occurrence still renders independently"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publication_rejects_query_sources_before_keying_and_bounds_valid_memos() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-query-source-bound-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(dir.join("pages").join("P.md"), "- TODO target\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let refs = RefIndex::new();
        let cache: SharedQueryCache = RefCell::new(QueryCache::default());
        let ctx = Ctx {
            refs: &refs,
            reverse_refs: None,
            graph: Some(&graph),
            slugs: None,
            inline_assets: false,
            print_asset_budget: None,
            query_cache: Some(&cache),
            pages: None,
        };

        let oversized = "x".repeat(crate::query::QUERY_SOURCE_MAX_BYTES + 1);
        assert!(render_query(&graph, &oversized, &ctx, 0).contains("publication limit"));
        let nested = format!(
            "{}(task TODO){}",
            "(and ".repeat(1_000),
            ")".repeat(1_000)
        );
        assert!(render_query(&graph, &nested, &ctx, 0).contains("nesting is too deep"));
        assert!(cache.borrow().entries.is_empty());

        for index in 0..(QUERY_CACHE_MAX_ENTRIES + 20) {
            let source = format!("(and (task TODO) (content \"memo-{index}\"))");
            let _ = render_query(&graph, &source, &ctx, 0);
        }
        let cache = cache.borrow();
        assert_eq!(cache.entries.len(), QUERY_CACHE_MAX_ENTRIES);
        assert!(cache.bytes <= QUERY_CACHE_MAX_BYTES);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_query_keeps_and_hydrates_a_match_below_a_nonmatching_gap() {
        let dir =
            std::env::temp_dir().join(format!("tine-publish-query-gap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:publishing/all-pages-public? true}\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Tasks.md"),
            "- TODO parity ancestor\n  - DONE parity gap\n    - TODO parity grandchild\n      - live child below retained grandchild\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Dashboard.md"),
            "- {{query (task TODO)}}\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let (outdir, _) = publish_graph(&graph).unwrap();
        let dashboard =
            fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

        assert_eq!(
            dashboard.matches("parity grandchild").count(),
            2,
            "{dashboard}"
        );
        assert_eq!(
            dashboard
                .matches("live child below retained grandchild")
                .count(),
            2,
            "{dashboard}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_reuses_pass1_docs_for_repeated_page_embeds() {
        let dir =
            std::env::temp_dir().join(format!("tine-publish-embed-reuse-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:publishing/all-pages-public? true}\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Target.md"),
            "- shared embed target\n  id:: 33333333-3333-3333-3333-333333333333\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Org Page.org"),
            "public:: true\n* org fixture page\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Dashboard.md"),
            "- {{embed [[Target]]}}\n\
             - {{embed [[Target]]}}\n\
             - {{embed [[Target]]}}\n\
             - {{embed [[Target]]}}\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        let _guard = publish_test_counts::count_for(&dir);
        let (outdir, _) = publish_graph(&g).unwrap();

        assert_eq!(
            publish_test_counts::page_doc_loads(),
            0,
            "public page embeds should reuse pass-1 parsed docs, not reload from disk"
        );
        let dash =
            fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();
        assert_eq!(
            dash.matches("shared embed target").count(),
            4,
            "each embed occurrence still renders"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// Dev utility (not run by default): materialize a richer sample export at a
    /// stable path so the published sidebar + search can be screenshot-verified.
    /// `cargo test -p tine-core --lib -- --ignored gen_sample_export --nocapture`
    /// then open `file://$TMPDIR/tine-sample-export/publish/index.html`.
    #[test]
    #[ignore]
    fn gen_sample_export() {
        let dir = std::env::temp_dir().join("tine-sample-export");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:publishing/all-pages-public? true\n :favorites [\"Welcome\" \"Parameterized IP\"]}\n",
        )
        .unwrap();
        let write = |name: &str, body: &str| fs::write(dir.join("pages").join(name), body).unwrap();
        write(
            "Welcome.md",
            "- # Welcome to the published graph\n  id:: aaaaaaaa-0000-0000-0000-000000000001\n- This is a static export with a **sidebar** and fuzzy [[search]].\n- See [[Parameterized IP]] and the [[ILP Survey]].\n",
        );
        write(
            "Parameterized IP.md",
            "- # Parameterized IP\n- Fixed-parameter tractability of integer programming; **parameterized complexity** of ILPs.\n  id:: aaaaaaaa-0000-0000-0000-000000000002\n- Related to [[ILP Survey]].\n",
        );
        write(
            "ILP Survey.md",
            "- # ILP Survey\n- A survey of integer linear programming techniques and n-fold IP.\n- Back to [[Welcome]].\n",
        );
        write("Search.md", "- Notes on full-text search and ranking.\n");
        write("Project Ideas.md", "- # Project Ideas\n- A grab-bag of ideas, including continuous bribery and opinion diffusion.\n");
        // Exercises every construct the M3 render_html rewrite added to the export:
        // fenced code (highlight.js), tables (data-align), callouts, inline/display math,
        // lists (incl. checkboxes + def-list term), blockquote, org timestamps.
        write(
            "Rendering Showcase.md",
            "- # Rendering Showcase\n\
             - A paragraph with **bold**, *italic*, `inline code`, a [[Welcome]] link and a #demo tag.\n\
             - ```rust\n  fn solve(n: usize) -> usize {\n      (0..n).filter(|x| x & 1 == 0).count()\n  }\n  ```\n\
             - | Method | Time | Note |\n  | :--- | :---: | ---: |\n  | n-fold IP | fast | linear |\n  | brute force | slow | exp |\n\
             - > [!NOTE] Heads up\n  > Callouts now render straight from the AST.\n\
             - > [!WARNING]\n  > Macros are dropped in a static export (they can't run).\n\
             - Inline math $e^{i\\pi}+1=0$ and a display block:\n\
             - $$\\int_0^1 x^2\\,dx = \\tfrac{1}{3}$$\n\
             - Unordered:\n  * first\n  * second\n      * nested\n\
             - Tasks:\n  * [ ] open item\n  * [x] done item\n\
             - Coffee\n  : A hot drink brewed from beans.\n\
             - > A plain blockquote over\n  > two lines.\n\
             - let's try again *(something)* -> --> -- en --- em \\alpha \\Delta\n\
             - A meeting <2026-06-30 Tue 14:00> and a deadline.\n",
        );
        fs::write(
            dir.join("journals").join("2026_06_28.md"),
            "- Worked on the **published export**: sidebar + search.\n- Linked [[Parameterized IP]].\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        let (outdir, count) = publish_graph(&g).unwrap();
        println!("SAMPLE_EXPORT_DIR={outdir} pages={count}");

        // Also emit the single-page print/PDF document for the showcase page, so
        // the print CSS + self-contained render can be eyeballed / screenshotted.
        let print = g
            .page_print_html("Rendering Showcase", PrintOpts::default())
            .unwrap()
            .unwrap();
        let pfile = dir.join("print-sample.html");
        fs::write(&pfile, print).unwrap();
        println!("SAMPLE_PRINT_HTML={}", pfile.display());
    }
}
