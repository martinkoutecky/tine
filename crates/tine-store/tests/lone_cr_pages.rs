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
    path: String,
    store: Store,
    id: PageId,
}

impl Page {
    fn new(source: &str) -> Self {
        Self::at("pages/p.md", source)
    }

    fn at(path: &str, source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-lone-cr-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::write(root.join(path), source).unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        Page {
            root,
            path: path.into(),
            store,
            id: PageId::from(path),
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
        fs::read_to_string(self.root.join(&self.path)).unwrap()
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
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
    let original = source(|i| if i % 3 == 0 { "\r\n" } else { "\n" });
    let page = Page::new(&original);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].children[0].raw = "deep!".into();
    });
    assert_eq!(edited, original.replace("\t\t\t- deep", "\t\t\t- deep!"));
}

#[test]
fn lone_cr_page_without_a_final_terminator_keeps_its_last_line() {
    let _integration_guard = crate::integration_lock();
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

/// An org page (K01a follow-up): headlines separate blocks, bodies are verbatim.
const ORG: &[&str] = &[
    "#+title: Fix",
    "",
    "* one",
    "body line",
    "** child",
    "* two",
    "* three",
];

fn org_source(ending: impl Fn(usize) -> &'static str) -> String {
    ORG.iter()
        .enumerate()
        .map(|(i, line)| format!("{line}{}", ending(i)))
        .collect()
}

#[test]
fn lone_cr_org_page_is_editable_and_parses_like_its_lf_twin() {
    let _integration_guard = crate::integration_lock();
    let lf = Page::at("pages/p.org", &org_source(|_| "\n"));
    let cr = Page::at("pages/p.org", &org_source(|_| "\r"));
    let (lf_doc, cr_doc) = (lf.doc(), cr.doc());
    assert!(!cr_doc.read_only, "a lone-CR org page is editable");
    assert_eq!(shape(&cr_doc), shape(&lf_doc));
    assert_eq!(cr_doc.pre_block, lf_doc.pre_block);
}

#[test]
fn lone_cr_org_page_keeps_its_bytes_through_saves() {
    let _integration_guard = crate::integration_lock();
    let original = org_source(|_| "\r");
    let page = Page::at("pages/p.org", &original);
    for _ in 0..3 {
        assert_eq!(page.save(EditKind::ReplacePage, |_| {}), original);
    }
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[2].raw = "three!".into();
    });
    assert_eq!(edited, original.replace("* three", "* three!"));
    let body = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "one\nbody line\nsecond body line".into();
    });
    assert_eq!(
        body,
        original
            .replace("* three", "* three!")
            .replace("body line\r", "body line\rsecond body line\r")
    );

    let page = Page::at("pages/p.org", &original);
    let inserted = page.save(EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(1, block("new"));
    });
    assert_eq!(inserted, original.replace("* two", "* new\r* two"));
}

#[test]
fn mixed_crlf_and_cr_org_page_keeps_each_untouched_terminator() {
    let _integration_guard = crate::integration_lock();
    let original = org_source(|i| if i % 2 == 0 { "\r\n" } else { "\r" });
    let page = Page::at("pages/p.org", &original);
    assert!(!page.doc().read_only);
    assert_eq!(page.doc().blocks.len(), 3);
    assert_eq!(page.save(EditKind::ReplacePage, |_| {}), original);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        // A CRLF org line keeps its `\r` in the verbatim body (pre-K01a model).
        doc.blocks[2].raw = doc.blocks[2].raw.replace("three", "three!");
    });
    assert_eq!(edited, original.replace("* three", "* three!"));
}

/// A CRLF org page with the constructs that can sit next to a line ending: page
/// keywords, a blank line, a body and a list item, a keyword-only headline, a
/// property drawer, and an empty headline (`*`, as OG writes an empty block)
/// with a tags-only child.
const ORG_CENSUS: &[&str] = &[
    "#+title: Fix",
    "#+filetags: :x:",
    "",
    "* one",
    "body line",
    "- item",
    "",
    "** child",
    "* TODO",
    "* two",
    ":PROPERTIES:",
    ":id: 6679-abc",
    ":END:",
    "*",
    "** :tag:",
    "* three",
];

