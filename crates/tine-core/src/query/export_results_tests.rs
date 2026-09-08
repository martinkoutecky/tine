//! RET3's export-core gates: the database-owned bounded subtree export answers
//! exactly what the source-walking export answers, reads no document to do it,
//! reads OUTPUT payload only for the nodes it admitted, keeps two physically
//! distinct roots apart where the walk's id lookup cannot, and FAILS rather
//! than shrinking when the projection contradicts itself.
//!
//! **The oracle is the existing walk export.** Every parity gate compares the
//! COMPLETE `QueryExportBatch` — `omitted_queries`, and per macro the key,
//! `shown`, `total`, `omitted_nodes`, the group order and every `BlockDto`
//! field of every node of every emitted tree — against
//! [`crate::query::export_query_subtrees`] over the SAME graph, the same specs
//! and the same four caps.
//!
//! **The harness is `sql_gates_tests`'s.** The `Corpus` (a graph plus the
//! projection built from it by the PRODUCTION producer), the lowering entry and
//! the damaged-copy helpers are the ones §5's and R3's gates already own; this
//! file adds fixtures, never a second projection builder (D-14).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tine_storage::sqlite::PhysicalProjectionQuerySnapshot;

use crate::model::{BlockDto, Graph, PageKind, RefGroup};
use crate::query::export_results::{
    apply_located_view, export_subtree_census, hydrate_located_export_queries,
    reset_export_subtree_census, select_located_export_queries, set_before_completeness_batch_hook,
    ExportSubtreeInputs, ExportSubtreeSource, LocatedExportRoot,
};
use crate::query::results::{
    read_located_results_merged, reset_result_read_census, result_read_census,
    set_before_export_payload_batch_hook, BackendOrder, RecencyPage, ResultIdentity, ResultLocator,
    ResultReadError, ResultReadShared, ResultSource,
};
use crate::query::sql::sql_gates_tests::{scratch, serialize, Corpus};
use crate::query::{
    export_query_subtrees, page_recency_secs_for, ConstructionProfile, ExportSelectionAnswer,
    QueryDialect, QueryExportBatch, QueryExportResult, QueryExportSpec,
    QUERY_EXPORT_CONSTRUCTION_BYTES, QUERY_EXPORT_CONSTRUCTION_ROWS,
};

// ===== fixtures =====

/// The export fixture corpus. Every shape RET3's acceptance list names has a
/// page here, and each page is deliberately small enough to reason about node
/// for node.
fn write_export_corpus(root: &Path) {
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::create_dir_all(root.join("journals")).expect("journals");

    // Nested roots, a second root on the same page, and a branch nobody
    // requested (a whole-page hydration would clone it).
    std::fs::write(
        root.join("pages/Alpha.md"),
        "- LATER alpha root\n\
         \t- alpha child one\n\
         \t\t- alpha grandchild\n\
         \t- alpha child two\n\
         - TODO alpha second root\n\
         \t- second child\n\
         - plain unrelated block\n\
         \t- unrelated child\n",
    )
    .expect("Alpha page");

    // OVERLAPPING roots: `nested outer` matches, and so does its own
    // grandchild, so one export requests a subtree that CONTAINS another.
    std::fs::write(
        root.join("pages/Nested.md"),
        "- TODO nested outer\n\
         \t- ordinary middle\n\
         \t\t- TODO nested inner\n\
         \t\t\t- inner leaf\n",
    )
    .expect("Nested page");

    // Org, so the format-sensitive facets (marker, heading level) are exported
    // through the same constructor.
    std::fs::write(
        root.join("pages/Beta.org"),
        "* TODO beta root\n** beta child\n*** beta grandchild\n* TODO beta second\n",
    )
    .expect("Beta org page");

    // The RECURSIVE budget shape: a child that cannot fit, followed — under an
    // ANCESTOR — by a sibling that still can.
    std::fs::write(
        root.join("pages/Budget.md"),
        format!(
            "- DOING budget root\n\
             \t- small child A\n\
             \t\t- {}\n\
             \t\t- small grandchild after the huge one\n\
             \t- small child B\n",
            "x".repeat(4_000)
        ),
    )
    .expect("Budget page");

    // A requested root that is NOT top level, followed by a sibling of its own
    // parent: the row that ends its subtree has a real parent, so the boundary
    // check has something to verify.
    std::fs::write(
        root.join("pages/Boundary.md"),
        "- ordinary parent\n\
         \t- [#A] requested nested root\n\
         \t\t- nested leaf\n\
         \t- following uncle sibling\n\
         - another top level\n",
    )
    .expect("Boundary page");

    // A journal page, so the journal/page kind rank is exercised by base order.
    std::fs::write(
        root.join("journals/2026_06_28.md"),
        "- TODO journal root\n\t- journal child\n",
    )
    .expect("journal page");
}

/// A page with `count` top-level blocks, exactly one of which is a requested
/// root with a two-node subtree.
fn write_huge_page_corpus(root: &Path, count: usize) {
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::create_dir_all(root.join("journals")).expect("journals");
    let mut page = String::from("- TODO tiny requested root\n\t- the one requested child\n");
    for index in 0..count {
        page.push_str(&format!("- ordinary block {index}\n"));
    }
    std::fs::write(root.join("pages/Huge.md"), page).expect("Huge page");
}

/// A nonempty subtree for the dialect gate. Its physical path and structural
/// order are fixed, so the gate can name its exact runtime ids below rather
/// than proving only that two evaluators returned the same anonymous rows.
fn write_dialect_corpus(root: &Path) {
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::create_dir_all(root.join("journals")).expect("journals");
    std::fs::write(
        root.join("pages/Dialect.md"),
        "- TODO dialect root\n\
         \t- dialect child\n\
         \t\t- dialect grandchild\n\
         - DONE excluded root\n",
    )
    .expect("Dialect page");
}

// ===== the adapter under test =====

