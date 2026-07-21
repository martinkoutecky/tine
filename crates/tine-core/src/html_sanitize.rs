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
    // OG 6e7afa8eb's raw-HTML DOMPurify path preserves native playback elements
    // (src/main/frontend/security.cljs:5-11 and
    // src/main/frontend/components/block.cljs:3258-3261). Keep this bounded to
    // media, not embedded browsing/plugin contexts such as iframe/object/embed.
    "audio",
    "video",
    "source",
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
    // User-driven playback plus inert playback state/fetch/layout metadata.
    // `autoplay` remains absent; `src`/`poster` are checked by ammonia against
    // the safe scheme set below. Mirror RAW_HTML_ATTRS in the live renderer.
    tag_attrs.insert(
        "audio",
        ["src", "controls", "loop", "muted", "preload"]
            .into_iter()
            .collect(),
    );
    tag_attrs.insert(
        "video",
        [
            "src", "controls", "loop", "muted", "preload", "poster", "width", "height",
        ]
        .into_iter()
        .collect(),
    );
    tag_attrs.insert("source", ["src", "type"].into_iter().collect());
    tag_attrs.insert("details", ["open"].into_iter().collect());

    // `data:` stays allowed for media payloads (base64-embedded images etc.) —
    // DOMPurify (= OG's sanitizer, security.cljs:5-11) permits `data:` on its
    // DATA_URI_TAGS (img/audio/video/source). `javascript:` was never admitted.
    // OG/DOMPurify do NOT allow `data:` in link hrefs, so strip it there via
    // the attribute filter below (this tightens a pre-existing looseness where
    // the global scheme set let `<a href="data:...">` through).
    let schemes: HashSet<&str> = ["https", "http", "mailto", "tel", "data"]
        .into_iter()
        .collect();

    Builder::default()
        .tags(tags)
        .generic_attributes(generic)
        .tag_attributes(tag_attrs)
        .url_schemes(schemes)
        .attribute_filter(|element, attribute, value| {
            let is_data = value
                .trim_start_matches(|c: char| c.is_ascii_control() || c == ' ')
                .to_ascii_lowercase()
                .starts_with("data:");
            // DOMPurify permits data: only in media-tag data-URI attributes
            // (src); it strips data: from link hrefs and from `poster`. Mirror
            // both exclusions so live render and static export agree.
            let deny = (element == "a" && attribute == "href") || attribute == "poster";
            if deny && is_data {
                None
            } else {
                Some(value.into())
            }
        })
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

    #[test]
    fn native_media_policy_matches_the_live_renderer() {
        let out = sanitize(
            r#"<audio controls loop muted preload="metadata" src="https://media.example/audio.ogg">
                 <source src="https://media.example/audio.opus" type="audio/ogg">
               </audio>
               <video controls loop muted preload="none" poster="https://media.example/poster.jpg"
                      width="640" height="360" src="https://media.example/video.mp4">
                 <source src="https://media.example/video.webm" type="video/webm">
               </video>"#,
        );

        for needle in [
            "<audio",
            "<video",
            "<source",
            "controls",
            "loop",
            "muted",
            "preload=\"metadata\"",
            "poster=\"https://media.example/poster.jpg\"",
            "width=\"640\"",
            "height=\"360\"",
            "type=\"audio/ogg\"",
            "type=\"video/webm\"",
        ] {
            assert!(out.contains(needle), "expected {needle:?} in {out:?}");
        }
    }

    #[test]
    fn executable_media_and_external_request_primitives_stay_dead() {
        let out = sanitize(
            r#"<script>steal()</script>
               <img src="https://media.example/x.png" onerror="steal()">
               <audio autoplay src="javascript:steal()"></audio>
               <video autoplay poster="data:image/png;base64,AAAA" src="data:video/mp4;base64,AAAA"></video>
               <source src="javascript:steal()" type="video/mp4">
               <iframe src="https://evil.example"></iframe>
               <object data="https://evil.example"></object>
               <embed src="https://evil.example">"#,
        );

        for needle in [
            "<script",
            "onerror",
            "autoplay",
            "javascript:",
            "<iframe",
            "<object",
            "<embed",
        ] {
            assert!(
                !out.contains(needle),
                "did not expect {needle:?} in {out:?}"
            );
        }
        // data: in media `src` survives (DOMPurify/OG DATA_URI_TAGS); data: in
        // `poster` is stripped, matching DOMPurify's live-render behavior.
        assert!(!out.contains("data:image"), "poster kept: {out:?}");
        assert!(out.contains("data:video/mp4;base64,AAAA"), "src: {out:?}");
    }

    #[test]
    fn data_href_on_links_is_stripped_like_dompurify() {
        let out = sanitize(r#"<a href="data:text/html,<script>steal()</script>">x</a>"#);
        assert!(!out.contains("data:"), "{out:?}");
        assert!(out.contains(">x</a>"), "{out:?}");
    }
}
