//! Raw-HTML sanitizer for the static-HTML export.
//!
//! Inventories live in `fixtures/html-sanitize-policy.json` (I-12). The browser
//! adapter uses global attributes; native export retains tag-scoped attributes,
//! explicit schemes and link rel. Shared safety outcomes are fixture-tested.
//! Media tags (`audio`/`video`/`source`) follow OG 6e7afa8eb's raw-HTML DOMPurify path
//! (src/main/frontend/security.cljs:5-11, components/block.cljs:3258-3261); keep the set
//! bounded to media, never iframe/object/embed. `autoplay` stays absent.

use ammonia::Builder;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativePolicy {
    generic_attributes: Vec<String>,
    tag_attributes: HashMap<String, Vec<String>>,
    url_schemes: Vec<String>,
    link_rel: String,
    deny_data_attributes: Vec<String>,
    deny_data_tag_attributes: HashMap<String, Vec<String>>,
}
#[derive(Deserialize)]
struct Policy {
    tags: Vec<String>,
    native: NativePolicy,
}
fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(include_str!("../../../fixtures/html-sanitize-policy.json"))
            .expect("checked-in HTML sanitizer policy")
    })
}

/// Sanitize a raw-HTML fragment to the shared allowlist. Event handlers,
/// `style`, `<iframe>`/`<script>`, and `javascript:` URIs are stripped;
/// `href`/`src` are limited to safe schemes.
pub fn sanitize(html: &str) -> String {
    let policy = policy();
    let tags: HashSet<&str> = policy.tags.iter().map(String::as_str).collect();
    let generic = policy
        .native
        .generic_attributes
        .iter()
        .map(String::as_str)
        .collect();
    let tag_attrs = policy
        .native
        .tag_attributes
        .iter()
        .map(|(tag, attrs)| (tag.as_str(), attrs.iter().map(String::as_str).collect()))
        .collect();
    let schemes = policy
        .native
        .url_schemes
        .iter()
        .map(String::as_str)
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
            let deny = policy
                .native
                .deny_data_attributes
                .iter()
                .any(|a| a == attribute)
                || policy
                    .native
                    .deny_data_tag_attributes
                    .get(element)
                    .is_some_and(|attrs| attrs.iter().any(|a| a == attribute));
            if deny && is_data {
                None
            } else {
                Some(value.into())
            }
        })
        .link_rel(Some(&policy.native.link_rel))
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
