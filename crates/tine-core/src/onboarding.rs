//! Scaffolding for a brand-new graph created from Tine's onboarding wizard.
//!
//! A first-time user who has never used Logseq picks "Create a new graph" and
//! lands in a small, narrated demo graph that teaches Tine by example: a
//! "Welcome to Tine" tour plus a few linked/namespaced pages exercising block
//! references, embeds, tasks, and the app's less-obvious features. Everything
//! written here is ordinary Logseq Markdown — the same graph opens in Logseq.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::Path;

use crate::model::{atomic_write, markdown_page_dto, Graph, PageDto, PageKind};

/// `logseq/config.edn` for the demo graph (triple-lowbar namespace filenames,
/// the welcome page pinned as a favorite).
const CONFIG_EDN: &str = include_str!("templates/config.edn");

/// The capture-window screenshot embedded by the quick-capture page.
const QUICK_CAPTURE_PNG: &[u8] = include_bytes!("templates/assets/quick-capture.png");

/// (page title, Markdown body) for each page the demo graph ships with. Titles
/// with a `/` create namespaces (e.g. `Features/Quick capture`).
const PAGES: &[(&str, &str)] = &[
    ("Welcome to Tine", include_str!("templates/welcome.md")),
    ("Tine Guide", include_str!("templates/guide.md")),
    (
        "Features/Quick capture",
        include_str!("templates/quick-capture.md"),
    ),
    (
        "Features/Tips & shortcuts",
        include_str!("templates/tips.md"),
    ),
    ("Features/Sheets", include_str!("templates/sheets.md")),
    ("Features/Formulas", include_str!("templates/formulas.md")),
    ("Features/PDF annotation", include_str!("templates/pdf.md")),
    ("Feature showcase", include_str!("templates/showcase.md")),
    ("Project/Roadmap", include_str!("templates/roadmap.md")),
];

/// In-memory namespace for bundled, read-only guide pages. These pages are
/// rendered live in the running app but are not graph files.
pub const GUIDE_DISPLAY_PREFIX: &str = "Tine-guide/";

/// Real graph namespace used only by the explicit guide-copy action.
pub const GUIDE_COPY_PREFIX: &str = "tine-guide/";

struct GuideTemplate {
    title: &'static str,
    markdown: &'static str,
}

const GUIDE_TEMPLATES: &[GuideTemplate] = &[
    GuideTemplate {
        title: "Tine Guide",
        markdown: include_str!("templates/guide.md"),
    },
    GuideTemplate {
        title: "Features/Sheets",
        markdown: include_str!("templates/sheets.md"),
    },
    GuideTemplate {
        title: "Features/Formulas",
        markdown: include_str!("templates/formulas.md"),
    },
    GuideTemplate {
        title: "Features/Quick capture",
        markdown: include_str!("templates/quick-capture.md"),
    },
    GuideTemplate {
        title: "Features/PDF annotation",
        markdown: include_str!("templates/pdf.md"),
    },
    GuideTemplate {
        title: "Features/Tips & shortcuts",
        markdown: include_str!("templates/tips.md"),
    },
    GuideTemplate {
        title: "Feature showcase",
        markdown: include_str!("templates/showcase.md"),
    },
];

struct GuideAsset {
    name: &'static str,
    bytes: &'static [u8],
}

const GUIDE_ASSETS: &[GuideAsset] = &[GuideAsset {
    name: "quick-capture.png",
    bytes: QUICK_CAPTURE_PNG,
}];