/// Every page id in one corpus's projection, read through the seam the gates
/// already use for fixture lookups.
fn all_page_ids(corpus: &Corpus) -> Vec<[u8; 16]> {
    let reader = rusqlite::Connection::open_with_flags(
        corpus.projection_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("the projection opens read-only");
    let mut statement = reader
        .prepare("SELECT page_id FROM pages ORDER BY page_id")
        .expect("the page index is readable");
    let ids = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .expect("the page index is readable")
        .map(|id| {
            id.expect("a page id row")
                .as_slice()
                .try_into()
                .expect("a 16-byte page id")
        })
        .collect();
    ids
}

/// The recency producer, bound to one corpus root — R3's own axis, reached
/// through the `(journal day, page path)` signature the result read offers.
fn recency_for(root: &Path) -> impl Fn(RecencyPage<'_>) -> i64 + '_ {
    move |page| page_recency_secs_for(page.journal_day, &root.join(page.path))
}

fn spec(key: &str, query: &str) -> QueryExportSpec {
    QueryExportSpec {
        key: key.to_string(),
        query: query.to_string(),
        advanced: false,
        simple_dialect: None,
        current_page: None,
    }
}

/// The four caps one export batch is bounded by.
#[derive(Clone, Copy, Debug)]
struct Caps {
    queries: usize,
    roots: usize,
    nodes: usize,
    bytes: usize,
}

impl Default for Caps {
    fn default() -> Self {
        Caps {
            queries: 64,
            roots: 64,
            nodes: 4_096,
            bytes: 1024 * 1024,
        }
    }
}

/// **The shape the manager's public Direct adapter will have**, expressed here
/// so the core can be proved before that adapter exists: capture the identity
/// policy and the snapshot, select through the located result read and the
/// shared view, then hydrate over the SAME snapshot.
///
/// Everything outside the core's own boundary — parsing, lowering, the
/// per-query 20k/32MiB construction ceiling and `omitted_queries` — is the
/// caller's, exactly as it is for the oracle.
fn export_over(
    corpus: &Corpus,
    snapshot: &mut PhysicalProjectionQuerySnapshot,
    specs: &[QueryExportSpec],
    caps: Caps,
    identity: &ResultIdentity,
) -> Result<QueryExportBatch, ResultReadError> {
    let today = corpus.today();
    let root = corpus.root.clone();
    let recency = recency_for(&root);
    let (limit, selected) = {
        let snapshot = &mut *snapshot;
        select_located_export_queries(specs, caps.queries, caps.roots, |spec| {
            let dialect = spec.simple_dialect();
            let (_query, view) = crate::query::parse_query_text(&spec.query, dialect, today);
            let (_block_query, statement) = corpus.lower_block_anchored(&spec.query, dialect);
            let shared = ResultReadShared {
                order: BackendOrder::Direct,
                identity,
                max_rows: QUERY_EXPORT_CONSTRUCTION_ROWS,
                max_bytes: QUERY_EXPORT_CONSTRUCTION_BYTES,
                profile: ConstructionProfile::from_view(&view),
                recency: &recency,
            };
            let pre = read_located_results_merged(
                &mut [ResultSource {
                    snapshot: &mut *snapshot,
                    statement: &statement,
                }],
                &shared,
            )?;
            Ok(apply_located_view(pre, &view))
        })?
    };
    let results = hydrate_located_export_queries(
        &mut [ExportSubtreeSource { snapshot }],
        selected,
        &ExportSubtreeInputs {
            identity,
            max_nodes: caps.nodes,
            max_bytes: caps.bytes,
        },
    )?;
    Ok(QueryExportBatch {
        results,
        omitted_queries: specs.len().saturating_sub(limit),
    })
}

/// The walk's answer for the same specs and caps.
fn walk_export(graph: &Graph, specs: &[QueryExportSpec], caps: Caps) -> QueryExportBatch {
    export_query_subtrees(
        graph,
        specs,
        caps.queries,
        caps.roots,
        caps.nodes,
        caps.bytes,
    )
}

// ===== difference reporting =====

/// ONE line per difference between two export batches, naming the macro, the
/// position and the FIELD — never a page name or a raw line, so the same
/// comparison would be safe against a real corpus.
fn batch_differences(label: &str, walk: &QueryExportBatch, read: &QueryExportBatch) -> Vec<String> {
    let mut out = Vec::new();
    if walk.omitted_queries != read.omitted_queries {
        out.push(format!(
            "{label}: omitted_queries walk={} read={}",
            walk.omitted_queries, read.omitted_queries
        ));
    }
    if walk.results.len() != read.results.len() {
        out.push(format!(
            "{label}: results walk={} read={}",
            walk.results.len(),
            read.results.len()
        ));
        return out;
    }
    for (at, (expected, actual)) in walk.results.iter().zip(&read.results).enumerate() {
        out.extend(result_differences(
            &format!("{label}: macro {at}"),
            expected,
            actual,
        ));
    }
    out
}

fn result_differences(
    label: &str,
    walk: &QueryExportResult,
    read: &QueryExportResult,
) -> Vec<String> {
    let mut out = Vec::new();
    for (what, expected, actual) in [
        ("shown", walk.shown, read.shown),
        ("total", walk.total, read.total),
        ("omitted_nodes", walk.omitted_nodes, read.omitted_nodes),
    ] {
        if expected != actual {
            out.push(format!("{label}: {what} walk={expected} read={actual}"));
        }
    }
    if walk.key != read.key {
        out.push(format!("{label}: key differs"));
    }
    if walk.groups.len() != read.groups.len() {
        out.push(format!(
            "{label}: groups walk={} read={}",
            walk.groups.len(),
            read.groups.len()
        ));
        return out;
    }
    for (at, (expected, actual)) in walk.groups.iter().zip(&read.groups).enumerate() {
        out.extend(group_differences(
            &format!("{label} group {at}"),
            expected,
            actual,
        ));
    }
    out
}

fn group_differences(label: &str, walk: &RefGroup, read: &RefGroup) -> Vec<String> {
    let mut out = Vec::new();
    if walk.page != read.page {
        out.push(format!("{label}: page name differs"));
    }
    if walk.kind != read.kind {
        out.push(format!("{label}: page kind differs"));
    }
    if !read.evidence.is_empty() {
        out.push(format!("{label}: carries evidence"));
    }
    out.extend(tree_differences(label, &walk.blocks, &read.blocks));
    out
}

/// Every field of every node of two emitted forests, in order.
fn tree_differences(label: &str, walk: &[BlockDto], read: &[BlockDto]) -> Vec<String> {
    let mut out = Vec::new();
    if walk.len() != read.len() {
        out.push(format!(
            "{label}: nodes walk={} read={}",
            walk.len(),
            read.len()
        ));
        return out;
    }
    for (at, (expected, actual)) in walk.iter().zip(read).enumerate() {
        for field in block_field_differences(expected, actual) {
            out.push(format!("{label}/{at}: field {field}"));
        }
        out.extend(tree_differences(
            &format!("{label}/{at}"),
            &expected.children,
            &actual.children,
        ));
    }
    out
}

fn block_field_differences(expected: &BlockDto, actual: &BlockDto) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if expected.id != actual.id {
        fields.push("id");
    }
    if expected.raw != actual.raw {
        fields.push("raw");
    }
    if expected.collapsed != actual.collapsed {
        fields.push("collapsed");
    }
    if !actual.breadcrumb.is_empty() {
        fields.push("breadcrumb");
    }
    if actual.page_property {
        fields.push("page_property");
    }
    if expected.marker != actual.marker {
        fields.push("marker");
    }
    if expected.priority != actual.priority {
        fields.push("priority");
    }
    if expected.heading_level != actual.heading_level {
        fields.push("heading_level");
    }
    if expected.scheduled != actual.scheduled {
        fields.push("scheduled");
    }
    if expected.deadline != actual.deadline {
        fields.push("deadline");
    }
    if expected.tags != actual.tags {
        fields.push("tags");
    }
    if expected.properties != actual.properties {
        fields.push("properties");
    }
    fields
}

fn node_count(blocks: &[BlockDto]) -> usize {
    blocks
        .iter()
        .map(|block| 1 + node_count(&block.children))
        .sum()
}

fn emitted_nodes(batch: &QueryExportBatch) -> usize {
    batch
        .results
        .iter()
        .flat_map(|result| result.groups.iter())
        .map(|group| node_count(&group.blocks))
        .sum()
}

fn preorder_ids(blocks: &[BlockDto]) -> Vec<String> {
    fn collect(blocks: &[BlockDto], ids: &mut Vec<String>) {
        for block in blocks {
            ids.push(block.id.clone());
            collect(&block.children, ids);
        }
    }
    let mut ids = Vec::new();
    collect(blocks, &mut ids);
    ids
}

// ===== the ordered full-batch parity sweep =====

/// The specs the sweep runs, chosen so that every acceptance shape appears:
/// nested and overlapping roots, several macros under one global budget, an Org
/// page, a journal page, a macro that selects nothing, and a macro that selects
/// a root whose subtree is one node.
fn sweep_specs() -> Vec<QueryExportSpec> {
    vec![
        spec("todo", "(task TODO)"),
        spec("later", "(task LATER)"),
        spec("doing", "(task DOING)"),
        spec("none", "(task CANCELED)"),
        spec("both", "(task TODO DOING)"),
        spec("priority", "(priority A)"),
    ]
}

/// The cap combinations: unbounded-ish, zero everything (which clamps to one),
/// a root cap that cuts across macros, and node/byte caps that close inside a
/// subtree.
fn sweep_caps() -> Vec<Caps> {
    vec![
        Caps::default(),
        Caps {
            queries: 0,
            roots: 0,
            nodes: 0,
            bytes: 0,
        },
        Caps {
            queries: 2,
            ..Caps::default()
        },
        Caps {
            roots: 1,
            ..Caps::default()
        },
        Caps {
            roots: 3,
            ..Caps::default()
        },
        Caps {
            nodes: 1,
            ..Caps::default()
        },
        Caps {
            nodes: 2,
            ..Caps::default()
        },
        Caps {
            nodes: 5,
            ..Caps::default()
        },
        Caps {
            bytes: 1,
            ..Caps::default()
        },
        Caps {
            bytes: 400,
            ..Caps::default()
        },
        Caps {
            bytes: 900,
            ..Caps::default()
        },
        Caps {
            bytes: 2_000,
            ..Caps::default()
        },
    ]
}

#[test]
fn export_core_answers_exactly_what_the_walk_export_answers() {
    let _serial = serialize();
    let root = scratch("ret3-export-parity");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    // Every macro ALONE, and then all of them under one shared budget: a cap
    // that closes early in the combined run would otherwise hide a shape that
    // only appears when a later macro still has budget left.
    let mut runs: Vec<Vec<QueryExportSpec>> =
        sweep_specs().into_iter().map(|spec| vec![spec]).collect();
    runs.push(sweep_specs());
    let mut differences: Vec<String> = Vec::new();
    let mut nodes = 0usize;
    for identity in [
        ResultIdentity::Stored,
        // A FRESH Direct session: no page's identity is this session's, so
        // every row — root and descendant alike — resolves its public id
        // structurally from the stored path and order key.
        ResultIdentity::DirectStructural {
            session_pages: std::sync::Arc::new(std::collections::HashSet::new()),
            all_session: false,
        },
    ] {
        for specs in &runs {
            for caps in sweep_caps() {
                let keys: Vec<&str> = specs.iter().map(|spec| spec.key.as_str()).collect();
                let label = format!("{keys:?} {caps:?}");
                let walk = walk_export(&corpus.graph, specs, caps);
                nodes += emitted_nodes(&walk);
                let mut snapshot = corpus.snapshot();
                let read = export_over(&corpus, &mut snapshot, specs, caps, &identity);
                snapshot.finish();
                match read {
                    Ok(read) => differences.extend(batch_differences(&label, &walk, &read)),
                    Err(error) => differences.push(format!("{label}: the export failed: {error}")),
                }
            }
        }
    }
    assert!(nodes > 0, "the sweep exported something to compare");
    assert!(
        differences.is_empty(),
        "the database export differs from the walk export:\n{}",
        differences.join("\n")
    );
}

#[test]
fn tql_export_preserves_the_declared_dialect_and_exact_subtree_membership() {
    let _serial = serialize();
    let root = scratch("ret3-export-dialect");
    write_dialect_corpus(&root);
    let corpus = Corpus::open(root, true);
    let caps = Caps::default();
    let og = vec![spec("dialect", "(task TODO)")];
    let tql = vec![QueryExportSpec {
        simple_dialect: Some(QueryDialect::Tql),
        ..spec("dialect", "task = 'TODO'")
    }];

    let og_walk = walk_export(&corpus.graph, &og, caps);
    let tql_walk = walk_export(&corpus.graph, &tql, caps);
    let runtime_id = |order| {
        crate::model::doc_runtime_id_for_order("pages/Dialect.md", order)
            .expect("the fixture structural order is valid")
            .to_string()
    };
    let expected = vec![
        runtime_id("00000000"),
        runtime_id("00000000/00000000"),
        runtime_id("00000000/00000000/00000000"),
    ];
    assert_eq!(tql_walk.results.len(), 1);
    assert_eq!(tql_walk.results[0].shown, 1);
    assert_eq!(tql_walk.results[0].total, 1);
    assert_eq!(tql_walk.results[0].omitted_nodes, 0);
    assert_eq!(tql_walk.results[0].groups.len(), 1);
    assert_eq!(tql_walk.results[0].groups[0].page, "Dialect");
    assert_eq!(
        preorder_ids(&tql_walk.results[0].groups[0].blocks),
        expected
    );
    assert!(
        batch_differences("OG/TQL walk", &og_walk, &tql_walk).is_empty(),
        "equivalent OG and TQL exports differ"
    );

    let mut snapshot = corpus.snapshot();
    let tql_read = export_over(&corpus, &mut snapshot, &tql, caps, &ResultIdentity::Stored)
        .expect("the declared TQL export answers");
    snapshot.finish();
    assert_eq!(
        preorder_ids(&tql_read.results[0].groups[0].blocks),
        expected
    );
    let differences = batch_differences("TQL walk/read", &tql_walk, &tql_read);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

/// Sorted, coalesced and sampled selections reach the same export as the walk's.
///
/// The view runs over LOCATED entries here, so this is the gate that would
/// catch a located sort that reordered DTOs without their locators: the exported
/// SUBTREES would then belong to the wrong roots while the root rows still
/// looked right.
#[test]
fn export_core_matches_the_walk_through_sort_coalescing_and_sampling() {
    let _serial = serialize();
    let root = scratch("ret3-export-view");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let specs = vec![
        spec("sorted", "(and (task TODO) (sort-by page desc))"),
        spec("sampled", "(and (task TODO) (sample 2))"),
        spec(
            "sorted-sampled",
            "(and (task TODO) (sort-by page asc) (sample 3))",
        ),
    ];
    let mut differences = Vec::new();
    for caps in [
        Caps::default(),
        Caps {
            nodes: 3,
            ..Caps::default()
        },
    ] {
        let walk = walk_export(&corpus.graph, &specs, caps);
        let mut snapshot = corpus.snapshot();
        let read = export_over(
            &corpus,
            &mut snapshot,
            &specs,
            caps,
            &ResultIdentity::Stored,
        )
        .expect("the export answers");
        snapshot.finish();
        differences.extend(batch_differences(&format!("{caps:?}"), &walk, &read));
    }
    assert!(
        differences.is_empty(),
        "a viewed export differs from the walk's:\n{}",
        differences.join("\n")
    );
}

// ===== the recursive budget shape =====

/// **The one budget property that a global preorder prefix would break.**
///
/// `budget root` has two children: `small child A`, whose own first child is
/// too large to fit, and `small child B`. When A's huge child is refused, A's
/// remaining child loop stops — but B is a LATER SIBLING OF AN ANCESTOR and
/// still fits. A "emit in preorder until the budget closes" implementation
/// would stop at the huge node and lose B.
#[test]
fn a_child_that_cannot_fit_stops_its_parent_but_not_its_grandparents_next_child() {
    let _serial = serialize();
    let root = scratch("ret3-export-recursive-budget");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let specs = vec![spec("budget", "(task DOING)")];
    // Enough for the root, `small child A` and `small child B`, and nowhere
    // near enough for the 4 000-byte grandchild.
    let caps = Caps {
        bytes: 1_000,
        ..Caps::default()
    };
    let walk = walk_export(&corpus.graph, &specs, caps);
    let mut snapshot = corpus.snapshot();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        caps,
        &ResultIdentity::Stored,
    )
    .expect("the export answers");
    snapshot.finish();

    let tree = &read.results[0].groups[0].blocks[0];
    let children: Vec<&str> = tree
        .children
        .iter()
        .map(|child| child.raw.as_str())
        .collect();
    assert_eq!(
        children,
        ["small child A", "small child B"],
        "the ancestor's later sibling still fits"
    );
    assert!(
        tree.children[0].children.is_empty(),
        "the parent whose child did not fit stops its own child loop"
    );
    // The huge grandchild and the grandchild after it are both omitted.
    assert_eq!(read.results[0].omitted_nodes, 2);
    assert!(
        batch_differences("recursive budget", &walk, &read).is_empty(),
        "the walk agrees, node for node"
    );
}

/// The same shape under the NODE budget rather than the byte budget.
#[test]
fn the_node_budget_is_recursive_too() {
    let _serial = serialize();
    let root = scratch("ret3-export-node-budget");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let specs = vec![spec("alpha", "(task LATER)")];
    let mut differences = Vec::new();
    for nodes in 1..=5 {
        let caps = Caps {
            nodes,
            ..Caps::default()
        };
        let walk = walk_export(&corpus.graph, &specs, caps);
        let mut snapshot = corpus.snapshot();
        let read = export_over(
            &corpus,
            &mut snapshot,
            &specs,
            caps,
            &ResultIdentity::Stored,
        )
        .expect("the export answers");
        snapshot.finish();
        differences.extend(batch_differences(&format!("nodes={nodes}"), &walk, &read));
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

// ===== the work the export actually performed =====

/// A huge page with a TINY requested subtree costs the subtree, not the page.
///
/// One page statement, one root statement, one topology statement (the subtree
/// plus the boundary row fit in one batch of `TOPOLOGY_BATCH`), one boundary
/// check, and THREE output payload statements for the one admitted descendant.
/// The root reuses the DTO the selection already built, so it costs no output
/// payload at all.
#[test]
fn a_huge_page_with_a_tiny_requested_subtree_reads_the_subtree_only() {
    let _serial = serialize();
    let root = scratch("ret3-export-huge-page");
    write_huge_page_corpus(&root, 4_000);
    let corpus = Corpus::open(root, true);
    let specs = vec![spec("tiny", "(task TODO)")];
    let mut snapshot = corpus.snapshot();
    reset_export_subtree_census();
    reset_result_read_census();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        Caps::default(),
        &ResultIdentity::Stored,
    )
    .expect("the export answers");
    snapshot.finish();
    let topology = export_subtree_census();
    let payload = result_read_census();

    assert_eq!(read.results[0].shown, 1);
    assert_eq!(read.results[0].omitted_nodes, 0);
    assert_eq!(node_count(&read.results[0].groups[0].blocks), 2);

    assert_eq!(topology.page_statements, 1);
    assert_eq!(topology.root_statements, 1);
    assert_eq!(topology.topology_statements, 1);
    // ONE bounded fetch of at most `TOPOLOGY_BATCH` narrow rows — never the
    // 4 002-block page — and only two of them are decoded before the subtree
    // ends: the requested child, and the row after it.
    assert_eq!(topology.topology_rows_fetched, 128);
    assert_eq!(topology.topology_rows_examined, 2);
    // The row that ends the subtree is a PAGE ROOT, so there is no parent to
    // verify; `a_nested_root_verifies_the_parent_that_ends_its_subtree` covers
    // the case where there is one.
    assert_eq!(topology.boundary_statements, 0);

    // Selection paid for its single matched row; the export output paid for the
    // single admitted DESCENDANT, and for nothing else on a 4 002-block page.
    assert_eq!(payload.payload_block_rows, 1);
    assert_eq!(payload.export_payload_statements, 3);
    assert_eq!(payload.export_payload_block_rows, 1);
}

/// OUTPUT payload is read for ADMITTED descendants and for no one else.
#[test]
fn output_payload_is_read_only_for_admitted_descendants() {
    let _serial = serialize();
    let root = scratch("ret3-export-admitted-only");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let specs = vec![spec("alpha", "(task LATER)")];
    // The root plus exactly one descendant.
    let caps = Caps {
        nodes: 2,
        ..Caps::default()
    };
    let mut snapshot = corpus.snapshot();
    reset_export_subtree_census();
    reset_result_read_census();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        caps,
        &ResultIdentity::Stored,
    )
    .expect("the export answers");
    snapshot.finish();
    let topology = export_subtree_census();
    let payload = result_read_census();

    assert_eq!(node_count(&read.results[0].groups[0].blocks), 2);
    assert_eq!(read.results[0].omitted_nodes, 2);
    // Topology still covers the WHOLE requested subtree — that is what makes
    // `omitted_nodes` exact after the output budget has closed: three requested
    // descendants, plus the boundary row that ends the subtree.
    assert_eq!(topology.topology_rows_examined, 4);
    // …while output payload covers the single admitted descendant.
    assert_eq!(payload.export_payload_block_rows, 1);
    assert_eq!(payload.export_payload_statements, 3);
}

/// The export answers from the projection alone: with the graph's source files
/// DELETED from disk, the same batch still comes back byte for byte.
///
/// A structural claim ("this module holds no `Graph`") is checked below; this
/// is the observation-boundary version of it.
#[test]
fn the_export_reads_no_source_document() {
    let _serial = serialize();
    let root = scratch("ret3-export-no-documents");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let specs = sweep_specs();
    let caps = Caps::default();
    let walk = walk_export(&corpus.graph, &specs, caps);

    // The projection lives in its own directory; the graph's Markdown/Org does
    // not, so removing it leaves the database intact and the source gone.
    std::fs::remove_dir_all(corpus.root.join("pages")).expect("the source pages are removable");
    std::fs::remove_dir_all(corpus.root.join("journals"))
        .expect("the source journals are removable");

    let mut snapshot = corpus.snapshot();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        caps,
        &ResultIdentity::Stored,
    )
    .expect("the export answers without any source document");
    snapshot.finish();
    let differences = batch_differences("no documents", &walk, &read);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

/// The architectural claim, as a test rather than a comment: the export core
/// names no graph, no document, no parser and no filesystem.
#[test]
fn the_export_core_source_names_no_graph_document_or_filesystem() {
    let source = include_str!("export_results.rs");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "Graph",
        "Document",
        "DocBlock",
        "QueryPageSource",
        "with_pages",
        "with_hydration_pages",
        "std::fs",
        "File::",
        "parse_query",
    ] {
        assert!(
            !code.contains(forbidden),
            "the export core must not name `{forbidden}`: it answers from the \
             projection alone, over snapshots its caller owns"
        );
    }
}

