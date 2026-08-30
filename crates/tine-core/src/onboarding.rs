//! Scaffolding for a brand-new graph created from Tine's onboarding wizard.
//!
//! A first-time user who has never used Logseq picks "Create a new graph" and
//! lands in a small, narrated demo graph that teaches Tine by example: a
//! "Welcome to Tine" tour plus a few linked/namespaced pages exercising block
//! references, embeds, tasks, and the app's less-obvious features. Everything
//! written here is ordinary Logseq Markdown — the same graph opens in Logseq.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use crate::model::{atomic_write_new, markdown_page_dto, Graph, PageDto, PageKind};

/// `logseq/config.edn` for the demo graph (triple-lowbar namespace filenames,
/// the welcome page pinned as a favorite).
const CONFIG_EDN: &str = include_str!("templates/config.edn");

/// The capture-window screenshot embedded by the quick-capture page.
const QUICK_CAPTURE_PNG: &[u8] = include_bytes!("templates/assets/quick-capture.png");

/// In-memory namespace for bundled, read-only guide pages. These pages are
/// rendered live in the running app but are not graph files.
pub const GUIDE_DISPLAY_PREFIX: &str = "Tine-guide/";

/// Real graph namespace used only by the explicit guide-copy action.
pub const GUIDE_COPY_PREFIX: &str = "tine-guide/";

/// One canonical manifest feeds all three Guide surfaces: the onboarding graph,
/// the in-app read-only Guide, and the generated website demo. Keeping the list
/// in one place prevents a page from silently disappearing from one surface.
struct GuideTemplate {
    title: &'static str,
    markdown: &'static str,
}

