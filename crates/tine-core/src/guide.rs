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
        title: "Features/Queries",
        markdown: include_str!("templates/queries.md"),
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

#[cfg(test)]
mod journal_guide_tests {
    #[test]
    fn journal_controls_are_documented_in_the_bundled_guide() {
        let tips = include_str!("templates/tips.md");
        for control in [
            "/That day",
            "g n",
            "g p",
            "**g h** opens the graph's home page",
            ":default-home {:page",
            "default journal template",
            "Carry unfinished tasks",
            ":hidden [\"archive/private\"]",
        ] {
            assert!(
                tips.contains(control),
                "missing journal Guide control: {control}"
            );
        }
    }
}

#[cfg(test)]
mod search_guide_tests {
    #[test]
    fn search_fold_and_graph_opt_out_are_documented() {
        let tips = include_str!("templates/tips.md");
        for text in [
            "`cafe` finds `café`",
            "`lodz` finds `Łódź`",
            "`か` and `が` differ",
            ":feature/enable-search-remove-accents? false",
            "Markdown keep their original spelling",
        ] {
            assert!(
                tips.contains(text),
                "missing search Guide explanation: {text}"
            );
        }
        let queries = include_str!("templates/queries.md");
        assert!(queries.contains("Both respect `:feature/enable-search-remove-accents? false`"));
    }
}

#[cfg(test)]
mod rename_guide_tests {
    #[test]
    fn pdf_export_save_refusal_is_documented_in_the_bundled_guide() {
        let tips = include_str!("templates/tips.md");
        assert!(tips.contains("**Export to PDF…** saves pending page edits"));
        assert!(tips.contains("stops the export and shows an error"));
    }

    #[test]
    fn rename_merge_and_journal_rename_proposals_are_documented_in_the_bundled_guide() {
        let tips = include_str!("templates/tips.md");
        for control in [
            "offers to **merge** them",
            "the aliases join",
            // GH #535: when an unsaved page still stops a rename, or a refusal
            // reads as arbitrary.
            "stops the rename only if the rename would change that page",
            "Other pages keep their unsaved edits through the rename",
            "or in Org by `#+TITLE:`, gets the new name",
            "Opening a graph never renames journal files",
            "**Rename to date names**",
        ] {
            assert!(
                tips.contains(control),
                "missing family-16 Guide control: {control}"
            );
        }
    }

    #[test]
    fn property_editor_is_documented_in_the_bundled_guide() {
        // GH #164 (family 4): both doors, the add-row, non-ASCII keys, and the
        // read-only refusal; nothing else tells a reader the form exists.
        let tips = include_str!("templates/tips.md");
        for control in [
            "**Page properties…**",
            "right-click a block for **Properties…**",
            "**Add a property**",
            "does not have to be plain ASCII",
            "offers no property editing at all",
        ] {
            assert!(
                tips.contains(control),
                "missing family-4 Guide control: {control}"
            );
        }
    }
}

#[cfg(test)]
mod query_guide_tests {
    /// og 14 Q2: the Guide describes queries as this build runs them — the
    /// OG simple language, the advanced subset with its ran/ignored note, OG's
    /// current-page binding, host-block view properties, visible diagnostics,
    /// the refusal bound, and what is NOT offered here yet.
    #[test]
    fn queries_are_documented_as_this_build_runs_them() {
        let queries = include_str!("templates/queries.md");
        for control in [
            "{{query (task TODO DOING)}}",
            "(page-property type book)",
            "(between scheduled today +7d)",
            "at most 10,000 years",
            "tine.sample:: 10",
            "**ran**",
            "**ignored**",
            ":inputs [:current-page]",
            "then today's journal. It is not the page the query block sits on",
            "Tine didn't understand part of this query, so it returned no results",
            "more than 20,000 rows",
            "{{tine-query …}}",
        ] {
            assert!(
                queries.contains(control),
                "missing query Guide control: {control}"
            );
        }
        assert!(
            super::GUIDE_TEMPLATES
                .iter()
                .any(|template| template.title == "Features/Queries"),
            "the queries page is in the Guide manifest"
        );
        assert!(include_str!("templates/guide.md").contains("[[Features/Queries]]"));
    }

    /// og 14 Q4a: the query block's sentence, sheet, text pane, crossing
    /// notice and why-empty are documented, and "Not in this build yet" no
    /// longer lists the text language or the explanation this build ships.
    #[test]
    fn query_sheet_is_documented_as_shipped() {
        let queries = include_str!("templates/queries.md");
        for control in [
            "Type `/query` and choose **Query**",
            "**Find blocks ▾ where …**",
            "**+ Add condition**",
            "**Group selected ▾**",
            "turns it off without removing it",
            "**Save query text**",
            "**Show me**",
            "**Undo that change**",
            "**why empty?**",
        ] {
            assert!(
                queries.contains(control),
                "missing query Guide control: {control}"
            );
        }
        assert!(!queries.contains("## Not in this build yet"));
    }

    #[test]
    fn query_display_and_type_declarations_are_documented() {
        let queries = include_str!("templates/queries.md");
        for control in [
            "press **Display**",
            "Choosing **Page** in the column picker",
            "`tine.fields::` remains the table's schema",
            "number of blocks or pages",
            "**declare type…**",
            "**list of**",
            "mismatches against a declaration",
        ] {
            assert!(
                queries.contains(control),
                "missing Q4b Guide control: {control}"
            );
        }
    }
}
