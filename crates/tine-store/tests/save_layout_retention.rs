//! G6 (og 13a): a save rewrites only the physical lines of blocks it changed.
//! Every case goes through the real `Store::save` entry point and asserts the
//! exact bytes on disk.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::{BlockDto, PageDto};
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

/// Layout the DTO cannot express: every root under a base tab, a depth jump
/// (a child three tabs deep), a whitespace-only continuation line, trailing
/// spaces, and page properties separated by a blank line.
const SOURCE: &str = "title:: Fix\ntags:: a\n\n\t- one\n\t  cont  \n\t  \n\t\t\t- deep\n\t- two\n\t\t- child\n\t- three";

struct Page {
    root: PathBuf,
    store: Store,
    id: PageId,
}

impl Page {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-layout-retention-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::write(root.join("pages/p.md"), source).unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        Page {
            root,
            store,
            id: PageId::from("pages/p.md"),
        }
    }

    /// Apply `edit` to the loaded DTO, save it, and return the disk bytes.
    fn save(&self, kind: EditKind, edit: impl FnOnce(&mut PageDto)) -> String {
        let read = self.store.page(&self.id).unwrap();
        let mut doc = read.doc;
        edit(&mut doc);
        let outcome = self
            .store
            .save(kind, &self.id, SaveBase::Existing(read.rev), &doc);
        assert!(
            matches!(outcome, SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_)),
            "{outcome:?}"
        );
        fs::read_to_string(self.root.join("pages/p.md")).unwrap()
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        self.store.close();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn block(raw: &str) -> BlockDto {
    BlockDto {
        raw: raw.into(),
        ..Default::default()
    }
}

fn saved(source: &str, kind: EditKind, edit: impl FnOnce(&mut PageDto)) -> String {
    Page::new(source).save(kind, edit)
}

#[test]
fn eof_blank_lines_do_not_grow_on_repeated_saves() {
    // F1: n trailing newlines became 2n-1 on every whole-page save.
    for source in ["- a\n- b\n\n\n", "- a\n-\n\n", "- a\n- b\n- c\n\n\n"] {
        let page = Page::new(source);
        for _ in 0..4 {
            assert_eq!(page.save(EditKind::ReplacePage, |_| {}), source);
        }
        let inserted = page.save(EditKind::InsertBlocks, |doc| {
            doc.blocks.insert(0, block("new"));
        });
        assert_eq!(inserted, format!("- new\n{source}"));
        let deleted = page.save(EditKind::DeleteBlocks, |doc| {
            doc.blocks.remove(0);
        });
        assert_eq!(deleted, source);
    }
}

#[test]
fn eof_blank_lines_survive_whole_page_serialization() {
    // The fallback serializer must not double them either.
    use tine_core::doc;
    for source in ["- a\n- b\n\n\n", "- a\n-\n\n", "x:: y\n\n- a\n\n\n\n"] {
        let parsed = doc::parse(source);
        let opts = doc::SerializeOpts::detect(Some(source));
        assert_eq!(doc::serialize_with(&parsed, &opts), source);
    }
}

#[test]
fn structural_saves_keep_untouched_block_bytes() {
    let insert = saved(SOURCE, EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(1, block("new"));
    });
    assert_eq!(insert, SOURCE.replace("\t- two", "\t- new\n\t- two"));

    let paste = saved(SOURCE, EditKind::InsertBlocks, |doc| {
        doc.blocks[0].children.push(block("p1"));
        doc.blocks[0].children.push(block("p2"));
    });
    assert_eq!(
        paste,
        SOURCE.replace("\t\t\t- deep\n", "\t\t\t- deep\n\t\t\t- p1\n\t\t\t- p2\n")
    );

    let delete = saved(SOURCE, EditKind::DeleteBlocks, |doc| {
        doc.blocks.remove(1);
    });
    assert_eq!(delete, SOURCE.replace("\t- two\n\t\t- child\n", ""));

    let moved = saved(SOURCE, EditKind::MoveBlocks, |doc| {
        doc.blocks.swap(1, 2);
    });
    assert_eq!(
        moved,
        SOURCE.replace(
            "\t- two\n\t\t- child\n\t- three",
            "\t- three\n\t- two\n\t\t- child"
        )
    );
}

#[test]
fn indent_and_outdent_rebase_only_the_moved_subtree() {
    let indented = saved(SOURCE, EditKind::MoveBlocks, |doc| {
        let two = doc.blocks.remove(1);
        doc.blocks[0].children.push(two);
    });
    // `two` joins `deep` at its sibling's column; `child` stays deeper.
    assert_eq!(
        indented,
        SOURCE.replace("\t- two\n\t\t- child", "\t\t\t- two\n\t\t\t\t- child")
    );

    let outdented = saved(SOURCE, EditKind::MoveBlocks, |doc| {
        let child = doc.blocks[1].children.remove(0);
        doc.blocks.insert(2, child);
    });
    assert_eq!(outdented, SOURCE.replace("\t\t- child", "\t- child"));
}

#[test]
fn page_property_and_multi_block_edits_keep_other_lines() {
    let props = saved(SOURCE, EditKind::SaveBlock, |doc| {
        doc.pre_block = Some("title:: Fix\ntags:: a, b".into());
    });
    assert_eq!(props, SOURCE.replace("tags:: a\n", "tags:: a, b\n"));

    let multi = saved(SOURCE, EditKind::SaveBlock, |doc| {
        doc.blocks[1].raw = "two!".into();
        doc.blocks[2].raw = "three!".into();
    });
    assert_eq!(
        multi,
        SOURCE
            .replace("\t- two", "\t- two!")
            .replace("\t- three", "\t- three!")
    );
}

#[test]
fn crlf_pages_keep_untouched_bytes_and_line_endings() {
    let source = format!("{}\r\n", SOURCE.replace('\n', "\r\n"));
    let edited = saved(&source, EditKind::SaveBlock, |doc| {
        doc.blocks[2].raw = "three!".into();
    });
    assert_eq!(edited, source.replace("\t- three", "\t- three!"));

    let inserted = saved(&source, EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(1, block("new"));
    });
    assert_eq!(inserted, source.replace("\t- two", "\t- new\r\n\t- two"));
}
