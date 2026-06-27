//! Static HTML export: render the whole graph to a folder of linked HTML pages
//! plus an index. Inline formatting, nested block lists, and `[[page]]` links
//! become real anchors between the generated files.

use crate::doc::{self, DocBlock};
use crate::model::{Graph, PageKind};
use crate::refs::{as_block_ref, block_id, read_bracket_link};
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

fn render_block(b: &DocBlock, out: &mut String, refs: &RefIndex) {
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
    // A block with an `id::` gets an anchor so `((block refs))` can link to it.
    match block_id(&b.raw) {
        Some(id) => out.push_str(&format!("<li id=\"{id}\">")),
        None => out.push_str("<li>"),
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
            render_block(c, out, refs);
        }
        out.push_str("</ul>");
    }
    out.push_str("</li>");
}

fn page_html(title: &str, doc: &doc::Document, kind: PageKind, refs: &RefIndex) -> String {
    let mut body = String::new();
    body.push_str("<ul class=\"outline\">");
    for b in &doc.roots {
        render_block(b, &mut body, refs);
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
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title>\
<link rel=\"stylesheet\" href=\"style.css\">{}</head><body>\
<a class=\"home\" href=\"index.html\">\u{2190} index</a>{}{}</body></html>",
        esc(title),
        KATEX_HEAD,
        heading,
        body
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
  background:var(--bg);color:var(--fg);max-width:740px;margin:0 auto;padding:48px 24px 96px;line-height:1.6;
  -webkit-font-smoothing:antialiased;font-size:16px}
a.home{color:var(--muted);font-size:.82rem;text-decoration:none}
a.home:hover{color:var(--fg)}
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
    let all_public = graph.config.all_pages_public;

    let pages = graph.list_pages();
    let mut index = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Index</title>\
<link rel=\"stylesheet\" href=\"style.css\"></head><body><h1 class=\"page\">Pages</h1><ul class=\"outline index-list\">",
    );
    let mut count = 0;
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

    // Pass 2: render each public page with the ref index, and list it on the index.
    for (name, slug, kind, parsed) in &public {
        let file = format!("{slug}.html");
        fs::write(out.join(&file), page_html(name, parsed, *kind, &refs))?;
        let tag = if *kind == PageKind::Journal { "<span class=\"k\">journal</span>" } else { "" };
        index.push_str(&format!("<li><a class=\"ref\" href=\"{}\">{}</a>{}</li>", file, esc(name), tag));
        count += 1;
    }
    index.push_str("</ul><footer>Published with Tine</footer></body></html>");
    fs::write(out.join("index.html"), index)?;
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
}
