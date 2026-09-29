//! C3W W1 (L04): a page whose file name is close to the 255-byte name limit
//! (80 CJK characters, which OG/Logseq writes in place) must stay saveable. The
//! same-directory temp name used to be the file name plus ~15–25 bytes, so every
//! save and create of such a page failed with ENAMETOOLONG.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{PageId, SaveBase, SaveOutcome, Store};

fn scratch() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3w-w1-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    root
}

fn long_title(chars: usize) -> String {
    "漢".repeat(chars)
}

fn dto(name: &str, raw: &str) -> PageDto {
    PageDto {
        name: name.into(),
        kind: PageKind::Page,
        title: name.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "w1".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    }
}

#[test]
fn w1_page_with_a_near_limit_file_name_saves_and_creates() {
    // 84 CJK chars = 252 bytes + ".md" = 255 bytes: the longest legal name. A
    // create is exercised up to 83 chars: at 84, `.org` twin probing in
    // transaction.rs (`disk_twin`) is refused by the OS (recorded C3W pending;
    // outside this lane's write set).
    for chars in [80usize, 83, 84] {
        let root = scratch();
        let title = long_title(chars);
        let rel = format!("pages/{title}.md");
        assert!(rel.len() - "pages/".len() <= 255);
        fs::write(root.join(&rel), "- before\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let id = PageId::from(rel.as_str());
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "after".into();
        let outcome = store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        );
        assert!(
            matches!(outcome, SaveOutcome::Saved(_)),
            "{chars}-char page save: {outcome:?}"
        );
        assert_eq!(fs::read_to_string(root.join(&rel)).unwrap(), "- after\n");

        if chars == 84 {
            let _ = fs::remove_dir_all(&root);
            continue;
        }
        let new_title = "字".repeat(chars);
        let new_rel = format!("pages/{new_title}.md");
        let created = store.save(
            tine_store::EditKind::CreatePage,
            &PageId::from(new_rel.as_str()),
            SaveBase::CreateNew,
            &dto(&new_title, "fresh"),
        );
        assert!(
            matches!(created, SaveOutcome::Saved(_)),
            "{chars}-char page create: {created:?}"
        );
        assert!(fs::read_to_string(root.join(&new_rel))
            .unwrap()
            .contains("fresh"));
        // No temp is left behind in the directory.
        let leftovers: Vec<_> = fs::read_dir(root.join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = fs::remove_dir_all(&root);
    }
}
