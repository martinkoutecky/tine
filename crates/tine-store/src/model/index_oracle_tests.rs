//! GJ5 equality oracle: a canonical text image of what the initial build
//! derives from a graph (every block projection and the snapshot indexes the
//! first publication builds). Two builds of one graph copy are equal exactly
//! when their images are byte-equal. Page slots and per-page name sets are
//! keyed by path and sorted, because the parallel load assigns slots in
//! completion order and `collect_document_referenced_names` returns its names
//! in hash order; neither order carries meaning.
use super::*;

pub(crate) use super::index_oracle::snapshot_image;

#[test]
fn the_block_oracle_includes_ids_absent_from_every_page() {
    let dir = fixture();
    let graph = Graph::open(dir.path());
    graph.with_pages(|_| ());
    let mut snapshot = ReadSnapshot::capture(
        &graph,
        graph.config.clone(),
        Arc::new(EntryList::from(graph.list_pages_shared().as_slice())),
        None,
        &[],
    );
    let slot = snapshot.pages.slots().next().unwrap().0;
    let mut index = snapshot.block_index.get().unwrap().clone();
    index.edit("stale-key-held-by-no-page", |owners| owners.push(slot));
    let _ = snapshot.block_index.take();
    snapshot.block_index.set(index).ok().unwrap();
    assert!(snapshot_image(&snapshot, &[]).contains("block owners \"stale-key-held-by-no-page\""));
}

#[test]
#[should_panic(expected = "block index missed held id")]
fn an_index_miss_checks_every_page_in_tests() {
    let dir = fixture();
    let graph = Graph::open(dir.path());
    graph.with_pages(|_| ());
    let mut snapshot = ReadSnapshot::capture(
        &graph,
        graph.config.clone(),
        Arc::new(EntryList::from(graph.list_pages_shared().as_slice())),
        None,
        &[],
    );
    let id = snapshot
        .pages
        .iter()
        .find_map(|(_, doc)| doc.roots.first())
        .unwrap()
        .uuid
        .clone();
    let mut index = snapshot.block_index.get().unwrap().clone();
    index.edit(&id, Vec::clear);
    snapshot.block_index.take();
    snapshot.block_index.set(index).ok().unwrap();
    snapshot.block_owner_page(&id);
}

/// The canonical image of `graph`'s initial build.
pub(super) fn initial_build_image(graph: &Graph) -> String {
    graph.with_pages(|_| ());
    let snapshot = ReadSnapshot::capture(
        graph,
        graph.config.clone(),
        Arc::new(EntryList::from(graph.list_pages_shared().as_slice())),
        None,
        &[],
    );
    snapshot_image(&snapshot, &[])
}

/// `TINE_INDEX_ORACLE_GRAPH=<graph copy> TINE_INDEX_ORACLE_OUT=<file>
/// cargo test -p tine-store index_oracle_image -- --ignored` writes the image;
/// run it on the base and on the candidate and compare the files with `cmp`.
#[test]
#[ignore = "oracle: needs TINE_INDEX_ORACLE_GRAPH and TINE_INDEX_ORACLE_OUT"]
fn index_oracle_image() {
    let root = std::env::var("TINE_INDEX_ORACLE_GRAPH").expect("TINE_INDEX_ORACLE_GRAPH");
    let out = std::env::var("TINE_INDEX_ORACLE_OUT").expect("TINE_INDEX_ORACLE_OUT");
    let graph = Graph::open(&root);
    fs::write(out, initial_build_image(&graph)).unwrap();
}

