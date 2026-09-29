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
