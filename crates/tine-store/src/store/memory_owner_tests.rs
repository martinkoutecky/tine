//! Browser core slice 5a, round 3: one owner per question. A saved page's
//! blocks hold distinct runtime ids in a memory graph, as on the desktop
//! (`page_host::binding_tests::an_editor_save_holds_each_runtime_id_once`),
//! and a change that is not graph text never reaches the page validator.
use super::memory_replace_tests::{assert_opens_alike, open, Files};
use crate::{ChangeKind, FileId, FileRev, PageId};
use std::collections::{BTreeMap, BTreeSet};
use tine_core::model::{BlockDto, PageDto};

const AUTHORED: &str = "6566c5a4-0000-4000-8000-000000000003";

fn label(block: &BlockDto) -> &str {
    block.raw.split_whitespace().next().unwrap_or("")
}

/// Every block's (label, runtime id), in page order.
fn ids(blocks: &[BlockDto], out: &mut Vec<(String, String)>) {
    for block in blocks {
        out.push((label(block).to_owned(), block.id.clone()));
        ids(&block.children, out);
    }
}

/// An unkeyed block, as an editor or API hands a new one over.
fn new_block(like: &BlockDto) -> BlockDto {
    let mut block = like.clone();
    (block.id, block.raw, block.children) = (String::new(), "omega".into(), Vec::new());
    block.has_id = None;
    block
}

fn find<'a>(blocks: &'a mut [BlockDto], name: &str) -> &'a mut BlockDto {
    let at = blocks.iter().position(|b| label(b) == name).unwrap();
    &mut blocks[at]
}

pub(crate) type Edit = fn(&mut PageDto);

/// The review's cases: each puts an unkeyed block at the position whose
/// positional id a kept block holds (the authored-id block among them).
pub(crate) fn edits() -> [(&'static str, Edit); 3] {
    [
        ("prepend a root", |dto| {
            let new = new_block(&dto.blocks[0]);
            dto.blocks.insert(0, new);
        }),
        ("insert a child", |dto| {
            let new = new_block(&dto.blocks[0]);
            find(&mut dto.blocks, "beta").children.insert(0, new);
        }),
        ("append after reparenting", |dto| {
            let new = new_block(&dto.blocks[0]);
            let gamma = find(&mut dto.blocks, "beta").children.remove(0);
            dto.blocks.push(gamma);
            find(&mut dto.blocks, "beta").children.push(new);
        }),
    ]
}

/// The page in each format and line ending: `alpha` has an authored id.
pub(crate) fn pages() -> Vec<(&'static str, bool, String)> {
    let md = format!("- alpha\n  id:: {AUTHORED}\n- beta\n  - gamma\n- zeta\n");
    let org = format!("* alpha\n:PROPERTIES:\n:id: {AUTHORED}\n:END:\n* beta\n** gamma\n* zeta\n");
    let mut out = Vec::new();
    for (rel, text) in [("pages/Note.md", md), ("pages/Note.org", org)] {
        out.push((rel, false, text.clone()));
        out.push((rel, true, text.replace('\n', "\r\n")));
    }
    out
}

/// After the save of `edited` (from `before`), the page read back holds each
/// runtime id once, each kept block keeps its id, and the authored id stays.
pub(crate) fn assert_distinct(case: &str, before: &PageDto, after: &PageDto) {
    let (mut kept, mut now) = (Vec::new(), Vec::new());
    ids(&before.blocks, &mut kept);
    ids(&after.blocks, &mut now);
    let distinct: BTreeSet<_> = now.iter().map(|(_, id)| id).collect();
    assert_eq!(distinct.len(), now.len(), "{case}: an id repeats: {now:?}");
    let now: BTreeMap<_, _> = now.into_iter().collect();
    for (label, id) in kept {
        assert_eq!(now.get(&label), Some(&id), "{case}: {label} kept its id");
    }
    assert!(now.contains_key("omega"), "{case}: the new block is there");
    let alpha = after.blocks.iter().find(|b| label(b) == "alpha").unwrap();
    assert!(
        alpha.raw.contains(AUTHORED),
        "{case}: the authored id stays"
    );
}

/// Each label's count in `text`: a collapsed pair writes one block twice.
fn assert_each_once(case: &str, text: &str) {
    for label in ["alpha", "beta", "gamma", "zeta", "omega"] {
        assert_eq!(
            text.matches(label).count(),
            1,
            "{case}: {label} in {text:?}"
        );
    }
}

/// Review finding 1: a memory graph's submit gives every unkeyed block an id
/// no other block of the page holds; the next save writes each block once.
#[test]
fn a_submitted_page_holds_each_runtime_id_once() {
    for (rel, crlf, text) in pages() {
        for (edit, apply) in edits() {
            let case = format!("{rel} crlf={crlf} {edit}");
            let mut files = Files::new();
            files.insert("logseq/config.edn".into(), b"{}\n".to_vec());
            files.insert(rel.into(), text.clone().into_bytes());
            let mut view = open(&files);
            let id = PageId::from(rel);
            let before = view.page(&id).unwrap();
            let mut dto = before.clone();
            apply(&mut dto);
            view.submit_page(&id, &dto, true).unwrap();
            let after = view.page(&id).unwrap();
            assert_distinct(&case, &before, &after);
            let (bytes, _) = view.submit_page(&id, &after, true).unwrap();
            assert_each_once(&case, std::str::from_utf8(&bytes).unwrap());
        }
    }
}

/// Review finding 3: a change that is not graph text keeps its file change
/// and revision, and neither admission nor publication checks it as page
/// text; a page write still is checked. The view answers as a fresh open.
#[test]
fn an_opaque_write_never_reaches_the_page_validator() {
    let opaque = [
        "assets/a.md",
        "assets/a.org",
        "assets/a.bin",
        "publish/p.md",
        "logseq/bak/pages/b.md",
        "node_modules/m.md",
        ".hidden/h.md",
        "pages/.dot.md",
        "pages/Note.sync-conflict-20260101-000000-ABCDEFG.md",
    ];
    let mut files = Files::new();
    files.insert("logseq/config.edn".into(), b"{}\n".to_vec());
    files.insert("pages/Note.md".into(), b"- first\n".to_vec());
    let mut view = open(&files);
    let checks = crate::cost_counters::thread_parse_input_checks;
    for (step, rel) in opaque.iter().enumerate() {
        let bytes = format!("- opaque text {step} [[Note]]\n").into_bytes();
        files.insert(rel.to_string(), bytes.clone());
        let before = checks();
        let change = (view.replace_files(vec![(rel.to_string(), Some(bytes.clone()))])).unwrap();
        assert_eq!(checks() - before, 0, "{rel}: checked as page text");
        let file = (FileId::from(rel.to_string()), ChangeKind::Created);
        let rev = Some(FileRev::from_bytes(&bytes));
        assert_eq!(change.files, vec![(file.0, file.1, rev)], "{rel}");
        assert_opens_alike(&view, &files);
    }
    let before = checks();
    let page = b"- second\n".to_vec();
    files.insert("pages/Note.md".into(), page.clone());
    view.replace_files(vec![("pages/Note.md".into(), Some(page))])
        .unwrap();
    assert!(checks() > before, "a page write is checked");
    assert_opens_alike(&view, &files);
}