// ===== physical identity =====

/// The structural runtime id one block of a Direct Files page will carry.
fn structural_id(path: &str, order_key: &str) -> String {
    crate::model::doc_runtime_id_for_order(path, order_key)
        .expect("the fixture order key resolves")
        .to_string()
}

/// **Where the walk's id lookup is wrong and this one is not.**
///
/// The walk finds an export root by scanning the page for the FIRST block whose
/// runtime uuid — or whose `id::` property — equals the selected result's public
/// id. A block that merely CARRIES another block's id in an `id::` property
/// therefore wins, and the walk exports the decoy's subtree.
///
/// The database export never looks a root up by its public id at all: selection
/// attached the physical block id, so the requested subtree is the requested
/// one. This gate asserts the CORRECT physical outcome, deliberately not the
/// oracle's.
#[test]
fn a_root_is_the_physical_block_that_was_selected_not_the_first_block_that_shares_its_id() {
    let _serial = serialize();
    let root = scratch("ret3-export-duplicate-ids");
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::create_dir_all(root.join("journals")).expect("journals");
    // The REAL target is the second top-level block of the page.
    let real = structural_id("pages/Dup.md", "00000001");
    std::fs::write(
        root.join("pages/Dup.md"),
        format!(
            "- decoy parent\n  id:: {real}\n\
             \t- decoy child one\n\
             \t- decoy child two\n\
             - TODO real target\n\
             \t- the real child\n"
        ),
    )
    .expect("Dup page");
    let corpus = Corpus::open(root, true);
    let specs = vec![spec("dup", "(task TODO)")];
    let caps = Caps::default();

    let walk = walk_export(&corpus.graph, &specs, caps);
    let mut snapshot = corpus.snapshot();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        caps,
        &ResultIdentity::Stored,
    )
    .expect("the export answers");
    snapshot.finish();

    let exported = &read.results[0].groups[0].blocks[0];
    assert_eq!(exported.id, real, "the selected root is the one exported");
    assert_eq!(exported.raw, "TODO real target");
    let children: Vec<&str> = exported
        .children
        .iter()
        .map(|child| child.raw.as_str())
        .collect();
    assert_eq!(
        children,
        ["the real child"],
        "the requested physical subtree, not the decoy's"
    );

    // And the oracle really does get this wrong, which is why the assertion
    // above is spelled out rather than compared against it.
    let confused = &walk.results[0].groups[0].blocks[0];
    assert!(
        confused.raw.starts_with("decoy parent"),
        "the source walk resolves the root through the shared `id::` property"
    );
}

