//! Static HTML export: render the whole graph to a folder of linked HTML pages
//! plus an index. Inline formatting, nested block lists, and `[[page]]` links
//! become real anchors between the generated files.

use crate::doc::{self, DocBlock};
use crate::model::{Graph, PageKind};
use crate::refs::{as_block_ref, block_id, read_bracket_link};
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
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Where a block lives — its target page slug + its first content line — keyed by
/// the block's `id::` uuid, built from the exported (public) pages so that
/// `((block refs))` can link to the actual block (`<slug>.html#<uuid>`).
struct RefTarget {
    slug: String,
    text: String,
}
type RefIndex = std::collections::HashMap<String, RefTarget>;

/// First non-property / non-scheduling / non-blank line of a block (its text).
fn first_content_line(raw: &str) -> String {
    raw.lines()
        .find(|l| {
            let t = l.trim();
            !is_property_line(l)
                && !t.starts_with("SCHEDULED:")
                && !t.starts_with("DEADLINE:")
                && !t.is_empty()
        })
        .unwrap_or("")
        .to_string()
}

fn collect_block_refs(blocks: &[DocBlock], slug: &str, refs: &mut RefIndex) {
    for b in blocks {
        if let Some(id) = block_id(&b.raw) {
            refs.insert(id, RefTarget { slug: slug.to_string(), text: first_content_line(&b.raw) });
        }
        collect_block_refs(&b.children, slug, refs);
    }
}

/// Emit a block reference: a link to its target block showing the label (else the
/// target's text). An unresolved ref (target not public / missing) renders as
/// muted text — never the broken link the old `[^)]` parse produced.
fn emit_block_ref(refs: &RefIndex, id: &str, label: Option<&str>, out: &mut String) {
    match refs.get(id) {
        Some(t) => {
            let text = label.map(esc).unwrap_or_else(|| esc(&t.text));
            out.push_str(&format!(
                "<a class=\"ref block-ref\" href=\"{}.html#{}\">{}</a>",
                t.slug, id, text
            ));
        }
        None => {
            let text = label.map(esc).unwrap_or_else(|| esc(&format!("(({id}))")));
            out.push_str(&format!("<span class=\"block-ref\">{text}</span>"));
        }
    }
}

