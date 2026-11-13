use super::*;
use crate::model::ReadSnapshot;
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;

#[test]
fn literal_block_picker_keeps_its_existing_referenceable_block_surface() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(
        dir.path().join("pages/Host.md"),
        "category:: headerneedle\n\n- Bodyneedle\n  reviewer:: propertyneedle\n",
    )
    .unwrap();
    let store = crate::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    let graph = store.whole_graph().unwrap().test_read_snapshot();
    for term in ["headerneedle", "propertyneedle"] {
        let found =
            QueryPlan::block_search_literal_with_policy(term, 20, true).execute(&graph, || false);
        assert!(
            found.hits.is_empty(),
            "Ctrl+K properties must not introduce unreferenceable picker targets: {term}"
        );
        assert_eq!(
            QueryPlan::friendly(term, 0, 20)
                .execute(&graph, || false)
                .hits
                .len(),
            1
        );
    }
    assert_eq!(
        QueryPlan::block_search_literal_with_policy("Bodyneedle", 20, true)
            .execute(&graph, || false)
            .hits
            .len(),
        1
    );
    store.close();
}

#[test]
fn alias_completion_preserves_case_and_lists_every_matching_name() {
    let dir = crate::test_support::fresh_dir("tine-gh675-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(
        dir.join("pages/Tine.md"),
        "alias:: TineOutline, [[Tine Outliner]], SSE\n- body\n",
    )
    .unwrap();
    let graph = snapshot_for_dir(&dir);
    let entries = crate::query::quick_switch(&graph, "tin", 100);
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"Tine"), "{names:?}");
    assert!(names.contains(&"TineOutline"), "{names:?}");
    assert!(names.contains(&"Tine Outliner"), "{names:?}");
    assert!(entries.iter().all(|e| e.rel_path_str() == "pages/Tine.md"));
    assert_eq!(
        crate::query::quick_switch(&graph, "sse", 100)[0].name,
        "SSE"
    );
    assert_eq!(crate::query::quick_switch(&graph, "tin", 1).len(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn alias_completion_preserves_authored_case() {
    let dir = crate::test_support::fresh_dir("tine-gh675-case-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    for (name, content) in [
        ("Owner.md", "alias:: SSE\n- body\n"),
        ("Bullet.md", "- aliases:: BulletAlias\n- body\n"),
        ("Org.org", "#+ALIAS: OrgAlias\n* body\n"),
        ("Drawer.org", ":PROPERTIES:\n:alias: DrawerAlias\n:END:\n"),
    ] {
        fs::write(dir.join("pages").join(name), content).unwrap();
    }
    let graph = snapshot_for_dir(&dir);
    assert_eq!(
        crate::query::quick_switch(&graph, "sse", 100)[0].name,
        "SSE"
    );
    for name in ["BulletAlias", "OrgAlias", "DrawerAlias"] {
        assert_eq!(
            crate::query::quick_switch(&graph, &name.to_lowercase(), 100)[0].name,
            name
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pinyin_page_names_are_optional_and_rank_below_native_names() {
    let dir = crate::test_support::fresh_dir("tine-pinyin-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    for name in [
        "北京",
        "北景",
        "臺灣",
        "北京Notes",
        "東京",
        "beijing",
        "beijing notebook",
        "x beijing",
        "b_e_i_j_i_n_g",
    ] {
        fs::write(
            dir.join("pages").join(format!("{name}.md")),
            "- Han body 北京\n",
        )
        .unwrap();
    }
    fs::write(dir.join("pages/Owner.md"), "alias:: 南京\n- [[上海]]\n").unwrap();
    fs::write(dir.join("pages/Han body.md"), "- 北京\n").unwrap();
    let graph = snapshot_for_dir(&dir);
    let run = |query: &str, enabled| {
        QueryPlan::friendly(query, 100, 100)
            .with_transliteration(enabled)
            .execute(&graph, || false)
    };
    let off = page_hits_to_entries(run("beijing", false).hits);
    assert!(!off.iter().any(|page| page.name == "北京"));
    let on = run("beijing", true);
    let classes: Vec<_> = on
        .hits
        .iter()
        .filter_map(|hit| match hit {
            QueryHit::Page { match_class, .. } => Some(*match_class),
            _ => None,
        })
        .collect();
    assert_eq!(
        &classes[..4],
        &[
            ObjectiveMatchClass::Exact,
            ObjectiveMatchClass::Prefix,
            ObjectiveMatchClass::Substring,
            ObjectiveMatchClass::Fuzzy
        ]
    );
    assert!(classes[4..]
        .iter()
        .all(|class| *class == ObjectiveMatchClass::Transliteration));
    let names = |query| {
        page_hits_to_entries(run(query, true).hits)
            .into_iter()
            .map(|page| page.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names("beijing"), names("beijing"));
    let collisions = names("bj");
    assert!(
        collisions.iter().position(|n| n == "北京").unwrap()
            < collisions.iter().position(|n| n == "北景").unwrap()
    );
    assert!(names("taiwan").contains(&"臺灣".into()));
    assert!(names("beijingnotes").contains(&"北京Notes".into()));
    assert!(names("dongjing").contains(&"東京".into()));
    assert!(!names("tokyo").contains(&"東京".into()));
    assert!(run("nanjing", true).hits.iter().any(|hit| matches!(hit,
        QueryHit::Page { page, matched_alias: Some(alias), .. }
        if page.name == "Owner" && alias == "南京")));
    assert!(names("shanghai").contains(&"上海".into()));
    assert!(!names("bjng").contains(&"北京".into()));
    assert!(!names("/beijing/").contains(&"北京".into()));
    assert!(!names("bj -beijing").contains(&"北京".into()));
    let blocks = |enabled| {
        run("beijing", enabled)
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                QueryHit::Block { page, block, .. } => Some((page, block.id)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(true), blocks(false));
    assert!(
        !blocks(true).iter().any(|(page, _)| page == "Han body"),
        "Han-only body cannot match a Pinyin-only query"
    );
    assert!(
        names("beijing").contains(&"北京".into()),
        "positive page control"
    );
    assert!(names("北京").contains(&"北京".into()));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pinyin_or_preserves_a_later_native_match_class() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/北京.md"), "- body\n").unwrap();
    let graph = snapshot_for_dir(dir.path());
    let class = |enabled| {
        QueryPlan::friendly("beijing OR 北京", 10, 0)
            .with_transliteration(enabled)
            .execute(&graph, || false)
            .hits
            .into_iter()
            .find_map(|hit| match hit {
                QueryHit::Page {
                    page, match_class, ..
                } if page.name == "北京" => Some(match_class),
                _ => None,
            })
    };
    let native = class(false).expect("the native OR branch must match");
    assert_ne!(native, ObjectiveMatchClass::Transliteration);
    assert_eq!(class(true), Some(native));
}

#[test]
fn graph_search_obeys_og_accent_switch_on_real_pages_and_blocks() {
    let dir = crate::test_support::fresh_dir("tine-accent-search-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(dir.join("pages/Café.md"), "- café\n").unwrap();
    let original = fs::read(dir.join("pages/Café.md")).unwrap();
    let run = |snapshot: Arc<ReadSnapshot>, query: &str| {
        snapshot.run_graph_search_latest_scoped(
            &crate::store::Cancel(Arc::new(std::sync::atomic::AtomicBool::new(false))),
            query,
            8,
            8,
            None,
            false,
            FriendlyPageMatchScope::Names,
            None,
            None,
            false,
        )
    };
    let default = run(snapshot_for_dir(&dir), "cafe");
    assert!(default
        .hits
        .iter()
        .any(|hit| matches!(hit, QueryHit::Page { .. })));
    assert!(default
        .hits
        .iter()
        .any(|hit| matches!(hit, QueryHit::Block { .. })));
    fs::write(
        dir.join("logseq/config.edn"),
        "{:feature/enable-search-remove-accents? false}",
    )
    .unwrap();
    let off_snapshot = snapshot_for_dir(&dir);
    assert!(run(Arc::clone(&off_snapshot), "cafe").hits.is_empty());
    assert_eq!(run(off_snapshot, "café").hits.len(), 2);
    assert_eq!(fs::read(dir.join("pages/Café.md")).unwrap(), original);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn erased_accent_query_never_matches_the_whole_graph() {
    let (dir, graph) = fixture();
    let mark = "\u{301}";
    assert!(QueryPlan::friendly(mark, 8, 8)
        .execute(&graph, || false)
        .hits
        .is_empty());
    assert!(QueryPlan::friendly(&format!("{mark} OR foo"), 8, 8)
        .execute(&graph, || false)
        .hits
        .iter()
        .any(|hit| matches!(hit, QueryHit::Block { .. })));
    fs::remove_dir_all(dir).unwrap();
}
use std::sync::Arc;

fn text_pred(mode: TextMatchMode, value: &str) -> TextPredicate {
    TextPredicate {
        clause_id: 1,
        field: TextField::PageName,
        mode,
        value: value.into(),
    }
}

fn snapshot_for_dir(dir: &std::path::Path) -> Arc<ReadSnapshot> {
    let store = crate::store::Store::open(dir, Default::default())
        .unwrap()
        .0;
    store.whole_graph().unwrap().test_read_snapshot()
}

impl ReadSnapshot {
    fn run_graph_search(
        &self,
        source: &str,
        page_limit: usize,
        block_limit: usize,
        explain: bool,
    ) -> QueryExecution {
        QueryPlan::friendly(source, page_limit, block_limit).execute_with_explain(
            self,
            || false,
            explain,
        )
    }
}

fn fixture() -> (PathBuf, Arc<ReadSnapshot>) {
    let dir = crate::test_support::fresh_dir("tine-query-plan-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("pages").join("Opinion Diffusion.md"),
        "- Parent\n\t- 🧠 foo ready\n- foo draft\n- ready only\n- regex ABC\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Opdf Notes.md"),
        "alias:: Research Hub\n\n- unrelated\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Xopdf.md"), "- unrelated\n").unwrap();
    fs::write(
        dir.join("pages").join("References.md"),
        "- [[Virtual Opdf]]\n",
    )
    .unwrap();
    let graph = snapshot_for_dir(&dir);
    (dir, graph)
}

fn block_fingerprint(groups: Vec<tine_core::model::RefGroup>) -> Vec<(String, String)> {
    groups
        .into_iter()
        .flat_map(|group| {
            group
                .blocks
                .into_iter()
                .map(move |block| (group.page.clone(), block.raw))
        })
        .collect()
}

fn reference_literal_search(
    graph: &ReadSnapshot,
    query: &str,
    limit: usize,
) -> Vec<(String, String)> {
    if limit == 0 || query.is_empty() {
        return Vec::new();
    }
    let query = canonical_fold(query);
    let fragments: Vec<&str> = query.split_whitespace().collect();
    if fragments.is_empty() {
        return Vec::new();
    }
    // Independent oracle for OG's `LIKE '%a%b%'`: a regex built from the
    // escaped fragments, not the production greedy scan.
    let pattern = fragments
        .iter()
        .map(|fragment| regex::escape(fragment))
        .collect::<Vec<_>>()
        .join(".*");
    let regex = regex::Regex::new(&format!("(?s){pattern}")).unwrap();
    // Independent oracle for OG's FTS5 leg over a token vector: barewords
    // (ASCII alphanumerics, `_`, anything non-ASCII) are ANDed phrases;
    // any other character makes that MATCH a syntax error and OG falls
    // back to its quoted phrase over all tokens.
    fn tokens(text: &str) -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect()
    }
    let bare = fragments.iter().all(|p| {
        p.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii())
    });
    let phrases: Vec<Vec<String>> = if bare {
        fragments
            .iter()
            .map(|p| tokens(p))
            .filter(|t| !t.is_empty())
            .collect()
    } else {
        vec![tokens(&query)]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect()
    };
    let query = move |text: &str| {
        if regex.is_match(text) {
            return true;
        }
        let hay = tokens(text);
        !phrases.is_empty()
            && phrases
                .iter()
                .all(|ph| hay.windows(ph.len()).any(|w| w == ph.as_slice()))
    };
    let query: &dyn Fn(&str) -> bool = &query;
    graph.with_pages(|pages| {
        let mut out = Vec::new();
        fn visit(
            page: &str,
            blocks: &[DocBlock],
            query: &dyn Fn(&str) -> bool,
            remaining: &mut usize,
            out: &mut Vec<(String, String)>,
        ) {
            for block in blocks {
                if *remaining == 0 {
                    return;
                }
                let projection = block.projection();
                if query(block.visible_folded(true)) {
                    out.push((page.to_string(), block.raw().to_owned()));
                    *remaining -= 1;
                }
                visit(page, &block.children, query, remaining, out);
            }
        }
        let mut remaining = limit;
        for (entry, document) in pages {
            visit(
                &entry.name,
                &document.roots,
                query,
                &mut remaining,
                &mut out,
            );
            if remaining == 0 {
                break;
            }
        }
        out
    })
}

#[test]
fn friendly_simple_term_is_fuzzy_only_for_page_names() {
    let plan = QueryPlan::friendly("opdf", 8, 50);
    assert_eq!(plan.branches.len(), 2);
    assert!(matches!(
        &plan.branches[0].predicate,
        QueryExpr::Text(TextPredicate {
            field: TextField::PageName,
            mode: TextMatchMode::Fuzzy,
            value,
            ..
        }) if value == "opdf"
    ));
    assert!(matches!(
        &plan.branches[1].predicate,
        QueryExpr::Text(TextPredicate {
            field: TextField::VisibleContent,
            mode: TextMatchMode::Contains,
            ..
        })
    ));
}

#[test]
fn graph_search_reports_per_category_truncation() {
    let (dir, graph) = fixture();

    let page_truncated = graph.run_graph_search("opdf", 2, 1, false);
    assert!(page_truncated.has_more.pages);
    assert!(!page_truncated.has_more.blocks);

    let block_truncated = graph.run_graph_search("foo", 10, 1, false);
    assert!(!block_truncated.has_more.pages);
    assert!(block_truncated.has_more.blocks);

    let complete = graph.run_graph_search("foo", 10, 10, false);
    assert!(!complete.has_more.pages);
    assert!(!complete.has_more.blocks);

    fs::remove_dir_all(dir).unwrap();
}

// L7s3-02: a sorted search keeps every match until it truncates; only the
// rows it returns may pay for breadcrumb labels.
#[test]
fn sorted_search_builds_breadcrumbs_only_for_the_rows_it_returns() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    let deep: String = (0..40)
        .map(|depth| format!("{}- needle {depth}\n", "\t".repeat(depth)))
        .collect();
    fs::write(dir.path().join("pages/Deep.md"), deep).unwrap();
    let graph = snapshot_for_dir(dir.path());
    let view = ViewSettings {
        sort: vec![(tine_core::query::ir::Field::new("content"), SortDir::Asc)],
        ..Default::default()
    };
    crate::query::CRUMB_LINES.with(|count| count.set(0));
    let result = QueryPlan::block_search("needle", 1)
        .with_display(None, Some(view))
        .execute(&graph, || false);
    let QueryHit::Block { block, .. } = &result.hits[0] else {
        panic!("a block hit");
    };
    assert_eq!(
        crate::query::CRUMB_LINES.with(std::cell::Cell::get),
        block.breadcrumb.len(),
        "the 39 losers must not build labels"
    );
}

#[test]
fn explicit_fuzzy_subsequence_has_score_and_utf16_spans() {
    let plan = QueryPlan::page_name_fuzzy("of", 8);
    let pred = &plan.branches[0].predicate;
    let hit = eval_expr(&plan, pred, TextField::PageName, "🧠 Opinion Diffusion").unwrap();
    assert_eq!(hit.evidence[0].score, Some(100));
    assert_eq!(
        hit.evidence[0].spans,
        vec![
            MatchSpan { start: 3, end: 4 },
            MatchSpan { start: 13, end: 14 }
        ]
    );
}

#[test]
fn unicode_substring_and_regex_spans_use_utf16_units() {
    let contains = QueryPlan::page_name_fuzzy("foo", 8);
    let hit = eval_expr(
        &contains,
        &contains.branches[0].predicate,
        TextField::PageName,
        "🧠 foo",
    )
    .unwrap();
    assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 3, end: 6 }]);

    let regex_plan = QueryPlan::friendly("/foo/", 8, 8);
    let page = &regex_plan.branches[0].predicate;
    let hit = eval_expr(&regex_plan, page, TextField::PageName, "🧠 foo").unwrap();
    assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 3, end: 6 }]);
}

