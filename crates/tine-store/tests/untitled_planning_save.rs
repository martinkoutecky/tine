//! GH #1012: a date picked on a block without title text is written after an
//! empty first line. Each shape goes through the real path: the parser-owned
//! edit door, the page host's guarded save, the bytes on disk, a fresh
//! `Store::open`, and a second save without an edit.
use std::fs;

use tine_core::block_regions::{self, edit, Edit};
use tine_store::{EditKind, PageId, SaveBase, Store};

const ID: &str = "6679a1b2-0000-4000-8000-000000000001";

struct Graph {
    dir: tempfile::TempDir,
    path: &'static str,
}

impl Graph {
    fn new(path: &'static str, source: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::create_dir(dir.path().join("journals")).unwrap();
        fs::write(dir.path().join(path), source).unwrap();
        Graph { dir, path }
    }
    fn bytes(&self) -> String {
        fs::read_to_string(self.dir.path().join(self.path)).unwrap()
    }
    /// Read the page, let `change` rewrite its first block, save it through the
    /// page host and return the first block's raw as it was saved.
    fn save(&self, change: impl FnOnce(&str) -> String) -> String {
        let store =
            std::sync::Arc::new(Store::open(self.dir.path(), Default::default()).unwrap().0);
        let writer = crate::test_host_writer::HostWriter::start(&store);
        let id = PageId::from(self.path);
        let read = store.page(&id).expect("the page opens");
        let mut doc = read.doc;
        doc.blocks[0].raw = change(&doc.blocks[0].raw);
        let raw = doc.blocks[0].raw.clone();
        let outcome = writer.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        );
        assert!(outcome.is_ok(), "save failed: {outcome:?}");
        drop(writer);
        store.close();
        raw
    }
}

#[test]
fn a_date_picked_on_a_block_without_title_text_saves_and_reloads_unchanged() {
    let _guard = crate::integration_lock();
    let s = "SCHEDULED: <2026-10-03 Sat>";
    let lb = ":LOGBOOK:\nCLOCK: [2026-10-01 Wed 10:00]--[2026-10-01 Wed 10:05] =>  00:05:00\n:END:";
    let md_lb = lb.replace('\n', "\n  ");
    // (path, source, saved file after the pick)
    let cases: Vec<(&'static str, String, String)> = vec![
        (
            "pages/empty.md",
            "-\n- other\n".into(),
            format!("-\n  {s}\n- other\n"),
        ),
        (
            "pages/logbook.md",
            format!("- {md_lb}\n- other\n"),
            format!("-\n  {s}\n  {md_lb}\n- other\n"),
        ),
        (
            "pages/id.md",
            format!("- id:: {ID}\n- other\n"),
            format!("-\n  {s}\n  id:: {ID}\n- other\n"),
        ),
        (
            "pages/crlf.md",
            format!("- {md_lb}\n- other\n").replace('\n', "\r\n"),
            format!("-\n  {s}\n  {md_lb}\n- other\n").replace('\n', "\r\n"),
        ),
        (
            "pages/empty.org",
            "*\n* other\n".into(),
            format!("*\n{s}\n* other\n"),
        ),
        (
            "pages/props.org",
            format!("*\n:PROPERTIES:\n:id: {ID}\n:END:\n* other\n"),
            format!("*\n{s}\n:PROPERTIES:\n:id: {ID}\n:END:\n* other\n"),
        ),
    ];
    for (path, source, after) in cases {
        let org = path.ends_with(".org");
        let graph = Graph::new(path, &source);
        let edited = graph.save(|raw| {
            edit(
                raw,
                org,
                Edit::Planning {
                    which: "Scheduled".into(),
                    value: Some("<2026-10-03 Sat>".into()),
                },
            )
            .unwrap()
        });
        // The region edit: the date after an empty first line, LF in the model.
        assert!(edited.starts_with(&format!("\n{s}")), "{path}: {edited:?}");
        // The file: the shape Logseq reads, in the file's own line endings.
        assert_eq!(graph.bytes(), after, "{path}");
        // A native reload reads the same raw, with its id.
        let mut reloaded = String::new();
        let resaved = graph.save(|raw| {
            reloaded = raw.to_string();
            raw.to_string()
        });
        assert_eq!(reloaded, edited, "{path}: reload");
        assert_eq!(resaved, edited);
        if source.contains(ID) {
            let regions = block_regions::parse(&reloaded, org);
            assert_eq!(regions.id.map(|p| p.value), Some(ID.to_string()), "{path}");
        }
        // A save without an edit writes the same bytes.
        assert_eq!(graph.bytes(), after, "{path}: resave");
    }
}
