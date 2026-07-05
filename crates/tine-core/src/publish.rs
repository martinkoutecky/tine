//! Static HTML export: render the whole graph to a folder of linked HTML pages
//! plus an index. Inline formatting, nested block lists, and `[[page]]` links
//! become real anchors between the generated files.

use crate::doc::{self, DocBlock};
use crate::model::{BlockDto, Graph, PageKind};
use crate::refs::block_id;
use lsdoc::ast::{Block, Inline, Url};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::io;

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

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Read a local `assets/<file>` image referenced by an export `data-asset` path
/// (e.g. `../assets/cat.png`) and return it as a self-contained `data:` URI, for
/// the single-page print/PDF export. Returns `None` for a non-local URL (http/…),
/// no graph, an unreadable file, or a path that escapes `assets/` — the caller
/// then keeps the original `src` (a broken image, never a failed export).
fn inline_asset_uri(ctx: &Ctx, src: &str) -> Option<String> {
    let graph = ctx.graph?;
    // Only local asset references; leave remote/data URLs untouched.
    if src.contains("://") || src.starts_with("data:") {
        return None;
    }
    // `read_asset` re-guards against traversal; pass just the file name so a
    // `../assets/x` (or `assets/x`) ref resolves to `<graph>/assets/x`.
    let name = src.rsplit('/').next().unwrap_or(src);
    let bytes = graph.read_asset(name).ok()?;
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

/// Append `s` HTML-escaped for an ATTRIBUTE value (`& < > " '`) — matches lsdoc's
/// `esc_attr`, so a re-emitted attribute (asset src, alt) round-trips identically.
fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
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

        if name == "a" && has_class(inner, "page-ref") {
            if let Some(page) = tag_attr(inner, "data-page") {
                out.push_str(&format!(
                    "<a class=\"ref\" href=\"{}.html\">",
                    slug(&unescape(page))
                ));
                strip_brackets = true;
                continue;
            }
        }
        if name == "a" && has_class(inner, "tag") {
            if let Some(page) = tag_attr(inner, "data-page") {
                out.push_str(&format!(
                    "<a class=\"tag\" href=\"{}.html\">",
                    slug(&unescape(page))
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
                            t.slug, id, text
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
                let src = if ctx.inline_assets {
                    // Self-contained print doc: read the asset and emit a `data:`
                    // URI. Falls back to the relative path if it can't be read
                    // (missing file / non-local URL) — a broken img, never a panic.
                    inline_asset_uri(ctx, &src).unwrap_or(src)
                } else {
                    src
                };
                out.push_str(&format!(
                    "<img class=\"inline-image\" src=\"{}\" alt=\"{}\">",
                    esc_attr(&src),
                    esc_attr(&alt)
                ));
                continue;
            }
        }
        if (name == "video" || name == "audio") && has_class(inner, "media-embed") {
            if let Some(asset) = tag_attr(inner, "data-asset") {
                let _ = take_to_close(html, &mut i, name); // empty element
                out.push_str(&format!(
                    "<{name} class=\"media-embed\" controls src=\"{}\"></{name}>",
                    esc_attr(&unescape(asset))
                ));
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
    crate::render::parse_block(raw, false)
        .into_iter()
        .filter(|b| {
            !matches!(b, Block::Properties { .. }) && !crate::doc::block_is_standalone_planning(b)
        })
        .collect()
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
    graph: Option<&'a Graph>,
    /// When true (the single-page PDF/print export), rewrite each `data-asset`
    /// image `src` to a self-contained `data:` URI by reading the asset bytes,
    /// so the printed document needs no sibling `assets/` folder. The whole-graph
    /// site export keeps the relative `../assets/<file>` links (`false`).
    inline_assets: bool,
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
        "namespace" => render_namespace(graph, arg0),
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
    let groups = if crate::query::is_advanced(src) {
        crate::query::run_advanced_query(graph, src, None).groups
    } else {
        crate::query::run_query(graph, src)
    };
    let total: usize = groups.iter().map(|g| g.blocks.len()).sum();
    let mut out = format!(
        "<div class=\"query\"><div class=\"query-head\">Query <span class=\"query-count\">{}</span></div>",
        total
    );
    if total == 0 {
        out.push_str("<div class=\"query-empty\">No matching blocks.</div>");
    } else {
        out.push_str("<ul class=\"query-results\">");
        for g in &groups {
            for blk in &g.blocks {
                render_result_block(blk, &mut out, ctx, depth);
            }
        }
        out.push_str("</ul>");
    }
    out.push_str("</div>");
    out
}

/// Inline an `{{embed ((uuid))}}` or `{{embed [[Page]]}}`.
fn render_embed(graph: &Graph, arg: &str, ctx: &Ctx, depth: u8) -> String {
    if let Some(uuid) = arg.strip_prefix("((").and_then(|s| s.strip_suffix("))")) {
        return match crate::query::resolve_block(graph, uuid.trim()) {
            Some(g) => {
                let mut out = String::from("<div class=\"embed block-embed\"><ul>");
                for blk in &g.blocks {
                    render_result_block(blk, &mut out, ctx, depth);
                }
                out.push_str("</ul></div>");
                out
            }
            None => "<div class=\"embed embed-missing\">Embedded block not found.</div>".into(),
        };
    }
    if let Some(page) = arg.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        let page = page.trim();
        return match load_page_doc(graph, page) {
            Some(doc) => {
                let mut out = format!(
                    "<div class=\"embed page-embed\"><a class=\"embed-title ref\" href=\"{}.html\">{}</a><ul>",
                    slug(page),
                    esc(page)
                );
                for b in &doc.roots {
                    render_embedded_block(b, &mut out, ctx, depth);
                }
                out.push_str("</ul></div>");
                out
            }
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
    format!(
        "<div class=\"video-embed\"><a href=\"{}\">{}</a></div>",
        esc_attr(url),
        esc(url)
    )
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
fn render_namespace(graph: &Graph, ns: &str) -> String {
    let prefix = format!("{}/", ns.trim());
    let mut children: Vec<String> = graph
        .list_pages()
        .into_iter()
        .map(|e| e.name)
        .filter(|n| n.starts_with(&prefix))
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
            slug(c),
            esc(c)
        ));
    }
    out.push_str("</ul></div>");
    out
}

/// Read + parse a page's file by name (for `{{embed [[Page]]}}`), case-insensitively.
fn load_page_doc(graph: &Graph, name: &str) -> Option<doc::Document> {
    let pages = graph.list_pages();
    let e = pages.iter().find(|e| e.name.eq_ignore_ascii_case(name))?;
    let content = fs::read_to_string(&e.path).ok()?;
    Some(doc::parse(&content))
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
    out.push_str(&format!("<li id=\"{anchor}\">"));
    let text = ast_plain_text(&blocks);
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
    out.push_str(&decorate(&lsdoc::render_html(&blocks, &md_opts()), ctx, 0));
    out.push_str("</div>");
    emit_trailer_facets(b.scheduled(), b.deadline(), &b.properties(), out);

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
    shell(title, &format!("{heading}{body}"))
}

/// The shared two-column document shell used by every generated page: `<head>` +
/// the persistent sidebar (home link, search box, and a `#tine-pages` list filled
/// by `app.js`) + the page's `<main>` + the export scripts. The sidebar markup is
/// identical on every page; `app.js` reads the embedded `search-index.js` globals
/// (`window.__tinePages` / `__tineBlocks`) — read as `<script>` globals, never
/// `fetch`ed — so navigation and search work offline / opened straight off disk
/// (`file://`, where `fetch` of a sibling file is blocked but `<script src>` is not).
fn shell(title: &str, main: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title>\
<link rel=\"stylesheet\" href=\"style.css\">{katex}{hljs}</head><body>\
<aside class=\"sidebar\">\
<a class=\"home\" href=\"index.html\">\u{2302} Home</a>\
<input id=\"tine-search\" type=\"search\" placeholder=\"Search\u{2026}\" autocomplete=\"off\" spellcheck=\"false\">\
<div id=\"tine-results\" hidden></div>\
<nav id=\"tine-pages\"></nav>\
</aside><main>{main}</main>\
<script src=\"search-index.js\"></script><script src=\"fuse.min.js\"></script><script src=\"app.js\"></script>\
</body></html>",
        title = esc(title),
        katex = KATEX_HEAD,
        hljs = HLJS_HEAD,
        main = main,
    )
}

/// A **self-contained single page** for the print-to-PDF export: the same block
/// render as a published page, but with the stylesheet + print rules inlined and
/// no sidebar / search / app scripts — so it prints (or opens) standalone. Assets
/// are inlined as `data:` URIs upstream (`inline_assets`), so no sibling folder is
/// needed. KaTeX / highlight.js still load from CDN (math/code typeset when online;
/// offline shows raw TeX / plain code, same as a published page).
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
{katex}{hljs}<style>{fonts}{style}\n{print}\n{tuned}</style></head><body class=\"print\">\
<main>{main}</main></body></html>",
        title = esc(title),
        katex = KATEX_HEAD,
        hljs = HLJS_HEAD,
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
    let ctx = Ctx {
        refs: &refs,
        graph: Some(graph),
        inline_assets: true,
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
const KATEX_HEAD: &str = r#"<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/katex.min.css"><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/katex.min.js"></script><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/contrib/mhchem.min.js"></script><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/contrib/auto-render.min.js" onload="renderMathInElement(document.body,{delimiters:[{left:'\\[',right:'\\]',display:true},{left:'\\(',right:'\\)',display:false}],throwOnError:false})"></script>"#;

// highlight.js (from CDN) syntax-highlights the `<pre class="code-block"><code
// class="hljs language-X">` blocks lsdoc emits (the export's `data-lang` → `language-X`).
// `highlightAll()` reads the `language-X` class; `defer` + onload runs it after the body
// parses. Offline / no network → plain (already-escaped) code, never broken.
const HLJS_HEAD: &str = r#"<link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11.11.1/build/styles/github.min.css"><script defer src="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11.11.1/build/highlight.min.js" onload="hljs.highlightAll()"></script>"#;

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
ul.outline ul{padding-left:1.25rem;margin:.1rem 0;border-left:1px solid var(--line)}
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
.embed{border-left:3px solid var(--line);padding:.1rem 0 .1rem .8rem;margin:.35rem 0}
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
        var href = e.slug + '.html#' + e.anchor;
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

/// Export public pages to `<root>/publish/`. Returns (output dir, page count).
/// Only pages with `public:: true` are published, unless
/// `:publishing/all-pages-public?` is set in config (matching Logseq).
pub fn publish_graph(graph: &Graph) -> io::Result<(String, usize)> {
    let out = graph.root.join("publish");
    fs::create_dir_all(&out)?;
    fs::write(out.join("style.css"), STYLE)?;
    // Sidebar + fuzzy search are JS-driven: Fuse (vendored, OG's version) + our tiny
    // app.js, both loaded as `<script src>` so they work offline / over file://.
    fs::write(
        out.join("fuse.min.js"),
        include_str!("../assets/fuse.min.js"),
    )?;
    fs::write(out.join("app.js"), APP_JS)?;
    let all_public = graph.config.all_pages_public;
    let favorites: HashSet<&str> = graph.config.favorites.iter().map(|s| s.as_str()).collect();

    let pages = graph.list_pages();
    let mut entries: Vec<_> = pages.iter().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    // Pass 1: parse every page, keep only the public ones, and build the block-ref
    // index from them (a `((ref))` only resolves to a block that's actually
    // exported). Slug + kind are captured here so pass 2 can render + link.
    let mut public: Vec<(&str, String, PageKind, doc::Document)> = Vec::new();
    let mut refs = RefIndex::new();
    for e in entries {
        let Ok(content) = fs::read_to_string(&e.path) else {
            continue;
        };
        let parsed = doc::parse(&content);
        if !all_public && !page_is_public(parsed.pre_block.as_deref()) {
            continue;
        }
        let slug = slug(&e.name);
        collect_block_refs(&parsed.roots, &slug, &mut refs);
        public.push((e.name.as_str(), slug, e.kind, parsed));
    }

    // Pass 2: render each public page (collecting the per-block search index along
    // the way), accumulate the sidebar page index (`__tinePages`) and the static
    // no-JS all-pages list shown in the index page's <main>.
    let mut index_list = String::new();
    let mut all_blocks: Vec<serde_json::Value> = Vec::new();
    let mut sidebar_pages: Vec<serde_json::Value> = Vec::new();
    let mut count = 0;
    // The render context: the block-ref index + the graph (so `{{query}}`/`{{embed}}`/
    // `{{namespace}}` macros can resolve against real data at publish time).
    let ctx = Ctx {
        refs: &refs,
        graph: Some(graph),
        inline_assets: false,
    };
    for (name, slug, kind, parsed) in &public {
        let file = format!("{slug}.html");
        fs::write(
            out.join(&file),
            page_html(name, slug, parsed, *kind, &ctx, &mut all_blocks),
        )?;
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
    fs::write(out.join("search-index.js"), data)?;

    // Index page <main>: the alphabetical all-pages list — the no-JS fallback / home
    // — wrapped in the same sidebar shell as every page.
    let main = format!(
        "<h1 class=\"page\">Pages</h1><ul class=\"outline index-list\">{}</ul>\
<footer>Published with Tine</footer>",
        index_list
    );
    fs::write(out.join("index.html"), shell("Index", &main))?;
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
            graph: None,
            inline_assets: false,
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

        // embedded search data: pages + blocks globals, favorite flag, stripped text
        let sidx = fs::read_to_string(out.join("search-index.js")).unwrap();
        assert!(
            sidx.starts_with("window.__tinePages="),
            "{}",
            &sidx[..60.min(sidx.len())]
        );
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
        fs::write(
            dir.join("pages").join("Report.md"),
            "- # Report\n- Some **bold** text and a [[Welcome]] link.\n- ![shot](../assets/pic.png)\n",
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
            !html.contains("<aside class=\"sidebar\">"),
            "no sidebar in print doc"
        );
        assert!(!html.contains("src=\"app.js\""), "no app.js in print doc");
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
            "- TODO [#A] do the thing\n  SCHEDULED: <2026-07-10 Fri>\n\
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
            main.contains("an embeddable target"),
            "embed inlined target content: {main}"
        );
        // video → youtube iframe
        assert!(main.contains("youtube.com/embed/abc123"), "video iframe");

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