#[test]
fn canonical_unicode_matches_pages_aliases_blocks_and_original_utf16_spans() {
    let multiword = QueryPlan::friendly("Canonical page", 8, 8);
    let multiword_page = best_page_match(
        &multiword,
        &multiword.branches[0].predicate,
        "Canonical page",
        &[],
    )
    .unwrap();
    assert_eq!(multiword_page.1, ObjectiveMatchClass::Exact);

    let syntax = QueryPlan::friendly("foo -draft", 8, 8);
    assert!(best_page_match(&syntax, &syntax.branches[0].predicate, "foo -draft", &[],).is_none());
    assert!(best_page_match(&syntax, &syntax.branches[0].predicate, "foo ready", &[],).is_some());

    let page_plan = QueryPlan::page_name_fuzzy("Café", 8);
    let page_pred = &page_plan.branches[0].predicate;
    let page = best_page_match(&page_plan, page_pred, "Cafe\u{301}", &[]).unwrap();
    assert_eq!(page.1, ObjectiveMatchClass::Exact);
    let page_evidence =
        eval_expr(&page_plan, page_pred, TextField::PageName, "Cafe\u{301}").unwrap();
    assert_eq!(
        page_evidence.evidence[0].spans,
        vec![MatchSpan { start: 0, end: 5 }]
    );

    let alias_plan = QueryPlan::page_name_fuzzy("Résumé", 8);
    let alias = best_page_match(
        &alias_plan,
        &alias_plan.branches[0].predicate,
        "Canonical page",
        &["Re\u{301}sume\u{301}".into()],
    )
    .unwrap();
    assert_eq!(alias.1, ObjectiveMatchClass::Exact);
    assert_eq!(alias.3.as_deref(), Some("Re\u{301}sume\u{301}"));

    let block_plan = QueryPlan::block_search("Résumé", 8);
    let block_pred = &block_plan.branches[0].predicate;
    let original = "🧠 Re\u{301}sume\u{301}";
    let folded = canonical_fold(original);
    assert!(eval_expr_fast(
        &block_plan,
        block_pred,
        TextField::VisibleContent,
        original,
        &folded,
    ));
    let evidence = eval_expr(&block_plan, block_pred, TextField::VisibleContent, original).unwrap();
    assert_eq!(
        evidence.evidence[0].spans,
        vec![MatchSpan { start: 3, end: 11 }]
    );

    let hangul = QueryPlan::block_search("\u{ac00}", 8);
    let hit = eval_expr(
        &hangul,
        &hangul.branches[0].predicate,
        TextField::VisibleContent,
        "\u{1100}\u{1161}",
    )
    .unwrap();
    assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 0, end: 2 }]);

    let reordered = QueryPlan::block_search("è\u{315}", 8);
    let hit = eval_expr(
        &reordered,
        &reordered.branches[0].predicate,
        TextField::VisibleContent,
        "e\u{315}\u{300}",
    )
    .unwrap();
    assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 0, end: 3 }]);

    let negative = QueryPlan::block_search("cafe", 8);
    assert!(eval_expr(
        &negative,
        &negative.branches[0].predicate,
        TextField::VisibleContent,
        "café",
    )
    .is_some());

    let expansion = QueryPlan::block_search("i\u{307}", 8);
    let hit = eval_expr(
        &expansion,
        &expansion.branches[0].predicate,
        TextField::VisibleContent,
        "\u{130}",
    )
    .unwrap();
    assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 0, end: 1 }]);
}

