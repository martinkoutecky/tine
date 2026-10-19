//! BqbeQ (D-20261019-B6): the app's create path while the graph loads. The
//! window opens today's absent journal on the page host and saves its first
//! block (the daily template); the host's open, save and publication finish
//! without the whole-graph build, and the first complete view holds the
//! page once with the saved text, from the launch pass's own install.
use std::fs;
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use tine_core::date::JournalDate;
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{EditKind, PageHost, PageId, PageOperation, Store};

fn answer(mail: &mpsc::Receiver<serde_json::Value>, id: u64) -> serde_json::Value {
    loop {
        let value = mail.recv_timeout(Duration::from_secs(10)).unwrap();
        if value["answer"]["id"] == id {
            return value;
        }
    }
}

fn builds(store: &Store) -> u64 {
    store.diagnostics()["onDemandBuilds"]["count"]
        .as_u64()
        .unwrap()
}

fn graph(root: &Path) {
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for index in 0..20 {
        let text = format!("- page {index} [[P0]]\n");
        fs::write(root.join(format!("pages/P{index}.md")), text).unwrap();
    }
}

#[test]
fn todays_journal_created_on_the_host_while_loading_waits_for_no_graph_build() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    graph(&root);
    let hold = root.join(".tine-test-pause-load");
    fs::write(&hold, "").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let (sink, mail) = mpsc::channel();
    let send = move |m| {
        let _ = sink.send(serde_json::to_value(m).unwrap());
    };
    let host = PageHost::start(&store, &dir.path().join("app"), "load", |_, _| {}, send).unwrap();
    let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let id = reloaded["nextId"].as_u64().unwrap();
    let today = JournalDate::today();
    let stem = today.file_stem();
    let rel = format!("journals/{stem}.md");
    let page = PageId::from(rel.as_str());
    let opened = host.open(session, id, &page, &today.title()).unwrap();
    let key = serde_json::to_value(opened).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let version = answer(&mail, id)["answer"]["version"].as_u64().unwrap();
    let doc = PageDto {
        name: stem.clone(),
        kind: PageKind::Journal,
        title: today.title(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b".into(),
            raw: "from the daily template".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    };
    let kinds = [EditKind::ReplacePage];
    host.submit(session, id + 1, &key, &doc, version, None, &kinds)
        .unwrap();
    let version = answer(&mail, id + 1)["answer"]["version"].as_u64().unwrap();
    let published = host.wait_published(session, &[(key, version, None)], Duration::from_secs(10));
    assert_eq!(published, PageOperation::Applied, "published");
    assert!(fs::read_to_string(root.join(&rel))
        .unwrap()
        .contains("from the daily template"));
    assert!(matches!(store.is_graph_ready(), Ok(false)), "still loading");
    assert_eq!(
        builds(&store),
        0,
        "BqbeQ: the host's open and save of a new page built the whole graph cache while loading \
         (publish_retired, 8.8 s on the realistic 10k graph)"
    );
    fs::remove_file(&hold).unwrap();
    let view = store.whole_graph().unwrap();
    let corpus = view.corpus();
    let found: Vec<_> = (corpus.pages.iter()).filter(|p| p.id == rel).collect();
    assert_eq!(found.len(), 1, "today's journal is in the view once");
    assert!(found[0].document.roots[0]
        .raw()
        .contains("from the daily template"));
    let outcomes = &store.diagnostics()["launch"]["loadPassOutcomes"];
    assert_eq!(
        outcomes,
        &serde_json::json!([]),
        "the launch pass installed"
    );
    assert_eq!(
        builds(&store),
        0,
        "no second whole-graph build before Ready"
    );
    drop(host);
    store.close();
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".git" || name == "bak" || name == ".recycle" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Every page of a view: id, name and root text, sorted.
fn fingerprint(store: &Store) -> Vec<(String, String, String)> {
    let view = store.whole_graph().unwrap();
    let mut pages: Vec<_> = (view.corpus().pages.iter())
        .map(|page| {
            let text: Vec<String> = (page.document.roots.iter())
                .map(|root| root.raw().to_string())
                .collect();
            (page.id.to_string(), page.name.clone(), text.join("\n"))
        })
        .collect();
    pages.sort();
    pages
}

/// AGENTS.md §4 corpus gate, on a COPY of the anonymized graph: saves made
/// while the graph loads (a new today's journal and an edit of an existing
/// page) leave the first complete view equal to a fresh open of the result.
/// `cargo test -p tine-store --test integration own_saves_while_loading_match_a_fresh_open_on_the_anonymized_graph -- --ignored`
#[test]
#[ignore = "needs ~/research/logseq-anonymized"]
fn own_saves_while_loading_match_a_fresh_open_on_the_anonymized_graph() {
    let source = Path::new(&std::env::var("HOME").unwrap()).join("research/logseq-anonymized");
    assert!(source.is_dir(), "the anonymized graph is missing");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    copy_tree(&source, &root);
    let today = JournalDate::today();
    let rel = format!("journals/{}.md", today.file_stem());
    let _ = fs::remove_file(root.join(&rel));
    let mut existing: Vec<_> = fs::read_dir(root.join("pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    existing.sort();
    let edited = format!(
        "pages/{}",
        existing[existing.len() / 2]
            .file_name()
            .unwrap()
            .to_string_lossy()
    );
    let hold = root.join(".tine-test-pause-load");
    fs::write(&hold, "").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let read = store.page(&PageId::from(edited.as_str())).unwrap();
    let mut doc = read.doc.clone();
    doc.blocks.push(BlockDto {
        raw: "BQBEQ-CORPUS-EDIT".into(),
        ..Default::default()
    });
    let journal = PageDto {
        name: today.file_stem(),
        kind: PageKind::Journal,
        title: today.title(),
        pre_block: None,
        blocks: vec![BlockDto {
            raw: "BQBEQ-CORPUS-TODAY".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    };
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &PageId::from(rel.as_str()),
        tine_store::SaveBase::CreateNew,
        &journal,
    );
    tx.save_page(
        &[EditKind::ReplacePage],
        &PageId::from(edited.as_str()),
        tine_store::SaveBase::Existing(read.rev.clone()),
        &doc,
    );
    assert!(matches!(
        tx.commit(),
        tine_store::TxOutcome::Committed { change: None, .. }
    ));
    assert_eq!(builds(&store), 0, "the saves built the graph while loading");
    fs::remove_file(&hold).unwrap();
    let during = fingerprint(&store);
    assert_eq!(builds(&store), 0);
    assert_eq!(
        store.diagnostics()["launch"]["loadPassOutcomes"],
        serde_json::json!([])
    );
    store.close();
    drop(store);
    let fresh = Store::open(&root, Default::default()).unwrap().0;
    let after = fingerprint(&fresh);
    assert_eq!(during.len(), after.len(), "page count");
    for (a, b) in during.iter().zip(&after) {
        assert_eq!(a, b);
    }
    assert_eq!(during.iter().filter(|p| p.0 == rel).count(), 1);
    assert!(during.iter().any(|p| p.2.contains("BQBEQ-CORPUS-EDIT")));
    eprintln!(
        "anonymized copy: {} pages, first view == fresh open",
        during.len()
    );
    fresh.close();
}
