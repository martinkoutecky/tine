//! I-25: every journal identity read shares immutable compiled formats.
use crate::{Area, Store};
use std::borrow::Borrow;
use std::fs;
use tine_core::date::JournalFormat;

#[test]
fn inventory_reuses_compiled_journal_formats_and_refreshes_them_on_config_change() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("journals")).unwrap();
    fs::create_dir(root.path().join("logseq")).unwrap();
    fs::write(root.path().join("journals/2026_09_29.md"), "- body\n").unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let first = store.graph.current_journal_format();
    let second = store.graph.current_journal_format();
    assert!(std::ptr::eq(Borrow::<JournalFormat>::borrow(&first), Borrow::<JournalFormat>::borrow(&second)),
        "I-25: current_journal_format shares one compiled value; never clone patterns per inventory file");
    let listing = store.scan_area(Area::Journals, None).unwrap();
    assert_eq!(listing.files[0].day.unwrap().0, 20260929);
    fs::write(
        root.path().join("logseq/config.edn"),
        "{:journal/file-name-format \"dd-MM-yyyy\"}\n",
    )
    .unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let changed = store.graph.current_journal_format();
    assert_eq!(
        changed.file_stem(tine_core::date::JournalDate::from_ordinal(20260929)),
        "29-09-2026"
    );
    assert_eq!(
        first.file_format(),
        "yyyy_MM_dd",
        "captured formats remain immutable"
    );
    assert_eq!(
        store.scan_area(Area::Journals, None).unwrap().files[0]
            .day
            .unwrap()
            .0,
        20260929,
        "fallback date stems stay eligible after a custom format change"
    );
    store.close();
}
