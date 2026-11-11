//! D-20261102-PERF row 3 (remainder): creating, deleting and renaming a page
//! publishes a precise change summary instead of `full`. The GX4 oracle
//! (`change_summary.rs`) compares every mounted reader's real answer before
//! and after each operation and requires its key; the counter test shows that
//! mounted queries the page cannot affect do not rerun.
use super::change_summary::{
    answers_with, apply_published, check, commit, edit, fixture, strings, Answers,
};
use crate::{Area, EditKind, PageId, SaveBase, Store};
use serde_json::Value;
use std::fs;
use std::sync::Arc;
use tine_core::model::PageDto;
use tine_core::query::ir::Query;

const EXTRA_TARGETS: &[&str] = &[
    "Fresh",
    "Gone",
    "Retitled",
    "Empty",
    "Moved Empty",
    "Org Page",
    "NS/New",
    "Oct 23rd, 2026",
    "2026-10-24",
    "Aliased",
    "Nick",
];
const EXTRA_PATHS: &[&str] = &[
    "pages/Fresh.md",
    "pages/Gone.md",
    "pages/Empty.md",
    "pages/Moved Empty.md",
    "pages/Org Page.org",
    "pages/ns-new.md",
    "journals/2026_10_23.md",
    "pages/2026-10-24.md",
    "pages/Aliased.md",
    "pages/Shortcut.md",
    "pages/a.md",
];

fn parse(source: &str) -> Query {
    use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
    parse_query_pair(
        source,
        QueryTextDialect::MacroQuery,
        &[],
        tine_core::query::registry::Registry::none(),
    )
    .query
}

/// A TQL query (`{{tine-query …}}`), the dialect with page rows by name.
fn tql(source: &str) -> Query {
    use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
    parse_query_pair(
        source,
        QueryTextDialect::MacroTql,
        &[],
        tine_core::query::registry::Registry::none(),
    )
    .query
}

/// Mounted readers that depend on the pages the scenarios create, rename and
/// delete: by reference, page name, namespace, journal day and task.
fn dependent_queries() -> Vec<(&'static str, Query)> {
    [
        ("fresh-ref", "[[Fresh]]"),
        ("gone-ref", "[[Gone]]"),
        ("fresh-page", "(page \"Fresh\")"),
        ("retitled-page", "(page \"Retitled\")"),
        ("namespace", "(namespace NS)"),
        ("day-ref", "[[Oct 23rd, 2026]]"),
        ("dated-ref", "[[2026-10-24]]"),
        ("todo", "(task TODO)"),
        ("kind", "(page-property kind note)"),
    ]
    .into_iter()
    .map(|(id, source)| (id, parse(source)))
    .collect()
}

/// `n` mounted queries no page in the scenarios can affect.
fn unrelated_queries(n: usize) -> Vec<(String, Query)> {
    (0..n)
        .map(|i| {
            let source = if i % 2 == 0 {
                format!("[[Topic {i}]]")
            } else {
                format!("(and (task NOW) [[Topic {i}]])")
            };
            (format!("unrelated-{i}"), parse(&source))
        })
        .collect()
}

fn mount(store: &Store, id: &str, query: &Query) {
    store
        .register_query_subscription(
            id.into(),
            Some(crate::QuerySubscription::Query {
                query: query.clone(),
                context: Default::default(),
            }),
            None,
        )
        .unwrap();
}

fn page(store: &Store, pre_block: Option<&str>, raw: &str) -> PageDto {
    let mut doc = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    doc.pre_block = pre_block.map(str::to_owned);
    doc.blocks.truncate(1);
    doc.blocks[0].raw = raw.into();
    doc.blocks[0].children.clear();
    doc
}

fn create(store: &Arc<Store>, path: &str, pre_block: Option<&str>, raw: &str) -> Value {
    let doc = page(store, pre_block, raw);
    commit(store, &[(path, doc, SaveBase::CreateNew)])
}

fn move_page(store: &Arc<Store>, from: &str, to: &str) -> Value {
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    let from = PageId::from(from);
    tx.move_file(
        &from.file(),
        store.page(&from).unwrap().rev,
        &store.file_id(Area::Pages, to).unwrap(),
        None,
    );
    apply_published(store, tx)
}