#[derive(Debug, Clone, serde::Serialize)]
pub struct GuidePage {
    pub title: String,
    pub markdown: String,
    pub page: PageDto,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GuideCopyResult {
    pub name: String,
    pub created: bool,
    pub created_pages: Vec<String>,
    pub skipped_pages: Vec<String>,
    pub copied_assets: Vec<String>,
}

pub fn guide_page_name(title: &str) -> String {
    format!("{GUIDE_DISPLAY_PREFIX}{title}")
}

pub fn guide_copy_page_name(title: &str) -> String {
    format!("{GUIDE_COPY_PREFIX}{title}")
}

pub fn bundled_guide_pages() -> Vec<GuidePage> {
    GUIDE_TEMPLATES
        .iter()
        .map(|t| {
            let mut page = markdown_page_dto(&guide_page_name(t.title), t.title, t.markdown);
            page.read_only = true;
            page.guide = true;
            GuidePage {
                title: t.title.to_string(),
                markdown: t.markdown.to_string(),
                page,
            }
        })
        .collect()
}

#[cfg(test)]
fn rewrite_guide_links(markdown: &str, copied_titles: &[&str]) -> String {
    let renames: HashMap<String, String> = copied_titles
        .iter()
        .map(|title| {
            (
                crate::refs::page_key(title),
                guide_copy_page_name(title.trim()),
            )
        })
        .collect();
    rewrite_bundled_guide_links(markdown, &renames)
}

fn guide_link_renames() -> HashMap<String, String> {
    GUIDE_TEMPLATES
        .iter()
        .map(|template| {
            (
                crate::refs::page_key(template.title),
                guide_copy_page_name(template.title),
            )
        })
        .collect()
}

fn rewrite_bundled_guide_links(markdown: &str, renames: &HashMap<String, String>) -> String {
    crate::refs::rename_refs_multi(markdown, renames, false)
}

pub fn copy_guide_into_graph(graph: &Graph, title: &str) -> io::Result<GuideCopyResult> {
    let Some(viewed) = GUIDE_TEMPLATES
        .iter()
        .find(|t| crate::refs::same_page(t.title, title))
    else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "unknown bundled guide page",
        ));
    };
    let renames = guide_link_renames();
    let mut created_pages = Vec::new();
    let mut skipped_pages = Vec::new();
    for template in GUIDE_TEMPLATES {
        let name = guide_copy_page_name(template.title);
        let markdown = rewrite_bundled_guide_links(template.markdown, &renames);
        if graph.create_markdown_page_if_absent(&name, &markdown)? {
            created_pages.push(name);
        } else {
            skipped_pages.push(name);
        }
    }
    let copied_assets = copy_referenced_guide_assets(graph)?;
    let created = !created_pages.is_empty() || !copied_assets.is_empty();
    Ok(GuideCopyResult {
        name: guide_copy_page_name(viewed.title),
        created,
        created_pages,
        skipped_pages,
        copied_assets,
    })
}

fn copy_referenced_guide_assets(graph: &Graph) -> io::Result<Vec<String>> {
    let mut referenced = HashSet::new();
    for template in GUIDE_TEMPLATES {
        collect_guide_asset_refs(template.markdown, &mut referenced);
    }
    let mut referenced: Vec<String> = referenced.into_iter().collect();
    referenced.sort();

    let assets = graph.assets_path();
    let mut copied = Vec::new();
    for name in referenced {
        if name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "guide assets must be top-level files",
            ));
        }
        let Some(asset) = GUIDE_ASSETS.iter().find(|asset| asset.name == name) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("missing bundled guide asset {name}"),
            ));
        };
        std::fs::create_dir_all(&assets)?;
        let path = assets.join(&name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(asset.bytes)?;
                file.sync_all()?;
                copied.push(name);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(copied)
}

