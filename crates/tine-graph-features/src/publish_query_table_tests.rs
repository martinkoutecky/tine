//! The shared local-target table, export side: publication's asset filter
//! (`publication_asset_names`) over the asset collector, per row. Kept out of
//! `publish_query.rs` so that file names only `Corpus::asset_refs` as its
//! source of asset references.

use super::publication_asset_names;
use std::collections::HashSet;
use tine_core::local_target::{local_target, Destination, Origin};
use tine_core::lsdoc::ast::Inline;

/// The attachment a row's source names for export: the owner's answer for
/// the row, but a page icon's value is read as the image embed it is
/// written as (the collector does not know a value is an icon; decision:
/// icons keep their own page-relative rule). The folder itself is no file.
fn expected_copy(row: &serde_json::Value) -> Vec<String> {
    let name = if row["origin"] == "page-icon" {
        let nodes =
            tine_core::render::parse_inline_bounded(row["source"].as_str().unwrap(), "md").unwrap();
        let Some(Inline::Link { url, .. }) = nodes.first() else {
            panic!("{}: an icon row's source is one image embed", row["id"]);
        };
        local_target(Destination::Link { url }, Origin::MdEmbed)
            .attachment()
            .map(|name| name.as_str().to_owned())
    } else {
        row["owner"]
            .as_str()
            .unwrap()
            .strip_prefix("attachment:")
            .map(|rest| rest.split(" pdf=").next().unwrap().to_owned())
    };
    name.filter(|name| !name.is_empty()).into_iter().collect()
}

/// The shared local-target table (fixtures/local-target-cases.json): export
/// copies exactly the attachment the owner names for each row (test 14: an
/// HTTPS link copies nothing; test 17: an odd name copies exactly that file).
/// The row's `today.export` records what export copied before it asked the
/// owner; where that differs, the row must name the decision or rule.
#[test]
fn local_target_table_export() {
    let table: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/local-target-cases.json")).unwrap();
    let record = std::env::var_os("LOCAL_TARGET_RECORD").is_some();
    let mut failures = Vec::new();
    for row in table["rows"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let mut found = HashSet::new();
        tine_core::asset_refs::collect_asset_targets(row["source"].as_str().unwrap(), &mut found);
        let copied = publication_asset_names(found);
        let kept = expected_copy(row);
        if record {
            println!("REC {}", serde_json::json!({"id": id, "export": copied}));
            continue;
        }
        if copied != kept {
            failures.push(format!("{id}: export {copied:?} != owner {kept:?}"));
        }
        let before: Vec<String> = serde_json::from_value(row["today"]["export"].clone()).unwrap();
        let differed = before != kept;
        if row["origin"] != "page-icon" && differed != !row["change"]["export"].is_null() {
            failures.push(format!(
                "{id}: export differed = {differed}, change = {}",
                row["change"]["export"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Design tests 14-17 end to end, through the store: for a sole reference in
/// each spelling, the orphan list leaves out exactly the file the reader
/// shows, trash refuses it and publication copies it; an HTTPS link copies
/// nothing; an odd name copies exactly the named file and not a look-alike.
#[test]
fn orphans_trash_and_export_see_the_file_the_reader_shows() {
    use crate::assets::{orphan_assets, trash_asset, TrashOutcome};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for folder in ["pages", "journals", "assets/sub"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    let page = [
        "- ![x](../ASSETS/upper.png)",
        "- ![x](../assets%5Cbackslash.png)",
        "- ![x](../assets/sub%5Cnested.png)",
        "- {{img ../assets/macro.png, 200}}",
        "- [[../assets/org-bare.png]]",
        "- <video poster=\"../assets/poster.png\"></video>",
        "- [:img {:src \"../assets/hiccup.png\"}]",
        "- [x](https://example.com/assets/web.png)",
        "- ![x](../assets/photo%23v2.png)",
        "- ![x](../assets/demo%2520v2.png)",
    ];
    std::fs::write(root.join("pages/a.md"), page.join("\n") + "\n").unwrap();
    let shown = [
        "upper.png",
        "backslash.png",
        "macro.png",
        "org-bare.png",
        "poster.png",
        "hiccup.png",
        "photo#v2.png",
        "demo%20v2.png",
    ];
    // Look-alikes the reader does not show: the undecoded and twice-decoded
    // spellings, and a file nothing names.
    let others = ["web.png", "demo v2.png", "unused.png"];
    for name in shown.iter().chain(&others).chain(&["sub/nested.png"]) {
        std::fs::write(root.join("assets").join(name), name.as_bytes()).unwrap();
    }
    let (store, _, _) = tine_store::Store::open(root, Default::default()).unwrap();

    let orphans: Vec<String> = orphan_assets(&store)
        .unwrap()
        .into_iter()
        .map(|asset| asset.name)
        .collect();
    for name in shown {
        assert!(
            !orphans.contains(&name.to_owned()),
            "{name} listed as unused: {orphans:?}"
        );
    }
    assert!(orphans.contains(&"unused.png".to_owned()), "{orphans:?}");
    assert!(orphans.contains(&"demo v2.png".to_owned()), "{orphans:?}");

    let corpus = store.whole_graph().unwrap().corpus(None, true);
    let copied: Vec<String> =
        super::publication_assets(&store, &corpus, 32 * 1024 * 1024, &mut Vec::new())
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
    let mut expected: Vec<String> = shown
        .iter()
        .chain(&["sub/nested.png"])
        .map(|name| format!("assets/{name}"))
        .collect();
    expected.sort();
    assert_eq!(copied, expected);

    for name in shown {
        assert_eq!(
            trash_asset(&store, name).unwrap(),
            TrashOutcome::Referenced,
            "{name}"
        );
        assert!(root.join("assets").join(name).exists(), "{name}");
    }
    assert_eq!(
        trash_asset(&store, "unused.png").unwrap(),
        TrashOutcome::Trashed
    );
    store.close();
}
