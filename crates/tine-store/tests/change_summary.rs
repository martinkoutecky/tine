//! GX4 S1 soundness oracle: compare real immutable host answers, then require
//! an invalidation key for every changed answer. The oracle does no writes
//! outside its temporary graph; ordinary edits use the guarded transaction.
use crate::{Area, Cancel, EditKind, IrAnswer, IrRequest, PageId, SaveBase, Store};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use tine_core::model::{BlockDto, PageDto, RefGroup};
use tine_core::refs::page_key;

const PERSISTED: &str = "00000000-0000-4000-8000-000000000001";

fn mount_fileless_query(store: &Store, id: &str, source: &str) -> tine_core::query::ir::Query {
    use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
    let query = parse_query_pair(
        source,
        QueryTextDialect::MacroTql,
        &[],
        tine_core::query::registry::Registry::none(),
    )
    .query;
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
    query
}

fn fileless_query_names(
    store: &Store,
    query: &tine_core::query::ir::Query,
    presentation: &str,
) -> Vec<String> {
    let settings = tine_core::query::ir::ViewSettings {
        columns: vec![tine_core::query::ir::Field(presentation.into())],
        ..Default::default()
    };
    let IrAnswer::Result(result) = store
        .whole_graph()
        .unwrap()
        .query_ir(
            IrRequest::Run {
                query,
                view: &settings,
                context: &Default::default(),
            },
            &Cancel::default(),
        )
        .unwrap()
        .unwrap()
    else {
        panic!("query answer");
    };
    let tine_core::query::ir::QueryRows::Page { pages } = result.rows else {
        panic!("page rows");
    };
    pages.into_iter().map(|row| row.name).collect()
}

#[test]
fn review_fileless_query_rerun_does_not_return_old_memo() {
    let _guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(3);
    let query = mount_fileless_query(&store, "newref-page", "@page and name = 'Newref'");
    assert!(fileless_query_names(&store, &query, "mounted").is_empty());
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "see [[Newref]]".into()
    });
    let invalidated = wire["summary"]["full"] == true
        || strings(&wire["summary"]["queryTo"]).contains("newref-page");
    let rerun = fileless_query_names(&store, &query, "mounted");
    let fresh = fileless_query_names(&store, &query, "fresh");
    println!("F4 invalidated={invalidated}, same-key={rerun:?}, fresh={fresh:?}, wire={wire}");
    assert_eq!(fresh, vec!["Newref"]);
    assert_eq!(
        rerun,
        vec!["Newref"],
        "same presentation must show the new row"
    );
    assert!(invalidated, "the mounted query must rerun: {wire}");
    store.close();
}

#[test]
fn review_blank_namespace_child_starts_supplying_parent() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    let path = "pages/Brand%2FChild.md";
    fs::write(dir.path().join(path), "").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    crate::test_host_writer::HostWriter::for_store(&store).settle();
    let query = mount_fileless_query(&store, "brand-parent", "@page and name = 'Brand'");
    let before = fileless_query_names(&store, &query, "mounted");
    assert!(before.is_empty());
    let base = store.page(&PageId::from(path)).unwrap();
    let mut doc = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    doc.pre_block = None;
    doc.blocks.truncate(1);
    doc.blocks[0].raw = "child body".into();
    doc.blocks[0].children.clear();
    let wire = commit(&store, &[(path, doc, SaveBase::Existing(base.rev))]);
    let fresh = fileless_query_names(&store, &query, "fresh");
    let invalidated = wire["summary"]["full"] == true
        || strings(&wire["summary"]["queryTo"]).contains("brand-parent");
    let mounted = if invalidated {
        fileless_query_names(&store, &query, "mounted")
    } else {
        before
    };
    println!("F5 invalidated={invalidated}, mounted={mounted:?}, fresh={fresh:?}, wire={wire}");
    assert_eq!(fresh, vec!["Brand"]);
    assert_eq!(
        mounted,
        vec!["Brand"],
        "the mounted query must show the new parent: {wire}"
    );
    assert!(
        invalidated,
        "the new parent must refresh the mounted query: {wire}"
    );
    assert_eq!(wire["inventoryChanged"], true, "{wire}");
    store.close();
}

#[test]
fn fileless_pages_vanishing_rerun_the_queries_that_listed_them() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    let newref = mount_fileless_query(&store, "newref-page", "@page and name = 'Newref'");
    let brand = mount_fileless_query(&store, "brand-parent", "@page and name = 'Brand'");
    let path = "pages/Brand%2FChild.md";
    fs::write(dir.path().join(path), "- child body\n").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    crate::test_host_writer::HostWriter::for_store(&store).settle();
    edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "see [[Newref]]".into()
    });
    assert_eq!(
        fileless_query_names(&store, &newref, "mounted"),
        vec!["Newref"]
    );
    assert_eq!(
        fileless_query_names(&store, &brand, "mounted"),
        vec!["Brand"]
    );
    // The last reference goes: Newref stops existing.
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "no reference".into()
    });
    assert!(
        fileless_query_names(&store, &newref, "mounted").is_empty(),
        "same presentation must drop the vanished row: {wire}"
    );
    assert!(
        strings(&wire["summary"]["queryTo"]).contains("newref-page"),
        "{wire}"
    );
    assert_eq!(wire["summary"]["full"], false, "{wire}");
    assert_eq!(wire["inventoryChanged"], true, "{wire}");
    // The only child goes blank: Brand stops being a namespace parent.
    let wire = edit(&store, path, |doc| {
        doc.blocks.clear();
    });
    assert!(
        fileless_query_names(&store, &brand, "mounted").is_empty(),
        "same presentation must drop the vanished parent: {wire}"
    );
    assert!(
        strings(&wire["summary"]["queryTo"]).contains("brand-parent"),
        "{wire}"
    );
    assert_eq!(wire["summary"]["full"], false, "{wire}");
    assert_eq!(wire["inventoryChanged"], true, "{wire}");
    store.close();
}

#[test]
fn an_edit_that_changes_no_page_existence_drops_no_fileless_listing() {
    let _guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(3);
    let query = mount_fileless_query(&store, "newref-page", "@page and name = 'Newref'");
    assert!(fileless_query_names(&store, &query, "mounted").is_empty());
    let retained = store.whole_graph().unwrap().graph.query_memo_len();
    assert!(retained > 0, "the mounted answer is memoized");
    // An ordinary block edit, a reference to a page that has a file, and a
    // reference-only page the query cannot list: none can change its answer.
    for raw in ["unrelated writing", "see [[A]]", "see [[Other]]"] {
        let wire = edit(&store, "pages/B.md", |doc| {
            doc.blocks[0].raw = format!("{raw}\nid:: {PERSISTED}")
        });
        assert_eq!(wire["summary"]["full"], false, "{raw}: {wire}");
        assert!(
            !strings(&wire["summary"]["queryTo"]).contains("newref-page"),
            "{raw}: {wire}"
        );
        assert_eq!(
            store.whole_graph().unwrap().graph.query_memo_len(),
            retained,
            "{raw}: the carried answer survives"
        );
    }
    store.close();
}

