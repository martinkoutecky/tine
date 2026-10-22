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
    let TxOutcome::Committed {
        change: Some(change),
        ..
    } = tx.commit()
    else {
        panic!("expected publication")
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
    linked: BTreeMap<String, Value>,
    unlinked: BTreeMap<String, Value>,
    tagged: BTreeMap<String, Value>,
    icons: BTreeMap<String, Value>,
    blocks: BTreeMap<String, Value>,
    pages: BTreeMap<String, Value>,
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
    for &target in TARGETS {
        let key = page_key(target);
        out.linked
            .insert(key.clone(), groups(&view.backlinks(target).unwrap()));
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

// Readers whose target is open need S2's host registrations for text:P and
// tags:P. S1's universal source-page keys are sufficient to conservatively
// invalidate those readers, without pretending explicit links cover prose.
fn changed_source_keys(before: &Value, after: &Value) -> BTreeSet<String> {
    let empty = serde_json::Map::new();
    let a = before.as_object().unwrap_or(&empty);
    let b = after.as_object().unwrap_or(&empty);
    a.keys()
        .chain(b.keys())
        .filter(|key| a.get(*key) != b.get(*key))
        .cloned()
        .collect()
}

fn check(before: &Answers, after: &Answers, summary: &Value) -> Result<[usize; 6], String> {
    let counts = [
        changed_keys(&before.linked, &after.linked).len(),
        changed_keys(&before.unlinked, &after.unlinked).len(),
        changed_keys(&before.tagged, &after.tagged).len(),
        changed_keys(&before.icons, &after.icons).len(),
        changed_keys(&before.blocks, &after.blocks).len(),
        changed_keys(&before.pages, &after.pages).len(),
    ];
    if summary["full"] == true {
        return Ok(counts);
    }
    let pages = strings(&summary["pages"]);
    let blocks = strings(&summary["blocks"]);
    let links = strings(&summary["linksTo"]);
    for target in changed_keys(&before.linked, &after.linked) {
        if !links.contains(&target) {
            // Existing contributors' subtrees can change even if this block
            // has no own explicit refs (inherited refs / displayed children).
            let old_groups: Vec<RefGroup> = before.linked[&target]
                .as_object()
                .unwrap()
                .values()
                .map(|group| serde_json::from_value(group.clone()).unwrap())
                .collect();
            let mut shown = BTreeSet::new();
            for group in old_groups {
                block_ids(&group.blocks, &mut shown);
            }
            if shown.is_disjoint(&blocks) {
                return Err(format!("linked:{target}"));
            }
        }
    }
    for target in changed_keys(&before.unlinked, &after.unlinked) {
        for source in changed_source_keys(&before.unlinked[&target], &after.unlinked[&target]) {
            if !pages.contains(&source) {
                return Err(format!("unlinked:{target} source:{source}"));
            }
        }
    }
    for target in changed_keys(&before.tagged, &after.tagged) {
        for source in changed_source_keys(&before.tagged[&target], &after.tagged[&target]) {
            if !pages.contains(&source) || !summary["props"][&source].is_array() {
                return Err(format!("tagged:{target} source:{source}"));
            }
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
fn change_summary_random_reader_oracle() {
    let _guard = crate::integration_lock();
    let (mut total, mut full, mut readers, mut precise_readers) = (0, 0, [0; 6], [0; 6]);
    let mut omitted = [0; 4];
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
                    match random % 11 {
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
                        _ => doc.blocks[0].raw = format!("[[DayShortcut]] {suffix}"),
                    }
                    // Keep the block resolve oracle live through text edits.
                    if path == "pages/B.md" {
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
            for i in 0..6 {
                readers[i] += counts[i];
                if summary["full"] == false {
                    precise_readers[i] += counts[i];
                }
            }
            if summary["full"] == false {
                for (i, field) in ["pages", "blocks", "linksTo", "props"]
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
                assert!(check(&before, &after, &empty).is_err());
            }
        }
        store.close();
    }
    eprintln!("GX4 oracle: 8 sequences, {total} commits, full={full}; changed reader answers [linked, unlinked, tagged, icons, blocks, pages]={readers:?}; precise={precise_readers:?}");
    eprintln!("GX4 field-omission mutants rejected [pages, blocks, linksTo, props]={omitted:?}");
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
            doc.blocks[0].raw = "one keystroke".into()
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
