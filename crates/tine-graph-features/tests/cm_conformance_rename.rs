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
//! Round-9 bridge (`bridge-round` file in the fixture directory; the gate
//! reads it): each rename is also asked as the model's decided merge
//! (M-Q5..7, CM-OWN-CHILD). A confirmed merge is compared with the model's
//! merge (refusal, survivor, moved children, occurrences, united aliases,
//! stored query arguments through `Rulings9.queryMap`), a plain rename with
//! the model's rename (whose refusal now includes CM-OWN-CHILD).
//!
//! Round-10 bridge: a reference-only source (`refSource`) is renamed by
//! `rename10` and merged by `refMerge`, whose `source` is null (nothing to
//! fold; the after graph is still compared). The model's namespace test is
//! Tine's (`urls`, `shared::model_input`), so `./x` is no child of `.`.
//!
//! Favorites follow rename only in the frontend (`src/favorites.ts`, Q12):
//! the model's `favorites` answer is not compared here (a harness gap).
//!
//! Round 11: compare the explicit ordered `offers`, concrete target and executed
//! `answer` projections (including cancellation's original bytes). OwnPage may
//! fill an existing empty alias file. Historical rename10 success no longer
//! implies a menu option. Source kind comes directly from the bridge.
//! Round 12 adds aliasOnly sources and their full target matrix, including
//! source ties, empty targets, same-key respelling and separate query maps.
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
use tine_graph_features::pages::{
    rename_offer, rename_page_as, RenameChoice, RenameOffer, RenameOutcome,
};
use tine_store::{Resolved, Store};

use tine_store as store_crate;

#[path = "../../tine-store/src/cm_conformance_shared_tests.rs"]
mod shared;

#[path = "cm_conformance_r13_rename.rs"]
mod r13_rename;

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
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else if path
                .extension()
                .is_some_and(|x| x == "md" || x == "markdown" || x == "org")
            {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                out.insert(rel, fs::read_to_string(&path).unwrap());
            }
        }
    }
    for dir in ["pages", "journals"] {
        visit(root, &root.join(dir), &mut out);
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
        /// The page id the confirmed merge went into (empty for a rename).
        into: String,
    },
    Refused(String),
}

