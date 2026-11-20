use super::*;

#[test]
fn marker_stem_names_a_folded_day_in_either_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("logseq")).unwrap();
    std::fs::write(
        dir.path().join("logseq/config.edn"),
        "{:journal/page-title-format \"/yyyyMMdd/\"}\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    for kind in [PageKind::Journal, PageKind::Page] {
        assert_eq!(
            marker_stem_answer(&store, kind, "20261102"),
            (PageKind::Journal, "/20261102/".into())
        );
    }
    store.close();
}
