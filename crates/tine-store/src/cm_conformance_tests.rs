//! Conceptual model conformance, step 1 (private queue card "Conceptual model:
//! conformance harness + divergence map"). Each fixture graph, and a few
//! hundred seeded random graphs, go through this store and through the Lean
//! model's executable bridge (`cm-bridge`, path in `CM_BRIDGE`); the test
//! lists every place where they answer differently and compares that list
//! with `tests/fixtures/cm-conformance/expected-differences.json`.
//!
//! The model's given functions come from Tine: names, titles, aliases, the
//! outline and each block's occurrences are extracted with Tine's parser and
//! handed to the bridge, so a difference is in the meaning layer (pages,
//! owners, navigation, Linked References, counts, alias groups, the link
//! table, queries), never in parsing. The identity fold handed to the model is
//! Tine's: `page_key`, with a date-shaped spelling folded to its day. So is
//! the URL-origin half of the namespace test (`urls`, round-10 bridge).
//!
//! Rename (rule 6) needs the page host and `tine-graph-features`; it lives in
//! `crates/tine-graph-features/tests/cm_conformance_rename.rs`, sharing
//! `cm_conformance_shared_tests.rs` with this file.
//!
//! Step 1 records differences; it changes no production code. Without
//! `CM_BRIDGE` the test skips. `CM_REPORT` names a file for the full report;
//! `CM_REFRESH=1` rewrites the expected differences; `CM_RANDOM=<n>` changes
//! the number of random graphs (default 300).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};
use tine_core::refs::page_key;

use tine_core::model::{PageEntry, PageKind};

use crate::model::compare_page_claimants;
use crate::query::document_alias_spellings;
use crate::Store;

/// The store names the shared code uses (`super::store_crate` there).
mod store_crate {
    pub(crate) use crate::{
        Cancel, InventoryEntry, InventoryScope, IrAnswer, IrRequest, Resolved, Store, WholeGraph,
    };
}

#[path = "cm_conformance_shared_tests.rs"]
mod shared;

#[path = "cm_conformance_extraction_tests.rs"]
mod extraction;

use shared::{Ctx, Diff, GraphSpec, Section};

const RANDOM_DEFAULT: usize = 300;
const WORKERS: usize = 4;

/// Batch 20: the bridge must receive the prose block too. Otherwise an
/// empty difference report cannot detect its omission from query answers.
#[test]
fn preamble_projection_keeps_prose_and_outline_positions() {
    let fixture = json!({"files": {
        "pages/Markdown.md": "related:: [[Target]]\n\nProse [[Target]]\n\n- body\n",
        "pages/Org.org": "#+TITLE: Org\n#+RELATED: [[Target]]\n\nProse [[Target]]\n* body\n",
    }});
    let spec = GraphSpec::from_fixture("preamble-projection", &fixture);
    let dir = tempfile::tempdir().unwrap();
    spec.write(dir.path());
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    for (rel, text) in &spec.files {
        let file = shared::file_json(
            rel,
            text,
            document_alias_spellings,
            tine_core::config::FileNameFormat::Legacy,
            &Default::default(),
        );
        let blocks = file["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 3, "{rel}: header, prose, outline: {blocks:?}");
        assert_eq!(blocks[1]["text"], "Prose [[Target]]", "{rel}");
        assert_eq!(blocks[1]["occ"], json!([["link", "Target"]]), "{rel}");
        assert_eq!(blocks[2]["text"], "body", "{rel}");
    }
    store.close();
}

/// The Linked References link table (Q9's third owner, crate private) as
/// pairs of model keys.
fn link_pairs(
    view: &crate::WholeGraph,
    input: &Value,
    diffs: &mut Vec<Diff>,
) -> BTreeSet<(String, String)> {
    let snapshot = view.test_read_snapshot();
    let Some(table) = snapshot.link_graph(&|| false) else {
        diffs.push(Diff::new("harness: link table not built", "link table"));
        return BTreeSet::new();
    };
    // This historical model uses untyped date-folding keys. Visit either
    // kind of the live table's typed key through every spelling in play.
    let mut model_key: BTreeMap<String, String> = BTreeMap::new();
    for (s, k) in input["keys"].as_object().unwrap() {
        model_key
            .entry(page_key(s))
            .or_insert_with(|| k.as_str().unwrap().to_owned());
    }
    let mut node_key: BTreeMap<u32, String> = BTreeMap::new();
    for (pk, mk) in &model_key {
        for journal in [false, true] {
            if let Some(n) = table.node(pk, journal) {
                if (n as usize) < table.physical() {
                    node_key.entry(n).or_insert_with(|| mk.clone());
                }
            }
        }
    }
    for (i, v) in table.virtuals().iter().enumerate() {
        let pk = page_key(&v.name);
        let mk = model_key.get(&pk).cloned().unwrap_or(pk);
        node_key.insert((table.physical() + i) as u32, mk);
    }
    let total = table.physical() + table.virtuals().len();
    let mut pairs = BTreeSet::new();
    for n in 0..total as u32 {
        let Some(s) = node_key.get(&n) else {
            diffs.push(Diff::new(
                "harness: link table node without a known key",
                format!("link table node {n}"),
            ));
            continue;
        };
        for t in table.links(n) {
            match node_key.get(t) {
                Some(t) => {
                    pairs.insert((s.clone(), t.clone()));
                }
                None => diffs.push(Diff::new(
                    "harness: link table node without a known key",
                    format!("link table node {t}"),
                )),
            }
        }
    }
    pairs
}

/// What `Store::page_named(name, kind)` opens, as a relative path. While the
/// store is still loading this is the file-listing resolver
/// (`page_named_from_files`); a name it cannot answer makes `page_named` wait
/// for the graph, so a wait longer than `patience` releases the paused load
/// (`release`) and is reported as `Err`.
fn page_named_path(
    store: &std::sync::Arc<Store>,
    name: &str,
    kind: PageKind,
    patience: Option<(std::time::Duration, &Path)>,
) -> Result<Option<String>, String> {
    let Some((patience, release)) = patience else {
        return Ok(store
            .page_named(name, kind)
            .map_err(|e| format!("{e:?}"))?
            .map(|read| read.id.as_str().to_owned()));
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let s = std::sync::Arc::clone(store);
    let n = name.to_owned();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(
            s.page_named(&n, kind)
                .map(|r| r.map(|read| read.id.as_str().to_owned()))
                .map_err(|e| format!("{e:?}")),
        );
    });
    match rx.recv_timeout(patience) {
        Ok(answer) => {
            worker.join().unwrap();
            answer
        }
        Err(_) => {
            let _ = fs::remove_file(release);
            worker.join().unwrap();
            Err("waited for the graph".into())
        }
    }
}

