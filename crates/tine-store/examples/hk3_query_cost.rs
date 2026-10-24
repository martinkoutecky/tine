//! Measurement only. Run on a disposable graph copy, under the dossier's 2 GiB cap.
//! Emits counts/timings only; never emits graph names, query text or result content.
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path, time::Instant};
use tine_core::{
    doc::DocBlock,
    query::{
        ir::{Attr, ExecutionContext, Leaf, Query, QueryRows},
        macro_text::query_macro_extents,
        parse_query_input,
        registry::Registry,
        QueryInput,
    },
};
use tine_store::{Cancel, EditKind, IrAnswer, IrRequest, SaveBase, Store, TxOutcome};

fn shape(query: &Query) -> &'static str {
    let filter = query.evaluable_filter();
    if query.is_invalid() {
        "refused"
    } else if filter.reads_links() {
        "links_or_near"
    } else if filter.any_leaf(&mut |l| {
        matches!(
            l,
            Leaf::Attr {
                attr: Attr::UsedAsTag,
                ..
            }
        )
    }) {
        "used_as_tag"
    } else if filter.has_props_leaf() {
        "registry_typed_props"
    } else {
        "page_local"
    }
}

// Walk the parser AST, so fenced examples and inline code are not query blocks.
fn count_ast(v: &Value, raw: &str, counts: &mut BTreeMap<String, usize>) {
    if let Some(obj) = v.as_object() {
        let name = obj.get("name").and_then(Value::as_str).unwrap_or("");
        if obj.get("k").and_then(Value::as_str) == Some("macro")
            && (name.eq_ignore_ascii_case("query") || name.eq_ignore_ascii_case("tine-query"))
        {
            // The shared brace-aware reader recovers maps beyond lsdoc's first }}.
            let start = obj
                .get("span")
                .and_then(|s| s.get(0))
                .and_then(Value::as_u64)
                .map(|s| s as usize);
            let prefix = 2isize - (raw.len() - raw.trim_start().len()) as isize;
            let extent = query_macro_extents(raw).into_iter().find(|e| {
                e.name.eq_ignore_ascii_case(name)
                    && start.is_none_or(|s| e.start as isize + prefix == s as isize)
            });
            if let Some(e) = extent {
                let input = if name.eq_ignore_ascii_case("query") {
                    QueryInput::MacroQuery
                } else {
                    QueryInput::MacroTql
                };
                let (q, _) = parse_query_input(
                    &e.argument,
                    input,
                    tine_core::date::JournalDate::today(),
                    Registry::none(),
                );
                *counts
                    .entry(format!("{}:{}", name.to_ascii_lowercase(), shape(&q)))
                    .or_default() += 1;
            }
            return;
        }
        if obj.get("kind").and_then(Value::as_str) == Some("custom")
            && name.eq_ignore_ascii_case("query")
        {
            let (q, _) = parse_query_input(
                raw,
                QueryInput::Advanced,
                tine_core::date::JournalDate::today(),
                Registry::none(),
            );
            *counts
                .entry(format!("begin_query:{}", shape(&q)))
                .or_default() += 1;
            return;
        }
        for child in obj.values() {
            count_ast(child, raw, counts);
        }
    } else if let Some(items) = v.as_array() {
        for child in items {
            count_ast(child, raw, counts);
        }
    }
}
fn count_blocks(blocks: &[DocBlock], counts: &mut BTreeMap<String, usize>, total: &mut usize) {
    for block in blocks {
        *total += 1;
        if block.raw().contains("{{") || block.raw().to_ascii_lowercase().contains("#+begin_query")
        {
            let ast = tine_core::render::parse_block(block.raw(), block.is_org());
            count_ast(&serde_json::to_value(ast).unwrap(), block.raw(), counts);
        }
        count_blocks(&block.children, counts, total);
    }
}
// This mode never opens a Store or writes anything, including on the real graph.
fn inventory_read_only(root: &Path) {
    fn files(
        dir: &Path,
        counts: &mut BTreeMap<String, usize>,
        blocks: &mut usize,
        per_file: &mut Vec<usize>,
    ) {
        if !dir.exists() {
            return;
        }
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                files(&path, counts, blocks, per_file);
                continue;
            }
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !matches!(ext, "md" | "org") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("UTF-8 graph input");
            let doc = if ext == "org" {
                tine_core::org::parse_org(&text)
            } else {
                tine_core::doc::parse(&text)
            };
            let mut local = BTreeMap::new();
            count_blocks(&doc.roots, &mut local, blocks);
            per_file.push(local.values().sum());
            for (key, value) in local {
                *counts.entry(key).or_default() += value;
            }
        }
    }
    let mut counts = BTreeMap::new();
    let mut blocks = 0;
    let mut pages = Vec::new();
    let mut journals = Vec::new();
    files(&root.join("pages"), &mut counts, &mut blocks, &mut pages);
    files(
        &root.join("journals"),
        &mut counts,
        &mut blocks,
        &mut journals,
    );
    journals.sort_unstable();
    let percentile = |p: f64| {
        journals
            .get(((journals.len() as f64 * p).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0)
    };
    println!(
        "{}",
        json!({"inventory":{"pages":pages.len()+journals.len(),"blocks":blocks,"queries":counts,"journals":journals.len(),"journalQueryMedian":percentile(0.5),"journalQueryP95":percentile(0.95),"journalQueryMax":journals.last()}})
    );
}
fn edit(store: &Store, id: &tine_store::PageId, n: usize, tag: bool) {
    let mut page = store.page(id).unwrap();
    // A dedicated one-block fixture is created by the runner, never a user's page.
    if tag {
        page.doc.pre_block = Some(format!(
            "tags:: hk3-probe-{n}\nstatus:: inactive\nhk3-prop-{n}:: value"
        ));
    } else {
        page.doc.blocks[0].raw = format!("probe {n}");
    }
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        id,
        SaveBase::Existing(page.rev),
        &page.doc,
    );
    assert!(
        matches!(tx.commit(), TxOutcome::Committed { .. }),
        "guarded fixture save failed"
    );
}
fn sample(store: &Store, query: Option<(&Query, &tine_core::query::ir::ViewSettings)>) -> Value {
    let start = Instant::now();
    let graph = store.whole_graph().unwrap();
    let context = ExecutionContext::none();
    let request = query.map_or(IrRequest::Registry, |(query, view)| IrRequest::Run {
        query,
        view,
        context: &context,
    });
    let answer = graph
        .query_ir(request, &Cancel::default())
        .unwrap()
        .unwrap();
    let ms = start.elapsed().as_secs_f64() * 1000.;
    let (rows, returned_rows, bytes, exceeded) = match answer {
        IrAnswer::Result(r) => {
            let returned_rows = match &r.rows {
                QueryRows::Page { pages } => pages.len(),
                QueryRows::Block { groups } => groups.iter().map(|g| g.blocks.len()).sum(),
            };
            (
                r.total,
                returned_rows,
                serde_json::to_vec(&r).unwrap().len(),
                r.exceeded,
            )
        }
        IrAnswer::Registry(r) => (
            r.rows().len(),
            r.rows().len(),
            serde_json::to_vec(&r.snapshot()).unwrap().len(),
            false,
        ),
        _ => unreachable!(),
    };
    json!({"ms":ms,"rows":rows,"returnedRows":returned_rows,"bytes":bytes,"exceeded":exceeded,"withSerializationMs":start.elapsed().as_secs_f64()*1000.})
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--inventory-read-only") {
        inventory_read_only(Path::new(args.get(2).expect("graph directory")));
        return;
    }
    let root = args.get(1).expect("disposable graph copy");
    let root = Path::new(&root).canonicalize().unwrap();
    assert!(
        root.starts_with(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/og-hk3-query-cost/bulk")
                .canonicalize()
                .unwrap()
        ),
        "only lane scratch copies allowed"
    );
    let (store, _, _) = Store::open(&root, Default::default()).unwrap();
    store.whole_graph().unwrap();
    store.stop_watcher_for_tests();
    let mut counts = BTreeMap::new();
    let mut journal_counts = Vec::new();
    let mut blocks = 0;
    let mut pages = 0;
    let mut seed = None;
    {
        let corpus = store.whole_graph().unwrap().corpus();
        for page in &corpus.pages {
            pages += 1;
            if page.name != "HK3 Probe"
                && page.kind == tine_core::model::PageKind::Page
                && seed.as_ref().is_none_or(|s| &page.name < s)
            {
                seed = Some(page.name.clone());
            }
            let mut local = BTreeMap::new();
            count_blocks(&page.document.roots, &mut local, &mut blocks);
            if page.kind == tine_core::model::PageKind::Journal {
                journal_counts.push(local.values().sum::<usize>());
            }
            for (key, value) in local {
                *counts.entry(key).or_insert(0usize) += value;
            }
        }
    }
    journal_counts.sort_unstable();
    let quantile = |p: f64| {
        journal_counts
            .get(((journal_counts.len() as f64 * p).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0)
    };
    println!(
        "{}",
        json!({"inventory":{"pages":pages,"blocks":blocks,"queries":counts,"journals":journal_counts.len(),"journalQueryMedian":quantile(0.5),"journalQueryP95":quantile(0.95),"journalQueryMax":journal_counts.last()}})
    );
    let seed = seed.expect("ordinary page").replace('\'', "''");
    let cases = [
        (
            "links",
            format!("@page and any(links, name = '{seed}')"),
            QueryInput::Tql,
        ),
        (
            "linked_by",
            format!("@page and any(linked_by, name = '{seed}')"),
            QueryInput::Tql,
        ),
        (
            "near1",
            format!("@page and near(name = '{seed}', 1)"),
            QueryInput::Tql,
        ),
        (
            "near2",
            format!("@page and near(name = '{seed}', 2)"),
            QueryInput::Tql,
        ),
        (
            "orphans",
            "@page and none(linked_by, true)".into(),
            QueryInput::Tql,
        ),
        ("used_as_tag", "(all-page-tags)".into(), QueryInput::Og),
        ("typed_props", "(property status)".into(), QueryInput::Og),
        (
            "journal_tasks_control",
            "(task TODO DOING)".into(),
            QueryInput::Og,
        ),
        ("registry", "".into(), QueryInput::Tql),
    ];
    let id = tine_store::PageId::from("pages/HK3 Probe.md");
    let mut serial = 0;
    for (label, text, input) in cases {
        let (q, view) = parse_query_input(
            &text,
            input,
            tine_core::date::JournalDate::today(),
            Registry::none(),
        );
        assert!(
            label == "registry" || !q.is_invalid(),
            "invalid benchmark case {label}"
        );
        let request = (label != "registry").then_some((&q, &view));
        // Prime index projections and this query; exclude this sample from distributions.
        let first = sample(&store, request);
        let mut runs = Vec::new();
        for _ in 0..20 {
            let repeat = sample(&store, request);
            serial += 1;
            edit(&store, &id, serial, false);
            let edited = sample(&store, request);
            serial += 1;
            edit(&store, &id, serial, true);
            let tags_changed = sample(&store, request);
            runs.push(json!({"repeat":repeat,"afterTextEdit":edited,"afterTagEdit":tags_changed}));
        }
        println!("{}", json!({"shape":label,"first":first,"runs":runs}));
    }
    store.close();
}
