use tine_core::block_regions::{parse, Edit};
#[test]
fn structural_edits_preserve_literals_and_reparse() {
    for org in [false, true] {
        let code = if org {
            "#+BEGIN_SRC text\nid:: literal\n:LOGBOOK:\nCLOCK: [2026-09-29 Tue 09:00]\n:END:\n#+END_SRC"
        } else {
            "```\nid:: literal\n:LOGBOOK:\nCLOCK: [2026-09-29 Tue 09:00]\n:END:\n```"
        };
        let raw = format!("žluťoučký\n{code}");
        let r = parse(&raw, org);
        assert!(r.properties.is_empty());
        assert!(r.drawers.is_empty());
        assert_eq!(r.literals.len(), 1);
        let next = r
            .apply(
                &raw,
                org,
                Edit::Property {
                    key: "klíč".into(),
                    value: Some("値".into()),
                },
            )
            .unwrap();
        assert!(next.contains(code));
        assert_eq!(parse(&next, org).property("klíč").unwrap().value, "値");
        let clean = parse(&next, org)
            .apply(
                &next,
                org,
                Edit::Property {
                    key: "klíč".into(),
                    value: None,
                },
            )
            .unwrap();
        assert_eq!(clean, raw);
    }
}
#[test]
fn parity_fixture_is_identical_to_native_regions() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/block-regions.json")).unwrap();
    let expected: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/block-regions-native.json")).unwrap();
    assert_eq!(fixtures.len(), expected.len());
    for (f, e) in fixtures.iter().zip(expected) {
        assert_eq!(
            serde_json::to_value(parse(
                f["raw"].as_str().unwrap(),
                f["org"].as_bool().unwrap()
            ))
            .unwrap(),
            e
        );
    }
}

#[test]
fn folded_directive_metadata_survives_id_removal() {
    let raw = "Task\n:PROPERTIES:\n:id: original\n:END:\n#+OWNER: retained";
    let r = parse(raw, true);
    assert!(r
        .properties
        .iter()
        .any(|p| p.key.eq_ignore_ascii_case("owner")));
    let out = r
        .apply(
            raw,
            true,
            Edit::Property {
                key: "id".into(),
                value: None,
            },
        )
        .unwrap();
    assert!(out.contains("#+OWNER: retained"));
}

#[test]
fn removing_adjacent_duplicate_metadata_preserves_the_body() {
    for raw in [
        "Body\nid:: first\nid:: second",
        "Body\r\nid:: first\r\nid:: second",
    ] {
        let out = parse(raw, false)
            .apply(raw, false, Edit::StripCopy { template: false })
            .unwrap();
        assert_eq!(out, "Body");
    }
}

#[test]
fn legacy_trailing_property_moves_to_the_head_without_touching_body() {
    let raw = "Title\nbody\nold:: legacy";
    let out = parse(raw, false)
        .apply(
            raw,
            false,
            Edit::Property {
                key: "old".into(),
                value: Some("new".into()),
            },
        )
        .unwrap();
    assert_eq!(out, "Title\nold:: new\nbody");
}

#[test]
fn glued_planning_edits_detach_and_preserve_body_suffixes() {
    for org in [false, true] {
        for nl in ["\n", "\r\n"] {
            let raw =
                format!("Task{nl}DEADLINE: <2026-07-07 Tue>tail{nl}DEADLINE: <2026-07-08 Wed>尾");
            let regions = parse(&raw, org);
            let changed = regions
                .apply(
                    &raw,
                    org,
                    Edit::Planning {
                        which: "Deadline".into(),
                        value: Some("<2026-07-30 Thu>".into()),
                    },
                )
                .unwrap();
            assert_eq!(
                changed,
                format!("Task{nl}DEADLINE: <2026-07-30 Thu>{nl}tail{nl}尾")
            );
            let removed = regions
                .apply(
                    &raw,
                    org,
                    Edit::Planning {
                        which: "Deadline".into(),
                        value: None,
                    },
                )
                .unwrap();
            assert_eq!(removed, format!("Task{nl}tail{nl}尾"));
            let normalized = regions.apply(&raw, org, Edit::NormalizePlanning).unwrap();
            assert_eq!(normalized, format!("Task{nl}DEADLINE: <2026-07-07 Tue>{nl}DEADLINE: <2026-07-08 Wed>{nl}tail{nl}尾"));
            let inline = "Discuss DEADLINE: <2026-07-07 Tue>tail";
            assert!(parse(inline, org).planning.is_empty());
        }
    }
}
