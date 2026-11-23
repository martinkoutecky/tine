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
                live.external(rel, &bytes);
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
            live.rewrite_externally(rel, &bytes);
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
