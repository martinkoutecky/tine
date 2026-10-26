//! F3 semantic oracle: construct both sides from typed fixture pieces, never
//! parse/replace the input with the production rewriter. OG 0.10.9
//! frontend/handler/page.cljs: replace-page-ref!, replace-tag-ref!,
//! rename-update-refs!, rename-namespace-pages!, merge-pages! establish the
//! link/tag rewrite and destination-before-source block order. Tine's existing
//! export-definition contract additionally rewrites definition selectors only.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tine_graph_features::pages;
use tine_store::{PageHost, PageId, Store};

pub(super) struct Rng(pub u64);

impl Rng {
    pub(super) fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize % n
    }

    pub(super) fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

pub(super) fn seeds() -> usize {
    let count = std::env::var("TINE_F3_SEEDS")
        .ok()
        .map(|n| n.parse().unwrap())
        .unwrap_or(1);
    assert!(count > 0, "TINE_F3_SEEDS must be positive");
    count
}

pub(super) struct Graph {
    pub(super) dir: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) store: Arc<Store>,
}

impl Graph {
    pub(super) fn new(files: &BTreeMap<String, Vec<u8>>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("graph");
        for area in ["pages", "journals", "assets", "logseq"] {
            fs::create_dir_all(root.join(area)).unwrap();
        }
        for (rel, bytes) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        Self { dir, root, store }
    }
}

impl Drop for Graph {
    fn drop(&mut self) {
        self.store.close();
    }
}

pub(super) fn collect(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), out);
            } else {
                out.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Only actual live files or the declared recovery roots count. Temporary
/// files and arbitrary copies elsewhere in the graph cannot satisfy this.
pub(super) fn conserved(
    root: &Path,
    payload: &[u8],
    live: &[String],
    recovery: &[PathBuf],
    at: &str,
) {
    let mut files = BTreeMap::new();
    for rel in live {
        if let Ok(bytes) = fs::read(root.join(rel)) {
            files.insert(root.join(rel), bytes);
        }
    }
    for dir in recovery {
        for (rel, bytes) in collect(dir) {
            if !rel.ends_with(".tmp") {
                files.insert(dir.join(rel), bytes);
            }
        }
    }
    assert!(
        files.values().any(|bytes| bytes == payload),
        "lost input payload at {at}: {:?}; locations: {:?}",
        String::from_utf8_lossy(payload),
        files.keys().collect::<Vec<_>>()
    );
}

/// Retained transaction/host namespaces, not the entire trash tree. These
/// schedules contain page moves/merges, asset trash, and undo conflict copies.
pub(super) fn transaction_recovery(root: &Path) -> Vec<PathBuf> {
    ["pages", "assets", "conflicts"]
        .map(|kind| root.join("logseq/.tine-trash").join(kind))
        .to_vec()
}

fn restore_id(name: &str) -> bool {
    let Some((stamp, counters)) = name.split_once("-pre-restore-extras-") else {
        return false;
    };
    let Some((pid, seq)) = counters.split_once('-') else {
        return false;
    };
    stamp.len() == 19
        && stamp.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 | 13 | 16 => b == b'-',
            10 => b == b'_',
            _ => b.is_ascii_digit(),
        })
        && !pid.is_empty()
        && !seq.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && seq.bytes().all(|b| b.is_ascii_digit())
}

/// Store::restore's two same-filesystem namespaces (storage-contract.md and
/// restore.rs API contract). Validate the reported pair before trusting it.
pub(super) fn restore_recovery(root: &Path, reported: &[PathBuf]) -> Vec<PathBuf> {
    let root = fs::canonicalize(root).unwrap();
    assert_eq!(reported.len(), 2, "restore must report both recovery roots");
    let mut roots = Vec::new();
    for parent in ["logseq/.tine-trash", "assets/.tine-restore-recovery"] {
        let matches: Vec<_> = reported
            .iter()
            .filter(|dir| dir.parent() == Some(root.join(parent).as_path()))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "undeclared restore recovery roots: {reported:?}"
        );
        let dir = matches[0];
        assert!(
            restore_id(dir.file_name().unwrap().to_str().unwrap()),
            "unrecognised restore id: {dir:?}"
        );
        assert!(
            fs::symlink_metadata(dir).unwrap().file_type().is_dir(),
            "recovery root must be a directory: {dir:?}"
        );
        assert_eq!(
            fs::canonicalize(dir).unwrap(),
            *dir,
            "retargeted recovery root"
        );
        roots.push(dir.clone());
    }
    assert_eq!(
        roots[0].file_name(),
        roots[1].file_name(),
        "mismatched restore ids"
    );
    roots
}

