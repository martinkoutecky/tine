//! Phase A of `RUNTIME-ID-MODEL.md` v3 (store side), acceptance cases of
//! `RUNTIME-ID-FINDINGS.md` that fail on 16cee4693: each page file's keys
//! come from one owner, against the path's prior S(P). Graph-level, with no
//! host or watcher: a hold and its owner's publications are the calls the
//! page host's bridge makes. The two calls whose signature phase A changed
//! go through `page_ids_compat.rs`.

use super::page_ids_compat as compat;
use super::*;

fn graph(files: &[(&str, &str)]) -> (tempfile::TempDir, Graph) {
    let temp = tempfile::tempdir().unwrap();
    for area in ["pages", "journals", "logseq"] {
        fs::create_dir_all(temp.path().join(area)).unwrap();
    }
    for (rel, text) in files {
        fs::write(temp.path().join(rel), text).unwrap();
    }
    let graph = Graph::open(temp.path());
    (temp, graph)
}

/// Every block's (first line, key), in page order.
fn pairs(blocks: &[DocBlock]) -> Vec<(String, String)> {
    fn walk(blocks: &[DocBlock], out: &mut Vec<(String, String)>) {
        for block in blocks {
            let line = block.raw().lines().next().unwrap_or("").to_owned();
            out.push((line, block.uuid.to_string()));
            walk(&block.children, out);
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

/// The installed row of `rel`, as (first line, key) pairs.
fn row(graph: &Graph, rel: &str) -> Vec<(String, String)> {
    let path = graph.root.join(rel);
    (graph.with_cached(&path, |row| row.map(|(_, doc)| pairs(&doc.roots))))
        .flatten()
        .unwrap_or_else(|| panic!("{rel} has a row"))
}

fn keys(pairs: &[(String, String)]) -> Vec<String> {
    pairs.iter().map(|(_, key)| key.clone()).collect()
}

/// The serializer's keyed parse of `text` as `rel`: `keys` in page order.
fn keyed(graph: &Graph, rel: &str, text: &str, keys: &[String]) -> Document {
    fn set(blocks: &mut [DocBlock], keys: &mut std::slice::Iter<'_, String>) {
        for block in blocks {
            block.uuid = keys.next().expect("a key per block").as_str().into();
            set(&mut block.children, keys);
        }
    }
    let mut doc = parse_doc(&graph.root.join(rel), text);
    set(&mut doc.roots, &mut keys.iter());
    doc
}

/// What the page answers to a read or the host's mail of `text`.
fn served(graph: &Graph, rel: &str, text: &str) -> Vec<(String, String)> {
    let dto = (graph.page_dto_for_bytes(&graph.root.join(rel), text.as_bytes()))
        .unwrap()
        .unwrap();
    fn walk(blocks: &[tine_core::model::BlockDto], out: &mut Vec<(String, String)>) {
        for block in blocks {
            let line = block.raw.lines().next().unwrap_or("").to_owned();
            out.push((line, block.id.clone()));
            walk(&block.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&dto.blocks, &mut out);
    out
}

/// A page, its structural save (a block prepended, as the window sends it:
/// every block keyed, the new one with its own key), in Markdown and Org.
fn cases() -> [(&'static str, &'static str, &'static str); 2] {
    [
        ("pages/p.md", "- a\n- b\n  - c\n", "- n\n- a\n- b\n  - c\n"),
        ("pages/p.org", "* a\n* b\n** c\n", "* n\n* a\n* b\n** c\n"),
    ]
}

/// Bytes a sync client writes for `text`: CRLF, lone CR, no final break.
fn rewrites(text: &str) -> Vec<String> {
    let body = text.strip_suffix('\n').unwrap();
    vec![
        text.replace('\n', "\r\n"),
        text.replace('\n', "\r"),
        body.to_owned(),
    ]
}

/// Hold `rel`, publish its current bytes, then its structural save to
/// `saved` (written to disk too, as the host writes it). The saved keys.
fn structural_save(graph: &Graph, rel: &str, text: &str, saved: &str) -> Vec<String> {
    graph.with_pages(|_| ());
    graph.hold_unhosted(rel);
    compat::publish(graph, rel, text, None);
    let mut keys = vec!["0d9b1f2e-0000-4000-8000-00000000000a".to_owned()];
    keys.extend(self::keys(&row(graph, rel)));
    fs::write(graph.root.join(rel), saved).unwrap();
    compat::publish(graph, rel, saved, Some(keyed(graph, rel, saved, &keys)));
    assert_eq!(self::keys(&row(graph, rel)), keys, "{rel}: the save's keys");
    keys
}

/// A1 (desktop held): two identical bullets swapped and saved. The bytes
/// do not change, the keys do: the row takes the submitted keys.
#[test]
fn a1_a_same_bytes_key_swap_installs_the_swapped_keys() {
    for (rel, text) in [("pages/a.md", "- x\n- x\n"), ("pages/a.org", "* x\n* x\n")] {
        let (_temp, graph) = graph(&[(rel, text)]);
        graph.with_pages(|_| ());
        graph.hold_unhosted(rel);
        compat::publish(&graph, rel, text, None);
        let before = keys(&row(&graph, rel));
        let swapped = vec![before[1].clone(), before[0].clone()];
        let doc = keyed(&graph, rel, text, &swapped);
        compat::publish(&graph, rel, text, Some(doc));
        assert_eq!(
            keys(&row(&graph, rel)),
            swapped,
            "{rel}: the swapped keys are the page's"
        );
    }
}

/// F7 and F10: after a structural save, a sync client rewrites the bytes
/// with the same outline while the page is held. The row keeps the saved
/// keys, and the host's mail and a held read of those bytes (both
/// `page_dto_for_bytes`) carry the row's keys, not positional ones.
#[test]
fn f7_f10_mail_and_held_reads_carry_the_rows_keys_after_a_prepend_and_a_rewrite() {
    for (rel, text, saved) in cases() {
        for bytes in rewrites(saved) {
            let case = format!("{rel} {bytes:?}");
            let (_temp, graph) = graph(&[(rel, text)]);
            let keys = structural_save(&graph, rel, text, saved);
            fs::write(graph.root.join(rel), &bytes).unwrap();
            compat::publish(&graph, rel, &bytes, None);
            let row = row(&graph, rel);
            assert_eq!(self::keys(&row), keys, "{case}: the row keeps its keys");
            assert_eq!(served(&graph, rel, &bytes), row, "{case}: read = row");
        }
    }
}

/// F9: the page closes after the save; the bytes are rewritten with the
/// same outline and the page is held again before the watcher reconciles
/// them. The hold's publication keys them against the retired row.
#[test]
fn f9_a_hold_before_the_reconcile_keeps_the_saved_keys() {
    for (rel, text, saved) in cases() {
        for bytes in rewrites(saved) {
            let case = format!("{rel} {bytes:?}");
            let (_temp, graph) = graph(&[(rel, text)]);
            let keys = structural_save(&graph, rel, text, saved);
            let _ = graph.release(rel);
            fs::write(graph.root.join(rel), &bytes).unwrap();
            graph.hold_unhosted(rel);
            compat::publish(&graph, rel, &bytes, None);
            assert_eq!(self::keys(&row(&graph, rel)), keys, "{case}");
        }
    }
}

/// A4 and M1: a config change that moves what the page build reads
/// rebuilds every row (`invalidate_cache`). A structurally saved page keeps
/// its keys, held or not.
#[test]
fn a4_a_config_rebuild_keeps_a_structurally_saved_pages_keys() {
    for (rel, text, saved) in cases() {
        for held in [false, true] {
            let case = format!("{rel} held={held}");
            let (_temp, graph) = graph(&[(rel, text), ("pages/other.md", "- o\n")]);
            let keys = structural_save(&graph, rel, text, saved);
            if !held {
                let _ = graph.release(rel);
                let _ = graph.sync_file_internal(&graph.root.join(rel), None);
            }
            let mut config = (*graph.current_config()).clone();
            config.journal_file_name_format = Some("yyyy-MM-dd".into());
            graph.reload_config(config).unwrap();
            graph.with_pages(|_| ());
            assert_eq!(self::keys(&row(&graph, rel)), keys, "{case}");
        }
    }
}

/// T10's complete prior (A4, M1): between an invalidation and the
/// rebuild's install, a read of same-outline bytes and a hold's publication
/// of them key against the row the invalidation retired, so the seed they
/// leave, which the rebuild installs whole, carries the saved keys.
#[test]
fn t10_a_read_and_a_publication_before_the_rebuild_keep_the_saved_keys() {
    for (rel, text, saved) in cases() {
        let bytes = saved.replace('\n', "\r\n");
        let (_temp, graph) = graph(&[(rel, text)]);
        let keys = structural_save(&graph, rel, text, saved);
        let _ = graph.release(rel);
        let _ = graph.sync_file_internal(&graph.root.join(rel), None);
        graph.invalidate_cache();
        fs::write(graph.root.join(rel), &bytes).unwrap();
        let read = self::keys(&served(&graph, rel, &bytes));
        assert_eq!(read, keys, "{rel}: a read before the rebuild");
        graph.hold_unhosted(rel);
        compat::publish(&graph, rel, &bytes, None);
        graph.with_pages(|_| ());
        assert_eq!(
            self::keys(&row(&graph, rel)),
            keys,
            "{rel}: the rebuilt row"
        );
    }
}

/// Finding 6 of the ee4ea5b9a review (DS-4): the save base's keys decide
/// only which old block a saved block's layout comes from, so a save writes
/// the bytes develop (16cee4693) writes. The deleted lineage test's cases at
/// the save boundary, in Markdown and Org with mixed line terminators: a
/// fresh row (unchanged, reordered, reordered and edited), and a row whose
/// keys a save moved, saved again unchanged and reordered. The expected
/// strings are develop's output for these saves.
#[test]
fn a_save_writes_the_bytes_develop_writes() {
    let mut written = Vec::new();
    for (rel, source, moved) in [
        ("pages/p.md", "- p\r\n- p\n- r\r\n", "- r\r\n- p\r\n- p\n"),
        ("pages/p.org", "* p\r\n* p\n* r\r\n", "* r\r\n* p\r\n* p\n"),
    ] {
        let (_temp, graph) = graph(&[(rel, source)]);
        graph.with_pages(|_| ());
        let path = graph.root.join(rel);
        let save = |dto: &PageDto, existing: &str| {
            let (bytes, _) = graph
                .prepare_page_bytes(dto, &path, Some(existing))
                .unwrap();
            String::from_utf8(bytes).unwrap()
        };
        let dto = graph
            .load_page(&graph.entry_for_path(&path).unwrap())
            .unwrap();
        let mut swapped = dto.clone();
        swapped.blocks.swap(0, 2);
        let mut edited = swapped.clone();
        edited.blocks[1].raw.push_str(" edited");
        for dto in [&dto, &swapped, &edited] {
            written.push(save(dto, source));
        }
        // The window moves `r` first and saves; the row keeps its keys.
        let mut window = dto.clone();
        window.blocks.rotate_right(1);
        assert_eq!(save(&window, source), moved, "{rel}: the moving save");
        let keys: Vec<String> = window.blocks.iter().map(|b| b.id.clone()).collect();
        graph.hold_unhosted(rel);
        compat::publish(&graph, rel, source, None);
        fs::write(&path, moved).unwrap();
        compat::publish(&graph, rel, moved, Some(keyed(&graph, rel, moved, &keys)));
        let mut again = window.clone();
        again.blocks.swap(1, 2);
        for dto in [&window, &again] {
            written.push(save(dto, moved));
        }
    }
    eprintln!("SAVE-BYTES {written:?}");
    let expected: [&str; 10] = [
        "- p\r\n- p\n- r\r\n",
        "- r\r\n- p\n- p\r\n",
        "- r\r\n- p edited\n- p\r\n",
        "- r\r\n- p\r\n- p\n",
        "- r\r\n- p\r\n- p\n",
        "* p\r\n* p\n* r\r\n",
        "* r\r\n* p\n* p\r\n",
        "* r\r\n* p edited\n* p\r\n",
        "* r\r\n* p\r\n* p\n",
        "* r\r\n* p\r\n* p\n",
    ];
    assert_eq!(written, expected, "develop's bytes");
}

/// A7: the page is held and saved before the cache exists; the build that
/// follows installs the save's keys (its seed), not positional ones.
#[test]
fn a7_a_save_before_the_cache_exists_survives_the_build() {
    for (rel, text, saved) in cases() {
        let (_temp, graph) = graph(&[(rel, text)]);
        assert!(!graph.cache_built());
        graph.hold_unhosted(rel);
        let keys: Vec<String> = (0..4)
            .map(|at| format!("6b1e0c3a-0000-4000-8000-00000000000{at}"))
            .collect();
        fs::write(graph.root.join(rel), saved).unwrap();
        compat::publish(&graph, rel, saved, Some(keyed(&graph, rel, saved, &keys)));
        graph.with_pages(|_| ());
        assert_eq!(self::keys(&row(&graph, rel)), keys, "{rel}");
    }
}

/// A8 and M2 (retirement by path): a hold retires the page's row into its
/// claim; a close before the hold ever published (`release`), or the host
/// stopping (`release_all`), leaves the claim to the reread, whose
/// reconcile of the same outline keeps the keys.
#[test]
fn a8_a_release_before_the_first_publication_keeps_the_keys() {
    for (rel, text, saved) in cases() {
        for all in [false, true] {
            let case = format!("{rel} release_all={all}");
            let (_temp, graph) = graph(&[(rel, text)]);
            let keys = structural_save(&graph, rel, text, saved);
            let _ = graph.release(rel);
            let path = graph.root.join(rel);
            let _ = graph.sync_file_internal(&path, None);
            // Opened again, and closed before its first publication.
            graph.hold_unhosted(rel);
            if all {
                let _ = graph.release_all();
            } else {
                let _ = graph.release(rel);
            }
            let bytes = saved.replace('\n', "\r\n");
            fs::write(&path, &bytes).unwrap();
            let _ = graph.sync_file_internal(&path, None);
            assert_eq!(self::keys(&row(&graph, rel)), keys, "{case}");
        }
    }
}

/// A18: a case-only spelling move of an open, structurally saved page
/// (`pages/Foo.md` to `pages/foo.md`, the same file): the keys go with it,
/// and nothing stays at the old spelling.
#[test]
fn a18_a_case_only_spelling_move_keeps_the_keys() {
    let (old, new) = ("pages/Foo.md", "pages/foo.md");
    let (text, saved) = ("- a\n- b\n", "- n\n- a\n- b\n");
    let (_temp, graph) = graph(&[(old, text)]);
    let keys = structural_save(&graph, old, text, saved);
    let from = graph.root.join(old);
    fs::rename(&from, graph.root.join(new)).unwrap();
    graph.held.spellings().spell(old, new);
    compat::respelled(&graph, old, &from);
    compat::publish(&graph, old, saved, None);
    assert_eq!(
        self::keys(&row(&graph, new)),
        keys,
        "keys moved with the file"
    );
    assert_eq!(graph.cached_rev(&from), None, "nothing at the old spelling");
}

/// Blocker 1 of the ee4ea5b9a review: A's first block moves to B (both
/// open, the transfer registers the key to B). A Rescan keys every page
/// against its current row: with B open and A closed, A does not take the
/// moved key back positionally; with B closed, B keeps the keys it served.
#[test]
fn r2_a_rescan_after_a_move_keeps_each_key_on_one_page() {
    for (a, b, text_a, text_b, moved_a, moved_b) in [
        (
            "pages/a.md",
            "pages/b.md",
            "- a1\n- a2\n",
            "- b1\n",
            "- a2\n",
            "- a1\n- b1\n",
        ),
        (
            "pages/a.org",
            "pages/b.org",
            "* a1\n* a2\n",
            "* b1\n",
            "* a2\n",
            "* a1\n* b1\n",
        ),
    ] {
        for b_open in [true, false] {
            let case = format!("{a} B open={b_open}");
            let (_temp, graph) = graph(&[(a, text_a), (b, text_b)]);
            graph.with_pages(|_| ());
            let (ka, kb) = (keys(&row(&graph, a)), keys(&row(&graph, b)));
            for (rel, text) in [(a, text_a), (b, text_b)] {
                graph.hold_unhosted(rel);
                compat::publish(&graph, rel, text, None);
            }
            let served_b = vec![ka[0].clone(), kb[0].clone()];
            fs::write(graph.root.join(a), moved_a).unwrap();
            compat::publish(
                &graph,
                a,
                moved_a,
                Some(keyed(&graph, a, moved_a, &ka[1..])),
            );
            fs::write(graph.root.join(b), moved_b).unwrap();
            compat::publish(
                &graph,
                b,
                moved_b,
                Some(keyed(&graph, b, moved_b, &served_b)),
            );
            (graph.ids).moved([ka[0].as_str().into()], &graph.root.join(b));
            let closed = if b_open { a } else { b };
            let _ = graph.release(closed);
            let _ = graph.sync_file_internal(&graph.root.join(closed), None);
            assert!(
                graph.rebuild_cache_cancellable(|| false),
                "{case}: installs"
            );
            if b_open {
                let now = keys(&row(&graph, a));
                assert_eq!(now, ka[1..], "{case}: A keeps its key, not B's");
            } else {
                assert_eq!(keys(&row(&graph, b)), served_b, "{case}: B's served keys");
            }
        }
    }
}

/// Blocker 1: a save before the cache exists leaves a seed; the page closes
/// before the launch build installs (before the build starts, and while it
/// parses, which the launch patch re-derives). The installed row is the
/// save's keys.
#[test]
fn r2_a_save_seed_released_before_the_launch_install_keeps_its_keys() {
    for (rel, text, saved) in cases() {
        for mid_warm in [false, true] {
            let case = format!("{rel} mid_warm={mid_warm}");
            let (_temp, graph) = graph(&[(rel, text), ("pages/other.md", "- o\n")]);
            let keys: Vec<String> = (0..4)
                .map(|at| format!("6b1e0c3a-0000-4000-8000-00000000002{at}"))
                .collect();
            let save = || {
                graph.hold_unhosted(rel);
                fs::write(graph.root.join(rel), saved).unwrap();
                compat::publish(&graph, rel, saved, Some(keyed(&graph, rel, saved, &keys)));
                let _ = graph.release(rel);
            };
            if mid_warm {
                let pause: crate::store::TestPause = Default::default();
                *graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&pause));
                std::thread::scope(|scope| {
                    let warm = scope.spawn(|| graph.warm_page_cache_cancellable(&|| false));
                    let (state, ready) = &*pause;
                    let mut state = state.lock().unwrap();
                    while !state.0 {
                        state = ready.wait(state).unwrap();
                    }
                    drop(state);
                    save();
                    pause.0.lock().unwrap().1 = true;
                    pause.1.notify_all();
                    let _ = warm.join().unwrap();
                });
                *graph.warm_after_first_page_pause.lock().unwrap() = None;
            } else {
                save();
                assert!(graph.warm_page_cache_cancellable(&|| false), "{case}");
            }
            graph.with_pages(|_| ());
            assert_eq!(self::keys(&row(&graph, rel)), keys, "{case}");
        }
    }
}

/// Finding 3: after an invalidation, a read of another outline leaves a
/// seed; the file then reverts to the saved bytes, the retired row's
/// revision. The read and the rebuilt row take the retired row's keys.
#[test]
fn r2_a_revert_to_the_retired_revision_keeps_the_saved_keys() {
    for (rel, text, saved) in cases() {
        let (_temp, graph) = graph(&[(rel, text)]);
        let keys = structural_save(&graph, rel, text, saved);
        let path = graph.root.join(rel);
        let _ = graph.release(rel);
        let _ = graph.sync_file_internal(&path, None);
        graph.invalidate_cache();
        fs::write(&path, text).unwrap();
        let _ = graph.load_page(&graph.entry_for_path(&path).unwrap());
        fs::write(&path, saved).unwrap();
        assert_eq!(self::keys(&served(&graph, rel, saved)), keys, "{rel}: read");
        graph.with_pages(|_| ());
        assert_eq!(self::keys(&row(&graph, rel)), keys, "{rel}: the row");
    }
}

/// Finding 3: an owner publication whose parse fails does not consume the
/// path's claim; the next publication of the saved outline keeps its keys.
#[test]
fn r2_a_failed_owner_parse_keeps_the_claim() {
    for (rel, text, saved) in cases() {
        let (_temp, graph) = graph(&[(rel, text)]);
        let keys = structural_save(&graph, rel, text, saved);
        let path = graph.root.join(rel);
        let _ = graph.release(rel);
        let _ = graph.sync_file_internal(&path, None);
        graph.hold_unhosted(rel);
        let bad = format!("- {TEST_PAGE_PARSE_PANIC_SENTINEL}\n");
        let _ = graph.publish_owned(rel, Some(Arc::from(bad.as_bytes())), None);
        let bytes = saved.replace('\n', "\r\n");
        fs::write(&path, &bytes).unwrap();
        compat::publish(&graph, rel, &bytes, None);
        assert_eq!(self::keys(&row(&graph, rel)), keys, "{rel}");
    }
}

/// Finding 3: a hold of an unchanged row claims nothing, so its release
/// leaves no claim behind.
#[test]
fn r2_a_hold_of_an_unchanged_row_leaves_no_claim() {
    for (rel, text, _) in cases() {
        let (_temp, graph) = graph(&[(rel, text)]);
        graph.with_pages(|_| ());
        graph.held.spellings().spell(rel, rel);
        graph.hold_current(rel.into());
        let _ = graph.release(rel);
        assert!(graph.held.claims.lock().unwrap().is_empty(), "{rel}");
    }
}

/// Cold build and whole-cache rebuild time and peak RSS on a synthetic
/// graph of 4,000 pages of 25 blocks (numbers for the receipt, base and
/// phase A alike; run with `--ignored --nocapture`).
#[test]
#[ignore]
fn measure_cold_build_and_rebuild() {
    let text: String = (0..25)
        .map(|at| format!("- block {at} [[Shared]]\n"))
        .collect();
    let rels: Vec<String> = (0..4000).map(|at| format!("pages/p{at}.md")).collect();
    let files: Vec<(&str, &str)> = rels
        .iter()
        .map(|rel| (rel.as_str(), text.as_str()))
        .collect();
    let (_temp, graph) = graph(&files);
    let started = std::time::Instant::now();
    graph.with_pages(|_| ());
    let cold = started.elapsed();
    let mut rebuilds = Vec::new();
    for _ in 0..3 {
        let started = std::time::Instant::now();
        graph.invalidate_cache();
        graph.with_pages(|_| ());
        rebuilds.push(started.elapsed());
    }
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let hwm = status
        .lines()
        .find(|l| l.starts_with("VmHWM"))
        .unwrap_or("");
    eprintln!("RUNTIME-ID-COST cold={cold:?} rebuilds={rebuilds:?} {hwm}");
}
