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
//! Tine's: `page_key`, with a date-shaped spelling folded to its day.
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
    pub(crate) use crate::{Cancel, InventoryScope, IrAnswer, IrRequest, Resolved, WholeGraph};
}

#[path = "cm_conformance_shared_tests.rs"]
mod shared;

#[path = "cm_conformance_extraction_tests.rs"]
mod extraction;

use shared::{Ctx, Diff, GraphSpec, Section};

const RANDOM_DEFAULT: usize = 300;
const WORKERS: usize = 4;

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
    // The table keys nodes by `page_key` of the page name; the model by the
    // date-folding key. Map through every spelling in play.
    let mut model_key: BTreeMap<String, String> = BTreeMap::new();
    for (s, k) in input["keys"].as_object().unwrap() {
        model_key
            .entry(page_key(s))
            .or_insert_with(|| k.as_str().unwrap().to_owned());
    }
    let mut node_key: BTreeMap<u32, String> = BTreeMap::new();
    for (pk, mk) in &model_key {
        if let Some(n) = table.node(pk) {
            if (n as usize) < table.physical() {
                node_key.entry(n).or_insert_with(|| mk.clone());
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
    let config = snapshot.config();
    let fmt = config.journal_format();
    let name_fmt = config.file_name_format;
    let entries = snapshot.page_list_arc();
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
            .min_by(|a, b| compare_page_claimants(a, b, &fmt, name_fmt))
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
        let Some(owners) = snapshot.alias_owner_paths(&key).filter(|o| !o.is_empty()) else {
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
    let input = shared::model_input(&view, spec, document_alias_spellings, &[]);
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
    let dir = shared::fixture_dir(env!("CARGO_MANIFEST_DIR")).join(subdir);
    let refresh = std::env::var_os("CM_REFRESH").is_some();
    if refresh {
        fs::create_dir_all(&dir).unwrap();
        for e in fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            if p.extension().is_some_and(|x| x == "json") && !cases.contains_key(&stem) {
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
                .filter(|n| !cases.contains_key(n))
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
        eprintln!("CM_BRIDGE unset: conceptual model conformance skipped");
        return;
    };
    let bridge = Path::new(&bridge);
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
    for spec in fixture_specs() {
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
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
    check_frontend_cases(
        "frontend-names",
        cases,
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