fn trash(store: &Arc<Store>, path: &str) -> Value {
    let mut tx = store.page_plan(Some(EditKind::DeletePage));
    let from = PageId::from(path);
    tx.trash(
        &from.file(),
        store.page(&from).unwrap().rev,
        crate::TrashIf::Any,
    );
    apply_published(store, tx)
}

/// An outside write the watcher path publishes (refresh, as the oracle does).
fn outside(store: &Arc<Store>, root: &std::path::Path, path: &str, text: Option<&str>) -> Value {
    let sub = store.subscribe();
    match text {
        Some(text) => fs::write(root.join(path), text).unwrap(),
        None => fs::remove_file(root.join(path)).unwrap(),
    }
    store.refresh(crate::Depth::Stamps).unwrap();
    crate::test_host_writer::HostWriter::for_store(store).settle();
    let mut last = None;
    while let Some(change) = sub.try_recv().unwrap() {
        last = Some(serde_json::to_value(change).unwrap());
    }
    last.expect("outside publication")
}

struct Oracle {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    queries: Vec<(&'static str, Query)>,
    unrelated: Vec<(String, Query)>,
}

impl Oracle {
    fn new(unrelated: usize) -> Self {
        let (dir, store) = fixture(3);
        let queries = dependent_queries();
        for (id, query) in &queries {
            mount(&store, id, query);
        }
        let unrelated = unrelated_queries(unrelated);
        for (id, query) in &unrelated {
            mount(&store, id, query);
        }
        Oracle {
            dir,
            store,
            queries,
            unrelated,
        }
    }

    fn answers(&self) -> Answers {
        let mut queries: Vec<(&str, Query)> = self
            .queries
            .iter()
            .map(|(id, query)| (*id, query.clone()))
            .collect();
        queries.extend(
            self.unrelated
                .iter()
                .map(|(id, q)| (id.as_str(), q.clone())),
        );
        answers_with(
            &self.store,
            self.dir.path(),
            EXTRA_TARGETS,
            EXTRA_PATHS,
            &queries,
        )
    }