/// More pages than the parallel threshold, with every explicit-name source
/// (links, tags, namespaces, page and block property keys and values, task
/// markers, aliases, Org) and Han names whose spellings share one page key.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for sub in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(
        root.join("pages/kitchen-sink.md"),
        include_str!("../../../../src/fixtures/kitchen-sink.md"),
    )
    .unwrap();
    fs::write(
        root.join("pages/中文页面.md"),
        "alias:: 汉字别名, Über 中文\n\n- 读书 [[中文页面]] [[学习/数学]] #标签 [[豈]] [[\u{F900}]]\n  分类:: [[中文页面]]\n",
    )
    .unwrap();
    fs::write(
        root.join("pages/notes.org"),
        "#+title: Notes\n* TODO [[Org Target]] :orgtag:\n** child [[ns/a/b]]\n",
    )
    .unwrap();
    for i in 0..80 {
        fs::write(
            root.join(format!("pages/P{i:02}.md")),
            format!(
                "tags:: hub{}, [[Shared]]\n\n- TODO [[Topic {}]] #tag{} [[ns/a/{}]]\n  status:: open\n  Custom_Key:: [[Topic {}]]\n  - DONE child ((nope)) [[Topic {}]] [[P{:02}]]\n- [[topic {}]] again\n",
                i % 3,
                i % 7,
                i % 5,
                i % 4,
                i % 9,
                i % 7,
                (i + 1) % 80,
                i % 7
            ),
        )
        .unwrap();
    }
    fs::write(
        root.join("journals/2026_10_18.md"),
        "- [[P01]] [[中文页面]] [[Topic 3]] [[\u{8C48}]]\n",
    )
    .unwrap();
    dir
}

/// GJ5 S1/S4: the parallel bulk build of the explicit-reference index (and
/// the initial capture that uses it) equals inserting each page's names in
/// turn, the former serial build.
#[test]
fn explicit_index_build_equals_inserting_each_page() {
    let dir = fixture();
    let graph = Graph::open(dir.path());
    graph.with_pages(|_| ());
    let pages = graph.peek_pages().unwrap();
    assert!(pages.len() >= 64 && page_cache_worker_count() > 1);
    let mut reference = SnapshotExplicitIndex {
        shards: (0..SnapshotExplicitIndex::SHARDS)
            .map(|_| Arc::new(SharedMap::new()))
            .collect(),
    };
    for (entry, doc) in pages.iter() {
        let names = crate::query::document_explicit_reference_names(entry, doc, &graph.config);
        reference.update(&entry.path, &[], &names);
    }
    let reference = serde_json::to_string(&reference).unwrap();
    assert!(reference.contains("topic 3") && reference.contains("ns/a"));
    let built = SnapshotExplicitIndex::build(&pages, &graph.config);
    assert_eq!(serde_json::to_string(&built).unwrap(), reference);
    let snapshot = ReadSnapshot::capture(
        &graph,
        graph.config.clone(),
        Arc::new(EntryList::from(graph.list_pages_shared().as_slice())),
        None,
        &[],
    );
    assert_eq!(
        serde_json::to_string(&snapshot.explicit_index).unwrap(),
        reference
    );
}

/// GJ5 S6: transliteration ownership filters referenced names before copying
/// them; it must equal collecting every referenced name, then filtering.
#[test]
fn name_keys_equal_collecting_then_filtering() {
    let dir = fixture();
    let graph = Graph::open(dir.path());
    let image = initial_build_image(&graph);
    let pages = graph.peek_pages().unwrap();
    let mut reference: std::collections::BTreeMap<String, usize> = Default::default();
    for (page, doc) in pages.iter() {
        let names: std::collections::BTreeSet<String> = std::iter::once(page.name.clone())
            .chain(crate::query::document_alias_spellings(doc))
            .chain(collect_document_referenced_names(doc))
            .filter_map(|name| crate::transliteration::cache_identity(&name))
            .collect();
        for name in names {
            *reference.entry(name).or_default() += 1;
        }
    }
    assert!(reference.contains_key("\u{8C48}") && reference.contains_key("über 中文"));
    let owners: String = reference
        .iter()
        .map(|(name, owners)| format!("name_owners {name:?} {owners}\n"))
        .collect();
    let built: String = image
        .lines()
        .filter(|line| line.starts_with("name_owners "))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(built, owners);
}

/// The first line where two images differ, for a readable mismatch.
pub(crate) fn first_difference(live: &str, other: &str) -> Option<String> {
    if live == other {
        return None;
    }
    let (a, b): (Vec<_>, Vec<_>) = (live.lines().collect(), other.lines().collect());
    let at = a
        .iter()
        .zip(&b)
        .position(|(a, b)| a != b)
        .unwrap_or(a.len().min(b.len()));
    let line = |lines: &[&str]| {
        lines
            .get(at)
            .map(|l| l.chars().take(400).collect::<String>())
    };
    Some(format!(
        "line {at}\n  live : {:?}\n  other: {:?}",
        line(&a),
        line(&b)
    ))
}

