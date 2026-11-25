//! The shared local-target table (`fixtures/local-target-cases.json`), Rust
//! side: the owner gives each row's recorded answer, and the asset collector
//! (orphan listing, trash recheck) keeps the owner's attachment alive and
//! never fewer names than before it asked the owner (the row's `today`). A row
//! whose recorded liveness differs from the owner's attachment must name the
//! decision or rule that explains it. The TypeScript side pins the renderer;
//! export is pinned in tine-graph-features (`publish_query_table_tests.rs`).

use super::*;
use serde_json::Value;
use std::collections::HashSet;

const TABLE: &str = include_str!("../../../fixtures/local-target-cases.json");

fn rows() -> Vec<Value> {
    let table: Value = serde_json::from_str(TABLE).unwrap();
    table["rows"].as_array().unwrap().clone()
}

fn find<'a>(node: &'a Value, k: &str) -> Option<&'a Value> {
    match node {
        Value::Object(map) if map.get("k").and_then(Value::as_str) == Some(k) => Some(node),
        Value::Object(map) => map.values().find_map(|v| find(v, k)),
        Value::Array(items) => items.iter().find_map(|v| find(v, k)),
        _ => None,
    }
}

/// The row's destination as production hands it over: the parser's own link
/// node, the first token of the `img` macro's first argument, or the text.
fn target_of(row: &Value) -> LocalTarget {
    target_as(row, Origin::parse(row["origin"].as_str().unwrap()).unwrap())
}

/// The attachment the collector should find in the row's source: the owner's
/// for the row, but a page icon's value is read as the image embed it is
/// written as (the collector does not know a value is an icon). The folder
/// itself is no file.
pub(crate) fn collected_attachment(row: &Value) -> Option<String> {
    let target = match row["origin"].as_str() {
        Some("page-icon") => target_as(row, Origin::MdEmbed),
        _ => target_of(row),
    };
    target
        .attachment()
        .map(|name| name.as_str().to_owned())
        .filter(|name| !name.is_empty())
}

fn target_as(row: &Value, origin: Origin) -> LocalTarget {
    if let Some(dest) = row.get("dest").and_then(Value::as_str) {
        return local_target(Destination::Text(dest), origin);
    }
    let source = row["source"].as_str().unwrap();
    let nodes =
        crate::render::parse_inline_bounded(source, row["format"].as_str().unwrap()).unwrap();
    let nodes = serde_json::to_value(&nodes).unwrap();
    if origin == Origin::ImgMacro {
        let macro_node = find(&nodes, "macro").unwrap();
        let first = macro_node["args"][0].as_str().unwrap_or("");
        let url = first.split_whitespace().next().unwrap_or("");
        return local_target(Destination::Text(url), origin);
    }
    match find(&nodes, "link") {
        Some(link) => {
            let url: Url = serde_json::from_value(link["url"].clone()).unwrap();
            local_target(Destination::Link { url: &url }, origin)
        }
        None => local_target(Destination::Text(source), origin),
    }
}

