//! An own save whose written bytes re-parse into a different tree than the
//! window sent, after a structural edit, must still give every block of the
//! page its own runtime id: in the published index row, in a later read of the
//! unheld page, and so in the next save (GH #800 mechanism). Driven through the
//! real page host and Store, with fully keyed DTOs as the window sends them.
use super::*;
use std::collections::{HashMap, HashSet};
use tine_core::model::BlockDto;

fn ids(blocks: &[BlockDto], into: &mut Vec<(String, String)>) {
    for block in blocks {
        into.push((block.id.clone(), block.raw.clone()));
        ids(&block.children, into);
    }
}

fn doc_ids(blocks: &[tine_core::doc::DocBlock], into: &mut Vec<(String, String)>) {
    for block in blocks {
        into.push((block.uuid.to_string(), block.raw().to_owned()));
        doc_ids(&block.children, into);
    }
}

fn repeated(pairs: &[(String, String)]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    pairs
        .iter()
        .filter(|(id, _)| !seen.insert(id.clone()))
        .cloned()
        .collect()
}

/// The window's load and save of a page DTO (`flatten` then `toDto` in
/// `src/document/convert.ts`): one node per id within a page, the later
/// block of a repeated id replacing the earlier one, children by id.
fn window_round_trip(blocks: &[BlockDto]) -> Vec<BlockDto> {
    fn flatten(blocks: &[BlockDto], nodes: &mut HashMap<String, BlockDto>) -> Vec<String> {
        blocks
            .iter()
            .map(|block| {
                let children = flatten(&block.children, nodes);
                let node = BlockDto {
                    children: children
                        .iter()
                        .map(|id| BlockDto {
                            id: id.clone(),
                            ..Default::default()
                        })
                        .collect(),
                    ..block.clone()
                };
                nodes.insert(block.id.clone(), node);
                block.id.clone()
            })
            .collect()
    }
    fn to_dto(id: &str, nodes: &HashMap<String, BlockDto>) -> BlockDto {
        let node = &nodes[id];
        BlockDto {
            children: node
                .children
                .iter()
                .map(|child| to_dto(&child.id, nodes))
                .collect(),
            ..node.clone()
        }
    }
    let mut nodes = HashMap::new();
    let roots = flatten(blocks, &mut nodes);
    roots.iter().map(|id| to_dto(id, &nodes)).collect()
}

fn until_contains(live: &Live, key: &str, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !live.disk(key).contains(text) {
        assert!(
            Instant::now() < deadline,
            "{text:?} never reached disk: {:?}",
            live.disk(key)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Outline lines of `body` whose text is `text`.
fn lines(body: &str, text: &str) -> usize {
    body.lines()
        .filter(|line| line.trim_start().strip_prefix("- ") == Some(text))
        .count()
}

#[test]
fn an_own_save_that_reparses_into_a_new_block_after_a_move_keeps_every_runtime_id_distinct() {
    let rel = "pages/p.md";
    let live = Live::new(&[(rel, "- A\n\t- a\n- B\n")]);
    let page = PageId::from(rel);
    let (key, opened) = live.open(rel);
    // What the window holds: the opened page's ids (positional ones).
    let loaded = live.store.page(&page).unwrap().doc;
    let (a_root, b_root) = (loaded.blocks[0].clone(), loaded.blocks[1].clone());
    assert_eq!((a_root.raw.as_str(), b_root.raw.as_str()), ("A", "B"));
    let save = |dto: &PageDto, version: u64, kind: EditKind| {
        let id = live.id();
        let session = live.host.session();
        live.host
            .submit(session, id, &key, dto, version, None, &[kind])
            .unwrap();
        live.answer(&key, id).answer.unwrap().version
    };
    // 1. Move B above A: every block keeps its id.
    let mut moved = loaded.clone();
    moved.blocks = vec![b_root.clone(), a_root.clone()];
    let version = save(&moved, opened.version, EditKind::MoveBlocks);
    until_contains(&live, &key, "- B\n- A");
    // 2. In B, Shift+Enter then `- c`: one block's text, which the written
    // bytes re-parse as a child block of B.
    let mut typed = moved.clone();
    typed.blocks[0].raw = "B\n- c".into();
    let version = save(&typed, version, EditKind::SaveBlock);
    until_contains(&live, &key, "- c");
    live.until_published(&[(key.to_string(), version, None)]);
    let mut row = Vec::new();
    let pages = live.store.graph.peek_pages().expect("a built page cache");
    let (_, row_doc) = (pages.iter())
        .find(|(entry, _)| entry.rel_path_str() == rel)
        .expect("the owner's row");
    doc_ids(&row_doc.roots, &mut row);
    assert_eq!(
        row.iter().map(|(_, raw)| raw.as_str()).collect::<Vec<_>>(),
        ["B", "c", "A", "a"],
        "Tine's parser reads the soft line `- c` as a child block"
    );
    // 3. The page leaves the window; a later read of the unheld page.
    live.close(&key);
    let deadline = Instant::now() + Duration::from_secs(20);
    while live.held(&key) {
        assert!(Instant::now() < deadline, "the page was never released");
        std::thread::sleep(Duration::from_millis(20));
    }
    let read = live.store.page(&page).unwrap().doc;
    let mut unheld = Vec::new();
    ids(&read.blocks, &mut unheld);
    // 4. The window installs that read and edits A: the next save.
    let (key, reopened) = live.open(rel);
    let mut next = read.clone();
    next.blocks = window_round_trip(&read.blocks);
    let a = next.blocks.iter_mut().find(|b| b.raw == "A").unwrap();
    a.raw = "A edited".into();
    let id = live.id();
    let session = live.host.session();
    live.host
        .submit(
            session,
            id,
            &key,
            &next,
            reopened.version,
            None,
            &[EditKind::SaveBlock],
        )
        .unwrap();
    live.answer(&key, id);
    until_contains(&live, &key, "A edited");
    let after = live.disk(&key);
    assert_eq!(
        (lines(&after, "a"), lines(&after, "c")),
        (1, 1),
        "the next edit writes each block once (GH #800 mechanism): {after:?}"
    );
    assert_eq!(
        repeated(&row),
        Vec::<(String, String)>::new(),
        "the published own-save row gives two blocks one runtime id: {row:?}"
    );
    assert_eq!(
        repeated(&unheld),
        Vec::<(String, String)>::new(),
        "a read of the unheld page gives two blocks one runtime id: {unheld:?}"
    );
    live.host.stop();
}
