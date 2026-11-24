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