    /// Run `op`, check every changed reader is named, and return the summary
    /// with the number of unrelated queries it reran: all of them on `full`
    /// (the window then advances every key, `changeKeys.ts`), else those in
    /// `queryTo`.
    fn step(&self, label: &str, op: impl FnOnce(&Self) -> Value) -> (Value, usize) {
        let before = self.answers();
        let wire = op(self);
        let after = self.answers();
        let summary = wire["summary"].clone();
        check(&before, &after, &summary)
            .unwrap_or_else(|key| panic!("{label}: omitted {key}: {summary}"));
        let fired = strings(&summary["queryTo"]);
        let unrelated = self
            .unrelated
            .iter()
            .filter(|(id, _)| summary["full"] == true || fired.contains(id))
            .count();
        (summary, unrelated)
    }
}

/// Every reader that depends on a created, renamed or deleted page still
/// updates (the oracle's soundness check); alias-free pages are precise.
#[test]
fn page_lifecycle_summaries_name_every_dependent_reader() {
    let _guard = crate::test_transactions::integration_lock();
    let oracle = Oracle::new(4);
    let store = &oracle.store;
    let root = oracle.dir.path().to_path_buf();
    let precise = |label: &str, (summary, unrelated): (Value, usize)| {
        assert_eq!(summary["full"], false, "{label}: {summary}");
        assert_eq!(unrelated, 0, "{label}: unrelated queries reran: {summary}");
        summary
    };
    // References to pages that do not exist yet.
    oracle.step("setup", |o| {
        edit(&o.store, "pages/A.md", |doc| {
            doc.blocks[0].raw = "TODO see [[Fresh]] and [[Gone]] and [[Oct 23rd, 2026]]".into()
        })
    });
    // Create a page a mounted query already references by name.
    let summary = precise(
        "create referenced",
        oracle.step("create referenced", |o| {
            create(
                &o.store,
                "pages/Fresh.md",
                Some("kind:: note"),
                "TODO fresh [[Target]]",
            )
        }),
    );
    assert!(strings(&summary["pages"]).contains("fresh"));
    assert!(strings(&summary["linksTo"]).contains("fresh"));
    for id in ["fresh-page", "todo", "kind"] {
        assert!(strings(&summary["queryTo"]).contains(id), "{id}: {summary}");
    }
    // An empty page still names its readers.
    let summary = precise(
        "create empty",
        oracle.step("create empty", |o| {
            outside(&o.store, &root, "pages/Empty.md", Some(""))
        }),
    );
    assert!(strings(&summary["pages"]).contains("empty"), "{summary}");
    // A namespace child named by `title::`.
    precise(
        "create namespace child",
        oracle.step("create namespace child", |o| {
            create(
                &o.store,
                "pages/ns-new.md",
                Some("title:: NS/New"),
                "child of NS",
            )
        }),
    );
    // An Org page, written from outside.
    precise(
        "create org",
        oracle.step("create org", |o| {
            outside(
                &o.store,
                &root,
                "pages/Org Page.org",
                Some("* TODO org mentions [[Fresh]] and Target\n"),
            )
        }),
    );
    // A journal day that a mounted query references, and a date-named page.
    let summary = precise(
        "create journal",
        oracle.step("create journal", |o| {
            create(&o.store, "journals/2026_10_23.md", None, "day [[Fresh]]")
        }),
    );
    assert_eq!(summary["journals"], true);
    assert_eq!(summary["days"], true);
    assert!(
        strings(&summary["queryTo"]).contains("day-ref"),
        "{summary}"
    );
    precise(
        "create date-named page",
        oracle.step("create date-named page", |o| {
            create(&o.store, "pages/2026-10-24.md", None, "dated page")
        }),
    );
    // Rename by `title::` (same file), then by moving the file.
    let summary = precise(
        "retitle",
        oracle.step("retitle", |o| {
            edit(&o.store, "pages/Fresh.md", |doc| {
                doc.pre_block = Some("title:: Retitled\nkind:: note".into())
            })
        }),
    );
    for key in ["fresh", "retitled"] {
        assert!(strings(&summary["pages"]).contains(key), "{key}: {summary}");
    }
    for id in ["fresh-page", "retitled-page"] {
        assert!(strings(&summary["queryTo"]).contains(id), "{id}: {summary}");
    }
    precise(
        "move",
        oracle.step("move", |o| {
            move_page(&o.store, "pages/Empty.md", "Moved Empty.md")
        }),
    );
    // Delete a page with backlinks, from the app and from outside.
    oracle.step("create gone", |o| {
        create(
            &o.store,
            "pages/Gone.md",
            None,
            "gone body links [[Target]]",
        )
    });
    let summary = precise(
        "delete with backlinks",
        oracle.step("delete with backlinks", |o| {
            trash(&o.store, "pages/Gone.md")
        }),
    );
    assert!(strings(&summary["pages"]).contains("gone"));
    assert!(strings(&summary["linksTo"]).contains("target"), "{summary}");
    precise(
        "outside delete",
        oracle.step("outside delete", |o| {
            outside(&o.store, &root, "pages/Org Page.org", None)
        }),
    );
    precise(
        "delete journal",
        oracle.step("delete journal", |o| {
            trash(&o.store, "journals/2026_10_23.md")
        }),
    );
    // A case twin of an existing page that loses the name is the active page
    // on neither side: the conservative path, and the oracle still holds.
    for (label, text) in [
        ("create twin", Some("- twin of A\n")),
        ("delete twin", None),
    ] {
        let (summary, _) = oracle.step(label, |o| outside(&o.store, &root, "pages/a.md", text));
        assert_eq!(summary["full"], true, "{label}: {summary}");
    }
    // Aliases regroup pages: a page that declares one, or that another page
    // declares (Target has `alias:: Shortcut`), keeps the conservative path.
    let (summary, _) = oracle.step("create aliased", |o| {
        create(
            &o.store,
            "pages/Aliased.md",
            Some("alias:: Nick"),
            "aliased",
        )
    });
    assert_eq!(summary["full"], true, "{summary}");
    let (summary, _) = oracle.step("create alias target", |o| {
        create(
            &o.store,
            "pages/Shortcut.md",
            None,
            "a page named like an alias",
        )
    });
    assert_eq!(summary["full"], true, "{summary}");
    store.close();
}

/// The counter: N unrelated mounted queries, one page operation each. Before
/// this change every create, rename and delete reran all N (`full`); now the
/// number is constant (0), whatever N is.
#[test]
fn page_lifecycle_reruns_no_unrelated_mounted_query() {
    let _guard = crate::test_transactions::integration_lock();
    let mut rows = Vec::new();
    for n in [8, 64] {
        let oracle = Oracle::new(n);
        let root = oracle.dir.path().to_path_buf();
        let ops: [(&str, Box<dyn Fn(&Oracle) -> Value>); 6] = [
            (
                "create",
                Box::new(|o| create(&o.store, "pages/Fresh.md", None, "fresh")),
            ),
            (
                "move",
                Box::new(|o| move_page(&o.store, "pages/Fresh.md", "Moved Empty.md")),
            ),
            (
                "retitle",
                Box::new(|o| {
                    edit(&o.store, "pages/Moved Empty.md", |doc| {
                        doc.pre_block = Some("title:: Retitled".into())
                    })
                }),
            ),
            (
                "delete",
                Box::new(|o| trash(&o.store, "pages/Moved Empty.md")),
            ),
            (
                "journal create",
                Box::new(|o| create(&o.store, "journals/2026_10_23.md", None, "day")),
            ),
            (
                "outside create",
                Box::new(move |o| outside(&o.store, &root, "pages/Gone.md", Some("- gone\n"))),
            ),
        ];
        for (label, op) in ops {
            let (summary, unrelated) = oracle.step(label, |o| op(o));
            rows.push((n, label, summary["full"] == true, unrelated));
        }
        oracle.store.close();
    }
    let report: Vec<_> = rows
        .iter()
        .map(|(n, label, full, unrelated)| {
            format!("n={n} {label}: full={full} unrelated reruns={unrelated}")
        })
        .collect();
    eprintln!("page lifecycle counters:\n{}", report.join("\n"));
    for (n, label, full, unrelated) in rows {
        assert!(!full, "{label} with {n} unrelated queries published full");
        assert_eq!(unrelated, 0, "{label} with {n} unrelated queries");
    }
}

/// Timings on the realistic 10k graph: each page operation's publication, and
/// the work its summary makes the mounted readers redo (every mounted query
/// when `full`, else those in `queryTo`). Run with
/// `TINE_LIFECYCLE_BENCH_GRAPH=<dir from scripts/generate-realistic-graph.mjs ROOT 7000 3000 543>
/// cargo test --release -p tine-store --features test-faults --lib page_lifecycle_timings_on_the_realistic_graph -- --ignored --nocapture`.
/// The graph is copied to a scratch directory first; the source is not written.
#[test]
#[ignore = "needs TINE_LIFECYCLE_BENCH_GRAPH (scripts/generate-realistic-graph.mjs)"]
fn page_lifecycle_timings_on_the_realistic_graph() {
    use std::time::{Duration, Instant};
    let _guard = crate::test_transactions::integration_lock();
    let source = std::path::PathBuf::from(
        std::env::var("TINE_LIFECYCLE_BENCH_GRAPH").expect("TINE_LIFECYCLE_BENCH_GRAPH"),
    );
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("cp")
        .arg("-a")
        .arg(source.join("."))
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    let started = Instant::now();
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let graph = store.whole_graph().unwrap();
    eprintln!("open: {:?}", started.elapsed());
    let names: Vec<String> = graph
        .corpus(None, false)
        .pages
        .iter()
        .filter(|page| page.kind == tine_core::model::PageKind::Page)
        .map(|page| page.name.clone())
        .collect();
    let template = fs::read_dir(dir.path().join("pages"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".md"))
        .min()
        .unwrap();
    // 64 mounted queries over real, linked pages of the graph.
    let mounted: Vec<(String, Query)> = names
        .iter()
        .step_by((names.len() / 64).max(1))
        .take(64)
        .enumerate()
        .map(|(i, name)| {
            let source = if i % 2 == 0 {
                format!("[[{name}]]")
            } else {
                format!("(and (task TODO DONE) [[{name}]])")
            };
            (format!("mounted-{i}"), parse(&source))
        })
        .collect();
    for (id, query) in &mounted {
        mount(&store, id, query);
    }
    let rerun = |summary: &Value| -> (usize, Duration) {
        let view = store.whole_graph().unwrap();
        // A `full` summary may omit `queryTo`; it reruns everything anyway.
        let fired = if summary["queryTo"].is_array() {
            strings(&summary["queryTo"])
        } else {
            Default::default()
        };
        let full = summary["full"] == true;
        let started = Instant::now();
        let mut count = 0;
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        for (id, query) in &mounted {
            if !full && !fired.contains(id.as_str()) {
                continue;
            }
            count += 1;
            let settings = tine_core::query::ir::ViewSettings {
                columns: vec![tine_core::query::ir::Field(format!(
                    "bench-{}",
                    RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ))],
                ..Default::default()
            };
            view.query_ir(
                crate::IrRequest::Run {
                    query,
                    view: &settings,
                    context: &Default::default(),
                },
                &crate::Cancel::default(),
            )
            .unwrap()
            .unwrap();
            // Re-register, as the frontend does at read.
            mount(&store, id, query);
        }
        (count, started.elapsed())
    };
    // Warm every mounted query once.
    rerun(&serde_json::json!({"full": true, "queryTo": []}));
    let doc = |raw: &str| {
        let mut doc = store
            .page(&PageId::from(format!("pages/{template}").as_str()))
            .unwrap()
            .doc;
        doc.pre_block = None;
        doc.blocks.truncate(1);
        doc.blocks[0].raw = raw.into();
        doc.blocks[0].children.clear();
        doc
    };
    let target = &names[names.len() / 3];
    let ops: Vec<(&str, Box<dyn Fn() -> Value>)> = vec![
        (
            "create",
            Box::new(|| {
                commit(
                    &store,
                    &[(
                        "pages/Bench Fresh.md",
                        doc(&format!("fresh links [[{target}]]")),
                        SaveBase::CreateNew,
                    )],
                )
            }),
        ),
        (
            "rename",
            Box::new(|| move_page(&store, "pages/Bench Fresh.md", "Bench Moved.md")),
        ),
        ("delete", Box::new(|| trash(&store, "pages/Bench Moved.md"))),
    ];
    let mut rows = Vec::new();
    for (label, op) in ops {
        let started = Instant::now();
        let wire = op();
        let publish = started.elapsed();
        let summary = &wire["summary"];
        assert!(summary.is_object(), "{label}: no published summary: {wire}");
        let (count, reread) = rerun(summary);
        rows.push(format!(
            "{label}: full={} publish={publish:?} mounted reruns={count}/64 rerun time={reread:?}",
            summary["full"]
        ));
    }
    eprintln!(
        "page lifecycle timings ({} pages):\n{}",
        names.len(),
        rows.join("\n")
    );
    store.close();
}

/// The native journey's graph (`scripts/e2e-page-lifecycle-summary.mjs`): a
/// task page that links Target is renamed, then deleted. The query over its
/// task and Target reruns; one over another task and page does not.
#[test]
fn moving_a_task_page_reruns_only_the_query_it_can_match() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    let dependent = ("dependent", parse("(and (task TODO) [[Target]])"));
    let unrelated = (
        "unrelated".to_owned(),
        parse("(and (task NOW) [[Unrelated]])"),
    );
    mount(&store, dependent.0, &dependent.1);
    mount(&store, &unrelated.0, &unrelated.1);
    let oracle = Oracle {
        dir,
        store,
        queries: vec![dependent],
        unrelated: vec![unrelated],
    };
    oracle.step("unrelated page", |o| {
        create(
            &o.store,
            "pages/Unrelated.md",
            None,
            "NOW unrelated body [[Unrelated]]",
        )
    });
    oracle.step("linker", |o| {
        create(
            &o.store,
            "pages/Linker.md",
            None,
            "TODO linker body [[Target]]",
        )
    });
    for (label, op) in [
        (
            "rename",
            Box::new(|o: &Oracle| move_page(&o.store, "pages/Linker.md", "Linker Renamed.md"))
                as Box<dyn Fn(&Oracle) -> Value>,
        ),
        (
            "delete",
            Box::new(|o: &Oracle| trash(&o.store, "pages/Linker Renamed.md")),
        ),
    ] {
        let (summary, unrelated) = oracle.step(label, |o| op(o));
        assert_eq!(summary["full"], false, "{label}: {summary}");
        assert!(
            strings(&summary["queryTo"]).contains("dependent"),
            "{label}: {summary}"
        );
        assert_eq!(unrelated, 0, "{label}: {summary}");
    }
    oracle.store.close();
}

