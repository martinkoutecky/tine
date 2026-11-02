//! Conceptual model conformance, rename (rule 6, code-map Q11). Each rename
//! fixture of `crates/tine-store/tests/fixtures/cm-conformance/` (those with
//! `renames`) and a few hundred seeded random graphs with a random rename run
//! through Tine's real rename (`rename_or_merge_page` under a page host
//! started for the test, on a temporary copy) and through the Lean model's
//! bridge (`CM_BRIDGE`). Compared: refusal against the model's `allowed`
//! (and which refusal: the base guard or a journal risk), the moved files and
//! their new names, the after-rename occurrences (`occAfter`, task markers and
//! priorities included), alias members (`aliasesAfter`), stored `{{query}}`
//! page arguments through the model's spelling map (rule 6.2,
//! D-20261031-CMQ), and the after-graph's meaning through the bridge again.
//!
//! A rename onto a name another page claims refuses in Tine unless the user
//! confirms the merge (`merge_into`); the harness confirms it, as the app's
//! dialog would, when the name has one claimant, and says so in the class.
//!
//! Favorites follow rename only in the frontend (`src/favorites.ts`, Q12):
//! the model's `favorites` answer is not compared here (a harness gap).
//!
//! Shares `cm_conformance_shared_tests.rs` with the store harness (which also holds
//! the test-side alias extractor copy to the original). Step 1 changes no
//! production code. Without `CM_BRIDGE` the test skips; `CM_REFRESH=1`
//! rewrites the `rename` part of the expected differences; `CM_RANDOM_RENAMES`
//! changes the number of random graphs; `CM_REPORT_RENAME` names a report file.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tine_graph_features::pages::{rename_or_merge_page, RenameOutcome};
use tine_store::{Resolved, Store};

use tine_store as store_crate;

#[path = "../../tine-store/src/cm_conformance_shared_tests.rs"]
mod shared;

use shared::{Ctx, Diff, GraphSpec, Section};

const RANDOM_DEFAULT: usize = 200;
const WORKERS: usize = 4;

type Droppers = std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>;

fn later_drop<T: Send + 'static>(droppers: &Droppers, value: T) {
    droppers
        .lock()
        .unwrap()
        .push(std::thread::spawn(move || drop(value)));
}

/// The graph's page and journal files after the rename.
fn read_graph(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for dir in ["pages", "journals"] {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in entries {
            let p = e.unwrap().path();
            if p.is_file() && p.extension().is_some_and(|x| x == "md") {
                let rel = format!("{dir}/{}", p.file_name().unwrap().to_string_lossy());
                out.insert(rel, fs::read_to_string(&p).unwrap());
            }
        }
    }
    out
}

/// A bridge query with its page arguments pushed through the model's map.
fn map_query(q: &Value, map: &Value) -> Value {
    match q {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| {
                    let v = match (k.as_str(), v) {
                        ("refs" | "name", Value::String(s)) => {
                            json!(map[s].as_str().unwrap_or(s))
                        }
                        _ => map_query(v, map),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| map_query(x, map)).collect()),
        other => other.clone(),
    }
}

enum TineRename {
    Performed {
        outcome: RenameOutcome,
        confirmed: bool,
    },
    Refused(String),
}

fn run_tine(store: &Arc<Store>, old: &str, new: &str, droppers: &Droppers) -> TineRename {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    let mut result = rename_or_merge_page(store, &host, old, new, None, None, &[]);
    let mut confirmed = false;
    if let Err(e) = &result {
        if e.kind() == std::io::ErrorKind::AlreadyExists
            && e.to_string().contains("already exists elsewhere")
        {
            // The app asks the user to confirm a merge into the sole claimant.
            let claimant = match store.whole_graph().unwrap().resolve(new, false) {
                Resolved::Existing { id, others } if others.is_empty() => Some(id.to_string()),
                Resolved::Alias { owners } if owners.len() == 1 => Some(owners[0].to_string()),
                _ => None,
            };
            if let Some(into) = claimant {
                confirmed = true;
                result = rename_or_merge_page(store, &host, old, new, None, Some(&into), &[]);
            }
        }
    }
    later_drop(droppers, (host, app_data));
    match result {
        Ok(report) => TineRename::Performed {
            outcome: report.outcome,
            confirmed,
        },
        Err(e) => TineRename::Refused(format!(
            "{:?}: {e}{}",
            e.kind(),
            if confirmed {
                " (after confirming the merge)"
            } else {
                ""
            }
        )),
    }
}