#[test]
fn fileless_change_producer_probes_only_changed_names() {
    let _guard = crate::test_transactions::integration_lock();
    for unrelated in [8, 64] {
        let (dir, store) = fixture(3);
        let path = "pages/Brand%2FChild.md";
        fs::write(dir.path().join(path), "").unwrap();
        for i in 0..unrelated {
            fs::write(
                dir.path().join(format!("pages/Unrelated {i}.md")),
                "- unrelated body\n",
            )
            .unwrap();
        }
        store.refresh(crate::Depth::Stamps).unwrap();
        crate::test_host_writer::HostWriter::for_store(&store).settle();
        let brand = mount_fileless_query(&store, "brand-parent", "@page and name = 'Brand'");
        let newref = mount_fileless_query(&store, "newref-page", "@page and name = 'Newref'");
        assert!(fileless_query_names(&store, &brand, "mounted").is_empty());
        assert!(fileless_query_names(&store, &newref, "mounted").is_empty());
        // External batches publish on this thread, where lifecycle_work is
        // counted; guarded saves publish on the page-host worker instead.
        let publish = |path: &str, raw: &str| {
            fs::write(dir.path().join(path), raw).unwrap();
            let sub = store.subscribe();
            let before = crate::cost_counters::lifecycle_work();
            store.refresh(crate::Depth::Stamps).unwrap();
            let probes = crate::cost_counters::lifecycle_work() - before;
            crate::test_host_writer::HostWriter::for_store(&store).settle();
            let mut changes = Vec::new();
            while let Some(change) = sub.try_recv().unwrap() {
                changes.push(serde_json::to_value(change).unwrap());
            }
            assert_eq!(changes.len(), 1, "one external publication: {changes:?}");
            (changes.pop().unwrap(), probes)
        };
        let (wire, probes) = publish(path, "- child body\n");
        eprintln!("fileless producer: unrelated={unrelated} first child text probes={probes}");
        assert_eq!(probes, 2, "one parent looked up on both sides: {wire}");
        assert!(strings(&wire["summary"]["queryTo"]).contains("brand-parent"));
        assert_eq!(fileless_query_names(&store, &brand, "mounted"), ["Brand"]);
        for (label, changed_path, raw, id, query, expected) in [
            ("last child text", path, "", "brand-parent", &brand, vec![]),
            (
                "first reference",
                "pages/A.md",
                "- see [[Newref]]\n",
                "newref-page",
                &newref,
                vec!["Newref"],
            ),
            (
                "last reference",
                "pages/A.md",
                "- plain text\n",
                "newref-page",
                &newref,
                vec![],
            ),
        ] {
            let (wire, probes) = publish(changed_path, raw);
            eprintln!("fileless producer: unrelated={unrelated} {label} probes={probes}");
            assert_eq!(probes, 2, "{label}: {wire}");
            assert!(
                strings(&wire["summary"]["queryTo"]).contains(id),
                "{label}: {wire}"
            );
            assert_eq!(
                fileless_query_names(&store, query, "mounted"),
                expected,
                "{label}"
            );
        }
        assert_eq!(publish("pages/A.md", "- more plain text\n").1, 0);
        store.close();
    }
}

#[test]
fn fileless_listing_memos_refresh_for_a_full_edit_batch() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    let paths: Vec<_> = (0..33).map(|i| format!("pages/Batch {i}.md")).collect();
    for path in &paths {
        fs::write(dir.path().join(path), "- plain text\n").unwrap();
    }
    store.refresh(crate::Depth::Stamps).unwrap();
    crate::test_host_writer::HostWriter::for_store(&store).settle();
    let query = mount_fileless_query(&store, "newref-page", "@page and name = 'Newref'");
    assert!(fileless_query_names(&store, &query, "mounted").is_empty());
    let sub = store.subscribe();
    for path in &paths {
        fs::write(dir.path().join(path), "- see [[Newref]]\n").unwrap();
    }
    store.refresh(crate::Depth::Stamps).unwrap();
    crate::test_host_writer::HostWriter::for_store(&store).settle();
    let wire = serde_json::to_value(sub.try_recv().unwrap().expect("one batch")).unwrap();
    assert!(sub.try_recv().unwrap().is_none());
    assert_eq!(wire["summary"]["full"], true, "{wire}");
    assert_eq!(fileless_query_names(&store, &query, "mounted"), ["Newref"]);
    store.close();
}

const TARGETS: &[&str] = &[
    "A",
    "B",
    "Target",
    "Shortcut",
    "NS",
    "NS/Leaf",
    "Oct 22nd, 2026",
    "2026-10-22",
    "2026_10_22",
    "DayShortcut",
];

pub(super) fn query_cases() -> Vec<(
    &'static str,
    tine_core::query::ir::Query,
    tine_core::query::ir::ViewSettings,
)> {
    use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
    [
        (
            "simple",
            "(and (task TODO) [[Target]])",
            QueryTextDialect::MacroQuery,
        ),
        ("tql", "@page and name = 'A'", QueryTextDialect::MacroTql),
        (
            "advanced",
            "[:find (pull ?b [*]) :where [?b :block/marker _]]",
            QueryTextDialect::Advanced,
        ),
        (
            "links",
            "@page and near(name = 'Target', 1)",
            QueryTextDialect::MacroTql,
        ),
        (
            "tag-targets",
            "@page and used_as_tag = true",
            QueryTextDialect::MacroTql,
        ),
        (
            "registry-query",
            "(property status)",
            QueryTextDialect::MacroQuery,
        ),
        (
            "clock",
            "@block and scheduled >= today",
            QueryTextDialect::MacroTql,
        ),
    ]
    .into_iter()
    .map(|(id, source, dialect)| {
        let parsed = parse_query_pair(
            source,
            dialect,
            &[],
            tine_core::query::registry::Registry::none(),
        );
        let resolved = tine_core::query::resolve_for_execution(
            &parsed.query,
            &Default::default(),
            tine_core::date::JournalDate::today(),
        );
        assert!(
            !resolved.query().is_invalid(),
            "oracle query {id}: {:?}",
            resolved.query().diagnostics
        );
        (id, parsed.query, parsed.view)
    })
    .collect()
}

pub(super) fn fixture(blocks: usize) -> (tempfile::TempDir, std::sync::Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::create_dir_all(dir.path().join("journals")).unwrap();
    let body = (0..blocks)
        .map(|i| format!("- body {i}\n"))
        .collect::<String>();
    fs::write(dir.path().join("pages/A.md"), body).unwrap();
    fs::write(
        dir.path().join("pages/B.md"),
        format!("- parent\n  id:: {PERSISTED}\n  - child\n"),
    )
    .unwrap();
    fs::write(
        dir.path().join("pages/Target.md"),
        "alias:: Shortcut\nicon:: T\n\n- target\n",
    )
    .unwrap();
    fs::write(dir.path().join("pages/NS.md"), "- namespace\n").unwrap();
    fs::write(
        dir.path().join("journals/2026_10_22.md"),
        "alias:: DayShortcut\n\n- journal\n",
    )
    .unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    store.stop_watcher_for_tests();
    for &target in TARGETS {
        store.register_unlinked_reference(target.to_owned(), Some(target.to_owned()));
    }
    for (id, query, _) in query_cases() {
        store
            .register_query_subscription(
                id.into(),
                Some(crate::QuerySubscription::Query {
                    query,
                    context: Default::default(),
                }),
                None,
            )
            .unwrap();
    }
    store
        .register_query_subscription(
            "registry".into(),
            Some(crate::QuerySubscription::Registry),
            None,
        )
        .unwrap();
    (dir, store)
}

