//! The shared local-target table (`fixtures/local-target-cases.json`), Rust
//! side: the owner gives each row's recorded answer, and the asset collector
//! (orphan listing, trash recheck) still answers what the row records. A row
//! whose liveness differs from the owner's attachment must name the decision
//! or rule that explains it. The TypeScript side pins the renderer; export is
//! pinned in tine-graph-features (`publish_query_table_tests.rs`).

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
    let origin = Origin::parse(row["origin"].as_str().unwrap()).unwrap();
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
            let full = link["full"].as_str().unwrap();
            local_target(Destination::Link { url: &url, full }, origin)
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
    let media = match t.media() {
        None => "",
        Some(Media::Image) => " media=image",
        Some(Media::Audio) => " media=audio",
        Some(Media::Video) => " media=video",
    };
    format!("{route}{pdf}{media}")
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
        let mut found = HashSet::new();
        crate::asset_refs::collect_asset_refs(row["source"].as_str().unwrap(), &mut found);
        let mut collector: Vec<String> = found.iter().cloned().collect();
        collector.sort();
        // The owner keeps one file alive: its attachment (the folder itself is no file).
        let kept = owner
            .attachment()
            .map(AssetName::as_str)
            .filter(|n| !n.is_empty());
        let liveness_differs = kept.is_some_and(|name| !found.contains(name));
        if record {
            println!(
                "REC {}",
                serde_json::json!({"id": id, "owner_rust": describe(&owner), "collector": collector, "liveness_differs": liveness_differs})
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
        if collector != strings(&row["today"]["collector"]) {
            failures.push(format!(
                "{id}: collector {collector:?} != {}",
                row["today"]["collector"]
            ));
        }
        if liveness_differs != !row["change"]["liveness"].is_null() {
            failures.push(format!(
                "{id}: liveness differs = {liveness_differs}, change = {}",
                row["change"]["liveness"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Principle 2: pure and O(destination bytes). Times the owner alone over the
/// whole table (parsing excluded) and bounds it loosely against hangs.
#[test]
fn table_cost() {
    let rows = rows();
    let inputs: Vec<(Url, String, Origin)> = rows
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
            Some((
                url,
                link["full"].as_str()?.to_owned(),
                Origin::parse(row["origin"].as_str()?)?,
            ))
        })
        .collect();
    let started = std::time::Instant::now();
    const ROUNDS: u32 = 1000;
    for _ in 0..ROUNDS {
        for (url, full, origin) in &inputs {
            std::hint::black_box(local_target(Destination::Link { url, full }, *origin));
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

fn parsed(source: &str, format: &str) -> (Url, String) {
    let nodes = crate::render::parse_inline_bounded(source, format).unwrap();
    let nodes = serde_json::to_value(&nodes).unwrap();
    let link = find(&nodes, "link").unwrap();
    (
        serde_json::from_value(link["url"].clone()).unwrap(),
        link["full"].as_str().unwrap().to_owned(),
    )
}

fn owner_of(url: &Url, full: &str, origin: Origin) -> String {
    describe(&local_target(Destination::Link { url, full }, origin))
}

/// The owner reads the authored destination from the parser's `full` text,
/// never from the `Complex` fields: for each Org form of `C://docs/note.txt`
/// the parser's own node gives the drive file, and the same fields under a
/// `full` that does not contain them give nothing (a rebuild would still
/// answer the drive file).
#[test]
fn complex_destinations_come_from_full() {
    for (source, origin) in [
        ("[[C://docs/note.txt][x]]", Origin::OrgLink),
        ("[[C://docs/note.txt]]", Origin::OrgBare),
        ("<C://docs/note.txt>", Origin::OrgBare),
    ] {
        let (url, full) = parsed(source, "org");
        let want = "drive:C:/docs/note.txt";
        assert_eq!(owner_of(&url, &full, origin), want, "{source}");
        let other = owner_of(&url, "[[D://elsewhere.txt]]", origin);
        assert_eq!(other, "refused-malformed", "{source}");
    }
    // The parser gives `C:docs` and `C://docs` the same fields; `full` keeps the
    // drive-relative form, which is refused, and the second `//` of a file URL.
    let cases = [
        ("org", "[[C:docs/note.txt][x]]", "refused-malformed"),
        ("org", "[[C://d/n.txt][C:x]]", "drive:C:/d/n.txt"),
        ("md", "<C:docs/note.txt>", "refused-malformed"),
        ("md", "[C:x](C://docs/note.txt)", "drive:C:/docs/note.txt"),
        ("md", "[x](file:////s/a.txt)", "file-url:file:////s/a.txt"),
        ("md", "[x](https://e/a\\_b.png \"t\")", "web media=image"),
    ];
    for (format, source, want) in cases {
        let (url, full) = parsed(source, format);
        let origin = if format == "md" {
            Origin::MdLink
        } else {
            Origin::OrgLink
        };
        let owner = owner_of(&url, &full, origin);
        assert_eq!(owner, want, "{source} ({url:?}, {full:?})");
    }
}

/// One inventory: the owner's classifier and the list `src/media.ts` reads
/// are the same table.
#[test]
fn media_inventory_is_the_classifier_table() {
    let inventory = media_inventory();
    for pair in inventory.split(',') {
        let (ext, kind) = pair.split_once(':').unwrap();
        let want = match kind {
            "image" => Media::Image,
            "audio" => Media::Audio,
            "video" => Media::Video,
            other => panic!("unknown kind {other}"),
        };
        assert_eq!(media_kind(&format!("x.{}", ext.to_uppercase())), Some(want));
    }
    assert_eq!(inventory.split(',').count(), MEDIA_EXTS.len());
}