fn collect_guide_asset_refs(markdown: &str, into: &mut HashSet<String>) {
    let mut rest = markdown;
    while let Some(i) = rest.find("../assets/") {
        let after = &rest[i + "../assets/".len()..];
        let end = after
            .find(|c: char| {
                matches!(
                    c,
                    ')' | ']' | '"' | '\'' | '<' | '>' | '|' | '\n' | '\r' | '\t'
                )
            })
            .unwrap_or(after.len());
        let name = after[..end].trim();
        if !name.is_empty() {
            into.insert(name.to_string());
        }
        rest = &after[end..];
    }
}

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

    fn scratch(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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
        assert!(dir.join("pages/Tine Guide.md").is_file());
        assert!(dir.join("pages/Feature showcase.md").is_file());
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

    #[test]
    fn bundled_guide_pages_are_read_only_virtual_pages() {
        let pages = bundled_guide_pages();
        let index = pages
            .iter()
            .find(|p| p.title == "Tine Guide")
            .expect("guide index is bundled");
        assert_eq!(index.page.name, "Tine-guide/Tine Guide");
        assert!(index.page.read_only);
        assert!(index.page.guide);

        let sheets = pages
            .iter()
            .find(|p| p.title == "Features/Sheets")
            .expect("sheets guide is bundled");
        assert!(sheets.markdown.contains("Create one yourself"));
        assert!(sheets.page.blocks.iter().any(|b| b.raw.contains("Positional grid")));
    }

    #[test]
    fn guide_copy_rewrites_interguide_page_refs_only() {
        let copied = [
            "Tine Guide",
            "Features/Sheets",
            "Features/Formulas",
            "Features/Quick capture",
            "Features/PDF annotation",
            "Features/Tips & shortcuts",
            "Feature showcase",
        ];
        let index = GUIDE_TEMPLATES
            .iter()
            .find(|p| p.title == "Tine Guide")
            .unwrap()
            .markdown;
        let mut sample = index.to_string();
        sample.push_str(
            "\n- [[Martin]] #demo #sheets-demo\n- [read showcase]([[Feature showcase]])\n- {{embed [[Features/Tips & shortcuts]]}}\n- {{query [[Features/Quick capture]]}}\n",
        );
        let out = rewrite_guide_links(&sample, &copied);
        assert!(
            out.contains("[[tine-guide/Features/Sheets]]"),
            "index link was not rewritten: {out}"
        );
        assert!(
            out.contains("[read showcase]([[tine-guide/Feature showcase]])"),
            "labelled page link was not rewritten: {out}"
        );
        assert!(
            out.contains("{{embed [[tine-guide/Features/Tips & shortcuts]]}}"),
            "embed page link was not rewritten: {out}"
        );
        assert!(
            out.contains("{{query [[tine-guide/Features/Quick capture]]}}"),
            "query page link was not rewritten: {out}"
        );
        assert!(
            out.contains("[[Martin]] #demo #sheets-demo"),
            "non-guide refs must stay verbatim: {out}"
        );
    }

    #[test]
    fn copy_guide_into_graph_writes_whole_lowercase_namespace_and_assets() {
        let dir = scratch("tine-guide-copy-whole");
        let graph = Graph::open(&dir);

        let copied = copy_guide_into_graph(&graph, "Features/Sheets").unwrap();
        assert_eq!(copied.name, "tine-guide/Features/Sheets");
        assert!(copied.created);
        assert_eq!(copied.created_pages.len(), GUIDE_TEMPLATES.len());
        assert!(copied.skipped_pages.is_empty());
        assert_eq!(copied.copied_assets, vec!["quick-capture.png".to_string()]);

        for guide in bundled_guide_pages() {
            let name = guide_copy_page_name(&guide.title);
            let path = graph.path_for(&name, PageKind::Page);
            assert!(path.is_file(), "missing copied guide page {name}");
            let dto = graph
                .load_named(&name, PageKind::Page)
                .unwrap()
                .unwrap_or_else(|| panic!("copied guide page {name} should load"));
            assert_eq!(dto.name, name);
            assert!(
                !dto.read_only && !dto.guide,
                "copied guide page must be normal/editable: {name}"
            );
        }

        let index = std::fs::read_to_string(graph.path_for("tine-guide/Tine Guide", PageKind::Page))
            .unwrap();
        assert!(index.contains("[[tine-guide/Features/Sheets]]"));
        assert!(!index.contains("[[Features/Sheets]]"));
        assert!(dir.join("assets/quick-capture.png").is_file());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recopy_guide_skips_existing_pages_without_clobbering_user_edits() {
        let dir = scratch("tine-guide-recopy");
        let graph = Graph::open(&dir);

        copy_guide_into_graph(&graph, "Tine Guide").unwrap();
        let edited = guide_copy_page_name("Features/Sheets");
        let edited_path = graph.path_for(&edited, PageKind::Page);
        std::fs::write(&edited_path, "- user edits stay\n").unwrap();

        let before: std::collections::HashMap<String, String> = GUIDE_TEMPLATES
            .iter()
            .map(|template| {
                let name = guide_copy_page_name(template.title);
                let body = std::fs::read_to_string(graph.path_for(&name, PageKind::Page)).unwrap();
                (name, body)
            })
            .collect();

        let existing = copy_guide_into_graph(&graph, "Features/Sheets").unwrap();
        assert_eq!(existing.name, edited);
        assert!(!existing.created);
        assert!(existing.created_pages.is_empty());
        assert_eq!(existing.skipped_pages.len(), GUIDE_TEMPLATES.len());
        for (name, body) in before {
            assert_eq!(
                std::fs::read_to_string(graph.path_for(&name, PageKind::Page)).unwrap(),
                body,
                "recopy clobbered {name}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&edited_path).unwrap(),
            "- user edits stay\n"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