pub(super) fn apply_published(store: &std::sync::Arc<Store>, tx: crate::PagePlan<'_>) -> Value {
    let subscription = store.subscribe();
    let op = tx.plan().expect("planned");
    crate::test_host_writer::HostWriter::for_store(store)
        .apply(op)
        .expect("applied");
    // Published before the reply, page by page; a new hold's retired rows
    // publish too and name no file.
    let mut own = Vec::new();
    while let Some(change) = subscription.try_recv().unwrap() {
        if change.origin == crate::Origin::Own {
            own.push(serde_json::to_value(change).unwrap());
        } else {
            assert!(
                change.files.is_empty(),
                "no outside echo: {:?}",
                change.files
            );
        }
    }
    assert!(!own.is_empty(), "the operation published");
    crate::test_host_writer::merged_changes(own)
}

pub(super) fn commit(store: &std::sync::Arc<Store>, pages: &[(&str, PageDto, SaveBase)]) -> Value {
    let mut tx = store.page_plan(Some(EditKind::ReplacePage));
    for (path, doc, base) in pages {
        tx.save_page(
            &[EditKind::ReplacePage, EditKind::CreatePage],
            &PageId::from(*path),
            base.clone(),
            doc,
        );
    }
    apply_published(store, tx)
}

pub(super) fn edit(
    store: &std::sync::Arc<Store>,
    path: &str,
    mutate: impl FnOnce(&mut PageDto),
) -> Value {
    let mut read = store.page(&PageId::from(path)).unwrap();
    mutate(&mut read.doc);
    commit(store, &[(path, read.doc, SaveBase::Existing(read.rev))])
}

pub(super) fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn search_subscription_skips_unrelated_writes_and_tracks_both_families() {
    use tine_core::query::ir::FriendlyPageMatchScope;
    let _guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(60);
    for (id, source, scope) in [
        ("text", "needle", FriendlyPageMatchScope::Names),
        ("content", "needle", FriendlyPageMatchScope::Content),
        ("name", "/^A$/", FriendlyPageMatchScope::Names),
        ("regex", "/needle [0-9]+/", FriendlyPageMatchScope::Both),
    ] {
        let registration = store
            .register_query_subscription(
                id.into(),
                Some(crate::QuerySubscription::Search {
                    source: source.into(),
                    page_match_scope: scope,
                    transliteration: true,
                }),
                None,
            )
            .unwrap();
        assert_eq!(registration.mode, "narrow");
    }
    for i in 0..8 {
        let wire = edit(&store, "pages/B.md", |doc| {
            doc.blocks[0].raw = format!("unrelated writing {i}\nid:: {PERSISTED}");
        });
        let keys = strings(&wire["summary"]["queryTo"]);
        for key in ["text", "content", "name", "regex"] {
            assert!(!keys.contains(key), "unrelated writing refreshed {key}");
        }
    }
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "needle 12".into()
    });
    let keys = strings(&wire["summary"]["queryTo"]);
    for key in ["text", "content", "name", "regex"] {
        assert!(keys.contains(key), "addition missed {key}");
    }
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "removed search match".into()
    });
    let keys = strings(&wire["summary"]["queryTo"]);
    for key in ["text", "content", "name", "regex"] {
        assert!(keys.contains(key), "removal missed {key}");
    }
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.pre_block = Some("owner:: changed".into())
    });
    let keys = strings(&wire["summary"]["queryTo"]);
    assert!(
        keys.contains("name"),
        "a matching page's display properties changed"
    );
    assert!(!keys.contains("text"));
    assert!(!keys.contains("content"));
}

#[test]
fn search_subscription_tracks_virtual_names_from_properties_including_new_pinyin_keys() {
    use tine_core::query::ir::FriendlyPageMatchScope;
    let _guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(1);
    for source in ["hiddenneedle", "beijing"] {
        store
            .register_query_subscription(
                source.into(),
                Some(crate::QuerySubscription::Search {
                    source: source.into(),
                    page_match_scope: FriendlyPageMatchScope::Names,
                    transliteration: true,
                }),
                None,
            )
            .unwrap();
    }
    let wire = edit(&store, "pages/B.md", |doc| {
        doc.pre_block = Some("tags:: [[hiddenneedle]], [[北京]]".into());
    });
    assert_eq!(wire["summary"]["full"], false);
    let keys = strings(&wire["summary"]["queryTo"]);
    assert!(keys.contains("hiddenneedle"));
    assert!(keys.contains("beijing"));
    let wire = edit(&store, "pages/B.md", |doc| doc.pre_block = None);
    let keys = strings(&wire["summary"]["queryTo"]);
    assert!(keys.contains("hiddenneedle"));
    assert!(keys.contains("beijing"));
}

fn block_ids(blocks: &[BlockDto], ids: &mut BTreeSet<String>) {
    for block in blocks {
        ids.insert(block.id.clone());
        block_ids(&block.children, ids);
    }
}

#[derive(Default)]
pub(super) struct Answers {
    days: Value,
    journals: Value,
    templates: Value,
    pub(super) queries: BTreeMap<String, Value>,
    linked: BTreeMap<String, Value>,
    unlinked: BTreeMap<String, Value>,
    tagged: BTreeMap<String, Value>,
    icons: BTreeMap<String, Value>,
    pub(super) blocks: BTreeMap<String, Value>,
    pub(super) pages: BTreeMap<String, Value>,
    referrers: BTreeMap<String, Value>,
    tables: BTreeMap<String, Value>,
}

fn groups(groups: &[RefGroup]) -> Value {
    // Native answers have deterministic row order, but page load slot order
    // is not a semantic dependency. Canonicalize groups by source identity.
    let map: BTreeMap<_, _> = groups
        .iter()
        .map(|g| (page_key(&g.page), serde_json::to_value(g).unwrap()))
        .collect();
    serde_json::to_value(map).unwrap()
}

fn answers(store: &Store, root: &std::path::Path) -> Answers {
    answers_with(store, root, &[], &[], &[])
}

