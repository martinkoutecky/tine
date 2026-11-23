//! REG-RUNTIME-ID-SAME-OUTLINE: after an editor's structural save, the index
//! row carries the window's runtime ids, which are not positional. Bytes Tine
//! did not save but which hold the same outline (other line terminators, a
//! final terminator added or removed) publish no page change, so they must
//! keep those ids: otherwise a block's id names another block in linked
//! references, queries and a `((` pick. Two ways in: the owner adopts the
//! bytes of a held page (H), or an unheld page is reconciled and then opened
//! again (R). Driven through the real page host and Store, with fully keyed
//! DTOs as the window sends them.
use super::*;
use crate::model::Graph;
use tine_core::model::BlockDto;

const AUTHORED: &str = "6566c5a4-0000-4000-8000-000000000003";

fn label(block: &BlockDto) -> &str {
    block.raw.split_whitespace().next().unwrap_or("")
}

fn find<'a>(blocks: &'a mut [BlockDto], name: &str) -> &'a mut BlockDto {
    let at = blocks.iter().position(|b| label(b) == name).unwrap();
    &mut blocks[at]
}

type Edit = fn(&mut PageDto);

/// Structural edits the window saves with every block keyed: each leaves the
/// page's ids off their positions.
fn edits() -> [(&'static str, Edit); 3] {
    [
        ("move beta above alpha", |dto| {
            let beta = dto.blocks.remove(1);
            dto.blocks.insert(0, beta);
        }),
        ("move gamma to the end", |dto| {
            let gamma = find(&mut dto.blocks, "beta").children.remove(0);
            dto.blocks.push(gamma);
        }),
        ("a new block first", |dto| {
            let mut omega = dto.blocks[1].clone();
            // A window-minted id, as `src/document` gives a new block.
            (omega.id, omega.raw, omega.children) =
                ("b1763900000000-1".into(), "omega".into(), Vec::new());
            dto.blocks.insert(0, omega);
        }),
    ]
}

/// The page in each format, LF: `alpha` has an authored id.
fn pages() -> [(&'static str, String); 2] {
    [
        (
            "pages/Note.md",
            format!("- alpha\n  id:: {AUTHORED}\n- beta\n  - gamma\n- zeta\n"),
        ),
        (
            "pages/Note.org",
            format!("* alpha\n:PROPERTIES:\n:id: {AUTHORED}\n:END:\n* beta\n** gamma\n* zeta\n"),
        ),
    ]
}

/// `text` in each line terminator, with and without a final one: bytes
/// that hold the same outline, as an editor or git rewrites them.
fn equivalents(text: &str) -> Vec<(String, String)> {
    let lf = text.replace("\r\n", "\n").replace('\r', "\n");
    let body = lf.strip_suffix('\n').unwrap_or(&lf);
    let mut out = Vec::new();
    for (name, end) in [("LF", "\n"), ("CRLF", "\r\n"), ("lone CR", "\r")] {
        let lines = body.replace('\n', end);
        out.push((format!("{name}, final terminator"), format!("{lines}{end}")));
        out.push((format!("{name}, no final terminator"), lines));
    }
    out.retain(|(_, bytes)| bytes != text);
    out
}

/// Replace `rel` with `body` as an editor's atomic save does (staged outside
/// the graph, then renamed over the page), with a timestamp other than the
/// file's. A truncating write would let the watcher read the page empty
/// in between: another outline, whose observation rightly gives the page
/// positional ids before these bytes arrive.
fn rewrite(live: &Live, rel: &str, body: &str) {
    let path = live.root.join(rel);
    let staged = live.root.parent().unwrap().join("same-outline-staged");
    let modified = || fs::metadata(&path).unwrap().modified().unwrap();
    let before = modified();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        fs::write(&staged, body).unwrap();
        fs::rename(&staged, &path).unwrap();
        if modified() != before {
            return;
        }
        assert!(Instant::now() < deadline, "{rel} kept its timestamp");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// [`rewrite`] a held page and wait for its owner's observation of `body`.
fn adopt(live: &Live, rel: &str, body: &str) {
    rewrite(live, rel, body);
    let token = token(body);
    live.wait(rel, "observation", |mail| {
        (mail.page.as_ref()).is_some_and(|page| page.disk == Some(token.clone()))
    });
}

/// Every block's (text, runtime id), in page order.
fn pairs(blocks: &[BlockDto]) -> Vec<(String, String)> {
    fn walk(blocks: &[BlockDto], out: &mut Vec<(String, String)>) {
        for block in blocks {
            out.push((block.raw.clone(), block.id.clone()));
            walk(&block.children, out);
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

/// The index row of `rel` once it holds `text` (what linked references,
/// queries and search read), as (text, runtime id) pairs.
fn indexed_row(live: &Live, rel: &str, text: &str, case: &str) -> Vec<(String, String)> {
    let path = live.root.join(rel);
    let rev = Some(content_rev(text));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if live.store.graph.cached_rev(&path) == rev {
            let pages = live.store.graph.peek_pages().expect("a built page cache");
            let (_, doc) = (pages.iter())
                .find(|(entry, _)| entry.path == path)
                .unwrap_or_else(|| panic!("{case}: no row"));
            let blocks: Vec<BlockDto> = (doc.roots.iter())
                .map(tine_core::projection::block_to_dto)
                .collect();
            return pairs(&blocks);
        }
        assert!(Instant::now() < deadline, "{case}: never indexed {text:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Open `rel`, save `apply`'s structural edit as the window does, and return
/// the page key, the saved bytes and the index row's ids, which are the
/// window's.
fn structural_save(
    live: &Live,
    rel: &str,
    text: &str,
    apply: Edit,
    case: &str,
) -> (PageKey, String, Vec<(String, String)>) {
    let (key, page) = live.open(rel);
    let MailText::Page { dto } = &page.text else {
        panic!("{case}: an open answer carries the page");
    };
    let mut edited = dto.clone();
    apply(&mut edited);
    let (id, session) = (live.id(), live.host.session());
    let kinds = [EditKind::MoveBlocks];
    (live
        .host
        .submit(session, id, &key, &edited, page.version, None, &kinds))
    .unwrap();
    assert!(live.answer(&key, id).answer.unwrap().took, "{case}");
    let deadline = Instant::now() + Duration::from_secs(20);
    let saved = loop {
        let now = live.disk(rel);
        if now != text {
            break now;
        }
        assert!(Instant::now() < deadline, "{case}: never saved");
        std::thread::sleep(Duration::from_millis(20));
    };
    let row = indexed_row(live, rel, &saved, case);
    // The precondition: the own save's row carries the window's ids.
    let window = pairs(&edited.blocks);
    assert_eq!(row, window, "{case}: the saved row holds the window's ids");
    (key, saved, row)
}

/// Path H: the owner of an open page adopts same-outline bytes it did not
/// save; the index row keeps every block's id.
#[test]
fn an_owner_adoption_of_the_same_outline_keeps_runtime_ids() {
    for (rel, text) in pages() {
        for (edit, apply) in edits() {
            let case = format!("{rel} {edit}");
            let live = Live::new(&[(rel, &text)]);
            let (_, saved, kept) = structural_save(&live, rel, &text, apply, &case);
            for (variant, bytes) in equivalents(&saved) {
                let case = format!("{case}, then {variant}");
                adopt(&live, rel, &bytes);
                let now = indexed_row(&live, rel, &bytes, &case);
                assert_eq!(now, kept, "{case}: every block keeps its id");
            }
            live.host.stop();
        }
    }
}

/// Path R: the page is closed after the structural save, rewritten with the
/// same outline while closed, and opened again; the index row keeps every
/// block's id.
#[test]
fn a_reopened_page_rewritten_with_the_same_outline_keeps_runtime_ids() {
    for (rel, text) in pages() {
        for (edit, apply) in edits() {
            let case = format!("{rel} {edit}");
            let live = Live::new(&[(rel, &text)]);
            let (key, saved, kept) = structural_save(&live, rel, &text, apply, &case);
            // CRLF, as git's autocrlf checks a page out on Windows.
            let (variant, bytes) = (equivalents(&saved).into_iter())
                .find(|(variant, _)| variant == "CRLF, final terminator")
                .unwrap();
            let case = format!("{case}, closed, then {variant}, reopened");
            live.close(&key);
            let deadline = Instant::now() + Duration::from_secs(20);
            while live.held(rel) {
                assert!(Instant::now() < deadline, "{case}: never released");
                std::thread::sleep(Duration::from_millis(20));
            }
            rewrite(&live, rel, &bytes);
            // A read of the unheld page reconciles it with the file, as the
            // watcher does.
            live.store.page(&PageId::from(rel)).unwrap();
            let _ = live.open(rel);
            let deadline = Instant::now() + Duration::from_secs(20);
            while !live.held(rel) {
                assert!(Instant::now() < deadline, "{case}: never held");
                std::thread::sleep(Duration::from_millis(20));
            }
            let now = indexed_row(&live, rel, &bytes, &case);
            assert_eq!(now, kept, "{case}: every block keeps its id");
            live.host.stop();
        }
    }
}

/// Close `rel` and wait until the watcher no longer holds it.
fn close_and_release(live: &Live, key: &PageKey, rel: &str, case: &str) {
    live.close(key);
    let deadline = Instant::now() + Duration::from_secs(20);
    while live.held(rel) {
        assert!(Instant::now() < deadline, "{case}: never released");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Path R before the reconcile: the page is closed, rewritten with the same
/// outline, and opened again before any reconcile saw the rewrite (the
/// hold's claim then misses the file's revision); the index row keeps every
/// block's id, in every variant H covers.
#[test]
fn a_page_reopened_before_the_reconcile_saw_the_same_outline_keeps_runtime_ids() {
    for (rel, text) in pages() {
        for (edit, apply) in edits() {
            let case = format!("{rel} {edit}");
            // Polling every three seconds: the reopen below comes first.
            let live = Live::watching(&[(rel, &text)], crate::WatchMode::Poll);
            let (mut key, saved, kept) = structural_save(&live, rel, &text, apply, &case);
            let path = live.root.join(rel);
            for (variant, bytes) in equivalents(&saved) {
                let case = format!("{case}, closed, then {variant}, reopened");
                close_and_release(&live, &key, rel, &case);
                let before = live.store.graph.cached_rev(&path);
                rewrite(&live, rel, &bytes);
                // The precondition: no reconcile has seen these bytes.
                let rev = Some(content_rev(&bytes));
                assert_eq!(live.store.graph.cached_rev(&path), before, "{case}");
                assert_ne!(before, rev, "{case}");
                key = live.open(rel).0;
                let now = indexed_row(&live, rel, &bytes, &case);
                assert_eq!(now, kept, "{case}: every block keeps its id");
            }
            live.host.stop();
        }
    }
}

/// The reconcile of an unheld page rewritten with the same outline costs
/// O(1) in its blocks: the row keeps its document (no clone, no projection
/// or id work, no install) and the generation, and only its byte witness
/// (revision and anchor flag) moves to the new bytes, the anchor flag both
/// set and cleared.
#[test]
fn a_same_outline_reconcile_keeps_the_row_in_place() {
    const BLOCKS: usize = 60;
    let rel = "pages/Long.md";
    let mut text: String = (1..BLOCKS).map(|i| format!("- block {i}\n")).collect();
    text.push_str("- note\n  <<<<<<< not an anchor while indented\n");
    let anchored = text.replace("\n  <<<<<<< ", "\n<<<<<<< ");
    let crlf = text.replace('\n', "\r\n");
    // Polling every three seconds: the page read below reconciles first.
    let live = Live::watching(&[(rel, &text)], crate::WatchMode::Poll);
    let (graph, path, id) = (&live.store.graph, live.root.join(rel), PageId::from(rel));
    let row = || {
        let pages = graph.peek_pages().expect("a built page cache");
        let (_, doc) = pages.iter().find(|(entry, _)| entry.path == path).unwrap();
        Arc::clone(doc)
    };
    let row_before = row();
    assert_eq!(block_count(&row_before.roots), BLOCKS);
    assert_eq!(graph.vcs_anchor_state(&path), Some(false));
    for (variant, bytes, anchor) in [
        ("CRLF", &crlf, false),
        ("a column-0 anchor line", &anchored, true),
        ("the indented line again", &text, false),
    ] {
        let (generation, upserted) = (graph.cache_generation(), Graph::upserted_blocks());
        rewrite(&live, rel, bytes);
        live.store.page(&id).unwrap();
        let rev = Some(content_rev(bytes));
        assert_eq!(graph.cached_rev(&path), rev, "{variant}");
        assert_eq!(graph.vcs_anchor_state(&path), Some(anchor), "{variant}");
        // (the same document, blocks installed, generation moves)
        let cost = (
            Arc::ptr_eq(&row(), &row_before),
            Graph::upserted_blocks() - upserted,
            graph.cache_generation() - generation,
        );
        assert_eq!(cost, (true, 0, 0), "{variant}: the row stays in place");
    }
    live.host.stop();
}

fn block_count(blocks: &[tine_core::doc::DocBlock]) -> usize {
    (blocks.iter())
        .map(|block| 1 + block_count(&block.children))
        .sum()
}
