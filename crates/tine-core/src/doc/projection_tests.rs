use super::*;

#[test]
fn runtime_uuid_is_not_serialized() {
    let mut block = DocBlock::new("body");
    block.uuid = "runtime-only".into();
    let value = serde_json::to_value(&block).unwrap();
    assert!(value.get("uuid").is_none());
    assert_eq!(
        serde_json::to_string(&block).unwrap(),
        r#"{"raw":"body","children":[],"is_org":false}"#
    );
}

#[test]
fn editing_raw_invalidates_projection() {
    let mut block = DocBlock::new("before [[Old]]");
    assert!(block.projection().refs_contains("Old"));
    block.set_raw("after [[New]]");
    assert_eq!(block.visible_text(), "after [[New]]");
    assert!(block.projection().refs_contains("New"));
    assert!(!block.projection().refs_contains("Old"));
}

#[test]
fn projection_matches_direct_computation() {
    let b = DocBlock::new("TODO ship [[Foo Bar]] and #tag\nid:: abc\nprop:: secret");
    let p = b.projection();
    // visible_lower == canonical_fold(visible_text(raw)): property lines dropped
    assert_eq!(b.visible_folded(true), "todo ship [[foo bar]] and #tag");
    assert!(
        !b.visible_folded(true).contains("secret"),
        "property values excluded"
    );
    // refs_contains ≡ references_page (case-insensitive, normalized)
    assert!(p.refs_contains("foo bar"));
    assert!(p.refs_contains("TAG"));
    assert!(!p.refs_contains("nope"));
    // memoized (stable across calls); a clone recomputes to an equal projection
    assert_eq!(
        b.projection().visible_folded(&b.raw, true),
        b.visible_folded(true)
    );
    assert_eq!(b.clone().projection().refs_norm(), p.refs_norm());
}

#[test]
fn facets_read_off_one_lsdoc_parse() {
    // marker / heading / properties all come off lsdoc's single parse now.
    let b = DocBlock::new("TODO finish it\nfoo:: bar\nid:: 123");
    assert_eq!(b.marker(), Some("TODO"));
    assert_eq!(b.property("foo").as_deref(), Some("bar"));
    assert_eq!(b.property("id").as_deref(), Some("123"));
    assert_eq!(b.heading_level(), None);
    // STARTED is an mldoc/lsdoc marker (in the recognized set).
    assert_eq!(DocBlock::new("STARTED x").marker(), Some("STARTED"));
    // ATX-heading bullet → level off lsdoc `Bullet.size`.
    assert_eq!(DocBlock::new("## A heading").heading_level(), Some(2));
    assert_eq!(DocBlock::new("plain text").heading_level(), None);
}

#[test]
fn priority_is_header_position_only() {
    // audit C3: lsdoc only treats a header-position `[#A]` as priority; the old
    // `[#A]`-anywhere scanner disagreed with the chip on load.
    assert_eq!(DocBlock::new("TODO [#A] task").priority(), Some("A"));
    assert_eq!(DocBlock::new("Discuss [#A] tags").priority(), None); // mid-text
    assert_eq!(DocBlock::new("TODO task [#A] later").priority(), None); // not after marker
}

/// GH #623: the fallback storage is invisible to readers. A plain block stores
/// neither visible text; a block with a property trailer stores `visible`; a block
/// whose fold differs (case/accents) stores `visible_lower`; each reads back the
/// text the eager form returned.
#[test]
fn visible_text_fallbacks_store_only_what_differs() {
    let plain = DocBlock::new("plain lowercase text");
    assert_eq!(plain.visible_text(), "plain lowercase text");
    assert_eq!(plain.visible_folded(true), "plain lowercase text");
    assert_eq!(plain.visible_folded(false), "plain lowercase text");
    let p = plain.projection();
    assert!(p.stored_visible().is_none() && p.stored_lower().is_none());

    let cased = DocBlock::new("Caf\u{e9} Ship");
    assert_eq!(cased.visible_text(), "Caf\u{e9} Ship");
    assert_eq!(cased.visible_folded(true), "cafe ship");
    assert_eq!(cased.visible_folded(false), "caf\u{e9} ship");
    let p = cased.projection();
    assert!(p.stored_visible().is_none() && p.stored_lower().is_some());

    let props = DocBlock::new("Body Text\nid:: abc\nkey:: v");
    assert_eq!(props.visible_text(), "Body Text");
    assert_eq!(props.visible_folded(true), "body text");
    assert!(props.projection().stored_visible().is_some());

    let mut edited = DocBlock::new("Caf\u{e9}");
    assert_eq!(edited.visible_folded(true), "cafe");
    edited.set_raw("plain");
    assert_eq!(edited.visible_text(), "plain");
    assert_eq!(edited.visible_folded(true), "plain");
    assert!(edited.projection().stored_lower().is_none());
}

