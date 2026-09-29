//! Step-0 evidence for the graph-text-scope rows (master 993b8445a / GH #246):
//! og already discovers Markdown and Org documents anywhere in the graph under
//! the same fixed-exclusion, dot-directory, configured-`:hidden` and sync
//! conflict-copy policy as master's `GraphTextScope`, and never rewrites a
//! document it did not save. master's binding/digest identity for that policy
//! exists only to key its oplog/enrollment/restore machinery (excluded here).

use std::path::PathBuf;
use tine_store::{OpenOptions, Resolved, Store};

fn write(root: &std::path::Path, path: &str, bytes: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, bytes).unwrap();
}

#[test]
fn documents_outside_the_configured_roots_follow_the_graph_text_policy() {
    let root: PathBuf = std::env::temp_dir().join(format!("tine-246-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["content/pages", "content/journals"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    write(
        &root,
        "logseq/config.edn",
        "{:pages-directory \"content/pages\"\n :journals-directory \"content/journals\"\n :hidden [\"archive/private\"]}\n",
    );
    let accepted = [
        ("Root.md", "Root"),
        ("UPPER.MD", "UPPER"),
        ("external/multi/level/Long.markdown", "Long"),
        ("external/multi/level/Outline.org", "Outline"),
        ("content/pages/nested/Configured.md", "Configured"),
    ];
    for (path, _) in accepted {
        write(&root, path, "- body\n");
    }
    let excluded = [
        ("archive/private/hidden.md", "hidden"),
        ("node_modules/pkg/nm.md", "nm"),
        ("logseq/bak/bakpage.md", "bakpage"),
        ("logseq/version-files/versioned.md", "versioned"),
        ("assets/asset-doc.md", "asset-doc"),
        ("publish/published.org", "published"),
        (".dot/dotdoc.md", "dotdoc"),
        (
            "external/Page (conflicted copy 2026-07-25).md",
            "Page (conflicted copy 2026-07-25)",
        ),
    ];
    for (path, _) in excluded {
        write(&root, path, "- excluded\n");
    }

    let store = Store::open(&root, OpenOptions::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    for (path, name) in accepted {
        assert!(
            matches!(graph.resolve(name, false), Resolved::Existing { .. }),
            "{path} must be discoverable as {name}"
        );
    }
    for (path, name) in excluded {
        assert!(
            !matches!(graph.resolve(name, false), Resolved::Existing { .. }),
            "{path} must stay outside the graph text scope"
        );
    }
    drop(graph);
    // The area scans the backup path is built on stay confined to the configured
    // roots, so Root.md and external/ are discovered above but not in any area
    // listing (see RECEIPT-22D: the backup/restore half of ffb4cb3d7 is open).
    for area in [tine_store::Area::Pages, tine_store::Area::Journals] {
        let listing = store.scan_area(area, None).unwrap();
        assert!(
            listing
                .files
                .iter()
                .all(|f| !f.rel.ends_with("Root.md") && !f.rel.contains("external")),
            "an area scan is confined to its configured root"
        );
    }
    drop(store);
    assert_eq!(std::fs::read(root.join("Root.md")).unwrap(), b"- body\n");
    std::fs::remove_dir_all(&root).ok();
}
