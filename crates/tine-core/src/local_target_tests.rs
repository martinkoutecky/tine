//! The shared local-target table (`fixtures/local-target-cases.json`), Rust
//! side: the owner gives each row's recorded answer, the asset collector's own
//! answer is still the row's `today.collector`, and liveness (orphan listing,
//! trash recheck) is exactly that plus the owner's attachment. A row whose
//! recorded collector answer misses the owner's attachment must name the
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
        crate::asset_refs::collect_live_asset_refs(source, &mut found);
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
        // Liveness is exactly the earlier collector's names plus that
        // attachment (and its first folder): never fewer names than before
        // the collector asked the owner.
        let before = strings(&row["today"]["collector"]);
        // The collector's own answer, which export copies, is still exactly
        // what it was before liveness asked the owner.
        let mut own = HashSet::new();
        crate::asset_refs::collect_asset_refs(source, &mut own);
        let mut own: Vec<String> = own.into_iter().collect();
        own.sort();
        if own != before {
            failures.push(format!("{id}: collector {own:?} != today {before:?}"));
        }
        let mut live: HashSet<String> = before.iter().cloned().collect();
        if let Some(name) = &kept {
            live.insert(name.clone());
            if let Some((folder, _)) = name.split_once('/') {
                live.insert(folder.to_owned());
            }
        }
        // Liveness holds develop's names as written (the widening rule adds
        // more, pinned by `liveness_keeps_every_name_any_answerer_gives`).
        if !live.is_subset(&found) {
            let mut want: Vec<&String> = live.difference(&found).collect();
            want.sort();
            failures.push(format!(
                "{id}: liveness {collector:?} lost today + owner {want:?}"
            ));
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
/// no attachment: the owner reads a folder written `assets` in any case.
#[test]
fn collector_prefilter_admits_every_spelling_the_owner_reads() {
    for (text, name) in [
        ("![x](../ASSETS/a.png)", "a.png"),
        ("![x](../AsSeTs/b.png)", "b.png"),
        ("{{img ../Assets/c.png, 200}}", "c.png"),
        ("[[../ASSETS/d.png]]", "d.png"),
    ] {
        let mut targets = HashSet::new();
        crate::asset_refs::collect_asset_targets(text, &mut targets);
        assert_eq!(targets, HashSet::from([name.to_owned()]), "{text}");
        let mut live = HashSet::new();
        crate::asset_refs::collect_live_asset_refs(text, &mut live);
        assert!(live.contains(name), "{text}: {live:?}");
    }
    // The prefilter reads long text in overlapping windows: a folder name at
    // every offset around a window edge is still found.
    for pad in 230..280 {
        let text = format!("{}![x](../AsSeTs/e.png)", "x".repeat(pad));
        let mut targets = HashSet::new();
        crate::asset_refs::collect_asset_targets(&text, &mut targets);
        assert_eq!(targets, HashSet::from(["e.png".to_owned()]), "pad {pad}");
    }
}

/// A file the reader shows never becomes unused because the collector also
/// asks the owner: the sole reference keeps every name the collector kept
/// before (here the literal file names the native reader loads, which the
/// owner decodes or reads as a graph file or a page), so the orphan list
/// leaves it out and the trash recheck refuses it.
#[test]
fn liveness_keeps_the_names_the_collector_kept_before_the_owner() {
    let failures = [
        ("![x](../assets/my%20file(1).png)", "my%20file(1).png"),
        ("![x](../assets/my%20file[1].png)", "my%20file[1].png"),
        ("![x](../assets/my%20a|b.png)", "my%20a|b.png"),
        ("![x](../myassets/x(1).png)", "x(1).png"),
        (r#"[x](file:assets/report.png "caption")"#, "report.png"),
        (r#"[x](assets/README "caption")"#, "README"),
    ]
    .into_iter()
    .filter_map(|(text, shown)| {
        let mut live = HashSet::new();
        crate::asset_refs::collect_live_asset_refs(text, &mut live);
        (!live.contains(shown)).then(|| format!("{text}: {shown} not kept in {live:?}"))
    })
    .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Liveness only adds names, also when one note's references disagree: a
/// stale prose path under a file (`x.png/thumb.png`, which a folder rule
/// could read as "x.png is a folder") and two spellings of one file on a
/// case-insensitive volume. Every name the collector alone keeps, and every
/// file the reader shows, stays kept; the orphan list and trash compare
/// names exactly, so an extra name can only keep a file off the unused list.
#[test]
fn liveness_only_adds_names_when_references_disagree() {
    let cases: [(&str, &[&str]); 6] = [
        (
            "- ![x](../assets/./x.png)\n- see ../assets/x.png/thumb.png",
            &["x.png"],
        ),
        (
            "- ![x](../ASSETS/x.png)\n- see ../assets/x.png/thumb.png",
            &["x.png"],
        ),
        (
            "- ![x](../assets/x.png/../x.png)\n- see ../assets/x.png/thumb.png",
            &["x.png"],
        ),
        (
            "- <img src=\"../assets/x.png/thumb.png\">\n- ![x](../assets/./x.png)",
            &["x.png"],
        ),
        (
            "- ![x](../ASSETS/Pic.png)\n- banner:: ../assets/pic.png",
            &["Pic.png", "pic.png"],
        ),
        (
            "- ![x](../Assets/Sub/Pic.png)\n- ![x](../assets/sub/pic.png)",
            &["Sub/Pic.png", "Sub", "sub/pic.png", "sub"],
        ),
    ];
    let mut failures = Vec::new();
    for (text, shown) in cases {
        let mut own = HashSet::new();
        crate::asset_refs::collect_asset_refs(text, &mut own);
        let mut live = HashSet::new();
        crate::asset_refs::collect_live_asset_refs(text, &mut live);
        if let Some(lost) = own.difference(&live).next() {
            failures.push(format!(
                "{text:?}: liveness lost {lost} the collector keeps"
            ));
        }
        for name in shown {
            if !live.contains(*name) {
                failures.push(format!("{text:?}: {name} not kept in {live:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn is_live(text: &str, name: &str) -> bool {
    let mut live = HashSet::new();
    crate::asset_refs::collect_live_asset_refs(text, &mut live);
    crate::asset_refs::is_live(name, |name| live.contains(name))
}

/// The destination text a reader hands its helpers for a row: the parser's
/// link destination (a `Complex` one with and without its protocol), the
/// `img` macro's first token, or the row's explicit destination.
fn destinations_of(row: &Value) -> Vec<String> {
    if let Some(dest) = row.get("dest").and_then(Value::as_str) {
        return vec![dest.to_owned()];
    }
    let source = row["source"].as_str().unwrap();
    let nodes =
        crate::render::parse_inline_bounded(source, row["format"].as_str().unwrap()).unwrap();
    let nodes = serde_json::to_value(&nodes).unwrap();
    if row["origin"] == "img-macro" {
        let first = find(&nodes, "macro").unwrap()["args"][0]
            .as_str()
            .unwrap_or("")
            .to_owned();
        return first
            .split_whitespace()
            .next()
            .into_iter()
            .map(str::to_owned)
            .collect();
    }
    let Some(link) = find(&nodes, "link") else {
        return vec![source.to_owned()];
    };
    match serde_json::from_value::<Url>(link["url"].clone()).unwrap() {
        Url::Search { v } | Url::File { v } | Url::PageRef { v } => vec![v],
        Url::Complex { protocol, link } => {
            let link = link.unwrap_or_default();
            let full = protocol.map(|p| complex_destination(&p, &link));
            [Some(link), full].into_iter().flatten().collect()
        }
        _ => Vec::new(),
    }
}

/// The widening rule's property over the shared table: for every row, every
/// file name any answerer gives is live (compared by `is_live`, as the
/// orphan list and trash compare). Rust answerers: the owner (attachment and
/// both PDF readers' names), develop's collector, and the reader's helpers
/// `asset_rel_path` (raw and decoded, as images and asset links read it) and
/// `pdf_asset_file` (for a destination some answerer opens as a PDF). The
/// TypeScript reader's names come from the row's recorded `today.renderer`,
/// which `src/render/localTarget.table.test.tsx` pins against the reader.
#[test]
fn liveness_keeps_every_name_any_answerer_gives() {
    let mut failures = Vec::new();
    for row in rows() {
        let id = row["id"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let mut names: Vec<(String, &str)> = Vec::new();
        let owner = target_of(&row);
        if let Some(name) = owner.attachment() {
            names.push((name.as_str().to_owned(), "owner attachment"));
        }
        match owner.pdf() {
            Some(Pdf::AssetReader { name })
            | Some(Pdf::OutsideReader {
                fallback: Some(name),
            }) => names.push((name.as_str().to_owned(), "owner pdf")),
            _ => (),
        }
        for name in strings(&row["today"]["collector"]) {
            names.push((name, "collector"));
        }
        let renderer = row["today"]["renderer"].as_str().unwrap();
        for prefix in ["attachment:", "image:", "media:", "icon:", "asset-pdf:"] {
            if let Some(name) = renderer.strip_prefix(prefix).filter(|n| *n != "none") {
                names.push((name.to_owned(), "reader"));
            }
        }
        let pdf = renderer.contains("pdf") || describe(&owner).contains("pdf=");
        for dest in destinations_of(&row) {
            if let Some(rel) = asset_rel_path(&dest) {
                names.push((percent_decode(&rel), "asset_rel_path decoded"));
                names.push((rel, "asset_rel_path"));
            }
            if pdf {
                names.push((pdf_asset_file(&dest), "pdf_asset_file"));
            }
        }
        for (name, who) in names {
            if !name.is_empty() && !is_live(source, &name) {
                failures.push(format!("{id} {source:?}: {who} {name:?} not live"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every trace the design checks and the step reviews found where develop
/// offers a file some surface uses for trash: the sole reference, and the
/// directory entry the surface opens. Each must be live.
#[test]
fn liveness_keeps_the_files_develop_offered_for_trash() {
    let cases: &[(&str, &str)] = &[
        // v1 check 5: a case-folding volume opens `pic.png` for `Pic.png`.
        ("![x](../assets/Pic.png)", "pic.png"),
        // v2 check 6: ext4 casefold opens `Maße.png` for `MASSE.png`.
        ("![x](../assets/MASSE.png)", "Maße.png"),
        // v1 check 5: a decoded `.pdf` the owner sends to the asset reader.
        ("[x](guide.p%64f)", "guide.pdf"),
        // v1 check 9: a page icon spelled with escapes.
        ("icon:: ![](%2e%2e/%61ssets/i.png)", "i.png"),
        // v2 check 5: the native image reads the undecoded tail.
        ("![](../ASSETS/my%20f.png)", "my%20f.png"),
        // v3 check 9: query suffix under an uppercase folder.
        ("![](../ASSETS/a%20b.png?rev=1)", "a b.png"),
        ("![](../ASSETS/a%20b.png?rev=1)", "a%20b.png?rev=1"),
        ("![](../ASSETS/a%20b.png?rev=1)", "a%20b.png"),
        // Judge revision: the loose scan kept `a.mp4}}`, the player shows `a.mp4`.
        ("{{video ../assets/a.mp4}}", "a.mp4"),
        ("{{video ../Assets/b.mp4}}", "b.mp4"),
        // Judge revision (pending decision 4): bare and graph-relative PDFs
        // open `assets/<basename>`.
        ("[g](guide.pdf)", "guide.pdf"),
        ("![Guide](docs/guide.pdf)", "guide.pdf"),
        ("[[file:docs/a.pdf][x]]", "a.pdf"),
        ("![x](%43:%5Cdocs%5Cbad%ZZ.PDF)", "bad%ZZ.PDF"),
        // Judge revision (c40): the reader requests `a.png,` for the comma form.
        ("{{img ../assets/a.png, 100}}", "a.png,"),
        // Judge hard call 2: any case, `\` and `%5C` separators in raw HTML.
        ("<img src=\"..\\ASSETS\\x.png\">", "x.png"),
        ("<img src=\"..%5CAssets%5Cy.png\">", "y.png"),
        // Step 2 review B2: entity-spelled HTML names and Hiccup escapes.
        ("<img src=\"../assets/r&#101;al.png\">", "real.png"),
        ("<img src=\"../&#97;ssets/e.png\">", "e.png"),
        ("[:img {:src \"../\\u0061ssets/h.png\"}]", "h.png"),
        // `hls__` pointers: the PDF pane reads `file-path::` to line end.
        ("file-path:: ../ASSETS/my report.pdf", "my report.pdf"),
        ("file-path:: report.pdf", "report.pdf"),
        ("#+FILE-PATH: ../Assets/o.pdf", "o.pdf"),
        // An uppercase raw-HTML attribute with a space and parentheses.
        ("<img src=\"../ASSETS/a b(1).png\">", "a b(1).png"),
        // Lenient decode: an invalid UTF-8 escape stays literal.
        ("![](../ASSETS/a%FF%20b.png)", "a%FF b.png"),
        // A non-`assets` relative image the reader opens by basename.
        ("![x](pics/photo.png)", "photo.png"),
    ];
    let failures: Vec<String> = cases
        .iter()
        .filter(|(text, name)| !is_live(text, name))
        .map(|(text, name)| format!("{text:?} leaves {name:?} trashable"))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The second review's traces where a file a surface uses stays trashable:
/// whitespace-only names the fold empties, `hls__` pointers the parser reads
/// (any key case, Org directives and drawers, an extensionless PDF), an image
/// macro split at U+FEFF, and a graph-file link through a second `assets`
/// segment to an extensionless file.
#[test]
fn liveness_keeps_the_files_the_second_review_found_trashable() {
    let cases: &[(&str, &str)] = &[
        // B1: a name the fold empties keeps develop's exact match.
        ("[blank](../assets/%20)", " "),
        ("[blank](../assets/%20%20)", "  "),
        ("[blank](../assets/%09)", "\t"),
        ("[blank](../assets/%0A)", "\n"),
        ("[blank](../assets/%C2%A0)", "\u{a0}"),
        ("[blank](../assets/%E3%80%80)", "\u{3000}"),
        // B2: the parser's property values, any key case and Org form.
        ("File-Path:: ../assets/m.pdf", "m.pdf"),
        ("FILE-path:: report.pdf", "report.pdf"),
        ("#+File-Path: o.pdf", "o.pdf"),
        (":PROPERTIES:\n:file-path: d.pdf\n:END:\n", "d.pdf"),
        ("file-path:: /outside/report", "report"),
        // B4: the reader splits the macro at U+FEFF.
        ("{{img ../assets/x.png\u{feff}100}}", "x.png"),
        ("{{IMG ../ASSETS/y.png\u{feff}center}}", "y.png"),
        // B5: the opener applies `..` before it finds the `assets` folder.
        ("[x](%61ssets/../%61ssets/README)", "README"),
        ("[x](file:%61ssets/../%61ssets/NOTES)", "NOTES"),
        ("[[file:%61ssets/../%61ssets/LICENSE][x]]", "LICENSE"),
        ("[x](assets/sub/../../assets/CHANGES)", "CHANGES"),
    ];
    let failures: Vec<String> = cases
        .iter()
        .filter(|(text, name)| !is_live(text, name))
        .map(|(text, name)| format!("{text:?} leaves {name:?} trashable"))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Membership only widens: a set holding a whitespace-only name as written
/// matches exactly that entry, and an entry is never matched through an
/// empty key.
#[test]
fn membership_keeps_the_exact_match_for_names_the_fold_empties() {
    let set: HashSet<String> = [" ".to_owned()].into();
    assert!(crate::asset_refs::is_live(" ", |n| set.contains(n)));
    let empty_key: HashSet<String> = ["".to_owned()].into();
    assert!(!crate::asset_refs::is_live("  ", |n| empty_key.contains(n)));
}

/// The parse prefilter reads each line a bounded number of times: a closer
/// is never searched for past its line, so a long block of unclosed `[[`,
/// `{{` or `](<` lines costs O(text bytes) (second review, B3). Counts the
/// bytes handed to the closer searches.
#[test]
fn the_destination_prefilter_never_searches_past_a_line() {
    use crate::asset_refs::PREFILTER_SEARCHED;
    for unit in ["[[a\n", "{{a\n", "[x](<a\n"] {
        let text = unit.repeat(4096);
        PREFILTER_SEARCHED.with(|n| n.set(0));
        let mut live = HashSet::new();
        crate::asset_refs::collect_live_asset_refs(&text, &mut live);
        let searched = PREFILTER_SEARCHED.with(|n| n.get());
        assert!(
            searched <= 2 * text.len(),
            "{unit:?}: searched {searched} bytes of {}",
            text.len()
        );
    }
}

/// One lenient decoder (I-12): liveness asks `tine-outline`'s, it keeps no
/// copy of its own.
#[test]
fn liveness_has_no_decoder_of_its_own() {
    let source = include_str!("asset_refs.rs");
    assert!(source.contains("decode_escapes_keeping_malformed"));
    assert!(!source.contains("utf8_chunks"));
    assert!(is_live("![](../ASSETS/A%ff%20B.png)", "A%ff B.png"));
}

/// The widening rule decodes once, as every surface does: a twice-escaped
/// name keeps its once-decoded file, never the twice-decoded look-alike, and
/// a text with no file name keeps nothing.
#[test]
fn liveness_widens_by_one_decode_only() {
    assert!(is_live("![x](../assets/demo%2520v2.png)", "demo%20v2.png"));
    assert!(!is_live("![x](../assets/demo%2520v2.png)", "demo v2.png"));
    let html = "<img src=\"../assets/demo%2520v2.png\">";
    assert!(!is_live(html, "demo v2.png"));
    let mut live = HashSet::new();
    crate::asset_refs::collect_live_asset_refs("plain prose, a [[Page]] and #tag", &mut live);
    assert!(live.is_empty(), "{live:?}");
}

/// The widening rule is O(text bytes) in what it keeps: four times the text
/// keeps at most about four times the name bytes, for many mentions on one
/// line, one long run of nested folders and many bracketed destinations.
#[test]
fn liveness_keeps_name_bytes_linear_in_the_text() {
    let kept = |text: &str| {
        let mut live = HashSet::new();
        crate::asset_refs::collect_live_asset_refs(text, &mut live);
        live.iter().map(String::len).sum::<usize>()
    };
    for unit in [
        "<img src=\"../ASSETS/a b.png\"> ![x](../Assets/c%20d.png?x=1) ",
        "assets/",
        "[[x.pdf]] {{video ../assets/v.mp4}} [y](z.pdf) ",
    ] {
        let small = kept(&unit.repeat(200));
        let large = kept(&unit.repeat(800));
        assert!(small > 0, "{unit:?}");
        assert!(large <= small * 5, "{unit:?}: {small} -> {large}");
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
