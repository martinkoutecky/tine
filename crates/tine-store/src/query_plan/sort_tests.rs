//! L7s3-02: a sorted friendly search keeps only the best `limit` candidates.
//! The answer is the full sort's first `limit` rows; the work a candidate that
//! loses causes does not grow with the number of matches.
use super::*;
use crate::model::ReadSnapshot;
use std::sync::Arc;
use tine_core::query::ir::Field;

fn snapshot(pages: &[(String, String)]) -> (tempfile::TempDir, Arc<ReadSnapshot>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    for (name, text) in pages {
        std::fs::write(dir.path().join("pages").join(name), text).unwrap();
    }
    let store = crate::store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    (dir, store.whole_graph().unwrap().test_read_snapshot())
}

fn view(sort: &[(&str, SortDir)]) -> ViewSettings {
    ViewSettings {
        sort: sort
            .iter()
            .map(|(field, dir)| (Field::new(*field), *dir))
            .collect(),
        ..Default::default()
    }
}

fn search(
    graph: &Arc<ReadSnapshot>,
    limit: usize,
    sort: &[(&str, SortDir)],
) -> (Vec<QueryHit>, bool, (usize, usize)) {
    BLOCK_SELECTION.with(|c| c.set((0, 0)));
    let found = QueryPlan::block_search("needle", limit)
        .with_display(None, Some(view(sort)))
        .execute(graph, || false);
    let work = BLOCK_SELECTION.with(std::cell::Cell::get);
    (found.hits, found.has_more.blocks, work)
}

fn texts(hits: &[QueryHit]) -> Vec<String> {
    hits.iter()
        .map(|hit| match hit {
            QueryHit::Block { display_text, .. } => display_text.clone(),
            other => panic!("a block hit, got {other:?}"),
        })
        .collect()
}

const LIMIT: usize = 5;
const MATCHES: usize = 200;

// The first `LIMIT` blocks are the best by the sort; every later block loses to
// the worst retained one, so only `LIMIT` candidates are kept or admitted.
#[test]
fn sorted_search_keeps_and_admits_only_the_best_limit_candidates() {
    let mut text = String::new();
    for i in 0..MATCHES {
        let priority = if i < LIMIT { 'A' } else { 'B' };
        text.push_str(&format!("- [#{priority}] needle {i}\n"));
    }
    let (_dir, graph) = snapshot(&[("Many.md".into(), text)]);
    let (hits, has_more, (admitted, peak)) = search(&graph, LIMIT, &[("priority", SortDir::Asc)]);
    assert_eq!(
        texts(&hits),
        (0..LIMIT)
            .map(|i| format!("[#A] needle {i}"))
            .collect::<Vec<_>>()
    );
    assert!(has_more, "{MATCHES} matches exceed the limit of {LIMIT}");
    assert_eq!(peak, LIMIT, "no moment holds more than the limit");
    assert_eq!(admitted, LIMIT, "a losing candidate pays for nothing kept");
}

// The worst arrival order: every block beats the one before it, so every block is
// admitted, but the set held is still the limit.
#[test]
fn sorted_search_holds_at_most_the_limit_when_every_candidate_improves() {
    let text: String = (0..MATCHES)
        .map(|i| format!("- needle {i}\n  rank:: r{:03}\n", MATCHES - i))
        .collect();
    let (_dir, graph) = snapshot(&[("Many.md".into(), text)]);
    let (hits, has_more, (admitted, peak)) = search(&graph, LIMIT, &[("rank", SortDir::Asc)]);
    assert_eq!(
        texts(&hits)
            .iter()
            .map(|t| t.lines().next().unwrap().to_owned())
            .collect::<Vec<_>>(),
        (0..LIMIT)
            .map(|i| format!("needle {}", MATCHES - 1 - i))
            .collect::<Vec<_>>()
    );
    assert!(has_more);
    assert_eq!(peak, LIMIT);
    assert!(admitted <= MATCHES);
}

/// One block of the differential fixture: what its text says about its order.
struct Block {
    path: String,
    position: usize,
    priority: Option<char>,
    rank: String,
    text: String,
}

fn fixture() -> (Vec<(String, String)>, Vec<Block>) {
    let mut files = Vec::new();
    let mut blocks = Vec::new();
    let mut seed = 0x2545_f491_u32;
    let mut next = move |modulo: u32| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) % modulo
    };
    for page in 0..6 {
        let path = format!("P{page}.md");
        let mut text = String::new();
        for position in 0..25 {
            let priority = [Some('A'), Some('B'), Some('C'), None][next(4) as usize];
            let rank = format!("r{}", next(4));
            let matching = next(5) != 0;
            let label = format!(
                "{} {page}.{position}",
                if matching { "needle" } else { "hay" }
            );
            let line = match priority {
                Some(p) => format!("[#{p}] {label}"),
                None => label,
            };
            text.push_str(&format!("- {line}\n  rank:: {rank}\n"));
            if matching {
                blocks.push(Block {
                    path: path.clone(),
                    position,
                    priority,
                    rank,
                    text: line,
                });
            }
        }
        files.push((path, text));
    }
    (files, blocks)
}

