//! Integration tests against the on-disk demo graph (standard layout).

use crate::model::Graph;
use crate::store::Store as InternalStore;
use crate::test_config_client::ConfigClient;
use std::path::PathBuf;
use std::sync::Arc;
use tine_graph_features::{conflicts, pages};
use tine_store::{Cancel, PageId, SaveBase, SaveOutcome, SearchRequest, Store};

fn demo_graph() -> Graph {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph");
    Graph::open(root)
}

fn store_at(root: &std::path::Path) -> Store {
    Store::open(root, tine_store::OpenOptions::default())
        .unwrap()
        .0
}

fn save_named(store: &Store, name: &str, edit: impl FnOnce(&mut tine_core::model::PageDto)) {
    let id = match store.whole_graph().unwrap().resolve(name, false) {
        tine_store::Resolved::Existing { id, .. } => id,
        _ => panic!("missing page {name}"),
    };
    let mut read = store.page(&id).unwrap();
    edit(&mut read.doc);
    assert!(matches!(
        store.save(&id, SaveBase::Existing(read.rev), &read.doc),
        SaveOutcome::Saved(_)
    ));
}

fn query_simple(store: &Store, source: &str) -> Arc<Vec<tine_core::model::RefGroup>> {
    match store
        .whole_graph()
        .unwrap()
        .query(source, tine_store::QueryDialect::Simple, None)
        .unwrap()
    {
        tine_store::QueryResult::Simple(groups) => groups,
        _ => unreachable!(),
    }
}

fn search_count(store: &Store, query: &str) -> usize {
    store
        .whole_graph()
        .unwrap()
        .search(
            &SearchRequest {
                text: query.into(),
                within: None,
                page_limit: 10,
                block_limit: 10,
                explain: false,
            },
            &Cancel(Arc::new(std::sync::atomic::AtomicBool::new(false))),
        )
        .unwrap()
        .hits
        .len()
}

#[test]
fn lists_journals_and_pages() {
    let g = demo_graph();
    let pages = g.list_pages();
    assert!(pages.iter().any(|p| p.name == "logseq-claude"));
    let journals = g.journals_desc();
    // Newest first.
    assert_eq!(
        journals.first().map(|j| j.name.as_str()),
        Some("Jun 14th, 2026")
    );
    assert!(journals.len() >= 2);
}

#[test]
fn loads_a_page_with_nesting_and_properties() {
    let g = demo_graph();
    let entry = g
        .find_entry("logseq-claude", tine_core::PageKind::Page)
        .unwrap();
    let dto = g.load_page(&entry).unwrap();
    assert_eq!(
        dto.pre_block.as_deref(),
        Some("title:: logseq-claude\ntags:: project, tooling")
    );
    // Has a nested child under the first block.
    assert!(dto.blocks[0].children.len() >= 1);
}

#[test]
fn backlinks_to_parameterized_complexity() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph");
    let store = store_at(&root);
    let groups = store
        .whole_graph()
        .unwrap()
        .backlinks("parameterized complexity")
        .unwrap();
    let pages: Vec<&str> = groups.iter().map(|gr| gr.page.as_str()).collect();
    // Referenced from the journal, logseq-claude, and n-fold IP.
    assert!(pages.contains(&"logseq-claude"), "pages: {pages:?}");
    assert!(pages.contains(&"n-fold IP"), "pages: {pages:?}");
}