/// Image the store's current publication (built incrementally from the one
/// before, whose lazy indexes the previous image forced), then compare it with
/// a parent-free capture of the same parsed pages and with a second store
/// opened on the same files. Differences go to `mismatches`.
fn check(
    store: &crate::store::Store,
    root: &Path,
    label: &str,
    probes: &[&str],
    log: &mut String,
    mismatches: &mut Vec<String>,
) {
    check_lines(store, root, label, probes, log, mismatches, |_| true);
}

/// [`check`] comparing only the image lines `keep` admits (the log keeps all).
fn check_lines(
    store: &crate::store::Store,
    root: &Path,
    label: &str,
    probes: &[&str],
    log: &mut String,
    mismatches: &mut Vec<String>,
    keep: impl Fn(&str) -> bool,
) {
    let only = |image: &str| -> String {
        image
            .lines()
            .filter(|line| keep(line))
            .flat_map(|line| [line, "\n"])
            .collect()
    };
    let view = store.whole_graph().unwrap();
    let live_image = snapshot_image(&view.graph, probes);
    let live = only(&live_image);
    let fresh = ReadSnapshot::capture(
        &store.graph,
        (*view.config.config).clone(),
        Arc::clone(&view.list),
        None,
        &[],
    );
    let fresh = only(&snapshot_image(&fresh, probes));
    let reopened = {
        let other = crate::store::Store::open(root, Default::default())
            .unwrap()
            .0;
        let view = other.whole_graph().unwrap();
        let image = only(&snapshot_image(&view.graph, probes));
        drop(view);
        other.close();
        image
    };
    for (what, other) in [
        ("parent-free capture", &fresh),
        ("reopened store", &reopened),
    ] {
        if let Some(difference) = first_difference(&live, other) {
            mismatches.push(format!("{label}: incremental vs {what}: {difference}"));
        }
    }
    log.push_str(&format!("=== {label}\n{live_image}"));
}

fn rev(root: &Path, rel: &str) -> crate::FileRev {
    crate::FileRev::from_file(&root.join(rel)).unwrap()
}

fn commit(tx: crate::Transaction<'_>, what: &str) {
    match tx.commit() {
        crate::TxOutcome::Committed { .. } => {}
        _ => panic!("{what}: transaction not committed"),
    }
}

fn save(store: &crate::store::Store, rel: &str, edit: impl FnOnce(&mut tine_core::PageDto)) {
    let id = crate::PageId::from(rel.to_owned());
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    edit(&mut doc);
    match store.save(
        crate::EditKind::ReplacePage,
        &id,
        crate::SaveBase::Existing(read.rev),
        &doc,
    ) {
        crate::SaveOutcome::Saved(_) => {}
        other => panic!("save {rel}: {other:?}"),
    }
}

/// [`save`] for an edit that may change nothing.
fn save_any(store: &crate::store::Store, rel: &str, edit: impl FnOnce(&mut tine_core::PageDto)) {
    let id = crate::PageId::from(rel.to_owned());
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    edit(&mut doc);
    match store.save(
        crate::EditKind::ReplacePage,
        &id,
        crate::SaveBase::Existing(read.rev),
        &doc,
    ) {
        crate::SaveOutcome::Saved(_) | crate::SaveOutcome::Unchanged(_) => {}
        other => panic!("save {rel}: {other:?}"),
    }
}

/// Markdown files under `dir` (relative to `root`), recursively.
fn markdown_files(root: &Path, dir: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root.join(dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{dir}/{name}");
        if entry.path().is_dir() {
            markdown_files(root, &rel, out);
        } else if name.ends_with(".md") {
            out.push(rel);
        }
    }
}

