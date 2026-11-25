//! Raw-HTML sanitizer for the static-HTML export.
//!
//! Inventories live in `fixtures/html-sanitize-policy.json` (I-12). The browser
//! adapter uses global attributes; native export retains tag-scoped attributes,
//! explicit schemes and link rel. Shared safety outcomes are fixture-tested.
//! Media tags (`audio`/`video`/`source`) follow OG 6e7afa8eb's raw-HTML DOMPurify path
//! (src/main/frontend/security.cljs:5-11, components/block.cljs:3258-3261); keep the set
//! bounded to media, never iframe/object/embed. `autoplay` stays absent.
//! `style` keeps only colour declarations (`color_only_style`, which the app
//! calls through wasm as the batched `color_only_styles`), so an export shows
//! the colours the app shows and nothing that positions, hides or fetches.
//! Every kept `href` obeys `safe_export_url`, the rule export applies to
//! Markdown links.

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
const POLICY_JSON: &str = include_str!("../../../fixtures/html-sanitize-policy.json");
fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(POLICY_JSON).expect("checked-in HTML sanitizer policy")
    })
}
/// The policy's `browser.styleProperties`, read on its own so the app's wasm
/// build (which calls only `color_only_styles`) carries no native-policy code.
fn style_properties() -> &'static [String] {
    #[derive(Deserialize)]
    struct Browser {
        #[serde(rename = "styleProperties")]
        style_properties: Vec<String>,
    }
    #[derive(Deserialize)]
    struct StyleOnly {
        browser: Browser,
    }
    static PROPERTIES: OnceLock<Vec<String>> = OnceLock::new();
    PROPERTIES.get_or_init(|| {
        serde_json::from_str::<StyleOnly>(POLICY_JSON)
            .expect("checked-in HTML sanitizer policy")
            .browser
            .style_properties
    })
}

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

/// ECMAScript's WhiteSpace and LineTerminator code points (ECMA-262
/// §12.2, §12.3): what JavaScript's `String.prototype.trim` strips and what
/// `\s` matches in a JavaScript regex. The app's colour rule was written in
/// JavaScript, so this set (U+FEFF in, U+0085 out) is the one it trims; Rust's
/// `str::trim` and regex `\s` use Unicode White_Space, which differs on both.
const ECMASCRIPT_WHITESPACE: &[char] = &[
    // WhiteSpace: TAB, VT, FF, ZWNBSP, and every Zs (space separator).
    '\u{0009}', '\u{000B}', '\u{000C}', '\u{FEFF}', '\u{0020}', '\u{00A0}', '\u{1680}', '\u{2000}',
    '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}',
    '\u{2009}', '\u{200A}', '\u{202F}', '\u{205F}', '\u{3000}',
    // LineTerminator: LF, CR, LINE SEPARATOR, PARAGRAPH SEPARATOR.
    '\u{000A}', '\u{000D}', '\u{2028}', '\u{2029}',
];

/// `s` without leading and trailing [`ECMASCRIPT_WHITESPACE`]; matches
/// ECMAScript `String.prototype.trim`.
fn ecmascript_trim(s: &str) -> &str {
    s.trim_matches(ECMASCRIPT_WHITESPACE)
}

/// [`ECMASCRIPT_WHITESPACE`] as a regex class body, JavaScript's `\s`.
#[cfg(test)]
fn ecmascript_whitespace_class() -> String {
    ECMASCRIPT_WHITESPACE
        .iter()
        .map(|c| format!("\\x{{{:X}}}", u32::from(*c)))
        .collect()
}

/// The colour functions a literal may use, matched ASCII-case-insensitively.
const COLOR_FUNCTIONS: &[&str] = &[
    "rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch",
];
/// Words a colour function's body may not contain (ASCII case folded): a nested call.
const COLOR_FUNCTION_BANNED: &[&str] = &[
    "var", "calc", "url", "env", "attr", "image", "min", "max", "clamp",
];

