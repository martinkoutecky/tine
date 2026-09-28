//! K01a (og 15a, I-4): a Markdown page whose lines end in a lone `\r` (classic
//! Mac, some tools, a sync peer) parses with `\r` as a line break, as mldoc and
//! lsdoc do (`eol_chars = ['\r'; '\n']`), and a save keeps every untouched line's
//! bytes, including its own terminator. Mixed-ending pages keep each untouched
//! line's terminator too. Every case goes through the real `Store::save`.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::{BlockDto, PageDto};
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

/// The layout fixture of `save_layout_retention.rs`, one line per entry.
const LINES: &[&str] = &[
    "title:: Fix",
    "tags:: a",
    "",
    "\t- one",
    "\t  cont  ",
    "\t  ",
    "\t\t\t- deep",
    "\t- two",
    "\t\t- child",
    "\t- three",
];

/// Join `LINES` with the terminator `ending(i)` after line `i` (the last line
/// is terminated too).
fn source(ending: impl Fn(usize) -> &'static str) -> String {
    LINES
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{line}{}", ending(i)))
        .collect()
}

struct Page {
    root: PathBuf,
    store: Store,
    id: PageId,
}

impl Page {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-lone-cr-{}-{}",
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

    fn doc(&self) -> PageDto {
        self.store.page(&self.id).unwrap().doc
    }

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

fn shape(doc: &PageDto) -> Vec<(String, usize)> {
    fn walk(blocks: &[BlockDto], out: &mut Vec<(String, usize)>) {
        for b in blocks {
            out.push((b.raw.clone(), b.children.len()));
            walk(&b.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    out
}

#[test]
fn lone_cr_page_parses_like_its_lf_twin() {
    let lf = Page::new(&source(|_| "\n"));
    let cr = Page::new(&source(|_| "\r"));
    let (lf_doc, cr_doc) = (lf.doc(), cr.doc());
    assert_eq!(cr_doc.blocks.len(), 3, "lone CR separates blocks");
    assert_eq!(shape(&cr_doc), shape(&lf_doc));
    assert_eq!(cr_doc.pre_block, lf_doc.pre_block);
    assert!(
        shape(&cr_doc).iter().all(|(raw, _)| !raw.contains('\r')),
        "no stray CR in the model: {:?}",
        shape(&cr_doc)
    );
}

#[test]
fn lone_cr_page_keeps_its_bytes_through_saves() {
    let original = source(|_| "\r");
    let page = Page::new(&original);
    for _ in 0..3 {
        assert_eq!(page.save(EditKind::ReplacePage, |_| {}), original);
    }
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[2].raw = "three!".into();
    });
    assert_eq!(edited, original.replace("\t- three", "\t- three!"));

    let page = Page::new(&original);
    let inserted = page.save(EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(1, block("new"));
    });
    assert_eq!(inserted, original.replace("\t- two", "\t- new\r\t- two"));

    let page = Page::new(&original);
    let props = page.save(EditKind::SaveBlock, |doc| {
        doc.pre_block = Some("title:: Fix\ntags:: a, b".into());
    });
    assert_eq!(props, original.replace("tags:: a\r", "tags:: a, b\r"));
}

#[test]
fn mixed_crlf_and_cr_page_keeps_each_untouched_terminator() {
    let original = source(|i| if i % 2 == 0 { "\r\n" } else { "\r" });
    let page = Page::new(&original);
    assert_eq!(page.doc().blocks.len(), 3);
    assert_eq!(page.save(EditKind::ReplacePage, |_| {}), original);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[1].raw = "two!".into();
    });
    assert_eq!(edited, original.replace("\t- two\r", "\t- two!\r"));

    // A new line takes the file's CRLF convention (as a CRLF page's does).
    let page = Page::new(&original);
    let inserted = page.save(EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(1, block("new"));
    });
    assert_eq!(
        inserted,
        original.replace("\t- two\r", "\t- new\r\n\t- two\r")
    );
}

#[test]
fn mixed_crlf_and_lf_page_keeps_each_untouched_terminator() {
    let original = source(|i| if i % 3 == 0 { "\r\n" } else { "\n" });
    let page = Page::new(&original);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].children[0].raw = "deep!".into();
    });
    assert_eq!(edited, original.replace("\t\t\t- deep", "\t\t\t- deep!"));
}

#[test]
fn lone_cr_page_without_a_final_terminator_keeps_its_last_line() {
    let original = "- a\r- b\r\t- c";
    let page = Page::new(original);
    assert_eq!(page.doc().blocks.len(), 2);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "a!".into();
    });
    assert_eq!(edited, "- a!\r- b\r\t- c");
    let appended = page.save(EditKind::InsertBlocks, |doc| {
        doc.blocks.push(block("d"));
    });
    assert_eq!(appended, "- a!\r- b\r\t- c\r- d");
}