/// [`answers`] over extra reader targets, page paths and mounted queries
/// (`(id, query)`, registered under the same id).
pub(super) fn answers_with(
    store: &Store,
    root: &std::path::Path,
    extra_targets: &[&str],
    extra_paths: &[&str],
    extra_queries: &[(&str, tine_core::query::ir::Query)],
) -> Answers {
    let view = store.whole_graph().unwrap();
    let mut out = Answers::default();
    out.days = json!(view
        .inventory(crate::InventoryScope::ReferencedJournals)
        .0
        .iter()
        .filter_map(|entry| entry.day.map(|day| day.0))
        .collect::<Vec<_>>());
    out.journals = serde_json::to_value(view.journal_content_days()).unwrap();
    out.templates = serde_json::to_value(view.templates()).unwrap();
    // A distinct presentation key forces a fresh evaluation rather than
    // validating the summary against the memo's own carried answer.
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let extra = extra_queries
        .iter()
        .map(|(id, query)| (*id, query.clone(), Default::default()));
    for (id, query, mut settings) in query_cases().into_iter().chain(extra) {
        settings.columns = vec![tine_core::query::ir::Field(format!(
            "oracle-{}",
            RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))];
        let IrAnswer::Result(result) = view
            .query_ir(
                IrRequest::Run {
                    query: &query,
                    view: &settings,
                    context: &Default::default(),
                },
                &Cancel::default(),
            )
            .unwrap()
            .unwrap()
        else {
            panic!("query answer");
        };
        out.queries
            .insert(id.into(), serde_json::to_value(&result.rows).unwrap());
    }
    let IrAnswer::Registry(registry) = view
        .query_ir(IrRequest::Registry, &Cancel::default())
        .unwrap()
        .unwrap()
    else {
        panic!("registry answer");
    };
    out.queries.insert(
        "registry".into(),
        serde_json::to_value(registry.rows()).unwrap(),
    );
    let mut ids = BTreeSet::from([PERSISTED.to_owned()]);
    for path in [
        "pages/A.md",
        "pages/B.md",
        "pages/Target.md",
        "journals/2026_10_22.md",
        "pages/Life.md",
        "pages/Moved.md",
    ]
    .into_iter()
    .chain(extra_paths.iter().copied())
    {
        if !root.join(path).is_file() {
            continue;
        }
        if let Ok(read) = store.page(&PageId::from(path)) {
            block_ids(&read.doc.blocks, &mut ids);
            out.pages.insert(
                page_key(&read.doc.name),
                serde_json::to_value(&read.doc).unwrap(),
            );
        }
    }
    for id in ids {
        let answer = view.blocks(std::slice::from_ref(&id), true).unwrap();
        out.blocks
            .insert(id, serde_json::to_value(&answer[0]).unwrap());
    }
    out.referrers.insert(
        PERSISTED.to_owned(),
        groups(&view.block_referrers(PERSISTED).unwrap()),
    );
    for &target in TARGETS.iter().chain(extra_targets) {
        let key = page_key(target);
        out.linked
            .insert(key.clone(), groups(&view.backlinks(target).unwrap()));
        let (query, table_view) = tine_core::query::parse_query_text(
            &format!("(tag \"{target}\")"),
            tine_core::query::QueryDialect::Og,
            tine_core::date::JournalDate::today(),
        );
        let IrAnswer::Result(table) = view
            .query_ir(
                IrRequest::Run {
                    query: &query,
                    view: &table_view,
                    context: &Default::default(),
                },
                &Cancel::default(),
            )
            .unwrap()
            .unwrap()
        else {
            panic!("table answer")
        };
        let tine_core::query::ir::QueryRows::Block {
            groups: table_groups,
        } = table.rows
        else {
            panic!("tag table groups")
        };
        out.tables.insert(key.clone(), groups(&table_groups));
        out.unlinked.insert(
            key.clone(),
            groups(&view.unlinked_references(target).unwrap()),
        );
        let IrAnswer::Result(tagged) = view
            .query_ir(IrRequest::TaggedPages { name: target }, &Cancel::default())
            .unwrap()
            .unwrap()
        else {
            panic!("tagged result")
        };
        let tine_core::query::ir::QueryRows::Page { pages } = tagged.rows else {
            panic!("tagged page rows")
        };
        let rows: BTreeMap<_, _> = pages
            .into_iter()
            .map(|row| (page_key(&row.name), serde_json::to_value(row).unwrap()))
            .collect();
        out.tagged
            .insert(key.clone(), serde_json::to_value(rows).unwrap());
        out.icons.insert(
            key,
            serde_json::to_value(view.page_icons(&[target.to_owned()]).get(target)).unwrap(),
        );
    }
    out
}

fn changed_keys(
    before: &BTreeMap<String, Value>,
    after: &BTreeMap<String, Value>,
) -> BTreeSet<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

pub(super) fn check(
    before: &Answers,
    after: &Answers,
    summary: &Value,
) -> Result<[usize; 8], String> {
    if summary["full"] != true {
        for (key, was, now) in [
            ("days", &before.days, &after.days),
            ("journals", &before.journals, &after.journals),
            ("templates", &before.templates, &after.templates),
        ] {
            if was != now && summary[key] != true {
                return Err(key.into());
            }
        }
        let fired = strings(&summary["queryTo"]);
        for id in changed_keys(&before.queries, &after.queries) {
            if !fired.contains(&id) {
                return Err(format!("query:{id}"));
            }
        }
    }
    let counts = [
        changed_keys(&before.linked, &after.linked).len(),
        changed_keys(&before.unlinked, &after.unlinked).len(),
        changed_keys(&before.tagged, &after.tagged).len(),
        changed_keys(&before.icons, &after.icons).len(),
        changed_keys(&before.blocks, &after.blocks).len(),
        changed_keys(&before.pages, &after.pages).len(),
        changed_keys(&before.referrers, &after.referrers).len(),
        changed_keys(&before.tables, &after.tables).len(),
    ];
    if summary["full"] == true {
        return Ok(counts);
    }
    let pages = strings(&summary["pages"]);
    let blocks = strings(&summary["blocks"]);
    let links = strings(&summary["linksTo"]);
    let shown_changed = |old: Option<&Value>| {
        let old_groups: Vec<RefGroup> = old
            .unwrap_or(&json!({}))
            .as_object()
            .unwrap()
            .values()
            .map(|group| serde_json::from_value(group.clone()).unwrap())
            .collect();
        let mut shown = BTreeSet::new();
        let mut ancestor_source = false;
        for group in old_groups {
            block_ids(&group.blocks, &mut shown);
            // Existing breadcrumb DTOs expose text, not ancestor identities.
            // The frontend conservatively keys those ancestors by source page.
            ancestor_source |= pages.contains(&page_key(&group.page))
                && group
                    .blocks
                    .iter()
                    .any(|block| !block.breadcrumb.is_empty());
        }
        ancestor_source || !shown.is_disjoint(&blocks)
    };
    for (kind, old, new, targets) in [
        ("linked", &before.linked, &after.linked, links.clone()),
        (
            "table",
            &before.tables,
            &after.tables,
            strings(&summary["tagTableTo"]),
        ),
        (
            "referrers",
            &before.referrers,
            &after.referrers,
            strings(&summary["refsTo"]),
        ),
    ] {
        for target in changed_keys(old, new) {
            if !targets.contains(&target) {
                // Existing contributors' subtrees can change even if this block
                // has no own explicit refs (inherited refs / displayed children).
                if !shown_changed(old.get(&target)) {
                    return Err(format!("{kind}:{target}"));
                }
            }
        }
    }
    for target in changed_keys(&before.unlinked, &after.unlinked) {
        if !strings(&summary["textTo"]).contains(&target)
            && !shown_changed(before.unlinked.get(&target))
        {
            return Err(format!("unlinked:{target}"));
        }
    }
    for target in changed_keys(&before.tagged, &after.tagged) {
        if !strings(&summary["tagsTo"]).contains(&target) {
            return Err(format!("tagged:{target}"));
        }
    }
    for target in changed_keys(&before.icons, &after.icons) {
        if !summary["props"][&target]
            .as_array()
            .is_some_and(|keys| keys.iter().any(|key| key == "icon" || key == "tine.icon"))
        {
            return Err(format!("icon:{target}"));
        }
    }
    for id in changed_keys(&before.blocks, &after.blocks) {
        if !blocks.contains(&id) {
            return Err(format!("block:{id}"));
        }
    }
    for page in changed_keys(&before.pages, &after.pages) {
        if !pages.contains(&page) {
            return Err(format!("page:{page}"));
        }
    }
    Ok(counts)
}

