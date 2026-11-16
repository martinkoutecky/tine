use crate::doc::DocBlock;

#[test]
fn nested_candidate_confirmation_keeps_one_source_parse_per_block() {
    use crate::render::BLOCK_PARSES;
    for org in [false, true] {
        for n in [1, 10, 100] {
            let bullet = if org { "*" } else { "-" };
            let text: String = (0..n)
                .map(|i| format!("{bullet} {i} [[a [[X]]]]\n"))
                .collect();
            BLOCK_PARSES.with(|p| p.set(0));
            let proposal = crate::refs::rename_proposal(
                &text,
                org,
                &std::collections::HashMap::from([("a [[x]]".to_owned(), "b [[Y]]".to_owned())]),
                &std::collections::HashMap::new(),
                crate::config::FileNameFormat::TripleLowbar,
            )
            .unwrap();
            assert_eq!(
                proposal.text,
                if org {
                    text.clone()
                } else {
                    text.replace("a [[X]]", r"b \[\[Y\]\]")
                }
            );
            // One source read, then one Org or two Markdown candidate confirmations.
            assert_eq!(
                BLOCK_PARSES.with(|p| p.get()),
                if org { 2 * n } else { 3 * n }
            );
        }
    }
}

#[test]
fn reader_name_extents_and_nesting_come_from_the_same_projection() {
    for (org, raw, want) in [
        (
            false,
            "[[a [[Old]]]]",
            vec![
                ("a [[Old]]", "a [[Old]]", false, false),
                ("Old", "Old", true, false),
            ],
        ),
        (
            false,
            "#[[Old]]",
            vec![("Old", "Old", false, false), ("Old", "Old", false, true)],
        ),
        (
            false,
            "#[[Old]]Tail",
            vec![
                ("OldTail", "[[Old]]Tail", false, false),
                ("Old", "Old", true, false),
            ],
        ),
        (
            false,
            "#[[a [[Old]]]]",
            vec![
                ("[[a [[Old]]]]", "[[a [[Old]]]]", false, false),
                ("a [[Old]]", "a [[Old]]", true, false),
                ("Old", "Old", true, false),
            ],
        ),
        (
            false,
            "[a [[Old]]](file:x.md)",
            vec![("a [[Old]]", "a [[Old]]", false, false)],
        ),
        (
            true,
            "[[file:x.org][a [[Old]]]]",
            vec![("a [[Old]]", "a [[Old]]", false, false)],
        ),
        (true, "head :a[[Old]]:", vec![("Old", "Old", false, false)]),
    ] {
        let mut block = DocBlock::new(raw);
        block.set_org(org);
        let source = block.read_for_rename();
        let got: Vec<_> = source
            .explicit
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    &raw[r.slot.extent.clone()],
                    r.slot.nested,
                    r.slot.encoding,
                )
            })
            .collect();
        assert_eq!(got, want, "{raw:?}");
    }
    // These native per-level assertions belonged to the first pass's held
    // projection test; only the transient output owns them now.
    let source = DocBlock::new("[[Outer [[Inner]]]]").read_for_rename();
    assert_eq!(source.explicit[0].range, 0..19);
    assert_eq!(source.explicit[1].range, 8..17);
    assert!(!source.explicit[0].slot.nested);
    assert!(source.explicit[1].slot.nested);
}

#[test]
fn escaped_name_runs_keep_all_native_bytes() {
    for raw in [
        r"[[New\\\\Tail]]",
        r"[**New\\\\Tail**](file:x.md)",
        r"tags:: New\\\\Tail, Other",
    ] {
        let block = DocBlock::new(raw);
        let source = block.read_for_rename();
        let r = source
            .explicit
            .iter()
            .find(|r| r.name == r"New\\Tail")
            .unwrap();
        assert_eq!(&raw[r.slot.run().unwrap().clone()], r"New\\\\Tail");
    }
}

