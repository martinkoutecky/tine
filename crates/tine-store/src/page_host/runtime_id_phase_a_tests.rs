//! Phase A of `RUNTIME-ID-MODEL.md` v3 through the desktop's page host and
//! a real store: the window's submissions are fully keyed (F15). Each test
//! fails on 16cee4693.
use super::*;

/// Every block's (raw, id), in page order.
fn pairs(blocks: &[tine_core::model::BlockDto]) -> Vec<(String, String)> {
    fn walk(blocks: &[tine_core::model::BlockDto], out: &mut Vec<(String, String)>) {
        for block in blocks {
            out.push((block.raw.clone(), block.id.clone()));
            walk(&block.children, out);
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

/// The index row of `rel`, once `test` holds of it.
fn row_when(
    live: &Live,
    rel: &str,
    what: &str,
    test: impl Fn(&[(String, String)]) -> bool,
) -> Vec<(String, String)> {
    let path = live.root.join(rel);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let row = live.store.graph.with_pages(|pages| {
            let row = pages.iter().find(|(entry, _)| entry.path == path);
            let blocks: Vec<_> = (row.into_iter())
                .flat_map(|(_, doc)| doc.roots.iter().map(tine_core::projection::block_to_dto))
                .collect();
            pairs(&blocks)
        });
        if test(&row) {
            return row;
        }
        assert!(Instant::now() < deadline, "{rel}: {what}; row {row:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn page_dto(page: &MailPage) -> PageDto {
    let MailText::Page { dto } = &page.text else {
        panic!("the answer carries the page: {:?}", page.text);
    };
    PageDto::clone(dto)
}

/// A1 (desktop, held): two identical bullets swapped in the editor and
/// saved. The bytes do not change; the index takes the swapped keys, and
/// the publication announces the page.
#[test]
fn a1_an_identical_bullet_swap_reaches_the_index() {
    for (rel, text) in [("pages/a.md", "- x\n- x\n"), ("pages/a.org", "* x\n* x\n")] {
        let live = Live::new(&[(rel, text)]);
        let (key, page) = live.open(rel);
        let dto = page_dto(&page);
        let before = pairs(&dto.blocks);
        let mut swapped = dto.clone();
        swapped.blocks.swap(0, 1);
        let rev = live.store.whole_graph().unwrap().rev();
        let (id, session) = (live.id(), live.host.session());
        let kinds = [EditKind::SaveBlock];
        (live.host)
            .submit(session, id, &key, &swapped, page.version, None, &kinds)
            .unwrap();
        assert!(live.answer(&key, id).answer.unwrap().took, "{rel}");
        let want = vec![before[1].clone(), before[0].clone()];
        row_when(&live, rel, "the swapped keys", |row| row == want);
        assert_eq!(live.disk(rel), text, "{rel}: the bytes did not change");
        let deadline = Instant::now() + Duration::from_secs(20);
        while live.store.whole_graph().unwrap().rev() == rev {
            assert!(Instant::now() < deadline, "{rel}: no publication");
            std::thread::sleep(Duration::from_millis(20));
        }
        live.host.stop();
    }
}

/// A3 (desktop): a block moved from A to B keeps its key (as today); a raw
/// rewrite of A whose first position hashes to that key gives A's block
/// another key, so no key has two pages.
#[test]
fn a3_a_source_rewrite_never_reallocates_a_moved_key() {
    let (a, b) = ("pages/A.md", "pages/B.md");
    let live = Live::new(&[(a, "- moved\n- stays\n"), (b, "- bee\n")]);
    let (a_key, a_page) = live.open(a);
    let (b_key, b_page) = live.open(b);
    let (mut source, mut receiver) = (page_dto(&a_page), page_dto(&b_page));
    let block = source.blocks.remove(0);
    let moved = block.id.clone();
    receiver.blocks.push(block);
    let (id, session) = (live.id(), live.host.session());
    (live.host)
        .move_blocks(
            session,
            id,
            (&a_key, &source, a_page.version),
            (&b_key, &receiver, b_page.version),
            &[EditKind::MoveBlocks],
        )
        .unwrap();
    live.answer(&a_key, id);
    row_when(&live, b, "B holds the moved key", |row| {
        row.iter().any(|(_, key)| *key == moved)
    });
    row_when(&live, a, "A gave the key up", |row| {
        !row.iter().any(|(_, key)| *key == moved)
    });
    // An external program rewrites A: its first block sits where the moved
    // one was, at the position whose structural key B now holds.
    live.close(&a_key);
    let deadline = Instant::now() + Duration::from_secs(20);
    while live.held(a) {
        assert!(Instant::now() < deadline, "A never released");
        std::thread::sleep(Duration::from_millis(20));
    }
    live.rewrite_externally(a, "- new first\n- stays\n");
    live.until_indexed(a, "- new first\n- stays\n");
    let row = row_when(&live, a, "A's rewrite", |row| row.len() == 2);
    assert!(
        !row.iter().any(|(_, key)| *key == moved),
        "A reallocated B's key: {row:?}"
    );
    let b_row = row_when(&live, b, "B keeps it", |_| true);
    assert!(b_row.iter().any(|(_, key)| *key == moved), "{b_row:?}");
    live.host.stop();
}