/// Render a single line of inline markdown to HTML. `refs` resolves `((block ref))`
/// targets to their exported page + anchor.
fn render_inline(input: &str, refs: &RefIndex) -> String {
    let mut out = String::new();
    let mut i = 0;
    let mut plain = String::new();
    fn flush(plain: &mut String, out: &mut String) {
        if !plain.is_empty() {
            out.push_str(&esc(plain));
            plain.clear();
        }
    }
    'outer: while i < input.len() {
        let rest = &input[i..];
        // $$display$$ / $inline$ math — emit the TeX wrapped in `\[..\]` / `\(..\)`
        // delimiters (escaped as text) for KaTeX's auto-render to typeset in the
        // browser. Parsed before every other rule so emphasis/code/link handling
        // can't mangle the TeX; mirrors the in-app parser (parseInline.ts): `$$` is
        // display, and the body must be non-empty.
        if rest.starts_with('$') {
            let dbl = rest.starts_with("$$");
            let delim = if dbl { "$$" } else { "$" };
            if let Some(end) = rest[delim.len()..].find(delim) {
                if end > 0 {
                    let tex = &rest[delim.len()..delim.len() + end];
                    flush(&mut plain, &mut out);
                    let (l, r, cls) = if dbl {
                        ("\\[", "\\]", "math math-display")
                    } else {
                        ("\\(", "\\)", "math")
                    };
                    out.push_str(&format!("<span class=\"{cls}\">{l}{}{r}</span>", esc(tex)));
                    i += delim.len() * 2 + end;
                    continue;
                }
            }
        }
        // ((block ref)) — link to the target block (or muted text if unresolved).
        if let Some(after) = rest.strip_prefix("((") {
            if let Some(end) = after.find("))") {
                flush(&mut plain, &mut out);
                emit_block_ref(refs, after[..end].trim(), None, &mut out);
                i += 2 + end + 2;
                continue;
            }
        }
        // [[page]]
        if let Some(after) = rest.strip_prefix("[[") {
            if let Some(end) = after.find("]]") {
                flush(&mut plain, &mut out);
                let name = &after[..end];
                out.push_str(&format!("<a class=\"ref\" href=\"{}.html\">{}</a>", slug(name), esc(name)));
                i += 2 + end + 2;
                continue;
            }
        }
        // ![alt](url) — paren-balanced target (URLs may contain parens).
        if rest.starts_with("![") {
            if let Some((alt, url, len)) = read_bracket_link(&rest[1..]) {
                flush(&mut plain, &mut out);
                out.push_str(&format!("<img alt=\"{}\" src=\"{}\">", esc(alt), esc(url)));
                i += 1 + len;
                continue;
            }
        }
        // [label](url) — paren-balanced; `[label](((uuid)))` is a block ref.
        if rest.starts_with('[') {
            if let Some((label, url, len)) = read_bracket_link(rest) {
                flush(&mut plain, &mut out);
                match as_block_ref(url) {
                    Some(id) => emit_block_ref(refs, id, Some(label), &mut out),
                    None => out.push_str(&format!("<a href=\"{}\">{}</a>", esc(url), esc(label))),
                }
                i += len;
                continue;
            }
        }
        // #tag
        if rest.starts_with('#') {
            let after = &rest[1..];
            let len = after
                .find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '/')))
                .unwrap_or(after.len());
            if len > 0 {
                flush(&mut plain, &mut out);
                let tag = &after[..len];
                out.push_str(&format!("<a class=\"tag\" href=\"{}.html\">#{}</a>", slug(tag), esc(tag)));
                i += 1 + len;
                continue;
            }
        }
        // **bold** / __bold__ / *italic* / _italic_
        for (delim, tag) in [("**", "strong"), ("__", "strong"), ("*", "em"), ("_", "em")] {
            if rest.starts_with(delim) {
                if let Some(end) = rest[delim.len()..].find(delim) {
                    let inner = &rest[delim.len()..delim.len() + end];
                    if !inner.is_empty() {
                        flush(&mut plain, &mut out);
                        out.push_str(&format!("<{tag}>{}</{tag}>", render_inline(inner, refs)));
                        i += delim.len() * 2 + end;
                        continue 'outer;
                    }
                }
            }
        }
        // `code`
        if rest.starts_with('`') {
            if let Some(end) = rest[1..].find('`') {
                flush(&mut plain, &mut out);
                out.push_str(&format!("<code>{}</code>", esc(&rest[1..1 + end])));
                i += 2 + end;
                continue;
            }
        }
        // default: copy one char
        let ch = input[i..].chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut plain, &mut out);
    out
}

fn is_property_line(l: &str) -> bool {
    match l.find("::") {
        Some(idx) if idx > 0 => {
            let key = l[..idx].trim();
            !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        }
        _ => false,
    }
}