/// Review F1: a lifecycle publication also brings or removes pages without a
/// file. A namespace child brings its parent (`Brand/Child` lists `Brand`), a
/// created page's references bring reference-only pages, and the last child
/// or referrer leaving removes them. A page query whose answer holds such a
/// row must be named (create, rename and delete), and stays precise.
#[test]
fn page_lifecycle_names_queries_over_fileless_pages_it_brings_or_removes() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    let queries = vec![
        ("brand-parent", tql("@page and name = 'Brand'")),
        ("kin-parent", tql("@page and name = 'Kin'")),
        ("newref-page", tql("@page and name = 'Newref'")),
    ];
    for (id, query) in &queries {
        mount(&store, id, query);
    }
    let unrelated = unrelated_queries(4);
    for (id, query) in &unrelated {
        mount(&store, id, query);
    }
    let oracle = Oracle {
        dir,
        store,
        queries,
        unrelated,
    };
    let precise = |label: &str, (summary, unrelated): (Value, usize), id: &str| {
        assert_eq!(summary["full"], false, "{label}: {summary}");
        assert_eq!(unrelated, 0, "{label}: unrelated queries reran: {summary}");
        assert!(
            strings(&summary["queryTo"]).contains(id),
            "{label}: {summary}"
        );
    };
    // Create: a namespace child brings its parent; a reference brings a page.
    precise(
        "create namespace child",
        oracle.step("create namespace child", |o| {
            create(
                &o.store,
                "pages/brand-child.md",
                Some("title:: Brand/Child"),
                "child body",
            )
        }),
        "brand-parent",
    );
    precise(
        "create referrer",
        oracle.step("create referrer", |o| {
            create(&o.store, "pages/Referrer.md", None, "mentions [[Newref]]")
        }),
        "newref-page",
    );
    // Rename: the namespace child leaves `Brand` for `Kin`.
    let label = "rename namespace child";
    let (summary, unrelated) = oracle.step(label, |o| {
        edit(&o.store, "pages/brand-child.md", |doc| {
            doc.pre_block = Some("title:: Kin/Child".into())
        })
    });
    precise(label, (summary.clone(), unrelated), "brand-parent");
    assert!(
        strings(&summary["queryTo"]).contains("kin-parent"),
        "{summary}"
    );
    // Delete: the last child and the only referrer leave.
    precise(
        "delete namespace child",
        oracle.step("delete namespace child", |o| {
            trash(&o.store, "pages/brand-child.md")
        }),
        "kin-parent",
    );
    precise(
        "delete referrer",
        oracle.step("delete referrer", |o| trash(&o.store, "pages/Referrer.md")),
        "newref-page",
    );
    oracle.store.close();
}

