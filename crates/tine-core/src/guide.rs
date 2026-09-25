//! Bundled Guide content: the canonical Guide pages, the demo-graph config and
//! the embedded assets. Pure data; the store (`tine_store::onboarding`) writes it.

use crate::model::PageDto;
use crate::projection::markdown_page_dto;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, serde::Serialize)]
pub struct GuidePage {
    pub title: String,
    pub markdown: String,
    pub page: PageDto,
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

/// `logseq/config.edn` for the demo graph (triple-lowbar namespace filenames,
/// the welcome page pinned as a favorite).
pub const CONFIG_EDN: &str = include_str!("templates/config.edn");

/// The capture-window screenshot embedded by the quick-capture page.
pub const QUICK_CAPTURE_PNG: &[u8] = include_bytes!("templates/assets/quick-capture.png");

/// In-memory namespace for bundled, read-only guide pages. These pages are
/// rendered live in the running app but are not graph files.
pub const GUIDE_DISPLAY_PREFIX: &str = "Tine-guide/";

/// Real graph namespace used only by the explicit guide-copy action.
pub const GUIDE_COPY_PREFIX: &str = "tine-guide/";

/// One canonical manifest feeds all three Guide surfaces: the onboarding graph,
/// the in-app read-only Guide, and the generated website demo. Keeping the list
/// in one place prevents a page from silently disappearing from one surface.
pub struct GuideTemplate {
    pub title: &'static str,
    pub markdown: &'static str,
}

pub const GUIDE_TEMPLATES: &[GuideTemplate] = &[
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
];

pub struct GuideAsset {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

pub const GUIDE_ASSETS: &[GuideAsset] = &[GuideAsset {
    name: "quick-capture.png",
    bytes: QUICK_CAPTURE_PNG,
}];

pub fn guide_page_name(title: &str) -> String {
    format!("{GUIDE_DISPLAY_PREFIX}{title}")
}

pub fn guide_copy_page_name(title: &str) -> String {
    format!("{GUIDE_COPY_PREFIX}{title}")
}

pub fn guide_link_renames() -> HashMap<String, String> {
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

pub fn rewrite_bundled_guide_links(markdown: &str, renames: &HashMap<String, String>) -> String {
    crate::refs::rename_refs_multi(markdown, renames, false)
}

pub fn collect_guide_asset_refs(markdown: &str, into: &mut HashSet<String>) {
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