/// (kind, key) multisets, sorted, per final path.
type Occs = BTreeMap<String, Vec<(String, String)>>;

fn diff_multisets(
    what: &str,
    model: &Occs,
    tine: &Occs,
    diffs: &mut Vec<Diff>,
    label: &dyn Fn(&(String, String)) -> String,
) {
    let paths: BTreeSet<&String> = model.keys().chain(tine.keys()).collect();
    let empty = Vec::new();
    for path in paths {
        let mut m = model.get(path).unwrap_or(&empty).clone();
        let mut t = tine.get(path).unwrap_or(&empty).clone();
        m.sort();
        t.sort();
        if m == t {
            continue;
        }
        let mut only_m = m.clone();
        for x in &t {
            if let Some(i) = only_m.iter().position(|y| y == x) {
                only_m.remove(i);
            }
        }
        let mut only_t = t.clone();
        for x in &m {
            if let Some(i) = only_t.iter().position(|y| y == x) {
                only_t.remove(i);
            }
        }
        for x in &only_m {
            diffs.push(Diff::new(
                format!("rename {what}: model-only {}", label(x)),
                format!(
                    "{what} of {path}: model has {x:?}, Tine does not (model {m:?}, Tine {t:?})"
                ),
            ));
        }
        for x in &only_t {
            diffs.push(Diff::new(
                format!("rename {what}: Tine-only {}", label(x)),
                format!(
                    "{what} of {path}: Tine has {x:?}, model does not (model {m:?}, Tine {t:?})"
                ),
            ));
        }
    }
}

