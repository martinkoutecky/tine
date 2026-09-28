use super::*;
use tine_store::FaultPoint;

#[test]
fn conflict_read_refuses_malformed_sidecar_instead_of_reporting_empty_disk() {
    let root = std::env::temp_dir().join(format!("tine-pdf-conflict-read-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/paper.edn"), "not edn").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        read_highlights_checked(&store, "paper.pdf")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_sidecar_crop_can_be_rolled_back_to_recoverable_trash() {
    let root = std::env::temp_dir().join(format!(
        "tine-pdf-crop-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    let sidecar = b"{:highlights [] :extra {}}";
    std::fs::write(root.join("assets/paper.edn"), sidecar).unwrap();
    assert!(root.join("assets").join(&rel).is_file());
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "../crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).unwrap();
    assert_eq!(
        std::fs::read(root.join("assets/paper.edn")).unwrap(),
        sidecar
    );
    assert!(!root.join("assets").join(&rel).exists());
    assert_eq!(
        std::fs::read_dir(root.join("logseq/.tine-trash/assets"))
            .unwrap()
            .count(),
        1
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn crop_rollback_refuses_a_current_sidecar_reference_or_malformed_sidecar() {
    let root = std::env::temp_dir().join(format!("tine-pdf-crop-guard-{}", std::process::id()));
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    let sidecar = root.join("assets/paper.edn");
    std::fs::write(&sidecar, r#"{:highlights [{:id "crop-id" :page 1 :position {:page 1 :bounding {:top 1 :left 1 :width 2 :height 2} :rects []} :content {:image 42} :properties {:color "yellow"}}] :extra {}}"#).unwrap();
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    std::fs::write(&sidecar, "not edn").unwrap();
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    std::fs::write(
        &sidecar,
        "{:highlights [{:id \"crop-id\" :content {:image 42}}]}",
    )
    .unwrap();
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn crop_rollback_keeps_crop_when_sidecar_changes_at_transaction_guard() {
    let root = std::env::temp_dir().join(format!("tine-pdf-crop-race-{}", std::process::id()));
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    std::fs::write(root.join("assets/paper.edn"), "{:highlights [] :extra {}}").unwrap();
    store.inject_fault(FaultPoint::Stage2Mismatch);
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn highlight_deletion_retires_its_crop_through_the_guarded_sidecar_path() {
    let root = std::env::temp_dir().join(format!("tine-pdf-delete-crop-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rect = pdf::Rect {
        top: 1.0,
        left: 1.0,
        width: 2.0,
        height: 2.0,
        source_width: None,
        source_height: None,
    };
    let area = Highlight {
        id: "crop-id".into(),
        page: 1,
        position: pdf::Position {
            page: 1,
            bounding: rect.clone(),
            rects: vec![rect],
        },
        color: "yellow".into(),
        text: None,
        image: Some(42),
    };
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    write_highlights(&store, "paper.pdf", "Paper", &[area.clone()], &[]).unwrap();
    assert!(root.join("assets").join(&rel).is_file());
    write_highlights(&store, "paper.pdf", "Paper", &[], &[area]).unwrap();
    assert!(!root.join("assets").join(&rel).exists());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn optional_page_checks_org_headline_depth_and_preserves_sidecar_reads() {
    let dir = std::env::temp_dir().join(format!("tine-pdf-org-depth-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(
        dir.join("pages/notes.org"),
        format!("{} deep\n", "*".repeat(513)),
    )
    .unwrap();
    std::fs::write(dir.join("assets/notes.edn"), "{:ok true}").unwrap();
    let store = Store::open(&dir, Default::default()).unwrap().0;
    let page = store.file_id(Area::Pages, "notes.org").unwrap();
    let sidecar = store.file_id(Area::Assets, "notes.edn").unwrap();
    assert_eq!(
        optional(&store, &page).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(optional(&store, &sidecar).unwrap().unwrap().0, "{:ok true}");
    std::fs::remove_dir_all(dir).unwrap();
}
