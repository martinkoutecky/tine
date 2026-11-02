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
use tine_core::doc;
use tine_core::refs::page_key;

use crate::query::document_alias_spellings;
use crate::Store;

/// The store names the shared code uses (`super::store_crate` there).
mod store_crate {
    pub(crate) use crate::{Cancel, InventoryScope, IrAnswer, IrRequest, Resolved, WholeGraph};
}

#[path = "cm_conformance_shared_tests.rs"]
mod shared;

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
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let view = store.whole_graph().expect("graph loads");
    let mut diffs = Vec::new();
    // The rename harness uses the test-side copy of the alias extractor:
    // hold it to the original here, on every file of every graph.
    for (rel, text) in &spec.files {
        let parsed = doc::parse(text);
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
            droppers
                .lock()
                .unwrap()
                .push(std::thread::spawn(move || drop((store, dir))));
            return diffs;
        }
    };
    let ctx = Ctx::new(&view, spec.journal_format(), &input, &model);
    diffs.extend(shared::compare_meaning(&ctx, &spec.queries));
    if !spec.name.starts_with("random-") {
        diffs.extend(shared::compare_reference_days(&ctx));
        frontend.lock().unwrap().insert(
            spec.name.clone(),
            shared::frontend_case(&ctx, spec.journal_title_format.as_deref()),
        );
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
    droppers
        .lock()
        .unwrap()
        .push(std::thread::spawn(move || drop((store, dir))));
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

/// The frontend check's inputs (`tests/fixtures/cm-conformance/frontend/`)
/// are written by this harness and committed, so `npm test` runs the
/// frontend's real link decision without the bridge. Stale or missing cases
/// fail here; `CM_REFRESH=1` rewrites them.
fn check_frontend_cases(cases: BTreeMap<String, Value>) {
    let dir = shared::fixture_dir(env!("CARGO_MANIFEST_DIR")).join("frontend");
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
        "conceptual model conformance: frontend cases are stale: {stale:?}; refresh with \
         CM_REFRESH=1 CM_BRIDGE=<cm-bridge> cargo test -p tine-store --lib cm_conformance, \
         then CM_REFRESH=1 npx vitest run src/cmConformanceNavigate.test.ts"
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
    check_frontend_cases(frontend.into_inner().unwrap());
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "store",
        &section,
        "CM_REFRESH=1 CM_BRIDGE=<cm-bridge> cargo test -p tine-store --lib cm_conformance",
    );
}
