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