/// Reduce one inline-markdown line to readable plain text for the search index +
/// snippets: drop heading hashes and emphasis/code markers, unwrap `[[Page]]`,
/// keep `[label](url)` / `![alt](url)` labels, drop `((block refs))`. This is a
/// deliberately small helper — NOT the full `render_inline` (a faithful AST-driven
/// strip will come with the lsdoc migration); it only needs to read well enough to
/// match and preview.
fn strip_inline_markup(line: &str) -> String {
    let mut s = line.trim_start();
    let h = s.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&h) && s.as_bytes().get(h) == Some(&b' ') {
        s = s[h + 1..].trim_start();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    'scan: while i < s.len() {
        let rest = &s[i..];
        // [[Page]] → Page
        if let Some(after) = rest.strip_prefix("[[") {
            if let Some(end) = after.find("]]") {
                out.push_str(after[..end].trim());
                i += 2 + end + 2;
                continue;
            }
        }
        // ((uuid)) → dropped (an opaque id reads as noise)
        if let Some(after) = rest.strip_prefix("((") {
            if let Some(end) = after.find("))") {
                i += 2 + end + 2;
                continue;
            }
        }
        // ![alt](url) → alt ; [label](url) → label (paren-balanced URLs survive)
        if rest.starts_with("![") {
            if let Some((alt, _url, len)) = read_bracket_link(&rest[1..]) {
                out.push_str(alt);
                i += 1 + len;
                continue;
            }
        }
        if rest.starts_with('[') {
            if let Some((label, _url, len)) = read_bracket_link(rest) {
                out.push_str(label);
                i += len;
                continue;
            }
        }
        // emphasis / code markers → dropped (keep the inner text)
        for d in ["**", "__", "*", "`", "_"] {
            if rest.starts_with(d) {
                i += d.len();
                continue 'scan;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Plain text of a whole block's body (its displayed lines, no property /
/// SCHEDULED / DEADLINE lines), markup-stripped and space-joined — the unit that
/// goes into the search index. Empty for structural-only blocks.
fn block_plain_text(raw: &str) -> String {
    let mut out = String::new();
    for l in raw.lines() {
        let t = l.trim();
        if t.is_empty() || is_property_line(l) || t.starts_with("SCHEDULED:") || t.starts_with("DEADLINE:")
        {
            continue;
        }
        let stripped = strip_inline_markup(t);
        let stripped = stripped.trim();
        if stripped.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(stripped);
    }
    out
}

fn render_block(
    b: &DocBlock,
    out: &mut String,
    refs: &RefIndex,
    slug: &str,
    title: &str,
    counter: &mut u32,
    blocks: &mut Vec<serde_json::Value>,
) {
    // strip property / SCHEDULED / DEADLINE lines from the displayed body
    let lines: Vec<&str> = b
        .raw
        .lines()
        .filter(|l| {
            let t = l.trim();
            !is_property_line(l) && !t.starts_with("SCHEDULED:") && !t.starts_with("DEADLINE:")
        })
        .collect();
    let first = lines.first().copied().unwrap_or("");
    // Every block gets a stable anchor so a search hit can deep-link straight to it:
    // its `id::` uuid when present, else a generated per-page `b{n}` (these never
    // collide with 36-char uuids). Emitting the `<li id>` and recording the search
    // index entry in the SAME place keeps the HTML anchor and the index in lock-step.
    let anchor = match block_id(&b.raw) {
        Some(id) => id,
        None => {
            let a = format!("b{}", *counter);
            *counter += 1;
            a
        }
    };
    out.push_str(&format!("<li id=\"{anchor}\">"));
    let text = block_plain_text(&b.raw);
    if !text.is_empty() {
        blocks.push(json!({"slug": slug, "title": title, "anchor": anchor, "text": text}));
    }
    // heading?
    let hashes = first.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && first[hashes..].starts_with(' ') {
        out.push_str(&format!("<h{0}>{1}</h{0}>", hashes, render_inline(&first[hashes + 1..], refs)));
    } else {
        out.push_str(&format!("<div class=\"b\">{}</div>", render_inline(first, refs)));
    }
    for line in lines.iter().skip(1) {
        out.push_str(&format!("<div class=\"b\">{}</div>", render_inline(line, refs)));
    }
    if !b.children.is_empty() {
        out.push_str("<ul>");
        for c in &b.children {
            render_block(c, out, refs, slug, title, counter, blocks);
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
    refs: &RefIndex,
    blocks: &mut Vec<serde_json::Value>,
) -> String {
    let mut body = String::new();
    body.push_str("<ul class=\"outline\">");
    let mut counter = 0u32;
    for b in &doc.roots {
        render_block(b, &mut body, refs, slug, title, &mut counter, blocks);
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
<link rel=\"stylesheet\" href=\"style.css\">{katex}</head><body>\
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
        main = main,
    )
}

// KaTeX (from CDN) typesets the `\(..\)` / `\[..\]` math emitted by render_inline,
// client-side in the published pages. mhchem (\ce{…}) must register before
// auto-render runs; `defer` preserves script order, so auto-render's onload fires
// only after katex.min.js and mhchem have executed. Math therefore typesets when
// the page is viewed online; an offline viewer shows the raw TeX.
const KATEX_HEAD: &str = r#"<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/katex.min.css"><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/katex.min.js"></script><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/contrib/mhchem.min.js"></script><script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.47/dist/contrib/auto-render.min.js" onload="renderMathInElement(document.body,{delimiters:[{left:'\\[',right:'\\]',display:true},{left:'\\(',right:'\\)',display:false}],throwOnError:false})"></script>"#;

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
a.ref,a.tag{color:var(--link);text-decoration:none}
a.ref:hover,a.tag:hover{text-decoration:underline}
a.block-ref,span.block-ref{background:var(--code);border-radius:4px;padding:0 .28em;font-size:.95em}
a.block-ref{color:var(--link);text-decoration:none}
a.block-ref:hover{text-decoration:underline}
span.block-ref{color:var(--muted)}
a.tag{font-size:.92em}
a[href^="http"]{color:var(--link)}
code{background:var(--code);border-radius:4px;padding:.05em .35em;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.9em}
img{max-width:100%;border-radius:6px;margin:.3rem 0}
.math-display{display:block;text-align:center;margin:.5rem 0}
strong{font-weight:650}
.index-list li{margin:.15rem 0}
.index-list .k{color:var(--muted);font-size:.8rem;margin-left:.4rem}
hr{border:none;border-top:1px solid var(--line);margin:1.2rem 0}
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
    fs::write(out.join("fuse.min.js"), include_str!("../assets/fuse.min.js"))?;
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
        let Ok(content) = fs::read_to_string(&e.path) else { continue };
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
    for (name, slug, kind, parsed) in &public {
        let file = format!("{slug}.html");
        fs::write(out.join(&file), page_html(name, slug, parsed, *kind, &refs, &mut all_blocks))?;
        let journal = *kind == PageKind::Journal;
        let tag = if journal { "<span class=\"k\">journal</span>" } else { "" };
        index_list.push_str(&format!("<li><a class=\"ref\" href=\"{}\">{}</a>{}</li>", file, esc(name), tag));
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

    #[test]
    fn inline_html() {
        let h = render_inline("see [[Foo Bar]] and **bold** and `x` and #tag", &no_refs());
        assert!(h.contains("<a class=\"ref\" href=\"foo-bar.html\">Foo Bar</a>"));
        assert!(h.contains("<strong>bold</strong>"));
        assert!(h.contains("<code>x</code>"));
        assert!(h.contains("<a class=\"tag\" href=\"tag.html\">#tag</a>"));
    }

    #[test]
    fn escapes_html() {
        assert!(render_inline("a < b & c", &no_refs()).contains("a &lt; b &amp; c"));
    }

    #[test]
    fn math_emits_katex_delimiters() {
        // Inline $..$ → \(..\); display $$..$$ → \[..\]; both protected from the
        // emphasis/code rules and left for KaTeX auto-render to typeset.
        let h = render_inline(r"Euler $e^{i\pi}+1=0$ and $$\int_0^1 x\,dx$$", &no_refs());
        assert!(h.contains(r#"<span class="math">\(e^{i\pi}+1=0\)</span>"#), "{h}");
        assert!(
            h.contains(r#"<span class="math math-display">\[\int_0^1 x\,dx\]</span>"#),
            "{h}"
        );
        // Underscores inside math must NOT become italics.
        assert!(!render_inline(r"$a_1 + b_2$", &no_refs()).contains("<em>"));
    }

    #[test]
    fn block_refs_resolve_and_paren_urls_survive() {
        let mut refs = RefIndex::new();
        refs.insert(
            "5cfb2cc4-2f18-4b6e-b4c0-dcf657179204".into(),
            RefTarget { slug: "related-work".into(), text: "Related Work section".into() },
        );
        // Labeled block ref → a link to the target block's anchor, showing the label.
        let h = render_inline("see [Related Work](((5cfb2cc4-2f18-4b6e-b4c0-dcf657179204)))", &refs);
        assert!(
            h.contains(r#"<a class="ref block-ref" href="related-work.html#5cfb2cc4-2f18-4b6e-b4c0-dcf657179204">Related Work</a>"#),
            "{h}"
        );
        // Bare block ref → the target's text, linked.
        let b = render_inline("((5cfb2cc4-2f18-4b6e-b4c0-dcf657179204))", &refs);
        assert!(b.contains("related-work.html#5cfb2cc4-2f18-4b6e-b4c0-dcf657179204"), "{b}");
        assert!(b.contains("Related Work section"), "{b}");
        // Unresolved ref → muted text, no broken link / no stray `))`.
        let u = render_inline("[X](((deadbeef-0000-0000-0000-000000000000)))", &refs);
        assert!(u.contains(r#"<span class="block-ref">X</span>"#), "{u}");
        assert!(!u.contains("((deadbeef"), "{u}");
        // A real URL with parentheses is captured whole (no truncation at first ')').
        let w = render_inline("[wiki](https://en.wikipedia.org/wiki/Foo_(bar))", &no_refs());
        assert!(w.contains(r#"<a href="https://en.wikipedia.org/wiki/Foo_(bar)">wiki</a>"#), "{w}");
    }

    #[test]
    fn strips_markup_for_search_index() {
        // headings, emphasis/code, [[wiki]], [label](url), ![alt](url), ((ref)).
        assert_eq!(strip_inline_markup("## Heading **bold** _it_ `c`"), "Heading bold it c");
        assert_eq!(strip_inline_markup("see [[Foo Bar]] and [lbl](http://x)"), "see Foo Bar and lbl");
        assert_eq!(strip_inline_markup("img ![cat](cat.png) end"), "img cat end");
        // labeled block ref keeps the label, drops the uuid; bare ref drops entirely.
        assert_eq!(strip_inline_markup("[See](((1234)))"), "See");
        assert_eq!(strip_inline_markup("ref ((1111-2222)) gone"), "ref  gone");
    }

    #[test]
    fn block_text_drops_props_and_scheduling() {
        // `raw` is the dedented block body (no leading bullet). Property / SCHEDULED /
        // DEADLINE lines are not searchable content; the rest is markup-stripped.
        let raw = "task **important** [[Page]]\nSCHEDULED: <2026-01-01 Thu>\nid:: 1111\nkey:: val\ncontinued bit";
        assert_eq!(block_plain_text(raw), "task important Page continued bit");
        // structural-only block → empty (won't be indexed)
        assert_eq!(block_plain_text("id:: abc\ncollapsed:: true"), "");
    }

    #[test]
    fn publish_emits_sidebar_search_and_block_anchors() {
        let dir = std::env::temp_dir().join(format!("tine-publish-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(dir.join("logseq").join("config.edn"), "{:favorites [\"Alpha\"]}\n").unwrap();
        fs::write(
            dir.join("pages").join("Alpha.md"),
            "public:: true\n- # Intro to [[Beta]] and **bold** text\n  id:: 11111111-1111-1111-1111-111111111111\n- a unique searchwidget term\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("Beta.md"), "public:: true\n- linking back to [[Alpha]]\n").unwrap();
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
        assert!(sidx.starts_with("window.__tinePages="), "{}", &sidx[..60.min(sidx.len())]);
        assert!(sidx.contains("window.__tineBlocks="));
        assert!(sidx.contains("\"favorite\":true"), "Alpha is a favorite: {sidx}");
        assert!(sidx.contains("searchwidget"), "block content indexed");
        assert!(sidx.contains("Intro to Beta and bold text"), "markup stripped in index: {sidx}");
        assert!(!sidx.contains("[[Beta]]"), "no raw wiki brackets in index");

        // page html: EVERY block carries an anchor (id:: uuid or generated b{n}); the
        // sidebar + scripts are present; no anchorless <li>.
        let alpha = fs::read_to_string(out.join("alpha.html")).unwrap();
        assert!(alpha.contains("id=\"11111111-1111-1111-1111-111111111111\""), "id:: anchor kept");
        assert!(alpha.contains("id=\"b0\""), "id-less block got a generated anchor: {alpha}");
        assert!(!alpha.contains("<li>"), "no anchorless <li>");
        assert!(alpha.contains("<aside class=\"sidebar\">"), "sidebar present");
        assert!(alpha.contains("id=\"tine-search\""), "search box present");
        assert!(alpha.contains("src=\"app.js\""), "app.js linked");

        // index lists public pages, excludes the private one, and uses the shell.
        let index = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(index.contains("alpha.html") && index.contains("beta.html"));
        assert!(!index.contains("secret.html"), "private page excluded");
        assert!(index.contains("<aside class=\"sidebar\">"), "index uses the sidebar shell");

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
        fs::write(
            dir.join("journals").join("2026_06_28.md"),
            "- Worked on the **published export**: sidebar + search.\n- Linked [[Parameterized IP]].\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        let (outdir, count) = publish_graph(&g).unwrap();
        println!("SAMPLE_EXPORT_DIR={outdir} pages={count}");
    }
}
