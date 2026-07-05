//! Raw-HTML sanitizer for the static-HTML export.
//!
//! This is the Rust half of a MIRRORED policy — the TypeScript half is
//! `src/render/htmlSanitize.ts` (DOMPurify, for the live app). lsdoc emits
//! `raw_html`/`inline_html` nodes with the source bytes verbatim (byte-parity
//! with mldoc); sanitizing is a render-layer safety decision applied at each
//! render boundary, not in the parser. The export re-publishes user HTML as
//! served content, so it needs the same allowlist the app enforces.
//!
//! `fixtures/html-sanitize-cases.json` contract-tests that this and the TS side
//! agree. Keep the two allowlists in lockstep.

use ammonia::Builder;
use std::collections::{HashMap, HashSet};

/// Mirror of `RAW_HTML_TAGS` in `src/render/htmlSanitize.ts`.
const TAGS: &[&str] = &[
    "b",
    "strong",
    "i",
    "em",
    "u",
    "ins",
    "del",
    "s",
    "strike",
    "sub",
    "sup",
    "mark",
    "kbd",
    "abbr",
    "small",
    "code",
    "cite",
    "q",
    "span",
    "br",
    "p",
    "div",
    "blockquote",
    "details",
    "summary",
    "a",
    "img",
];

/// Sanitize a raw-HTML fragment to the shared allowlist. Event handlers,
/// `style`, `<iframe>`/`<script>`, and `javascript:` URIs are stripped;
/// `href`/`src` are limited to safe schemes.
pub fn sanitize(html: &str) -> String {
    let tags: HashSet<&str> = TAGS.iter().copied().collect();

    // `class`/`title` on any tag; `href`/`src`/etc. only where they belong.
    let generic: HashSet<&str> = ["class", "title"].into_iter().collect();
    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attrs.insert("a", ["href"].into_iter().collect());
    tag_attrs.insert(
        "img",
        ["src", "alt", "width", "height"].into_iter().collect(),
    );
    tag_attrs.insert("details", ["open"].into_iter().collect());

    let schemes: HashSet<&str> = ["https", "http", "mailto", "tel", "data"]
        .into_iter()
        .collect();

    Builder::default()
        .tags(tags)
        .generic_attributes(generic)
        .tag_attributes(tag_attrs)
        .url_schemes(schemes)
        .link_rel(Some("noopener noreferrer"))
        .clean(html)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::sanitize;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        name: String,
        input: String,
        #[serde(rename = "mustContain")]
        must_contain: Vec<String>,
        #[serde(rename = "mustNotContain")]
        must_not_contain: Vec<String>,
    }
    #[derive(Deserialize)]
    struct Cases {
        cases: Vec<Case>,
    }

    // Same fixtures the TS/DOMPurify side runs — this asserts the two render
    // surfaces enforce the same policy.
    #[test]
    fn contract_fixtures() {
        let raw = include_str!("../../../fixtures/html-sanitize-cases.json");
        let parsed: Cases = serde_json::from_str(raw).expect("fixtures parse");
        for c in parsed.cases {
            let out = sanitize(&c.input);
            for needle in &c.must_contain {
                assert!(
                    out.contains(needle.as_str()),
                    "[{}] expected output to contain {needle:?}, got {out:?}",
                    c.name
                );
            }
            for needle in &c.must_not_contain {
                assert!(
                    !out.contains(needle.as_str()),
                    "[{}] expected output NOT to contain {needle:?}, got {out:?}",
                    c.name
                );
            }
        }
    }
}
