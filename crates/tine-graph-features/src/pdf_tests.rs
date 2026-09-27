use super::*;

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