#[test]
fn day_and_template_hints_are_edit_scoped_and_cover_removal() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    let mut previous = answers(&store, dir.path());
    for (raw, days, templates) in [
        ("plain unrelated text", false, false),
        ("[[2026-10-30]]", true, false),
        ("removed day", true, false),
        ("TODO planned\nSCHEDULED: <2026-10-30 Fri>", true, false),
        ("removed planning", true, false),
        ("a template\ntemplate:: Example", false, true),
        ("edited template body\ntemplate:: Example", false, true),
        ("removed template", false, true),
    ] {
        let wire = edit(&store, "pages/A.md", |doc| doc.blocks[0].raw = raw.into());
        let summary = &wire["summary"];
        assert_eq!(summary["full"], false);
        assert_eq!(summary["days"], days, "{raw}");
        assert_eq!(summary["journals"], false, "{raw}");
        assert_eq!(summary["templates"], templates, "{raw}");
        let next = answers(&store, dir.path());
        check(&previous, &next, summary).unwrap();
        for (key, was, now) in [
            ("days", &previous.days, &next.days),
            ("templates", &previous.templates, &next.templates),
        ] {
            if was != now {
                let mut omitted = summary.clone();
                omitted[key] = json!(false);
                assert_eq!(check(&previous, &next, &omitted), Err(key.into()));
            }
        }
        previous = next;
    }
    // Editing an existing journal's text alone leaves the feed's days and
    // Upcoming as they were (cost audit row 9, D-20261102-PERF): `journals`
    // fires for the calendar dots, `days` does not, so the Journals feed is
    // not re-read from its first day. A day reference or planning written in
    // a journal still fires `days`, on adding and on removing it.
    for (raw, days) in [
        ("", false),
        ("edited journal text", false),
        ("[[2026-10-31]]", true),
        ("reference removed", true),
        ("TODO planned\nSCHEDULED: <2026-10-31 Sat>", true),
        ("planning removed", true),
        ("plain again", false),
    ] {
        let wire = edit(&store, "journals/2026_10_22.md", |doc| {
            doc.blocks[0].raw = raw.into()
        });
        let summary = &wire["summary"];
        assert_eq!(summary["full"], false, "{raw}");
        assert_eq!(summary["days"], days, "{raw}");
        assert_eq!(summary["journals"], true, "{raw}");
        let next = answers(&store, dir.path());
        check(&previous, &next, summary).unwrap();
        previous = next;
    }
}

