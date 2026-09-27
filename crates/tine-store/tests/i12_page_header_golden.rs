use std::fs;
use std::path::Path;
use tine_core::model::PageDto;
use tine_store::{PageId, SaveBase, SaveOutcome, Store};

#[test]
fn rust_save_matches_shared_page_header_golden() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/i12-page-header-golden.json");
    let mut fixture: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fixture_path).unwrap()).unwrap();
    let cases = fixture["cases"].as_array_mut().unwrap();
    assert!(
        !cases.is_empty(),
        "I-12: page-header golden needs save fixtures"
    );

    let root = std::env::temp_dir().join(format!(
        "tine-i12-header-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let update = std::env::var("TINE_UPDATE_GOLDEN").as_deref() == Ok("1");
    for (index, case) in cases.iter_mut().enumerate() {
        let doc: PageDto = serde_json::from_value(case["input"].clone()).unwrap();
        let id = PageId::from(format!("pages/I12_{index}.md"));
        let outcome = store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::CreateNew,
            &doc,
        );
        assert!(
            matches!(outcome, SaveOutcome::Saved { .. }),
            "I-12: Rust save oracle rejected fixture {index}: {outcome:?}"
        );
        let saved = store.page(&id).unwrap().doc;
        let actual = serde_json::json!({
            "pre_block": saved.pre_block,
            "blocks": saved.blocks.iter().map(|block| &block.raw).collect::<Vec<_>>()
        });
        if update {
            case["expected"] = actual;
        } else {
            assert_eq!(
                case["expected"], actual,
                "I-12: Rust save must match the shared page-header golden; exemplar tine_core::model::first_root_is_promotable_page_header, fixture {index}"
            );
        }
    }
    drop(store);
    fs::remove_dir_all(root).unwrap();
    if update {
        fs::write(
            &fixture_path,
            format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap()),
        )
        .unwrap();
    }
}