/// Tine's rename as the app runs it: the dialog's question
/// (`rename_offer`, the planner's own answer), then the answer the user
/// gives. A merge offer is confirmed. An alias-only name (M-Q5) offers both
/// a merge into its owner and "make it its own page": `own_page` picks the
/// second (the caller runs both); the returned flag says the offer had both.
fn run_tine(
    store: &Arc<Store>,
    old: &str,
    new: &str,
    own_page: bool,
    droppers: &Droppers,
) -> (TineRename, bool) {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    let mut both = false;
    let (choice, confirmed, into) = match rename_offer(store, old, new, None) {
        Ok(RenameOffer::Rename) => (Some(RenameChoice::Rename), false, String::new()),
        Ok(RenameOffer::Merge { into, .. }) => (None, true, into),
        Ok(RenameOffer::AliasOf {
            owner,
            merge,
            own_page: can_own,
            ..
        }) => {
            both = merge && can_own;
            if (own_page || !merge) && can_own {
                (Some(RenameChoice::OwnPage), false, String::new())
            } else {
                (None, true, owner)
            }
        }
        Ok(RenameOffer::Refused { reason, message }) => {
            later_drop(droppers, (host, app_data));
            let kind = format!("{reason:?}");
            let kind = kind.split([' ', '{']).next().unwrap_or("").to_owned();
            return (TineRename::Refused(format!("{kind}: {message}")), false);
        }
        Err(e) => {
            later_drop(droppers, (host, app_data));
            return (TineRename::Refused(format!("{:?}: {e}", e.kind())), false);
        }
    };
    let choice = choice.unwrap_or(RenameChoice::MergeInto(&into));
    let result = rename_page_as(store, &host, old, new, None, choice, &[]);
    later_drop(droppers, (host, app_data));
    let tine = match result {
        Ok(report) => TineRename::Performed {
            outcome: report.outcome,
            confirmed,
            into: into.clone(),
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
    };
    (tine, both)
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
/// Round 9 (M-Q5): when Tine's dialog offered both a merge and "make it its
/// own page", the merge is run here and the own-page answer by the caller
/// ([`run_both`]).
fn run_rename(
    store_classes: &BTreeSet<String>,
    bridge: &Path,
    spec: &GraphSpec,
    old: &str,
    new: &str,
    coarse_random: bool,
    own_page: bool,
    droppers: &Droppers,
) -> (Vec<Diff>, bool) {
    let dir = tempfile::tempdir().unwrap();
    spec.write(dir.path());
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let view = store.whole_graph().expect("graph loads");
    let map_strings = spec.stored_query_args();
    let mut rspec = spec.clone();
    rspec.renames = vec![(old.to_owned(), new.to_owned())];
    let input = shared::model_input(&rspec, shared::alias_spellings, &map_strings);
    let mut diffs = Vec::new();
    let model = match shared::run_bridge(bridge, &input) {
        Ok(m) => m,
        Err(e) => {
            diffs.push(Diff::new("harness: bridge failed", e));
            drop(view);
            later_drop(droppers, (store, dir));
            return (diffs, false);
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
    // The model's answers: the rename, and (round-9 bridge) the same pair as
    // a merge (M-Q5..7). Older bridges have no `merges`.
    let operation = &model["renames"][0];
    match spec.name.as_str() {
        "r11-empty-alias-target" => assert_eq!(operation["sourceKind"], "realPage"),
        "r11-fileless-empty-alias-target" => assert_eq!(operation["sourceKind"], "fileless"),
        "r11-empty-alias-source" => assert_eq!(operation["sourceKind"], "emptyAliasFile"),
        _ => {}
    }
    if spec.name.starts_with("r12-alias-source-") {
        assert_eq!(operation["sourceKind"], "aliasOnly", "{}", spec.name);
        assert_eq!(operation["aliasSource"], true);
        assert_eq!(operation["noFileSource"], true);
        let own = operation["answers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|answer| answer["choice"] == "ownPage")
            .unwrap();
        assert_eq!(own["allowed"], false, "no file to make into its own page");
    }
    let mg = model["merges"].get(0).cloned();
    let offers: Vec<&str> = operation["offers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let actual_offers = match rename_offer(&store, old, new, None) {
        Ok(RenameOffer::Rename) => vec!["rename", "cancel"],
        Ok(RenameOffer::Merge { .. }) => vec!["mergeOwner", "cancel"],
        Ok(RenameOffer::AliasOf {
            merge, own_page, ..
        }) => {
            let mut choices = Vec::new();
            if merge {
                choices.push("mergeOwner");
            }
            if own_page {
                choices.push("ownPage");
            }
            choices.push("cancel");
            choices
        }
        _ => vec!["cancel"],
    };
    if offers != actual_offers {
        diffs.push(Diff::new(
            "rename offers: model and Tine offer different answers",
            format!("model {offers:?}, Tine {actual_offers:?}"),
        ));
    }
    let own_page = own_page || actual_offers == ["ownPage", "cancel"];
    let r = if own_page {
        &operation["ownAnswer"]
    } else {
        operation
    };
    let cancel = operation["answers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["choice"] == "cancel")
        .unwrap();
    assert_eq!(cancel["allowed"], true);
    let cancelled: BTreeSet<_> = cancel["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        cancelled,
        spec.files.keys().map(String::as_str).collect(),
        "cancel preserves every file"
    );
    for file in cancel["files"].as_array().unwrap() {
        assert_eq!(
            file["text"].as_str(),
            spec.files
                .get(file["path"].as_str().unwrap())
                .map(String::as_str),
            "cancel preserves exact bytes"
        );
    }
    let file_names: BTreeMap<String, String> = ctx
        .files
        .iter()
        .map(|(p, f)| (p.clone(), f.name.clone()))
        .collect();
    drop(ctx);
    drop(view);

    let allowed = offers.contains(&if own_page { "ownPage" } else { "rename" });
    let base = operation["baseAllowed"].as_bool().unwrap();
    let own_child = r["ownChildUsed"].as_bool().unwrap_or(false);
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
        if own_child {
            why.push("own child");
        }
        format!("refuses ({})", why.join(" + "))
    };
    let merge_allowed = mg.as_ref().and_then(|m| m["allowed"].as_bool());
    let merge_says = match &mg {
        None => "has no merge".to_owned(),
        Some(m) if m["allowed"] == true => "merges".to_owned(),
        Some(m) => {
            let mut why = Vec::new();
            if m["target"].is_null() {
                why.push("no single target");
            }
            // Round 10: a reference-only source (`refSource`) has no file,
            // so a null `source` is no refusal reason there.
            if m["source"].is_null() && m["noFileSource"] != true {
                why.push("no single source");
            }
            if m["typedInSubtree"] == true || m["targetInSubtree"] == true {
                why.push("own child");
            }
            if m["journalRisks"].as_array().is_some_and(|a| !a.is_empty()) {
                why.push("journal risk");
            }
            if why.is_empty() {
                why.push("guard");
            }
            format!("refuses the merge ({})", why.join(" + "))
        }
    };
    let (tine, both) = run_tine(&store, old, new, own_page, droppers);
    let rename = format!("rename {old:?} -> {new:?}");
    // Which model answer the after graph is compared with: the rename's or
    // the merge's (`None`: not compared further).
    let mut chosen: Option<(&Value, bool)> = None;
    let performed: Option<(RenameOutcome, bool)> = match (&tine, allowed) {
        (TineRename::Refused(why), _) => {
            let kind = why.split(':').next().unwrap_or(why);
            if allowed {
                diffs.push(Diff::new(
                    format!("rename: model allows, Tine refuses ({kind})"),
                    format!("{rename}: model allows, Tine refuses: {why}"),
                ));
            } else if merge_allowed == Some(true) {
                diffs.push(Diff::new(
                    format!("merge: model merges, Tine refuses ({kind})"),
                    format!("{rename}: model refuses the rename ({model_says}) but merges into {}, Tine refuses: {why}", mg.as_ref().unwrap()["target"]),
                ));
            }
            None
        }
        // `Unchanged` writes nothing: in effect the same as a refusal (the
        // user sees no error, but no file changes).
        (
            TineRename::Performed {
                outcome: RenameOutcome::Unchanged,
                ..
            },
            false,
        ) => None,
        (
            TineRename::Performed {
                outcome,
                confirmed: false,
                ..
            },
            false,
        ) => {
            // Round 9: when the model would merge instead (M-Q7: into a
            // journal day), Tine's plain rename is a different operation.
            let merges_instead = if merge_allowed == Some(true) {
                " but merges"
            } else {
                ""
            };
            diffs.push(Diff::new(
                format!("rename: model {model_says}{merges_instead}, Tine performs"),
                format!(
                    "{rename}: model {model_says} (journal risks {risks:?}){merges_instead}{}, Tine performs ({outcome:?})",
                    if merge_allowed == Some(true) {
                        format!(" into {}", mg.as_ref().unwrap()["target"])
                    } else {
                        String::new()
                    }
                ),
            ));
            None
        }
        (
            TineRename::Performed {
                outcome,
                confirmed: true,
                into,
            },
            _,
        ) => match (&mg, merge_allowed) {
            (Some(m), Some(true)) => {
                if m["target"].as_str() != Some(into.as_str()) {
                    diffs.push(Diff::new(
                        "merge: model and Tine merge into different pages",
                        format!(
                            "{rename}: model merges into {}, Tine into {into}",
                            m["target"]
                        ),
                    ));
                    None
                } else {
                    chosen = Some((m, true));
                    Some((*outcome, true))
                }
            }
            (None, _) if !allowed => {
                diffs.push(Diff::new(
                        format!("rename: model {model_says}, Tine performs a confirmed merge"),
                        format!(
                            "{rename}: model {model_says} (journal risks {risks:?}), Tine performs ({outcome:?})"
                        ),
                    ));
                None
            }
            (None, _) => None,
            (Some(_), _) => {
                diffs.push(Diff::new(
                        format!("merge: model {merge_says}, Tine performs a confirmed merge"),
                        format!(
                            "{rename}: model {model_says} and {merge_says} (merge {}), Tine merges into {into} ({outcome:?})",
                            mg.as_ref().unwrap()
                                .as_object()
                                .map(|o| {
                                    o.iter()
                                        .filter(|(k, _)| {
                                            ["source", "target", "typedInSubtree", "targetInSubtree", "journalRisks"].contains(&k.as_str())
                                        })
                                        .map(|(k, v)| format!("{k}={v}"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default()
                        ),
                    ));
                None
            }
        },
        (
            TineRename::Performed {
                outcome,
                confirmed: false,
                ..
            },
            true,
        ) => {
            chosen = Some((r, own_page && operation["ownMerge"] == true));
            Some((*outcome, false))
        }
    };
    let Some((outcome, confirmed)) = performed else {
        later_drop(droppers, (store, dir));
        return (finish(diffs, duplicate, coarse_random), both);
    };
    let (r, is_merge) = chosen.expect("a performed operation has its model answer");
    let answer_name = if own_page {
        "ownPage"
    } else if is_merge {
        "mergeOwner"
    } else {
        "rename"
    };
    let answer = operation["answers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["choice"] == answer_name)
        .unwrap();
    assert_eq!(answer["allowed"], true, "executed answer is offered");
    // The model's final path of each before file: its own, for a moved file
    // the after file named by its new name, and for a merge's source the
    // survivor (the target's path).
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
    // A reference-only source (round 10 `refMerge`) has no file to fold.
    let merged_into: Option<(String, String)> = is_merge
        .then(|| r["source"].as_str())
        .flatten()
        .map(|src| (src.to_owned(), r["target"].as_str().unwrap().to_owned()));
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
        extra_config: spec.extra_config.clone(),
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
    let after_input = shared::model_input(&after_spec, shared::alias_spellings, &[]);
    let after_model = match shared::run_bridge(bridge, &after_input) {
        Ok(m) => m,
        Err(e) => {
            diffs.push(Diff::new("harness: bridge failed (after graph)", e));
            drop(view2);
            later_drop(droppers, (store2, dir));
            return (finish(diffs, duplicate, coarse_random), both);
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
        if let Some((src, dst)) = &merged_into {
            if path == src {
                if after_files.contains_key(dst) {
                    final_path.insert(path.clone(), dst.clone());
                } else {
                    diffs.push(Diff::new(
                        format!("merge: the survivor is gone after the merge{merge_tag}"),
                        format!("{rename}: model merges {src} into {dst}; no such after file"),
                    ));
                }
                if after_files.contains_key(src) && src != dst {
                    diffs.push(Diff::new(
                        format!("merge: the merged source file remains{merge_tag}"),
                        format!("{rename}: {src} still exists after merging into {dst}"),
                    ));
                }
                continue;
            }
        }
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
    let model_files: BTreeSet<&String> = answer["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| final_path.get(f["path"].as_str().unwrap()))
        .collect();
    if model_files != finals {
        diffs.push(Diff::new(
            "answer files: model execution and Tine keep different files",
            format!("{rename}: model {model_files:?}, Tine {finals:?}"),
        ));
    }
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
    for b in answer["files"].as_array().unwrap() {
        let path = b["path"].as_str().unwrap();
        let Some(fp) = final_path.get(path) else {
            continue;
        };
        for o in b["projection"]["occ"].as_array().unwrap() {
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
    let old_keys: BTreeSet<String> = tine_core::refs::namespace_refs(old).map(key).collect();
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
    for f in answer["files"].as_array().unwrap() {
        let Some(fp) = final_path.get(f["path"].as_str().unwrap()) else {
            continue;
        };
        let e = model_alias.entry(fp.clone()).or_default();
        for a in f["projection"]["aliases"].as_array().unwrap() {
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
                // A merge writes stored query arguments by the model's query
                // map (round 9, `Rulings9.queryMap`: the source's name becomes
                // the survivor's own name); a rename by the spelling map.
                let qmap = &answer["files"][0]["projection"]["queryMap"];
                let mapped = qmap[&a].as_str().map(str::to_owned).unwrap_or(a);
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
    (finish(diffs, duplicate, coarse_random), both)
}

/// [`run_rename`] with the dialog's first answer, and again with "make it its
/// own page" when the dialog offered both (M-Q5); the second run's
/// differences are tagged `(own page)` and both sets returned.
fn run_both(
    store_classes: &BTreeSet<String>,
    bridge: &Path,
    spec: &GraphSpec,
    old: &str,
    new: &str,
    coarse_random: bool,
    droppers: &Droppers,
) -> (Vec<Diff>, Option<Vec<Diff>>) {
    let (diffs, both) = run_rename(
        store_classes,
        bridge,
        spec,
        old,
        new,
        coarse_random,
        false,
        droppers,
    );
    let own = both.then(|| {
        run_rename(
            store_classes,
            bridge,
            spec,
            old,
            new,
            coarse_random,
            true,
            droppers,
        )
        .0
    });
    (diffs, own)
}

fn finish(mut diffs: Vec<Diff>, duplicate: bool, coarse_random: bool) -> Vec<Diff> {
    if coarse_random && duplicate {
        diffs = shared::coarse(diffs);
    }
    diffs.sort();
    diffs.dedup();
    diffs
}

/// Census Q4 item 4 (CM step 2, batch 2): the duplicate-day keeper. The
/// journal conflict list (`journal_conflicts`) sorts a day's files and the
/// merge (`conflicts.rs`) folds the strays into `files[0]`; the day itself
/// opens the claimant order's winner (`WholeGraph::resolve`). One question,
/// "which file owns this day", so the two must agree; Settings reads the
/// `canonical` flag, so it marks the keeper alone. Fixture graphs with a
/// duplicate day only; `None` when the graph has none.
fn duplicate_day_keepers(spec: &GraphSpec, droppers: &Droppers) -> Option<Vec<Diff>> {
    let dir = tempfile::tempdir().unwrap();
    spec.write(dir.path());
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let view = store.whole_graph().expect("graph loads");
    let days = tine_graph_features::journals::journal_conflicts(&store).unwrap();
    let mut diffs = Vec::new();
    for day in &days {
        let keeper = day.files.first().map(|f| f.path.clone());
        let opens = match view.resolve(&day.title, true) {
            Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
            _ => None,
        };
        // Settings offers Merge into the one file flagged canonical: it must
        // be the keeper, and only it.
        let flagged: Vec<&str> = day
            .files
            .iter()
            .filter(|f| f.canonical)
            .map(|f| f.path.as_str())
            .collect();
        if flagged != keeper.as_deref().into_iter().collect::<Vec<_>>() {
            diffs.push(Diff::new(
                "owners: duplicate-day canonical flag is not the keeper alone",
                format!("{}: keeper {keeper:?}, flagged {flagged:?}", day.title),
            ));
        }
        if keeper != opens {
            diffs.push(Diff::new(
                "owners: duplicate-day keeper (journal_conflicts) vs the file the day opens",
                format!(
                    "{}: keeper {keeper:?} (files {:?}), the day opens {opens:?}",
                    day.title,
                    day.files
                        .iter()
                        .map(|f| (f.path.as_str(), f.canonical))
                        .collect::<Vec<_>>()
                ),
            ));
        }
    }
    later_drop(droppers, (view, store, dir));
    (!days.is_empty()).then_some(diffs)
}

fn all_fixtures() -> Vec<GraphSpec> {
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
        assert!(
            std::env::var_os("CM_REQUIRE_BRIDGE").is_none(),
            "the r13 gate requires CM_BRIDGE"
        );
        eprintln!("CM_BRIDGE unset: conceptual model rename conformance skipped");
        return;
    };
    let bridge = Path::new(&bridge);
    if shared::r13::is_bridge(bridge) {
        r13_rename::run(bridge);
        return;
    }
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
            let (diffs, own) = run_both(&store_classes, bridge, &spec, old, new, false, &droppers);
            let name = format!("{}: {old} -> {new}", spec.name);
            let runs = std::iter::once((name.clone(), diffs))
                .chain(own.map(|own| (format!("{name} (own page)"), own)));
            for (name, diffs) in runs {
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
    }
    for spec in all_fixtures() {
        if let Some(diffs) = duplicate_day_keepers(&spec, &droppers) {
            section.fixtures.insert(
                format!("{}: duplicate days", spec.name),
                diffs.into_iter().map(|d| d.detail).collect(),
            );
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
                let (mut diffs, own) =
                    run_both(&store_classes, bridge, spec, old, new, true, &droppers);
                for d in own.into_iter().flatten() {
                    diffs.push(Diff::new(format!("{} (own page)", d.class), d.detail));
                }
                diffs.sort();
                diffs.dedup();
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
