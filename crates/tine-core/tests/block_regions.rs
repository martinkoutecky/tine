use tine_core::block_regions::{parse, Edit};
#[test]
fn visible_body_only_removes_canonical_org_properties() {
    for nl in ["\n", "\r\n"] {
        let raw = [
            "Heading",
            "SCHEDULED: <2026-09-30 Wed>",
            ":PROPERTIES:",
            ":id: real",
            ":END:",
            "Body",
            ":PROPERTIES:",
            ":note: prose",
            ":END:",
        ]
        .join(nl);
        let regions = parse(&raw, true);
        assert_eq!(regions.id.as_ref().unwrap().value, "real");
        assert!(!regions.property("note").unwrap().primary);
        let expected = [
            "Heading",
            "SCHEDULED: <2026-09-30 Wed>",
            "Body",
            ":PROPERTIES:",
            ":note: prose",
            ":END:",
        ]
        .join(nl);
        assert_eq!(regions.apply(&raw, true, Edit::Visible).unwrap(), expected);
    }
}

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

#[test]
fn page_header_is_the_parsers_leading_property_run() {
    use tine_core::block_regions::{page_header, page_header_only};
    let keys = |raw: &str| -> Vec<String> {
        page_header(raw)
            .entries
            .into_iter()
            .map(|e| e.key)
            .collect()
    };
    assert_eq!(keys("title:: A\n\nalias:: B\nprose"), ["title", "alias"]);
    // I-12 (Martin 2026-10-01): not properties to the parser, so not header lines.
    assert!(keys("title::A").is_empty());
    assert!(keys(" title:: A").is_empty());
    assert!(keys("#tag:: x").is_empty());
    assert!(keys("prose\ntitle:: A").is_empty());
    assert_eq!(keys("title:: A\n```\nx:: y\n```\nalias:: B"), ["title"]);
    let h = page_header("klíč:: é\r\nb:: c");
    assert_eq!(h.entries.len(), 2);
    assert_eq!(h.end, "klíč:: é\r\nb:: c".len());
    assert!(page_header_only("a:: 1\n\nb:: 2").is_some());
    assert!(page_header_only("a:: 1\n").is_none());
    assert!(page_header_only("a:: 1\nprose").is_none());
    assert!(page_header_only("a:: 1\nb::2").is_none());
}

#[test]
fn literal_blocks_and_open_fence_follow_the_parser() {
    // Any fence run closes any opener; trailing blank lines are in the literal.
    let raw = "````text\nalpha\n```\nafter";
    let r = parse(raw, false);
    assert_eq!(r.literal_blocks.len(), 1);
    assert_eq!(
        r.literal_blocks[0].range.slice(raw),
        "````text\nalpha\n```\n"
    );
    assert!(r.open_fence.is_none());
    // `- ```js` opens nothing; the final lone run is the editor-state open fence.
    let raw = "- ```js\nx\n```";
    let r = parse(raw, false);
    assert!(r.literal_blocks.is_empty());
    assert_eq!(
        r.open_fence.as_ref().unwrap().start,
        raw.rfind("```").unwrap()
    );
    // Marker/priority spans are located inside the accepted header only.
    let r = parse("\u{85}TODO [#A] x", false);
    assert_eq!(
        r.header.marker_range.unwrap().slice("\u{85}TODO [#A] x"),
        "TODO"
    );
    assert_eq!(
        r.header.priority_range.unwrap().slice("\u{85}TODO [#A] x"),
        "[#A]"
    );
    assert!(parse("\u{feff}TODO x", false).header.marker.is_none());
}

#[test]
fn header_tokens_door_equals_the_regions_header() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/block-regions.json")).unwrap();
    for f in fixtures.iter().chain(std::iter::once(
        &serde_json::json!({"raw": "\u{85}TODO [#A] x", "org": false}),
    )) {
        let (raw, org) = (f["raw"].as_str().unwrap(), f["org"].as_bool().unwrap());
        assert_eq!(
            tine_core::block_regions::header_tokens(raw, org),
            parse(raw, org).header,
            "{raw:?}"
        );
    }
}