#[test]
fn unlinked_references_include_plain_text_alias_mentions() {
    let root =
        std::env::temp_dir().join(format!("tine-unlinked-alias-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/20260713145345.md"),
        "alias:: 20260713150352\n\n- Alias target page\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/Alias Mention Test.md"),
        "- No matching text yet.\n- The alias is already linked as [[20260713150352]].\n",
    )
    .unwrap();

    let store = store_at(&root);
    assert!(
        store
            .whole_graph()
            .unwrap()
            .unlinked_references("20260713145345")
            .unwrap()
            .is_empty(),
        "warm the derived cache before the source edit"
    );
    save_named(&store, "Alias Mention Test", |page| {
        page.blocks[0].raw = "This block mentions 20260713150352 as plain text.".into();
    });
    let groups = store
        .whole_graph()
        .unwrap()
        .unlinked_references("20260713145345")
        .unwrap();
    assert_eq!(
        groups
            .iter()
            .map(|group| group.page.as_str())
            .collect::<Vec<_>>(),
        vec!["Alias Mention Test"],
        "an alias is another plain-text name for the canonical target: {groups:?}"
    );
    assert_eq!(groups[0].blocks.len(), 1, "linked aliases stay excluded");
    assert_eq!(
        groups[0].blocks[0].raw,
        "This block mentions 20260713150352 as plain text."
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn backlinks_include_explicit_links_in_page_properties() {
    let root = std::env::temp_dir().join(format!(
        "tine-page-property-backlink-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/A.md"),
        "created:: none\n\n- Page containing a linked reference inside a page property.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals/2026_07_13.md"),
        "- Reference target journal page.\n",
    )
    .unwrap();

    let store = store_at(&root);
    assert!(
        store
            .whole_graph()
            .unwrap()
            .backlinks("Jul 13th, 2026")
            .unwrap()
            .is_empty(),
        "warm the derived cache before the page-property edit"
    );
    save_named(&store, "A", |page| {
        page.pre_block = Some("created:: [[Jul 13th, 2026]]".into());
    });
    let groups = store
        .whole_graph()
        .unwrap()
        .backlinks("Jul 13th, 2026")
        .unwrap();
    let source = groups
        .iter()
        .find(|group| group.page == "A")
        .expect("page-property source should be a backlink group");
    assert_eq!(source.blocks.len(), 1, "one page entity counts once");
    assert_eq!(source.blocks[0].raw, "created:: [[Jul 13th, 2026]]");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn backlinks_include_bare_tags_page_properties() {
    let root = std::env::temp_dir().join(format!(
        "tine-page-property-tags-backlink-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/page1.md"), "- Reference target page.\n").unwrap();
    std::fs::write(
        root.join("pages/page2.md"),
        "tags:: unrelated\n\n- Page tagged with the target after load.\n",
    )
    .unwrap();

    let store = store_at(&root);
    assert!(
        store
            .whole_graph()
            .unwrap()
            .backlinks("page1")
            .unwrap()
            .is_empty(),
        "warm the derived cache before the bare tags property edit"
    );
    save_named(&store, "page2", |page| {
        page.pre_block = Some("tags:: page1".into());
    });

    let groups = store.whole_graph().unwrap().backlinks("page1").unwrap();
    let source = groups
        .iter()
        .find(|group| group.page == "page2")
        .expect("a bare tags:: value should create a Linked Reference group");
    assert_eq!(source.blocks.len(), 1, "one property source counts once");
    assert_eq!(source.blocks[0].raw, "tags:: page1");
    assert!(source.blocks[0].page_property);
    assert_eq!(source.evidence.len(), 1);
    assert_eq!(source.evidence[0].occurrences.len(), 1);
    let occurrence = &source.evidence[0].occurrences[0];
    assert_eq!(occurrence.matched_name, "page1");
    assert_eq!(occurrence.canonical, "page1");
    assert_eq!(occurrence.span.start, "tags:: ".encode_utf16().count());
    assert_eq!(occurrence.span.end, "tags:: page1".encode_utf16().count());
    assert_eq!(occurrence.rule, "implicit_linkable_property");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn block_ref_counts_and_referrers() {
    // Isolated temp graph: a target block (id:: aaaaaaaa-0000-0000-0000-000000000001) referenced by a same-page
    // block and three blocks on another page (labeled, embed, and a double ref that
    // must dedupe to one). Exercises all three OG block-ref forms + same-page
    // inclusion.
    let root = std::env::temp_dir().join(format!("tine-blockref-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages").join("Target.md"),
        "- the target block\n  id:: aaaaaaaa-0000-0000-0000-000000000001\n- see ((aaaaaaaa-0000-0000-0000-000000000001)) on this page\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Other.md"),
        "- ref via [label](((aaaaaaaa-0000-0000-0000-000000000001)))\n- embedded {{embed ((aaaaaaaa-0000-0000-0000-000000000001))}}\n- two ((aaaaaaaa-0000-0000-0000-000000000001)) and ((aaaaaaaa-0000-0000-0000-000000000001)) here\n",
    )
    .unwrap();

    let store = store_at(&root);

    // Count = distinct referrer blocks: 1 (same page) + 3 (Other) = 4. The double
    // ref on the last Other block counts once.
    let counts = store.whole_graph().unwrap().block_ref_counts();
    assert_eq!(
        counts.get("aaaaaaaa-0000-0000-0000-000000000001").copied(),
        Some(4),
        "counts: {counts:?}"
    );

    // Referrers grouped by page, and the same-page referrer IS included (unlike
    // page backlinks).
    let groups = store
        .whole_graph()
        .unwrap()
        .block_referrers("aaaaaaaa-0000-0000-0000-000000000001")
        .unwrap();
    let mut by_page: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for gr in groups.iter() {
        by_page.insert(gr.page.as_str(), gr.blocks.len());
    }
    assert_eq!(
        by_page.get("Target").copied(),
        Some(1),
        "same-page referrer included"
    );
    assert_eq!(
        by_page.get("Other").copied(),
        Some(3),
        "all 3 Other referrers"
    );

    // The target block itself is not a referrer of itself.
    let target_refs: Vec<&str> = groups
        .iter()
        .find(|gr| gr.page == "Target")
        .map(|gr| gr.blocks.iter().map(|b| b.raw.as_str()).collect())
        .unwrap_or_default();
    assert!(
        target_refs
            .iter()
            .all(|r| r.contains("see ((aaaaaaaa-0000-0000-0000-000000000001))")),
        "{target_refs:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn publishes_only_public_pages() {
    let root = std::env::temp_dir().join(format!("tine-publish-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("Shared.md"),
        "public:: true\n\n- hello, see [[Secret]]\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("Secret.md"), "- private notes\n").unwrap();

    let store = Store::open(&root, Default::default()).unwrap().0;
    let (dir, n) = tine_graph_features::publish::publish_html(&store).unwrap();
    assert_eq!(n, 1, "only the public page is published");
    let p = std::fs::read_to_string(std::path::Path::new(&dir).join("shared.html"))
        .unwrap_or_else(|error| panic!("published page under {dir:?}: {error}"));
    assert!(p.contains("<h1 class=\"page\">Shared</h1>"));
    assert!(p.contains("<a class=\"ref\""), "should link [[refs]]");
    // The private page must not be exported.
    assert!(!std::path::Path::new(&dir).join("secret.html").exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_cache_reflects_saves_and_deletes() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    // Isolated temp graph so we can mutate it freely.
    let root = std::env::temp_dir().join(format!("tine-cache-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages").join("Seed.md"), "- a seed block\n").unwrap();

    let store = Store::open(&root, Default::default()).unwrap().0;
    let cancel = tine_store::Cancel(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let find = || {
        store
            .whole_graph()
            .unwrap()
            .find_blocks("zonkwort", 10, &cancel)
            .unwrap()
    };
    assert_eq!(find().len(), 0, "token absent initially");

    // Saving a page with the token must be visible to a subsequent search
    // without any disk re-scan (cache upsert).
    let page = PageDto {
        name: "Fresh".into(),
        kind: PageKind::Page,
        title: "Fresh".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "contains zonkwort here".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,

        guide: false,
    };
    assert!(matches!(
        store.save(&PageId::from("pages/Fresh.md"), SaveBase::CreateNew, &page),
        SaveOutcome::Saved(_)
    ));
    let hits = find();
    assert_eq!(hits.len(), 1, "saved page should be searchable");
    assert_eq!(hits[0].page, "Fresh");

    // Deleting the page removes it from the cache too.
    pages::delete_page_expected(&store, "Fresh", PageKind::Page, None, None).unwrap();
    assert_eq!(find().len(), 0, "deleted page should drop out");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_ignores_hidden_property_metadata() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-search-meta-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let store = store_at(&root);

    // A block whose only occurrence of "qzxmeta" is in a property line (like an
    // id:: uuid or hl-color::) must NOT match — the user can't see it.
    let page = PageDto {
        name: "Meta".into(),
        kind: PageKind::Page,
        title: "Meta".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "a perfectly ordinary block\nsome-prop:: qzxmeta".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,

        guide: false,
    };
    let id = PageId::from("pages/Meta.md");
    assert!(matches!(
        store.save(&id, SaveBase::CreateNew, &page),
        SaveOutcome::Saved(_)
    ));
    assert_eq!(
        search_count(&store, "qzxmeta"),
        0,
        "token only in a property line should not be a search hit"
    );
    // But the visible body is still searchable.
    assert_eq!(
        search_count(&store, "ordinary"),
        1,
        "visible body still matches"
    );

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_preserves_file_format_no_churn() {
    let root = std::env::temp_dir().join(format!("tine-fmt-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Logseq style: no trailing newline. Plus one with a newline, and a
    // space-indented file (Tine emits tabs by default — must preserve spaces).
    let no_nl = "- alpha\n\t- beta";
    let with_nl = "- gamma\n";
    let spaces = "- root\n  - two-space child\n    - grandchild";
    std::fs::write(root.join("pages").join("A.md"), no_nl).unwrap();
    std::fs::write(root.join("pages").join("B.md"), with_nl).unwrap();
    std::fs::write(root.join("pages").join("C.md"), spaces).unwrap();

    let store = store_at(&root);
    // Load then save unchanged must be byte-identical (no churn): each file's
    // trailing-newline + indent convention is preserved.
    for name in ["A", "B", "C"] {
        let id = PageId::from(format!("pages/{name}.md"));
        let read = store.page(&id).unwrap();
        assert!(matches!(
            store.save(&id, SaveBase::Existing(read.rev), &read.doc),
            SaveOutcome::Unchanged(_) | SaveOutcome::Saved(_)
        ));
    }
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("A.md")).unwrap(),
        no_nl
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("B.md")).unwrap(),
        with_nl
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("C.md")).unwrap(),
        spaces
    );

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sheet_field_rename_org_saves_and_reparses_every_dependency() {
    use tine_core::model::Format;

    let root = std::env::temp_dir().join(format!(
        "tine-sheet-field-rename-org-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    let path = root.join("pages").join("Sheet.org");
    let before = [
        "* Table",
        ":PROPERTIES:",
        ":tine.view: table",
        ":tine.fields: severity=number;occurrence=number;detection=number",
        ":tine.formula.rpn: severity * occurrence * detection + if(label == \"occurrence\", formula.occurrence, 0)",
        ":tine.filter: occurrence > 1",
        ":tine.group-by: prop:occurrence",
        ":tine.col-aggregates: prop:occurrence=sum;prop:severity=max",
        ":END:",
        "** Row",
        ":PROPERTIES:",
        ":severity: 2",
        ":occurrence: 2",
        ":detection: 2",
        ":label: other",
        ":END:",
        "",
    ]
    .join("\n");
    std::fs::write(&path, &before).unwrap();

    // This DTO is the exact write shape produced by the frontend's already-unit-
    // tested rename plan. Exercise the guarded Store save, inspect bytes,
    // then reopen Store so this cannot pass on an in-memory document.
    let store = store_at(&root);
    let id = PageId::from("pages/Sheet.org");
    let read = store.page(&id).expect("org sheet");
    let mut page = read.doc;
    assert_eq!(page.format, Format::Org);
    assert!(!page.read_only, "canonical Org fixture must be writable");
    let owner = &mut page.blocks[0];
    owner.raw = owner
        .raw
        .replace(
            ":tine.fields: severity=number;occurrence=number;detection=number",
            ":tine.fields: severity=number;OCC=number;detection=number",
        )
        .replace(
            ":tine.formula.rpn: severity * occurrence * detection + if(label == \"occurrence\", formula.occurrence, 0)",
            ":tine.formula.rpn: severity * OCC * detection + if(label == \"occurrence\", formula.occurrence, 0)",
        )
        .replace(":tine.filter: occurrence > 1", ":tine.filter: OCC > 1")
        .replace(
            ":tine.group-by: prop:occurrence",
            ":tine.group-by: prop:OCC",
        )
        .replace(
            ":tine.col-aggregates: prop:occurrence=sum;prop:severity=max",
            ":tine.col-aggregates: prop:OCC=sum;prop:severity=max",
        );
    owner.children[0].raw = owner.children[0].raw.replace(":occurrence: 2", ":OCC: 2");
    assert!(matches!(
        store.save(&id, SaveBase::Existing(read.rev), &page),
        SaveOutcome::Saved(_)
    ));

    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(disk.contains(":tine.fields: severity=number;OCC=number;detection=number"));
    assert!(disk.contains(
        ":tine.formula.rpn: severity * OCC * detection + if(label == \"occurrence\", formula.occurrence, 0)"
    ));
    assert!(disk.contains(":tine.filter: OCC > 1"));
    assert!(disk.contains(":tine.group-by: prop:OCC"));
    assert!(disk.contains(":tine.col-aggregates: prop:OCC=sum;prop:severity=max"));
    assert!(disk.contains(":OCC: 2"));
    assert!(!disk.contains("occurrence=number"));
    assert!(!disk.contains("severity * occurrence * detection"));
    assert!(!disk.contains(":tine.filter: occurrence > 1"));
    assert!(!disk.contains("prop:occurrence"));
    assert!(!disk.contains(":occurrence: 2"));
    assert!(
        disk.contains("if(label == \"occurrence\", formula.occurrence, 0)"),
        "string literal and formula member are not field identities"
    );

    store.close();
    let reopened_store = store_at(&root);
    let reopened = reopened_store.page(&id).expect("reparsed org sheet").doc;
    assert_eq!(reopened.format, Format::Org);
    assert!(
        !reopened.read_only,
        "renamed Org bytes must remain writable"
    );
    assert_eq!(reopened.blocks[0].raw, page.blocks[0].raw);
    assert_eq!(
        reopened.blocks[0].children[0].raw,
        page.blocks[0].children[0].raw
    );
    assert!(
        reopened.blocks[0]
            .properties
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("tine.fields")
                && value.contains("OCC=number")),
        "fresh parser recognizes the renamed schema"
    );
    assert!(
        reopened.blocks[0].children[0]
            .properties
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("OCC") && value == "2"),
        "fresh parser recognizes the renamed row property"
    );

    reopened_store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_refuses_to_clobber_external_change() {
    use crate::store::{SaveBase, SaveOutcome};
    let root = std::env::temp_dir().join(format!("tine-conflict-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Production `Store::open` binds the canonical root; the write-target
    // guard compares canonical targets against it. On Windows and macOS the
    // temp dir is not canonical (`\\?\`/8.3 names, `/var` -> `/private/var`).
    let root = std::fs::canonicalize(&root).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();

    let g = Arc::new(Graph::open(&root));
    let store = InternalStore::from_graph_for_tests(Arc::clone(&g));
    let id = PageId::from("pages/N.md");
    // Build the cache (Tine now "knows" N = "- one"), then load it for editing.
    g.search("one", 10);
    let read = store.page(&id).unwrap();
    let dto = read.doc;

    // An external writer (another app / Syncthing) changes the file.
    crate::test_fixture_io::atomic_write(&path, "- EXTERNAL EDIT").unwrap();

    // Saving the now-stale page must fail with a conflict and NOT overwrite.
    let outcome = store.save(&id, SaveBase::Existing(read.rev), &dto);
    assert!(
        matches!(outcome, SaveOutcome::Conflict { .. }),
        "stale save over an external edit: {outcome:?}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- EXTERNAL EDIT");

    // "Keep mine" force-saves over it.
    let (_, disk_rev) = store.read(&id.file(), None).unwrap();
    assert!(matches!(
        store.save(&id, SaveBase::Existing(disk_rev), &dto),
        SaveOutcome::Saved(_)
    ));
    assert!(std::fs::read_to_string(&path).unwrap().contains("one"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_conflicts_when_file_deleted_externally() {
    use crate::store::{SaveBase, SaveOutcome};
    let root = std::env::temp_dir().join(format!("tine-del-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Canonical like production `Store::open` (see save_refuses_to_clobber_external_change).
    let root = std::fs::canonicalize(&root).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();
    let g = Arc::new(Graph::open(&root));
    let store = InternalStore::from_graph_for_tests(Arc::clone(&g));
    let id = PageId::from("pages/N.md");
    g.search("one", 10); // warm cache
    let read = store.page(&id).unwrap();

    // The file is deleted on disk (Syncthing / Logseq) after we loaded it.
    std::fs::remove_file(&path).unwrap();

    // Saving must conflict, NOT silently resurrect the deleted note.
    let outcome = store.save(&id, SaveBase::Existing(read.rev), &read.doc);
    assert!(
        matches!(outcome, SaveOutcome::Deleted),
        "stale save over an external delete: {outcome:?}"
    );
    assert!(
        !path.exists(),
        "deleted file must stay deleted on a conflicting save"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn load_reflects_external_change_then_save_is_clean() {
    let root = std::env::temp_dir().join(format!("tine-reconcile-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();
    let store = store_at(&root);
    let id = PageId::from("pages/N.md");
    let _ = store.whole_graph().unwrap();
    let _ = store.page(&id).unwrap();

    // External writer changes the file; the 3s watcher hasn't run yet.
    std::fs::write(&path, "- TWO external").unwrap();

    // Store::page must serve the NEW content (not the stale cache),
    // and the rev it returns must match disk so a save doesn't spuriously conflict.
    let read = store.page(&id).unwrap();
    assert!(
        read.doc.blocks[0].raw.contains("TWO external"),
        "load reflects external change"
    );
    assert!(matches!(
        store.save(&id, SaveBase::Existing(read.rev), &read.doc),
        SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_)
    ));
    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn consecutive_self_saves_do_not_conflict() {
    // Regression: inserting a SCHEDULED date then deleting it (consecutive saves
    // with no external writer) must not raise a spurious "changed on disk"
    // conflict. Each save returns the new baseline rev, which the next save passes
    // back (exactly what the frontend does) — so our own write is never mistaken
    // for an external edit.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-selfsave-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let store = store_at(&root);
    let mk = |raw: &str| PageDto {
        name: "D".into(),
        kind: PageKind::Page,
        title: "D".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,

        guide: false,
    };
    // 1) date picker inserts a SCHEDULED line (page is new — no baseline yet).
    let id = PageId::from("pages/D.md");
    let r1 = match store.save(
        &id,
        SaveBase::CreateNew,
        &mk("TODO task\nSCHEDULED: <2026-06-16 Tue>"),
    ) {
        SaveOutcome::Saved(rev) => rev,
        outcome => panic!("first save failed: {outcome:?}"),
    };
    // 2) user deletes the inserted text — must NOT be read as an external edit.
    let r2 = match store.save(&id, SaveBase::Existing(r1), &mk("TODO task")) {
        SaveOutcome::Saved(rev) => rev,
        outcome => panic!("second save failed: {outcome:?}"),
    };
    // 3) and a further edit still saves cleanly.
    assert!(matches!(
        store.save(&id, SaveBase::Existing(r2), &mk("TODO task edited")),
        SaveOutcome::Saved(_)
    ));
    assert!(std::fs::read_to_string(root.join("pages").join("D.md"))
        .unwrap()
        .contains("edited"));

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sync_file_detects_external_change_and_suppresses_self() {
    let root = std::env::temp_dir().join(format!("tine-sync-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("S.md");
    std::fs::write(&path, "- before").unwrap();

    let store = store_at(&root);
    let id = PageId::from("pages/S.md");
    let initial = store.whole_graph().unwrap().rev();

    // No external change yet → sync reports nothing.
    store.scan_refresh().unwrap();
    assert_eq!(store.whole_graph().unwrap().rev(), initial);

    // External edit → sync reports the entry and refreshes the cache.
    std::fs::write(&path, "- after the change").unwrap();
    store.scan_refresh().unwrap();
    let changed = store.whole_graph().unwrap();
    assert!(changed.rev() > initial, "external change detected");
    assert_eq!(store.page(&id).unwrap().doc.name, "S");
    assert_eq!(store.page(&id).unwrap().doc.kind, tine_core::PageKind::Page);
    assert_eq!(
        search_count(&store, "after"),
        1,
        "cache updated to new content"
    );
    assert_eq!(search_count(&store, "before"), 0);

    // Re-syncing the same content is a no-op (self-write suppression).
    store.scan_refresh().unwrap();
    assert_eq!(store.whole_graph().unwrap().rev(), changed.rev());

    // Deletion is reported and drops it from the cache.
    std::fs::remove_file(&path).unwrap();
    store.scan_refresh().unwrap();
    assert!(store.whole_graph().unwrap().rev() > changed.rev());
    assert!(matches!(
        store.page(&id),
        Err(tine_store::StoreError::NotFound)
    ));
    assert_eq!(search_count(&store, "after"), 0);

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn noop_save_does_not_bump_cache_generation() {
    // A save whose serialized bytes match disk (focus/blur, forced flush of an
    // unchanged page) must NOT bump cache_gen — that key invalidates every
    // memoized query/backlink/derived result, so a no-op re-save would force a
    // whole-graph requery on every open dashboard. A real edit still bumps it.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-noopgen-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let store = store_at(&root);
    let _ = store.whole_graph().unwrap();
    let mk = |raw: &str| PageDto {
        name: "N".into(),
        kind: PageKind::Page,
        title: "N".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,

        guide: false,
    };
    let id = PageId::from("pages/N.md");
    let r1 = match store.save(&id, SaveBase::CreateNew, &mk("hello")) {
        SaveOutcome::Saved(rev) => rev,
        outcome => panic!("first save failed: {outcome:?}"),
    };
    let rev1 = store.whole_graph().unwrap().rev();
    // Re-save byte-identical content (no-op) with the returned baseline.
    let r2 = match store.save(&id, SaveBase::Existing(r1.clone()), &mk("hello")) {
        SaveOutcome::Unchanged(rev) => rev,
        outcome => panic!("unchanged save failed: {outcome:?}"),
    };
    assert_eq!(r1, r2, "rev must be stable across a no-op save");
    assert_eq!(
        store.whole_graph().unwrap().rev(),
        rev1,
        "no-op save must not publish a new graph view"
    );
    // A real edit DOES bump it.
    assert!(matches!(
        store.save(&id, SaveBase::Existing(r2), &mk("hello world")),
        SaveOutcome::Saved(_)
    ));
    assert!(
        store.whole_graph().unwrap().rev() > rev1,
        "a real edit must publish a new graph view"
    );

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn self_write_marker_does_not_outlive_its_save() {
    // The self-write marker only covers the rename→cache_upsert window and is
    // dropped by the writer once the write is published, so it can't linger and
    // later suppress a REAL external change that restores Tine's earlier bytes
    // (a delete+recreate). Before this fix, the stale marker made
    // the recreate look like our own write and it was silently dropped.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-marker-life-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let store = store_at(&root);
    let _ = store.whole_graph().unwrap();
    let page = PageDto {
        name: "C".into(),
        kind: PageKind::Page,
        title: "C".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: "noted".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,

        guide: false,
    };
    let id = PageId::from("pages/C.md");
    assert!(matches!(
        store.save(&id, SaveBase::CreateNew, &page),
        SaveOutcome::Saved(_)
    ));
    let path = root.join("pages").join("C.md");
    let saved_bytes = std::fs::read(&path).unwrap();
    let saved_rev = store.whole_graph().unwrap().rev();
    std::fs::remove_file(&path).unwrap();
    store.scan_refresh().unwrap();
    let removed_rev = store.whole_graph().unwrap().rev();
    assert!(removed_rev > saved_rev, "page deletion must be published");
    assert!(matches!(
        store.page(&id),
        Err(tine_store::StoreError::NotFound)
    ));
    std::fs::write(&path, saved_bytes).unwrap();
    store.scan_refresh().unwrap();
    assert!(
        store.whole_graph().unwrap().rev() > removed_rev,
        "a stale self-write marker must not suppress the page reappearing"
    );
    assert_eq!(store.page(&id).unwrap().doc.blocks[0].raw, "noted");

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn disk_rev_fast_path_is_fresh_and_detects_external_change() {
    // B1: an unchanged file syncs to a no-op via the disk_rev fast-path (no
    // reparse), but a genuine external edit is still detected — the rev must
    // never mask a change, and the served content must update.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-diskrev-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let store = store_at(&root);
    let _ = store.whole_graph().unwrap();
    let page = PageDto {
        name: "R".into(),
        kind: PageKind::Page,
        title: "R".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: "alpha".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,

        guide: false,
    };
    let id = PageId::from("pages/R.md");
    assert!(matches!(
        store.save(&id, SaveBase::CreateNew, &page),
        SaveOutcome::Saved(_)
    ));
    let path = root.join("pages").join("R.md");
    let saved = store.whole_graph().unwrap().rev();
    store.scan_refresh().unwrap();
    assert_eq!(
        store.whole_graph().unwrap().rev(),
        saved,
        "unchanged save → suppressed via disk_rev fast-path"
    );
    store.scan_refresh().unwrap();
    assert_eq!(
        store.whole_graph().unwrap().rev(),
        saved,
        "still unchanged → disk_rev fast-path again"
    );

    // A real external edit must still be detected (not masked by disk_revs).
    std::fs::write(&path, "- beta\n").unwrap();
    store.scan_refresh().unwrap();
    assert!(
        store.whole_graph().unwrap().rev() > saved,
        "external change detected despite disk_revs entry"
    );
    assert_eq!(store.page(&id).unwrap().doc.name, "R");
    // The cache now serves the new content (load_page goes through sync_file_content).
    let dto = store.page(&id).unwrap().doc;
    assert!(
        dto.blocks.iter().any(|b| b.raw.contains("beta")),
        "served stale after external edit"
    );

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn self_write_is_not_reported_as_external_change() {
    // Regression for the false "changed on disk" conflict seen during normal
    // typing (no external writer): a watcher poll after Tine's own save must not
    // report a false external change. Post-save, disk_revs reflects the write and
    // suppresses the poll (the short-lived marker covers only the in-flight
    // window). Uses the multi-line `> quote` shape that surfaced the original bug.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-selfwrite-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let store = store_at(&root);
    let _ = store.whole_graph().unwrap();

    let path = root.join("pages").join("W.md");
    let page = PageDto {
        name: "W".into(),
        kind: PageKind::Page,
        title: "W".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: "hello\n> quote".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,

        guide: false,
    };
    let id = PageId::from("pages/W.md");
    assert!(matches!(
        store.save(&id, SaveBase::CreateNew, &page),
        SaveOutcome::Saved(_)
    ));
    assert!(std::fs::read_to_string(&path).unwrap().contains("hello"));

    // A watcher poll right after our own save must emit nothing.
    let saved = store.whole_graph().unwrap().rev();
    store.scan_refresh().unwrap();
    assert_eq!(
        store.whole_graph().unwrap().rev(),
        saved,
        "Tine's own save must not be reported as an external change"
    );

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn query_between_filters_by_journal_date() {
    let root = std::env::temp_dir().join(format!("tine-between-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("journals").join("2022_06_15.md"),
        "- TODO [[scs]] recent\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2019_01_01.md"),
        "- TODO [[scs]] old\n",
    )
    .unwrap();

    let store = store_at(&root);
    let groups = query_simple(
        &store,
        "(and (task TODO) (and [[scs]] (between [[Jan 1st, 2021]] [[Jan 1st, 2100]])))",
    );
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(
        raws.iter().any(|r| r.contains("recent")),
        "in-range journal matches: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("old")),
        "out-of-range journal excluded: {raws:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_page_moves_file_and_updates_refs() {
    let root = std::env::temp_dir().join(format!("tine-rename-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Old Name.md"), "- the page body\n").unwrap();
    std::fs::write(
        root.join("pages").join("Other.md"),
        "- see [[Old Name]] and #[[Old Name]]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- ref [[Old Name]]\n",
    )
    .unwrap();

    pages::rename_page_expected(
        &Store::open(&root, Default::default()).unwrap().0,
        "Old Name",
        "New Name",
        None,
    )
    .unwrap();

    // File moved.
    assert!(!root.join("pages").join("Old Name.md").exists());
    assert!(root.join("pages").join("New Name.md").exists());
    // References rewritten everywhere.
    let other = std::fs::read_to_string(root.join("pages").join("Other.md")).unwrap();
    assert!(other.contains("[[New Name]]"), "{other}");
    assert!(other.contains("#[[New Name]]"), "{other}");
    assert!(!other.contains("Old Name"), "{other}");
    let journal = std::fs::read_to_string(root.join("journals").join("2026_06_15.md")).unwrap();
    assert!(journal.contains("[[New Name]]"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_cascades_namespace_and_rewrites_self_refs() {
    // F2 (namespace cascade) + F3 (self-refs): renaming `Proj` must move every
    // `Proj/*` page to `Renamed/*`, rewrite refs to all of them everywhere, and
    // rewrite the renamed pages' OWN refs.
    let root = std::env::temp_dir().join(format!("tine-ns-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    // These namespace pages use the `___` separator, so the graph must be pinned
    // to :triple-lowbar (the modern format) — otherwise (legacy default) `___`
    // isn't a separator and the cascade wouldn't see them as `Proj/*` children.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Proj.md"),
        "- root, see [[Proj]] self\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Proj___Child.md"),
        "- child of [[Proj]]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Proj___Child___Deep.md"),
        "- deep\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Other.md"),
        "- [[Proj]] and [[Proj/Child]]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- note [[Proj/Child/Deep]]\n",
    )
    .unwrap();

    pages::rename_page_expected(
        &Store::open(&root, Default::default()).unwrap().0,
        "Proj",
        "Renamed",
        None,
    )
    .unwrap();

    let p = root.join("pages");
    // Subtree moved.
    assert!(!p.join("Proj.md").exists());
    assert!(!p.join("Proj___Child.md").exists());
    assert!(!p.join("Proj___Child___Deep.md").exists());
    assert!(p.join("Renamed.md").exists());
    assert!(p.join("Renamed___Child.md").exists());
    assert!(p.join("Renamed___Child___Deep.md").exists());
    // F3: the renamed page's own self-ref rewritten.
    let renamed = std::fs::read_to_string(p.join("Renamed.md")).unwrap();
    assert!(renamed.contains("[[Renamed]]"), "self-ref: {renamed}");
    // Child's ref to its parent rewritten.
    let child = std::fs::read_to_string(p.join("Renamed___Child.md")).unwrap();
    assert!(child.contains("[[Renamed]]"), "child→parent ref: {child}");
    // Refs everywhere rewritten, parent and namespaced child both.
    let other = std::fs::read_to_string(p.join("Other.md")).unwrap();
    assert!(
        other.contains("[[Renamed]]") && other.contains("[[Renamed/Child]]"),
        "{other}"
    );
    assert!(!other.contains("Proj"), "{other}");
    let journal = std::fs::read_to_string(root.join("journals").join("2026_06_15.md")).unwrap();
    assert!(journal.contains("[[Renamed/Child/Deep]]"), "{journal}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn legacy_namespace_file_round_trips() {
    // No :file/name-format ⇒ legacy (OG's default). A `%2F`-encoded namespace
    // file (what OG writes on a legacy graph) must be discoverable under its
    // slashed name and save back to the SAME file — never silently forked into a
    // `___` twin. This is the G1/G2 fix: Tine used to always use/expect `___`.
    let root = std::env::temp_dir().join(format!("tine-ns-legacy-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("math%2Falgebra.md"),
        "- legacy ns page\n",
    )
    .unwrap();

    let store = store_at(&root);
    // Discoverable under the decoded, slashed name.
    let id = match store.whole_graph().unwrap().resolve("math/algebra", false) {
        tine_store::Resolved::Existing { id, .. } => id,
        _ => panic!("legacy %2F file should resolve under its slashed name"),
    };
    let read = store.page(&id).unwrap();
    assert_eq!(read.doc.name, "math/algebra");
    // An edited save round-trips to the SAME file; no `___` twin appears.
    assert!(matches!(
        store.save(&id, SaveBase::Existing(read.rev), &read.doc),
        SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_)
    ));
    assert!(root.join("pages").join("math%2Falgebra.md").exists());
    assert!(
        !root.join("pages").join("math___algebra.md").exists(),
        "must not fork a triple-lowbar twin"
    );

    store.close();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_rewrites_bare_tags_property() {
    // F1: a page referencing the renamed page only through a bare `tags::` value
    // (no inline [[..]]) must have that value rewritten; siblings preserved.
    let root = std::env::temp_dir().join(format!("tine-tags-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Old.md"), "- old page body\n").unwrap();
    std::fs::write(
        root.join("pages").join("Note.md"),
        "tags:: Old, keep\n\n- body\n",
    )
    .unwrap();

    pages::rename_page_expected(
        &Store::open(&root, Default::default()).unwrap().0,
        "Old",
        "New",
        None,
    )
    .unwrap();

    assert!(!root.join("pages").join("Old.md").exists());
    assert!(root.join("pages").join("New.md").exists());
    let note = std::fs::read_to_string(root.join("pages").join("Note.md")).unwrap();
    assert!(
        note.contains("tags:: New, keep"),
        "bare tag rewritten + sibling kept: {note}"
    );
    assert!(!note.contains("Old"), "{note}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_aborts_on_target_collision_without_changes() {
    // Renaming onto an existing page name aborts with NO change (Tine doesn't
    // merge). Both files must be byte-identical afterwards.
    let root = std::env::temp_dir().join(format!("tine-collide-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("A.md"), "- a body [[B]]\n").unwrap();
    std::fs::write(root.join("pages").join("B.md"), "- b body\n").unwrap();

    assert!(
        pages::rename_page_expected(
            &Store::open(&root, Default::default()).unwrap().0,
            "A",
            "B",
            None
        )
        .is_err(),
        "rename onto existing page must fail"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("A.md")).unwrap(),
        "- a body [[B]]\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("B.md")).unwrap(),
        "- b body\n"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_ref_only_page_rewrites_refs_without_a_file() {
    // A page that exists only via references (no file of its own) still has its
    // refs rewritten across the graph.
    let root = std::env::temp_dir().join(format!("tine-refonly-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("Ref.md"),
        "- mentions [[Ghost]] here\n",
    )
    .unwrap();

    pages::rename_page_expected(
        &Store::open(&root, Default::default()).unwrap().0,
        "Ghost",
        "Spirit",
        None,
    )
    .unwrap();

    assert!(!root.join("pages").join("Ghost.md").exists());
    let r = std::fs::read_to_string(root.join("pages").join("Ref.md")).unwrap();
    assert!(r.contains("[[Spirit]]") && !r.contains("Ghost"), "{r}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn query_and_not_includes_everything_except_excluded() {
    // (and (task TODO) (not [[X]])) must return ALL TODO blocks that don't
    // reference [[X]] — regression for "NOT excludes right but drops others".
    let root = std::env::temp_dir().join(format!("tine-not-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages").join("P.md"),
        "- TODO alpha\n- TODO beta [[X]]\n- TODO gamma\n- DONE delta\n",
    )
    .unwrap();
    let store = store_at(&root);
    let raws: Vec<String> = query_simple(&store, "(and (task TODO) (not [[X]]))")
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(raws.iter().any(|r| r.contains("alpha")), "{raws:?}");
    assert!(raws.iter().any(|r| r.contains("gamma")), "{raws:?}");
    assert!(
        !raws.iter().any(|r| r.contains("beta")),
        "X-referencing excluded: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("delta")),
        "non-TODO excluded: {raws:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn page_aliases_resolve_and_collect_backlinks() {
    use tine_core::model::PageKind;
    let root = std::env::temp_dir().join(format!("tine-alias-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Canonical page "Parameterized Complexity" with an alias "PC".
    std::fs::write(
        root.join("pages").join("Parameterized Complexity.md"),
        "alias:: PC\n\n- the canonical page\n",
    )
    .unwrap();
    // One page links the canonical name, another links the alias.
    std::fs::write(
        root.join("pages").join("A.md"),
        "- see [[Parameterized Complexity]]\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("B.md"), "- via [[PC]]\n").unwrap();

    let g = Graph::open(&root);
    // Loading the alias resolves to the canonical page.
    let dto = g
        .load_named("PC", PageKind::Page)
        .unwrap()
        .expect("alias resolves");
    assert!(dto.blocks.iter().any(|b| b.raw.contains("canonical page")));
    // Backlinks of the canonical page include the alias-referencing page.
    let store = store_at(&root);
    let pages: Vec<String> = store
        .whole_graph()
        .unwrap()
        .backlinks("Parameterized Complexity")
        .unwrap()
        .iter()
        .map(|gr| gr.page.clone())
        .collect();
    assert!(pages.contains(&"A".to_string()), "{pages:?}");
    assert!(
        pages.contains(&"B".to_string()),
        "alias ref counted: {pages:?}"
    );
    // Backlinks queried via the alias name also resolve to the canonical set.
    let via_alias: Vec<String> = store
        .whole_graph()
        .unwrap()
        .backlinks("PC")
        .unwrap()
        .iter()
        .map(|gr| gr.page.clone())
        .collect();
    assert!(
        via_alias.contains(&"A".to_string()) && via_alias.contains(&"B".to_string()),
        "{via_alias:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_favorites_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-fav-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Existing config with another key that must be preserved + a COMMENTED
    // :favorites decoy that must NOT be edited (the write must target the real key).
    std::fs::write(root.join("logseq").join("config.edn"),
        "{;; :favorites [\"example\"]\n :preferred-workflow :now\n :journals-directory \"journals\"}\n").unwrap();

    let g = Graph::open(&root);
    g.set_favorites(&["Inbox".into(), "Reading List".into()])
        .unwrap();
    // Re-open and confirm favorites parsed back + the other key survived.
    let g2 = Graph::open(&root);
    assert_eq!(
        g2.meta().favorites,
        vec!["Inbox".to_string(), "Reading List".to_string()]
    );
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert!(
        cfg.contains(":journals-directory"),
        "other keys preserved: {cfg}"
    );
    assert!(
        cfg.contains(";; :favorites [\"example\"]"),
        "commented decoy untouched: {cfg}"
    );

    // Updating again replaces (not appends) the vector.
    g2.set_favorites(&["Only One".into()]).unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["Only One".to_string()]
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_favorites_edn_aware_vector_end() {
    // Round-6 audit: the end of `:favorites [...]` was found with the first raw
    // `]`, so a favorite NAME containing `]` truncated the replacement and left a
    // corrupt fragment. The end scan is now EDN-aware (skips strings/escapes).
    let root = std::env::temp_dir().join(format!("tine-fav-edn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    // Existing vector whose FIRST entry contains a `]`, plus a sibling key.
    std::fs::write(
        &cfg_path,
        "{:favorites [\"A]B\" \"C\"]\n :journals-directory \"journals\"}\n",
    )
    .unwrap();

    Graph::open(&root).set_favorites(&["Only".into()]).unwrap();
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["Only".to_string()]
    );
    assert!(
        cfg.contains(":journals-directory"),
        "sibling preserved: {cfg}"
    );
    // The whole old vector is gone — no truncation fragment left behind.
    assert!(
        !cfg.contains("A]B"),
        "old first entry fully replaced: {cfg}"
    );
    assert!(
        !cfg.contains("\"C\""),
        "old second entry gone (no leftover): {cfg}"
    );
    assert_eq!(
        cfg.matches(":favorites").count(),
        1,
        "exactly one :favorites: {cfg}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_preferred_workflow_ignores_key_inside_string_literal() {
    // Round-7 audit: the key was located with a non-string-aware scan, so a
    // `:preferred-workflow` inside a string value could be edited instead of the
    // real key. `find_keyword` now skips strings.
    let root = std::env::temp_dir().join(format!("tine-wf-str-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    // A string value mentions the key; the REAL key is separate and says :now.
    std::fs::write(
        &cfg_path,
        "{:note \":preferred-workflow :now\"\n :preferred-workflow :now}\n",
    )
    .unwrap();

    Graph::open(&root).set_preferred_workflow("todo").unwrap();
    let c = std::fs::read_to_string(&cfg_path).unwrap();
    // The string decoy is untouched; the REAL (non-string) key flipped to :todo.
    // (Asserted on file content: the matching READER `keyword_value` isn't
    // string-aware for key LOCATION — a separate, far more pathological case
    // codex did not flag — so we verify the writer targeted the right key here.)
    assert!(
        c.contains("\":preferred-workflow :now\""),
        "string decoy untouched: {c}"
    );
    assert!(
        c.contains(":preferred-workflow :todo"),
        "real key flipped in file: {c}"
    );
    assert_eq!(
        c.matches(":todo").count(),
        1,
        "exactly the real value changed: {c}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn config_readers_are_edn_aware_for_bracket_brace_in_values() {
    // Round-7 audit: the hardened writers can now emit a `]`/`}` inside a string
    // value (a favorited/templated page titled `f[x]` / `Plan {B}`); the readers
    // used delimiter scans that truncated at the first raw `]`/`}` and silently
    // lost the value on reload. Writer→file→reader must now round-trip.
    let root = std::env::temp_dir().join(format!("tine-cfg-read-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();

    // Favorites with a `]` in a name (and a plain sibling) survive a reload.
    Graph::open(&root)
        .set_favorites(&["arr[0]".into(), "plain".into()])
        .unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["arr[0]".to_string(), "plain".to_string()]
    );

    // Default journal template name containing `}` survives a reload.
    Graph::open(&root)
        .set_default_journal_template(Some("Plan {B}"))
        .unwrap();
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("Plan {B}")
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn config_reader_preserves_semicolon_in_string_values() {
    // Round-8 audit: comment stripping cut every line at the first `;`, even
    // inside a string, so a favorited/templated page named `A;B` reloaded as `A`.
    // Comment stripping is now string-aware.
    let root = std::env::temp_dir().join(format!("tine-cfg-semi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();

    Graph::open(&root)
        .set_favorites(&["A;B".into(), "C".into()])
        .unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["A;B".to_string(), "C".to_string()]
    );

    Graph::open(&root)
        .set_default_journal_template(Some("Plan;B"))
        .unwrap();
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("Plan;B")
    );

    // A real (line-start) comment is still stripped — the decoy must not be read.
    let cfg_path = root.join("logseq").join("config.edn");
    let cur = std::fs::read_to_string(&cfg_path).unwrap();
    std::fs::write(
        &cfg_path,
        format!(";; :journals-directory \"DECOY\"\n{cur}"),
    )
    .unwrap();
    assert_ne!(
        Graph::open(&root).config.journals_dir,
        "DECOY",
        "line comment still stripped"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn editor_structure_preferences_round_trip_through_config_edn() {
    let root = std::env::temp_dir().join(format!(
        "tine-editor-structure-config-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let config = root.join("logseq").join("config.edn");
    std::fs::write(&config, "{:unrelated true}\n").unwrap();

    let graph = Graph::open(&root);
    assert!(!graph.meta().doc_mode_enter_for_new_block);
    assert!(!graph.meta().logical_outdenting);
    graph.set_doc_mode_enter_for_new_block(true).unwrap();
    graph.set_logical_outdenting(true).unwrap();

    let reopened = Graph::open(&root);
    assert!(reopened.meta().doc_mode_enter_for_new_block);
    assert!(reopened.meta().logical_outdenting);
    let persisted = std::fs::read_to_string(&config).unwrap();
    assert!(persisted.contains(":shortcut/doc-mode-enter-for-new-block? true"));
    assert!(persisted.contains(":editor/logical-outdenting? true"));
    assert!(persisted.contains(":unrelated true"));

    reopened.set_doc_mode_enter_for_new_block(false).unwrap();
    reopened.set_logical_outdenting(false).unwrap();
    let reopened_again = Graph::open(&root);
    assert!(!reopened_again.meta().doc_mode_enter_for_new_block);
    assert!(!reopened_again.meta().logical_outdenting);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_preferred_workflow_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-wf-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Existing config: another key + a *commented* decoy that must NOT be edited.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{;; :preferred-workflow :todo (example)\n :preferred-workflow :now\n :start-of-week 1}\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    assert_eq!(g.meta().preferred_workflow, "now");
    g.set_preferred_workflow("todo").unwrap();

    let g2 = Graph::open(&root);
    assert_eq!(g2.meta().preferred_workflow, "todo");
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert!(
        cfg.contains(":start-of-week 1"),
        "other keys preserved: {cfg}"
    );
    // The commented decoy line is untouched (still says :todo (example)).
    assert!(
        cfg.contains(";; :preferred-workflow :todo (example)"),
        "comment preserved: {cfg}"
    );

    // Flipping back replaces (not appends) the keyword.
    g2.set_preferred_workflow("now").unwrap();
    assert_eq!(Graph::open(&root).meta().preferred_workflow, "now");
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert_eq!(
        cfg.matches(":preferred-workflow :now").count(),
        1,
        "no duplicate key: {cfg}"
    );

    // Inserting into a config that lacks the key entirely.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{:start-of-week 0}\n",
    )
    .unwrap();
    let g3 = Graph::open(&root);
    g3.set_preferred_workflow("todo").unwrap();
    assert_eq!(Graph::open(&root).meta().preferred_workflow, "todo");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-jtmpl-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    // Existing config: a sibling key + a *commented* decoy that must NOT be edited.
    std::fs::write(
        &cfg_path,
        "{;; :default-templates {:journals \"Commented\"}\n :start-of-week 1}\n",
    )
    .unwrap();
    let cfg = || std::fs::read_to_string(&cfg_path).unwrap();
    let jtmpl = || Graph::open(&root).meta().default_journal_template;

    let g = Graph::open(&root);
    assert_eq!(jtmpl(), None, "unset to begin with");

    // Set (key absent → inserted, NOT touching the commented decoy).
    g.set_default_journal_template(Some("Daily")).unwrap();
    assert_eq!(jtmpl().as_deref(), Some("Daily"));
    assert!(
        cfg().contains(":start-of-week 1"),
        "sibling key preserved: {}",
        cfg()
    );
    assert!(
        cfg().contains(";; :default-templates {:journals \"Commented\"}"),
        "comment preserved: {}",
        cfg()
    );

    // Replace (no duplicate key).
    Graph::open(&root)
        .set_default_journal_template(Some("Weekly"))
        .unwrap();
    assert_eq!(jtmpl().as_deref(), Some("Weekly"));
    assert_eq!(
        cfg().matches(":journals").count(),
        2,
        "one real + one commented :journals: {}",
        cfg()
    );

    // A multi-word name round-trips.
    Graph::open(&root)
        .set_default_journal_template(Some("My Daily Log"))
        .unwrap();
    assert_eq!(jtmpl().as_deref(), Some("My Daily Log"));

    // Clear → back to factory default (blank journals).
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    assert_eq!(jtmpl(), None);
    assert!(
        cfg().contains(":start-of-week 1"),
        "sibling still preserved after clear: {}",
        cfg()
    );

    // Preserve a SIBLING key inside :default-templates when clearing :journals.
    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals \"D\" :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    assert_eq!(jtmpl(), None);
    assert!(
        cfg().contains(":pages \"P\""),
        "sibling inner key preserved: {}",
        cfg()
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_ignores_commented_journals_inside_map() {
    // `:journals` appears only inside a comment within the :default-templates map;
    // the writer must NOT edit the comment — it must insert a real key and leave
    // the comment + sibling untouched.
    let root = std::env::temp_dir().join(format!("tine-jtmpl-c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    std::fs::write(
        &cfg_path,
        "{:default-templates { ;; :journals \"Commented\"\n  :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("Real"))
        .unwrap();
    let c = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        c.contains(":journals \"Real\""),
        "real journals inserted: {}",
        c
    );
    assert!(
        c.contains(";; :journals \"Commented\""),
        "comment untouched: {}",
        c
    );
    assert!(c.contains(":pages \"P\""), "sibling preserved: {}", c);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_quoted_name_does_not_corrupt_config() {
    // A template name with an embedded quote is escaped on write; the escape-aware
    // value-end scan must then replace/clear only the value on the next edit,
    // never mis-scanning the `\"` and corrupting config.edn.
    let root = std::env::temp_dir().join(format!("tine-jtmpl-q-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    std::fs::write(
        &cfg_path,
        "{:start-of-week 1\n :default-templates {:journals \"Old\"}}\n",
    )
    .unwrap();
    let cfg = || std::fs::read_to_string(&cfg_path).unwrap();

    Graph::open(&root)
        .set_default_journal_template(Some("My \"Daily\""))
        .unwrap();
    assert!(
        cfg().contains("\\\""),
        "embedded quote should be escaped: {}",
        cfg()
    );

    // Replace with a plain name: clean single :journals, sibling intact, old value gone.
    Graph::open(&root)
        .set_default_journal_template(Some("Plain"))
        .unwrap();
    let c = cfg();
    assert!(
        c.contains(":journals \"Plain\""),
        "value replaced cleanly: {}",
        c
    );
    assert_eq!(
        c.matches(":journals").count(),
        1,
        "exactly one :journals: {}",
        c
    );
    assert!(c.contains(":start-of-week 1"), "sibling preserved: {}", c);
    assert!(
        !c.contains("Daily"),
        "old quoted value fully removed: {}",
        c
    );

    // Clearing a value that contains an escaped quote removes the whole pair.
    std::fs::write(&cfg_path, "{:default-templates {:journals \"a\\\"b\"}}\n").unwrap();
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    assert!(
        !cfg().contains(":journals"),
        "journals cleared, no garbage: {}",
        cfg()
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_edn_aware_value_location() {
    // Round-4 audit: the value after `:journals` must be located/replaced as the
    // IMMEDIATE token, never "the next quote anywhere in the map" (which could land
    // on a later key's string value), and the outer `:default-templates` must not be
    // matched inside a string literal.
    let root = std::env::temp_dir().join(format!("tine-jtmpl-edn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    let cfg = || std::fs::read_to_string(&cfg_path).unwrap();

    // (a) `:journals` has a NON-string value (nil) and a later sibling has a string.
    // Setting must replace `nil`, not the `:pages` value.
    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals nil :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("X"))
        .unwrap();
    let c = cfg();
    assert!(
        c.contains(":journals \"X\""),
        "nil value replaced with X: {}",
        c
    );
    assert!(
        c.contains(":pages \"P\""),
        "later sibling string untouched: {}",
        c
    );
    assert!(!c.contains("nil"), "old nil value gone: {}", c);
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("X")
    );

    // Clearing the same shape removes only `:journals nil`, keeps `:pages "P"`.
    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals nil :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    let c = cfg();
    assert!(!c.contains(":journals"), "journals pair removed: {}", c);
    assert!(
        c.contains(":pages \"P\""),
        "sibling string preserved on clear: {}",
        c
    );

    // (b) `:default-templates {…}` appears only INSIDE a string literal — it is not
    // the real key, so the writer must not edit it; it inserts a real key instead.
    std::fs::write(
        &cfg_path,
        "{:note \":default-templates {:journals \\\"fake\\\"}\"}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("Real"))
        .unwrap();
    let c = cfg();
    assert!(
        c.contains(":journals \"Real\""),
        "real journals inserted: {}",
        c
    );
    assert!(
        c.contains("fake"),
        "string-literal decoy preserved verbatim: {}",
        c
    );
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("Real")
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn config_writers_skip_comment_between_key_and_value() {
    // A `;` comment between a key and its value must not mislead a writer (the
    // readers already skip it). Verify the writers do too — no duplicate key, the
    // real value is replaced.
    let root = std::env::temp_dir().join(format!("tine-wr-cmt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");

    std::fs::write(&cfg_path, "{:favorites ; note\n [\"Old\"]}\n").unwrap();
    Graph::open(&root).set_favorites(&["New".into()]).unwrap();
    let c = std::fs::read_to_string(&cfg_path).unwrap();
    assert_eq!(
        c.matches(":favorites").count(),
        1,
        "replaced, not duplicated: {c}"
    );
    assert!(!c.contains("\"Old\""), "old value gone: {c}");
    assert_eq!(Graph::open(&root).meta().favorites, vec!["New".to_string()]);

    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals ; note\n \"Old\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("New"))
        .unwrap();
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("New")
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn deleted_journal_is_not_served_from_stale_cache() {
    use tine_core::PageKind;
    let root = std::env::temp_dir().join(format!("tine-del-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let jpath = root.join("journals").join("2026_06_18.md");
    std::fs::write(&jpath, "- hello\n").unwrap();

    let g = Graph::open(&root);
    // Warm the whole-graph cache so the journal is held in memory.
    let entries = g.journals_desc();
    assert_eq!(entries.len(), 1);
    let entry = entries[0].clone();
    assert!(g.load_page(&entry).is_ok());

    // External delete (OG Logseq / Syncthing) before the watcher reconciles.
    std::fs::remove_file(&jpath).unwrap();

    // load_page must report NotFound, NOT serve the cached copy — serving it with
    // a null rev would let a subsequent save recreate the deleted file.
    let err = g.load_page(&entry).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    // load_named treats the vanished page as absent (Ok(None)), not an error.
    assert!(g
        .load_named(&entry.name, PageKind::Journal)
        .unwrap()
        .is_none());
    // The stale entry was evicted, so the feed no longer lists it.
    assert!(
        g.journals_desc().is_empty(),
        "deleted journal must drop out of the feed"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn query_open_tasks() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph");
    let store = store_at(&root);
    let groups = query_simple(&store, "(task TODO DOING)");
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(
        raws.iter().any(|r| r.starts_with("TODO Ship the M0")),
        "got: {raws:?}"
    );
    assert!(
        raws.iter().any(|r| r.starts_with("DOING Wire up")),
        "got: {raws:?}"
    );
    // A DONE task must not match.
    assert!(
        !raws.iter().any(|r| r.contains("DONE Validate")),
        "got: {raws:?}"
    );
}

#[test]
fn agenda_query_excludes_finished_tasks() {
    // The journal "Scheduled & Deadline" agenda (ui.ts::agendaQuery) must hide
    // DONE/CANCELED/CANCELLED items — matching OG's get-date-scheduled-or-deadlines
    // — while keeping open tasks AND marker-less scheduled blocks. A ±100y window
    // keeps the test robust against the real-clock `today` the engine resolves.
    let root = std::env::temp_dir().join(format!("tine-agenda-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_26.md"),
        "- TODO open task\n  SCHEDULED: <2026-06-27 Sat>\n\
         - DONE finished task\n  SCHEDULED: <2026-06-27 Sat>\n\
         - CANCELED dropped task\n  DEADLINE: <2026-06-27 Sat>\n\
         - CANCELLED british drop\n  DEADLINE: <2026-06-27 Sat>\n\
         - plain meeting\n  SCHEDULED: <2026-06-27 Sat>\n",
    )
    .unwrap();
    let store = store_at(&root);
    let q = "(and (or (between scheduled -36500d +36500d) (between deadline -36500d +36500d)) \
             (not (task DONE CANCELED CANCELLED)))";
    let raws: Vec<String> = query_simple(&store, q)
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(
        raws.iter().any(|r| r.starts_with("TODO open task")),
        "open task missing: {raws:?}"
    );
    assert!(
        raws.iter().any(|r| r.starts_with("plain meeting")),
        "marker-less missing: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("DONE finished")),
        "DONE leaked: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("CANCELED dropped")),
        "CANCELED leaked: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("CANCELLED british")),
        "CANCELLED leaked: {raws:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_superstring_rewrites_journal_and_nonjournal_refs() {
    // Regression for the reported case: the new name CONTAINS the old name, and
    // the old name is referenced from BOTH a non-journal page and a journal, with
    // the cache already warm + a backlinks query run first (as in live use).
    let root = std::env::temp_dir().join(format!("tine-rename-ss-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Testtest.md"), "- the page\n").unwrap();
    std::fs::write(
        root.join("pages").join("MyPage.md"),
        "- see [[Testtest]] here\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- ref [[Testtest]]\n",
    )
    .unwrap();

    let store = Store::open(&root, Default::default()).unwrap().0;
    let _ = store.whole_graph().unwrap().backlinks("Testtest").unwrap();

    pages::rename_page_expected(&store, "Testtest", "TesttestTest", None).unwrap();

    let my = std::fs::read_to_string(root.join("pages").join("MyPage.md")).unwrap();
    let jr = std::fs::read_to_string(root.join("journals").join("2026_06_15.md")).unwrap();
    assert!(
        my.contains("[[TesttestTest]]") && !my.contains("[[Testtest]]"),
        "non-journal: {my}"
    );
    assert!(jr.contains("[[TesttestTest]]"), "journal: {jr}");
    let bl = store
        .whole_graph()
        .unwrap()
        .backlinks("TesttestTest")
        .unwrap();
    let after: Vec<&str> = bl.iter().map(|x| x.page.as_str()).collect();
    assert!(
        after.contains(&"MyPage"),
        "backlinks miss non-journal: {after:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_rewrites_nested_ref_in_open_page() {
    // Mirror the reported "Tine" page: a NESTED ref (sub-bullet) with a block
    // id::, the page already LOADED (open/pinned), cache warm + backlinks queried.
    let root = std::env::temp_dir().join(format!("tine-rename-nested-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Testtest.md"), "- the page\n").unwrap();
    std::fs::write(
        root.join("pages").join("Tine.md"),
        "- Tine notes\n\t- see [[Testtest]] in a sub-bullet\n\t  id:: 1111aaaa-bbbb-cccc-dddd-eeeeeeeeeeee\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- ref [[Testtest]]\n",
    )
    .unwrap();

    let store = Store::open(&root, Default::default()).unwrap().0;
    let _ = store.page(&PageId::from("pages/Tine.md")).unwrap();
    let _ = store.whole_graph().unwrap().backlinks("Testtest").unwrap();

    pages::rename_page_expected(&store, "Testtest", "TesttestTest", None).unwrap();

    let tine = std::fs::read_to_string(root.join("pages").join("Tine.md")).unwrap();
    assert!(
        tine.contains("[[TesttestTest]]"),
        "nested ref NOT rewritten: {tine:?}"
    );
    assert!(!tine.contains("[[Testtest]]"), "old ref remains: {tine:?}");
    let bl = store
        .whole_graph()
        .unwrap()
        .backlinks("TesttestTest")
        .unwrap();
    let pages: Vec<&str> = bl.iter().map(|x| x.page.as_str()).collect();
    assert!(pages.contains(&"Tine"), "backlinks miss Tine: {pages:?}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn trash_sync_conflict_refuses_real_pages() {
    let root = std::env::temp_dir().join(format!("tine-trashconflict-{}", std::process::id()));
    let pages = root.join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(pages.join("Real.md"), "- keep me\n").unwrap();
    let conflict = "Real.sync-conflict-20260705-120000-ABCDEFG.md";
    std::fs::write(pages.join(conflict), "- other device\n").unwrap();

    let store = Store::open(&root, Default::default()).unwrap().0;
    // Refuses a genuine page — never trashes real data.
    assert!(conflicts::trash_sync_conflict(&store, "pages/Real.md").is_err());
    assert!(pages.join("Real.md").exists(), "real page must survive");
    // Trashes an actual conflict copy.
    conflicts::trash_sync_conflict(&store, &format!("pages/{conflict}")).unwrap();
    assert!(
        !pages.join(conflict).exists(),
        "conflict copy should be gone"
    );
    assert!(conflicts::list_sync_conflicts(&store).is_empty());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sync_conflict_base_recognises_syncthing_and_dropbox() {
    use tine_core::model::sync_conflict_base;
    // Syncthing.
    assert_eq!(
        sync_conflict_base("Foo.sync-conflict-20260705-120000-ABCDEFG"),
        Some("Foo")
    );
    // Dropbox variants.
    assert_eq!(
        sync_conflict_base("Foo (conflicted copy 2026-07-05)"),
        Some("Foo")
    );
    assert_eq!(
        sync_conflict_base("Foo (martin's conflicted copy 2026-07-05)"),
        Some("Foo")
    );
    // Not a conflict copy.
    assert_eq!(sync_conflict_base("Foo"), None);
    assert_eq!(sync_conflict_base("2026_06_26"), None);
    assert_eq!(sync_conflict_base("My (draft) page"), None);
}

#[test]
fn sync_conflict_copies_excluded_from_pages_and_surfaced_separately() {
    let root = std::env::temp_dir().join(format!("tine-syncconflict-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // A real page + its Syncthing conflict copy (shares the id::, so exercises the
    // "must not churn the id space" reason to keep it out of the cache).
    std::fs::write(
        root.join("pages").join("Foo.md"),
        "- hello\n  id:: aaaaaaaa-0000-0000-0000-0000000000ff\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages")
            .join("Foo.sync-conflict-20260705-120000-ABCDEFG.md"),
        "- hello from the other device\n  id:: aaaaaaaa-0000-0000-0000-0000000000ff\n",
    )
    .unwrap();
    // A journal + its conflict copy.
    std::fs::write(root.join("journals").join("2026_06_26.md"), "- day one\n").unwrap();
    std::fs::write(
        root.join("journals")
            .join("2026_06_26.sync-conflict-20260705-130000-ABCDEFG.md"),
        "- day one, edited elsewhere\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    let store = Store::open(&root, Default::default()).unwrap().0;

    // The conflict copies must NOT appear as pages/journals.
    let names: Vec<String> = g.list_pages().into_iter().map(|p| p.name).collect();
    assert!(
        names.iter().any(|n| n == "Foo"),
        "real page missing: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains("sync-conflict")),
        "conflict copy leaked into page list: {names:?}"
    );

    // They ARE surfaced by list_sync_conflicts, each pointing at its winner.
    let mut conflicts = conflicts::list_sync_conflicts(&store);
    conflicts.sort_by(|a, b| a.base_name.cmp(&b.base_name));
    assert_eq!(conflicts.len(), 2, "conflicts: {conflicts:?}");
    let foo = conflicts
        .iter()
        .find(|c| c.base_name == "Foo")
        .expect("Foo conflict");
    assert_eq!(foo.base_path.as_deref(), Some("pages/Foo.md"));
    assert!(foo.tag.starts_with("sync-conflict-"), "tag: {}", foo.tag);
    assert!(
        foo.preview.contains("other device"),
        "preview: {}",
        foo.preview
    );
    let jrnl = conflicts
        .iter()
        .find(|c| c.base_name != "Foo")
        .expect("journal conflict");
    assert_eq!(jrnl.base_path.as_deref(), Some("journals/2026_06_26.md"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn resolve_sync_conflict_merges_and_trashes() {
    use std::collections::HashMap;
    let root = std::env::temp_dir().join(format!("tine-resolveconflict-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    let pages = root.join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let winner = "- alpha\n- beta line here\n  id:: aaaaaaaa-0000-0000-0000-0000000000b0\n";
    std::fs::write(pages.join("Foo.md"), winner).unwrap();
    let conflict = "- alpha\n- beta line there\n  id:: aaaaaaaa-0000-0000-0000-0000000000b0\n- extra from other device\n";
    let conflict_name = "Foo.sync-conflict-20260705-120000-ABCDEFG.md";
    std::fs::write(pages.join(conflict_name), conflict).unwrap();

    let store = Store::open(&root, Default::default()).unwrap().0;
    let win_rel = "pages/Foo.md";
    let conf_rel = format!("pages/{conflict_name}");

    // Diff to discover the row ids.
    let diff = conflicts::sync_conflict_diff(&store, win_rel, &conf_rel)
        .unwrap()
        .expect("a diff");
    let modified = diff
        .rows
        .iter()
        .find(|r| format!("{:?}", r.kind) == "Modified")
        .expect("modified row");
    let removed = diff
        .rows
        .iter()
        .find(|r| format!("{:?}", r.kind) == "Removed")
        .expect("removed row");

    assert_eq!(diff.base_rev, crate::model::content_rev(winner));
    // Guard: decisions from the diff must not apply after the winner changes.
    let changed_winner = winner.replace("beta line here", "beta line NEW!");
    crate::test_fixture_io::atomic_write(pages.join("Foo.md"), &changed_winner).unwrap();
    let err = conflicts::resolve_sync_conflict(
        &store,
        win_rel,
        &conf_rel,
        &HashMap::new(),
        &diff.base_rev,
        &diff.conflict_rev,
        "union",
    )
    .unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::AlreadyExists,
        "stale base_rev should conflict"
    );
    assert_eq!(
        std::fs::read_to_string(pages.join("Foo.md")).unwrap(),
        changed_winner,
        "winner untouched on guard"
    );
    crate::test_fixture_io::atomic_write(pages.join("Foo.md"), winner).unwrap();

    // The conflict side is revision-bound too. Otherwise decisions aligned to
    // an old copy could silently merge after a sync tool rewrites that copy.
    let changed_conflict = conflict.replace("beta line there", "beta line LATER");
    crate::test_fixture_io::atomic_write(pages.join(conflict_name), &changed_conflict).unwrap();
    let err = conflicts::resolve_sync_conflict(
        &store,
        win_rel,
        &conf_rel,
        &HashMap::new(),
        &diff.base_rev,
        &diff.conflict_rev,
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read_to_string(pages.join(conflict_name)).unwrap(),
        changed_conflict
    );
    assert_eq!(
        std::fs::read_to_string(pages.join("Foo.md")).unwrap(),
        winner
    );
    crate::test_fixture_io::atomic_write(pages.join(conflict_name), conflict).unwrap();

    // Resolve: take theirs for the modified block, pull in the removed one.
    let decisions = HashMap::from([
        (modified.id.clone(), "theirs".to_string()),
        (removed.id.clone(), "theirs".to_string()),
    ]);
    let base = crate::model::content_rev(winner);
    let conflict_rev = crate::model::content_rev(conflict);
    conflicts::resolve_sync_conflict(
        &store,
        win_rel,
        &conf_rel,
        &decisions,
        &base,
        &conflict_rev,
        "union",
    )
    .unwrap();

    let merged = std::fs::read_to_string(pages.join("Foo.md")).unwrap();
    assert!(merged.contains("beta line there"), "merged: {merged:?}");
    assert!(
        !merged.contains("beta line here"),
        "old winner text remains: {merged:?}"
    );
    assert!(
        merged.contains("extra from other device"),
        "removed block not pulled in: {merged:?}"
    );

    // Conflict copy is gone from pages/ and from the conflicts list, and lives in trash.
    assert!(
        !pages.join(conflict_name).exists(),
        "conflict copy not moved"
    );
    assert!(
        conflicts::list_sync_conflicts(&store).is_empty(),
        "conflict still listed"
    );
    let trash = root.join("logseq").join(".tine-trash").join("conflicts");
    let trashed: Vec<_> = std::fs::read_dir(&trash).unwrap().flatten().collect();
    assert!(
        trashed
            .iter()
            .any(|e| e.file_name().to_string_lossy().contains("sync-conflict")),
        "conflict copy not in trash: {trashed:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(unix)]
#[test]
fn page_symlinks_are_not_indexed_or_reconciled() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("tine-page-symlink-{}", std::process::id()));
    let outside = std::env::temp_dir().join(format!("tine-outside-page-{}.md", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&outside);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(&outside, "- outside secret\n").unwrap();
    let link = root.join("pages/Secret.md");
    symlink(&outside, &link).unwrap();

    let (store, _, _) = Store::open(&root, Default::default()).unwrap();
    let before = store.whole_graph().unwrap();
    assert!(before
        .inventory()
        .0
        .iter()
        .all(|entry| entry.name != "Secret"));
    let id = store.file_id(tine_store::Area::Pages, "Secret.md").unwrap();
    assert!(store.path_for_os_handoff(&id, false).is_err());
    store.scan_refresh().unwrap();
    assert_eq!(store.whole_graph().unwrap().rev(), before.rev());
    assert!(link.exists());

    store.close();
    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_file(&outside).ok();
}