fn census_source(ending: &str) -> String {
    ORG_CENSUS
        .iter()
        .map(|line| format!("{line}{ending}"))
        .collect()
}

#[test]
fn crlf_org_page_with_an_empty_headline_is_editable_like_its_lf_twin() {
    let _integration_guard = crate::integration_lock();
    let lf = Page::at("pages/p.org", &census_source("\n"));
    let crlf = Page::at("pages/p.org", &census_source("\r\n"));
    let (lf_doc, crlf_doc) = (lf.doc(), crlf.doc());
    assert!(!lf_doc.read_only);
    assert!(!crlf_doc.read_only, "a CRLF org page with `*` is editable");
    assert_eq!(shape(&crlf_doc), shape(&lf_doc));
    assert_eq!(crlf_doc.pre_block, lf_doc.pre_block);
    assert!(
        shape(&crlf_doc).iter().all(|(raw, _)| !raw.contains('\r')),
        "no stray CR in the model: {:?}",
        shape(&crlf_doc)
    );
    // A CRLF file Tine cannot reproduce stays read-only, like its LF twin.
    let skipped = Page::at("pages/p.org", "* a\r\n*** c\r\n");
    assert!(skipped.doc().read_only);
}

#[test]
fn crlf_and_lf_org_pages_keep_their_bytes_through_saves() {
    let _integration_guard = crate::integration_lock();
    for e in ["\n", "\r\n"] {
        let original = census_source(e);
        let page = Page::at("pages/p.org", &original);
        for _ in 0..3 {
            assert_eq!(page.save(EditKind::ReplacePage, |_| {}), original, "{e:?}");
        }
        let edited = page.save(EditKind::SaveBlock, |doc| {
            doc.blocks[4].raw = "three!".into();
        });
        let mut want = original.replace("* three", "* three!");
        assert_eq!(edited, want, "{e:?}");
        let filled = page.save(EditKind::SaveBlock, |doc| {
            doc.blocks[3].raw = "filled".into();
        });
        want = want.replace(&format!("{e}*{e}"), &format!("{e}* filled{e}"));
        assert_eq!(filled, want, "{e:?}: the empty headline is editable");
        // A line typed into a block takes the file's convention (K01a).
        let typed = page.save(EditKind::SaveBlock, |doc| {
            doc.blocks[0].raw = doc.blocks[0]
                .raw
                .replace("body line", "body line\nsecond body line");
        });
        want = want.replace(
            &format!("body line{e}"),
            &format!("body line{e}second body line{e}"),
        );
        assert_eq!(typed, want, "{e:?}");
        let props = page.save(EditKind::SaveBlock, |doc| {
            doc.pre_block = doc.pre_block.as_ref().map(|p| p.replace(":x:", ":x:y:"));
        });
        want = want.replace(":x:", ":x:y:");
        assert_eq!(props, want, "{e:?}");

        let page = Page::at("pages/p.org", &original);
        let inserted = page.save(EditKind::InsertBlocks, |doc| {
            doc.blocks.insert(4, block("new"));
        });
        assert_eq!(
            inserted,
            original.replace("* three", &format!("* new{e}* three")),
            "{e:?}: a new block takes the file's convention"
        );
    }
}

#[test]
fn crlf_org_page_without_a_final_terminator_keeps_its_last_line() {
    let _integration_guard = crate::integration_lock();
    let original = "* a\r\n*\r\n* b";
    let page = Page::at("pages/p.org", original);
    assert!(!page.doc().read_only);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "a!".into();
    });
    assert_eq!(edited, "* a!\r\n*\r\n* b");
    let appended = page.save(EditKind::InsertBlocks, |doc| {
        doc.blocks.push(block("d"));
    });
    assert_eq!(appended, "* a!\r\n*\r\n* b\r\n* d");
    // A body still carrying CRLF (pasted text, a draft saved before the org
    // model was LF-canonical) is written with single CRLF terminators.
    let pasted = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "a!\r\npasted\r".into();
    });
    assert_eq!(pasted, "* a!\r\npasted\r\n*\r\n* b\r\n* d");
}

