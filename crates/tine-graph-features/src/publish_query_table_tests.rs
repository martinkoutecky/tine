//! The shared local-target table, export side: publication's asset filter
//! (`publication_asset_names`) over the asset collector, per row. Kept out of
//! `publish_query.rs` so that file names only `Corpus::asset_refs` as its
//! source of asset references.

use super::publication_asset_names;
use std::collections::HashSet;

/// The shared local-target table (fixtures/local-target-cases.json): what
/// publication copies today for each row, and whether that differs from
/// the one attachment the owner names. A differing row must name the
/// decision or rule that explains the change.
#[test]
fn local_target_table_export() {
    let table: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/local-target-cases.json")).unwrap();
    let record = std::env::var_os("LOCAL_TARGET_RECORD").is_some();
    let mut failures = Vec::new();
    for row in table["rows"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let mut found = HashSet::new();
        tine_core::asset_refs::collect_asset_refs(row["source"].as_str().unwrap(), &mut found);
        let copied = publication_asset_names(found);
        // The owner's attachment, from the row's answer (the folder itself is no file).
        let owner = row["owner"].as_str().unwrap();
        let kept: Vec<String> = owner
            .strip_prefix("attachment:")
            .map(|rest| {
                rest.split(" pdf=")
                    .next()
                    .unwrap()
                    .split(" media=")
                    .next()
                    .unwrap()
            })
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .into_iter()
            .collect();
        let differs = copied != kept;
        if record {
            println!(
                "REC {}",
                serde_json::json!({"id": id, "export": copied, "export_differs": differs})
            );
            continue;
        }
        let recorded: Vec<String> = serde_json::from_value(row["today"]["export"].clone()).unwrap();
        if copied != recorded {
            failures.push(format!("{id}: export {copied:?} != {recorded:?}"));
        }
        if differs != !row["change"]["export"].is_null() {
            failures.push(format!(
                "{id}: export differs = {differs}, change = {}",
                row["change"]["export"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