/// GH #721: OG writes `collapsed:: true` after a block's last line, which mldoc
/// folds into a final quote's lazy continuation. The kernel still owns the row:
/// toggling replaces it in place (never appends), removes every copy, and
/// quoted content is never touched.
#[test]
fn trailing_builtin_rows_swallowed_by_a_final_quote_stay_owned() {
    let raw = "Outside\n> Something\ncollapsed:: true";
    let r = parse(raw, false);
    let p = r.property("collapsed").expect("owned");
    assert!(p.primary);
    assert_eq!(p.value, "true");
    let set = |raw: &str, value: Option<&str>| {
        parse(raw, false)
            .apply(
                raw,
                false,
                Edit::Property {
                    key: "collapsed".into(),
                    value: value.map(str::to_string),
                },
            )
            .unwrap()
    };
    assert_eq!(set(raw, Some("true")), raw);
    assert_eq!(set(raw, None), "Outside\n> Something");
    // Already-stacked rows (as in the report) collapse to one on the next write
    // and all disappear on expand.
    let stacked = "Outside\n> Something\ncollapsed:: true\ncollapsed:: true\ncollapsed:: true";
    assert_eq!(parse(stacked, false).properties.len(), 3);
    assert_eq!(set(stacked, Some("true")), raw);
    assert_eq!(set(stacked, None), "Outside\n> Something");
    // Visible text drops the rows.
    assert_eq!(
        parse(stacked, false)
            .apply(stacked, false, Edit::Visible)
            .unwrap(),
        "Outside\n> Something"
    );
    // Quoted rows, ordinary `key::` rows and non-built-in keys stay content.
    for plain in [
        "> collapsed:: true",
        "Outside\n> a\n> collapsed:: true",
        "Outside\n> a\nfoo:: bar",
        "> a\ncollapsed::true",
    ] {
        assert!(
            parse(plain, false).property("collapsed").is_none(),
            "{plain}"
        );
    }
    // A hidden row followed by an ordinary lazy line is not trailing metadata.
    assert!(parse("> a\ncollapsed:: true\nmore", false)
        .property("collapsed")
        .is_none());
    // A trailing accepted property node (id) after the swallowed row is kept.
    let both = "> a\ncollapsed:: true\nid:: 11111111-1111-4111-8111-111111111111";
    let r = parse(both, false);
    assert!(r.property("collapsed").is_some());
    assert!(r.id.is_some());
}

/// GH #721 review B1: the kernel must never claim an identity the parser (and
/// therefore the backend uuid index, which reads the lsdoc Properties facet)
/// does not see. lsdoc folds an uppercase `ID::` row after a final quote into
/// the quote; were the kernel to claim it, Copy block ref would reuse that id
/// without writing a parser-accepted row and the `((uuid))` would not resolve.
#[test]
fn a_swallowed_row_never_becomes_a_frontend_only_block_id() {
    let uuid = "6a1b0000-0000-4000-8000-000000000001";
    for raw in [
        format!("Intro\n> quoted\nID:: {uuid}"),
        format!("> quoted\nId:: {uuid}"),
        format!("> quoted\ncollapsed:: true\nID:: {uuid}"),
        format!("> quoted\nid:: {uuid}"),
        format!("> quoted\ncollapsed:: true\nid:: {uuid}"),
    ] {
        let kernel = parse(&raw, false).id.map(|p| p.value);
        let facet = tine_core::DocBlock::new(raw.clone()).property("id");
        assert_eq!(kernel, facet, "{raw:?}");
    }
}

#[test]
fn swallowed_rows_leave_the_render_ast_but_not_quoted_text() {
    use lsdoc::ast::{Block, Inline};
    fn quote_text(blocks: &[Block]) -> String {
        let mut out = String::new();
        for b in blocks {
            if let Block::Quote { children, .. } = b {
                for c in children {
                    if let Block::Paragraph { inline, .. } = c {
                        for i in inline {
                            match i {
                                Inline::Plain { text, .. } => out.push_str(text),
                                Inline::Break { .. } => out.push('|'),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        out
    }
    for (raw, expected) in [
        ("Outside\n> Something\ncollapsed:: true", "Something|"),
        (
            "Outside\n> Something\ncollapsed:: true\ncollapsed:: true\ncollapsed:: true",
            "Something|",
        ),
        ("> a\n> b\ncollapsed:: true", "a|b|"),
        ("> a\nmore\ncollapsed:: true", "a|more|"),
        // Not metadata: kept as text.
        ("> a\nfoo:: bar", "a|foo:: bar|"),
        ("> a\n> collapsed:: true", "a|collapsed:: true|"),
    ] {
        let mut blocks = tine_core::render::parse_block(raw, false);
        tine_core::block_regions::strip_swallowed_rows(raw, &mut blocks);
        assert_eq!(quote_text(&blocks), expected, "{raw:?}");
    }
}