/// Census Q5 item 3 (CM step 2, batch 10): the namespace parents have one
/// rule (`model::namespace_parents`) read two ways, the whole-graph pass
/// (`inventory(All)`, page queries, All Pages' counts) and the one-key range
/// read (the publication's inventory delta). They must agree on every key the
/// graph spells and every prefix of one; the bridge comparison then checks the
/// whole-graph answer against the model's pages (S8).
fn compare_namespace_parent_answers(view: &crate::WholeGraph) -> Vec<Diff> {
    let graph = &view.graph;
    let table = graph.referenced_name_table();
    let whole = graph.namespace_seeds(table);
    let mut keys: BTreeSet<String> = whole.keys().cloned().collect();
    for entry in &view.inventory(crate::InventoryScope::All).0 {
        let key = page_key(&entry.name);
        for (_, prefix) in crate::model::namespace_parents::parent_prefixes(&key) {
            keys.insert(prefix);
        }
        keys.insert(key);
    }
    keys.into_iter()
        .filter_map(|key| {
            let one = graph.namespace_seed(table, &key);
            (one != whole.get(&key).copied()).then(|| {
                Diff::new(
                    "namespace parents: the one-key answer differs from the whole-graph answer",
                    format!("{key:?}: whole {:?}, one key {one:?}", whole.get(&key)),
                )
            })
        })
        .collect()
}