/// Two physical pages that DISPLAY the same name keep their own subtrees.
///
/// The display name is a page fact, so it is renamed on a copy of the
/// projection: the walk cannot see this state at all (its source files still
/// carry the real names), so the correct physical outcome is asserted directly.
/// The grouping rule is still the oracle's — one output group per
/// `(kind, display name)` — and it is the SUBTREES underneath that must not mix.
#[test]
fn equal_display_names_do_not_merge_two_physical_subtrees() {
    let _serial = serialize();
    let root = scratch("ret3-export-equal-names");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let path = copy_projection(&corpus, "equal-names");
    {
        let writer = rusqlite::Connection::open(&path).expect("the copy opens writable");
        let changed = writer
            .execute(
                "UPDATE pages SET name = 'Same' WHERE name IN ('Alpha', 'Nested')",
                [],
            )
            .expect("the rename applies");
        assert_eq!(changed, 2, "both fixture pages were renamed");
    }
    let specs = vec![spec("todo", "(task TODO)"), spec("later", "(task LATER)")];
    let mut snapshot = PhysicalProjectionQuerySnapshot::open_direct(&path, || Ok(()))
        .expect("the renamed copy opens");
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        Caps::default(),
        &ResultIdentity::Stored,
    )
    .expect("the export answers");
    snapshot.finish();
    let _ = std::fs::remove_file(&path);

    let groups: Vec<&RefGroup> = read
        .results
        .iter()
        .flat_map(|result| result.groups.iter())
        .collect();
    let renamed: Vec<&&RefGroup> = groups.iter().filter(|group| group.page == "Same").collect();
    assert!(
        !renamed.is_empty(),
        "the renamed pages contributed export groups"
    );
    // `Alpha` contributes two roots and `Nested` two more; each keeps its own
    // children, which is only possible if the roots were located physically.
    let mut by_root: HashMap<&str, Vec<&str>> = HashMap::new();
    for group in &groups {
        for block in &group.blocks {
            by_root.insert(
                block.raw.as_str(),
                block.children.iter().map(|c| c.raw.as_str()).collect(),
            );
        }
    }
    assert_eq!(
        by_root.get("LATER alpha root").map(Vec::as_slice),
        Some(["alpha child one", "alpha child two"].as_slice())
    );
    assert_eq!(
        by_root.get("TODO alpha second root").map(Vec::as_slice),
        Some(["second child"].as_slice())
    );
    assert_eq!(
        by_root.get("TODO nested outer").map(Vec::as_slice),
        Some(["ordinary middle"].as_slice())
    );
    assert_eq!(
        by_root.get("TODO nested inner").map(Vec::as_slice),
        Some(["inner leaf"].as_slice())
    );
}