// Top-K under a sort is the full sort's first K rows, whatever the limit, the
// direction, the number of sort fields or the ties between equal keys.
#[test]
fn sorted_search_equals_the_full_sort_truncated() {
    let (files, blocks) = fixture();
    let (_dir, graph) = snapshot(&files);
    let sorts: [&[(&str, SortDir)]; 4] = [
        &[("priority", SortDir::Asc)],
        &[("priority", SortDir::Desc)],
        &[("rank", SortDir::Asc), ("priority", SortDir::Desc)],
        &[("page", SortDir::Desc), ("rank", SortDir::Asc)],
    ];
    for sort in sorts {
        let (full, full_more, _) = search(&graph, 10_000, sort);
        assert!(!full_more);
        assert_eq!(full.len(), blocks.len(), "{sort:?}");
        for limit in [1, 2, 3, 7, 25, blocks.len() - 1, blocks.len(), 500] {
            let (hits, more, (_, peak)) = search(&graph, limit, sort);
            let want = limit.min(full.len());
            assert_eq!(
                format!("{hits:?}"),
                format!("{:?}", &full[..want]),
                "{sort:?} limit {limit}"
            );
            assert_eq!(more, full.len() > limit, "{sort:?} limit {limit}");
            assert!(peak <= limit, "{sort:?} limit {limit}: held {peak}");
        }
    }
}

// The full sort itself, against the fixture's own record of each block's keys.
#[test]
fn sorted_search_orders_by_the_authored_keys_then_path_then_position() {
    let (files, blocks) = fixture();
    let (_dir, graph) = snapshot(&files);
    let priority_key = |block: &Block| block.priority.map_or("Z".to_owned(), |p| p.to_string());
    let mut want: Vec<&Block> = blocks.iter().collect();
    want.sort_by(|a, b| {
        priority_key(a)
            .cmp(&priority_key(b))
            .then_with(|| (&a.path, a.position).cmp(&(&b.path, b.position)))
    });
    let (hits, ..) = search(&graph, 10_000, &[("priority", SortDir::Asc)]);
    assert_eq!(
        texts(&hits),
        want.iter().map(|b| b.text.clone()).collect::<Vec<_>>()
    );
    let mut want: Vec<&Block> = blocks.iter().collect();
    want.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then_with(|| priority_key(b).cmp(&priority_key(a)))
            .then_with(|| (&a.path, a.position).cmp(&(&b.path, b.position)))
    });
    let sort = [("rank", SortDir::Asc), ("priority", SortDir::Desc)];
    let (hits, ..) = search(&graph, 10_000, &sort);
    assert_eq!(
        texts(&hits),
        want.iter().map(|b| b.text.clone()).collect::<Vec<_>>()
    );
}

fn page_search(
    graph: &Arc<ReadSnapshot>,
    limit: usize,
    sort: &[(&str, SortDir)],
) -> (Vec<QueryHit>, bool, usize) {
    PAGE_SELECTION.with(|c| c.set((0, 0)));
    let found =
        QueryPlan::friendly_with_scope("needle", limit, 1, false, FriendlyPageMatchScope::Both)
            .with_display(Some(view(sort)), None)
            .execute(graph, || false);
    let peak = PAGE_SELECTION.with(|c| c.get().1);
    let pages = (found.hits.into_iter())
        .filter(|hit| matches!(hit, QueryHit::Page { .. }))
        .collect();
    (pages, found.has_more.pages, peak)
}

// Pages are candidates by name or by a matching block; both sort by the page's
// own keys, and equal keys fall back to the tie key.
fn page_fixture() -> Vec<(String, String)> {
    let mut files = Vec::new();
    for i in 0..40 {
        files.push((
            format!("Needle{i:02}.md"),
            format!("rank:: r{}\n\n- body {i}\n", i % 4),
        ));
    }
    for i in 0..10 {
        files.push((
            format!("Hay{i}.md"),
            format!("rank:: r{}\n\n- needle in {i}\n", (i + 1) % 4),
        ));
    }
    files
}

#[test]
fn sorted_page_search_equals_the_full_sort_truncated() {
    let (_dir, graph) = snapshot(&page_fixture());
    let sorts: [&[(&str, SortDir)]; 4] = [
        &[("rank", SortDir::Asc)],
        &[("rank", SortDir::Desc)],
        &[("page", SortDir::Desc)],
        &[("rank", SortDir::Asc), ("page", SortDir::Desc)],
    ];
    for sort in sorts {
        let (full, full_more, _) = page_search(&graph, 10_000, sort);
        assert!(!full_more);
        assert_eq!(full.len(), 50, "{sort:?}");
        for limit in [1, 2, 5, 13, 49, 50, 500] {
            let (hits, more, peak) = page_search(&graph, limit, sort);
            assert_eq!(
                format!("{hits:?}"),
                format!("{:?}", &full[..limit.min(full.len())]),
                "{sort:?} limit {limit}"
            );
            assert_eq!(more, full.len() > limit, "{sort:?} limit {limit}");
            assert!(peak <= limit, "{sort:?} limit {limit}: held {peak}");
        }
    }
}

// The first rows, against the fixture's own record: rank, then the tie key (the
// physical path) whatever the direction of the rank.
#[test]
fn sorted_page_search_orders_by_the_authored_key_then_path() {
    let (_dir, graph) = snapshot(&page_fixture());
    let names = |sort: &[(&str, SortDir)]| -> Vec<String> {
        page_search(&graph, 6, sort)
            .0
            .iter()
            .map(|hit| match hit {
                QueryHit::Page { page, .. } => page.name.clone(),
                other => panic!("a page hit, got {other:?}"),
            })
            .collect()
    };
    // r0: Hay3 and Hay7 (matching by content), then Needle00 04 08 by path.
    assert_eq!(
        names(&[("rank", SortDir::Asc)]),
        ["Hay3", "Hay7", "Needle00", "Needle04", "Needle08", "Needle12"]
    );
    // r3: Hay2, Hay6, then Needle03 07 11 15 by path, though the rank descends.
    assert_eq!(
        names(&[("rank", SortDir::Desc)]),
        ["Hay2", "Hay6", "Needle03", "Needle07", "Needle11", "Needle15"]
    );
}