/// Census Q4 and AL2 (CM step 2, batch 2): every computer of "which file owns
/// this name" against the one rule. The model cannot judge twins (its order
/// on conflicted claimants is O1's fallback, path order), so the reference is
/// Tine's documented claimant order (`compare_page_claimants`, SPEC §2.1
/// "date-stem journal, matching name, Markdown, file name, path") for a real
/// name, and the first owner by relative path (`owner_paths`, D-20261019-AL2
/// / CI4) for an alias-only name. A computer that disagrees is a second
/// answerer (I-12). `loading` is a second store over a copy of the graph,
/// held before its first load, so `page_named` answers from the file
/// listing. Journal duplicate days are checked against `journal_conflicts` in
/// the graph-features harness.
fn compare_owner_answers(
    store: &std::sync::Arc<Store>,
    loading: Option<(&std::sync::Arc<Store>, &Path)>,
    view: &crate::WholeGraph,
    ctx: &Ctx,
) -> Vec<Diff> {
    use crate::{InventoryScope, Resolved};
    let mut diffs = Vec::new();
    let snapshot = view.test_read_snapshot();
    let entries = snapshot.physical_page_list_arc();
    let physical: Vec<&PageEntry> = entries.iter().filter(|e| e.rel_path.is_some()).collect();
    let rel_of: BTreeMap<&Path, &str> = physical
        .iter()
        .map(|e| (e.path.as_path(), e.rel_path_str()))
        .collect();
    let mut by_key: BTreeMap<String, Vec<&PageEntry>> = BTreeMap::new();
    for e in &physical {
        by_key.entry(page_key(&e.name)).or_default().push(e);
    }
    let least = |group: &[&PageEntry]| -> Option<String> {
        group
            .iter()
            .min_by(|a, b| compare_page_claimants(a, b))
            .map(|e| e.rel_path_str().to_owned())
    };
    let kind_name = |k: PageKind| {
        if k == PageKind::Journal {
            "journal"
        } else {
            "page"
        }
    };
    let inventory = view.inventory(InventoryScope::All);
    let real = crate::query::real_page_names(&snapshot);
    let patience = std::time::Duration::from_secs(5);
    for (key, group) in &by_key {
        let rule = least(group).unwrap();
        let twins = if group.len() > 1 {
            "twins"
        } else {
            "one claimant"
        };
        // The real-name table (alias group canonical name, `self_page`, the
        // link table): one winner per key, every kind.
        let table = real.get(key).map(|(p, _)| {
            rel_of
                .get(p.as_path())
                .copied()
                .unwrap_or("<unlisted>")
                .to_owned()
        });
        if table.as_deref() != Some(rule.as_str()) {
            diffs.push(Diff::new(
                format!("owners: real-name table vs claimant order ({twins})"),
                format!("owner of {key:?}: claimant order {rule}, real-name table {table:?}"),
            ));
        }
        for kind in [PageKind::Page, PageKind::Journal] {
            let of_kind: Vec<&PageEntry> =
                group.iter().copied().filter(|e| e.kind == kind).collect();
            let Some(rule) = least(&of_kind) else {
                continue;
            };
            let k = kind_name(kind);
            let twins = if of_kind.len() > 1 {
                "twins"
            } else {
                "one claimant"
            };
            let spellings: BTreeSet<&str> = of_kind.iter().map(|e| e.name.as_str()).collect();
            for name in spellings {
                let resolved = match view.resolve(name, kind == PageKind::Journal) {
                    Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
                    _ => None,
                };
                if resolved.as_deref() != Some(rule.as_str()) {
                    diffs.push(Diff::new(
                        format!("owners: resolve vs claimant order ({k}, {twins})"),
                        format!("{k} {name:?}: claimant order {rule}, resolve {resolved:?}"),
                    ));
                }
                let listed = inventory
                    .0
                    .iter()
                    .find(|e| {
                        e.is_journal == (kind == PageKind::Journal) && page_key(&e.name) == *key
                    })
                    .and_then(|e| match &e.target {
                        Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
                        _ => None,
                    });
                if listed.as_deref() != Some(rule.as_str()) {
                    diffs.push(Diff::new(
                        format!("owners: inventory vs claimant order ({k}, {twins})"),
                        format!("{k} {name:?}: claimant order {rule}, inventory {listed:?}"),
                    ));
                }
                match page_named_path(store, name, kind, None) {
                    Ok(Some(p)) if p == rule => {}
                    other => diffs.push(Diff::new(
                        format!("owners: page_named vs claimant order ({k}, {twins})"),
                        format!("{k} {name:?}: claimant order {rule}, page_named {other:?}"),
                    )),
                }
                if let Some((loading, release)) = loading {
                    match page_named_path(loading, name, kind, Some((patience, release))) {
                        Ok(Some(p)) if p == rule => {}
                        other => diffs.push(Diff::new(
                            format!("owners: page_named while loading vs claimant order ({k}, {twins})"),
                            format!("{k} {name:?}: claimant order {rule}, page_named while loading {other:?}"),
                        )),
                    }
                }
            }
            // O1: the model totalises a conflicted key by its own file order
            // (path order). Not a Tine defect; it explains model differences
            // in duplicate-claim graphs (e.g. "[duplicate claim] navigate").
            let model_key = ctx.key(&of_kind[0].name);
            if ctx.conflicts.contains(&model_key) {
                if let Some(model) = ctx.owner_of_page.get(&(k.to_owned(), model_key.clone())) {
                    if *model != rule {
                        diffs.push(Diff::new(
                            "O1 owners: model path order vs Tine claimant order (model fallback, not a Tine defect)",
                            format!("owner of ({k}, {model_key:?}): model {model}, Tine {rule}"),
                        ));
                    }
                }
            }
        }
    }
    // Alias-only names: the first owner by relative path.
    let spellings: BTreeSet<String> = ctx.files.values().flat_map(|f| f.aliases.clone()).collect();
    for s in spellings {
        let key = page_key(&s);
        let Some(owners) = snapshot
            .alias_owner_paths(&key, None)
            .filter(|o| !o.is_empty())
        else {
            continue;
        };
        let mut sorted = owners.clone();
        sorted.sort();
        if sorted != owners {
            diffs.push(Diff::new(
                "alias owner: owner_paths not in path order",
                format!("[[{s}]]: {owners:?}"),
            ));
        }
        let first = sorted[0].clone();
        let several = if owners.iter().collect::<BTreeSet<_>>().len() > 1 {
            "several owners"
        } else {
            "one owner"
        };
        // A declaration on a file that does not own its own name (a twin):
        // Tine counts it, the model counts owners' declarations only.
        for owner in owners.iter().collect::<BTreeSet<_>>() {
            let Some(entry) = physical.iter().find(|e| e.rel_path_str() == owner) else {
                continue;
            };
            let twins: Vec<&PageEntry> = by_key[&page_key(&entry.name)]
                .iter()
                .copied()
                .filter(|e| e.kind == entry.kind)
                .collect();
            if least(&twins).as_deref() != Some(owner.as_str()) {
                diffs.push(Diff::new(
                    "alias owner: declared by a non-owner twin (model: owners' declarations only)",
                    format!(
                        "[[{s}]] declared by {owner}, which does not own {:?}",
                        entry.name
                    ),
                ));
            }
        }
        if by_key.contains_key(&key) {
            continue; // a real name: not alias-only
        }
        let owner_name = physical
            .iter()
            .find(|e| e.rel_path_str() == first)
            .map(|e| (e.name.clone(), e.kind == PageKind::Journal));
        match view.resolve(&s, false) {
            Resolved::Alias { owners } if owners[0].as_str() == first => {}
            other => diffs.push(Diff::new(
                format!("alias owner: resolve vs first owner by path ({several})"),
                format!("[[{s}]]: first owner {first}, resolve {other:?}"),
            )),
        }
        match page_named_path(store, &s, PageKind::Page, None) {
            Ok(Some(p)) if p == first => {}
            other => diffs.push(Diff::new(
                format!("alias owner: page_named vs first owner by path ({several})"),
                format!("[[{s}]]: first owner {first}, page_named {other:?}"),
            )),
        }
        let link = view.link_target(&s);
        if Some((link.name.clone(), link.is_journal)) != owner_name {
            diffs.push(Diff::new(
                format!("alias owner: link_target vs first owner by path ({several})"),
                format!("[[{s}]]: first owner {first} {owner_name:?}, link_target {link:?}"),
            ));
        }
        let listed = inventory
            .0
            .iter()
            .find(|e| !e.is_journal && page_key(&e.name) == key);
        let listed_first = listed.and_then(|e| match &e.target {
            Resolved::Alias { owners } => owners.first().map(|o| o.as_str().to_owned()),
            _ => None,
        });
        if listed_first.as_deref() != Some(first.as_str()) {
            diffs.push(Diff::new(
                format!("alias owner: inventory vs first owner by path ({several})"),
                format!("[[{s}]]: first owner {first}, inventory {listed_first:?}"),
            ));
        }
    }
    diffs
}

