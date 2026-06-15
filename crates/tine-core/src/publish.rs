//! Static HTML export: render the whole graph to a folder of linked HTML pages
//! plus an index. Inline formatting, nested block lists, and `[[page]]` links
//! become real anchors between the generated files.

use crate::doc::{self, DocBlock};
use crate::model::{Graph, PageKind};
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

/// Render a single line of inline markdown to HTML.
fn render_inline(input: &str) -> String {
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
        // ![alt](url)
        if rest.starts_with("![") {
            if let (Some(open), Some(close)) = (rest.find("]("), rest.find(')')) {
                if open < close {
                    let alt = &rest[2..open];
                    let url = &rest[open + 2..close];
                    flush(&mut plain, &mut out);
                    out.push_str(&format!("<img alt=\"{}\" src=\"{}\">", esc(alt), esc(url)));
                    i += close + 1;
                    continue;
                }
            }
        }
        // [label](url)
        if rest.starts_with('[') {
            if let (Some(mid), Some(close)) = (rest.find("]("), rest.find(')')) {
                if mid < close {
                    let label = &rest[1..mid];
                    let url = &rest[mid + 2..close];
                    flush(&mut plain, &mut out);
                    out.push_str(&format!("<a href=\"{}\">{}</a>", esc(url), esc(label)));
                    i += close + 1;
                    continue;
                }
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
                        out.push_str(&format!("<{tag}>{}</{tag}>", render_inline(inner)));
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

fn render_block(b: &DocBlock, out: &mut String) {
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
    out.push_str("<li>");
    // heading?
    let hashes = first.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && first[hashes..].starts_with(' ') {
        out.push_str(&format!("<h{0}>{1}</h{0}>", hashes, render_inline(&first[hashes + 1..])));
    } else {
        out.push_str(&format!("<div class=\"b\">{}</div>", render_inline(first)));
    }
    for line in lines.iter().skip(1) {
        out.push_str(&format!("<div class=\"b\">{}</div>", render_inline(line)));
    }
    if !b.children.is_empty() {
        out.push_str("<ul>");
        for c in &b.children {
            render_block(c, out);
        }
        out.push_str("</ul>");
    }
    out.push_str("</li>");
}

fn page_html(title: &str, doc: &doc::Document, kind: PageKind) -> String {
    let mut body = String::new();
    body.push_str("<ul class=\"outline\">");
    for b in &doc.roots {
        render_block(b, &mut body);
    }
    body.push_str("</ul>");
    // Journal titles get a leading calendar glyph, like Logseq.
    let heading = if kind == PageKind::Journal {
        format!("<h1 class=\"page\"><span class=\"cal\">\u{1F4C5}</span>{}</h1>", esc(title))
    } else {
        format!("<h1 class=\"page\">{}</h1>", esc(title))
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title>\
<link rel=\"stylesheet\" href=\"style.css\"></head><body>\
<a class=\"home\" href=\"index.html\">\u{2190} index</a>{}{}</body></html>",
        esc(title),
        heading,
        body
    )
}

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
h1.page .cal{color:var(--muted);font-weight:400;margin-right:.4rem}
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
a.tag{font-size:.92em}
a[href^="http"]{color:var(--link)}
code{background:var(--code);border-radius:4px;padding:.05em .35em;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.9em}
img{max-width:100%;border-radius:6px;margin:.3rem 0}
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
    for e in entries {
        let Ok(content) = fs::read_to_string(&e.path) else { continue };
        let parsed = doc::parse(&content);
        if !all_public && !page_is_public(parsed.pre_block.as_deref()) {
            continue;
        }
        let file = format!("{}.html", slug(&e.name));
        fs::write(out.join(&file), page_html(&e.name, &parsed, e.kind))?;
        let tag = if e.kind == PageKind::Journal { "<span class=\"k\">journal</span>" } else { "" };
        index.push_str(&format!("<li><a class=\"ref\" href=\"{}\">{}</a>{}</li>", file, esc(&e.name), tag));
        count += 1;
    }
    index.push_str("</ul><footer>Published with Tine</footer></body></html>");
    fs::write(out.join("index.html"), index)?;
    Ok((out.display().to_string(), count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify() {
        assert_eq!(slug("Foo Bar"), "foo-bar");
        assert_eq!(slug("n-fold IP"), "n-fold-ip");
    }

    #[test]
    fn inline_html() {
        let h = render_inline("see [[Foo Bar]] and **bold** and `x` and #tag");
        assert!(h.contains("<a class=\"ref\" href=\"foo-bar.html\">Foo Bar</a>"));
        assert!(h.contains("<strong>bold</strong>"));
        assert!(h.contains("<code>x</code>"));
        assert!(h.contains("<a class=\"tag\" href=\"tag.html\">#tag</a>"));
    }

    #[test]
    fn escapes_html() {
        assert!(render_inline("a < b & c").contains("a &lt; b &amp; c"));
    }
}