/// GH #623: an ordinary block (no refs, properties, planning, tags, marker) carries
/// no `extra` allocation, and the inline projection slot stays small. The bound
/// is the point: the slot is paid by every block of every open graph.
#[test]
fn ordinary_block_projection_stays_small() {
    assert!(
        std::mem::size_of::<BlockProjection>() <= 56,
        "BlockProjection grew to {} B; every block pays this inline (GH #623). \
         Put rarely-set facets in ProjectionExtra.",
        std::mem::size_of::<BlockProjection>()
    );
    assert!(std::mem::size_of::<DocBlock>() <= 144);
    let plain = DocBlock::new("nothing special here");
    assert!(plain.projection().extra.is_none());
    let rich = DocBlock::new("TODO [[Page]] #tag\nkey:: v");
    assert!(rich.projection().extra.is_some());
    // page, tag, and (OG `with-page-refs`) the task marker's page "todo".
    assert_eq!(rich.projection().refs_norm().len(), 3);
    assert_eq!(rich.projection().refs_page().len(), 2);
    assert_eq!(rich.marker(), Some("TODO"));
}

#[test]
fn shared_projection_regions_release_parser_growth_capacity() {
    let raw = "TODO task\nkey:: value\nother:: text\nSCHEDULED: <2026-11-01 Sun>";
    let mut regions = crate::block_regions::parse(raw, false);
    regions.properties.reserve(20);
    regions.literals.reserve(20);
    let expected = regions.clone();
    let shared = shared_regions(regions);
    assert_eq!(*shared, expected);
    assert_eq!(shared.properties.capacity(), shared.properties.len());
    assert_eq!(shared.literals.capacity(), shared.literals.len());
}

#[test]
fn accent_sensitive_projection_cache_rebuilds_after_edit() {
    let mut block = DocBlock::new("café");
    assert_eq!(block.visible_folded(true), "cafe");
    assert_eq!(block.visible_folded(false), "café");
    block.set_raw("cafe");
    assert_eq!(block.visible_folded(false), "cafe");
}

#[test]
fn visible_text_drops_properties_utf8_safe() {
    // Multi-byte body before a trailing property block: the span→raw byte
    // mapping (`span - 2 + lead`) must land on char boundaries, not split UTF-8.
    let b = DocBlock::new("Über café résumé\nid:: 123\nkey:: v");
    assert_eq!(b.visible_text(), "Über café résumé");
    assert_eq!(b.visible_folded(true), "uber cafe resume");
    // leading whitespace in raw (lead > 0) still maps correctly.
    let b2 = DocBlock::new("  héllo\nid:: 9");
    assert_eq!(b2.visible_text().trim(), "héllo");
}

#[test]
fn planning_dates_off_lsdoc_timestamp_code_robust() {
    // Real planning lines → faithful `<…>` date text off lsdoc's Timestamp.
    let b = DocBlock::new("TODO ship it\nSCHEDULED: <2026-06-28 Sun>\nDEADLINE: <2026-07-01 Wed>");
    assert_eq!(b.scheduled(), Some("2026-06-28 Sun"));
    assert_eq!(b.deadline(), Some("2026-07-01 Wed"));
    // The robustness fix: a `DEADLINE:` inside inline code is `Code`, not a
    // Timestamp — so it is NEVER badged (the old regex wrongly badged it).
    let code = DocBlock::new("look at `DEADLINE: <2026-06-28 Sun>` here");
    assert_eq!(
        code.deadline(),
        None,
        "code-embedded planning is not badged"
    );
    assert_eq!(DocBlock::new("plain block").scheduled(), None);
}

#[test]
fn schedule_stays_a_facet_when_body_text_follows() {
    let b = DocBlock::new("Task\nSCHEDULED: <2026-07-13 Mon>\nnotes after the schedule");
    assert_eq!(b.scheduled(), Some("2026-07-13 Mon"));
    let utf8 = DocBlock::new("Überblick\nSCHEDULED: <2026-07-14 Tue>\n続き");
    assert_eq!(utf8.scheduled(), Some("2026-07-14 Tue"));
    let mid =
        DocBlock::new("Discuss SCHEDULED: <2026-07-13 Mon> inline\nnotes after the timestamp");
    assert_eq!(mid.scheduled(), None);
}

