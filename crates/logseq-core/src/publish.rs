//! Static HTML export: render the whole graph to a folder of linked HTML pages
//! plus an index. Inline formatting, nested block lists, and `[[page]]` links
//! become real anchors between the generated files.

use crate::doc::{self, DocBlock};
use crate::model::{Graph, PageKind};
use std::fs;
use std::io;
use std::path::Path;

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

fn page_html(title: &str, doc: &doc::Document) -> String {
    let mut body = String::new();
    body.push_str("<ul class=\"outline\">");
    for b in &doc.roots {
        render_block(b, &mut body);
    }
    body.push_str("</ul>");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
<link rel=\"stylesheet\" href=\"style.css\"></head><body>\
<a class=\"home\" href=\"index.html\">← index</a><h1>{}</h1>{}</body></html>",
        esc(title),
        esc(title),
        body
    )
}

const STYLE: &str = "body{font-family:Inter,system-ui,sans-serif;max-width:760px;margin:2rem auto;padding:0 1rem;color:#2b2b2b;line-height:1.6}\
h1{font-size:1.8rem}ul.outline,ul.outline ul{list-style:none}ul.outline{padding-left:0}ul.outline ul{padding-left:1.2rem;border-left:1px solid #eee}\
li{margin:2px 0}.b{}a.ref,a.tag{color:#1f6fd0;text-decoration:none}a.ref:hover,a.tag:hover{text-decoration:underline}\
code{background:#f1f1f1;border-radius:4px;padding:0 4px;font-family:monospace}a.home{color:#888;font-size:.85rem;text-decoration:none}\
img{max-width:100%}";

/// Export the whole graph to `<root>/publish/`. Returns (output dir, page count).
pub fn publish_graph(graph: &Graph) -> io::Result<(String, usize)> {
    let out = graph.root.join("publish");
    fs::create_dir_all(&out)?;
    fs::write(out.join("style.css"), STYLE)?;

    let pages = graph.list_pages();
    let mut index = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Index</title>\
<link rel=\"stylesheet\" href=\"style.css\"></head><body><h1>Pages</h1><ul class=\"outline\">",
    );
    let mut count = 0;
    let mut entries: Vec<_> = pages.iter().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    for e in entries {
        let Ok(content) = fs::read_to_string(&e.path) else { continue };
        let parsed = doc::parse(&content);
        let file = format!("{}.html", slug(&e.name));
        fs::write(out.join(&file), page_html(&e.name, &parsed))?;
        let tag = if e.kind == PageKind::Journal { " (journal)" } else { "" };
        index.push_str(&format!("<li><a href=\"{}\">{}</a>{}</li>", file, esc(&e.name), tag));
        count += 1;
    }
    index.push_str("</ul></body></html>");
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