/// The table's notation for an answer; the TypeScript side writes the same.
pub(crate) fn describe(t: &LocalTarget) -> String {
    let route = match t.route() {
        Route::PageRef { name } => format!("page:{name}"),
        Route::BlockRef { id } => format!("block:{id}"),
        Route::Web => "web".into(),
        Route::Mail => "mail".into(),
        Route::AppScheme { scheme } => format!("app:{scheme}"),
        Route::Refused => "refused".into(),
        Route::Fragment { href } => format!("fragment:{href}"),
        Route::Attachment { name } => format!("attachment:{}", name.as_str()),
        Route::GraphFile { path } => format!("graph-file:{path}"),
        Route::OutsideFile {
            path,
            reason: OutsideReason::Absolute,
        } => format!("outside-absolute:{path}"),
        Route::OutsideFile {
            path,
            reason: OutsideReason::Escapes,
        } => format!("outside-escapes:{path}"),
        Route::Drive { path } => format!("drive:{path}"),
        Route::FileUrl { url } => format!("file-url:{url}"),
        Route::PageRelative { path } => format!("page-relative:{path}"),
        Route::RefusedMalformed => "refused-malformed".into(),
    };
    let pdf = match t.pdf() {
        None => String::new(),
        Some(Pdf::AssetReader { name }) => format!(" pdf=asset:{}", name.as_str()),
        Some(Pdf::OutsideReader { fallback: None }) => " pdf=outside".into(),
        Some(Pdf::OutsideReader {
            fallback: Some(name),
        }) => format!(" pdf=outside>asset:{}", name.as_str()),
    };
    format!("{route}{pdf}")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn table_owner_and_collector() {
    let record = std::env::var_os("LOCAL_TARGET_RECORD").is_some();
    let mut failures = Vec::new();
    for row in rows() {
        let id = row["id"].as_str().unwrap();
        let owner = target_of(&row);
        let source = row["source"].as_str().unwrap();
        let mut found = HashSet::new();
        crate::asset_refs::collect_asset_refs(source, &mut found);
        let mut collector: Vec<String> = found.iter().cloned().collect();
        collector.sort();
        let mut targets = HashSet::new();
        crate::asset_refs::collect_asset_targets(source, &mut targets);
        let kept = collected_attachment(&row);
        if record {
            println!(
                "REC {}",
                serde_json::json!({"id": id, "owner_rust": describe(&owner), "collector": collector, "targets": targets})
            );
            continue;
        }
        if describe(&owner) != row["owner"].as_str().unwrap() {
            failures.push(format!(
                "{id}: owner {} != {}",
                describe(&owner),
                row["owner"]
            ));
        }
        // The collector's targets are exactly the owner's attachment.
        let expected: HashSet<String> = kept.iter().cloned().collect();
        if targets != expected {
            failures.push(format!("{id}: targets {targets:?} != owner {expected:?}"));
        }
        // Liveness keeps that attachment, and never fewer names than before
        // the collector asked the owner.
        let before = strings(&row["today"]["collector"]);
        let lost: Vec<&String> = before
            .iter()
            .chain(kept.iter())
            .filter(|name| !found.contains(*name))
            .collect();
        if !lost.is_empty() {
            failures.push(format!("{id}: liveness lost {lost:?} (now {collector:?})"));
        }
        // The record is consistent: `change.liveness` names why the earlier
        // collector missed the owner's attachment.
        let missed_before = kept.as_ref().is_some_and(|name| !before.contains(name));
        if missed_before != !row["change"]["liveness"].is_null() {
            failures.push(format!(
                "{id}: liveness differed = {missed_before}, change = {}",
                row["change"]["liveness"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The collector's prefilter skips the parse only where the owner can find
/// no attachment: any case of `assets`, and raw HTML whose folder name is
/// spelled with character references, which the reader decodes.
#[test]
fn collector_prefilter_admits_every_spelling_the_owner_reads() {
    for (text, name) in [
        ("![x](../ASSETS/a.png)", "a.png"),
        ("![x](../AsSeTs/b.png)", "b.png"),
        (r#"<img src="../&#97;ssets/e.png"/>"#, "e.png"),
        (r#"<video poster="../&#x41;SSETS/f.png"></video>"#, "f.png"),
    ] {
        let mut targets = HashSet::new();
        crate::asset_refs::collect_asset_targets(text, &mut targets);
        assert_eq!(targets, HashSet::from([name.to_owned()]), "{text}");
        let mut live = HashSet::new();
        crate::asset_refs::collect_asset_refs(text, &mut live);
        assert!(live.contains(name), "{text}: {live:?}");
    }
}

/// Principle 2: pure and O(destination bytes). Times the owner alone over the
/// whole table (parsing excluded) and bounds it loosely against hangs.
#[test]
fn table_cost() {
    let rows = rows();
    let inputs: Vec<(Url, Origin)> = rows
        .iter()
        .filter(|row| row.get("dest").is_none() && row["origin"] != "img-macro")
        .filter_map(|row| {
            let nodes = crate::render::parse_inline_bounded(
                row["source"].as_str()?,
                row["format"].as_str()?,
            )?;
            let nodes = serde_json::to_value(&nodes).ok()?;
            let link = find(&nodes, "link")?;
            let url = serde_json::from_value(link["url"].clone()).ok()?;
            Some((url, Origin::parse(row["origin"].as_str()?)?))
        })
        .collect();
    let started = std::time::Instant::now();
    const ROUNDS: u32 = 1000;
    for _ in 0..ROUNDS {
        for (url, origin) in &inputs {
            std::hint::black_box(local_target(Destination::Link { url }, *origin));
        }
    }
    let per_table = started.elapsed() / ROUNDS;
    println!(
        "local target: {} link rows in {per_table:?} per table",
        inputs.len()
    );
    assert!(inputs.len() > 100);
    assert!(
        per_table < std::time::Duration::from_millis(50),
        "{per_table:?}"
    );
}

/// Only the owner can make an `AssetName` or a `LocalTarget`: their fields are
/// private, so any other module fails to compile when it tries. This pins that
/// the fields stay private.
#[test]
fn asset_name_and_target_are_built_only_by_the_owner() {
    let own = include_str!("local_target.rs");
    assert!(
        own.contains("pub struct AssetName(String);"),
        "AssetName's field must stay private"
    );
    let target = own.split("pub struct LocalTarget {").nth(1).unwrap();
    let fields = &target[..target.find('}').unwrap()];
    assert!(
        !fields.contains("pub "),
        "LocalTarget's fields must stay private: {fields}"
    );
}

/// The strict percent-decoder and the ECMAScript trim set each have one Rust
/// answer, in `tine-outline` (`page_filename::decode_uri_component`,
/// `editor_text::is_js_whitespace`); the owner asks them and holds neither.
#[test]
fn owner_holds_no_decoder_or_whitespace_set() {
    let own = include_str!("local_target.rs");
    let own = &own[..own.find("#[cfg(test)]").unwrap()];
    for (needle, what) in [
        ("fn js_trim", "a trim of its own"),
        ("fn strict_decode", "a strict decoder of its own"),
        ("fn decode_uri_component", "a strict decoder of its own"),
        ("from_utf8(", "a strict UTF-8 decode of its own"),
        ("\\u{feff}", "a whitespace set of its own"),
        ("\\u{FEFF}", "a whitespace set of its own"),
        ("\\u{3000}", "a whitespace set of its own"),
    ] {
        assert!(!own.contains(needle), "the owner holds {what}: `{needle}`");
    }
    assert!(own.contains("use tine_outline::editor_text::js_trim;"));
    assert!(own.contains("use tine_outline::page_filename::decode_uri_component as strict_decode;"));
    // The shared answers still give the owner's rule: strict on malformed,
    // truncated and non-UTF-8 escapes; trim strips CRLF, NBSP and U+FEFF and
    // keeps U+0085.
    use tine_outline::{editor_text::js_trim, page_filename::decode_uri_component};
    assert_eq!(decode_uri_component("caf%C3%A9").as_deref(), Some("café"));
    for bad in ["50%", "%E", "%zz", "%C3", "%FF"] {
        assert_eq!(decode_uri_component(bad), None, "{bad}");
    }
    assert_eq!(js_trim("\r\n\u{a0}\u{feff}a\u{3000} "), "a");
    assert_eq!(js_trim("\u{85}a"), "\u{85}a");
}

fn parsed(source: &str, format: &str) -> Url {
    let nodes = crate::render::parse_inline_bounded(source, format).unwrap();
    let nodes = serde_json::to_value(&nodes).unwrap();
    let link = find(&nodes, "link").unwrap();
    serde_json::from_value(link["url"].clone()).unwrap()
}

fn owner_of(url: &Url, origin: Origin) -> String {
    describe(&local_target(Destination::Link { url }, origin))
}

/// The destination of a `Complex` link is Logseq's rule over the parser's
/// fields, never a scan of the link's source: titles, labels and escapes in
/// the source cannot move it.
#[test]
fn complex_destination_is_logseqs_rule() {
    let cases = [
        (
            "[x](C://docs/note.txt \"see C:docs/note.txt here\")",
            Origin::MdLink,
            "drive:C:/docs/note.txt",
        ),
        (
            "[x](C://docs/a\\(1\\).txt)",
            Origin::MdLink,
            "drive:C:/docs/a(1).txt",
        ),
        (
            "[x](C\\://docs/n.txt)",
            Origin::MdLink,
            "drive:C:/docs/n.txt",
        ),
        (
            "[x](C:\\/\\/docs/n.txt)",
            Origin::MdLink,
            "drive:C:/docs/n.txt",
        ),
        (
            "[x](C://docs\\\\n.txt)",
            Origin::MdLink,
            "drive:C:/docs/n.txt",
        ),
        (
            "[x](<C://docs/n.txt>)",
            Origin::MdLink,
            "drive:C:/docs/n.txt",
        ),
        (
            "![x](file:///tmp/a.txt \"file:////tmp/a.txt\")",
            Origin::MdEmbed,
            "outside-absolute:/tmp/a.txt",
        ),
        (
            "[file:////s/x](file:///tmp/a.txt)",
            Origin::MdLink,
            "outside-absolute:/tmp/a.txt",
        ),
        (
            "<file:///tmp/a\\_b.txt>",
            Origin::MdLink,
            "outside-absolute:/tmp/a_b.txt",
        ),
        ("[x](https://e.test/a\\_b \"t\")", Origin::MdLink, "web"),
    ];
    let mut failures = Vec::new();
    for (source, origin, want) in cases {
        let owner = owner_of(&parsed(source, "md"), origin);
        if owner != want {
            failures.push(format!("{source}: {owner} != {want}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The rule itself, and its cost: it reads only the two fields (a link's
/// source text is not an input), so a destination twice as long costs twice.
#[test]
fn complex_destination_reads_the_fields() {
    assert_eq!(complex_destination("C", "docs/n.txt"), "C://docs/n.txt");
    assert_eq!(
        complex_destination("https", "e.test/a_b"),
        "https://e.test/a_b"
    );
    assert_eq!(complex_destination("file", "/tmp/a.txt"), "/tmp/a.txt");
    assert_eq!(
        complex_destination("FILE", "/tmp/a.txt"),
        "FILE:///tmp/a.txt"
    );
    // The parser's output for the forms it cannot tell apart: read alike.
    for (format, a, b) in [
        ("org", "[[C:docs/n.txt][x]]", "[[C://docs/n.txt][x]]"),
        ("md", "<C:docs/n.txt>", "<C://docs/n.txt>"),
        ("md", "[x](file:////s/a.txt)", "[x](file://s/a.txt)"),
    ] {
        assert_eq!(parsed(a, format), parsed(b, format), "{a} / {b}");
        let origin = if format == "md" {
            Origin::MdLink
        } else {
            Origin::OrgLink
        };
        assert_eq!(
            owner_of(&parsed(a, format), origin),
            owner_of(&parsed(b, format), origin)
        );
    }
}
