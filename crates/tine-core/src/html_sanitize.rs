//! Raw-HTML sanitizer for the static-HTML export.
//!
//! Inventories live in `fixtures/html-sanitize-policy.json` (I-12). The browser
//! adapter uses global attributes; native export retains tag-scoped attributes,
//! explicit schemes and link rel. Shared safety outcomes are fixture-tested.
//! Media tags (`audio`/`video`/`source`) follow OG 6e7afa8eb's raw-HTML DOMPurify path
//! (src/main/frontend/security.cljs:5-11, components/block.cljs:3258-3261); keep the set
//! bounded to media, never iframe/object/embed. `autoplay` stays absent.
//! `style` keeps only colour declarations (`color_only_style`, which the app
//! calls through wasm), so an export shows the colours the app shows and
//! nothing that positions, hides or fetches. Every kept `href` obeys
//! `safe_export_url`, the rule export applies to Markdown links.

use ammonia::Builder;
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tine_outline::editor_text::js_trim;

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
const POLICY_JSON: &str = include_str!("../../../fixtures/html-sanitize-policy.json");
fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(POLICY_JSON).expect("checked-in HTML sanitizer policy")
    })
}
// The policy's `browser.styleProperties` as `STYLE_PROPERTIES`, compiled in
// (`build.rs`) so the app's wasm build carries neither the policy JSON nor a
// reader for it.
include!(concat!(env!("OUT_DIR"), "/style_properties.rs"));

