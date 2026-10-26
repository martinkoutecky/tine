//! GX4 S1 soundness oracle: compare real immutable host answers, then require
//! an invalidation key for every changed answer. The oracle does no writes
//! outside its temporary graph; ordinary edits use the guarded transaction.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use tine_core::model::{BlockDto, PageDto, RefGroup};
use tine_core::refs::page_key;
use tine_store::{Area, Cancel, EditKind, IrAnswer, IrRequest, PageId, SaveBase, Store, TxOutcome};

const PERSISTED: &str = "00000000-0000-4000-8000-000000000001";
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

fn query_cases() -> Vec<(
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

fn fixture(blocks: usize) -> (tempfile::TempDir, Store) {
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
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    store.stop_watcher_for_tests();
    for &target in TARGETS {
        store.register_unlinked_reference(target.to_owned(), Some(target.to_owned()));
    }
    for (id, query, _) in query_cases() {
        store
            .register_query_subscription(
                id.into(),
                Some(tine_store::QuerySubscription::Query {
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
            Some(tine_store::QuerySubscription::Registry),
            None,
        )
        .unwrap();
    (dir, store)
}

fn commit(store: &Store, pages: &[(&str, PageDto, SaveBase)]) -> Value {
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    for (path, doc, base) in pages {
        tx.save_page(
            &[EditKind::ReplacePage, EditKind::CreatePage],
            &PageId::from(*path),
            base.clone(),
            doc,
        );
    }
    let outcome = tx.commit();
    let TxOutcome::Committed {
        change: Some(change),
        ..
    } = outcome
    else {
        panic!("expected publication: {outcome:?}")
    };
    serde_json::to_value(change).unwrap()
}

fn edit(store: &Store, path: &str, mutate: impl FnOnce(&mut PageDto)) -> Value {
    let mut read = store.page(&PageId::from(path)).unwrap();
    mutate(&mut read.doc);
    commit(store, &[(path, read.doc, SaveBase::Existing(read.rev))])
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

fn block_ids(blocks: &[BlockDto], ids: &mut BTreeSet<String>) {
    for block in blocks {
        ids.insert(block.id.clone());
        block_ids(&block.children, ids);
    }
}

#[derive(Default)]
struct Answers {
    days: Value,
    journals: Value,
    templates: Value,
    queries: BTreeMap<String, Value>,
    linked: BTreeMap<String, Value>,
    unlinked: BTreeMap<String, Value>,
    tagged: BTreeMap<String, Value>,
    icons: BTreeMap<String, Value>,
    blocks: BTreeMap<String, Value>,
    pages: BTreeMap<String, Value>,
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
    let view = store.whole_graph().unwrap();
    let mut out = Answers::default();
    out.days = json!(view
        .inventory(tine_store::InventoryScope::ReferencedJournals)
        .0
        .iter()
        .filter_map(|entry| entry.day.map(|day| day.0))
        .collect::<Vec<_>>());
    out.journals = serde_json::to_value(view.journal_content_days()).unwrap();
    out.templates = serde_json::to_value(view.templates()).unwrap();
    // A distinct presentation key forces a fresh evaluation rather than
    // validating the summary against the memo's own carried answer.
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    for (id, query, mut settings) in query_cases() {
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
    ] {
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
        let answer = view.blocks(std::slice::from_ref(&id)).unwrap();
        out.blocks
            .insert(id, serde_json::to_value(&answer[0]).unwrap());
    }
    out.referrers.insert(
        PERSISTED.to_owned(),
        groups(&view.block_referrers(PERSISTED).unwrap()),
    );
    for &target in TARGETS {
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

fn check(before: &Answers, after: &Answers, summary: &Value) -> Result<[usize; 8], String> {
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
    let _guard = crate::integration_lock();
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
    let wire = edit(&store, "journals/2026_10_22.md", |doc| {
        doc.blocks[0].raw = "".into()
    });
    assert_eq!(wire["summary"]["days"], true);
    assert_eq!(wire["summary"]["journals"], true);
    check(&previous, &answers(&store, dir.path()), &wire["summary"]).unwrap();
}

#[test]
fn change_summary_random_reader_oracle() {
    let _guard = crate::integration_lock();
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
                    store.refresh(tine_store::Depth::Stamps).unwrap();
                    serde_json::to_value(sub.try_recv().unwrap().expect("external publication"))
                        .unwrap()
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
                    let mut tx = store.transaction(Some(EditKind::RenamePage));
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
                            tine_store::TrashIf::Any,
                        );
                    }
                    let TxOutcome::Committed {
                        change: Some(change),
                        ..
                    } = tx.commit()
                    else {
                        panic!("lifecycle commit")
                    };
                    serde_json::to_value(change).unwrap()
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
    let _guard = crate::integration_lock();
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
            Some(tine_store::QuerySubscription::Query {
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
    let _guard = crate::integration_lock();
    let (_dir, store) = fixture(1);
    let source = tine_store::QuerySubscription::Source {
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
        .register_query_subscription(
            "unknown".into(),
            Some(tine_store::QuerySubscription::Full),
            None,
        )
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
                    Some(tine_store::QuerySubscription::Query {
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
            Some(tine_store::QuerySubscription::Registry),
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
    let _guard = crate::integration_lock();
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
    let _guard = crate::integration_lock();
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
            Some(tine_store::QuerySubscription::Unlinked {
                name: "Target".into(),
            }),
            &revoked,
        )
        .unwrap();
    store
        .register_reader_while_live(
            "query-binding".into(),
            Some(tine_store::QuerySubscription::Full),
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
            Some(tine_store::QuerySubscription::Unlinked {
                name: "Target".into(),
            }),
            &revoked,
        )
        .unwrap();
    store
        .register_reader_while_live(
            "query-late".into(),
            Some(tine_store::QuerySubscription::Full),
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
            Some(tine_store::QuerySubscription::Unlinked {
                name: "Target".into(),
            }),
            &dropped,
        )
        .unwrap();
    store
        .register_reader_while_live(
            "query-dropped".into(),
            Some(tine_store::QuerySubscription::Full),
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
fn change_summary_date_named_page_does_not_hide_mentions_of_a_real_journal() {
    let _guard = crate::integration_lock();
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
    assert!(strings(&wire["summary"]["textTo"]).contains("2026-10-22"));
    check(&before, &answers(&store, dir.path()), &wire["summary"]).unwrap();
    store.close();
}

#[test]
fn change_summary_bulk_rebuild_and_config_fallbacks() {
    let _guard = crate::integration_lock();
    let (dir, store) = fixture(1);
    let template = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    let paths: Vec<_> = (0..33).map(|i| format!("pages/Bulk{i}.md")).collect();
    let new: Vec<_> = paths
        .iter()
        .map(|path| (path.as_str(), template.clone(), SaveBase::CreateNew))
        .collect();
    assert_eq!(commit(&store, &new)["summary"]["full"], true);
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
        let wire = commit(&store, &edits);
        assert_eq!(wire["summary"]["full"], count > 32);
        assert_eq!(wire["inventoryChanged"], false);
    }
    let sub = store.subscribe();
    store.refresh(tine_store::Depth::Rebuild).unwrap();
    let rebuilt = serde_json::to_value(sub.try_recv().unwrap().unwrap()).unwrap();
    assert_eq!(rebuilt["summary"]["full"], true);
    fs::create_dir_all(dir.path().join("logseq")).unwrap();
    fs::write(
        dir.path().join("logseq/config.edn"),
        "{:preferred-format :markdown}\n",
    )
    .unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let config = serde_json::to_value(sub.try_recv().unwrap().unwrap()).unwrap();
    assert_eq!(config["summary"]["full"], true);
    assert!(
        config["rev"].as_str().unwrap().parse::<u64>().unwrap()
            > rebuilt["rev"].as_str().unwrap().parse::<u64>().unwrap()
    );
    store.close();
}