#[test]
fn scheduled_and_deadline_sharing_one_line_are_both_planning() {
    // OG renders every Scheduled/Deadline Timestamp as nothing in the body and shows both chips.
    for raw in [
        "Review\nSCHEDULED: <2026-10-30 Thu> DEADLINE: <2026-11-02 Mon .+2d>",
        "Review\nDEADLINE: <2026-11-02 Mon .+2d> SCHEDULED: <2026-10-30 Thu>",
        "Review\n  SCHEDULED: <2026-10-30 Thu>   DEADLINE: <2026-11-02 Mon .+2d>",
    ] {
        let b = DocBlock::new(raw);
        assert_eq!(b.scheduled(), Some("2026-10-30 Thu"), "{raw}");
        assert_eq!(b.deadline(), Some("2026-11-02 Mon .+2d"), "{raw}");
        let body = format!(
            "{:?}",
            strip_planning_lines(crate::render::parse_block(raw, false), raw)
        );
        assert!(
            !body.contains("2026-1"),
            "no stray date in the body: {body}"
        );
        assert!(!body.contains("Timestamp"), "no timestamp left: {body}");
        assert!(body.contains("Review"), "title remains: {body}");
    }
    // Text after the last timestamp of the run stays body content.
    let tail = "Review\nSCHEDULED: <2026-10-30 Thu> DEADLINE: <2026-11-02 Mon> and a tail";
    assert_eq!(DocBlock::new(tail).deadline(), Some("2026-11-02 Mon"));
    let body = format!(
        "{:?}",
        strip_planning_lines(crate::render::parse_block(tail, false), tail)
    );
    assert!(
        body.contains("and a tail") && !body.contains("2026-1"),
        "{body}"
    );
    // Mid-text stays content, even with a second timestamp after it.
    let mid = DocBlock::new("Discuss SCHEDULED: <2026-10-30 Thu> DEADLINE: <2026-11-02 Mon>");
    assert_eq!((mid.scheduled(), mid.deadline()), (None, None));
    // A later timestamp after ordinary words on the line is not line-leading.
    let later =
        DocBlock::new("Review\nSCHEDULED: <2026-10-30 Thu> words DEADLINE: <2026-11-02 Mon>");
    assert_eq!(later.scheduled(), Some("2026-10-30 Thu"));
    assert_eq!(later.deadline(), None);
}

#[test]
fn line_leading_planning_timestamp_keeps_trailing_body_text() {
    for (tag, date) in [
        ("DEADLINE", "2026-07-30 Thu"),
        ("SCHEDULED", "2026-07-29 Wed"),
    ] {
        let raw = format!("TODO x\n{tag}: <{date}>tail");
        let b = DocBlock::new(raw.clone());
        if tag == "DEADLINE" {
            assert_eq!(b.deadline(), Some(date));
        } else {
            assert_eq!(b.scheduled(), Some(date));
        }
        let body = format!(
            "{:?}",
            strip_planning_lines(crate::render::parse_block(&raw, false), &raw)
        );
        assert!(body.contains("x"), "title remains body content: {body}");
        assert!(body.contains("tail"), "suffix remains body content: {body}");
        assert!(
            !body.contains(date),
            "timestamp is removed from body: {body}"
        );
        assert_eq!(b.raw(), raw, "facet projection never rewrites raw");
    }

    let indented = DocBlock::new("TODO x\n  DEADLINE: <2026-07-30 Thu>tail");
    assert_eq!(indented.deadline(), Some("2026-07-30 Thu"));

    // Deliberate OG divergence: only a line-leading timestamp is planning
    // chrome; a mid-text timestamp remains ordinary body content in Tine.
    let mid = DocBlock::new("Discuss DEADLINE: <2026-07-30 Thu> inline");
    assert_eq!(mid.deadline(), None);

    let inline_code = DocBlock::new("`DEADLINE: <2026-07-30 Thu>`");
    assert_eq!(inline_code.deadline(), None);
    let fenced = DocBlock::new("```\nDEADLINE: <2026-07-30 Thu>\n```");
    assert_eq!(fenced.deadline(), None);
}

#[test]
fn org_properties_from_drawer_not_key_colons() {
    // lsdoc correction: in ORG, `key:: val` is plain text (NOT a property);
    // org properties live in a `:PROPERTIES:` drawer. Tine's old line-scan
    // wrongly read org `key::` as a property — routing through lsdoc fixes it.
    let mut drawer = DocBlock::new("task\n:PROPERTIES:\n:id: 6679-abc\n:END:");
    drawer.is_org = true;
    assert_eq!(drawer.property("id").as_deref(), Some("6679-abc"));
    let mut plain = DocBlock::new("note\nfoo:: bar");
    plain.is_org = true;
    assert_eq!(plain.property("foo"), None, "org key:: is not a property");
    assert!(
        plain.visible_text().contains("foo:: bar"),
        "org key:: stays visible"
    );
}