/// One graph and one rename through Tine and the model; the differences.
fn run_rename(
    store_classes: &BTreeSet<String>,
    bridge: &Path,
    spec: &GraphSpec,
    old: &str,
    new: &str,
    coarse_random: bool,
    droppers: &Droppers,
) -> Vec<Diff> {
    let dir = tempfile::tempdir().unwrap();
    spec.write(dir.path());
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let view = store.whole_graph().expect("graph loads");
    let map_strings = spec.stored_query_args();
    let mut rspec = spec.clone();
    rspec.renames = vec![(old.to_owned(), new.to_owned())];
    let input = shared::model_input(&view, &rspec, shared::alias_spellings, &map_strings);
    let mut diffs = Vec::new();
    let model = match shared::run_bridge(bridge, &input) {
        Ok(m) => m,
        Err(e) => {
            diffs.push(Diff::new("harness: bridge failed", e));
            drop(view);
            later_drop(droppers, (store, dir));
            return diffs;
        }
    };
    diffs.extend(shared::harness_reports(&model));
    let ctx = Ctx::new(&view, spec.journal_format(), &input, &model);
    let duplicate = shared::has_duplicate_claim(&ctx);
    let before: BTreeSet<String> = shared::compare_meaning(&ctx, &spec.queries)
        .into_iter()
        .map(|d| d.class)
        .collect();
    let journal = spec.journal_format();
    let key = |s: &str| shared::fold(&journal, s);
    // The model's final path of each before file: its own, or for a moved
    // file the after file named by its new name.
    let r = &model["renames"][0];
    let moved: BTreeMap<String, String> = r["movedFiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_owned(),
                f["newName"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let file_names: BTreeMap<String, String> = ctx
        .files
        .iter()
        .map(|(p, f)| (p.clone(), f.name.clone()))
        .collect();
    drop(ctx);
    drop(view);

    let allowed = r["allowed"].as_bool().unwrap();
    let base = r["baseAllowed"].as_bool().unwrap();
    let risks: Vec<String> = r["journalRisks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.to_string())
        .collect();
    let model_says = if allowed {
        "allows".to_owned()
    } else {
        let mut why = Vec::new();
        if !base {
            why.push("base guard");
        }
        if !risks.is_empty() {
            why.push("journal risk");
        }
        format!("refuses ({})", why.join(" + "))
    };
    let tine = run_tine(&store, old, new, droppers);
    let rename = format!("rename {old:?} -> {new:?}");
    let performed: Option<(RenameOutcome, bool)> = match (&tine, allowed) {
        (TineRename::Refused(why), true) => {
            diffs.push(Diff::new(
                format!(
                    "rename: model allows, Tine refuses ({})",
                    why.split(':').next().unwrap_or(why)
                ),
                format!("{rename}: model allows, Tine refuses: {why}"),
            ));
            None
        }
        (TineRename::Refused(_), false) => None,
        // `Unchanged` writes nothing: in effect the same as a refusal (the
        // user sees no error, but no file changes).
        (
            TineRename::Performed {
                outcome: RenameOutcome::Unchanged,
                ..
            },
            false,
        ) => None,
        (TineRename::Performed { outcome, confirmed }, false) => {
            diffs.push(Diff::new(
                format!(
                    "rename: model {model_says}, Tine performs{}",
                    if *confirmed { " a confirmed merge" } else { "" }
                ),
                format!(
                    "{rename}: model {model_says} (journal risks {risks:?}), Tine performs ({outcome:?})"
                ),
            ));
            None
        }
        (
            TineRename::Performed {
                confirmed: true,
                outcome,
            },
            true,
        ) => {
            // The model allows a plain rename here (the new name is no real
            // page); Tine refuses it and offers only a merge into the new
            // name's claimant (an alias owner). The after graphs differ by
            // construction, so they are not compared further.
            diffs.push(Diff::new(
                "rename: model allows a plain rename, Tine only merges into the new name's claimant",
                format!("{rename}: model allows; Tine refuses AlreadyExists, merges after confirmation ({outcome:?})"),
            ));
            None
        }
        (TineRename::Performed { outcome, confirmed }, true) => Some((*outcome, *confirmed)),
    };
    let Some((outcome, confirmed)) = performed else {
        later_drop(droppers, (store, dir));
        return finish(diffs, duplicate, coarse_random);
    };
    let merge_tag = if confirmed { " (confirmed merge)" } else { "" };

    // The after graph, through a fresh store.
    let after_files = read_graph(dir.path());
    later_drop(droppers, store);
    let store2 = Store::open(dir.path(), Default::default()).unwrap().0;
    let view2 = store2.whole_graph().expect("after graph loads");
    let map = &r["map"];
    let after_spec = GraphSpec {
        name: format!("{} after {rename}", spec.name),
        files: after_files.clone(),
        favorites_page: spec.favorites_page.clone(),
        journal_title_format: spec.journal_title_format.clone(),
        journal_file_format: spec.journal_file_format.clone(),
        queries: spec
            .queries
            .iter()
            .map(|q| {
                let mq = map_query(&q.q, map);
                let (tine, dialect) = shared::tine_query_text(&q.anchor, &mq)
                    .unwrap_or((q.tine.clone(), q.dialect.clone()));
                shared::QuerySpec {
                    anchor: q.anchor.clone(),
                    q: mq,
                    tine,
                    dialect,
                }
            })
            .collect(),
        ..Default::default()
    };
    let after_input = shared::model_input(&view2, &after_spec, shared::alias_spellings, &[]);
    let after_model = match shared::run_bridge(bridge, &after_input) {
        Ok(m) => m,
        Err(e) => {
            diffs.push(Diff::new("harness: bridge failed (after graph)", e));
            drop(view2);
            later_drop(droppers, (store2, dir));
            return finish(diffs, duplicate, coarse_random);
        }
    };
    diffs.extend(shared::harness_reports(&after_model));
    let after_ctx = Ctx::new(
        &view2,
        after_spec.journal_format(),
        &after_input,
        &after_model,
    );
    // After paths by name key.
    let mut by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (p, f) in &after_ctx.files {
        by_key.entry(key(&f.name)).or_default().push(p.clone());
    }
    let mut final_path: BTreeMap<String, String> = BTreeMap::new();
    for path in file_names.keys() {
        match moved.get(path) {
            Some(new_name) => match by_key.get(&key(new_name)).map(Vec::as_slice) {
                Some([p]) => {
                    final_path.insert(path.clone(), p.clone());
                }
                found => diffs.push(Diff::new(
                    format!("rename moves: moved file not found once under its new name{merge_tag}"),
                    format!("{rename}: model moves {path} to {new_name:?}; after files named so: {found:?}"),
                )),
            },
            None => {
                if after_files.contains_key(path) {
                    final_path.insert(path.clone(), path.clone());
                } else {
                    diffs.push(Diff::new(
                        format!("rename moves: Tine moves or removes a file the model keeps{merge_tag}"),
                        format!("{rename}: {path} (named {:?}) is gone after the rename", file_names[path]),
                    ));
                }
            }
        }
    }
    let finals: BTreeSet<&String> = final_path.values().collect();
    for p in after_files.keys() {
        if !finals.contains(p) {
            diffs.push(Diff::new(
                format!("rename moves: Tine creates a file the model does not{merge_tag}"),
                format!("{rename}: after file {p} is no before file's model destination"),
            ));
        }
    }
    // Occurrences, per final path, as (kind, key) multisets. Blocks of files
    // whose destination is unknown are left out (reported above).
    let mut model_occ: Occs = BTreeMap::new();
    for b in r["occAfter"].as_array().unwrap() {
        let path = b["block"][0].as_str().unwrap();
        let Some(fp) = final_path.get(path) else {
            continue;
        };
        for o in b["occ"].as_array().unwrap() {
            model_occ.entry(fp.clone()).or_default().push((
                o[0].as_str().unwrap().to_owned(),
                key(o[1].as_str().unwrap()),
            ));
        }
    }
    let mut tine_occ: Occs = BTreeMap::new();
    for (p, f) in &after_ctx.files {
        if !finals.contains(p) {
            continue;
        }
        let e = tine_occ.entry(p.clone()).or_default();
        for (_, occ) in &f.blocks {
            e.extend(occ.iter().map(|(k, s)| (k.clone(), key(s))));
        }
    }
    let old_keys: BTreeSet<String> = shared::closure(old).iter().map(|s| key(s)).collect();
    let new_keys = [key(new), tine_core::refs::page_key(new)];
    let label = |x: &(String, String)| {
        let under = |k: &String| x.1 == *k || x.1.starts_with(&format!("{k}/"));
        let rel = if old_keys.contains(&x.1) || x.1.starts_with(&format!("{}/", key(old))) {
            "old name"
        } else if new_keys.iter().any(under) {
            "new name"
        } else {
            "other name"
        };
        format!("{} of {rel}{merge_tag}", x.0)
    };
    diff_multisets("occurrences", &model_occ, &tine_occ, &mut diffs, &label);
    // Alias members.
    let mut model_alias: Occs = BTreeMap::new();
    for f in r["aliasesAfter"].as_array().unwrap() {
        let Some(fp) = final_path.get(f["path"].as_str().unwrap()) else {
            continue;
        };
        let e = model_alias.entry(fp.clone()).or_default();
        for a in f["aliases"].as_array().unwrap() {
            e.push(("alias".to_owned(), key(a.as_str().unwrap())));
        }
    }
    let mut tine_alias: Occs = BTreeMap::new();
    for (p, f) in &after_ctx.files {
        if finals.contains(p) {
            let e = tine_alias.entry(p.clone()).or_default();
            e.extend(f.aliases.iter().map(|a| ("alias".to_owned(), key(a))));
        }
    }
    // A merge unites the survivor's aliases with the source's: compare sets.
    for m in [&mut model_alias, &mut tine_alias] {
        for v in m.values_mut() {
            v.sort();
            v.dedup();
        }
    }
    diff_multisets("aliases", &model_alias, &tine_alias, &mut diffs, &label);
    // Stored query page arguments through the model's map.
    let mut model_args: Occs = BTreeMap::new();
    for (path, text) in &spec.files {
        let Some(fp) = final_path.get(path) else {
            continue;
        };
        for m in shared::query_macros(text) {
            for a in shared::query_page_args(&m) {
                let mapped = map[&a].as_str().map(str::to_owned).unwrap_or(a);
                model_args
                    .entry(fp.clone())
                    .or_default()
                    .push(("query arg".to_owned(), key(&mapped)));
            }
        }
    }
    let mut tine_args: Occs = BTreeMap::new();
    for (path, text) in &after_files {
        if !finals.contains(path) {
            continue;
        }
        for m in shared::query_macros(text) {
            for a in shared::query_page_args(&m) {
                tine_args
                    .entry(path.clone())
                    .or_default()
                    .push(("query arg".to_owned(), key(&a)));
            }
        }
    }
    diff_multisets("query args", &model_args, &tine_args, &mut diffs, &label);
    // The after graph's meaning: Tine against the model again, reporting the
    // classes neither the before graph nor the store harness's random graphs
    // already show (those are meaning differences, not rename ones; LinkGraph
    // is crate private to the store, so the link table is not compared here).
    for d in shared::compare_meaning(&after_ctx, &after_spec.queries) {
        if d.is_harness() {
            diffs.push(d);
        } else if !before.contains(&d.class)
            && !store_classes.contains(&d.class)
            && !store_classes.contains(&shared::coarse(vec![d.clone()])[0].class)
        {
            diffs.push(Diff::new(
                format!("after rename: {}", d.class),
                format!("after {rename} ({outcome:?}): {}", d.detail),
            ));
        }
    }
    drop(after_ctx);
    drop(view2);
    later_drop(droppers, (store2, dir));
    finish(diffs, duplicate, coarse_random)
}

fn finish(mut diffs: Vec<Diff>, duplicate: bool, coarse_random: bool) -> Vec<Diff> {
    if coarse_random && duplicate {
        diffs = shared::coarse(diffs);
    }
    diffs.sort();
    diffs.dedup();
    diffs
}

fn rename_fixtures() -> Vec<GraphSpec> {
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
        .filter_map(|p| {
            let v: Value = serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap();
            v.get("renames")?;
            Some(GraphSpec::from_fixture(
                &p.file_stem().unwrap().to_string_lossy(),
                &v,
            ))
        })
        .collect()
}

#[test]
fn conceptual_model_rename_conformance() {
    let _integration_guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_BRIDGE") else {
        eprintln!("CM_BRIDGE unset: conceptual model rename conformance skipped");
        return;
    };
    let bridge = Path::new(&bridge);
    let started = Instant::now();
    let droppers = Droppers::default();
    // Meaning classes the store harness already records (its random part,
    // with and without the duplicate-claim prefix): an after graph showing
    // one of them is not a rename difference.
    let expected: Value = fs::read_to_string(
        shared::fixture_dir(env!("CARGO_MANIFEST_DIR")).join("expected-differences.json"),
    )
    .ok()
    .and_then(|t| serde_json::from_str(&t).ok())
    .unwrap_or(Value::Null);
    let store_classes: BTreeSet<String> = expected["store"]["random"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let mut section = Section::default();
    let mut report = BTreeMap::new();
    for spec in rename_fixtures() {
        for (old, new) in &spec.renames {
            let diffs = run_rename(&store_classes, bridge, &spec, old, new, false, &droppers);
            let name = format!("{}: {old} -> {new}", spec.name);
            report.insert(
                name.clone(),
                json!(diffs
                    .iter()
                    .map(|d| json!({"class": d.class, "detail": d.detail}))
                    .collect::<Vec<_>>()),
            );
            section
                .fixtures
                .insert(name, diffs.into_iter().map(|d| d.detail).collect());
        }
    }
    let fixtures_done = started.elapsed();
    let n = std::env::var("CM_RANDOM_RENAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(RANDOM_DEFAULT);
    let specs: Vec<GraphSpec> = (0..n as u64)
        .map(|seed| {
            let mut spec = shared::random_graph(seed);
            spec.renames = vec![shared::random_rename(seed, &spec)];
            spec
        })
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: std::sync::Mutex<BTreeMap<usize, Vec<Diff>>> = Default::default();
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(spec) = specs.get(i) else { break };
                let (old, new) = &spec.renames[0];
                let diffs = run_rename(&store_classes, bridge, spec, old, new, true, &droppers);
                results.lock().unwrap().insert(i, diffs);
            });
        }
    });
    for (i, diffs) in results.into_inner().unwrap() {
        let spec = &specs[i];
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
        "cm rename conformance: fixtures {:?}, {n} random renames {:?}, {} random classes",
        fixtures_done,
        started.elapsed() - fixtures_done,
        section.random.len()
    );
    if let Ok(dest) = std::env::var("CM_REPORT_RENAME") {
        let full = json!({"fixtures": report, "rename": section.to_json()});
        fs::write(dest, serde_json::to_string_pretty(&full).unwrap()).unwrap();
    }
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "rename",
        &section,
        "CM_REFRESH=1 CM_BRIDGE=<cm-bridge> cargo test -p tine-graph-features --test integration cm_conformance",
    );
}