/// Two roots with the SAME public id and the SAME display name, distinguished
/// only by their locators, select two different physical subtrees.
///
/// This is the selection half of the property above, at the level the manager's
/// adapters will reach it: no SQL, no view, just the located root projection.
#[test]
fn located_selection_keeps_two_identically_named_roots_apart() {
    let block = |raw: &str| BlockDto {
        id: "one-public-id".into(),
        raw: raw.into(),
        ..BlockDto::default()
    };
    let locator = |page: u8, id: u8| ResultLocator {
        source: 0,
        page_id: [page; 16],
        block_id: [id; 16],
    };
    let answer = ExportSelectionAnswer {
        groups: vec![crate::query::ResultViewGroup {
            page: "Same".into(),
            kind: PageKind::Page,
            blocks: vec![
                (block("first"), locator(1, 11)),
                (block("second"), locator(2, 22)),
            ],
            evidence: Vec::new(),
        }],
        total: 2,
        exceeded: false,
    };
    let mut once = Some(answer);
    let (limit, selected) = select_located_export_queries::<std::convert::Infallible>(
        &[spec("k", "(task TODO)")],
        8,
        8,
        |_spec| Ok(once.take().expect("one macro")),
    )
    .expect("the selection answers");
    assert_eq!(limit, 8);
    let roots: Vec<&LocatedExportRoot> = selected[0].roots.iter().collect();
    assert_eq!(roots.len(), 2);
    assert!(
        roots
            .iter()
            .all(|root| root.page == "Same" && root.block.id == "one-public-id"),
        "the two roots are indistinguishable by their public facts"
    );
    assert_eq!(roots[0].locator, locator(1, 11));
    assert_eq!(roots[1].locator, locator(2, 22));
}