/// The checkpoint form is the flat field list, whatever the in-memory shape.
#[test]
fn projection_wire_is_the_flat_field_list_and_round_trips() {
    #[derive(Serialize)]
    struct Flat<'a> {
        regions: &'a crate::block_regions::BlockRegions,
        visible: Option<&'a str>,
        visible_lower: Option<&'a str>,
        visible_literal: Option<Option<&'a str>>,
        plain_ranges: Vec<std::ops::Range<usize>>,
        extra: &'a Option<Box<ProjectionExtra>>,
    }
    for (raw, literal) in [
        ("plain lowercase", false),
        ("Mixed Case", false),
        ("Mixed Čase", true),
        ("TODO [[Page]] and [[Other]] #tag\nkey:: v", false),
        ("", false),
        ("```\nunclosed fence", false),
        ("id:: aaaaaaaa-0000-0000-0000-000000000001", true),
    ] {
        let block = DocBlock::new(raw);
        let p = block.projection();
        if literal {
            block.visible_folded(false);
        }
        let flat = Flat {
            regions: &p.regions,
            visible: p.stored_visible(),
            visible_lower: p.stored_lower(),
            visible_literal: p.visible_literal.get().map(|l| l.as_ref().map(|t| &***t)),
            plain_ranges: p.plain_ranges.view().iter().collect(),
            extra: &p.extra,
        };
        let bytes = postcard::to_stdvec(p).unwrap();
        assert_eq!(bytes, postcard::to_stdvec(&flat).unwrap(), "{raw}");
        assert_eq!(
            serde_json::to_string(p).unwrap(),
            serde_json::to_string(&flat).unwrap()
        );
        let back: BlockProjection = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(postcard::to_stdvec(&back).unwrap(), bytes, "{raw}");
    }
}

#[test]
fn plain_ranges_past_u32_keep_their_exact_values() {
    let far = (u32::MAX as usize + 1)..(u32::MAX as usize + 9);
    let ranges = PlainRanges::from(vec![far.clone()]);
    assert_eq!(ranges.view().iter().collect::<Vec<_>>(), vec![far]);
    assert!(matches!(
        PlainRanges::from(vec![1..2]),
        PlainRanges::One(1, 2)
    ));
    assert!(matches!(PlainRanges::from(vec![]), PlainRanges::None));
}

#[test]
fn compact_regions_keep_the_existing_flat_wire() {
    use crate::block_regions::*;
    #[derive(Serialize)]
    struct Flat<'a> {
        header: &'a Header,
        literals: &'a [Range],
        property_regions: &'a [Range],
        properties: &'a [Property],
        planning: &'a [Planning],
        drawers: &'a [Drawer],
        id: Option<&'a Property>,
        quarantined: bool,
        literal_blocks: &'a [LiteralBlock],
        open_fence: Option<&'a OpenFence>,
        headline_end: Option<usize>,
    }
    for (raw, org) in [
        ("plain", false),
        ("body\nid:: AAAAAAAA-0000-4000-8000-000000000001", false),
        ("```\nopen", false),
        ("TODO\nSCHEDULED: <2026-11-01 Sun>", false),
        ("\n:PROPERTIES:\n:ID: custom-key\n:END:", true),
        ("body\n:PROPERTIES:\n:ID: custom-key\n:END:", true),
    ] {
        let regions = crate::block_regions::parse(raw, org);
        let flat = Flat {
            header: &regions.header,
            literals: &regions.literals,
            property_regions: &regions.property_regions,
            properties: &regions.properties,
            planning: &regions.planning,
            drawers: &regions.drawers,
            id: regions.id.as_deref(),
            quarantined: regions.quarantined,
            literal_blocks: &regions.literal_blocks,
            open_fence: regions.open_fence.as_deref(),
            headline_end: regions.headline_end,
        };
        let bytes = postcard::to_stdvec(&regions).unwrap();
        assert_eq!(bytes, postcard::to_stdvec(&flat).unwrap(), "{raw}");
        assert_eq!(
            serde_json::to_string(&regions).unwrap(),
            serde_json::to_string(&flat).unwrap(),
            "{raw}"
        );
        let decoded: BlockRegions = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(regions, decoded);
    }
}