/// The rename target (the plain-named Markdown page with the most explicit
/// referrers) and the page edited to reference it.
fn pick_pages(snapshot: &ReadSnapshot) -> (String, String) {
    let mut plain: Vec<(usize, String, String)> = snapshot
        .pages
        .iter()
        .filter_map(|(entry, doc)| {
            let rel = entry.rel_path.as_ref()?.as_str().to_owned();
            let simple = entry
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-');
            (simple && rel == format!("pages/{}.md", entry.name) && !doc.roots.is_empty()).then(
                || {
                    let key = tine_core::refs::page_key(&entry.name);
                    let referrers = snapshot.explicit_index.shards
                        [SnapshotExplicitIndex::shard(&key)]
                    .get(&key)
                    .map_or(0, |paths| paths.len());
                    (referrers, rel, entry.name.clone())
                },
            )
        })
        .collect();
    plain.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let target = plain.first().expect("a plain Markdown page").2.clone();
    let mut rest: Vec<_> = plain
        .iter()
        .skip(1)
        .map(|(_, rel, _)| rel.clone())
        .collect();
    rest.sort();
    (target, rest.first().expect("a second plain page").clone())
}

/// GJ5 review r1: every incremental publication after a create, an edit, a
/// rename (with referrer rewrites), the removal of a Han name's last owner by
/// edit and by deletion, and deletions, with all lazy indexes forced before
/// each mutation, equals a parent-free capture and a fresh store. Writes go
/// through the store's guarded save, move and trash. Returns the step images
/// (for the base comparison) and the mismatches.
fn incremental_run(root: &Path) -> (String, Vec<String>) {
    let store = crate::store::Store::open(root, Default::default())
        .unwrap()
        .0;
    let (target, editor) = pick_pages(&store.whole_graph().unwrap().graph);
    let renamed = format!("{target} GJ5");
    let probe_rel = "pages/GJ5 Probe.md";
    let probes = [
        "GJ5 Probe",
        "探针别名",
        "孤独测试",
        "GJ5 Other",
        target.as_str(),
        renamed.as_str(),
    ];
    let (mut log, mut mismatches) = (format!("target {target:?} editor {editor:?}\n"), vec![]);
    let mut step = |label: &str| check(&store, root, label, &probes, &mut log, &mut mismatches);
    step("open");

    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
    tx.create(
        &crate::PageId::from(probe_rel.to_owned()).file(),
        crate::Content::Bytes(
            format!(
                "alias:: 探针别名\n\n- probe [[孤独测试]] [[{target}]] #gj5tag\n  gj5key:: [[孤独测试]]\n- TODO second [[GJ5 Other]]\n"
            )
            .into_bytes(),
        ),
    );
    commit(tx, "create probe");
    step("create probe page");

    save(&store, &editor, |doc| {
        let raw = &mut doc.blocks[0].raw;
        let (first, rest) = raw.split_once('\n').unwrap_or((raw.as_str(), ""));
        let edited = format!("{first} [[GJ5 Probe]] [[{target}]] #gj5edited");
        *raw = if rest.is_empty() {
            edited
        } else {
            format!("{edited}\n{rest}")
        };
    });
    step("edit a block");

    let map = crate::RenameMap(vec![(target.clone(), renamed.clone())]);
    let from = format!("pages/{target}.md");
    let to = format!("pages/{renamed}.md");
    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
    tx.move_file(
        &crate::PageId::from(from.clone()).file(),
        rev(root, &from),
        &crate::PageId::from(to.clone()).file(),
        Some(&map),
    );
    let mut files = Vec::new();
    markdown_files(root, "pages", &mut files);
    markdown_files(root, "journals", &mut files);
    files.sort();
    let needle = target.to_lowercase();
    for rel in files.iter().filter(|rel| **rel != from) {
        let text = fs::read_to_string(root.join(rel)).unwrap();
        if text.to_lowercase().contains(&needle) {
            tx.rewrite_refs(
                &crate::PageId::from(rel.clone()),
                rev(root, rel),
                Some(&text),
                &map,
                crate::TitleRebind::Keep,
            );
        }
    }
    commit(tx, "rename");
    step("rename with referrer rewrites");

    save(&store, probe_rel, |doc| {
        doc.blocks.truncate(1);
        doc.blocks[0].raw = format!("probe without the Han reference [[{renamed}]]");
    });
    step("edit away the last owner of a Han name");

    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
    tx.trash(
        &crate::PageId::from(to.clone()).file(),
        rev(root, &to),
        crate::TrashIf::Any,
    );
    commit(tx, "delete renamed page");
    step("delete the renamed page");

    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
    tx.trash(
        &crate::PageId::from(probe_rel.to_owned()).file(),
        rev(root, probe_rel),
        crate::TrashIf::Any,
    );
    commit(tx, "delete probe");
    step("delete the last owner of a Han alias");
    drop(step);
    store.close();
    (log, mismatches)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn incremental_publications_equal_a_fresh_build() {
    let dir = fixture();
    let (log, mismatches) = incremental_run(dir.path());
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    // Not vacuous: the Han names gain their only owner, then lose it by an
    // edit and by a deletion; the rename moves the target's references.
    let step = |label: &str| {
        let start = log.find(&format!("=== {label}\n")).expect(label);
        let rest = &log[start + 1..];
        &rest[..rest.find("\n=== ").unwrap_or(rest.len())]
    };
    let owned =
        |label: &str, name: &str| step(label).contains(&format!("name_owners {name:?} 1\n"));
    assert!(!owned("open", "孤独测试") && owned("create probe page", "孤独测试"));
    assert!(owned("rename with referrer rewrites", "孤独测试"));
    assert!(!owned("edit away the last owner of a Han name", "孤独测试"));
    assert!(owned("delete the renamed page", "探针别名"));
    assert!(!owned("delete the last owner of a Han alias", "探针别名"));
    assert!(log.starts_with("target \"P01\""), "{}", &log[..80]);
    // The journal, the probe page and two P00 blocks now link `P01 GJ5`.
    assert!(step("rename with referrer rewrites").contains("lazy backlinks \"P01 GJ5\" 4 false"));
}

/// `TINE_INDEX_ORACLE_GRAPH=<graph> TINE_INDEX_ORACLE_OUT=<file> cargo test -p
/// tine-store index_oracle_incremental -- --ignored` runs the incremental
/// scenario on a temporary copy of the graph (never on the graph itself) and
/// writes the step images and any mismatch; compare base and candidate files.
#[test]
#[ignore = "oracle: needs TINE_INDEX_ORACLE_GRAPH and TINE_INDEX_ORACLE_OUT"]
fn index_oracle_incremental() {
    let source = std::env::var("TINE_INDEX_ORACLE_GRAPH").expect("TINE_INDEX_ORACLE_GRAPH");
    let out = std::env::var("TINE_INDEX_ORACLE_OUT").expect("TINE_INDEX_ORACLE_OUT");
    let copy = tempfile::tempdir().unwrap();
    copy_tree(Path::new(&source), copy.path());
    let (log, mismatches) = incremental_run(copy.path());
    // Indexes key pages by absolute path; the copy's location is incidental.
    let text = format!(
        "mismatches {}\n{}\n{log}",
        mismatches.len(),
        mismatches.join("\n")
    );
    fs::write(out, text.replace(copy.path().to_str().unwrap(), "<root>")).unwrap();
}

/// D-20261018-M2c, T4 (incremental = rebuild) at the store: after every step
/// of a seeded random sequence of alias-relevant edits (declare with `alias::`
/// or `aliases::`, drop, change case only, create, rename, delete) the
/// generation that patched the alias relation by delta has exactly the image
/// of a parent-free capture and of a reopened store, relation included.
#[test]
fn random_alias_edits_equal_a_fresh_build() {
    const POOL: &[&str] = &[
        "Ax", "ax", "AX", "Ay", "Shared", "Other", "Page 1", "page 2",
    ];
    for seed in [3u64, 11, 29] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for sub in ["pages", "journals", "logseq"] {
            fs::create_dir_all(root.join(sub)).unwrap();
        }
        let mut live: Vec<String> = Vec::new();
        for index in 0..6 {
            let rel = format!("pages/Page {index}.md");
            let head = if index % 2 == 0 {
                format!("alias:: {}\n\n", POOL[index])
            } else {
                String::new()
            };
            fs::write(
                root.join(&rel),
                format!("{head}- body [[Page {}]]\n", (index + 1) % 6),
            )
            .unwrap();
            live.push(rel);
        }
        let store = crate::store::Store::open(root, Default::default())
            .unwrap()
            .0;
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move |bound: usize| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng % bound as u64) as usize
        };
        let (mut log, mut mismatches) = (String::new(), Vec::new());
        // The alias shapes only: the referenced-name index keeps whichever
        // case variant of a name it met first, which differs between a
        // patched generation and a rebuilt one and is not under test here.
        let alias_lines = |line: &str| {
            [
                "aliases ",
                "lazy alias",
                "row ",
                "neighbors ",
                "originals ",
                "owners ",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        };
        check_lines(
            &store,
            root,
            "open",
            &[],
            &mut log,
            &mut mismatches,
            alias_lines,
        );
        let mut fresh_page = 0;
        for step in 0..40 {
            let target = live[next(live.len())].clone();
            let label = match next(10) {
                7 | 8 => {
                    // An owner change without editing the new owner: a title::
                    // makes this file claim another page's name (or, without
                    // one, gives the name back to its file name).
                    let claimed = ["Page 1", "Page 2", "Page 3"][next(3)];
                    let alias = POOL[next(POOL.len())];
                    let line = if next(2) == 0 {
                        format!("title:: {claimed}\naliases:: {alias}")
                    } else {
                        format!("aliases:: {alias}")
                    };
                    save_any(&store, &target, |doc| doc.pre_block = Some(line.clone()));
                    format!("set `{}` on {target}", line.replace('\n', " / "))
                }
                9 => {
                    // A second file claiming an existing name.
                    fresh_page += 1;
                    let rel = format!("pages/Twin {fresh_page}.md");
                    let claimed = ["Page 1", "Page 2", "Page 3"][next(3)];
                    let alias = POOL[next(POOL.len())];
                    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
                    tx.create(
                        &crate::PageId::from(rel.clone()).file(),
                        crate::Content::Bytes(
                            format!("title:: {claimed}\naliases:: {alias}\n\n- twin\n")
                                .into_bytes(),
                        ),
                    );
                    commit(tx, "create twin");
                    live.push(rel.clone());
                    format!("create {rel} claiming {claimed} and declaring {alias}")
                }
                0 | 1 => {
                    let key = ["alias", "aliases"][next(2)];
                    let values: Vec<_> = (0..1 + next(3)).map(|_| POOL[next(POOL.len())]).collect();
                    let line = format!("{key}:: {}", values.join(", "));
                    save_any(&store, &target, |doc| doc.pre_block = Some(line.clone()));
                    format!("declare `{line}` on {target}")
                }
                2 => {
                    save_any(&store, &target, |doc| doc.pre_block = None);
                    format!("drop aliases of {target}")
                }
                3 => {
                    // Case only: the same names, other capitals.
                    let id = crate::PageId::from(target.clone());
                    let flipped = store.page(&id).unwrap().doc.pre_block.map(|pre| {
                        pre.chars()
                            .map(|c| {
                                if c.is_uppercase() {
                                    c.to_ascii_lowercase()
                                } else {
                                    c.to_ascii_uppercase()
                                }
                            })
                            .collect::<String>()
                    });
                    save_any(&store, &target, |doc| doc.pre_block = flipped.clone());
                    format!("flip case of the properties of {target}")
                }
                4 => {
                    fresh_page += 1;
                    let rel = format!("pages/Created {fresh_page}.md");
                    let alias = POOL[next(POOL.len())];
                    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
                    tx.create(
                        &crate::PageId::from(rel.clone()).file(),
                        crate::Content::Bytes(
                            format!("aliases:: {alias}\n\n- created\n").into_bytes(),
                        ),
                    );
                    commit(tx, "create");
                    live.push(rel.clone());
                    format!("create {rel} declaring {alias}")
                }
                5 => {
                    fresh_page += 1;
                    let to = format!("pages/Moved {fresh_page}.md");
                    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
                    tx.move_file(
                        &crate::PageId::from(target.clone()).file(),
                        rev(root, &target),
                        &crate::PageId::from(to.clone()).file(),
                        None,
                    );
                    commit(tx, "rename");
                    live.retain(|rel| *rel != target);
                    live.push(to.clone());
                    format!("rename {target} to {to}")
                }
                _ if live.len() > 3 => {
                    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
                    tx.trash(
                        &crate::PageId::from(target.clone()).file(),
                        rev(root, &target),
                        crate::TrashIf::Any,
                    );
                    commit(tx, "delete");
                    live.retain(|rel| *rel != target);
                    format!("delete {target}")
                }
                _ => continue,
            };
            assert!(
                store.whole_graph().unwrap().graph.alias_list_built(),
                "seed {seed} step {step}: the generation inherits the relation by delta, \
                 so the comparison below is delta against rebuild"
            );
            check_lines(
                &store,
                root,
                &format!("seed {seed} step {step}: {label}"),
                &[],
                &mut log,
                &mut mismatches,
                alias_lines,
            );
        }
        store.close();
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
        assert!(
            log.contains("lazy alias_relation\nrow "),
            "not vacuous: the relation holds rows"
        );
        assert!(
            log.lines().any(|line| line.starts_with("row ")
                && line.contains("rel: \"pages/Twin")
                && line.contains("name: \"Page ")),
            "not vacuous: a file that only claims another page's name by title:: declares aliases"
        );
    }
}