/// An aborted operation cannot return a report. Discover only recognised
/// restore-id pairs, never arbitrary hidden directories or transaction trash.
pub(super) fn discover_restore_recovery(root: &Path) -> Vec<PathBuf> {
    let root = fs::canonicalize(root).unwrap();
    let mut roots = Vec::new();
    for entry in fs::read_dir(root.join("logseq/.tine-trash")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if restore_id(name.to_str().unwrap()) && entry.file_type().unwrap().is_dir() {
            roots.extend(restore_recovery(
                &root,
                &[
                    entry.path(),
                    root.join("assets/.tine-restore-recovery").join(name),
                ],
            ));
        }
    }
    assert!(
        !roots.is_empty(),
        "abort did not leave a recognised restore namespace"
    );
    roots
}

#[test]
fn custody_rejects_undeclared_retained_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let hidden = root.join("assets/.not-declared-recovery");
    fs::create_dir_all(&hidden).unwrap();
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/Replace.md"), b"external during undo").unwrap();
    fs::write(hidden.join("old.edn"), b"displaced baseline").unwrap();
    let rejected = std::panic::catch_unwind(|| {
        conserved(
            root,
            b"displaced baseline",
            &["pages/Replace.md".into()],
            &transaction_recovery(root),
            "undeclared hidden folder",
        );
    });
    assert!(
        rejected.is_err(),
        "undeclared hidden bytes counted as custody"
    );
    // Positive control: the same payload in contract conflict trash counts.
    let declared = root.join("logseq/.tine-trash/conflicts");
    fs::create_dir_all(&declared).unwrap();
    fs::write(declared.join("old.md"), b"displaced baseline").unwrap();
    conserved(
        root,
        b"displaced baseline",
        &["pages/Replace.md".into()],
        &transaction_recovery(root),
        "declared conflict trash",
    );
}

#[test]
fn restore_custody_rejects_undeclared_roots_and_checks_all_displaced_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let id = "2026-10-23_12-34-56-pre-restore-extras-123-0";
    let reported = [
        root.join("logseq/.tine-trash").join(id),
        root.join("assets/.tine-restore-recovery").join(id),
    ];
    for path in &reported {
        fs::create_dir_all(path).unwrap();
    }
    let hidden = root.join("assets/.not-declared-recovery");
    fs::create_dir_all(&hidden).unwrap();
    fs::write(hidden.join("old.edn"), b"displaced asset").unwrap();
    let declared = restore_recovery(&root, &reported);
    assert_eq!(discover_restore_recovery(&root), declared);
    assert!(std::panic::catch_unwind(|| {
        conserved(
            &root,
            b"displaced asset",
            &["assets/paper.edn".into()],
            &declared,
            "misplaced restore asset",
        );
    })
    .is_err());
    assert!(
        std::panic::catch_unwind(|| {
            restore_recovery(&root, &[reported[0].clone(), hidden.clone()]);
        })
        .is_err(),
        "unrecognised reported root counted as custody"
    );
    fs::write(reported[1].join("paper.edn"), b"displaced asset").unwrap();
    conserved(
        &root,
        b"displaced asset",
        &["assets/paper.edn".into()],
        &declared,
        "reported restore root",
    );
}

