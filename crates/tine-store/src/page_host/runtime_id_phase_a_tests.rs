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
    a3_case(Live::new, "pages/A.md", "pages/B.md", "- bee\n");
}

/// Blocker 2 of the 40c9ce0f4 review: the move completes before the first
/// graph view, with no snapshot to show that A held the key. The receiver's
/// publication registers the keys it took from A's version, so A's rewrite
/// after launch does not hand the key out again. Fails on 40c9ce0f4.
#[test]
fn r4_a_move_before_the_first_view_keeps_the_key_off_the_source() {
    a3_case(Live::unviewed, "pages/A.md", "pages/B.md", "- bee\n");
    a3_case(Live::unviewed, "pages/A.org", "pages/B.org", "* bee\n");
}

/// Blocker 1 of the 40c9ce0f4 review: B moved above A, then a soft line in
/// B that the bytes reparse as a child (`- c`, `** c`). The custody reply,
/// sent while the save is debounced, carries the keys the publication then
/// installs: B never shows A's positional key. Fails on 40c9ce0f4. The
/// variants the review names as unaffected (Markdown `+ c` and `* c` stay
/// text, an exact answer; Org `* c` adds a root) answer as the row too.
#[test]
fn r4_a_reparsed_childs_reply_carries_the_published_keys() {
    let (md, org) = (("pages/p.md", "- A\n- B\n"), ("pages/p.org", "* A\n* B\n"));
    let cases = [
        (md, "B\n- c"),
        (org, "B\n** c"),
        (md, "B\n+ c"),
        (md, "B\n* c"),
        (org, "B\n* c"),
    ];
    for ((rel, text), typed) in cases {
        let live = Live::new(&[(rel, text)]);
        let (key, page) = live.open(rel);
        let save = |dto: &PageDto, version: u64| {
            let (id, session) = (live.id(), live.host.session());
            let kinds = [EditKind::SaveBlock];
            (live.host)
                .submit(session, id, &key, dto, version, None, &kinds)
                .unwrap();
            live.answer(&key, id)
        };
        let mut dto = page_dto(&page);
        dto.blocks.swap(0, 1);
        let version = save(&dto, page.version).answer.unwrap().version;
        let swapped = pairs(&dto.blocks);
        row_when(&live, rel, "B above A", |row| *row == swapped[..]);
        live.submits_wait();
        dto.blocks[0].raw = typed.into();
        let reply = save(&dto, version).page.unwrap();
        let reply = match &reply.text {
            MailText::Page { dto } => pairs(&dto.blocks),
            _ => pairs(&dto.blocks),
        };
        let row = row_when(&live, rel, "the edit published", |row| *row != swapped[..]);
        assert_eq!(reply, row, "{rel} {typed:?}: the reply's keys differ");
        live.host.stop();
    }
}

/// Blocker 1 of the cee055d33 review, at `move_blocks`: the receiver, listed
/// first, carries the moved key as an authored `id::`. The key is still
/// registered to it (the source held it as a runtime key), so the source's
/// rewrite does not hand it out again. Fails on cee055d33 (the first index
/// entry was the receiver's authored one).
#[test]
fn r3_a_receivers_authored_id_does_not_hide_the_moved_keys_holder() {
    let mut doc = crate::model::parse_doc(Path::new("pages/S.md"), "- moved\n");
    tine_core::projection::assign_runtime_ids_reserving(&mut doc.roots, "pages/S.md", None);
    let key = doc.roots[0].uuid.to_string();
    let receiver = format!("- bee\n  id:: {key}\n");
    a3_case(Live::new, "pages/S.md", "pages/R.md", &receiver);
}

fn a3_case(live: fn(&[(&str, &str)]) -> Live, a: &str, b: &str, b_text: &str) {
    let bullet = if a.ends_with(".org") { "*" } else { "-" };
    let text = |body: &str| body.replace('-', bullet);
    let live = live(&[(a, &text("- moved\n- stays\n")), (b, b_text)]);
    let viewed = !live.store.first_view_pending();
    let (a_key, a_page) = live.open(a);
    let (b_key, b_page) = live.open(b);
    let (mut source, mut receiver) = (page_dto(&a_page), page_dto(&b_page));
    let block = source.blocks.remove(0);
    let moved = block.id.clone();
    receiver.blocks.push(block);
    let (id, session) = (live.id(), live.host.session());
    let admit = || {
        (live.host).move_blocks(
            session,
            id,
            (&a_key, &source, a_page.version),
            (&b_key, &receiver, b_page.version),
            &[EditKind::MoveBlocks],
        )
    };
    if viewed {
        admit().unwrap();
    } else {
        // Admitted with no published snapshot (the launch's first view is
        // pending): nothing visible tells that A holds the key.
        live.pause(true);
        live.store.without_snapshot_for_tests(admit).unwrap();
        live.pause(false);
    }
    live.answer(&a_key, id);
    if !viewed {
        live.store.whole_graph_reconciled().unwrap();
    }
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
    let rewrite = text("- new first\n- stays\n");
    live.rewrite_externally(a, &rewrite);
    live.until_indexed(a, &rewrite);
    let row = row_when(&live, a, "A's rewrite", |row| row.len() == 2);
    assert!(
        !row.iter().any(|(_, key)| *key == moved),
        "A reallocated B's key: {row:?}"
    );
    let b_row = row_when(&live, b, "B keeps it", |_| true);
    assert!(b_row.iter().any(|(_, key)| *key == moved), "{b_row:?}");
    live.host.stop();
}