const GUIDE_TEMPLATES: &[GuideTemplate] = &[
    GuideTemplate {
        title: "Tine Guide",
        markdown: include_str!("templates/guide.md"),
    },
    // Welcome + Roadmap are link/block-ref targets of the other guide pages
    // (showcase → [[Welcome to Tine]]; welcome → [[Project/Roadmap]] + a block
    // over on Roadmap). The guide set must stay *closed* under its own links, or
    // those links dangle in the in-app guide and in the copied-into-graph copy.
    // The `guide_link_set_is_closed` test enforces this invariant.
    GuideTemplate {
        title: "Welcome to Tine",
        markdown: include_str!("templates/welcome.md"),
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
        title: "Features/Plugins",
        markdown: include_str!("templates/plugins.md"),
    },
    GuideTemplate {
        title: "Features/Managed sync",
        markdown: include_str!("templates/managed-sync.md"),
    },
    GuideTemplate {
        title: "Features/Tips & shortcuts",
        markdown: include_str!("templates/tips.md"),
    },
    GuideTemplate {
        title: "Feature showcase",
        markdown: include_str!("templates/showcase.md"),
    },
    GuideTemplate {
        title: "Project/Roadmap",
        markdown: include_str!("templates/roadmap.md"),
    },
    GuideTemplate {
        title: "Workflows/Structure repeated information",
        markdown: include_str!("templates/structure-repeated-information.md"),
    },
    GuideTemplate {
        title: "Reference/Files, external edits, and backups",
        markdown: include_str!("templates/files-external-edits-backups.md"),
    },
    GuideTemplate {
        title: "Start/Bring an existing graph",
        markdown: include_str!("templates/bring-existing-graph.md"),
    },
    GuideTemplate {
        title: "Reference/Troubleshooting and recovery",
        markdown: include_str!("templates/troubleshooting-recovery.md"),
    },
    GuideTemplate {
        title: "Workflows/Capture and plan your day",
        markdown: include_str!("templates/capture-plan-day.md"),
    },
    GuideTemplate {
        title: "Reference/Journals, tasks, and scheduling",
        markdown: include_str!("templates/journals-tasks-scheduling.md"),
    },
    GuideTemplate {
        title: "Workflows/Find and revisit",
        markdown: include_str!("templates/find-and-revisit.md"),
    },
    GuideTemplate {
        title: "Reference/Pages, links, references, and search",
        markdown: include_str!("templates/pages-links-references-search.md"),
    },
    GuideTemplate {
        title: "Workflows/Research a document",
        markdown: include_str!("templates/research-document.md"),
    },
    GuideTemplate {
        title: "Start/Where things are",
        markdown: include_str!("templates/where-things-are.md"),
    },
    GuideTemplate {
        title: "Workflows/Keep context visible",
        markdown: include_str!("templates/keep-context-visible.md"),
    },
    GuideTemplate {
        title: "Workflows/Extend Tine",
        markdown: include_str!("templates/extend-tine.md"),
    },
    GuideTemplate {
        title: "Reference/Platforms and mobile",
        markdown: include_str!("templates/platforms-and-mobile.md"),
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuideCopyResult {
    pub name: String,
    pub created: bool,
    pub created_pages: Vec<String>,
    pub skipped_pages: Vec<String>,
    pub copied_assets: Vec<String>,
}

pub(crate) struct GuideCopyPage {
    pub(crate) name: String,
    pub(crate) markdown: String,
    pub(crate) page: PageDto,
}

pub(crate) struct GuideCopyAsset {
    pub(crate) name: String,
    pub(crate) bytes: &'static [u8],
}

pub(crate) struct GuideCopyPlan {
    pub(crate) viewed_name: String,
    pub(crate) pages: Vec<GuideCopyPage>,
    pub(crate) assets: Vec<GuideCopyAsset>,
}

pub fn guide_page_name(title: &str) -> String {
    format!("{GUIDE_DISPLAY_PREFIX}{title}")
}

pub fn guide_copy_page_name(title: &str) -> String {
    format!("{GUIDE_COPY_PREFIX}{title}")
}

pub fn bundled_guide_pages() -> io::Result<Vec<GuidePage>> {
    GUIDE_TEMPLATES
        .iter()
        .map(|t| {
            let mut page = markdown_page_dto(&guide_page_name(t.title), t.title, t.markdown)?;
            page.read_only = true;
            page.guide = true;
            Ok(GuidePage {
                title: t.title.to_string(),
                markdown: t.markdown.to_string(),
                page,
            })
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

fn bind_copied_page_title(markdown: String, copied_name: &str) -> String {
    let Some(first_newline) = markdown.find('\n') else {
        return markdown;
    };
    let first = markdown[..first_newline].trim_end_matches('\r');
    let Some((key, _)) = first.split_once("::") else {
        return markdown;
    };
    if !key.trim().eq_ignore_ascii_case("title") {
        return markdown;
    }
    let newline = if markdown[..first_newline].ends_with('\r') {
        "\r\n"
    } else {
        "\n"
    };
    format!(
        "title:: {copied_name}{newline}{}",
        &markdown[first_newline + 1..]
    )
}

pub fn copy_guide_into_graph(graph: &Graph, title: &str) -> io::Result<GuideCopyResult> {
    let plan = guide_copy_plan(title)?;
    // Name-only creation needs one current parsed identity snapshot. App-open
    // graphs already have it; keep this public operation correct for cold callers.
    graph.with_pages(|_| ());
    graph.with_graph_text_write_transaction(move || {
        let mut created_pages = Vec::new();
        let mut skipped_pages = Vec::new();
        for planned in plan.pages {
            if graph.create_markdown_page_if_absent(&planned.name, &planned.markdown)? {
                created_pages.push(planned.name);
            } else {
                skipped_pages.push(planned.name);
            }
        }
        let mut copied_assets = Vec::new();
        for asset in plan.assets {
            if graph.create_asset_if_absent(&asset.name, asset.bytes)? {
                copied_assets.push(asset.name);
            }
        }
        let created = !created_pages.is_empty() || !copied_assets.is_empty();
        Ok(GuideCopyResult {
            name: plan.viewed_name,
            created,
            created_pages,
            skipped_pages,
            copied_assets,
        })
    })
}

pub(crate) fn guide_copy_plan(title: &str) -> io::Result<GuideCopyPlan> {
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
    let pages = GUIDE_TEMPLATES
        .iter()
        .map(|template| {
            let name = guide_copy_page_name(template.title);
            let markdown = bind_copied_page_title(
                rewrite_bundled_guide_links(template.markdown, &renames),
                &name,
            );
            let page = markdown_page_dto(&name, &name, &markdown)?;
            Ok(GuideCopyPage {
                name,
                markdown,
                page,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let assets = referenced_guide_assets()?
        .into_iter()
        .map(|asset| GuideCopyAsset {
            name: asset.name.to_owned(),
            bytes: asset.bytes,
        })
        .collect();
    Ok(GuideCopyPlan {
        viewed_name: guide_copy_page_name(viewed.title),
        pages,
        assets,
    })
}

fn referenced_guide_assets() -> io::Result<Vec<&'static GuideAsset>> {
    let mut referenced = HashSet::new();
    for template in GUIDE_TEMPLATES {
        collect_guide_asset_refs(template.markdown, &mut referenced);
    }
    let mut referenced: Vec<String> = referenced.into_iter().collect();
    referenced.sort();

    let mut assets = Vec::new();
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
        assets.push(asset);
    }
    Ok(assets)
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
    atomic_write_new(&logseq.join("config.edn"), CONFIG_EDN.as_bytes())?;
    atomic_write_new(&assets.join("quick-capture.png"), QUICK_CAPTURE_PNG)?;

    let graph = Graph::open(root);
    for template in GUIDE_TEMPLATES {
        let path = graph.path_for(template.title, PageKind::Page);
        atomic_write_new(&path, template.markdown.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, FileNameFormat};
    use std::collections::HashSet;

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
        assert!(dir
            .join("pages/Workflows___Structure repeated information.md")
            .is_file());

        // Every page loads by its (namespace-decoded) title, and every page parses.
        let graph = Graph::open(&dir);
        let entries = graph.list_pages();
        for template in GUIDE_TEMPLATES {
            let title = template.title;
            let entry = entries
                .iter()
                .find(|e| e.name == title)
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
        let counts = graph.block_ref_counts().unwrap();
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
        let pages = bundled_guide_pages().unwrap();
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
        assert!(sheets
            .page
            .blocks
            .iter()
            .any(|b| b.raw.contains("Positional grid")));

        let plugins = pages
            .iter()
            .find(|p| p.title == "Features/Plugins")
            .expect("plugins guide is bundled");
        assert!(plugins.markdown.contains("installed disabled"));
        assert!(plugins.markdown.contains("not Logseq or Obsidian plugins"));
    }

    #[test]
    fn structure_workflow_is_registered_linked_copyable_and_executable() {
        let title = "Workflows/Structure repeated information";
        let workflow = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("structure workflow is registered");
        assert!(workflow
            .markdown
            .contains("- # Structure repeated information"));
        assert!(workflow
            .markdown
            .contains("tine.fields:: status=enum:planned,active,done;owner=text;estimate=number"));
        assert!(workflow
            .markdown
            .contains("{{query (property owner Avery)}}"));
        assert!(workflow
            .markdown
            .contains("What you should see: a reusable selection of matching tracker blocks"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Workflows/Structure repeated information]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("structure workflow is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Workflows/Structure repeated information"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-structure-workflow-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Workflows/Structure repeated information"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("structure workflow was copied");
        assert!(copied_markdown.contains("{{query (property owner Avery)}}"));
        assert!(copied_markdown.contains("[[tine-guide/Features/Sheets]]"));
        assert!(copied_markdown.contains("[[tine-guide/Features/Formulas]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[derive(serde::Deserialize)]
    struct DeliberateGuideStub {
        title: String,
    }

    fn deliberate_guide_stub_keys() -> HashSet<String> {
        serde_json::from_str::<Vec<DeliberateGuideStub>>(include_str!(
            "../../../docs/guide-deliberate-stubs.json"
        ))
        .expect("guide deliberate-stub allowlist is valid JSON")
        .into_iter()
        .map(|stub| crate::refs::page_key(&stub.title))
        .collect()
    }

    fn guide_reference_targets(markdown: &str) -> Vec<String> {
        fn collect(blocks: &[crate::doc::DocBlock], targets: &mut Vec<String>) {
            for block in blocks {
                // `DocBlock::projection` is the canonical lsdoc-backed reference
                // extraction used by the graph's index and backlink paths.
                let projection = block.projection();
                targets.extend(
                    projection
                        .refs_page
                        .iter()
                        .filter(|target| !projection.block_refs.contains(target))
                        .cloned(),
                );
                collect(&block.children, targets);
            }
        }

        let document = crate::doc::parse(markdown);
        let mut targets = Vec::new();
        collect(&document.roots, &mut targets);
        targets
    }

    fn guide_template_link_errors(templates: &[GuideTemplate]) -> Vec<String> {
        let registered: HashSet<String> = templates
            .iter()
            .map(|template| crate::refs::page_key(template.title))
            .collect();
        let deliberate_stubs = deliberate_guide_stub_keys();
        let mut errors = Vec::new();
        for template in templates {
            for target in guide_reference_targets(template.markdown) {
                let key = crate::refs::page_key(&target);
                if !registered.contains(&key) && !deliberate_stubs.contains(&key) {
                    errors.push(format!(
                        "{}: unregistered Guide target {target}",
                        template.title
                    ));
                }
            }
        }
        errors
    }

    #[test]
    fn guide_link_set_is_closed_over_demo_pages() {
        let errors = guide_template_link_errors(GUIDE_TEMPLATES);
        assert!(
            errors.is_empty(),
            "Guide links must target a registered page or deliberate stub:\n{}",
            errors.join("\n")
        );
    }

    #[test]
    fn files_reference_page_is_registered_linked_and_copyable() {
        let title = "Reference/Files, external edits, and backups";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("files reference page is registered");
        assert!(page
            .markdown
            .contains("- # Files, external edits, and backups"));
        assert!(page.markdown.contains("logseq/.tine-trash"));
        assert!(page.markdown.contains("Watch for external edits"));
        assert!(page.markdown.contains("Snapshots to keep"));
        assert!(page.markdown.contains("Verify synchronized graph"));
        assert!(page.markdown.contains("`logseq/config.edn` is live too"));
        assert!(page.markdown.contains("Plain text (cleaned, as displayed)"));
        assert!(page.markdown.contains("What you should see"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Reference/Files, external edits, and backups]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("files reference is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Reference/Files, external edits, and backups"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-files-reference-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Reference/Files, external edits, and backups"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("files reference was copied");
        assert!(copied_markdown.contains("logseq/.tine-trash"));
        assert!(copied_markdown.contains("[[tine-guide/Features/Managed sync]]"));
        assert!(copied_markdown.contains("[[tine-guide/Features/Sheets]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bring_existing_graph_page_is_registered_linked_and_copyable() {
        let title = "Start/Bring an existing graph";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("bring-an-existing-graph page is registered");
        assert!(page.markdown.contains("- # Bring an existing graph"));
        assert!(page.markdown.contains("Open an existing graph"));
        assert!(page.markdown.contains("What you should see"));
        assert!(page
            .markdown
            .contains("[[Reference/Files, external edits, and backups]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index.markdown.contains("[[Start/Bring an existing graph]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("bring-an-existing-graph is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Start/Bring an existing graph"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-bring-existing-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Start/Bring an existing graph"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("bring-an-existing-graph was copied");
        assert!(copied_markdown.contains("Open an existing graph"));
        assert!(
            copied_markdown.contains("[[tine-guide/Reference/Files, external edits, and backups]]")
        );
        assert!(copied_markdown.contains("[[tine-guide/Features/Managed sync]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn troubleshooting_page_is_registered_linked_and_copyable() {
        let title = "Reference/Troubleshooting and recovery";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("troubleshooting page is registered");
        assert!(page.markdown.contains("- # Troubleshooting and recovery"));
        assert!(page.markdown.contains("TINE_DEBUG=1"));
        assert!(page.markdown.contains("Help improve Tine"));
        assert!(page
            .markdown
            .contains("Create a privacy-safe diagnostic report"));
        assert!(page.markdown.contains("**Verify synchronized graph**"));
        assert!(page.markdown.contains("Use disk version"));
        assert!(page.markdown.contains("What you should see"));
        assert!(page
            .markdown
            .contains("[[Reference/Files, external edits, and backups]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Reference/Troubleshooting and recovery]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("troubleshooting page is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Reference/Troubleshooting and recovery"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-troubleshooting-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Reference/Troubleshooting and recovery"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("troubleshooting page was copied");
        assert!(copied_markdown.contains("TINE_DEBUG=1"));
        assert!(
            copied_markdown.contains("[[tine-guide/Reference/Files, external edits, and backups]]")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn absence_sweep_recovery_is_taught_in_both_guide_surfaces() {
        let managed = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Features/Managed sync")
            .expect("managed-sync page is registered");
        assert!(managed
            .markdown
            .contains("Review a detected group deletion"));
        assert!(managed.markdown.contains("Four deleted pages"));
        assert!(managed.markdown.contains("**Restore**"));
        assert!(managed.markdown.contains("**Re-apply**"));
        assert!(managed.markdown.contains("**Keep deletion**"));
        assert!(managed.markdown.contains("Run Restore again"));
        assert!(managed
            .markdown
            .contains("Closing either the warning or the panel makes no decision"));
        assert!(managed
            .markdown
            .contains("[[Reference/Troubleshooting and recovery]]"));

        let recovery = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Reference/Troubleshooting and recovery")
            .expect("recovery page is registered");
        assert!(recovery
            .markdown
            .contains("Review several deletions in Tine-managed storage"));
        assert!(recovery
            .markdown
            .contains("Closing the warning or panel records no choice"));
        assert!(recovery.markdown.contains("finished sweep remains visible"));

        let dir = scratch("tine-guide-absence-sweep-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, "Features/Managed sync").unwrap();
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("managed-sync page was copied");
        assert!(copied_markdown.contains("Run Restore again"));
        assert!(copied_markdown.contains("[[tine-guide/Reference/Troubleshooting and recovery]]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_plan_day_workflow_is_registered_linked_and_copyable() {
        let title = "Workflows/Capture and plan your day";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("capture-and-plan workflow is registered");
        assert!(page.markdown.contains("- # Capture and plan your day"));
        assert!(page.markdown.contains("**Ctrl+Enter**"));
        assert!(page
            .markdown
            .contains("{{query (task TODO DOING NOW LATER)}}"));
        assert!(page.markdown.contains("Carry unfinished tasks"));
        assert!(page.markdown.contains("What you should see"));
        assert!(page.markdown.contains("[[Features/Quick capture]]"));
        assert!(page
            .markdown
            .contains("[[Reference/Troubleshooting and recovery]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Workflows/Capture and plan your day]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("capture-and-plan workflow is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Workflows/Capture and plan your day"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-capture-plan-day-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Workflows/Capture and plan your day"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("capture-and-plan workflow was copied");
        assert!(copied_markdown.contains("{{query (task TODO DOING NOW LATER)}}"));
        assert!(copied_markdown.contains("[[tine-guide/Features/Quick capture]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journals_scheduling_reference_page_is_registered_linked_and_copyable() {
        let title = "Reference/Journals, tasks, and scheduling";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("journals/scheduling reference page is registered");
        assert!(page
            .markdown
            .contains("- # Journals, tasks, and scheduling"));
        assert!(page.markdown.contains("TODO → DOING → DONE"));
        assert!(page.markdown.contains("TODO ↔ DOING or LATER ↔ NOW"));
        assert!(page.markdown.contains("Scheduled &amp; Deadline"));
        assert!(page.markdown.contains("`++1w`"));
        assert!(page
            .markdown
            .contains("(not (task DONE CANCELED CANCELLED))"));
        assert!(page
            .markdown
            .contains("[[Workflows/Capture and plan your day]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Reference/Journals, tasks, and scheduling]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("journals/scheduling reference page is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Reference/Journals, tasks, and scheduling"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-journals-scheduling-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Reference/Journals, tasks, and scheduling"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("journals/scheduling reference page was copied");
        assert!(copied_markdown.contains("`++1w`"));
        assert!(copied_markdown.contains("[[tine-guide/Workflows/Capture and plan your day]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_revisit_workflow_is_registered_linked_and_copyable() {
        let title = "Workflows/Find and revisit";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("find-and-revisit workflow is registered");
        assert!(page.markdown.contains("- # Find and revisit"));
        assert!(page.markdown.contains("**Ctrl+K**"));
        assert!(page.markdown.contains("Open search tab"));
        assert!(page.markdown.contains("+ New group"));
        assert!(page.markdown.contains("copy/export button"));
        assert!(page.markdown.contains("{{query [[Project/Roadmap]]}}"));
        assert!(page
            .markdown
            .contains("Name this search to save it as a page"));
        assert!(page.markdown.contains("What you should see"));
        assert!(page
            .markdown
            .contains("[[Reference/Pages, links, references, and search]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index.markdown.contains("[[Workflows/Find and revisit]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("find-and-revisit workflow is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Workflows/Find and revisit"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-find-revisit-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Workflows/Find and revisit"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("find-and-revisit workflow was copied");
        assert!(copied_markdown.contains("{{query [[tine-guide/Project/Roadmap]]}}"));
        assert!(copied_markdown
            .contains("[[tine-guide/Reference/Pages, links, references, and search]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pages_links_search_reference_is_registered_linked_and_copyable() {
        let title = "Reference/Pages, links, references, and search";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("pages/links/search reference page is registered");
        assert!(page
            .markdown
            .contains("- # Pages, links, references, and search"));
        assert!(page.markdown.contains("Unlinked References"));
        assert!(page.markdown.contains("available page/tag chips"));
        assert!(page.markdown.contains("**Copy / export**"));
        assert!(page.markdown.contains("dotted underline"));
        assert!(page.markdown.contains("alias:: Kitchen sink (features)"));
        assert!(page.markdown.contains("Save page"));
        assert!(page.markdown.contains("tine.view::"));
        assert!(page.markdown.contains("[[Workflows/Find and revisit]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Reference/Pages, links, references, and search]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("pages/links/search reference is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Reference/Pages, links, references, and search"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-pages-links-search-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Reference/Pages, links, references, and search"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("pages/links/search reference was copied");
        assert!(copied_markdown.contains("[[tine-guide/Workflows/Find and revisit]]"));
        assert!(copied_markdown.contains("[[tine-guide/Features/Tips & shortcuts]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn research_document_workflow_is_registered_linked_and_copyable() {
        let title = "Workflows/Research a document";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("research-document workflow is registered");
        assert!(page.markdown.contains("- # Research a document"));
        assert!(page.markdown.contains("**Notes**"));
        assert!(page.markdown.contains("**Copy ref**"));
        assert!(page.markdown.contains("hls__"));
        assert!(page.markdown.contains("normal tab in a companion pane"));
        assert!(page.markdown.contains("drag the PDF tab"));
        assert!(page.markdown.contains("structural companion pane"));
        assert!(page.markdown.contains("**Back** returns to the source page"));
        assert!(page.markdown.contains("What you should see"));
        assert!(page.markdown.contains("[[Features/PDF annotation]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index.markdown.contains("[[Workflows/Research a document]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("research-document workflow is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Workflows/Research a document"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-research-document-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Workflows/Research a document"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("research-document workflow was copied");
        assert!(copied_markdown.contains("[[tine-guide/Features/PDF annotation]]"));
        assert!(copied_markdown.contains("[[tine-guide/Workflows/Find and revisit]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn where_things_are_page_is_registered_linked_and_copyable() {
        let title = "Start/Where things are";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("where-things-are page is registered");
        assert!(page.markdown.contains("- # Where things are"));
        assert!(page.markdown.contains("**t l**"));
        assert!(page.markdown.contains("**t r**"));
        assert!(page.markdown.contains("**Shift+?**"));
        assert!(page.markdown.contains("Favorites can be arranged"));
        assert!(page.markdown.contains("[[Welcome to Tine]]"));
        assert!(page.markdown.contains("[[Workflows/Keep context visible]]"));
        assert!(page.markdown.contains("[[Features/Tips & shortcuts]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index.markdown.contains("[[Start/Where things are]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("where-things-are is available in the read-only Guide");
        assert_eq!(virtual_page.page.name, "Tine-guide/Start/Where things are");
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-where-things-are-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Start/Where things are"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("where-things-are was copied");
        assert!(copied_markdown.contains("[[tine-guide/Workflows/Keep context visible]]"));
        assert!(copied_markdown.contains("[[tine-guide/Features/Tips & shortcuts]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_context_visible_workflow_is_registered_linked_and_copyable() {
        let title = "Workflows/Keep context visible";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("keep-context-visible workflow is registered");
        assert!(page.markdown.contains("- # Keep context visible"));
        assert!(page.markdown.contains("**Shift-click**"));
        assert!(page.markdown.contains("**Ctrl+Shift+T**"));
        assert!(page.markdown.contains("+ New workspace"));
        assert!(page.markdown.contains("PDF readers use the same tabs and panes"));
        assert!(page.markdown.contains("**Notes** opens the PDF's notes page"));
        assert!(page.markdown.contains("What you should see"));
        assert!(page.markdown.contains("[[Start/Where things are]]"));
        assert!(page.markdown.contains("[[Features/Tips & shortcuts]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Workflows/Keep context visible]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("keep-context-visible workflow is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Workflows/Keep context visible"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-keep-context-visible-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Workflows/Keep context visible"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("keep-context-visible workflow was copied");
        assert!(copied_markdown.contains("[[tine-guide/Start/Where things are]]"));
        assert!(copied_markdown.contains("[[tine-guide/Workflows/Find and revisit]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extend_tine_workflow_is_registered_linked_and_copyable() {
        let title = "Workflows/Extend Tine";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("extend-tine workflow is registered");
        assert!(page.markdown.contains("- # Extend Tine"));
        assert!(page.markdown.contains("Install a local package"));
        assert!(page.markdown.contains("**installed disabled**"));
        assert!(page.markdown.contains("Unavailable on"));
        assert!(page.markdown.contains("graph.write.block"));
        assert!(page.markdown.contains("What you should see"));
        assert!(page.markdown.contains("Tine-owned presentation styles"));
        assert!(page.markdown.contains("Style and colors are independent"));
        assert!(page.markdown.contains("notnote's editorial style"));
        assert!(page
            .markdown
            .contains("The theme receives neither those tasks"));
        assert!(page.markdown.contains("[[Features/Plugins]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index.markdown.contains("[[Workflows/Extend Tine]]"));

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("extend-tine workflow is available in the read-only Guide");
        assert_eq!(virtual_page.page.name, "Tine-guide/Workflows/Extend Tine");
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-extend-tine-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Workflows/Extend Tine"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("extend-tine workflow was copied");
        assert!(copied_markdown.contains("[[tine-guide/Features/Plugins]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn platforms_and_mobile_reference_is_registered_linked_and_copyable() {
        let title = "Reference/Platforms and mobile";
        let page = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == title)
            .expect("platforms-and-mobile reference is registered");
        assert!(page.markdown.contains("- # Platforms and mobile"));
        // The page's core contract: the two questions stay distinct.
        assert!(page.markdown.contains("Question 1"));
        assert!(page.markdown.contains("Question 2"));
        assert!(page.markdown.contains("640 px"));
        assert!(page.markdown.contains("All files access"));
        assert!(page.markdown.contains("PDFs use that same one-pane history"));
        assert!(page.markdown.contains("Hardware Back to return first to the PDF"));
        assert!(page.markdown.contains("experimental 32-bit Windows"));
        assert!(page.markdown.contains("no public iOS app"));
        assert!(page.markdown.contains("[[Workflows/Keep context visible]]"));
        assert!(page.markdown.contains("[[Workflows/Extend Tine]]"));

        let index = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Tine Guide")
            .expect("guide index is registered");
        assert!(index
            .markdown
            .contains("[[Reference/Platforms and mobile]]"));
        // Both existing pages that deferred their platform detail to J10 link it.
        for referrer in ["Start/Where things are", "Workflows/Extend Tine"] {
            let template = GUIDE_TEMPLATES
                .iter()
                .find(|template| template.title == referrer)
                .expect("referrer template is registered");
            assert!(
                template
                    .markdown
                    .contains("[[Reference/Platforms and mobile]]"),
                "{referrer} links the platform reference"
            );
        }

        let virtual_page = bundled_guide_pages()
            .unwrap()
            .into_iter()
            .find(|page| page.title == title)
            .expect("platforms-and-mobile reference is available in the read-only Guide");
        assert_eq!(
            virtual_page.page.name,
            "Tine-guide/Reference/Platforms and mobile"
        );
        assert!(virtual_page.page.read_only);

        let dir = scratch("tine-guide-platforms-and-mobile-copy");
        let graph = Graph::open(&dir);
        let copied = copy_guide_into_graph(&graph, title).unwrap();
        assert!(copied
            .created_pages
            .iter()
            .any(|name| name == "tine-guide/Reference/Platforms and mobile"));
        let copied_markdown = std::fs::read_to_string(graph.path_for(&copied.name, PageKind::Page))
            .expect("platforms-and-mobile reference was copied");
        assert!(copied_markdown.contains("[[tine-guide/Workflows/Keep context visible]]"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tips_document_stable_and_configurable_page_widths() {
        let tips = GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Features/Tips & shortcuts")
            .expect("tips page is registered");
        assert!(tips.markdown.contains("Page width — t w"));
        assert!(tips
            .markdown
            .contains("keeps the same width while you edit"));
        assert!(tips.markdown.contains("Appearance** → **Advanced"));
        assert!(tips.markdown.contains("custom maximum"));
    }

    #[test]
    fn guide_link_validator_rejects_accidental_targets_and_ignores_inline_code() {
        let templates = [GuideTemplate {
            title: "Test source",
            markdown: "- [[Martin]] #demo [[Accidental target]] `[[literal brackets]]` ((00000000-0000-4000-8000-00000000feed))",
        }];

        assert_eq!(
            guide_template_link_errors(&templates),
            vec!["Test source: unregistered Guide target Accidental target"]
        );
    }

    #[test]
    fn guide_copy_rewrites_interguide_page_refs_only() {
        let copied = [
            "Tine Guide",
            "Features/Sheets",
            "Features/Formulas",
            "Features/Quick capture",
            "Features/PDF annotation",
            "Features/Plugins",
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
    fn copied_guide_pages_are_owned_for_native_watcher_echoes() {
        let dir = scratch("tine-guide-watcher-receipts");
        let graph = Graph::open(&dir);

        let copied = copy_guide_into_graph(&graph, "Tine Guide").unwrap();

        assert!(!copied.created_pages.is_empty());
        for name in copied.created_pages {
            let entry = graph
                .find_entry(&name, PageKind::Page)
                .unwrap_or_else(|| panic!("missing copied Guide page {name}"));
            assert!(
                graph.exact_graph_text_event_matches_tine_state(&entry.path),
                "the native watcher must recognize Tine's own Guide publication for {name}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
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

        for guide in bundled_guide_pages().unwrap() {
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

        let index =
            std::fs::read_to_string(graph.path_for("tine-guide/Tine Guide", PageKind::Page))
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

    #[cfg(unix)]
    #[test]
    fn copy_guide_rejects_pages_directory_symlink_swap() {
        use std::os::unix::fs::symlink;

        let dir = scratch("tine-guide-pages-swap");
        let outside = scratch("tine-guide-pages-outside");
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        let graph = Graph::open(&dir);
        std::fs::remove_dir(dir.join("pages")).unwrap();
        symlink(&outside, dir.join("pages")).unwrap();

        assert!(copy_guide_into_graph(&graph, "Tine Guide").is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        let _ = std::fs::remove_file(dir.join("pages"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn copy_guide_rejects_assets_directory_symlink_swap() {
        use std::os::unix::fs::symlink;

        let dir = scratch("tine-guide-assets-swap");
        let outside = scratch("tine-guide-assets-outside");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let graph = Graph::open(&dir);
        std::fs::remove_dir(dir.join("assets")).unwrap();
        symlink(&outside, dir.join("assets")).unwrap();

        assert!(copy_guide_into_graph(&graph, "Tine Guide").is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        let _ = std::fs::remove_file(dir.join("assets"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