/// A nested requested root's subtree ends at a sibling of its own PARENT, and
/// that parent is verified: it must be an EARLIER block of the same page.
///
/// This is the check a corrupted parent chain trips, so a gate has to prove it
/// runs on healthy data too — otherwise the corruption gate below could be
/// passing for the wrong reason.
#[test]
fn a_nested_root_verifies_the_parent_that_ends_its_subtree() {
    let _serial = serialize();
    let root = scratch("ret3-export-boundary");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let specs = vec![spec("nested-root", "(priority A)")];
    let caps = Caps::default();
    let walk = walk_export(&corpus.graph, &specs, caps);
    let mut snapshot = corpus.snapshot();
    reset_export_subtree_census();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        caps,
        &ResultIdentity::Stored,
    )
    .expect("the export answers");
    snapshot.finish();
    let topology = export_subtree_census();
    assert_eq!(read.results[0].shown, 1);
    assert_eq!(node_count(&read.results[0].groups[0].blocks), 2);
    assert_eq!(
        topology.boundary_statements, 1,
        "the row after the subtree has a parent, and it was verified"
    );
    let differences = batch_differences("nested boundary", &walk, &read);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

// ===== two sources =====

/// The Managed pending route's shape: two owned snapshots whose pages are
/// DISJOINT, one statement lowered per source, and roots selected from both.
///
/// Each root's subtree must be read on the snapshot its own locator names. The
/// answer is compared against the single-source walk export, which sees the same
/// graph — so a root hydrated against the wrong source shows up as a wrong
/// subtree, not merely as a wrong count.
#[test]
fn split_source_roots_hydrate_against_their_own_snapshot() {
    let _serial = serialize();
    let root = scratch("ret3-export-split-source");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let specs = vec![spec("todo", "(task TODO)")];
    let caps = Caps::default();
    let walk = walk_export(&corpus.graph, &specs, caps);

    // "Pending" owns Alpha; "accepted" is masked against it. Disjoint by
    // construction, exactly as R5a's overlay masking makes them.
    let pending_pages = [corpus.page_id("Alpha")];
    let today = corpus.today();
    let graph_root = corpus.root.clone();
    let recency = recency_for(&graph_root);
    let identity = ResultIdentity::Stored;

    // The overlay's statement masks every page the overlay does NOT own.
    let accepted_mask: Vec<[u8; 16]> = all_page_ids(&corpus)
        .into_iter()
        .filter(|id| !pending_pages.contains(id))
        .collect();
    let mut overlay = corpus.snapshot();
    let mut accepted = corpus.snapshot();
    let read = {
        let overlay = &mut overlay;
        let accepted = &mut accepted;
        let (limit, selected) =
            select_located_export_queries(&specs, caps.queries, caps.roots, |spec| {
                let dialect = spec.simple_dialect();
                let (_query, view) = crate::query::parse_query_text(&spec.query, dialect, today);
                // The overlay sees ONLY its own pages; the accepted source has them
                // masked out. Two statements, one IR.
                let (_q, overlay_statement) =
                    corpus.lower_block_anchored_masked(&spec.query, dialect, &accepted_mask);
                let (_q, accepted_statement) =
                    corpus.lower_block_anchored_masked(&spec.query, dialect, &pending_pages);
                let shared = ResultReadShared {
                    order: BackendOrder::Direct,
                    identity: &identity,
                    max_rows: QUERY_EXPORT_CONSTRUCTION_ROWS,
                    max_bytes: QUERY_EXPORT_CONSTRUCTION_BYTES,
                    profile: ConstructionProfile::from_view(&view),
                    recency: &recency,
                };
                let pre = read_located_results_merged(
                    &mut [
                        ResultSource {
                            snapshot: &mut *overlay,
                            statement: &overlay_statement,
                        },
                        ResultSource {
                            snapshot: &mut *accepted,
                            statement: &accepted_statement,
                        },
                    ],
                    &shared,
                )?;
                Ok::<_, ResultReadError>(apply_located_view(pre, &view))
            })
            .expect("the split selection answers");
        assert_eq!(limit, caps.queries);
        assert!(
            selected[0]
                .roots
                .iter()
                .any(|root| root.locator.source == 0)
                && selected[0]
                    .roots
                    .iter()
                    .any(|root| root.locator.source == 1),
            "the selection really did come from both sources"
        );
        let results = hydrate_located_export_queries(
            &mut [
                ExportSubtreeSource { snapshot: overlay },
                ExportSubtreeSource { snapshot: accepted },
            ],
            selected,
            &ExportSubtreeInputs {
                identity: &identity,
                max_nodes: caps.nodes,
                max_bytes: caps.bytes,
            },
        )
        .expect("the split export answers");
        QueryExportBatch {
            results,
            omitted_queries: 0,
        }
    };
    overlay.finish();
    accepted.finish();
    let differences = batch_differences("split source", &walk, &read);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

// ===== corruption fails the export =====

/// A standalone, consistent copy of a corpus's projection, made through
/// SQLite's own `VACUUM INTO` so a WAL-resident page cannot be missed.
fn copy_projection(corpus: &Corpus, tag: &str) -> PathBuf {
    let destination = std::env::temp_dir().join(format!(
        "tine-ret3-export-{tag}-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let source = rusqlite::Connection::open_with_flags(
        corpus.projection_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("the projection opens read-only");
    source
        .execute(
            "VACUUM INTO ?1",
            rusqlite::params![destination.to_string_lossy().as_ref()],
        )
        .expect("the projection copies");
    destination
}

/// Damage one copy and export from it.
fn export_damaged(
    corpus: &Corpus,
    tag: &str,
    damage: &str,
    bind: &[&dyn rusqlite::ToSql],
) -> Result<QueryExportBatch, ResultReadError> {
    let path = copy_projection(corpus, tag);
    {
        let writer = rusqlite::Connection::open(&path).expect("the copy opens writable");
        writer
            .pragma_update(None, "foreign_keys", false)
            .expect("foreign key enforcement is settable");
        let changed = writer.execute(damage, bind).expect("the damage applies");
        assert!(
            changed > 0,
            "the damage statement changed nothing: {damage}"
        );
    }
    let mut snapshot = PhysicalProjectionQuerySnapshot::open_direct(&path, || Ok(()))
        .expect("the damaged copy still opens");
    let answer = export_over(
        corpus,
        &mut snapshot,
        &[spec("later", "(task LATER)")],
        Caps::default(),
        &ResultIdentity::Stored,
    );
    snapshot.finish();
    let _ = std::fs::remove_file(&path);
    answer
}

/// The block id of one fixture block, named by a substring of its own source
/// line. Only ever called with this file's own fixture text.
fn block_id_of(corpus: &Corpus, needle: &str) -> Vec<u8> {
    let reader = rusqlite::Connection::open_with_flags(
        corpus.projection_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("the projection opens read-only");
    let mut statement = reader
        .prepare(
            "SELECT b.block_id FROM blocks b JOIN block_text t ON t.block_id = b.block_id \
             WHERE instr(t.content, ?1) > 0",
        )
        .expect("the fixture lookup prepares");
    let ids: Vec<Vec<u8>> = statement
        .query_map(rusqlite::params![needle], |row| row.get::<_, Vec<u8>>(0))
        .expect("the fixture lookup runs")
        .collect::<Result<Vec<_>, _>>()
        .expect("the fixture lookup runs");
    assert_eq!(ids.len(), 1, "{needle:?} names exactly one fixture block");
    ids.into_iter().next().expect("one fixture block")
}

/// Every kind of damage RET3 names, each on a row the healthy export ADMITS:
/// missing payload, missing metadata, wrong ownership and a corrupt parent
/// chain. None of them may come back as a shorter export.
#[test]
fn a_damaged_projection_fails_the_export_instead_of_shortening_it() {
    let _serial = serialize();
    let root = scratch("ret3-export-damage");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let child = block_id_of(&corpus, "alpha child one");
    let foreign = block_id_of(&corpus, "nested outer");

    let healthy = {
        let mut snapshot = corpus.snapshot();
        let answer = export_over(
            &corpus,
            &mut snapshot,
            &[spec("later", "(task LATER)")],
            Caps::default(),
            &ResultIdentity::Stored,
        )
        .expect("the healthy export answers");
        snapshot.finish();
        answer
    };
    assert_eq!(
        emitted_nodes(&healthy),
        4,
        "the healthy export emits the root and the three descendants each \
         damage below removes"
    );

    let damages: Vec<(&str, &str, Vec<&dyn rusqlite::ToSql>)> = vec![
        (
            "missing-payload",
            "DELETE FROM block_text WHERE block_id = ?1",
            vec![&child],
        ),
        (
            "missing-metadata",
            "DELETE FROM blocks WHERE block_id = ?1",
            vec![&child],
        ),
        (
            "wrong-ownership",
            "UPDATE blocks SET page_id = (SELECT page_id FROM pages WHERE name = 'Beta') \
             WHERE block_id = ?1",
            vec![&child],
        ),
        (
            "corrupt-parent-chain",
            "UPDATE blocks SET parent_block_id = ?2 WHERE block_id = ?1",
            vec![&child, &foreign],
        ),
        (
            "missing-result-row",
            "DELETE FROM query_block_results WHERE block_id = ?1",
            vec![&child],
        ),
    ];
    for (tag, damage, bind) in damages {
        match export_damaged(&corpus, tag, damage, &bind) {
            Err(ResultReadError::Corrupt(_)) => {}
            Err(other) => panic!("{tag}: expected corruption, got {other}"),
            Ok(answer) => panic!(
                "{tag}: a damaged projection exported {} nodes",
                emitted_nodes(&answer)
            ),
        }
    }
}

#[test]
fn a_missing_final_descendant_result_row_fails_instead_of_shortening_export() {
    let _serial = serialize();
    let root = scratch("ret3-export-tail-damage");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/Tail.md"),
        "- LATER selected root\n\t- final child\n",
    )
    .unwrap();
    let corpus = Corpus::open(root, true);
    let child = block_id_of(&corpus, "final child");
    let mut snapshot = corpus.snapshot();
    let healthy = export_over(
        &corpus,
        &mut snapshot,
        &[spec("later", "(task LATER)")],
        Caps::default(),
        &ResultIdentity::Stored,
    )
    .unwrap();
    snapshot.finish();
    assert_eq!(emitted_nodes(&healthy), 2);
    let damaged = export_damaged(
        &corpus,
        "missing-tail",
        "DELETE FROM query_block_results WHERE block_id = ?1",
        &[&child],
    );
    assert!(
        matches!(damaged, Err(ResultReadError::Corrupt(_))),
        "a missing final descendant must fail rather than shorten export: {damaged:?}"
    );
}

#[test]
fn multiple_missing_tail_result_rows_fail_instead_of_shortening_export() {
    let _serial = serialize();
    let root = scratch("ret3-export-multiple-tail-damage");
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::write(
        root.join("pages/MultipleTail.md"),
        "- LATER selected root\n\
         \t- retained child\n\
         \t- missing tail one\n\
         \t- missing tail two\n",
    )
    .expect("multiple-tail page");
    let corpus = Corpus::open(root, true);
    let first = block_id_of(&corpus, "missing tail one");
    let second = block_id_of(&corpus, "missing tail two");

    let damaged = export_damaged(
        &corpus,
        "multiple-missing-tail",
        "DELETE FROM query_block_results WHERE block_id IN (?1, ?2)",
        &[&first, &second],
    );
    assert!(
        matches!(damaged, Err(ResultReadError::Corrupt(_))),
        "multiple missing final descendants must fail rather than shorten export: {damaged:?}"
    );
}

#[test]
fn an_entire_missing_tail_branch_fails_instead_of_shortening_export() {
    let _serial = serialize();
    let root = scratch("ret3-export-missing-tail-branch");
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::write(
        root.join("pages/MissingBranch.md"),
        "- LATER selected root\n\
         \t- retained child\n\
         \t- missing branch root\n\
         \t\t- missing branch leaf\n",
    )
    .expect("missing-branch page");
    let corpus = Corpus::open(root, true);
    let branch = block_id_of(&corpus, "missing branch root");
    let leaf = block_id_of(&corpus, "missing branch leaf");

    let damaged = export_damaged(
        &corpus,
        "missing-tail-branch",
        "DELETE FROM query_block_results WHERE block_id IN (?1, ?2)",
        &[&branch, &leaf],
    );
    assert!(
        matches!(damaged, Err(ResultReadError::Corrupt(_))),
        "a wholly missing final branch must fail rather than shorten export: {damaged:?}"
    );
}

#[test]
fn a_missing_tail_outside_the_selected_subtree_does_not_poison_export() {
    let _serial = serialize();
    let root = scratch("ret3-export-unrelated-tail-damage");
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::write(
        root.join("pages/UnrelatedTail.md"),
        "- LATER selected root\n\
         \t- selected child\n\
         - unrelated root\n\
         \t- unrelated missing tail\n",
    )
    .expect("unrelated-tail page");
    let corpus = Corpus::open(root, true);
    let unrelated = block_id_of(&corpus, "unrelated missing tail");

    let read = export_damaged(
        &corpus,
        "unrelated-missing-tail",
        "DELETE FROM query_block_results WHERE block_id = ?1",
        &[&unrelated],
    )
    .expect("damage outside the selected subtree is irrelevant to this export");
    assert_eq!(read.results[0].shown, 1);
    assert_eq!(read.results[0].omitted_nodes, 0);
    assert_eq!(node_count(&read.results[0].groups[0].blocks), 2);
    assert_eq!(
        read.results[0].groups[0].blocks[0].children[0].raw,
        "selected child"
    );
}

#[test]
fn a_selected_zero_child_leaf_is_complete() {
    let _serial = serialize();
    let root = scratch("ret3-export-zero-child-leaf");
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::write(root.join("pages/Leaf.md"), "- LATER selected leaf\n").expect("leaf page");
    let corpus = Corpus::open(root, true);
    let mut snapshot = corpus.snapshot();
    reset_export_subtree_census();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &[spec("leaf", "(task LATER)")],
        Caps::default(),
        &ResultIdentity::Stored,
    )
    .expect("a selected leaf with no children is complete");
    snapshot.finish();
    let census = export_subtree_census();

    assert_eq!(emitted_nodes(&read), 1);
    assert_eq!(read.results[0].omitted_nodes, 0);
    assert_eq!(census.completeness_statements, 1);
    assert_eq!(census.completeness_parents, 1);
}

#[test]
fn a_cross_page_parent_pointer_fails_with_foreign_keys_disabled() {
    let _serial = serialize();
    let root = scratch("ret3-export-cross-page-parent");
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::write(
        root.join("pages/Selected.md"),
        "- LATER selected root\n\t- selected child\n",
    )
    .expect("selected page");
    std::fs::write(root.join("pages/Foreign.md"), "- foreign page block\n").expect("foreign page");
    let corpus = Corpus::open(root, true);
    let selected = block_id_of(&corpus, "selected root");
    let foreign = block_id_of(&corpus, "foreign page block");

    let damaged = export_damaged(
        &corpus,
        "cross-page-parent",
        "UPDATE blocks SET parent_block_id = ?1 WHERE block_id = ?2",
        &[&selected, &foreign],
    );
    assert!(
        matches!(damaged, Err(ResultReadError::Corrupt(_))),
        "a child on another page pointing at the selected parent must fail: {damaged:?}"
    );
}

#[test]
fn completeness_batches_exactly_128_parents_per_statement() {
    let _serial = serialize();
    for (parents, expected_statements) in [(128usize, 1usize), (129usize, 2usize)] {
        let root = scratch(&format!("ret3-export-completeness-batch-{parents}"));
        std::fs::create_dir_all(root.join("pages")).expect("pages");
        let mut page = String::from("- LATER selected root\n");
        for index in 1..parents {
            page.push_str(&format!("\t- child {index}\n"));
        }
        std::fs::write(root.join("pages/Batch.md"), page).expect("batch page");
        let corpus = Corpus::open(root, true);
        let mut snapshot = corpus.snapshot();
        reset_export_subtree_census();
        let read = export_over(
            &corpus,
            &mut snapshot,
            &[spec("batch", "(task LATER)")],
            Caps::default(),
            &ResultIdentity::Stored,
        )
        .expect("the complete wide subtree exports");
        snapshot.finish();
        let census = export_subtree_census();

        assert_eq!(emitted_nodes(&read), parents);
        assert_eq!(census.completeness_parents, parents);
        assert_eq!(
            census.completeness_statements, expected_statements,
            "{parents} discovered parents must use 128-parent statement batches"
        );
    }
}

#[test]
fn repeated_selected_roots_reuse_the_verified_topology() {
    let _serial = serialize();
    let root = scratch("ret3-export-repeated-root-cache");
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::write(
        root.join("pages/Repeated.md"),
        "- LATER repeated root\n\
         \t- first child\n\
         \t- second child\n",
    )
    .expect("repeated-root page");
    let corpus = Corpus::open(root, true);
    let specs = [
        spec("first occurrence", "(task LATER)"),
        spec("second occurrence", "(task LATER)"),
    ];
    let mut snapshot = corpus.snapshot();
    reset_export_subtree_census();
    let read = export_over(
        &corpus,
        &mut snapshot,
        &specs,
        Caps::default(),
        &ResultIdentity::Stored,
    )
    .expect("both occurrences export");
    snapshot.finish();
    let census = export_subtree_census();

    assert_eq!(read.results.len(), 2);
    assert!(read.results.iter().all(|result| result.shown == 1));
    assert!(read
        .results
        .iter()
        .all(|result| { node_count(&result.groups[0].blocks) == 3 && result.omitted_nodes == 0 }));
    assert_eq!(census.topology_statements, 1);
    assert_eq!(census.completeness_statements, 1);
    assert_eq!(
        census.completeness_parents, 3,
        "the root and two leaf parents are verified once, not once per occurrence"
    );
}

// ===== cancellation =====

#[test]
fn cancelling_between_completeness_batches_reads_no_descendant_payload() {
    let _serial = serialize();
    let root = scratch("ret3-export-cancel-completeness");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let mut page = String::from("- LATER selected root\n");
    for index in 0..128 {
        page.push_str(&format!("\t- child {index}\n"));
    }
    std::fs::write(root.join("pages/Batch.md"), page).unwrap();
    let corpus = Corpus::open(root, true);
    let mut snapshot = corpus.snapshot();
    let cancellation = snapshot.cancellation();
    reset_export_subtree_census();
    reset_result_read_census();
    set_before_completeness_batch_hook(Some(Box::new(move |batch| {
        if batch == 1 {
            cancellation.cancel();
        }
    })));
    let answer = export_over(
        &corpus,
        &mut snapshot,
        &[spec("batch", "(task LATER)")],
        Caps::default(),
        &ResultIdentity::Stored,
    );
    set_before_completeness_batch_hook(None);
    assert!(
        matches!(answer, Err(ResultReadError::Cancelled)),
        "{answer:?}"
    );
    let census = export_subtree_census();
    assert_eq!(census.completeness_statements, 1);
    assert_eq!(census.completeness_parents, 128);
    assert_eq!(result_read_census().export_payload_statements, 0);
    assert!(snapshot.run_projection_query("SELECT 1", &[]).is_err());
    snapshot.finish();
}

/// A job cancelled before the export starts reads nothing.
#[test]
fn a_cancelled_export_reads_nothing() {
    let _serial = serialize();
    let root = scratch("ret3-export-cancel-early");
    write_export_corpus(&root);
    let corpus = Corpus::open(root, true);
    let mut snapshot = corpus.snapshot();
    snapshot.cancellation().cancel();
    reset_export_subtree_census();
    let answer = export_over(
        &corpus,
        &mut snapshot,
        &[spec("todo", "(task TODO)")],
        Caps::default(),
        &ResultIdentity::Stored,
    );
    snapshot.finish();
    assert!(matches!(answer, Err(ResultReadError::Cancelled)));
    assert_eq!(export_subtree_census(), Default::default());
}

/// Cancellation BETWEEN output payload batches stops the export promptly, and
/// the caller's snapshot is what releases the transaction.
#[test]
fn cancelling_between_output_payload_batches_stops_the_export() {
    let _serial = serialize();
    let root = scratch("ret3-export-cancel-payload");
    // One requested root with 300 descendants, so the output payload takes
    // three batches of 128 and there is a boundary to cancel at.
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::create_dir_all(root.join("journals")).expect("journals");
    let mut page = String::from("- TODO wide root\n");
    for index in 0..300 {
        page.push_str(&format!("\t- wide child {index}\n"));
    }
    std::fs::write(root.join("pages/Wide.md"), page).expect("Wide page");
    let corpus = Corpus::open(root, true);
    let mut snapshot = corpus.snapshot();
    let cancellation = snapshot.cancellation();
    let (trigger, wait_for_trigger) = std::sync::mpsc::channel::<()>();
    let (acknowledge, wait_for_ack) = std::sync::mpsc::channel::<()>();
    let owner = std::thread::spawn(move || {
        if wait_for_trigger.recv().is_ok() {
            cancellation.cancel();
            let _ = acknowledge.send(());
        }
    });
    reset_result_read_census();
    set_before_export_payload_batch_hook(Some(Box::new(move |batch| {
        if batch == 1 {
            trigger.send(()).expect("the owner thread is listening");
            wait_for_ack.recv().expect("the owner cancels");
        }
    })));
    let answer = export_over(
        &corpus,
        &mut snapshot,
        &[spec("wide", "(task TODO)")],
        Caps::default(),
        &ResultIdentity::Stored,
    );
    set_before_export_payload_batch_hook(None);
    owner.join().expect("the owner thread finishes");
    let census = result_read_census();
    assert!(
        matches!(answer, Err(ResultReadError::Cancelled)),
        "a cancelled export must report Cancelled, got {answer:?}"
    );
    assert_eq!(
        census.export_payload_statements, 3,
        "exactly the first output batch's statements ran"
    );
    assert!(
        snapshot.run_projection_query("SELECT 1", &[]).is_err(),
        "a cancelled snapshot must not serve another statement"
    );
    snapshot.finish();
}