type Droppers = std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>;
/// Per fixture, the case the frontend check runs (`shared::frontend_case`).
type Frontend = std::sync::Mutex<BTreeMap<String, Value>>;

/// One graph through the store and the model; its differences.
fn run_graph(
    bridge: &Path,
    spec: &GraphSpec,
    droppers: &Droppers,
    frontend: &Frontend,
) -> Vec<Diff> {
    let dir = tempfile::tempdir().unwrap();
    spec.write(dir.path());
    // The same graph in a second store held before its first load: what
    // `page_named` answers from the file listing (census Q8 item 1).
    let loading_dir = tempfile::tempdir().unwrap();
    spec.write(loading_dir.path());
    let hold = loading_dir.path().join(".tine-test-pause-load");
    fs::write(&hold, "").unwrap();
    let loading = std::sync::Arc::new(
        Store::open(loading_dir.path(), Default::default())
            .unwrap()
            .0,
    );
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let view = store.whole_graph().expect("graph loads");
    let mut diffs = Vec::new();
    // The rename harness uses the test-side copy of the alias extractor:
    // hold it to the original here, on every file of every graph.
    for (rel, text) in &spec.files {
        let parsed = shared::parse_file(rel, text);
        let original = document_alias_spellings(&parsed);
        let copy = shared::alias_spellings(&parsed);
        if original != copy {
            diffs.push(Diff::new(
                "harness: alias extractor copy differs",
                format!("{rel}: store {original:?}, copy {copy:?}"),
            ));
        }
    }
    let input = shared::model_input(spec, document_alias_spellings, &[]);
    let model = match shared::run_bridge(bridge, &input) {
        Ok(m) => m,
        Err(e) => {
            diffs.push(Diff::new("harness: bridge failed", e));
            drop(view);
            let _ = fs::remove_file(&hold);
            droppers.lock().unwrap().push(std::thread::spawn(move || {
                drop((store, dir, loading, loading_dir))
            }));
            return diffs;
        }
    };
    let ctx = Ctx::new(&view, spec.journal_format(), &input, &model);
    diffs.extend(shared::compare_meaning(&ctx, &spec.queries));
    if !matches!(loading.is_graph_ready(), Ok(false)) {
        diffs.push(Diff::new(
            "harness: the held store is not loading",
            format!("{:?}", loading.is_graph_ready()),
        ));
    }
    diffs.extend(compare_owner_answers(
        &store,
        Some((&loading, hold.as_path())),
        &view,
        &ctx,
    ));
    // (A timed-out answer released the hold and is reported as a wait.)
    if hold.exists() && !matches!(loading.is_graph_ready(), Ok(false)) {
        diffs.push(Diff::new(
            "harness: the held store finished loading during the owner check",
            format!("{:?}", loading.is_graph_ready()),
        ));
    }
    let _ = fs::remove_file(&hold);
    diffs.extend(compare_namespace_parent_answers(&view));
    if !spec.name.starts_with("random-") {
        diffs.extend(shared::compare_reference_days(&ctx));
        let mut case = shared::frontend_case(
            &ctx,
            spec.journal_title_format.as_deref(),
            spec.journal_file_format.as_deref(),
        );
        // Per routable name, the file the backend opens for it
        // (`Store::page_named`, the one page-by-name door): what the
        // frontend's owner picks must equal.
        let owner: BTreeMap<String, Value> = case["open"]
            .as_object()
            .unwrap()
            .keys()
            .map(|n| {
                let at = |kind| page_named_path(&store, n, kind, None).ok().flatten();
                (
                    n.clone(),
                    json!({"page": at(PageKind::Page), "journal": at(PageKind::Journal)}),
                )
            })
            .collect();
        case["owner"] = json!(owner);
        frontend.lock().unwrap().insert(spec.name.clone(), case);
    }
    let pairs = link_pairs(&view, &input, &mut diffs);
    diffs.extend(shared::compare_links(&ctx, &pairs));
    if spec.name.starts_with("random-") && shared::has_duplicate_claim(&ctx) {
        diffs = shared::coarse(diffs);
    }
    // Closing a store joins its watcher (about 150 ms of waiting); do it off
    // the critical path, joined at the end of the test.
    drop(ctx);
    drop(view);
    droppers.lock().unwrap().push(std::thread::spawn(move || {
        drop((store, dir, loading, loading_dir))
    }));
    diffs.sort();
    diffs.dedup();
    diffs
}

