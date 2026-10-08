//! I-25: parser-owned edit regions must not enlarge every retained block.
use tine_core::doc::{DocBlock, Document};

#[test]
fn retained_blocks_keep_edit_regions_out_of_the_inline_read_projection() {
    assert!(
        std::mem::size_of::<DocBlock>() <= 512,
        "I-25: block storage stays within baseline +20%; share sparse edit regions through DocBlock::projection"
    );
    let mut first = DocBlock::new("ordinary body");
    let second = DocBlock::new("another body");
    assert!(
        std::ptr::eq(
            std::borrow::Borrow::<tine_core::block_regions::BlockRegions>::borrow(
                &first.projection().regions
            ),
            std::borrow::Borrow::<tine_core::block_regions::BlockRegions>::borrow(
                &second.projection().regions
            )
        ),
        "I-25: empty parser-owned regions share one immutable allocation"
    );
    first.set_raw("body\nstatus:: present");
    assert_eq!(first.projection().regions.properties[0].value, "present");
    assert!(second.projection().regions.properties.is_empty());
    let mut copy = first.clone();
    copy.set_raw("body\nstatus:: changed");
    assert_eq!(first.projection().regions.properties[0].value, "present");
    assert_eq!(copy.projection().regions.properties[0].value, "changed");
}

#[test]
fn region_storage_does_not_change_document_serialization() {
    let document = tine_core::doc::parse("- body\n  status:: present\n- plain\n");
    let bytes = serde_json::to_vec(&document).unwrap();
    for block in &document.roots {
        block.projection();
    }
    assert_eq!(serde_json::to_vec(&document).unwrap(), bytes);
    let decoded: Document = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, document);
}

#[test]
fn wire_identity_presence_comes_from_the_existing_parser_projection() {
    for (raw, org, has_id) in [
        ("plain", false, false),
        ("body\nid:: authored", false, true),
        ("```\nid:: literal\n```", false, false),
        ("body\n:PROPERTIES:\n:ID: authored\n:END:", true, true),
        (
            "#+BEGIN_SRC\n:PROPERTIES:\n:ID: literal\n:END:\n#+END_SRC",
            true,
            false,
        ),
    ] {
        let mut block = DocBlock::new(raw);
        block.set_org(org);
        block.uuid = "owned-fixture".into();
        assert_eq!(block.projection().regions.id.is_some(), has_id);
        for dto in [
            tine_core::projection::block_to_dto(&block),
            tine_core::projection::block_to_shallow_dto(&block),
        ] {
            assert_eq!(serde_json::to_value(dto).unwrap()["has_id"], has_id,
                "I-12/I-25: full and shallow DTOs ship parser-owned presence; exemplar projection.rs");
        }
    }
}

#[test]
fn incoming_identity_presence_cannot_replace_raw_in_the_save_projection() {
    for (raw, org, expected) in [
        ("plain", false, None),
        ("body\nid:: authored", false, Some("authored")),
        (
            "body\n:PROPERTIES:\n:ID: authored\n:END:",
            true,
            Some("authored"),
        ),
    ] {
        for has_id in [serde_json::Value::Null, false.into(), true.into()] {
            let dto: tine_core::model::BlockDto = serde_json::from_value(serde_json::json!({
                "id": "runtime", "raw": raw, "has_id": has_id
            }))
            .unwrap();
            let block = tine_core::projection::dto_block_to_doc(&dto, org);
            assert_eq!(
                block
                    .projection()
                    .regions
                    .id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                expected
            );
            assert_eq!(block.raw(), raw);
        }
    }
}

#[test]
fn unprojected_drafts_leave_identity_presence_unknown_and_off_the_wire() {
    let dto: tine_core::model::BlockDto = serde_json::from_value(serde_json::json!({
        "id": "runtime", "raw": "body\nid:: authored"
    }))
    .unwrap();
    assert_eq!(dto.has_id, None);
    assert!(serde_json::to_value(dto).unwrap().get("has_id").is_none());
}

#[test]
fn identity_wire_facts_use_the_shared_projection_answerer() {
    // One constructor spells the identity facts; the deep DTO is that constructor plus children
    // (behaviour pinned by bounded_projection::deep_dto_is_the_shallow_dto_plus_projected_children).
    let source = include_str!("../src/projection.rs");
    assert_eq!(source.matches("has_id: Some(b.projection().regions.id.is_some())").count(), 1,
        "I-12/I-25: DTO constructors reuse cached parser regions through block_to_shallow_dto; exemplar projection.rs::block_to_dto");
}