/// Reuses F2/F5's complete snapshot image, forcing lazy indexes before each
/// operation and comparing against a physically separate cold Store afterward.
pub(super) fn fresh(store: &Store, root: &Path, probes: &[&str], at: &str) {
    let live = store.conformance_image_for_tests(probes);
    let copy = tempfile::tempdir().unwrap();
    for (rel, bytes) in collect(root) {
        // Recovery is not live graph text. Copy only graph inputs.
        if rel.starts_with("pages/")
            || rel.starts_with("journals/")
            || rel == "logseq/config.edn"
            || (rel.starts_with("assets/") && !rel.contains("/."))
        {
            let path = copy.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
    }
    let cold = Store::open(copy.path(), Default::default()).unwrap().0;
    let rebuilt = cold.conformance_image_for_tests(probes);
    cold.close();
    if live != rebuilt {
        let line = live
            .lines()
            .zip(rebuilt.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "live/fresh index mismatch at {at}, line {line}:\nlive: {:?}\nfresh: {:?}",
            live.lines().nth(line),
            rebuilt.lines().nth(line)
        );
    }
}

/// Positive/negative controls prevent a live/cold pair of empty observations
/// from certifying query-index correctness. The expected edge comes from the
/// fixture and operation schedule, not the parser or updated index.
pub(super) fn assert_ref_query(store: &Store, name: &str, referrer: &str, present: bool, at: &str) {
    assert_query_controls(store, name, referrer, present, present, at);
}

fn assert_query_controls(
    store: &Store,
    name: &str,
    referrer: &str,
    present: bool,
    linked: bool,
    at: &str,
) {
    use tine_core::refs::page_key;
    use tine_store::{QueryDialect, QueryResult};
    let view = store.whole_graph().unwrap();
    let source = format!("[[{name}]]");
    let QueryResult::Simple(groups) = view.query(&source, QueryDialect::Simple).unwrap() else {
        panic!("simple query must return groups");
    };
    assert_eq!(
        groups
            .iter()
            .any(|g| page_key(&g.page) == page_key(referrer)),
        present,
        "simple query control {source} -> {referrer} at {at}"
    );
    // Alias Simple queries have their own positive controls. TQL's name
    // conditions address a link node's real name; call this with real/fileless
    // names for link controls, and test aliases with Simple below.
    let quoted = format!("'{}'", name.replace('\'', "''"));
    for source in [
        format!("@page and any(links, name = {quoted})"),
        format!("@page and any(links, name = {quoted} and any(linked_by, true))"),
        format!("@page and near(name = {quoted}, 1, 'in')"),
    ] {
        let pages = crate::link_graph::pages(&view, &source);
        assert_eq!(
            pages.contains(&page_key(referrer)),
            linked,
            "TQL query control {source} -> {referrer} at {at}"
        );
    }
}

fn assert_alias_query(store: &Store, name: &str, at: &str) {
    let tine_store::QueryResult::Simple(groups) = store
        .whole_graph()
        .unwrap()
        .query(&format!("[[{name}]]"), tine_store::QueryDialect::Simple)
        .unwrap()
    else {
        panic!("simple query")
    };
    assert!(
        groups.iter().any(|g| g.page == "Ref0"),
        "positive alias query {name} at {at}"
    );
}

fn assert_incoming_query(store: &Store, target: &str, at: &str) {
    let targets = crate::link_graph::pages(
        &store.whole_graph().unwrap(),
        "@page and any(linked_by, name = 'Ref0')",
    );
    assert!(
        targets.contains(&tine_core::refs::page_key(target)),
        "positive incoming-link query -> {target} at {at}"
    );
}

const UUID: &str = "12345678-1234-4234-8234-123456789abc";

// A tiny typed language for the generated reference-bearing text. Rendering
// the expected graph changes the semantic name argument, never input bytes.
fn referrer(
    name: &str,
    original: &str,
    alias: &str,
    org: bool,
    child: bool,
    order: &[usize],
) -> String {
    let tag = if name.contains(' ') {
        format!("#[[{name}]]")
    } else {
        format!("#{name}")
    };
    let pieces = [
        format!("see [[{name}]] and {tag}"),
        format!("alias [[{alias}]]; plain {original}"),
        format!("block (({UUID}))"),
        if child {
            format!("child [[{name}/Kid]]")
        } else {
            "unrelated [[Elsewhere/Kid]]".into()
        },
        // Tine's existing RenameMap contract scans bracketed query arguments
        // in an already selected file. OG selects individual ref-bearing
        // blocks; this fixture certifies Tine's current file-level policy.
        format!("{{{{query [[{name}]]}}}}"),
    ];
    order
        .iter()
        .map(|i| format!("{} {}\n", if org { "*" } else { "-" }, pieces[*i]))
        .collect()
}

fn definition(name: &str, original: &str, org: bool) -> String {
    if org {
        format!("* {{{{tine-query @page name = '{name}'}}}}\n:PROPERTIES:\n:export: site\n:END:\n* {{{{tine-query @page name = '{original}'}}}}\n")
    } else {
        format!("- {{{{tine-query @page name = '{name}'}}}}\n  export:: site\n- {{{{tine-query @page name = '{original}'}}}}\n")
    }
}

#[test]
fn generated_semantic_rename_merge_matches_independent_bytes_and_fresh_indexes() {
    let _guard = crate::integration_lock();
    let mut graphs = 0;
    let mut rewritten = 0;
    let mut held = 0;
    for seed in 1..=seeds() {
        let mut rng = Rng(0xf3_20261023 ^ seed as u64);
        for org in [false, true] {
            for mode in 0..4 {
                // Host rename, retained namespace/rename-merge, host plain merge.
                let child = mode == 1;
                let ext = if org { "org" } else { "md" };
                let bullet = if org { "*" } else { "-" };
                let old = format!(
                    "{}{}",
                    ["Alpha", "Čaj", "项目", "MiXeD"][rng.below(4)],
                    seed
                );
                let new = format!(
                    "{}{}",
                    ["Next", "Next Space", "Změna", "新页"][rng.below(4)],
                    seed
                );
                let alias = format!("Former{seed}");
                let after = if mode == 3 {
                    old.as_str()
                } else {
                    new.as_str()
                };
                let source = format!("pages/{old}.{ext}");
                let dest = format!("pages/{new}.{ext}");
                let before_body = format!("{bullet} source-{seed} [[{old}]]\n");
                // Explicit titles exercise rebind; aliases must keep their own spelling.
                let header = |name: &str| {
                    if org {
                        format!("#+TITLE: {name}\n#+ALIAS: {alias}\n")
                    } else {
                        format!("title:: {name}\nalias:: {alias}\n\n")
                    }
                };
                let mut before = BTreeMap::from([
                    (
                        "logseq/config.edn".into(),
                        b"{:file/name-format :triple-lowbar}\n".to_vec(),
                    ),
                    (
                        source.clone(),
                        format!("{}{before_body}", header(&old)).into_bytes(),
                    ),
                ]);
                let mut expected = before.clone();
                expected.remove(&source);
                if mode >= 2 {
                    // Existing Tine merge contract: destination aliases first,
                    // deduplicated source aliases after them; blocks append.
                    let aliases = |value: &str| {
                        if org {
                            format!("#+ALIAS: {value}\n")
                        } else {
                            format!("alias:: {value}\n\n")
                        }
                    };
                    before.insert(
                        source.clone(),
                        format!("{}{before_body}", aliases(&format!("{alias}, Shared")))
                            .into_bytes(),
                    );
                    before.insert(
                        dest.clone(),
                        format!("{}{bullet} kept-{seed}\n", aliases("Shared, Kept")).into_bytes(),
                    );
                    expected.insert(
                        dest.clone(),
                        format!(
                            "{}{bullet} kept-{seed}\n{bullet} source-{seed} [[{after}]]\n",
                            aliases(&format!("Shared, Kept, {alias}"))
                        )
                        .into_bytes(),
                    );
                } else {
                    expected.insert(
                        dest.clone(),
                        format!("{}{bullet} source-{seed} [[{new}]]\n", header(&new)).into_bytes(),
                    );
                }
                if child {
                    before.insert(
                        format!("pages/{old}___Kid.{ext}"),
                        format!("{bullet} kid [[{old}]]\n").into_bytes(),
                    );
                    expected.insert(
                        format!("pages/{new}___Kid.{ext}"),
                        format!("{bullet} kid [[{new}]]\n").into_bytes(),
                    );
                }
                let mut order: Vec<_> = (0..5).collect();
                for i in 0..2 + rng.below(3) {
                    rng.shuffle(&mut order);
                    let rel = format!("pages/Ref{i}.{ext}");
                    before.insert(
                        rel.clone(),
                        referrer(&old, &old, &alias, org, child, &order).into_bytes(),
                    );
                    expected.insert(
                        rel,
                        referrer(after, &old, &alias, org, child, &order).into_bytes(),
                    );
                    rewritten += 1;
                }
                let journal = format!("journals/2026_10_23.{ext}");
                before.insert(
                    journal.clone(),
                    format!("{bullet} journal [[{old}]]\n").into_bytes(),
                );
                expected.insert(
                    journal,
                    format!("{bullet} journal [[{after}]]\n").into_bytes(),
                );
                let def = format!("pages/Definition.{ext}");
                before.insert(def.clone(), definition(&old, &old, org).into_bytes());
                expected.insert(def, definition(after, &old, org).into_bytes());
                // Persisted id and its references must survive every rename/merge.
                let id_body = if org {
                    format!("* identity\n:PROPERTIES:\n:ID: {UUID}\n:END:\n")
                } else {
                    format!("- identity\n  id:: {UUID}\n")
                };
                before.insert(format!("pages/Identity.{ext}"), id_body.as_bytes().to_vec());
                expected.insert(format!("pages/Identity.{ext}"), id_body.into_bytes());
                let graph = Graph::new(&before);
                let probes = [&*old, &*new, &*alias, "Ref0"];
                let at = format!("seed {seed}, org {org}, mode {mode}");
                fresh(&graph.store, &graph.root, &probes, &at);
                assert_ref_query(&graph.store, &old, "Ref0", true, &at);
                // Merge fixtures share the alias Shared, so the link table
                // already connects both physical owners via alias references.
                // A literal Simple query still has no new-name block yet.
                assert_query_controls(&graph.store, &new, "Ref0", false, mode >= 2, &at);
                assert_alias_query(&graph.store, &alias, &at);
                assert_incoming_query(&graph.store, &old, &at);
                let host =
                    PageHost::start_for_tests(&graph.store, &graph.dir.path().join("app")).unwrap();
                // Public wire DTOs are serialized just as external clients see them.
                let session = serde_json::to_value(host.window_reloaded()).unwrap();
                let sid = session["session"].as_u64().unwrap();
                let id = session["nextId"].as_u64().unwrap();
                let opened = host
                    .open(sid, id, &PageId::from(format!("pages/Ref0.{ext}")), "Ref0")
                    .unwrap();
                let key = serde_json::to_value(opened).unwrap()["key"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                assert_eq!(
                    host.wait_published(sid, &[(key.clone(), 0, None)], Duration::from_secs(10)),
                    tine_store::PageOperation::Applied,
                    "Open applied and its clean held buffer is indexed: {at}"
                );
                held += 1;
                if mode == 3 {
                    // This legacy merge entry deliberately does not rename
                    // inbound refs or source-body links (pages::merge_pages).
                    pages::merge_pages(&graph.store, &host, &source, &dest)
                        .unwrap_or_else(|e| panic!("{at}: {e}"));
                } else {
                    pages::rename_or_merge_page(
                        &graph.store,
                        &host,
                        &old,
                        &new,
                        Some(&source),
                        (mode == 2).then_some(dest.as_str()),
                        &[],
                    )
                    .unwrap_or_else(|e| panic!("{at}: {e}"));
                }
                fresh(&graph.store, &graph.root, &probes, &at);
                assert_ref_query(&graph.store, after, "Ref0", true, &at);
                assert_query_controls(
                    &graph.store,
                    if mode == 3 { &new } else { &old },
                    "Ref0",
                    false,
                    mode == 3,
                    &at,
                );
                assert_alias_query(&graph.store, &alias, &at);
                assert_incoming_query(&graph.store, &new, &at);
                host.close(sid, id + 1, &key).unwrap();
                assert!(host.quiesce_for_tests(Duration::from_secs(10)), "{at}");
                let actual: BTreeMap<_, _> = collect(&graph.root)
                    .into_iter()
                    .filter(|(rel, _)| {
                        expected.contains_key(rel)
                            || rel.starts_with("pages/")
                            || rel.starts_with("journals/")
                    })
                    .collect();
                assert_eq!(actual, expected, "independent exact bytes: {at}");
                if mode >= 2 {
                    conserved(
                        &graph.root,
                        before.get(&source).unwrap(),
                        &[source.clone(), dest.clone()],
                        &transaction_recovery(&graph.root),
                        &at,
                    );
                }
                fresh(&graph.store, &graph.root, &probes, &at);
                drop(host);
                graphs += 1;
            }
        }
    }
    assert_eq!(graphs, seeds() * 8);
    assert_eq!(held, graphs);
    eprintln!("F3 semantics: graphs={graphs}, held={held}, generated_referrers={rewritten}, image_checks={}", graphs*3);
}

#[test]
fn generated_title_twins_refuse_without_changing_bytes_or_indexes() {
    let _guard = crate::integration_lock();
    for seed in 1..=seeds() {
        for org in [false, true] {
            let ext = if org { "org" } else { "md" };
            let body = if org {
                format!("#+TITLE: Twin{seed}\n* keep\n")
            } else {
                format!("title:: Twin{seed}\n\n- keep\n")
            };
            let files = BTreeMap::from([
                (format!("pages/One.{ext}"), body.as_bytes().to_vec()),
                (format!("pages/Two.{ext}"), body.as_bytes().to_vec()),
            ]);
            let graph = Graph::new(&files);
            let host =
                PageHost::start_for_tests(&graph.store, &graph.dir.path().join("app")).unwrap();
            let image = graph.store.conformance_image_for_tests(&[]);
            let error = pages::rename_page_expected(
                &graph.store,
                &host,
                &format!("Twin{seed}"),
                "Next",
                None,
            )
            .unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
            for (rel, bytes) in &files {
                assert_eq!(&fs::read(graph.root.join(rel)).unwrap(), bytes);
            }
            assert_eq!(image, graph.store.conformance_image_for_tests(&[]));
            fresh(&graph.store, &graph.root, &[], "ambiguous title refusal");
            drop(host);
        }
    }
    eprintln!("F3 title twins: refused={}", seeds() * 2);
}
