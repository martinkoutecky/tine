//! Scaffolding for a brand-new graph created from Tine's onboarding wizard.
//!
//! A first-time user who has never used Logseq picks "Create a new graph" and
//! lands in a small, narrated demo graph that teaches Tine by example: a
//! "Welcome to Tine" tour plus a few linked/namespaced pages exercising block
//! references, embeds, tasks, and the app's less-obvious features. Everything
//! written here is ordinary Logseq Markdown — the same graph opens in Logseq.

use std::io;
use std::path::Path;

use crate::model::{atomic_write, Graph, PageKind};

/// `logseq/config.edn` for the demo graph (triple-lowbar namespace filenames,
/// the welcome page pinned as a favorite).
const CONFIG_EDN: &str = include_str!("templates/config.edn");

/// The capture-window screenshot embedded by the quick-capture page.
const QUICK_CAPTURE_PNG: &[u8] = include_bytes!("templates/assets/quick-capture.png");

/// (page title, Markdown body) for each page the demo graph ships with. Titles
/// with a `/` create namespaces (e.g. `Features/Quick capture`).
const PAGES: &[(&str, &str)] = &[
    ("Welcome to Tine", include_str!("templates/welcome.md")),
    (
        "Features/Quick capture",
        include_str!("templates/quick-capture.md"),
    ),
    (
        "Features/Tips & shortcuts",
        include_str!("templates/tips.md"),
    ),
    ("Features/PDF annotation", include_str!("templates/pdf.md")),
    ("Project/Roadmap", include_str!("templates/roadmap.md")),
];

/// Scaffold a fresh demo graph at `root`: the standard Logseq directory layout,
/// a config, the narrated welcome pages, and the embedded assets. `root` must be
/// an existing directory (ideally empty); existing files are never overwritten
/// blindly — callers pass a freshly-created or empty directory.
pub fn create_demo_graph(root: &Path) -> io::Result<()> {
    let logseq = root.join("logseq");
    std::fs::create_dir_all(&logseq)?;
    std::fs::create_dir_all(root.join("pages"))?;
    std::fs::create_dir_all(root.join("journals"))?;
    let assets = root.join("assets");
    std::fs::create_dir_all(&assets)?;

    // Config first, so opening the graph below picks up the triple-lowbar
    // filename encoding the page paths are resolved with.
    atomic_write(&logseq.join("config.edn"), CONFIG_EDN.as_bytes())?;
    atomic_write(&assets.join("quick-capture.png"), QUICK_CAPTURE_PNG)?;

    let graph = Graph::open(root);
    for (title, body) in PAGES {
        let path = graph.path_for(title, PageKind::Page);
        atomic_write(&path, body.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, FileNameFormat};

    /// The bullet on Project/Roadmap that Welcome both references and embeds.
    const TARGET_ID: &str = "7a1c0f5e-0000-4000-8000-000000000001";

    #[test]
    fn demo_graph_scaffolds_and_resolves() {
        let dir = std::env::temp_dir().join(format!("tine-onboard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        create_demo_graph(&dir).unwrap();

        // Standard Logseq layout + embedded asset.
        assert!(dir.join("logseq/config.edn").is_file());
        assert!(dir.join("journals").is_dir());
        assert!(dir.join("assets/quick-capture.png").is_file());

        // Config is the modern triple-lowbar form, so namespaces are `___` files.
        let cfg = Config::parse(&std::fs::read_to_string(dir.join("logseq/config.edn")).unwrap());
        assert_eq!(cfg.file_name_format, FileNameFormat::TripleLowbar);
        assert!(dir.join("pages/Features___Quick capture.md").is_file());
        assert!(dir.join("pages/Project___Roadmap.md").is_file());

        // Every page loads by its (namespace-decoded) title, and every page parses.
        let graph = Graph::open(&dir);
        let entries = graph.list_pages();
        for (title, _) in PAGES {
            let entry = entries
                .iter()
                .find(|e| e.name == *title)
                .unwrap_or_else(|| panic!("page {title:?} not listed"));
            graph
                .load_page(entry)
                .unwrap_or_else(|e| panic!("page {title:?} failed to load: {e}"));
        }

        // The reference + the embed in Welcome both point at the Roadmap bullet:
        // it resolves, and its referrer count is 2 (no dangling refs).
        assert!(
            graph.resolve_block(TARGET_ID).is_some(),
            "block-ref target missing"
        );
        let counts = graph.block_ref_counts();
        assert_eq!(
            counts.get(TARGET_ID).copied(),
            Some(2),
            "expected 2 referrers of the demo block"
        );

        // Good outliner structure: a heading bullet actually PARENTS the body that
        // belongs to it (proper indentation), rather than leaving it as flat
        // siblings. Verify on the Welcome page.
        let welcome = entries
            .iter()
            .find(|e| e.name == "Welcome to Tine")
            .unwrap();
        let dto = graph.load_page(welcome).unwrap();
        let parents = |needle: &str| {
            dto.blocks
                .iter()
                .any(|b| b.raw.starts_with(needle) && !b.children.is_empty())
        };
        assert!(
            parents("## Try the basics"),
            "section heading should parent its body"
        );
        assert!(
            parents("# Welcome to Tine"),
            "page heading should parent its intro"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
