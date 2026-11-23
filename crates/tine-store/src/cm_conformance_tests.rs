//! Static conformance against K's exporter (`CM_STATIC_BRIDGE`). The parser's
//! names, day identities, aliases, outline and occurrences are inputs; static
//! answers and frontend fixtures come from that one model path. Operations
//! run separately through `tine-graph-features/tests/cm_witness.rs`.
//!
//! `CM_REQUIRE_BRIDGE=1` makes a missing exporter fatal. `CM_REPORT` writes
//! the full static report; `CM_REFRESH=1` refreshes the static differences and
//! frontend cases; `CM_RANDOM=<n>` changes the 300 seeded random graphs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use tine_core::refs::page_key;

use crate::query::document_alias_spellings;
use crate::Store;
use tine_core::model::PageKind;

/// The store names the shared code uses (`super::store_crate` there).
mod store_crate {
    pub(crate) use crate::{
        Cancel, InventoryEntry, InventoryScope, IrAnswer, IrRequest, LinkTarget, Resolved, Store,
        WholeGraph,
    };
}

#[path = "cm_conformance_shared_tests.rs"]
mod shared;

#[path = "cm_conformance_extraction_tests.rs"]
mod extraction;

use shared::{Ctx, Diff, GraphSpec, Section};

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
    store.whole_graph().unwrap();
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