/// The export's one rule for where a note-authored link may navigate (Markdown
/// links in `decorate`, raw-HTML and Hiccup anchors in `sanitize`). External
/// links are limited to ordinary web/contact schemes; graph assets and local
/// export links may use a relative path or fragment. Scheme-like and
/// network-path references do not fall through to the relative case.
pub fn safe_export_url(url: &str) -> Option<&str> {
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

/// JavaScript's `\s` as a regex class body: ECMAScript's WhiteSpace and
/// LineTerminator code points (ECMA-262 §12.2, §12.3), the set
/// `editor_text::is_js_whitespace` answers and `String.prototype.trim` strips (U+FEFF in,
/// U+0085 out). The app's colour rule was written in JavaScript; Rust's
/// `str::trim` and regex `\s` use Unicode White_Space, which differs on both.
/// A test checks this class against that set for every char.
const ECMASCRIPT_WHITESPACE_CLASS: &str =
    r"\t\n\x0B\x0C\r \xA0\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// The colour declarations of a `style` value, serialized `prop: value; …`;
/// "" if none, the one rule for both the app (through the wasm export
/// `color_only_style`) and export: the property must be one of the policy's
/// `browser.styleProperties`, and the value one closed colour literal (no
/// url(), var(), calc(), image-set() or `!important`).
///
/// Every step reproduces the app's former JavaScript rule (`colorOnlyStyle`,
/// kept as the oracle in `src/render/htmlSanitize.test.tsx`): split on `;`,
/// split at the first `:`, ECMAScript trim, Unicode lowercase of the property
/// (`toLowerCase`), and patterns that match like a JavaScript regex without
/// the `u` flag: `\s` is the ECMAScript set and `i` folds ASCII letters only
/// (`[a-zA-Z]`, `(?i-u)`; Rust's Unicode folding would let `ſ` and the Kelvin
/// sign match `s` and `k`).
pub fn color_only_style(value: &str) -> String {
    static COLOR_VALUE: OnceLock<Regex> = OnceLock::new();
    static FUNCTION_BANNED: OnceLock<Regex> = OnceLock::new();
    let color = COLOR_VALUE.get_or_init(|| {
        // JavaScript's `/i` without `u`: ASCII letters in either case, spelled out.
        let ws = ECMASCRIPT_WHITESPACE_CLASS;
        Regex::new(&format!(
            r"^(?:#(?:[0-9a-fA-F]{{3,4}}|[0-9a-fA-F]{{6}}|[0-9a-fA-F]{{8}})|[a-zA-Z]{{3,30}}|(?i-u:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch)\([{ws}]*[-+0-9a-zA-Z.%/,{ws}]{{1,80}}\))$"
        ))
        .expect("colour literal pattern")
    });
    let banned = FUNCTION_BANNED.get_or_init(|| {
        Regex::new(r"(?i-u)var|calc|url|env|attr|image|min|max|clamp").expect("banned functions")
    });
    let mut kept = String::new();
    for declaration in value.split(';') {
        let Some((property, v)) = declaration.split_once(':') else {
            continue;
        };
        let property = js_trim(property).to_lowercase();
        let v = js_trim(v);
        if !STYLE_PROPERTIES.contains(&property.as_str()) || !color.is_match(v) {
            continue;
        }
        // A function body may contain letters (`from`, `none`) but never a nested call.
        if v.find('(').is_some_and(|open| banned.is_match(&v[open..])) {
            continue;
        }
        if !kept.is_empty() {
            kept.push_str("; ");
        }
        kept.push_str(&property);
        kept.push_str(": ");
        kept.push_str(v);
    }
    kept
}

/// Sanitize a raw-HTML fragment to the shared allowlist. Event handlers,
/// `<iframe>`/`<script>`, and `javascript:` URIs are stripped; `style` keeps
/// only colour declarations; `href`/`src` are limited to safe schemes.
pub fn sanitize(html: &str) -> String {
    let policy = policy();
    let tags: HashSet<&str> = policy.tags.iter().map(String::as_str).collect();
    let generic = policy
        .native
        .generic_attributes
        .iter()
        .map(String::as_str)
        .chain(["style"])
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
            if attribute == "style" {
                let kept = color_only_style(value);
                return (!kept.is_empty()).then(|| kept.into());
            }
            // A link keeps its destination only under the export's link rule; an
            // anchor that fails keeps its text, as a Markdown link does.
            if attribute == "href" {
                return safe_export_url(value).map(|_| value.into());
            }
            let start = value.trim_start_matches(|c: char| c.is_ascii_control() || c == ' ');
            let is_data = start.to_ascii_lowercase().starts_with("data:");
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

    /// The colour rule's `\s` class is the outline's one JavaScript
    /// whitespace set, for every char.
    #[test]
    fn whitespace_class_is_the_js_whitespace_set() {
        let class = regex::Regex::new(&format!("^[{}]$", super::ECMASCRIPT_WHITESPACE_CLASS))
            .expect("class");
        let mut buf = [0u8; 4];
        let differ: Vec<u32> = (0..=0x10FFFFu32)
            .filter_map(char::from_u32)
            .filter(|&c| {
                class.is_match(c.encode_utf8(&mut buf))
                    != tine_outline::editor_text::is_js_whitespace(c)
            })
            .map(u32::from)
            .collect();
        assert!(differ.is_empty(), "class and set differ at {differ:X?}");
    }

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
    fn network_path_and_backslash_links_lose_their_href() {
        for href in [
            "//evil.example/x",
            " //evil.example",
            "\\\\evil.example",
            "/\\evil.example",
        ] {
            let out = sanitize(&format!("<a href=\"{href}\">x</a>"));
            assert!(!out.contains("href="), "{href:?}: {out:?}");
            assert!(out.contains(">x</a>"), "{href:?}: {out:?}");
        }
        for href in [
            "https://ok.example/",
            "page.html#b1",
            "#b1",
            "../assets/a.pdf",
        ] {
            let out = sanitize(&format!("<a href=\"{href}\">x</a>"));
            assert!(
                out.contains(&format!("href=\"{href}\"")),
                "{href:?}: {out:?}"
            );
        }
        // Media sources are unchanged: the export's link rule is about navigation.
        assert!(
            sanitize("<img src=\"//cdn.example/a.png\">").contains("src=\"//cdn.example/a.png\"")
        );
    }

    #[test]
    fn data_href_on_links_is_stripped_like_dompurify() {
        let out = sanitize(r#"<a href="data:text/html,<script>steal()</script>">x</a>"#);
        assert!(!out.contains("data:"), "{out:?}");
        assert!(out.contains(">x</a>"), "{out:?}");
    }
}