fn fixture_specs() -> Vec<GraphSpec> {
    let dir = shared::fixture_dir(env!("CARGO_MANIFEST_DIR"));
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name()
                    .is_some_and(|n| n != "expected-differences.json")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let v: Value = serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap();
            GraphSpec::from_fixture(&p.file_stem().unwrap().to_string_lossy(), &v)
        })
        .collect()
}

/// The frontend checks' inputs (`tests/fixtures/cm-conformance/<subdir>/`)
/// are written by this harness and committed, so `npm test` runs the
/// frontend's real decisions without the bridge. Stale or missing cases
/// fail here; `CM_REFRESH=1` rewrites them.
fn check_frontend_cases(subdir: &str, cases: BTreeMap<String, Value>, hint: &str) {
    check_frontend_cases_skipping(subdir, cases, &BTreeSet::new(), hint);
}

/// As [`check_frontend_cases`], leaving the committed cases of `skipped`
/// fixtures alone: this volume cannot write them ([`shared::listed_apart`]).
fn check_frontend_cases_skipping(
    subdir: &str,
    cases: BTreeMap<String, Value>,
    skipped: &BTreeSet<String>,
    hint: &str,
) {
    let dir = shared::fixture_dir(env!("CARGO_MANIFEST_DIR")).join(subdir);
    let refresh = std::env::var_os("CM_REFRESH").is_some();
    if refresh {
        fs::create_dir_all(&dir).unwrap();
        for e in fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            if p.extension().is_some_and(|x| x == "json")
                && !cases.contains_key(&stem)
                && !skipped.contains(&stem)
            {
                fs::remove_file(&p).unwrap();
            }
        }
    }
    let mut stale = Vec::new();
    for (name, case) in &cases {
        let path = dir.join(format!("{name}.json"));
        let text = serde_json::to_string_pretty(case).unwrap() + "\n";
        if refresh {
            fs::write(&path, text).unwrap();
        } else if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            stale.push(name.clone());
        }
    }
    if !refresh {
        let on_disk = fs::read_dir(&dir)
            .map(|d| {
                d.filter_map(|e| {
                    let p = e.ok()?.path();
                    Some(p.file_stem()?.to_string_lossy().into_owned())
                })
                .filter(|n| !cases.contains_key(n) && !skipped.contains(n))
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        stale.extend(
            on_disk
                .into_iter()
                .map(|n| format!("{n} (no such fixture)")),
        );
    }
    assert!(
        stale.is_empty(),
        "conceptual model conformance: {subdir} cases are stale: {stale:?}; refresh with {hint}"
    );
}

#[test]
fn conceptual_model_conformance() {
    let Ok(bridge) = std::env::var("CM_BRIDGE") else {
        assert!(
            std::env::var_os("CM_REQUIRE_BRIDGE").is_none(),
            "the r13 gate requires CM_BRIDGE; conformance must actually run"
        );
        eprintln!("CM_BRIDGE unset: conceptual model conformance skipped");
        return;
    };
    let bridge = Path::new(&bridge);
    if shared::r13::is_bridge(bridge) {
        shared::r13::store_conformance(
            bridge,
            fixture_specs(),
            env!("CARGO_MANIFEST_DIR"),
            |view, input, model| {
                let mut diffs = Vec::new();
                let actual = link_pairs(view, input, &mut diffs);
                let expected: BTreeSet<_> = model["links"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|edge| {
                        (
                            edge[0][1].as_str().unwrap().to_owned(),
                            edge[1][1].as_str().unwrap().to_owned(),
                        )
                    })
                    .collect();
                for (source, target) in actual.symmetric_difference(&expected) {
                    diffs.push(
                        Diff::new(
                            "links",
                            format!(
                                "link {source:?} -> {target:?}: model={}, Tine={}",
                                expected.contains(&(source.clone(), target.clone())),
                                actual.contains(&(source.clone(), target.clone()))
                            ),
                        )
                        .at_key(source)
                        .at_key(target),
                    );
                }
                diffs
            },
        );
        return;
    }
    let started = Instant::now();
    let mut section = Section::default();
    let mut report = BTreeMap::new();
    let droppers = Droppers::default();
    let frontend = Frontend::default();
    for spec in fixture_specs() {
        let diffs = run_graph(bridge, &spec, &droppers, &frontend);
        report.insert(
            spec.name.clone(),
            json!(diffs
                .iter()
                .map(|d| json!({"class": d.class, "detail": d.detail}))
                .collect::<Vec<_>>()),
        );
        section.fixtures.insert(
            spec.name.clone(),
            diffs.into_iter().map(|d| d.detail).collect(),
        );
    }
    let fixtures_done = started.elapsed();
    let n = std::env::var("CM_RANDOM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(RANDOM_DEFAULT);
    // Four workers (the machine is shared); results are folded in seed order,
    // so the classes and their smallest examples do not depend on scheduling.
    let specs: Vec<GraphSpec> = (0..n as u64).map(shared::random_graph).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: std::sync::Mutex<BTreeMap<usize, Vec<Diff>>> = Default::default();
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(spec) = specs.get(i) else { break };
                let diffs = run_graph(bridge, spec, &droppers, &frontend);
                results.lock().unwrap().insert(i, diffs);
            });
        }
    });
    let results = results.into_inner().unwrap();
    for (i, diffs) in results {
        let spec = &specs[i];
        // A graph the harness cannot read is a harness gap, never a class.
        if diffs.iter().any(Diff::is_harness) {
            section.harness_skipped.push(spec.name.clone());
            report.insert(
                spec.name.clone(),
                json!({"graph": spec.to_json(), "harness": diffs.iter().filter(|d| d.is_harness()).map(|d| &d.detail).collect::<Vec<_>>()}),
            );
            section.random_graphs += 1;
            continue;
        }
        section.add_random(spec, &diffs);
    }
    for h in droppers.into_inner().unwrap() {
        h.join().unwrap();
    }
    eprintln!(
        "cm conformance: fixtures {:?}, {n} random graphs {:?}, {} random classes",
        fixtures_done,
        started.elapsed() - fixtures_done,
        section.random.len()
    );
    if let Ok(dest) = std::env::var("CM_REPORT") {
        let full = json!({"fixtures": report, "store": section.to_json()});
        fs::write(dest, serde_json::to_string_pretty(&full).unwrap()).unwrap();
    }
    check_frontend_cases(
        "frontend",
        frontend.into_inner().unwrap(),
        "CM_REFRESH=1 CM_BRIDGE=<cm-bridge> cargo test -p tine-store --lib cm_conformance, \
         then CM_REFRESH=1 npx vitest run src/cmConformanceNavigate.test.ts",
    );
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "store",
        &section,
        "CM_REFRESH=1 CM_BRIDGE=<cm-bridge> cargo test -p tine-store --lib cm_conformance",
    );
}