#[test]
fn change_summary_random_reader_oracle() {
    let _guard = crate::test_transactions::integration_lock();
    let (mut total, mut full, mut readers, mut precise_readers) = (0, 0, [0; 8], [0; 8]);
    let mut omitted = [0; 8];
    let mut query_omissions = 0;
    for seed in 1..=8u64 {
        let (dir, store) = fixture(3);
        let mut random = seed;
        for step in 0..64 {
            random = random
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let before = answers(&store, dir.path());
            let path = if random & 1 == 0 {
                "pages/A.md"
            } else {
                "pages/B.md"
            };
            let wire = match step {
                40 | 41 => edit(&store, "pages/A.md", |doc| {
                    doc.pre_block = Some(format!("alias:: Alias{step}\nicon:: {step}"))
                }),
                58 => {
                    let sub = store.subscribe();
                    fs::write(
                        dir.path().join("pages/A.md"),
                        format!("tags:: Target\n\n- External Target {seed}\n"),
                    )
                    .unwrap();
                    store.refresh(crate::Depth::Stamps).unwrap();
                    // A prior save may still own the page while it retires;
                    // refresh forwards that page's observation to its host.
                    crate::test_host_writer::HostWriter::for_store(&store).settle();
                    let change = sub
                        .try_recv()
                        .unwrap()
                        .unwrap_or_else(|| panic!("seed={seed} step={step}: external publication"));
                    serde_json::to_value(change).unwrap()
                }
                59 => {
                    let mut a = store.page(&PageId::from("pages/A.md")).unwrap();
                    let mut b = store.page(&PageId::from("pages/B.md")).unwrap();
                    a.doc.blocks[0].raw = format!("Multi [[Target]] {seed}");
                    b.doc.blocks[0].raw = format!("Multi Target {seed}\nid:: {PERSISTED}");
                    commit(
                        &store,
                        &[
                            ("pages/A.md", a.doc, SaveBase::Existing(a.rev)),
                            ("pages/B.md", b.doc, SaveBase::Existing(b.rev)),
                        ],
                    )
                }
                60 => {
                    let mut doc = store.page(&PageId::from("pages/A.md")).unwrap().doc;
                    doc.pre_block = None;
                    commit(&store, &[("pages/Life.md", doc, SaveBase::CreateNew)])
                }
                61 | 62 => {
                    let mut tx = store.page_plan(Some(EditKind::RenamePage));
                    if step == 61 {
                        let from = PageId::from("pages/Life.md");
                        tx.move_file(
                            &from.file(),
                            store.page(&from).unwrap().rev,
                            &store.file_id(Area::Pages, "Moved.md").unwrap(),
                            None,
                        );
                    } else {
                        let from = PageId::from("pages/Moved.md");
                        tx.trash(
                            &from.file(),
                            store.page(&from).unwrap().rev,
                            crate::TrashIf::Any,
                        );
                    }
                    apply_published(&store, tx)
                }
                63 => edit(&store, "journals/2026_10_22.md", |doc| {
                    doc.blocks[0].raw = format!("Journal Target [[NS/Leaf]] {seed}")
                }),
                _ => edit(&store, path, |doc| {
                    let suffix = format!("{seed}:{step}");
                    let original_raw = doc.blocks[0].raw.clone();
                    match random % 16 {
                        0 => doc.blocks[0].raw = format!("Target plain {suffix}"),
                        1 => doc.blocks[0].raw = format!("[[Target]] linked {suffix}"),
                        2 => doc.blocks[0].raw = format!("no references {suffix}"),
                        3 => doc.blocks[0].raw = format!("[[NS/Leaf]] {suffix}"),
                        4 => doc.blocks[0].raw = format!("[[2026-10-22]] {suffix}"),
                        5 => doc.blocks[0].raw = format!("#Target {suffix}"),
                        6 => doc.pre_block = Some(format!("icon:: {suffix}\ntags:: Target")),
                        7 => {
                            doc.pre_block = Some(format!(
                                "tags:: {}\nnote:: {suffix}",
                                if step % 2 == 0 {
                                    "Target, NS"
                                } else {
                                    "Shortcut"
                                }
                            ))
                        }
                        8 => {
                            let mut child = doc.blocks[0].clone();
                            child.id = format!("child-{suffix}");
                            child.raw = format!("Child Target {suffix}");
                            child.children.clear();
                            doc.blocks[0].children = vec![child];
                        }
                        9 => doc.blocks[0].raw = format!("(({PERSISTED})) {suffix}"),
                        10 => doc.blocks[0].raw = format!("TODO [[Target]] {suffix}"),
                        11 => doc.pre_block = Some(format!("status:: {suffix}")),
                        12 => {
                            doc.blocks[0].raw = if path == "pages/A.md" {
                                format!("TODO Clock {suffix}\nSCHEDULED: <2026-10-26 Mon>")
                            } else {
                                // Keep the persisted block-id fixture separate from
                                // the planning-line fixture's property ownership.
                                format!("TODO [[Target]] Clock sibling {suffix}")
                            }
                        }
                        13 => {
                            doc.pre_block =
                                Some(format!("note:: Cleared query properties {suffix}"));
                            doc.blocks[0].raw = format!("Cleared properties {suffix}");
                        }
                        14 => doc.blocks[0].raw = format!("Template {suffix}\ntemplate:: Example"),
                        _ => doc.blocks[0].raw = format!("[[DayShortcut]] {suffix}"),
                    }
                    // Keep the block resolve oracle live through text edits.
                    if path == "pages/B.md" && doc.blocks[0].raw != original_raw {
                        doc.blocks[0].raw.push_str(&format!("\nid:: {PERSISTED}"));
                    }
                }),
            };
            let after = answers(&store, dir.path());
            let summary = &wire["summary"];
            assert_eq!(summary["rev"], wire["rev"]);
            assert_eq!(summary["inventoryChanged"], wire["inventoryChanged"]);
            let counts = check(&before, &after, summary)
                .unwrap_or_else(|key| panic!("seed={seed} step={step} omitted {key}: {summary}"));
            // Create, rename (a move) and delete of an alias-free page are
            // precise, so the oracle above checks their evidence (row 3).
            if (60..=62).contains(&step) {
                assert_eq!(summary["full"], false, "seed={seed} step={step}: {summary}");
            }
            total += 1;
            if summary["full"] == true {
                full += 1;
            }
            for i in 0..8 {
                readers[i] += counts[i];
                if summary["full"] == false {
                    precise_readers[i] += counts[i];
                }
            }
            if summary["full"] == false {
                let mut mutant = summary.clone();
                mutant["queryTo"] = json!([]);
                if check(&before, &after, &mutant).is_err() {
                    query_omissions += 1;
                }
                for (i, field) in [
                    "pages",
                    "blocks",
                    "linksTo",
                    "props",
                    "textTo",
                    "tagsTo",
                    "refsTo",
                    "tagTableTo",
                ]
                .into_iter()
                .enumerate()
                {
                    let mut mutant = summary.clone();
                    mutant[field] = if field == "props" {
                        json!({})
                    } else {
                        json!([])
                    };
                    if check(&before, &after, &mutant).is_err() {
                        omitted[i] += 1;
                    }
                }
            }
            // Mutation check: removing all evidence must be detected for any
            // commit that changed a reader. This also catches an inert oracle.
            if counts.iter().any(|&count| count > 0) {
                let mut empty = summary.clone();
                empty["full"] = json!(false);
                empty["pages"] = json!([]);
                empty["blocks"] = json!([]);
                empty["linksTo"] = json!([]);
                empty["props"] = json!({});
                empty["textTo"] = json!([]);
                empty["tagsTo"] = json!([]);
                empty["refsTo"] = json!([]);
                empty["tagTableTo"] = json!([]);
                assert!(check(&before, &after, &empty).is_err());
            }
        }
        store.close();
    }
    assert!(
        query_omissions > 0,
        "the registered-query oracle must detect missing keys"
    );
    eprintln!("GX4 oracle: 8 sequences, {total} commits, full={full}; changed reader answers [linked, unlinked, tagged, icons, blocks, pages, referrers, tables]={readers:?}; precise={precise_readers:?}");
    eprintln!("GX4 field-omission mutants rejected [pages, blocks, linksTo, props, textTo, tagsTo, refsTo, tagTableTo]={omitted:?}");
    assert!(
        omitted.iter().all(|&count| count > 0),
        "each field must have a sensitive oracle"
    );
    assert!(full * 5 < total, "most publications must be precise");
    assert!(
        precise_readers.iter().all(|&count| count > 0),
        "each reader needs non-full evidence"
    );
}

#[test]
fn change_summary_instant_clock_query_fires_after_its_displayed_row_expires() {
    let _guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(1);
    let timestamp = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    };
    let created = timestamp() + 1000;
    edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = format!("clock row\ncreated-at:: {created}");
    });
    let parsed = tine_core::query::wire_parse::parse_query_pair(
        "(between created-at now +1d)",
        tine_core::query::wire_parse::QueryTextDialect::MacroQuery,
        &[],
        tine_core::query::registry::Registry::none(),
    );
    store
        .register_query_subscription(
            "instant".into(),
            Some(crate::QuerySubscription::Query {
                query: parsed.query.clone(),
                context: Default::default(),
            }),
            None,
        )
        .unwrap();
    let count = || {
        let IrAnswer::Result(result) = store
            .whole_graph()
            .unwrap()
            .query_ir(
                IrRequest::Run {
                    query: &parsed.query,
                    view: &parsed.view,
                    context: &Default::default(),
                },
                &Cancel::default(),
            )
            .unwrap()
            .unwrap()
        else {
            panic!("query answer")
        };
        result.total
    };
    assert_eq!(count(), 1, "the displayed row starts ahead of now");
    while timestamp() <= created {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = format!("edited expired row\ncreated-at:: {created}");
    });
    assert_eq!(count(), 0);
    assert_eq!(wire["summary"]["full"], false);
    assert!(strings(&wire["summary"]["queryTo"]).contains("instant"), "instant-dependent displayed answers must be invalidated even when both current touches reject the row");
}

