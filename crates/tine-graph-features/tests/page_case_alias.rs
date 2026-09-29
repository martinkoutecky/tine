//! GH #597 (master 997254442450): on a filesystem that ignores case (Windows,
//! default macOS) a tab, Recent entry or sidebar item saved with another case
//! spelling of a page file's name still opens the file, under its disk
//! spelling. A case-sensitive filesystem (Linux CI) has no alias to follow, so
//! the positive test detects that and checks only the negative half there;
//! the Windows CI lane and macOS run the whole test. The route re-key half
//! lives in `src/components/Page.test.tsx`.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_store::{PageId, Store, StoreError};

fn fixture(label: &str) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-case-alias-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("pages/contents.md"), "- body\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

fn volume_ignores_case(root: &std::path::Path) -> bool {
    fs::symlink_metadata(root.join("pages/CONTENTS.md")).is_ok()
}

#[test]
fn a_path_saved_under_another_case_spelling_opens_the_file() {
    let (root, store) = fixture("open");
    let ignores_case = volume_ignores_case(&root);
    let alias = PageId::from("pages/Contents.md");
    if ignores_case {
        let read = store
            .page(&alias)
            .expect("the saved spelling opens the file");
        assert_eq!(
            read.id.as_str(),
            "pages/contents.md",
            "disk spelling is handed back"
        );
        assert_eq!(read.doc.name, "contents");
    } else {
        assert!(matches!(store.page(&alias), Err(StoreError::NotFound)));
    }
    // Never a different page: a name that differs by more than case is absent.
    assert!(matches!(
        store.page(&PageId::from("pages/Contents-2.md")),
        Err(StoreError::NotFound)
    ));
    let exact = store
        .page(&PageId::from("pages/contents.md"))
        .expect("the exact spelling always opens");
    assert_eq!(exact.id.as_str(), "pages/contents.md");
    drop(store);
    let _ = fs::remove_dir_all(root);
}
