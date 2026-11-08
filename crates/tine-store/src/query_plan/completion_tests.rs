use super::*;

pub(super) fn fingerprint(
    rows: Vec<(PageEntry, ObjectiveMatchClass)>,
) -> Vec<(
    String,
    String,
    std::path::PathBuf,
    PageKind,
    Option<i64>,
    ObjectiveMatchClass,
)> {
    rows.into_iter()
        .map(|(p, c)| {
            (
                p.name.clone(),
                p.rel_path_str().to_owned(),
                p.path,
                p.kind,
                p.date_key,
                c,
            )
        })
        .collect()
}

pub(super) fn oracle(
    plan: &QueryPlan,
    graph: &ReadSnapshot,
) -> Vec<(PageEntry, ObjectiveMatchClass)> {
    plan.execute(graph, || false)
        .hits
        .into_iter()
        .filter_map(|h| match h {
            QueryHit::Page {
                mut page,
                matched_alias,
                match_class,
                ..
            } => {
                if let Some(alias) = matched_alias {
                    page.name = alias;
                }
                Some((page, match_class))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn incremental_completion_equals_original_rank_for_random_unicode_names_and_queries() {
    let atoms = [
        "a", "B", "é", "e\u{301}", "İ", "Σ", "ς", "ß", "Ａ", "😀", "北", "京", "가", "ᄀ", "ᅡ", " ",
        "/", "-", "'", "1", "\u{315}",
    ];
    let mut seed = 0x543_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    for set in 0..3 {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::create_dir(root.path().join("journals")).unwrap();
        std::fs::write(
            root.path().join("journals/2026_11_08.md"),
            "alias:: [[Journal Alias]]\n- Journal body\n",
        )
        .unwrap();
        let mut names = vec![
            "北京".to_string(),
            "beijing".into(),
            "bjmarker".into(),
            "Café".into(),
            "Ａ".into(),
            "a/Child".into(),
        ];
        for n in 0..70 {
            let len = 1 + next() % 9;
            let mut name = (0..len)
                .map(|_| atoms[next() % atoms.len()])
                .collect::<String>();
            name.push_str(&format!(" {n}"));
            names.push(name);
        }
        for (n, name) in names.iter().enumerate() {
            // Filename namespace encoding follows the real file loader.
            let stem = name.replace('/', "___");
            let alias = &names[(n + 1) % names.len()];
            let extra = if n % 7 == 0 {
                format!(", [[{name} Alias]]")
            } else {
                String::new()
            };
            std::fs::write(
                root.path().join(format!("pages/{stem}.md")),
                format!("alias:: [[{alias}]], [[shared alias]]{extra}\n- [[Reference/{n}]] [[{name}]]\n"),
            )
            .unwrap();
        }
        let store = crate::Store::open(root.path(), Default::default())
            .unwrap()
            .0;
        let graph = store.whole_graph().unwrap().test_read_snapshot();
        let queries = names
            .iter()
            .cloned()
            .chain(
                names
                    .iter()
                    .enumerate()
                    .filter(|(n, _)| n % 7 == 0)
                    .map(|(_, name)| format!("{name} Alias")),
            )
            .chain(
                [
                    "a",
                    "é",
                    "e\u{301}",
                    "Σ",
                    "ΣΑ",
                    "가",
                    "Ａ",
                    "bj",
                    "bei jing",
                    "b-j",
                    "be'ijing",
                    "Reference/7",
                    "shared alias",
                    "not present",
                    "",
                    "a OR b",
                    "Nov",
                    "2026",
                    "08",
                    "Journal Alias",
                ]
                .into_iter()
                .map(str::to_owned),
            )
            .collect::<Vec<_>>();
        for accents in [false, true] {
            for transliteration in [false, true] {
                for query in &queries {
                    // Includes shortening, replacement, fold rewrites and repeated
                    // queries; a changing limit must not truncate the held pool.
                    let mut prefixes = vec![String::new()];
                    for c in query.chars() {
                        let mut q = prefixes.last().unwrap().clone();
                        q.push(c);
                        prefixes.push(q);
                    }
                    prefixes.push(query.clone());
                    prefixes.push("a".into());
                    for (step, q) in prefixes.iter().enumerate() {
                        let limit = [1, 7, 100][step % 3];
                        let mut plan = QueryPlan::legacy_page_search_with_policy(q, limit, accents)
                            .with_transliteration(transliteration);
                        plan.name_keys = Some(std::sync::Arc::clone(&graph.transliteration_names));
                        let actual = graph.completion_rank.execute(&plan, &graph, None);
                        assert_eq!(
                            fingerprint(actual),
                            fingerprint(oracle(&plan, &graph)),
                            "set={set} query={q:?} accents={accents} pinyin={transliteration}"
                        );
                    }
                }
            }
        }
        store.close();
    }
}

#[test]
fn extending_completion_only_ranks_previous_matches() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    for n in 0..1000 {
        let name = if n < 160 {
            format!("Needle {n}")
        } else {
            format!("Other {n}")
        };
        std::fs::write(root.path().join(format!("pages/{name}.md")), "- body\n").unwrap();
    }
    let store = std::sync::Arc::new(
        crate::Store::open(root.path(), Default::default())
            .unwrap()
            .0,
    );
    let writer = crate::test_host_writer::HostWriter::start(&store);
    let id = crate::PageId::from("pages/Other 999.md");
    let read = store.page(&id).unwrap();
    let block = read.doc.blocks[0].id.clone();
    let original = read.doc.blocks[0].raw.clone();
    let editor = Some(("Other 999", block.as_str(), original.as_str()));
    let view = store.whole_graph().unwrap();
    view.complete_page_names("N", 100, editor, false, Some("editor"));
    COMPLETION_SCORES.with(|n| n.set(0));
    let found = view.complete_page_names("Ne", 100, editor, false, Some("editor"));
    assert_eq!(found.len(), 100);
    let work = COMPLETION_SCORES.with(|n| n.get());
    assert!(work <= 160, "extension ranked {work} names: no rejected name should be revisited (160 previous matches)");
    let mut doc = read.doc;
    doc.blocks[0].raw = "[[New reference being authored]]".into();
    writer
        .save(
            crate::EditKind::ReplacePage,
            &id,
            crate::SaveBase::Existing(read.rev),
            &doc,
        )
        .unwrap();
    let after = store.whole_graph().unwrap();
    COMPLETION_SCORES.with(|n| n.set(0));
    after.complete_page_names("Nee", 100, editor, false, Some("editor"));
    let work = COMPLETION_SCORES.with(|n| n.get());
    assert!(
        work <= 160,
        "autosave lost the baseline-adjusted narrowing: {work} names ranked"
    );
    // This exact page was outside the first query's displayed 100. Retaining
    // only those rows would lose it when the longer query promotes it.
    let promoted = after.complete_page_names("Needle 159", 100, editor, false, Some("editor"));
    assert_eq!(promoted[0].0.name, "Needle 159");
    COMPLETION_SCORES.with(|n| n.set(0));
    after.complete_page_names("", 0, editor, false, Some("editor"));
    assert_eq!(
        COMPLETION_SCORES.with(|n| n.get()),
        0,
        "zero-limit lease release must not rank names"
    );
    assert_eq!(
        after
            .test_read_snapshot()
            .completion_rank
            .retained_candidates(),
        0
    );
    COMPLETION_SCORES.with(|n| n.set(0));
    after.complete_page_names("Need", 100, editor, false, Some("editor"));
    assert!(
        COMPLETION_SCORES.with(|n| n.get()) <= 1000,
        "reopening must do at most one full pass"
    );
    store.close();
}

#[test]
fn completion_publications_invalidate_changed_names_and_leave_held_views_exact() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    let a = root.path().join("pages/Needle.md");
    let b = root.path().join("pages/New name.md");
    std::fs::write(&a, "alias:: New alias\n- [[New parent/Child]]\n").unwrap();
    let store = crate::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    let mut held = Vec::new();
    for operation in 0..6 {
        let view = store.whole_graph().unwrap().test_read_snapshot();
        let plan = QueryPlan::legacy_page_search("N", 100);
        let answer = fingerprint(oracle(&plan, &view));
        assert_eq!(
            fingerprint(view.completion_rank.execute(&plan, &view, None)),
            answer
        );
        held.push((view, answer));
        match operation {
            0 => std::fs::write(&b, "- [[New parent/Other]]\n").unwrap(),
            1 => std::fs::rename(&b, root.path().join("pages/New renamed.md")).unwrap(),
            2 => std::fs::write(&a, "alias:: Newer alias\n- [[Newer parent/Child]]\n").unwrap(),
            3 => std::fs::remove_file(root.path().join("pages/New renamed.md")).unwrap(),
            4 => std::fs::write(&a, "- Body without refs\n").unwrap(),
            _ => {}
        }
        store.refresh(crate::Depth::Bytes).unwrap();
        for (view, expected) in &held {
            assert_eq!(
                fingerprint(view.completion_rank.execute(&plan, view, None)),
                *expected,
                "old views must keep their exact candidates"
            );
        }
    }
    // Close from a fresh Rescan generation, where no old pool was carried.
    store.refresh(crate::Depth::Rebuild).unwrap();
    store
        .whole_graph()
        .unwrap()
        .complete_page_names("", 0, None, false, None);
    for (view, _) in &held {
        assert_eq!(
            view.completion_rank.retained_candidates(),
            0,
            "close must free invalidated pools in held older snapshots too"
        );
    }
    store.close();
}
