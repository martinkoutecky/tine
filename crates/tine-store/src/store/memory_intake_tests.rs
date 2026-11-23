//! A2 (`RUNTIME-ID-FINDINGS.md`, R5): the browser's `submit_page` is an
//! untrusted entrance. A key submitted on two blocks, or one another page
//! holds, is kept by one block only: the page and the graph never hold one
//! key twice. Fails on 16cee4693 (both blocks published with it).
use super::memory_replace_tests::{open, Files};
use crate::PageId;
use std::collections::BTreeSet;
use tine_core::model::BlockDto;

fn ids(blocks: &[BlockDto], out: &mut Vec<String>) {
    for block in blocks {
        out.push(block.id.clone());
        ids(&block.children, out);
    }
}

#[test]
fn a2_a_submitted_key_names_one_block() {
    for (rel, other, text) in [
        ("pages/Note.md", "pages/Other.md", "- one\n- two\n- three\n"),
        (
            "pages/Note.org",
            "pages/Other.org",
            "* one\n* two\n* three\n",
        ),
    ] {
        let mut files = Files::new();
        files.insert("logseq/config.edn".into(), b"{}\n".to_vec());
        files.insert(rel.into(), text.as_bytes().to_vec());
        files.insert(other.into(), text.as_bytes().to_vec());
        let mut view = open(&files);
        let (id, other_id) = (PageId::from(rel), PageId::from(other));
        let theirs = view.page(&other_id).unwrap().blocks[0].id.clone();
        let mut dto = view.page(&id).unwrap();
        // The second block repeats the first's key; the third names a
        // block of the other page.
        dto.blocks[1].id = dto.blocks[0].id.clone();
        dto.blocks[2].id = theirs.clone();
        view.submit_page(&id, &dto, true).unwrap();
        let mut now = Vec::new();
        ids(&view.page(&id).unwrap().blocks, &mut now);
        let distinct: BTreeSet<_> = now.iter().collect();
        assert_eq!(distinct.len(), now.len(), "{rel}: one key twice: {now:?}");
        assert_eq!(now[0], dto.blocks[0].id, "{rel}: the first holder keeps it");
        let mut elsewhere = Vec::new();
        ids(&view.page(&other_id).unwrap().blocks, &mut elsewhere);
        assert!(
            elsewhere.contains(&theirs),
            "{rel}: the other page keeps its key"
        );
        assert!(!now.contains(&theirs), "{rel}: two pages hold {theirs}");
    }
}