/// Whether `v` is one closed colour literal: the JavaScript pattern
/// `/^(?:#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})|[a-z]{3,30}|(?:rgb|…|oklch)\(\s*[-+0-9a-z.%/,\s]{1,80}\))$/i`
/// matched directly, in one pass over `v`. (The browser build's `regex` has no
/// lazy DFA, so this pattern's 80-fold Unicode class ran on the slow engine,
/// about 5 µs a value.) A function body is the text between the name's `(` and
/// the closing `)` ending `v`; `\s*` then 1-80 body characters, where `\s` is
/// itself a body character, holds exactly when the body is non-empty, all body
/// characters, and at most 80 of them follow its leading whitespace.
fn is_color_literal(v: &str) -> bool {
    let bytes = v.as_bytes();
    if let Some(hex) = v.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if bytes.iter().all(u8::is_ascii_alphabetic) {
        return (3..=30).contains(&bytes.len());
    }
    let Some(open) = v.find('(') else {
        return false;
    };
    let Some(body) = v[open + 1..].strip_suffix(')') else {
        return false;
    };
    if !COLOR_FUNCTIONS
        .iter()
        .any(|name| v[..open].eq_ignore_ascii_case(name))
    {
        return false;
    }
    let is_space = |c: char| ECMASCRIPT_WHITESPACE.contains(&c);
    let is_body = |c: char| c.is_ascii_alphanumeric() || "-+.%/,".contains(c) || is_space(c);
    let leading = body.chars().take_while(|&c| is_space(c)).count();
    let mut total = 0usize;
    for c in body.chars() {
        if !is_body(c) {
            return false;
        }
        total += 1;
    }
    total >= 1 && total - leading <= 80
}

/// The colour declarations of a `style` value, serialized `prop: value; …`;
/// "" if none, the one rule for both the app (through the wasm export
/// `color_only_styles`) and export: the property must be one of the policy's
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
    let mut kept = String::new();
    push_color_only_style(value, &mut kept);
    kept
}

/// [`color_only_style`] appended to `out`, with no allocation of its own (the
/// batch calls it once per value).
fn push_color_only_style(value: &str, out: &mut String) {
    let properties = style_properties();
    let start = out.len();
    for declaration in value.split(';') {
        let Some((property, v)) = declaration.split_once(':') else {
            continue;
        };
        let property = ecmascript_trim(property);
        // `toLowerCase`: for ASCII text, ASCII lowercase, without a copy.
        let matched = if property.is_ascii() {
            properties.iter().find(|p| {
                p.bytes()
                    .eq(property.bytes().map(|b| b.to_ascii_lowercase()))
            })
        } else {
            let lower = property.to_lowercase();
            properties.iter().find(|p| **p == lower)
        };
        let v = ecmascript_trim(v);
        let Some(property) = matched.filter(|_| is_color_literal(v)) else {
            continue;
        };
        // A function body may contain letters (`from`, `none`) but never a nested call.
        if v.find('(').is_some_and(|open| {
            let body = v[open..].as_bytes();
            COLOR_FUNCTION_BANNED.iter().any(|word| {
                body.windows(word.len())
                    .any(|w| w.eq_ignore_ascii_case(word.as_bytes()))
            })
        }) {
            continue;
        }
        if out.len() > start {
            out.push_str("; ");
        }
        out.push_str(property);
        out.push_str(": ");
        out.push_str(v);
    }
}