/// Census Q2 ("what is a file's page name"), the computers the store crate
/// can run: every fixture through the store, no bridge needed (so this runs
/// in every gate). Compared with the answer (`shared::answered_name`, the
/// inventory: the `effective_page_name` family):
/// - OG's name (`ogNames`) and the Org file-link decode
///   (`shared::compare_file_names`);
/// - the launch walk (`page_identity::launch_listing_walk`), a declared
///   provisional answer: it names an ordinary page by its stem before the
///   load pass reads the title, so it differs exactly on titled pages;
/// - the test-only `Graph::list_sync_conflicts` copy of the conflict base
///   name.
/// The conflict listing of `tine-graph-features` (`conflicts.rs`) is run by
/// that crate's `cm_conformance_names.rs`. Differences: part `names` of
/// `expected-differences.json`; `CM_REFRESH=1` rewrites it and the
/// `frontend-names` cases `src/cmConformanceFileNames.test.ts` reads.
#[test]
fn file_name_answerers() {
    let mut section = Section::default();
    let mut cases = BTreeMap::new();
    let mut keep = Vec::new();
    let mut skipped = BTreeSet::new();
    for spec in fixture_specs() {
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
        if !spec.listed_apart(dir.path()) {
            section.skip_on_volume(&spec.name, "");
            skipped.insert(spec.name.clone());
            continue;
        }
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let view = store.whole_graph().expect("graph loads");
        let mut diffs = shared::compare_file_names(&view, &spec);
        let (listed, _, _, _) = crate::model::launch_listing_walk(&store.graph);
        for entry in &listed {
            let rel = store.graph.rel_path(&entry.path);
            let answer = shared::answered_name(&view, &rel);
            if answer.as_deref() != Some(entry.name.as_str()) {
                diffs.push(Diff::new(
                    "file name: the launch walk's provisional name differs from the answer",
                    format!("{rel}: launch walk {:?}, answer {answer:?}", entry.name),
                ));
            }
        }
        for copy in store.graph.list_sync_conflicts() {
            let Some(base) = &copy.base_path else {
                continue;
            };
            let answer = shared::answered_name(&view, base);
            if answer.as_deref() != Some(copy.base_name.as_str()) {
                diffs.push(Diff::new(
                    "file name: model.rs list_sync_conflicts (test copy) base name differs from the winner's name",
                    format!(
                        "{}: base name {:?}, {base} is named {answer:?}",
                        copy.path, copy.base_name
                    ),
                ));
            }
        }
        if let Some(case) = shared::names_case(&view, &spec) {
            cases.insert(spec.name.clone(), case);
        }
        diffs.sort();
        diffs.dedup();
        if !diffs.is_empty() {
            section.fixtures.insert(
                spec.name.clone(),
                diffs.into_iter().map(|d| d.detail).collect(),
            );
        }
        drop(view);
        keep.push(std::thread::spawn(move || drop((store, dir))));
    }
    for h in keep {
        h.join().unwrap();
    }
    check_frontend_cases_skipping(
        "frontend-names",
        cases,
        &skipped,
        "CM_REFRESH=1 cargo test -p tine-store --lib cm_conformance_tests::file_name_answerers, \
         then CM_REFRESH=1 npx vitest run src/cmConformanceFileNames.test.ts",
    );
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "names",
        &section,
        "CM_REFRESH=1 cargo test -p tine-store --lib cm_conformance_tests::file_name_answerers",
    );
}

