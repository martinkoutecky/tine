//! Browser core slice 5a, round 2: a memory graph's edits keep block
//! identity, refuse what the desktop refuses, diagnose the config as a fresh
//! open does, hold what a fresh open holds, and are opt-in.
use super::memory_replace_tests::{apply, assert_opens_alike, graph_files, open, Files};
use crate::model::file_source::{Content, FileSource, Files as Tree, MEMORY_ROOT};
use crate::{PageId, PageRefusal, WholeGraph};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::Arc;
use tine_core::model::{BlockDto, PageDto};

/// Each block's runtime id by its label (the first word of its text).
fn ids(blocks: &[BlockDto], out: &mut BTreeMap<String, String>) {
    for block in blocks {
        let label = block.raw.split_whitespace().next().unwrap_or("").to_owned();
        assert!(
            out.insert(label, block.id.clone()).is_none(),
            "labels are unique"
        );
        ids(&block.children, out);
    }
}

fn label(block: &BlockDto) -> &str {
    block.raw.split_whitespace().next().unwrap()
}

/// Review finding 1: a submit publishes the serializer's document, so each
/// block keeps its runtime id through a reorder, an insert, a delete and a
/// reparent, an authored id's block among them, in Markdown and Org, with LF
/// and CRLF line ends.
#[test]
fn a_submit_keeps_each_blocks_runtime_id() {
    let uuid = "6566c5a4-0000-4000-8000-000000000001";
    let markdown =
        format!("- alpha\n- beta\n  id:: {uuid}\n- gamma\n  - delta\n  - epsilon\n- zeta\n");
    let org = format!(
        "* alpha\n* beta\n:PROPERTIES:\n:id: {uuid}\n:END:\n* gamma\n** delta\n** epsilon\n* zeta\n"
    );
    type Edit = fn(&mut PageDto);
    let edits: [(&str, Edit); 4] = [
        ("reorder", |dto| dto.blocks.swap(0, 1)),
        ("insert", |dto| {
            let mut new = dto.blocks[0].clone();
            (new.id, new.raw, new.children) = (String::new(), "omega".into(), Vec::new());
            new.has_id = None;
            dto.blocks.insert(1, new);
        }),
        ("delete", |dto| {
            let at = dto.blocks.iter().position(|b| label(b) == "zeta").unwrap();
            dto.blocks.remove(at);
        }),
        ("reparent", |dto| {
            let at = dto.blocks.iter().position(|b| label(b) == "gamma").unwrap();
            let child = dto.blocks[at].children.remove(0);
            dto.blocks.insert(0, child);
        }),
    ];
    for (rel, text) in [("pages/Note.md", markdown), ("pages/Note.org", org)] {
        for crlf in [false, true] {
            let text = if crlf {
                text.replace('\n', "\r\n")
            } else {
                text.clone()
            };
            let mut files = Files::new();
            files.insert("logseq/config.edn".into(), b"{}\n".to_vec());
            files.insert(rel.into(), text.into_bytes());
            let mut view = open(&files);
            let id = PageId::from(rel);
            for (edit, apply_edit) in edits {
                let mut dto = view.page(&id).unwrap();
                let mut before = BTreeMap::new();
                ids(&dto.blocks, &mut before);
                apply_edit(&mut dto);
                view.submit_page(&id, &dto, true).unwrap();
                let mut after = BTreeMap::new();
                ids(&view.page(&id).unwrap().blocks, &mut after);
                for (label, runtime) in &before {
                    if let Some(now) = after.get(label) {
                        assert_eq!(now, runtime, "{rel} crlf={crlf} {edit}: {label}");
                    }
                }
                let raw = |label: &str| {
                    let dto = view.page(&id).unwrap();
                    let mut stack = dto.blocks;
                    while let Some(block) = stack.pop() {
                        if self::label(&block) == label {
                            return block.raw;
                        }
                        stack.extend(block.children);
                    }
                    panic!("{label} is gone");
                };
                assert!(
                    raw("beta").contains(uuid),
                    "{rel} {edit}: the authored id stays"
                );
            }
        }
    }
}

/// The config cases a fresh open diagnoses as unreadable: invalid UTF-8, a
/// directory, a `logseq` file, an iCloud placeholder file or directory.
fn unreadable_configs() -> Vec<(&'static str, Vec<(&'static str, Vec<u8>)>)> {
    vec![
        (
            "invalid UTF-8",
            vec![("logseq/config.edn", vec![0xff, 0xfe])],
        ),
        (
            "a directory",
            vec![("logseq/config.edn/inner.edn", b"{}".to_vec())],
        ),
        ("a logseq file", vec![("logseq", b"{}".to_vec())]),
        (
            "a placeholder file",
            vec![("logseq/.config.edn.icloud", Vec::new())],
        ),
        (
            "a placeholder directory",
            vec![("logseq/.config.edn.icloud/x", Vec::new())],
        ),
    ]
}