#[test]
fn change_summary_registered_query_kinds_and_precise_global_inputs() {
    let _guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(1);
    let source = crate::QuerySubscription::Source {
        source: "@page and name = 'A'".into(),
        dialect: tine_core::query::wire_parse::QueryTextDialect::MacroTql,
    };
    assert_eq!(
        store
            .register_query_subscription("workspace".into(), Some(source), None)
            .unwrap()
            .mode,
        "narrow"
    );
    store
        .register_query_subscription("unknown".into(), Some(crate::QuerySubscription::Full), None)
        .unwrap();
    let wire = edit(&store, "pages/B.md", |doc| {
        doc.blocks[0].raw = format!("Unrelated plain body\nid:: {PERSISTED}")
    });
    let keys = strings(&wire["summary"]["queryTo"]);
    for key in [
        "simple",
        "advanced",
        "tql",
        "workspace",
        "tag-targets",
        "registry-query",
        "registry",
        "clock",
    ] {
        assert!(!keys.contains(key), "unrelated edit fired {key}");
    }
    assert!(keys.contains("links")); // conservative memo contract
    assert!(keys.contains("unknown"));
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "TODO [[Target]] matching body".into()
    });
    let keys = strings(&wire["summary"]["queryTo"]);
    for key in ["simple", "advanced", "tql", "workspace", "links"] {
        assert!(keys.contains(key), "matching edit missed {key}");
    }
    let wire = edit(&store, "pages/B.md", |doc| {
        doc.pre_block = Some("tags:: Target\nstatus:: active".into())
    });
    let keys = strings(&wire["summary"]["queryTo"]);
    for key in ["tag-targets", "registry-query", "registry"] {
        assert!(keys.contains(key), "global-input edit missed {key}");
    }
    // A scheduled read renews the dependency plan against the latest snapshot;
    // plain edits then regain the precise global-input classification.
    for (id, query, _) in query_cases() {
        if ["tag-targets", "registry-query"].contains(&id) {
            store
                .register_query_subscription(
                    id.into(),
                    Some(crate::QuerySubscription::Query {
                        query,
                        context: Default::default(),
                    }),
                    None,
                )
                .unwrap();
        }
    }
    store
        .register_query_subscription(
            "registry".into(),
            Some(crate::QuerySubscription::Registry),
            None,
        )
        .unwrap();
    store
        .register_query_subscription("simple".into(), None, None)
        .unwrap();
    let wire = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "TODO [[Target]] changed again".into()
    });
    assert!(!strings(&wire["summary"]["queryTo"]).contains("simple"));
    for id in ["tag-targets", "registry-query", "registry"] {
        assert!(
            !strings(&wire["summary"]["queryTo"]).contains(id),
            "plain edit fired renewed {id}"
        );
    }
}

#[test]
fn change_summary_wire_cost_is_the_changed_block_not_page_size() {
    let _guard = crate::test_transactions::integration_lock();
    let mut sizes = Vec::new();
    for blocks in [1, 60] {
        let (_dir, store) = fixture(blocks);
        // Unchanged references must not make a one-block edit's message grow.
        if blocks > 1 {
            edit(&store, "pages/A.md", |doc| {
                for (i, block) in doc.blocks.iter_mut().enumerate().skip(1) {
                    block.raw = format!("untouched [[Target{i}]]");
                }
            });
        }
        let wire = edit(&store, "pages/A.md", |doc| {
            doc.blocks[0].raw = "one Target keystroke".into()
        });
        assert_eq!(wire["summary"]["full"], false);
        assert_eq!(wire["summary"]["blocks"].as_array().unwrap().len(), 1);
        let summary_bytes = serde_json::to_vec(&wire["summary"]).unwrap().len();
        let answer_bytes = serde_json::to_vec(&wire).unwrap().len();
        eprintln!("GX4 wire: {blocks}-block page: summary={summary_bytes} bytes, answer envelope={answer_bytes} bytes");
        sizes.push(summary_bytes);
        store.close();
    }
    assert_eq!(sizes[0], sizes[1]);
    assert!(sizes[0] < 400);
}

#[test]
fn change_summary_registered_mentions_and_incoming_targets() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(3);
    // Compare actual answers on add, retained-text edit and removal, including
    // alias, namespace and journal spellings. Code is not a plain occurrence.
    for raw in [
        "Target added",
        "Shortcut edited",
        "NS/Leaf plain",
        "2026-10-22 date",
        "Oct 22nd, 2026 date",
        "2026_10_22 date",
        "`Target` code",
        "nothing",
    ] {
        let before = answers(&store, dir.path());
        let wire = edit(&store, "pages/A.md", |doc| doc.blocks[0].raw = raw.into());
        assert_eq!(wire["summary"]["full"], false);
        check(&before, &answers(&store, dir.path()), &wire["summary"]).unwrap();
    }
    // Own-page text is excluded, even while the section is mounted collapsed.
    let own = edit(&store, "pages/Target.md", |doc| {
        doc.blocks[0].raw = "Target Shortcut".into()
    });
    let text = strings(&own["summary"]["textTo"]);
    assert!(!text.contains("target") && !text.contains("shortcut"));
    // An untouched mention on an edited page must not run the open reader.
    edit(&store, "pages/A.md", |doc| {
        doc.blocks[1].raw = "Target untouched".into()
    });
    let unrelated = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "unrelated edit".into()
    });
    assert!(strings(&unrelated["summary"]["textTo"]).is_empty());
    for tags in [
        Some("tags:: Shortcut"),
        Some("tags:: Target\nicon:: changed"),
        Some("icon:: changed\ntags:: Target"),
        Some("tags:: NS/Leaf"),
        None,
    ] {
        let before = answers(&store, dir.path());
        let wire = edit(&store, "pages/A.md", |doc| {
            doc.pre_block = tags.map(str::to_owned)
        });
        check(&before, &answers(&store, dir.path()), &wire["summary"]).unwrap();
        assert!(!strings(&wire["summary"]["tagsTo"]).is_empty());
    }
    for raw in [
        format!("(({PERSISTED})) first"),
        format!("(({PERSISTED})) edited"),
        "removed".into(),
    ] {
        let before = answers(&store, dir.path());
        let wire = edit(&store, "pages/A.md", |doc| doc.blocks[0].raw = raw);
        check(&before, &answers(&store, dir.path()), &wire["summary"]).unwrap();
        assert!(strings(&wire["summary"]["refsTo"]).contains(PERSISTED));
    }
    // Independent mounted readers share a target; releasing one keeps the other.
    store.register_unlinked_reference("second".into(), Some("Target".into()));
    for &target in TARGETS {
        store.register_unlinked_reference(target.into(), None);
    }
    let remaining = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "Target still mounted".into()
    });
    assert_eq!(
        strings(&remaining["summary"]["textTo"]),
        BTreeSet::from(["target".into()])
    );
    store.register_unlinked_reference("second".into(), None);
    let released = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "Target no reader".into()
    });
    assert!(strings(&released["summary"]["textTo"]).is_empty());
    let revoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    store
        .register_reader_while_live(
            "binding".into(),
            Some(crate::QuerySubscription::Unlinked {
                name: "Target".into(),
            }),
            &revoked,
        )
        .unwrap();
    store
        .register_reader_while_live(
            "query-binding".into(),
            Some(crate::QuerySubscription::Full),
            &revoked,
        )
        .unwrap();
    revoked.store(true, std::sync::atomic::Ordering::Release);
    let retired = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "Target retired binding".into()
    });
    assert!(strings(&retired["summary"]["textTo"]).is_empty());
    assert!(!strings(&retired["summary"]["queryTo"]).contains("query-binding"));
    store
        .register_reader_while_live(
            "late".into(),
            Some(crate::QuerySubscription::Unlinked {
                name: "Target".into(),
            }),
            &revoked,
        )
        .unwrap();
    store
        .register_reader_while_live(
            "query-late".into(),
            Some(crate::QuerySubscription::Full),
            &revoked,
        )
        .unwrap();
    let late = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "Target late registration".into()
    });
    assert!(strings(&late["summary"]["textTo"]).is_empty());
    assert!(!strings(&late["summary"]["queryTo"]).contains("query-late"));
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    store
        .register_reader_while_live(
            "dropped".into(),
            Some(crate::QuerySubscription::Unlinked {
                name: "Target".into(),
            }),
            &dropped,
        )
        .unwrap();
    store
        .register_reader_while_live(
            "query-dropped".into(),
            Some(crate::QuerySubscription::Full),
            &dropped,
        )
        .unwrap();
    drop(dropped);
    let dropped = edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "Target dropped binding".into()
    });
    assert!(strings(&dropped["summary"]["textTo"]).is_empty());
    assert!(!strings(&dropped["summary"]["queryTo"]).contains("query-dropped"));
    store.close();
}