/// Census Q1 ("the page-name key"): the answerer `tine_core::page_identity::
/// page_key` against every other fold that decides page identity, on a corpus
/// of tricky names (non-ASCII case, NFD, edge whitespace, boundary slashes,
/// final sigma, dotted I, ß, compatibility forms). No bridge needed.
///
/// The model has no executable key: SPEC §2.1 leaves `key` a given function
/// with law K1 (`key (spell k) = k`) and names "trim, boundary-slash,
/// lowercase, NFC" as the intended instance. So `page_key` is compared with
/// (a) that intended instance, transcribed from OG `page-name-sanity-lc`
/// (`graph_parser/util.cljs:162`: lower-case, `remove-boundary-slashes`,
/// NFC) plus Tine's decided trim, and (b) K1 with `spell = id` (is a key its
/// own key?). The second computers are compared as partitions: a pair of
/// names one fold calls the same page and the other does not.
///
/// Folds that are inline in production code are restated here in one line
/// each, with their sites (page search's exact match and completion dedup call
/// `page_key` since census Q1; tine-search `identity_fold` is gone). Store-level
/// probes run the real `[[` completion (`complete_page_names`) on a graph
/// whose references spell one page several ways.
///
/// Differences: part `keys` of `expected-differences.json`; the corpus with
/// each name's `page_key` goes to `frontend-keys/corpus.json` for
/// `src/cmConformanceKeys.test.ts`. `CM_REFRESH=1` rewrites both.
#[test]
fn key_answerers() {
    use unicode_normalization::UnicodeNormalization;
    const CORPUS: &[&str] = &[
        "Über",
        "über",
        "U\u{308}ber",
        "u\u{308}ber",
        "ÜBER",
        " Über ",
        "Über/",
        "/Über",
        "/Über/",
        "//Über//",
        "Foo",
        "foo",
        " Foo",
        "Foo ",
        "/Foo",
        "Foo/",
        "/Foo/",
        "//Foo//",
        "/ Foo /",
        "\u{3000}Foo",
        "Foo\u{a0}",
        "Foo/Bar",
        "foo/bar/",
        "ΟΔΟΣ",
        "οδος",
        "οδοσ",
        "İstanbul",
        "i\u{307}stanbul",
        "istanbul",
        "Straße",
        "STRASSE",
        "strasse",
        "ǅemal",
        "ǆemal",
        "Ｔｉｎｅ",
        "tine",
        "\u{212B}",
        "\u{C5}",
        "\u{E5}",
        "A\u{30A}",
    ];
    fn strip_boundary(s: &str) -> &str {
        let s = s.strip_prefix('/').unwrap_or(s);
        s.strip_suffix('/').unwrap_or(s)
    }
    // (name, fold, sites). Each is the production expression, restated. Only
    // declared different questions remain (census Q1): the LIKE pattern fold
    // (its differences are expected, a pattern is not a name) and the
    // `:inputs` literal (must stay difference-free). Page search and publish
    // call `page_key` and are no longer listed.
    type Fold = fn(&str) -> String;
    let folds: &[(&str, Fold, &str)] = &[
        (
            "like_fold",
            |s| s.to_lowercase().nfc().collect(),
            "query/eval.rs CmpOp::Like page-name pattern (declared pattern text)",
        ),
        (
            "inputs_then_key",
            |s| page_key(&s.to_lowercase()),
            "core query/advanced_patterns.rs :inputs [[page]] lowercase, then Attr::Name page_key",
        ),
    ];
    let keys: Vec<String> = CORPUS.iter().map(|s| page_key(s)).collect();
    let mut section = Section::default();
    let mut diffs = Vec::new();
    for (i, s) in CORPUS.iter().enumerate() {
        let intended: String = strip_boundary(&s.trim().to_lowercase()).nfc().collect();
        if keys[i] != intended {
            diffs.push(format!(
                "page_key vs SPEC §2.1 intended instance (OG page-name-sanity-lc + trim): {s:?} -> {:?}, intended {intended:?}",
                keys[i]
            ));
        }
        let again = page_key(&keys[i]);
        if again != keys[i] {
            diffs.push(format!(
                "page_key is not idempotent (K1 with spell = id): {s:?} -> {:?} -> {again:?}",
                keys[i]
            ));
        }
        // entry_identity::fold_leaf is a declared coarser filter: it must
        // never split what page_key joins.
        for (j, t) in CORPUS.iter().enumerate().skip(i + 1) {
            if keys[i] == keys[j]
                && crate::model::entry_identity::fold_leaf(std::ffi::OsStr::new(s))
                    != crate::model::entry_identity::fold_leaf(std::ffi::OsStr::new(t))
            {
                diffs.push(format!("fold_leaf splits a page_key class: {s:?} / {t:?}"));
            }
        }
    }
    for (name, fold, _sites) in folds {
        let folded: Vec<String> = CORPUS.iter().map(|s| fold(s)).collect();
        for i in 0..CORPUS.len() {
            for j in i + 1..CORPUS.len() {
                let by_key = keys[i] == keys[j];
                let by_fold = folded[i] == folded[j];
                if by_key != by_fold {
                    diffs.push(format!(
                        "{name}: {:?} / {:?}: page_key {}, {name} {}",
                        CORPUS[i],
                        CORPUS[j],
                        if by_key { "same" } else { "different" },
                        if by_fold { "same" } else { "different" },
                    ));
                }
            }
        }
    }
    section.fixtures.insert("corpus".into(), diffs);

    // Probe: `[[` completion over a graph whose references spell existing
    // pages in other forms. One row per page identity is the contract
    // (`query_plan/pages.rs`: "a physical page wins over an alias with the
    // same identity"); a second row for the same page_key is a phantom.
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in [
        ("pages/Foo.md", "- foo\n"),
        ("pages/Über.md", "- über\n"),
        (
            "pages/r.md",
            "- [[Foo/]] [[/Foo]] [[ Foo ]] [[U\u{308}ber]] [[Über/]]\n",
        ),
    ] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    let mut probe = Vec::new();
    for query in ["foo", "Foo/", "/Foo", "über", "U\u{308}ber"] {
        let rows = view.complete_page_names(query, 20, None, false, None);
        let mut by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (entry, class) in &rows {
            by_key
                .entry(page_key(&entry.name))
                .or_default()
                .push(format!("{:?} {class:?}", entry.name));
        }
        for (key, names) in by_key {
            if names.len() > 1 {
                probe.push(format!(
                    "completion {query:?}: {} rows for one page (key {key:?}): {names:?}",
                    names.len()
                ));
            }
        }
        let wanted = page_key(query);
        if let Some((entry, class)) = rows.iter().find(|(e, _)| page_key(&e.name) == wanted) {
            if *class != tine_core::query_plan::ObjectiveMatchClass::Exact {
                probe.push(format!(
                    "completion {query:?}: the page with the same page_key ({:?}) is ranked {class:?}, not Exact",
                    entry.name
                ));
            }
        }
    }
    section.fixtures.insert("completion".into(), probe);
    drop(view);
    drop(store);

    let corpus = json!(CORPUS
        .iter()
        .zip(&keys)
        .map(|(s, k)| json!({"name": s, "key": k}))
        .collect::<Vec<_>>());
    check_frontend_cases(
        "frontend-keys",
        BTreeMap::from([("corpus".to_owned(), corpus)]),
        "CM_REFRESH=1 cargo test -p tine-store --lib cm_conformance_tests::key_answerers, \
         then CM_REFRESH=1 npx vitest run src/cmConformanceKeys.test.ts",
    );
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "keys",
        &section,
        "CM_REFRESH=1 cargo test -p tine-store --lib cm_conformance_tests::key_answerers",
    );
}

