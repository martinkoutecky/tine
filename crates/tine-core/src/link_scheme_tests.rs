//! The bound: every answer depends only on the first [`ROUTE_PREFIX_BYTES`]
//! bytes, so a caller may pass just that prefix; and on every input decided
//! within the bound the answers are the unbounded rule's (kept here, as the
//! reference only).

use super::*;

const TABLE: &str = include_str!("../../../fixtures/local-target-cases.json");

/// The longest prefix of at most the bound that ends on a character boundary
/// (what a caller slicing UTF-16 units hands over is at least this).
fn prefix(s: &str) -> &str {
    let mut end = s.len().min(ROUTE_PREFIX_BYTES);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The unbounded rule as it stood before the bound (reference only).
fn reference_prefix(dest: &str, compact: bool) -> Option<String> {
    let mut name = String::new();
    for &c in dest.as_bytes() {
        if c <= b' ' && (compact || name.is_empty() || matches!(c, b'\t' | b'\n' | b'\r')) {
            continue;
        }
        if c == b':' && !name.is_empty() {
            return Some(if name.len() == 1 { String::new() } else { name });
        }
        let valid = if name.is_empty() {
            c.is_ascii_alphabetic()
        } else {
            c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-')
        };
        if !valid {
            return None;
        }
        name.push(c.to_ascii_lowercase() as char);
    }
    None
}

fn reference_route(dest: &str) -> SchemeRoute {
    let Some(scheme) = reference_prefix(dest, false) else {
        return match reference_prefix(dest, true) {
            Some(s) if refused_scheme(&s) => SchemeRoute::Drop,
            _ => SchemeRoute::Relative,
        };
    };
    match scheme.as_str() {
        "" => SchemeRoute::Drive,
        s if refused_scheme(s) => SchemeRoute::Drop,
        "http" | "https" => SchemeRoute::Web,
        "mailto" => SchemeRoute::Mail,
        "file"
            if crate::local_target::js_trim(dest)
                .get(..5)
                .is_some_and(|p| p.eq_ignore_ascii_case("file:")) =>
        {
            SchemeRoute::File
        }
        _ => SchemeRoute::Defer(scheme),
    }
}

fn reference_preview(dest: &str) -> PreviewResource {
    match reference_prefix(dest, true).as_deref() {
        None | Some("") | Some("file") => PreviewResource::LocalCandidate,
        Some("http" | "https" | "data") => PreviewResource::Browser,
        Some(_) => PreviewResource::Drop,
    }
}

/// Every destination string the table holds, raw from its sources too.
fn table_inputs() -> Vec<String> {
    let table: serde_json::Value = serde_json::from_str(TABLE).unwrap();
    let mut out = Vec::new();
    for row in table["rows"].as_array().unwrap() {
        if let Some(dest) = row.get("dest").and_then(|d| d.as_str()) {
            out.push(dest.to_owned());
        }
        let source = row["source"].as_str().unwrap();
        out.push(source.to_owned());
        for (open, close) in [("](", ')'), ("[[", ']'), ("<", '>')] {
            if let Some(at) = source.find(open) {
                let rest = &source[at + open.len()..];
                out.push(rest[..rest.find(close).unwrap_or(rest.len())].to_owned());
            }
        }
    }
    out
}

fn generated_inputs() -> Vec<String> {
    let b = ROUTE_PREFIX_BYTES;
    let mut out = Vec::new();
    for n in [
        0,
        1,
        7,
        b - 12,
        b - 11,
        b - 10,
        b - 2,
        b - 1,
        b,
        b + 1,
        2 * b,
        20_000,
    ] {
        let a = "a".repeat(n);
        let sp = " ".repeat(n);
        let tabs = "\t\n".repeat(n / 2);
        out.extend([
            format!("https://example.test/x?q={a}"),
            format!("data:image/png;base64,{a}"),
            format!("docs/a b.pdf?{a}"),
            format!("{sp}https://example.test/"),
            format!("{sp}javascript:alert(1)"),
            format!("{tabs}javascript:alert(1)"),
            format!("java{sp}script:alert(1)"),
            format!("j{tabs}ava\u{1}script:x"),
            format!("{sp}file:/x"),
            format!("{sp}C:/x"),
            format!("{a}:x"),
            format!("{a}"),
            format!("x{a}.png"),
            format!("zotero://select/{a}"),
            format!("{sp}"),
            format!("{}é", "a".repeat(n)),
            format!("{}é:x", "a".repeat(n)),
            format!("é{a}"),
            format!("\u{a0}{sp}file:x"),
        ]);
    }
    out
}

#[test]
fn every_answer_reads_only_the_bounded_prefix() {
    let inputs: Vec<String> = table_inputs()
        .into_iter()
        .chain(generated_inputs())
        .collect();
    assert!(inputs.len() > 400);
    for s in &inputs {
        let p = prefix(s);
        assert_eq!(scheme_route(s), scheme_route(p), "{s:.80?}");
        assert_eq!(anchor_route_code(s), anchor_route_code(p), "{s:.80?}");
        assert_eq!(preview_resource(s), preview_resource(p), "{s:.80?}");
        assert_eq!(
            preview_resource_code(s),
            preview_resource_code(p),
            "{s:.80?}"
        );
        assert_eq!(link_scheme(s), link_scheme(p), "{s:.80?}");
    }
}

/// The bound changes no answer the rule decides within it, and refuses the
/// rest (a scheme or leading whitespace running past the bound).
#[test]
fn the_bound_keeps_every_decided_answer() {
    for s in table_inputs().iter().chain(&generated_inputs()) {
        let route = scheme_route(s);
        let preview = preview_resource(s);
        let undecided = s.len() >= ROUTE_PREFIX_BYTES
            && route == SchemeRoute::Drop
            && reference_route(s) != SchemeRoute::Drop;
        if !undecided {
            assert_eq!(route, reference_route(s), "{s:.80?}");
        }
        if !(s.len() >= ROUTE_PREFIX_BYTES && preview == PreviewResource::Drop) {
            assert_eq!(preview, reference_preview(s), "{s:.80?}");
        }
    }
    // Every table destination is decided within the bound.
    for s in table_inputs() {
        assert_eq!(scheme_route(&s), reference_route(&s), "{s}");
        assert_eq!(preview_resource(&s), reference_preview(&s), "{s}");
    }
    // Undecided within the bound: refused, never opened or loaded.
    let hidden = format!("{}javascript:alert(1)", " ".repeat(ROUTE_PREFIX_BYTES));
    assert_eq!(scheme_route(&hidden), SchemeRoute::Drop);
    assert_eq!(preview_resource(&hidden), PreviewResource::Drop);
    let long = format!("{}:x", "a".repeat(ROUTE_PREFIX_BYTES));
    assert_eq!(scheme_route(&long), SchemeRoute::Drop);
    // Long tails after a decided prefix keep their answer.
    let query = format!("https://e.test/?{}", "a".repeat(20_000));
    assert_eq!(scheme_route(&query), SchemeRoute::Web);
}

#[test]
fn codes_name_the_routes_in_order() {
    let routes = [
        ("docs/a.png", SchemeRoute::Relative),
        ("https://x", SchemeRoute::Web),
        ("mailto:a@b", SchemeRoute::Mail),
        ("file:/x", SchemeRoute::File),
        ("C:/x", SchemeRoute::Drive),
        ("zotero://x", SchemeRoute::Defer("zotero".into())),
        ("javascript:x", SchemeRoute::Drop),
    ];
    for (code, (dest, want)) in routes.into_iter().enumerate() {
        assert_eq!(scheme_route(dest), want);
        assert_eq!(anchor_route_code(dest), code as u32, "{dest}");
    }
    for (code, dest) in ["docs/a.png", "https://x", "tel:1"].into_iter().enumerate() {
        assert_eq!(preview_resource_code(dest), code as u32, "{dest}");
    }
}

#[test]
fn every_refused_name_is_told_apart() {
    let table: serde_json::Value = serde_json::from_str(REFUSED_SCHEMES).unwrap();
    for name in table["refused"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        assert!(name.len() <= LONGEST_KNOWN, "{name}");
        assert_eq!(scheme_route(&format!("{name}:x")), SchemeRoute::Drop);
    }
}