/// I-12: the presentation the bridge is told (`showHeaders`,
/// `removeBlockChildren`) is the engine plan's own row gates, so conformance
/// compares the rows Tine shows, never a setting the adapter chose. Both
/// dialects, both anchors, the advanced form and the trailing options map; a
/// `@page` query is checked under the legacy block-wise bridge too.
#[test]
fn presentation_is_the_plans_row_gates() {
    use std::sync::Arc;
    use tine_core::date::JournalDate;
    use tine_core::query::{parse_query_text, QueryDialect};
    let cases = [
        ("[[Target]]", QueryDialect::Og),
        ("(page-ref Target)", QueryDialect::Og),
        ("(property related Target)", QueryDialect::Og),
        ("(page-property type book)", QueryDialect::Og),
        (
            "[[Target]] {:remove-block-children? false}",
            QueryDialect::Og,
        ),
        (
            "{:query [:find (pull ?b [*]) :where [?b :block/refs ?p] [?p :block/name \"target\"]]}",
            QueryDialect::Og,
        ),
        ("[[Target]]", QueryDialect::Tql),
        ("@page and name = 'Target'", QueryDialect::Tql),
    ];
    let today = JournalDate::from_days(0);
    let mut anchors = BTreeSet::new();
    for (text, dialect) in cases {
        let (query, _) = parse_query_text(text, dialect, today);
        anchors.insert(format!("{:?}", query.anchor));
        let told = shared::k::presentation(&query);
        for block_rows in [false, true] {
            let plan = crate::query::exec::Plan::new(
                &query,
                today,
                block_rows,
                query.removes_block_children(),
                false,
                || {
                    Arc::new(tine_core::query::registry::Registry::empty(
                        &Default::default(),
                    ))
                },
                || Arc::new(Default::default()),
            );
            let gates = plan.checkpoint_parts();
            assert_eq!(
                told,
                json!({"showHeaders": gates.page_property_rows,
                       "removeBlockChildren": gates.top_level_only}),
                "{text} ({dialect:?}, block_rows {block_rows})"
            );
        }
    }
    assert_eq!(anchors.len(), 2, "both anchors covered: {anchors:?}");
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

/// The Linked References link table as typed model pairs (`shared::k`): a
/// journal node is its day's token, as the confirmed model keys a day.
fn typed_link_pairs(
    view: &crate::WholeGraph,
    spec: &GraphSpec,
    input: &Value,
    diffs: &mut Vec<Diff>,
) -> BTreeSet<(shared::Page, shared::Page)> {
    let journal = spec.journal_format();
    let snapshot = view.test_read_snapshot();
    let Some(table) = snapshot.link_graph(&|| false) else {
        diffs.push(Diff::new("harness: link table not built", "link table"));
        return BTreeSet::new();
    };
    let mut node_page: BTreeMap<u32, shared::Page> = BTreeMap::new();
    for s in input["literalKeys"].as_object().unwrap().keys() {
        let pk = page_key(s);
        for is_journal in [false, true] {
            if let Some(n) = table.node(&pk, is_journal) {
                if (n as usize) < table.physical() {
                    node_page.entry(n).or_insert_with(|| {
                        if is_journal {
                            ("journal".to_owned(), shared::k::key(&journal, s))
                        } else {
                            ("page".to_owned(), pk.clone())
                        }
                    });
                }
            }
        }
    }
    for (i, v) in table.virtuals().iter().enumerate() {
        let page = match v.journal_day {
            Some(day) => ("journal".to_owned(), shared::k::ordinal_token(day)),
            None => ("page".to_owned(), page_key(&v.name)),
        };
        node_page.insert((table.physical() + i) as u32, page);
    }
    let total = table.physical() + table.virtuals().len();
    let mut pairs = BTreeSet::new();
    for n in 0..total as u32 {
        let Some(s) = node_page.get(&n) else {
            diffs.push(Diff::new(
                "harness: link table node without a known key",
                format!("link table node {n}"),
            ));
            continue;
        };
        for t in table.links(n) {
            match node_page.get(t) {
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

/// The static corpus and frontend cases use only K's exporter. Missing the
/// exporter is fatal in the focused gate (`CM_REQUIRE_BRIDGE=1`).
#[test]
fn conceptual_model_conformance() {
    let Ok(bridge) = std::env::var("CM_STATIC_BRIDGE") else {
        assert!(
            std::env::var_os("CM_REQUIRE_BRIDGE").is_none(),
            "the gate requires CM_STATIC_BRIDGE"
        );
        eprintln!("CM_STATIC_BRIDGE unset: static comparison skipped");
        return;
    };
    let mut cases = BTreeMap::new();
    let roots = fixture_specs();
    let frontend_names: BTreeSet<_> = roots.iter().map(|s| s.name.clone()).collect();
    shared::k::store_conformance(
        Path::new(&bridge),
        roots,
        env!("CARGO_MANIFEST_DIR"),
        typed_link_pairs,
        |store, view, spec, input, model| {
            for (path, text) in &spec.files {
                let doc = shared::parse_file(path, text);
                assert_eq!(
                    shared::alias_spellings(&doc),
                    document_alias_spellings(&doc),
                    "{path}: shared extractor"
                );
            }
            if frontend_names.contains(&spec.name) {
                let ctx = Ctx::new(view, spec.journal_format(), input, model);
                let date_diffs = shared::compare_reference_days(&ctx);
                assert!(
                    date_diffs.is_empty(),
                    "{}: native reference-day query answers: {date_diffs:?}",
                    spec.name
                );
                let mut case = shared::frontend_case(
                    &ctx,
                    spec.journal_title_format.as_deref(),
                    spec.journal_file_format.as_deref(),
                );
                let owner: BTreeMap<String, Value> = case["open"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(|name| {
                        let at = |kind| {
                            store
                                .page_named(name, kind)
                                .unwrap()
                                .map(|p| p.id.as_str().to_owned())
                        };
                        (
                            name.clone(),
                            json!({"page": at(PageKind::Page), "journal": at(PageKind::Journal)}),
                        )
                    })
                    .collect();
                case["owner"] = json!(owner);
                cases.insert(spec.name.clone(), case);
            }
            compare_namespace_parent_answers(view)
        },
    );
    check_frontend_cases("frontend", cases,
        "CM_REFRESH=1 CM_STATIC_BRIDGE=<cm-bridge-k> cargo test -p tine-store --lib conceptual_model_conformance, then CM_REFRESH=1 npx vitest run src/cmConformanceNavigate.test.ts");
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