/// `color_only_style` for many values in one call, so the app crosses into
/// wasm once per sanitized fragment rather than once per `[style]` element.
/// `values` is every value concatenated and `lengths[i]` the length of value
/// `i` in UTF-16 code units (JavaScript's `length`): lengths, not a separator,
/// so no attribute value can shift a boundary. Returns the kept declarations
/// concatenated and their UTF-16 lengths, in input order. O(total bytes).
///
/// A JavaScript caller's values reach Rust as one UTF-8 string, where a lone
/// high surrogate ending one value and a lone low surrogate starting the next
/// become one character. Encoded alone, each half would be U+FFFD, so a
/// character straddling a boundary is read as U+FFFD on both sides: each value
/// gets the answer it gets through `color_only_style`. Lengths that overrun the
/// text read the missing part as empty (no panic: a panic aborts the instance).
pub fn color_only_styles(values: &str, lengths: &[u32]) -> (String, Vec<u32>) {
    let bytes = values.as_bytes();
    let mut kept = String::with_capacity(values.len() / 2);
    let mut kept_lengths = Vec::with_capacity(lengths.len());
    let mut pos = 0usize;
    // The second half of a character that straddled the previous boundary,
    // owed to the next non-empty value (a character has at most two units).
    let mut carry = false;
    for &length in lengths {
        let mut remaining = length as usize;
        let head = carry && remaining > 0;
        if head {
            remaining -= 1;
            carry = false;
        }
        let start = pos;
        let mut straddle = false;
        while remaining > 0 && pos < bytes.len() {
            // ASCII is one unit a byte: take a whole run at once.
            let run = remaining.min(bytes.len() - pos);
            if bytes[pos..pos + run].is_ascii() {
                pos += run;
                remaining -= run;
                continue;
            }
            let (width, units) = match bytes[pos] {
                0x00..=0x7F => (1, 1),
                0xC0..=0xDF => (2, 1),
                0xE0..=0xEF => (3, 1),
                _ => (4, 2),
            };
            pos += width;
            if units > remaining {
                straddle = true;
                carry = true;
                break;
            }
            remaining -= units;
        }
        let end = if straddle { pos - 4 } else { pos };
        let before = kept.len();
        if head || straddle {
            let mut value = String::new();
            if head {
                value.push('\u{FFFD}');
            }
            value.push_str(&values[start..end]);
            if straddle {
                value.push('\u{FFFD}');
            }
            push_color_only_style(&value, &mut kept);
        } else {
            push_color_only_style(&values[start..end], &mut kept);
        }
        let added = &kept[before..];
        let units = if added.is_ascii() {
            added.len()
        } else {
            added.encode_utf16().count()
        };
        kept_lengths.push(units as u32);
    }
    (kept, kept_lengths)
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

    /// Batch `values` (JavaScript strings, as UTF-16) the way the app does and
    /// compare with one `color_only_style` call per value, each value encoded on
    /// its own (lone surrogates become U+FFFD, as TextEncoder makes them).
    fn assert_batch_agrees(values: &[Vec<u16>]) {
        use super::{color_only_style, color_only_styles};
        let joined: Vec<u16> = values.concat();
        let lengths: Vec<u32> = values.iter().map(|v| v.len() as u32).collect();
        let (kept, kept_lengths) = color_only_styles(&String::from_utf16_lossy(&joined), &lengths);
        let kept: Vec<u16> = kept.encode_utf16().collect();
        assert_eq!(kept_lengths.len(), values.len());
        let mut at = 0;
        for (value, &n) in values.iter().zip(&kept_lengths) {
            let got = String::from_utf16_lossy(&kept[at..at + n as usize]);
            at += n as usize;
            assert_eq!(
                got,
                color_only_style(&String::from_utf16_lossy(value)),
                "{value:?}"
            );
        }
        assert_eq!(at, kept.len());
    }

    fn js(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn color_only_styles_matches_one_call_per_value() {
        let values: Vec<Vec<u16>> = [
            "color: red",
            "",
            "position:fixed;color:#c00",
            ";",
            "color:\u{feff}red;background-color:#0c0",
            "font-size: 9px",
            "color: rgb(1,\u{a0}2,\u{feff}3)",
            "color: red; x: \u{1F600}",
            "\u{1F600}",
            "color: red",
        ]
        .iter()
        .map(|s| js(s))
        .collect();
        assert_batch_agrees(&values);
        assert_batch_agrees(&[]);
        assert_batch_agrees(&[js("")]);
    }

    #[test]
    fn color_only_styles_reads_a_surrogate_pair_split_across_values_as_two_lone_halves() {
        let high = 0xD83D;
        let low = 0xDE00;
        let mut first = js("color: red;x");
        first.push(high);
        let mut last = vec![low];
        last.extend(js(";color: blue"));
        assert_batch_agrees(&[first.clone(), last.clone()]);
        // Through an empty value in between, the halves still meet in the joined text.
        assert_batch_agrees(&[first.clone(), vec![], last.clone()]);
        assert_batch_agrees(&[vec![high], vec![low]]);
        assert_batch_agrees(&[js("color:red"), vec![low], vec![high]]);
    }

    /// The pattern `is_color_literal` and the banned-word check replaced,
    /// as they were written (regex, `(?i-u)` = JavaScript `/i` without `u`).
    #[test]
    fn color_literal_matches_the_former_patterns() {
        use super::{
            ecmascript_whitespace_class, is_color_literal, COLOR_FUNCTION_BANNED,
            ECMASCRIPT_WHITESPACE,
        };
        let ws = ecmascript_whitespace_class();
        let color = regex::Regex::new(&format!(
            r"^(?:#(?:[0-9a-fA-F]{{3,4}}|[0-9a-fA-F]{{6}}|[0-9a-fA-F]{{8}})|[a-zA-Z]{{3,30}}|(?i-u:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch)\([{ws}]*[-+0-9a-zA-Z.%/,{ws}]{{1,80}}\))$"
        ))
        .unwrap();
        let banned = regex::Regex::new(r"(?i-u)var|calc|url|env|attr|image|min|max|clamp").unwrap();
        let mut spaces: Vec<String> = ECMASCRIPT_WHITESPACE
            .iter()
            .map(|c| c.to_string())
            .collect();
        spaces.extend(["\u{85}", "\u{200B}", "\u{180E}", "\0", "\u{1F600}"].map(String::from));
        let heads = [
            "",
            "#",
            "#c",
            "#c0",
            "#c00",
            "#c00c",
            "#c00c0",
            "#cc00cc",
            "#cc00cc0",
            "#cc00cc00",
            "#cc00cc00c",
            "#g00",
            "#C0C0C0",
            "re",
            "red",
            "RED",
            "rebeccapurple",
            "\u{17F}ilver",
            "blac\u{212A}",
            "rgb",
            "RGBA",
            "hsl",
            "hsla",
            "hwb",
            "lab",
            "lch",
            "oklab",
            "OkLch",
            "rgbx",
            "xrgb",
            "url",
            "r g b",
            "abcdefghijabcdefghijabcdefghij",
            "abcdefghijabcdefghijabcdefghijk",
        ];
        let bodies = [
            "",
            "1 2 3",
            "220, 38, 38",
            "1 2 3 / 50%",
            "from red h s l",
            "var(--x)",
            "calc(1) 2 3",
            "1)",
            "(1",
            "-+.%/,",
            "a_b",
            "1;2",
            "\u{17F}",
            "MIN 1",
            "Image",
        ];
        let mut inputs: Vec<String> = Vec::new();
        for h in heads {
            inputs.push(h.to_string());
            inputs.push(format!("{h})"));
            for b in bodies {
                inputs.push(format!("{h}({b})"));
                inputs.push(format!("{h}({b}"));
                inputs.push(format!("{h}({b}))"));
            }
            for w in &spaces {
                inputs.push(format!("{h}{w}"));
                inputs.push(format!("{h}({w}1{w}2{w})"));
                inputs.push(format!("{h}({w})"));
                inputs.push(format!("{h}({w}{w})"));
            }
            for n in [79, 80, 81] {
                inputs.push(format!("{h}({})", "1".repeat(n)));
                for w in ["\u{A0}", " "] {
                    inputs.push(format!("{h}({}{})", w.repeat(5), "1".repeat(n)));
                    inputs.push(format!("{h}({}{})", "1".repeat(n), w));
                    inputs.push(format!("{h}({})", w.repeat(n)));
                }
            }
        }
        let matching = inputs.iter().filter(|v| color.is_match(v)).count();
        assert!(
            matching > 100 && matching < inputs.len() / 2,
            "{matching} of {}",
            inputs.len()
        );
        let differ: Vec<&String> = inputs
            .iter()
            .filter(|v| color.is_match(v) != is_color_literal(v))
            .collect();
        assert!(differ.is_empty(), "{differ:?}");
        let new_banned = |v: &str| {
            let body = v.to_ascii_lowercase();
            COLOR_FUNCTION_BANNED.iter().any(|word| body.contains(word))
        };
        let differ: Vec<&String> = inputs
            .iter()
            .filter(|v| banned.is_match(v) != new_banned(v))
            .collect();
        assert!(differ.is_empty(), "{differ:?}");
    }

    #[test]
    fn color_only_styles_reads_overrun_lengths_as_empty() {
        let (kept, lengths) = super::color_only_styles("color:red", &[9, 4, 0]);
        assert_eq!(kept, "color: red");
        assert_eq!(lengths, [10, 0, 0]);
    }
}