/// Census Q7 (the alias rule), the page header's alias list: the frontend's
/// `render/block.ts aliasNamesOf` re-derives a page's declared aliases from
/// the properties it reads (Page.tsx: `pageHeaderProperties`, every pre-block
/// and properties-only first-root line; RightSidebar.tsx:
/// `readPageProperties`, first line per key). This writes, per header shape,
/// the `PageDto` the frontend loads (pre-block and first root) and the store's
/// answer (`document_alias_spellings`), for `src/cmConformanceKeys.test.ts`;
/// it also asserts the store resolves each declared spelling to the page.
/// `CM_REFRESH=1` rewrites `frontend-aliases/headers.json`.
#[test]
fn alias_header_cases() {
    const HEADERS: &[(&str, &str)] = &[
        ("pages/h1.md", "alias:: A1, B1\n\n- body\n"),
        ("pages/h2.md", "aliases:: A2\n\n- body\n"),
        ("pages/h3.md", "alias:: A3\nalias:: B3\n\n- body\n"),
        ("pages/h4.md", "alias:: \"A4, B4\"\n\n- body\n"),
        ("pages/h5.md", "- alias:: A5\n- body\n"),
        ("pages/h6.md", "alias:: A6\n\n- alias:: B6\n- body\n"),
        ("pages/h7.md", "Alias:: A7\n\n- body\n"),
        ("pages/h8.md", "alias:: [[A8]], #B8\n\n- body\n"),
        ("pages/h9.md", "alias:: A9\naliases:: B9\n\n- body\n"),
        ("pages/o1.org", "#+ALIAS: OA1\n\n* body\n"),
        (
            "pages/o2.org",
            ":PROPERTIES:\n:alias: OA2\n:END:\n\n* body\n",
        ),
        ("pages/o3.org", "#+ALIASES: OA3\n\n* body\n"),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in HEADERS {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    let mut cases = Vec::new();
    for (rel, text) in HEADERS {
        let read = store.page(&crate::PageId::from(*rel)).unwrap();
        let declared = document_alias_spellings(&shared::parse_file(rel, text));
        let mut resolves = BTreeMap::new();
        for alias in &declared {
            let to = match view.resolve(alias, false) {
                crate::Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
                crate::Resolved::Alias { owners } => {
                    owners.first().map(|id| id.as_str().to_owned())
                }
                crate::Resolved::Absent { .. } => None,
            };
            resolves.insert(alias.clone(), to);
        }
        assert!(
            resolves.values().all(|to| to.as_deref() == Some(*rel)),
            "{rel}: the store declares {declared:?} but resolves {resolves:?}"
        );
        cases.push(json!({
            "path": rel,
            "format": if rel.ends_with(".org") { "org" } else { "md" },
            "preBlock": read.doc.pre_block,
            "firstRoot": read.doc.blocks.first().map(|b| b.raw.clone()),
            "declared": declared,
        }));
    }
    drop(view);
    drop(store);
    check_frontend_cases(
        "frontend-aliases",
        BTreeMap::from([("headers".to_owned(), json!(cases))]),
        "CM_REFRESH=1 cargo test -p tine-store --lib cm_conformance_tests::alias_header_cases, \
         then CM_REFRESH=1 npx vitest run src/cmConformanceKeys.test.ts",
    );
}