/// Review r1 (DS-4): a block that moves keeps its own lines' terminators in a
/// mixed-ending file; only edited and new lines take positional or convention
/// endings. Duplicate block texts each keep their own.
#[test]
fn mixed_ending_org_page_keeps_moved_blocks_terminators() {
    let _integration_guard = crate::integration_lock();
    let page = Page::at("pages/p.org", "* a\r\n* b\n* c\r\n");
    let moved = page.save(EditKind::ReplacePage, |doc| doc.blocks.swap(0, 1));
    assert_eq!(moved, "* b\n* a\r\n* c\r\n");

    let page = Page::at("pages/p.org", "* x\r\n* x\n* y\r\n");
    let front = page.save(EditKind::ReplacePage, |doc| {
        let y = doc.blocks.remove(2);
        doc.blocks.insert(0, y);
    });
    assert_eq!(front, "* y\r\n* x\r\n* x\n");

    // A moved block's body lines move with it; an indent change keeps them too.
    let page = Page::at("pages/p.org", "* a\r\nbody\n* b\n* c\r\n");
    let nested = page.save(EditKind::ReplacePage, |doc| {
        let a = doc.blocks.remove(0);
        doc.blocks[0].children.push(a);
    });
    assert_eq!(nested, "* b\n** a\r\nbody\n* c\r\n");
}

/// The Markdown sibling of the review r1 case.
#[test]
fn mixed_ending_markdown_page_keeps_moved_blocks_terminators() {
    let _integration_guard = crate::integration_lock();
    let page = Page::new("- a\r\n- b\n- c\r\n");
    let moved = page.save(EditKind::ReplacePage, |doc| doc.blocks.swap(0, 1));
    assert_eq!(moved, "- b\n- a\r\n- c\r\n");
}

/// Review r2: inside a changed block, an untouched line keeps its own
/// terminator and an inserted line takes the file's convention; a pasted lone
/// `\r` in block text is a line break that shifts no other block's endings.
#[test]
fn mixed_ending_org_page_keeps_untouched_lines_of_a_changed_block() {
    let _integration_guard = crate::integration_lock();
    let page = Page::at("pages/p.org", "* a\r\nbody\n* b\r\n");
    let inserted = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "a\nnew\nbody".into();
    });
    assert_eq!(inserted, "* a\r\nnew\r\nbody\n* b\r\n");
}

#[test]
fn mixed_ending_org_page_with_a_pasted_lone_cr_keeps_other_blocks_terminators() {
    let _integration_guard = crate::integration_lock();
    let page = Page::at("pages/p.org", "* a\r\n* b\n* c\r\n");
    let pasted = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "a\rpasted".into();
    });
    assert_eq!(pasted, "* a\r\npasted\r\n* b\n* c\r\n");
}

/// Every untouched line of a changed pre-block or block keeps its own ending,
/// wherever it sits; an inserted line takes the convention, an edited one the
/// ending of the line it replaces.
#[test]
fn mixed_ending_org_page_keeps_each_untouched_line_of_a_changed_preamble() {
    let _integration_guard = crate::integration_lock();
    let page = Page::at("pages/p.org", "#+title: T\r\n#+filetags: :x:\n* a\r\n");
    let directive = page.save(EditKind::SaveBlock, |doc| {
        doc.pre_block = Some("#+title: T\n#+author: A\n#+filetags: :x:".into());
    });
    assert_eq!(
        directive,
        "#+title: T\r\n#+author: A\r\n#+filetags: :x:\n* a\r\n"
    );
}

#[test]
fn mixed_ending_org_page_keeps_each_untouched_line_of_a_changed_block() {
    let _integration_guard = crate::integration_lock();
    let page = Page::at("pages/p.org", "* a\r\nx\ny\r\nz\n* b\r\n");
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "a\nnew\nx\ny\nz!".into();
    });
    assert_eq!(edited, "* a\r\nnew\r\nx\ny\r\nz!\n* b\r\n");
}