/// Review finding 2: under an unreadable config a submit is refused with the
/// desktop transaction's refusal and writes nothing; the raw replace still
/// writes, so the config can be repaired, and a submit then goes through.
#[test]
fn a_submit_under_an_unreadable_config_is_refused() {
    for (case, config) in unreadable_configs() {
        let mut files = Files::new();
        files.insert("pages/Note.md".into(), b"- first\n".to_vec());
        for (rel, bytes) in &config {
            files.insert(rel.to_string(), bytes.clone());
        }
        let mut view = open(&files);
        assert!(view.config().problem.is_some(), "{case}");
        let id = PageId::from("pages/Note.md");
        let mut dto = view.page(&id).unwrap();
        dto.blocks[0].raw = "edited".into();
        let rev = view.rev();
        let refused = view.submit_page(&id, &dto, true).unwrap_err();
        assert!(
            matches!(&refused, PageRefusal::InvalidTarget { message } if message.contains("ConfigUnreadable")),
            "{case}: {refused:?}"
        );
        assert_eq!(view.rev(), rev, "{case}");
        assert_eq!(view.memory_page_bytes(&id), Some(b"- first\n".to_vec()));
        // Repair through the raw path: what stands in the config's way goes
        // (a batch neither passes through nor replaces a key), a config comes.
        let clear: Vec<_> = (config.iter())
            .filter(|(rel, _)| *rel != "logseq/config.edn")
            .map(|(rel, _)| (rel.to_string(), None))
            .collect();
        if !clear.is_empty() {
            view.replace_files(clear).unwrap();
        }
        view.replace_files(vec![("logseq/config.edn".into(), Some(b"{}\n".to_vec()))])
            .unwrap();
        assert!(view.config().problem.is_none(), "{case}");
        view.submit_page(&id, &dto, true).unwrap();
        assert_eq!(view.memory_page_bytes(&id), Some(b"- edited\n".to_vec()));
    }
}

/// Review finding 3: a replace that adds or deletes what the config read
/// looks up or resolves through (a `logseq` file, the first or last child
/// under `logseq/config.edn`, an iCloud placeholder or its first or last
/// child) gives the config diagnosis a fresh open of the same files gives.
#[test]
fn a_replace_diagnoses_the_config_as_a_fresh_open() {
    let problem = |view: &WholeGraph| view.config().problem.as_ref().map(|e| e.kind);
    let steps: &[&[(&str, Option<&str>)]] = &[
        &[("logseq", Some("x"))],
        &[("logseq", None)],
        &[("logseq/config.edn/a", Some("x"))],
        &[("logseq/config.edn/b", Some("x"))],
        &[("logseq/config.edn/a", None)],
        &[("logseq/config.edn/b", None)],
        &[("logseq/.config.edn.icloud", Some(""))],
        &[("logseq/.config.edn.icloud", None)],
        &[("logseq/.config.edn.icloud/a", Some("x"))],
        &[("logseq/.config.edn.icloud/b", Some("x"))],
        &[("logseq/.config.edn.icloud/a", None)],
        &[("logseq/.config.edn.icloud/b", None)],
        &[("logseq/config.edn", Some("{}"))],
    ];
    let mut files = Files::new();
    files.insert("pages/Note.md".into(), b"- first\n".to_vec());
    let mut view = open(&files);
    for step in steps {
        let changes = apply(&mut files, step);
        view.replace_files(changes).unwrap();
        let fresh = open(&files);
        assert_eq!(problem(&view), problem(&fresh), "after {step:?}");
        assert_opens_alike(&view, &files);
    }
}

