//! Round-13 inputs for the operation comparisons (`tine-graph-features`,
//! `CM_BRIDGE`). The static comparison runs against the confirmed model
//! (`cm_conformance_k_shared_tests.rs`); this module retires with the
//! operation tests that use it.
use super::*;

#[test]
fn stored_query_inputs_keep_recursive_names_and_opaque_tokens_in_both_formats() {
    let base = r#"{{query (and [[Old]] (not (page "Branch")))}} {{tine-query @page and name = 'Other'}} {{query (task TODO)}}"#;
    for org in [false, true] {
        let literal = if org {
            "~{{query [[Hidden]]}}~"
        } else {
            "`{{query [[Hidden]]}}`"
        };
        let syntax = stored_queries(&format!("{base} {literal}"), org);
        assert_eq!(syntax.len(), 3, "literal macro is protected");
        let mut args = Vec::new();
        for q in &syntax {
            query_args(q, &mut args);
        }
        assert_eq!(args, ["Old", "Branch", "Other"]);
        assert_eq!(
            syntax[0],
            json!({"and": [{"refs": "Old"}, {"not": {"page": {"name": "Branch"}}}]})
        );
        assert!(syntax[2]["given"].as_str().unwrap().contains("task"));
    }
}

pub fn is_bridge(bridge: &Path) -> bool {
    run_bridge(bridge, &json!({})).expect("the selected bridge must run")["round"] == "r13"
}

pub fn fixtures(manifest: &str) -> Vec<(GraphSpec, Value)> {
    let path = fixture_dir(manifest).join("r13/cases.json");
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    cases
        .into_iter()
        .map(|v| {
            (
                GraphSpec::from_fixture(v["name"].as_str().unwrap(), &v),
                v["expect"].clone(),
            )
        })
        .collect()
}

pub fn assert_contract(spec: &GraphSpec, model: &Value, expected: &Value) {
    assert_eq!(model["round"], "r13", "{}", spec.name);
    assert!(
        harness_reports(model).is_empty(),
        "{}: {:?}",
        spec.name,
        harness_reports(model)
    );
    if let Some(paths) = expected.get("activeFiles") {
        assert_eq!(&model["activeFiles"], paths, "{}", spec.name);
    }
    for (i, size) in expected["querySizes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        assert_eq!(
            model["queries"][i].as_array().unwrap().len() as u64,
            size.as_u64().unwrap(),
            "{}",
            spec.name
        );
    }
    for (s, page) in expected["navigate"].as_object().into_iter().flatten() {
        assert_eq!(&model["navigate"][s], page, "{}: {s}", spec.name);
    }
    for (i, operation) in model["renames"].as_array().unwrap().iter().enumerate() {
        let candidates = operation["candidates"].as_array().unwrap();
        let offers = operation["offers"].as_array().unwrap();
        assert_eq!(
            operation["cancel"], model["physicalFiles"],
            "cancel is byte-exact"
        );
        assert_eq!(
            operation["automatic"].is_null(),
            offers.len() != 1,
            "unattended cardinality"
        );
        let ids: BTreeSet<_> = candidates
            .iter()
            .map(|c| c["candidate"].to_string())
            .collect();
        assert_eq!(
            ids.len(),
            candidates.len(),
            "deduplicate literal/member identities"
        );
        for candidate in candidates {
            let checks = candidate["checks"].as_object().unwrap();
            assert_eq!(checks.len(), 7);
            assert_eq!(candidate["safe"], checks.values().all(|v| v == true));
            assert_eq!(candidate["answerAllowed"], candidate["safe"]);
            assert_eq!(
                offers.contains(&candidate["candidate"]),
                candidate["safe"] == true
            );
            assert_eq!(
                candidate["favoritesPage"],
                json!(spec
                    .favorites_page
                    .clone()
                    .unwrap_or_else(|| NO_FAVORITES.to_owned()))
            );
            for row in candidate["images"].as_array().unwrap() {
                for image in row["after"].as_array().unwrap() {
                    assert!(
                        candidate["files"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|f| f["path"] == image[0]),
                        "origin images name retained physical outputs"
                    );
                }
            }
            assert!(
                !candidate.to_string().contains("__missing_path__"),
                "{}: missing codec input",
                spec.name
            );
        }
        if let Some(e) = expected["operations"].get(i) {
            if let Some(pages) = e.get("afterPages") {
                for candidate in candidates {
                    assert_eq!(&candidate["meaning"]["pages"], pages, "{}", spec.name);
                }
            }
            if let Some(v) = e.get("offers") {
                assert_eq!(&operation["offers"], v, "{}", spec.name);
            }
            if let Some(v) = e.get("preferred") {
                assert_eq!(&operation["preferred"], v, "{}", spec.name);
            }
            if let Some(block) = e.get("unchangedPreview") {
                let row = candidates[0]["preview"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["before"] == *block)
                    .expect("unchanged spelling must appear in the preview");
                assert_eq!(
                    row["beforeSignature"]["spellings"],
                    row["afterSignature"]["spellings"]
                );
                assert_ne!(
                    row["beforeSignature"]["openings"],
                    row["afterSignature"]["openings"]
                );
            }
            for (j, failed) in e["failedChecks"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                for name in failed.as_array().unwrap() {
                    assert_eq!(
                        candidates[j]["checks"][name.as_str().unwrap()],
                        false,
                        "{}: {name}",
                        spec.name
                    );
                }
            }
        }
    }
}