#[test]
fn change_summary_date_named_page_keeps_its_literal_self_exclusion() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(1);
    let mut doc = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    doc.pre_block = None;
    doc.blocks[0].raw = "2026-10-22 mention".into();
    commit(&store, &[("pages/2026-10-22.md", doc, SaveBase::CreateNew)]);
    let before = answers(&store, dir.path());
    let wire = edit(&store, "pages/2026-10-22.md", |doc| {
        doc.blocks[0].raw = "2026-10-22 changed mention".into()
    });
    assert_eq!(wire["summary"]["full"], false);
    assert!(!strings(&wire["summary"]["textTo"]).contains("2026-10-22"));
    check(&before, &answers(&store, dir.path()), &wire["summary"]).unwrap();
    store.close();
}

#[test]
fn change_summary_bulk_rebuild_and_config_fallbacks() {
    let _guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture(1);
    let template = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    let paths: Vec<_> = (0..33).map(|i| format!("pages/Bulk{i}.md")).collect();
    let new: Vec<_> = paths
        .iter()
        .map(|path| (path.as_str(), template.clone(), SaveBase::CreateNew))
        .collect();
    // Page creates publish a precise summary (D-20261102-PERF row 3), and
    // the host publishes page by page; the bound is exercised below.
    assert_eq!(commit(&store, &new)["summary"]["full"], false);
    for count in [32, 33] {
        let edits: Vec<_> = paths
            .iter()
            .take(count)
            .map(|path| {
                let mut read = store.page(&PageId::from(path.as_str())).unwrap();
                read.doc.blocks[0].raw = format!("batch {count}");
                (path.as_str(), read.doc, SaveBase::Existing(read.rev))
            })
            .collect();
        // The host publishes page by page, so no own change reaches the bound.
        let wire = commit(&store, &edits);
        assert_eq!(wire["summary"]["full"], false);
        assert_eq!(wire["inventoryChanged"], false);
    }
    // The bound itself: one publication naming more pages than it.
    crate::test_host_writer::HostWriter::for_store(&store).settle();
    store.watch.stop_background_for_test();
    let sub = store.subscribe();
    for count in [32, 33] {
        for path in paths.iter().take(count) {
            fs::write(dir.path().join(path), format!("- outside {count}\n")).unwrap();
        }
        store.refresh(crate::Depth::Stamps).unwrap();
        let wire = serde_json::to_value(sub.try_recv().unwrap().unwrap()).unwrap();
        assert_eq!(wire["summary"]["full"], count > 32);
        assert_eq!(wire["inventoryChanged"], false);
    }
    store.refresh(crate::Depth::Rebuild).unwrap();
    let rebuilt = serde_json::to_value(sub.try_recv().unwrap().unwrap()).unwrap();
    assert_eq!(rebuilt["summary"]["full"], true);
    fs::create_dir_all(dir.path().join("logseq")).unwrap();
    // A setting no index reads (outside `checkpoint::config_key`) rebuilds
    // nothing, so it invalidates no answer; a build-relevant one is `full`.
    fs::write(dir.path().join("logseq/config.edn"), "{:start-of-week 0}\n").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let settings = serde_json::to_value(sub.try_recv().unwrap().unwrap()).unwrap();
    assert_eq!(settings["summary"]["full"], false);
    fs::write(
        dir.path().join("logseq/config.edn"),
        "{:start-of-week 0 :property-pages/excludelist #{:unused}}\n",
    )
    .unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let config = serde_json::to_value(sub.try_recv().unwrap().unwrap()).unwrap();
    assert_eq!(config["summary"]["full"], true);
    assert!(
        config["rev"].as_str().unwrap().parse::<u64>().unwrap()
            > rebuilt["rev"].as_str().unwrap().parse::<u64>().unwrap()
    );
    store.close();
}

/// CM step 2 batch 9 (S10, D-20261019-Q8): a reference query matches the
/// referenced page's whole alias group, so an `alias::` edit on a page the
/// answer never contained can change it. The mounted reader must fire and the
/// memoized answer must follow, without a restart.
#[test]
fn an_alias_edit_refreshes_a_reference_query_it_does_not_touch() {
    use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
    let _guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(1);
    edit(&store, "pages/A.md", |doc| {
        doc.blocks[0].raw = "see [[Target]]".into()
    });
    let parsed = parse_query_pair(
        "(and [[Extra]] (page \"A\"))",
        QueryTextDialect::MacroQuery,
        &[],
        tine_core::query::registry::Registry::none(),
    );
    let rows = |store: &Store| {
        let IrAnswer::Result(result) = store
            .whole_graph()
            .unwrap()
            .query_ir(
                IrRequest::Run {
                    query: &parsed.query,
                    view: &parsed.view,
                    context: &Default::default(),
                },
                &Cancel::default(),
            )
            .unwrap()
            .unwrap()
        else {
            panic!("query answer");
        };
        let tine_core::query::ir::QueryRows::Block { groups } = result.rows else {
            panic!("block rows");
        };
        groups
            .iter()
            .flat_map(|group| group.blocks.iter().map(|block| block.raw.clone()))
            .collect::<Vec<_>>()
    };
    assert!(rows(&store).is_empty(), "no page is named Extra yet");
    store
        .register_query_subscription(
            "extra".into(),
            Some(crate::QuerySubscription::Query {
                query: parsed.query.clone(),
                context: Default::default(),
            }),
            None,
        )
        .unwrap();
    let wire = edit(&store, "pages/Target.md", |doc| {
        doc.pre_block = Some("alias:: Shortcut, Extra\nicon:: T".into())
    });
    assert!(
        strings(&wire["summary"]["queryTo"]).contains("extra"),
        "the alias edit must refresh the mounted query: {}",
        wire["summary"]
    );
    assert_eq!(rows(&store), vec!["see [[Target]]".to_owned()]);
    let wire = edit(&store, "pages/Target.md", |doc| {
        doc.pre_block = Some("alias:: Shortcut\nicon:: T".into())
    });
    assert!(strings(&wire["summary"]["queryTo"]).contains("extra"));
    assert!(rows(&store).is_empty(), "removing the alias drops the row");
}