/// Review finding 5: a replace parses and validates a file as a page only
/// where the open's eligibility check (under the batch's resulting config)
/// says it is graph text. Opaque bytes elsewhere are held as an open holds
/// them; under a page path they are still refused.
#[test]
fn a_replace_holds_what_a_fresh_open_holds() {
    let bad = vec![0xff, 0xfe];
    let held = [
        "assets/bad.md",
        "publish/bad.md",
        "published-queries/bad.md",
        "logseq/bak/pages/bad.md",
        "node_modules/x/bad.md",
        ".hidden/bad.md",
        "pages/.bad.md",
        "secret/bad.md",
        "pages/P0.sync-conflict-20260705-120000-ABCDEFG.md",
    ];
    let mut files = graph_files(2);
    files.insert(
        "logseq/config.edn".into(),
        b"{:hidden [\"secret\"]}\n".to_vec(),
    );
    let mut view = open(&files);
    for rel in held {
        files.insert(rel.into(), bad.clone());
        view.replace_files(vec![(rel.into(), Some(bad.clone()))])
            .unwrap_or_else(|error| panic!("{rel}: {error}"));
        assert_opens_alike(&view, &files);
        assert_eq!(
            view.unreadable_files(),
            open(&files).unreadable_files(),
            "{rel}"
        );
    }
    let rev = view.rev();
    // Graph text: refused, the graph unchanged.
    let refused = view.replace_files(vec![("pages/bad.md".into(), Some(bad.clone()))]);
    assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::InvalidData);
    // The batch's own config decides: un-hiding `secret` makes it graph text.
    let unhide = vec![
        ("logseq/config.edn".into(), Some(b"{}\n".to_vec())),
        ("secret/more.md".into(), Some(bad.clone())),
    ];
    assert_eq!(
        view.replace_files(unhide).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    // ...and hiding it in the same batch holds the bytes.
    let hide = vec![
        (
            "logseq/config.edn".into(),
            Some(b"{:hidden [\"secret\" \"other\"]}\n".to_vec()),
        ),
        ("other/bad.md".into(), Some(bad.clone())),
    ];
    assert_eq!(view.rev(), rev);
    view.replace_files(hide.clone()).unwrap();
    for (rel, bytes) in hide {
        files.insert(rel, bytes.unwrap());
    }
    assert_opens_alike(&view, &files);
}

/// Review finding 8: a deletion moves the last sibling into the freed slot
/// (O(1), no walk over later siblings), and no answer reads the listing's
/// order: a graph listed in reverse answers as the one listed forward.
#[test]
fn a_deletion_takes_no_walk_and_no_answer_reads_listing_order() {
    let root = Path::new(MEMORY_ROOT);
    let file = |rel: &str| {
        (
            rel.to_owned(),
            Content::Bytes(Arc::new(b"- x\n".to_vec())),
            None,
        )
    };
    let mut tree = Tree::new(root, ["pages/a.md", "pages/b.md", "pages/c.md"].map(file)).unwrap();
    tree.replace(vec![("pages/a.md".into(), None)]).unwrap();
    let source = FileSource::Memory(Arc::new(tree));
    let listed: Vec<_> = (source.read_dir(&root.join("pages")).unwrap())
        .map(|item| item.ok().unwrap().path)
        .collect();
    assert_eq!(listed, [root.join("pages/c.md"), root.join("pages/b.md")]);
    let files = graph_files(30);
    let forward = (files.iter()).map(|(rel, bytes)| (rel.clone(), bytes.clone()));
    let reverse = (files.iter().rev()).map(|(rel, bytes)| (rel.clone(), bytes.clone()));
    let (forward, _) = WholeGraph::from_files(forward.collect(), Vec::new(), false).unwrap();
    let (reverse, _) = WholeGraph::from_files(reverse.collect(), Vec::new(), false).unwrap();
    let answers = |view| super::subgraph_tests::answers(view, MEMORY_ROOT, false);
    assert_eq!(answers(&forward), answers(&reverse));
}

/// Review finding 9: a read-only view (a published export, a quick share)
/// keeps none of the files' bytes once built, as on develop, and refuses
/// both edits; only an editable one holds them.
#[test]
fn a_read_only_view_keeps_no_file_bytes_and_takes_no_edit() {
    let files = graph_files(5);
    let buffers: Vec<_> = (files.iter())
        .map(|(rel, bytes)| (rel.clone(), Arc::new(bytes.clone())))
        .collect();
    let retained = |buffers: &[(String, Arc<Vec<u8>>)]| {
        (buffers.iter())
            .filter(|(_, bytes)| Arc::strong_count(bytes) > 1)
            .count()
    };
    let (mut view, _) = WholeGraph::from_files(buffers.clone(), Vec::new(), false).unwrap();
    assert_eq!(
        retained(&buffers),
        0,
        "a read-only view holds no file bytes"
    );
    let error = view
        .replace_files(vec![("pages/P0.md".into(), None)])
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    let id = PageId::from("pages/P0.md");
    let dto = view.page(&id).unwrap();
    assert!(matches!(
        view.submit_page(&id, &dto, true),
        Err(PageRefusal::Failed { .. })
    ));
    drop(view);
    let (_editable, _) = WholeGraph::from_files(buffers.clone(), Vec::new(), true).unwrap();
    assert_eq!(
        retained(&buffers),
        buffers.len(),
        "an editable view holds each file"
    );
}