/// The same rule for an ordinary edit (found with F1; the gap predates this
/// lane): a reference that comes or goes can bring or remove a fileless page
/// and its namespace parent. Typing plain text adds no lookup.
#[test]
fn an_edit_that_adds_or_drops_a_reference_names_queries_over_its_fileless_pages() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    let queries = vec![
        ("newref-page", tql("@page and name = 'Newref'")),
        ("brand-parent", tql("@page and name = 'Brand'")),
    ];
    for (id, query) in &queries {
        mount(&store, id, query);
    }
    let unrelated = unrelated_queries(4);
    for (id, query) in &unrelated {
        mount(&store, id, query);
    }
    let oracle = Oracle {
        dir,
        store,
        queries,
        unrelated,
    };
    for (label, raw) in [
        ("edit adds references", "see [[Newref]] and [[Brand/X]]"),
        ("edit drops references", "see nothing"),
    ] {
        let (summary, unrelated) = oracle.step(label, |o| {
            edit(&o.store, "pages/A.md", |doc| doc.blocks[0].raw = raw.into())
        });
        assert_eq!(summary["full"], false, "{label}: {summary}");
        assert_eq!(unrelated, 0, "{label}: {summary}");
        for id in ["newref-page", "brand-parent"] {
            assert!(
                strings(&summary["queryTo"]).contains(id),
                "{label}: {id}: {summary}"
            );
        }
    }
    let before = crate::cost_counters::lifecycle_work();
    let (summary, _) = oracle.step("typing", |o| {
        edit(&o.store, "pages/A.md", |doc| {
            doc.blocks[0].raw = "see nothing yet".into()
        })
    });
    assert_eq!(crate::cost_counters::lifecycle_work(), before, "{summary}");
    assert!(
        !strings(&summary["queryTo"]).contains("brand-parent"),
        "{summary}"
    );
    oracle.store.close();
}