#[test]
fn canonical_unicode_executes_through_real_page_alias_and_block_projections() {
    let dir = crate::test_support::fresh_dir("tine-query-plan-unicode-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("pages").join("Cafe\u{301}.md"),
        "alias:: Re\u{301}sume\u{301}\n\n- Re\u{301}sume\u{301}\n",
    )
    .unwrap();
    let graph = snapshot_for_dir(&dir);

    let page = QueryPlan::page_name_fuzzy("Café", 8).execute(&graph, || false);
    assert!(matches!(
        page.hits.first(),
        Some(QueryHit::Page {
            page,
            match_class: ObjectiveMatchClass::Exact,
            matched_alias: None,
            evidence,
            ..
        }) if page.name == "Cafe\u{301}"
            && evidence[0].spans == vec![MatchSpan { start: 0, end: 5 }]
    ));
    let alias = QueryPlan::page_name_fuzzy("Résumé", 8).execute(&graph, || false);
    assert!(
        matches!(
            alias.hits.first(),
            Some(QueryHit::Page {
                page,
                match_class: ObjectiveMatchClass::Exact,
                matched_alias: Some(name),
                ..
            }) if page.name == "Cafe\u{301}" && name == "résumé"
        ),
        "{:#?}",
        alias.hits
    );

    let blocks = QueryPlan::block_search("Résumé", 8).execute(&graph, || false);
    assert!(matches!(
        blocks.hits.first(),
        Some(QueryHit::Block { display_text, evidence, .. })
            if display_text == "Re\u{301}sume\u{301}"
                && evidence[0].spans == vec![MatchSpan { start: 0, end: 8 }]
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn page_alias_search_is_scoped_to_its_physical_owner() {
    let dir = crate::test_support::fresh_dir("tine-query-plan-alias-owner-");
    fs::create_dir_all(dir.join("pages").join("sub")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("pages").join("Foo.md"),
        "alias:: bar\n\n- declaring page\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("sub").join("Foo.md"),
        "- same-named sibling\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Unique.md"),
        "alias:: quux\n\n- unique alias owner\n",
    )
    .unwrap();
    let graph = snapshot_for_dir(&dir);

    let alias_hits = graph
        .run_graph_search("bar", 10, 0, false)
        .hits
        .into_iter()
        .filter_map(|hit| match hit {
            QueryHit::Page {
                page,
                match_class,
                matched_alias,
                ..
            } if page.rel_path.is_some() => {
                Some((page.rel_path_str().to_owned(), match_class, matched_alias))
            }
            QueryHit::Page { .. } => None,
            QueryHit::Block { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        alias_hits,
        vec![(
            "pages/Foo.md".to_string(),
            ObjectiveMatchClass::Exact,
            Some("bar".to_string()),
        )],
        "a duplicate-named sibling must not inherit another file's alias"
    );

    let unique_hits = graph
        .run_graph_search("quux", 10, 0, false)
        .hits
        .into_iter()
        .filter_map(|hit| match hit {
            QueryHit::Page {
                page,
                match_class,
                matched_alias,
                ..
            } if page.rel_path.is_some() => {
                Some((page.rel_path_str().to_owned(), match_class, matched_alias))
            }
            QueryHit::Page { .. } => None,
            QueryHit::Block { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unique_hits,
        vec![(
            "pages/Unique.md".to_string(),
            ObjectiveMatchClass::Exact,
            Some("quux".to_string()),
        )],
        "a unique page name must retain ordinary alias matching"
    );

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn page_content_scope_admits_physical_owners_and_keeps_names_default() {
    let dir = crate::test_support::fresh_dir("tine-page-content-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(dir.join("pages/Alpha.md"), "- hiddenneedle in a block\n").unwrap();
    fs::write(dir.join("pages/Hiddenneedle.md"), "- unrelated\n").unwrap();
    let graph = snapshot_for_dir(&dir);
    let names =
        QueryPlan::friendly_with_scope("hiddenneedle", 10, 0, true, FriendlyPageMatchScope::Names)
            .execute(&graph, || false);
    let content = QueryPlan::friendly_with_scope(
        "hiddenneedle",
        10,
        0,
        true,
        FriendlyPageMatchScope::Content,
    )
    .execute(&graph, || false);
    let both =
        QueryPlan::friendly_with_scope("hiddenneedle", 10, 0, true, FriendlyPageMatchScope::Both)
            .execute(&graph, || false);
    let names_of = |result: &QueryExecution| {
        result
            .hits
            .iter()
            .filter_map(|hit| match hit {
                QueryHit::Page { page, .. } => Some(page.name.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(names_of(&names), ["Hiddenneedle"]);
    assert_eq!(names_of(&content), ["Alpha"]);
    assert_eq!(names_of(&both).len(), 2);
    assert!(
        matches!(content.hits.first(), Some(QueryHit::Page { evidence, .. }) if evidence.iter().any(|item| item.field == TextField::VisibleContent))
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn boolean_evidence_uses_positive_terms_and_first_matching_or_branch() {
    let plan = QueryPlan::friendly("foo -draft OR ready", 8, 8);
    let blocks = &plan.branches[1].predicate;
    let first = eval_expr(&plan, blocks, TextField::VisibleContent, "foo ship").unwrap();
    assert_eq!(first.evidence.len(), 1);
    assert_eq!(first.evidence[0].mode, TextMatchMode::Contains);
    assert!(eval_expr(&plan, blocks, TextField::VisibleContent, "foo draft").is_none());
    let second = eval_expr(&plan, blocks, TextField::VisibleContent, "ready draft").unwrap();
    assert_eq!(second.evidence.len(), 1);
    assert_ne!(first.evidence[0].clause_id, second.evidence[0].clause_id);
}

#[test]
fn invalid_regex_is_a_rust_diagnostic_and_matches_nothing() {
    let plan = QueryPlan::friendly("/(unclosed/", 8, 8);
    assert!(plan.branches.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].code, "invalid_regex");
    assert_eq!(
        plan.diagnostics[0].span,
        Some(MatchSpan { start: 0, end: 11 })
    );
}

#[test]
fn zero_limit_friendly_plans_still_classify_saveable_sources() {
    let rust_only_invalid = QueryPlan::friendly("/(a)\\1/", 0, 0);
    assert!(rust_only_invalid.branches.is_empty());
    assert_eq!(
        rust_only_invalid
            .diagnostics
            .first()
            .map(|item| item.code.as_str()),
        Some("invalid_regex")
    );
    let valid = QueryPlan::friendly("alpha", 0, 0);
    assert!(!valid.branches.is_empty());
    assert!(valid.branches.iter().all(|branch| branch.limit == 0));
    let excluded = QueryPlan::friendly("-draft", 0, 0);
    assert!(excluded.diagnostics.is_empty() && excluded.branches.is_empty());
}

#[test]
fn pure_negation_and_empty_friendly_search_have_no_branches() {
    assert!(QueryPlan::friendly("-draft", 8, 8).branches.is_empty());
    assert!(QueryPlan::friendly("", 8, 8).branches.is_empty());
    assert_eq!(QueryPlan::legacy_page_search("-draft", 8).branches.len(), 1);
}

#[test]
fn matching_modes_are_explicit() {
    let pred = text_pred(TextMatchMode::Fuzzy, "abc");
    assert_eq!(pred.mode, TextMatchMode::Fuzzy);
    let plan = QueryPlan::friendly("\"exact phrase\"", 8, 8);
    assert!(matches!(
        &plan.branches[1].predicate,
        QueryExpr::Text(TextPredicate {
            mode: TextMatchMode::Phrase,
            ..
        })
    ));
}

#[test]
fn combined_execution_returns_typed_ranked_hits_and_exact_evidence_text() {
    let (dir, graph) = fixture();
    let pages = QueryPlan::page_name_fuzzy("opdf", 10).execute(&graph, || false);
    let names = pages
        .hits
        .iter()
        .filter_map(|hit| match hit {
            QueryHit::Page { page, .. } => Some(page.name.as_str()),
            QueryHit::Block { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(names.first().copied(), Some("Opdf Notes"));
    assert!(
        names.iter().position(|name| *name == "Xopdf")
            < names.iter().position(|name| *name == "Opinion Diffusion")
    );
    let file_hit = pages.hits.iter().find_map(|hit| match hit {
        QueryHit::Page { page, .. } if page.name == "Opdf Notes" => Some(page),
        _ => None,
    });
    assert_eq!(file_hit.unwrap().rel_path_str(), "pages/Opdf Notes.md");
    let virtual_hit = pages.hits.iter().find_map(|hit| match hit {
        QueryHit::Page { page, .. } if page.name == "Virtual Opdf" => Some(page),
        _ => None,
    });
    assert_eq!(virtual_hit.unwrap().rel_path, None);

    let execution = graph.run_graph_search("foo -draft OR ready", 10, 10, true);
    let blocks = execution
        .hits
        .iter()
        .filter_map(|hit| match hit {
            QueryHit::Block {
                block,
                display_text,
                evidence,
                ..
            } => Some((block, display_text, evidence)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 2);
    let nested = blocks
        .iter()
        .find(|(_, text, _)| text.as_str() == "🧠 foo ready")
        .unwrap();
    assert_eq!(nested.0.breadcrumb, vec!["Parent"]);
    assert_eq!(nested.2.len(), 1, "successful NOT has no positive evidence");
    assert_eq!(nested.2[0].spans, vec![MatchSpan { start: 3, end: 6 }]);
    assert!(!execution.explanation.branches.is_empty());
    let explanation = serde_json::to_string(&execution.explanation).unwrap();
    assert!(explanation.contains("Block text"));
    assert!(explanation.contains("contains “foo”"));
    assert!(!explanation.contains("PageName"));
    assert!(!explanation.contains("VisibleContent"));

    let no_explain = graph.run_graph_search("foo", 10, 10, false);
    assert!(no_explain.explanation.branches.is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn current_page_scope_is_block_only_and_path_authoritative() {
    let (dir, graph) = fixture();
    fs::create_dir_all(dir.join("pages").join("duplicate")).unwrap();
    fs::write(
        dir.join("pages")
            .join("duplicate")
            .join("Opinion Diffusion.md"),
        "- duplicate foo\n",
    )
    .unwrap();
    let _ = graph.with_pages(|pages| pages.len());

    let execution = QueryPlan::friendly_for_page(
        "foo",
        50,
        QueryPageScope {
            path: Some("pages/Opinion Diffusion.md".into()),
            name: "Opinion Diffusion".into(),
            page_kind: PageKind::Page,
        },
    )
    .execute(&graph, || false);
    assert!(!execution.hits.is_empty());
    assert!(execution.hits.iter().all(|hit| matches!(
        hit,
        QueryHit::Block { page, path, block, .. }
            if page == "Opinion Diffusion"
                && path == "pages/Opinion Diffusion.md"
                && block.raw != "duplicate foo"
    )));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn page_hits_expose_objective_classes_and_alias_evidence() {
    let (dir, graph) = fixture();
    let exact = QueryPlan::page_name_fuzzy("Opdf Notes", 10).execute(&graph, || false);
    assert!(matches!(
        exact.hits.first(),
        Some(QueryHit::Page {
            match_class: ObjectiveMatchClass::Exact,
            matched_alias: None,
            ..
        })
    ));

    let alias = QueryPlan::page_name_fuzzy("Research Hub", 10).execute(&graph, || false);
    let hit = alias
        .hits
        .iter()
        .find(|hit| matches!(hit, QueryHit::Page { page, .. } if page.name == "Opdf Notes"));
    assert!(matches!(
        hit,
        Some(QueryHit::Page {
            display_text,
            match_class: ObjectiveMatchClass::Exact,
            matched_alias: Some(matched_alias),
            ..
        }) if display_text == "research hub" && matched_alias == "research hub"
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn regex_evidence_is_authoritative_and_bounded_to_projected_text() {
    let (dir, graph) = fixture();
    let execution = graph.run_graph_search("/[A-Z]{3}/", 10, 10, true);
    let (text, evidence) = execution
        .hits
        .iter()
        .find_map(|hit| match hit {
            QueryHit::Block {
                display_text,
                evidence,
                ..
            } if display_text == "regex ABC" => Some((display_text, evidence)),
            _ => None,
        })
        .unwrap();
    assert_eq!(text, "regex ABC");
    assert_eq!(evidence[0].spans, vec![MatchSpan { start: 6, end: 9 }]);
    fs::remove_dir_all(dir).unwrap();
}

fn picker_blocks(graph: &ReadSnapshot, query: &str) -> Vec<String> {
    block_fingerprint(crate::query::search(graph, query, usize::MAX))
        .into_iter()
        .map(|(_, raw)| raw)
        .collect()
}

/// OG `search-blocks` (electron/search.cljs): the non-FTS leg is
/// `content LIKE '%frag1%frag2%'`, so letters of a fragment are never
/// scattered across words and fragments keep their order.
#[test]
fn block_picker_matches_literal_fragments_not_scattered_letters() {
    let (dir, graph) = fixture();
    // f-d-r occur in order inside "foo draft" but only scattered.
    assert!(picker_blocks(&graph, "fdr").is_empty());
    assert!(picker_blocks(&graph, "frd").is_empty());
    assert!(picker_blocks(&graph, "rdy").is_empty());
    // Fragments in order, each literal and case-insensitive.
    assert_eq!(picker_blocks(&graph, "fo dr"), ["foo draft"]);
    assert_eq!(picker_blocks(&graph, "FOO   Ready"), ["🧠 foo ready"]);
    // A fragment is not an anchor: it may start or end mid-word.
    assert_eq!(picker_blocks(&graph, "oo dra"), ["foo draft"]);
    // Order matters and fragments cannot overlap.
    assert!(picker_blocks(&graph, "foo oo ready only").is_empty());
    assert!(picker_blocks(&graph, "oo foo").is_empty());
    // Single fragment: contiguous substring, prefix hits rank first.
    assert_eq!(
        picker_blocks(&graph, "ready"),
        ["ready only", "🧠 foo ready"]
    );
    // Blank and whitespace-only input has no candidates.
    assert!(picker_blocks(&graph, "").is_empty());
    assert!(picker_blocks(&graph, "  \t ").is_empty());
    fs::remove_dir_all(dir).unwrap();
}

fn picker_graph(blocks: &[&str]) -> (PathBuf, Arc<ReadSnapshot>) {
    let dir = crate::test_support::fresh_dir("tine-picker-ft-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    let body: String = blocks.iter().map(|b| format!("- {b}\n")).collect();
    fs::write(dir.join("pages").join("Ft.md"), body).unwrap();
    let graph = snapshot_for_dir(&dir);
    (dir, graph)
}

/// OG `search-blocks` full-text leg (`content MATCH ?`, FTS5): whole words
/// in any order, so `draft foo` finds `foo draft`.
#[test]
fn block_picker_full_text_leg_matches_whole_words_in_any_order() {
    let (dir, graph) = fixture();
    assert_eq!(picker_blocks(&graph, "draft foo"), ["foo draft"]);
    assert_eq!(picker_blocks(&graph, "ready foo"), ["🧠 foo ready"]);
    assert_eq!(picker_blocks(&graph, "ONLY   Ready"), ["ready only"]);
    // Words are whole tokens: `dra` is not a word of "foo draft", and out of
    // order it is not an in-order fragment either.
    assert!(picker_blocks(&graph, "dra foo").is_empty());
    // The in-order fragment leg still lists what the word leg cannot.
    assert_eq!(picker_blocks(&graph, "fo dra"), ["foo draft"]);
    // Every word must be present.
    assert!(picker_blocks(&graph, "draft ready").is_empty());
    fs::remove_dir_all(dir).unwrap();
}

/// OG merges `[...FTS matches (bm25 order), ...LIKE matches]`, de-duplicated
/// and cut at the limit, so a word-leg block precedes a fragment-only block
/// even when the fragment-only block would rank better on its own.
#[test]
fn block_picker_lists_the_full_text_leg_before_fragment_only_blocks() {
    let (dir, graph) = picker_graph(&["foobar tail", "xx bar yy foo"]);
    // "foobar tail" holds `foo` then `bar` as pieces (fragment leg only, no
    // whole word) and starts with them; "xx bar yy foo" holds both whole words.
    assert_eq!(
        picker_blocks(&graph, "foo bar"),
        ["xx bar yy foo", "foobar tail"]
    );
    // The limit applies to the merged list: the word leg fills it first.
    let top1 = block_fingerprint(crate::query::search(&graph, "foo bar", 1));
    assert_eq!(top1[0].1, "xx bar yy foo");
    fs::remove_dir_all(dir).unwrap();
}

/// FTS5 barewords reject punctuation, so OG's first MATCH input errors and
/// only its quoted-phrase input (adjacent tokens, in order) runs.
#[test]
fn block_picker_full_text_leg_falls_back_to_a_phrase_when_the_query_has_punctuation() {
    let (dir, graph) = picker_graph(&["foo bar draft", "draft foo-bar", "bar draft foo"]);
    assert_eq!(picker_blocks(&graph, "foo-bar draft"), ["foo bar draft"]);
    // Without punctuation the words are free to appear in any order.
    let mut any_order = picker_blocks(&graph, "bar draft foo");
    any_order.sort();
    assert_eq!(
        any_order,
        ["bar draft foo", "draft foo-bar", "foo bar draft"]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn block_picker_full_text_words_fold_case_and_accents() {
    let (dir, graph) = picker_graph(&["Příliš žluťoučký kůň", "zelená louka"]);
    let run = |query: &str, remove_accents: bool| {
        QueryPlan::block_search_literal_with_policy(query, 10, remove_accents)
            .execute(&graph, || false)
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                QueryHit::Block { display_text, .. } => Some(display_text),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run("KUN prilis", true), ["Příliš žluťoučký kůň"]);
    assert_eq!(run("kůň příliš", false), ["Příliš žluťoučký kůň"]);
    assert!(run("kun prilis", false).is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn block_picker_fragments_fold_diacritics_and_case_for_czech_text() {
    let dir = crate::test_support::fresh_dir("tine-picker-cs-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(
        dir.join("pages").join("Cs.md"),
        "- Příliš žluťoučký kůň úpěl ďábelské ódy\n- Zelená louka\n",
    )
    .unwrap();
    let graph = snapshot_for_dir(&dir);
    let run = |query: &str, remove_accents: bool| {
        let plan = QueryPlan::block_search_literal_with_policy(query, 10, remove_accents);
        plan.execute(&graph, || false)
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                QueryHit::Block { display_text, .. } => Some(display_text),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let whole = "Příliš žluťoučký kůň úpěl ďábelské ódy";
    // Accent-insensitive policy: ASCII and accented queries both match;
    // case is folded for non-ASCII letters too (SQLite LIKE would not).
    assert_eq!(run("prilis KUN", true), [whole]);
    assert_eq!(run("PŘÍLIŠ ŽLUŤ", true), [whole]);
    assert_eq!(run("ĎÁBEL ódy", true), [whole]);
    // Accent-sensitive policy keeps the diacritics significant.
    assert_eq!(run("příliš kůň", false), [whole]);
    assert!(run("prilis kun", false).is_empty());
    // Scattered letters of one word never match under either policy.
    assert!(run("pls", true).is_empty());
    assert!(run("zlk", true).is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fragment_evidence_marks_each_fragment_in_original_utf16_offsets() {
    let dir = crate::test_support::fresh_dir("tine-picker-ev-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(dir.join("pages").join("Ev.md"), "- 😀 Foo bar BAZ qux\n").unwrap();
    let graph = snapshot_for_dir(&dir);
    let plan = QueryPlan::block_search_literal_with_policy("fo baz", 10, true);
    let hits = plan.execute(&graph, || false).hits;
    let QueryHit::Block {
        evidence,
        match_class,
        ..
    } = &hits[0]
    else {
        panic!("expected a block hit");
    };
    assert_eq!(*match_class, ObjectiveMatchClass::Substring);
    assert_eq!(evidence[0].mode, TextMatchMode::Fragments);
    assert_eq!(
        evidence[0].spans,
        vec![
            MatchSpan { start: 3, end: 5 },
            MatchSpan { start: 11, end: 14 }
        ]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn words_evidence_marks_each_word_group_in_original_utf16_offsets() {
    let (dir, graph) = picker_graph(&["😀 Foo bar BAZ qux"]);
    let plan = QueryPlan::block_search_literal_with_policy("baz FOO", 10, true);
    let hits = plan.execute(&graph, || false).hits;
    let QueryHit::Block {
        evidence,
        match_class,
        score,
        ..
    } = &hits[0]
    else {
        panic!("expected a block hit");
    };
    // The whole-word leg wins over the fragment leg, which cannot match
    // out of order; both words are reported, in text order.
    assert_eq!(*match_class, ObjectiveMatchClass::Substring);
    assert_eq!(*score, 10_000_000 + 4_000_000 + 100_000 - 3 - 18);
    assert_eq!(evidence[0].mode, TextMatchMode::Words);
    assert_eq!(
        evidence[0].spans,
        vec![
            MatchSpan { start: 3, end: 6 },
            MatchSpan { start: 11, end: 14 }
        ]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn literal_block_search_adapter_preserves_fragment_membership_and_ranked_topk() {
    let (dir, graph) = fixture();
    for query in [
        "",
        "foo",
        "foo ready",
        "fo re",
        "o o",
        "ready foo",
        "draft foo",
        "foo_ready",
        "foo-ready only",
        "ready's foo",
        "e e e",
        "foo OR ready",
        "foo -draft",
        "-draft",
        "\"foo ready\"",
        "/[A-Z]{3}/",
        "/(unclosed/",
    ] {
        let full = block_fingerprint(crate::query::search(&graph, query, usize::MAX));
        let mut full_membership = full.clone();
        full_membership.sort();
        let mut reference = reference_literal_search(&graph, query, usize::MAX);
        reference.sort();
        assert_eq!(full_membership, reference, "query={query:?}");
        for limit in [0, 1, 2, 20] {
            assert_eq!(
                block_fingerprint(crate::query::search(&graph, query, limit)),
                full.iter().take(limit).cloned().collect::<Vec<_>>(),
                "query={query:?} limit={limit}"
            );
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn query_hit_json_contract_uses_tagged_entities_and_utf16_evidence() {
    let hit = QueryHit::Page {
        row: None,
        page: PageEntry {
            name: "🧠 Foo".into(),
            kind: PageKind::Page,
            date_key: None,
            rel_path: Some("pages/Foo.md".into()),
            path: std::path::PathBuf::new(),
        },
        display_text: "🧠 Foo".into(),
        evidence: vec![MatchEvidence {
            clause_id: 7,
            field: TextField::PageName,
            mode: TextMatchMode::Fuzzy,
            spans: vec![MatchSpan { start: 3, end: 6 }],
            score: None,
        }],
        score: 1_000,
        match_class: ObjectiveMatchClass::Prefix,
        matched_alias: None,
    };
    assert_eq!(
        serde_json::to_value(hit).unwrap(),
        serde_json::json!({
            "entity": "page",
            "page": {
                "name": "🧠 Foo",
                "kind": "page",
                "date_key": null,
                "path": "pages/Foo.md"
            },
            "display_text": "🧠 Foo",
            "evidence": [{
                "clause_id": 7,
                "field": "page_name",
                "mode": "fuzzy",
                "spans": [{"start": 3, "end": 6}]
            }],
            "score": 1_000,
            "match_class": "prefix"
        })
    );
}

#[test]
fn cancellation_discards_all_partial_combined_results() {
    let (dir, graph) = fixture();
    let checks = Cell::new(0usize);
    let execution = QueryPlan::friendly("unrelated", 10, 10).execute(&graph, || {
        checks.set(checks.get() + 1);
        checks.get() > 3
    });
    assert!(execution.cancelled);
    assert!(execution.hits.is_empty());
    fs::remove_dir_all(dir).unwrap();
}