#[test]
fn indexing_keeps_original_reference_bytes_without_name_slot_work() {
    use super::walk::SLOT_WORK;
    use crate::render::BLOCK_PARSES;
    for (raw, expected) in [
        ("[[Old]]", vec![("Old", 0..7, "explicit_link")]),
        (
            "[[a [[Old]]]]",
            vec![
                ("Old", 0..13, "explicit_nested_link"),
                ("a [[Old]]", 0..13, "explicit_nested_link"),
            ],
        ),
        (
            "#[[Old]]Tail",
            vec![
                ("OldTail", 0..12, "explicit_tag"),
                ("Old", 1..8, "explicit_link"),
            ],
        ),
        (
            r"tags:: New\\\\Tail, Other",
            vec![
                (r"New\\Tail", 7..16, "implicit_linkable_property"),
                ("Other", 18..23, "implicit_linkable_property"),
            ],
        ),
    ] {
        SLOT_WORK.with(|n| n.set(0));
        BLOCK_PARSES.with(|n| n.set(0));
        let block = DocBlock::new(raw);
        let projection = block.projection();
        assert_eq!(BLOCK_PARSES.with(|n| n.get()), 1, "{raw:?}");
        assert_eq!(SLOT_WORK.with(|n| n.get()), 0, "indexing {raw:?}");
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(name, range, rule)| super::ProjectedPageRef {
                name: name.to_owned(),
                range,
                rule,
            })
            .collect();
        assert_eq!(projection.reference_source().explicit, expected, "{raw:?}");
        assert_eq!(
            postcard::to_stdvec(projection.reference_source().explicit).unwrap(),
            postcard::to_stdvec(
                &expected
                    .iter()
                    .map(|r| (&r.name, &r.range, r.rule))
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
        let before = postcard::to_stdvec(projection).unwrap();
        let reading = block.read_for_rename();
        assert!(SLOT_WORK.with(|n| n.get()) > 0);
        drop(reading);
        assert!(std::ptr::eq(projection, block.projection()));
        assert_eq!(postcard::to_stdvec(block.projection()).unwrap(), before);
    }
}

#[test]
fn rename_and_observation_share_one_file_local_reading() {
    use crate::render::BLOCK_PARSES;
    for org in [false, true] {
        let mut block = DocBlock::new("[[a [[Old]]]] {{renderer [[Old]]}}");
        block.set_org(org);
        BLOCK_PARSES.with(|n| n.set(0));
        let weak = {
            let readings = super::RenameReadings::default();
            let first = readings.read(&block);
            let second = readings.read(&block);
            assert!(std::rc::Rc::ptr_eq(&first, &second));
            assert_eq!(BLOCK_PARSES.with(|n| n.get()), 1);
            assert_eq!(first.explicit.len(), 2);
            assert_eq!(first.targets.len(), 1);
            std::rc::Rc::downgrade(&first)
        };
        assert!(
            weak.upgrade().is_none(),
            "file-local evidence must be released"
        );
    }
}

#[test]
fn rename_proposal_keeps_one_source_parse_per_block() {
    use crate::render::BLOCK_PARSES;
    for n in [1, 10, 100] {
        for (old, parses) in [("Other", n), ("Old", 2 * n)] {
            let text: String = (0..n).map(|i| format!("- {i} [[{old}]]\n")).collect();
            BLOCK_PARSES.with(|p| p.set(0));
            let proposal = crate::refs::rename_proposal(
                &text,
                false,
                &std::collections::HashMap::from([("old".to_owned(), "New".to_owned())]),
                &std::collections::HashMap::new(),
                crate::config::FileNameFormat::TripleLowbar,
            )
            .unwrap();
            assert_eq!(
                proposal.text,
                if old == "Old" {
                    text.replace("[[Old]]", "[[New]]")
                } else {
                    text
                }
            );
            assert_eq!(BLOCK_PARSES.with(|p| p.get()), parses);
        }
    }
}