/// Review F2: a batch of page creations in one publication (an outside sync
/// or checkout, published by the watcher path) costs the precise summary
/// linear work, and a batch over the bound, which publishes `full`, none:
/// `full` is decided before any page is read. Before, each named path
/// rescanned every changed page (O(K^2)), the full batches included.
#[test]
fn page_lifecycle_batch_work_is_linear_and_nothing_over_the_bound() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    for (id, query) in dependent_queries() {
        mount(&store, id, &query);
    }
    mount(&store, "page-anchored", &tql("@page and name = 'Brand'"));
    let oracle = Oracle {
        dir,
        store,
        queries: Vec::new(),
        unrelated: Vec::new(),
    };
    let (dir, store) = (oracle.dir.path(), &oracle.store);
    let mut rows = Vec::new();
    for (round, k) in [8usize, 16, 40, 400].into_iter().enumerate() {
        // The mounted readers have run (as shown readers have), so their
        // fileless inputs are resident and the summary compares those rows.
        oracle.answers();
        let sub = store.subscribe();
        for i in 0..k {
            fs::write(
                dir.join(format!("pages/Batch {round}-{i}.md")),
                format!("- batch body {i}\n"),
            )
            .unwrap();
        }
        let before = crate::cost_counters::lifecycle_work();
        store.refresh(crate::Depth::Stamps).unwrap();
        let spent = crate::cost_counters::lifecycle_work() - before;
        crate::test_host_writer::HostWriter::for_store(store).settle();
        let mut summaries = Vec::new();
        while let Some(change) = sub.try_recv().unwrap() {
            summaries.push(serde_json::to_value(change).unwrap()["summary"].clone());
        }
        assert_eq!(
            summaries.len(),
            1,
            "{k} files, one publication: {summaries:?}"
        );
        rows.push((k, summaries[0]["full"] == true, spent));
    }
    eprintln!("page lifecycle batch work (files, full, probes): {rows:?}");
    let work = |k: usize| rows.iter().find(|row| row.0 == k).unwrap();
    for k in [8, 16] {
        assert!(!work(k).1, "{k} creations publish precise: {rows:?}");
        assert!(work(k).2 > 0, "{k} creations are counted: {rows:?}");
    }
    // Linear: a bounded number of probes per file (each file's page on both
    // sides, its name checked once, its name looked up as a fileless row on
    // both sides), so twice the files cost twice the probes.
    for k in [8, 16] {
        assert!(work(k).2 <= 4 * k as u64 + 2, "{k} creations: {rows:?}");
    }
    assert!(work(16).2 <= 2 * work(8).2 + 2, "{rows:?}");
    for k in [40, 400] {
        assert!(work(k).1, "{k} creations publish full: {rows:?}");
        assert_eq!(
            work(k).2,
            0,
            "{k} creations read no page for the summary: {rows:?}"
        );
    }
    store.close();
}