/// B1 of the alias-delta review. A transaction fails at step 1 because an
/// outside writer replaced B, and the undo of its own create of A fails too:
/// `publish_final_files` caches A (own) and B (external) together, then the
/// store publishes twice (Own for A, External for B). Every generation
/// published must equal a parent-free capture of the cache it was captured
/// from, so B's old alias row, its reference to `Target` and its candidate
/// signatures are gone from both, and a view held from before keeps them.
#[test]
fn split_own_external_publication_equals_a_fresh_build() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for sub in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(
        root.join("pages/B.md"),
        "alias:: oldb\n\n- B sees [[Target]]\n",
    )
    .unwrap();
    fs::write(root.join("pages/Target.md"), "- target\n").unwrap();
    let store = crate::store::Store::open(
        root,
        crate::OpenOptions {
            watch: crate::WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let held = store.whole_graph().unwrap();
    assert!(matches!(
        held.resolve("oldb", false),
        crate::Resolved::Alias { .. }
    ));
    assert_eq!(held.backlinks("Target").unwrap().len(), 1);
    let published_before = store.changes.published_count();

    let read = store.page(&crate::PageId::from("pages/B.md")).unwrap();
    let mut doc = read.doc;
    doc.pre_block = Some("alias:: intendedb".into());
    store.inject_fault(crate::FaultPoint::Stage2MismatchAt(1));
    store.inject_fault(crate::FaultPoint::UndoWithdrawalIo);
    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
    tx.create(
        &crate::PageId::from("pages/A.md").file(),
        crate::Content::Bytes(b"alias:: newa\n\n- A\n".to_vec()),
    );
    tx.save_page(
        &[crate::EditKind::ReplacePage],
        &crate::PageId::from("pages/B.md"),
        crate::SaveBase::Existing(read.rev),
        &doc,
    );
    assert!(matches!(
        tx.commit(),
        crate::TxOutcome::NotCommitted { step: 1, .. }
    ));
    assert_eq!(
        fs::read_to_string(root.join("pages/B.md")).unwrap(),
        "external stage-2"
    );

    let probes = ["Target", "oldb", "newa", "intendedb"];
    let published = store
        .changes
        .published_images(published_before, &|snapshot| {
            snapshot_image(snapshot, &probes)
        });
    assert!(
        published.len() >= 2,
        "the failed transaction publishes its own file and the external one separately"
    );
    let current = store.graph.cache_generation();
    for (index, (generation, live, fresh)) in published.iter().enumerate() {
        // The cache is not touched after the last publication, so every
        // generation published at the current cache generation is comparable
        // with a capture from it.
        assert_eq!(
            *generation, current,
            "publication {index} was captured from another cache generation"
        );
        if let Some(difference) = first_difference(live, fresh) {
            panic!("publication {index}: incremental vs parent-free capture: {difference}");
        }
    }
    let after = store.whole_graph().unwrap();
    assert!(!matches!(
        after.resolve("oldb", false),
        crate::Resolved::Alias { .. }
    ));
    assert_eq!(after.backlinks("Target").unwrap().len(), 0);
    // A view acquired before the transaction keeps what it saw.
    assert!(matches!(
        held.resolve("oldb", false),
        crate::Resolved::Alias { .. }
    ));
    assert_eq!(held.backlinks("Target").unwrap().len(), 1);
    store.close();
}
