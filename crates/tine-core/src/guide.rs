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
        title: "Reference/Command line",
        markdown: include_str!("templates/command-line.md"),
    },
    GuideTemplate {
        title: "Reference/Platforms and mobile",
        markdown: include_str!("templates/platforms-and-mobile.md"),
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
    // Markdown has no `file:` page links, so the filename format is not consulted.
    crate::refs::rename_refs_multi(
        markdown,
        renames,
        false,
        crate::config::FileNameFormat::TripleLowbar,
    )
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
            "Renaming the home page (or a namespace it lives in) keeps it the home page",
            "Drag an item by its header to reorder the list.",
            "default journal template",
            "Carry unfinished tasks",
            ":hidden [\"archive/private\"]",
        ] {
            assert!(
                tips.contains(control),
                "missing journal Guide control: {control}"
            );
        }
        // og 22b: settings writes never overwrite a half-delivered config.edn.
        let files = include_str!("templates/files-external-edits-backups.md");
        assert!(files.contains("`logseq/config.edn` is live too"));
        assert!(files.contains("refused rather than written if `config.edn` is half-written"));
        // og-B (ADR 0062): snapshots cover text outside pages/ and journals/.
        assert!(files.contains("pages kept in other folders or at the graph root"));
        assert!(files.contains("moved into the restore's recovery folder"));
    }
}

#[cfg(test)]
mod pdf_workspace_guide_tests {
    #[test]
    fn bundled_pdf_guide_describes_tab_and_mobile_reader() {
        let pdf = include_str!("templates/pdf.md");
        for detail in [
            "tab in a companion pane",
            "same one-pane history",
            "page and zoom are restored",
            "visible page regions",
            "long-press a highlight",
            "use **Back**",
        ] {
            assert!(
                pdf.contains(detail),
                "missing PDF workspace Guide detail: {detail}"
            );
        }
        assert!(super::GUIDE_TEMPLATES
            .iter()
            .any(|page| page.title == "Features/PDF annotation"));
        assert!(include_str!("templates/guide.md").contains("[[Features/PDF annotation]]"));
    }
}

#[cfg(test)]
mod parity_guide_tests {
    #[test]
    fn tips_cover_home_and_settings_parity() {
        let tips = include_str!("templates/tips.md");
        for phrase in [
            "Settings → Graph",
            "Settings → Shortcuts",
            "Settings → Appearance",
            "Toggle maximize active pane",
            "Open in new tab",
            "Settings → Help & diagnostics",
            "**Copy report**, or on desktop **Save report…**; nothing is uploaded",
            "If Tine did not close cleanly last time, it says so",
            ":ref/linked-references-collapsed-threshold",
            "**Ctrl/Cmd+Shift+C** copies an embed",
            "hover a result to copy it",
        ] {
            assert!(tips.contains(phrase), "Tips missing {phrase}");
        }
    }

    #[test]
    fn welcome_names_the_alias_completion_label() {
        // GH #558: an alias row in `[[` completion names the page it belongs to.
        let welcome = include_str!("templates/welcome.md");
        assert!(welcome.contains("labelled **alias of** its page"));
        // GH #259: a marker-label click toggles the open pair only.
        assert!(welcome.contains("flips between the pair and never clears `DONE`"));
    }
}

#[cfg(test)]
mod search_guide_tests {
    #[test]
    fn friendly_search_sections_scope_and_save_are_documented() {
        let tips = include_str!("templates/tips.md");
        for text in [
            "**Pages** and **Blocks**",
            "Pages match",
            "Names or content",
            "Save page",
            "tine.page-match-scope:: content",
            "Each section has its own **Display** control",
        ] {
            assert!(
                tips.contains(text),
                "missing friendly search Guide detail: {text}"
            );
        }
        let queries = include_str!("templates/queries.md");
        assert!(queries.contains("An alias result opens its owner page"));
        assert!(queries.contains("authored page properties as columns"));
        assert!(
            queries.contains("Sample keeps the first N of that order, separately for each section")
        );
        assert!(queries.contains("Pages board groups adjacent results"));
    }
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
    #[test]
    fn reference_panel_controls_are_documented() {
        let tips = include_str!("templates/tips.md");
        for text in [
            "select visible results",
            "Org source stays Org",
            "include chips match any",
            "collapse or expand all source groups",
        ] {
            assert!(
                tips.contains(text),
                "missing reference panel Guide detail: {text}"
            );
        }
    }
    #[test]
    fn table_resize_and_export_are_documented() {
        let sheets = include_str!("templates/sheets.md");
        for text in [
            "right edge to resize",
            "Double-click the handle",
            "tine.table-widths::",
            "+ Add row",
            "produce HTML",
        ] {
            assert!(sheets.contains(text), "missing sheet Guide detail: {text}");
        }
    }
}

#[cfg(test)]
mod rename_guide_tests {
    #[test]
    fn editor_gestures_are_documented_in_the_bundled_guide() {
        // Family 19: the code-block editor, nested drop and property autocomplete.
        let tips = include_str!("templates/tips.md");
        assert!(tips.contains("**Code blocks**: type ``` "));
        assert!(tips.contains("only the code itself is in the text box"));
        assert!(
            tips.contains("**Drag a bullet onto another bullet and move a little to the right**")
        );
        assert!(tips.contains("that bullet's last child"));
        assert!(
            tips.contains("Typing `::` at the start of a line inside a bullet starts a property")
        );
    }

    #[test]
    fn modified_link_clicks_are_documented_in_the_bundled_guide() {
        // GH #283/#438: one modified-click contract for internal links.
        let tips = include_str!("templates/tips.md");
        assert!(tips.contains("all take the same modified clicks"));
        assert!(tips.contains("**Ctrl/Cmd-click** or **middle-click** opens a background tab"));
        assert!(tips.contains("**Alt-click** opens the other pane"));
        assert!(tips.contains("an outline bullet's dot"));
        assert!(tips.contains("a pane's only tab still has a close button"));
        let refs = include_str!("templates/pages-links-references-search.md");
        assert!(refs.contains("follows its source's fold state live"));
    }

    #[test]
    fn ctrl_y_redo_alias_is_documented_in_the_bundled_guide() {
        // GH #491: both redo chords, the platforms that get the second one, and
        // that remapping Redo replaces both.
        let tips = include_str!("templates/tips.md");
        assert!(tips.contains("redo is Ctrl/Cmd+Shift+Z"));
        assert!(tips.contains("On Windows and Linux **Ctrl+Y** also redoes"));
        assert!(tips.contains("remapping Redo replaces both"));
    }

    #[test]
    fn pdf_export_save_refusal_is_documented_in_the_bundled_guide() {
        let tips = include_str!("templates/tips.md");
        assert!(tips.contains("**Export to PDF…** saves pending page edits"));
        assert!(tips.contains("stops the export and shows an error"));
    }

    #[test]
    fn concord_conflict_queue_and_resolver_are_documented_in_the_bundled_guide() {
        // og family 8c: the badge, the Conflicts page, the in-page resolver and
        // the marker save refusal are user-visible and named in the Guide.
        let tips = include_str!("templates/tips.md");
        for control in [
            "**N conflicts** badge",
            "**Conflicts** page",
            "**Discard copy**",
            "**Apply resolution**",
            "Tine refuses to save a page that still contains merge markers",
            "the file as it was first goes to the graph trash",
        ] {
            assert!(
                tips.contains(control),
                "missing family-8 Guide control: {control}"
            );
        }
    }

    #[test]
    fn duplicate_journal_days_resolve_from_the_conflict_queue_in_the_guide() {
        // Master 9dc54e4a7: a duplicate journal day is a queue object resolved
        // on the day itself; Settings keeps the per-file list as a fallback.
        let recovery = include_str!("templates/troubleshooting-recovery.md");
        for control in [
            "open the day from the **Duplicate journal days** group",
            "lists each file with **Open**, **Rename**, and **Trash** actions",
            "A Markdown file and an Org file for one day cannot be folded together",
        ] {
            assert!(
                recovery.contains(control),
                "missing duplicate-day Guide control: {control}"
            );
        }
        let files = include_str!("templates/files-external-edits-backups.md");
        assert!(files.contains("the day joins the **N conflicts** queue"));
    }

    #[test]
    fn a_capture_finished_after_a_graph_switch_is_documented() {
        // og H1b: kept in the graph it was started in, never the new one.
        let mobile = include_str!("templates/platforms-and-mobile.md");
        assert!(mobile.contains("saved into the graph it was started in and not inserted"));
    }

    #[test]
    fn in_page_find_guide_says_the_match_itself_is_revealed() {
        // GH #253 (master 46a5290a2): Find scrolls to the occurrence, not its block.
        let search = include_str!("templates/pages-links-references-search.md");
        assert!(search.contains("the match itself is scrolled into view"));
    }

    #[test]
    fn unlinked_reference_guide_says_the_highlight_is_clickable() {
        // GH #200 (master 552d988c9): nothing in the UI announces that a
        // highlighted mention is a control, so the Guide has to.
        let search = include_str!("templates/pages-links-references-search.md");
        assert!(search.contains("the highlight is clickable"));
    }

    #[test]
    fn file_link_guide_says_a_file_link_opens_on_desktop() {
        // GH #444 (master c817fb150): a `file:` link opens in the OS default
        // application; nothing else tells the reader which link forms are live.
        let search = include_str!("templates/pages-links-references-search.md");
        assert!(search.contains("A `file:` link"));
        assert!(!search.contains("`file:` links are not opened"));
    }

    #[test]
    fn empty_html_export_names_the_public_page_rule_in_the_guide() {
        // GH #560 (master 350efef1f): a zero-page export is explained, not silent.
        let files = include_str!("templates/files-external-edits-backups.md");
        assert!(files.contains("A graph with no `public:: true` page exports 0 pages"));
    }

    #[test]
    fn parser_comparison_guide_says_intentional_differences_are_not_bugs() {
        // Master c0c2ff11b: a known intentional lsdoc difference is suppressed,
        // not offered as a reportable parser bug.
        let recovery = include_str!("templates/troubleshooting-recovery.md");
        assert!(recovery.contains("Known intentional parser differences are not offered as bugs"));
    }

    #[test]
    fn the_conflict_review_stays_reachable_while_scrolling_in_the_guide() {
        // Master 61ea6600c: a pinned bar unrolls the review in place.
        let files = include_str!("templates/files-external-edits-backups.md");
        assert!(files.contains("A pinned notice keeps the review reachable while you scroll"));
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
        assert!(queries.contains("## Publish a query"));
        assert!(queries.contains("whole owner page"));
        assert!(queries.contains("Include all pages"));
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

    /// GH #542 (master c1b14a859): the Guide's advanced-query example is one
    /// Tine runs whole, and the page states the disclosed-superset rule.
    #[test]
    fn gh542_guide_advanced_query_example_runs_whole() {
        let workflow = super::GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Workflows/Find and revisit")
            .expect("the find-and-revisit workflow is registered");
        let start = workflow
            .markdown
            .find("`[:find ")
            .expect("the Guide shows an advanced query");
        let example = &workflow.markdown[start + 1..];
        let example = &example[..example.find('`').expect("closed code span")];
        let today = crate::date::JournalDate::today();
        let (query, _) = crate::query::parse_query_source(example, today);
        let result = crate::query::resolve_for_execution(
            &query,
            &crate::query::ir::ExecutionContext::none(),
            today,
        );
        assert!(result.report().supported, "{example}");
        assert!(
            result.report().ignored.is_empty(),
            "{:?}",
            result.report().ignored
        );
        assert!(workflow.markdown.contains("never fewer"));
        assert!(workflow.markdown.contains("is left out whole"));
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

#[cfg(test)]
mod theme_presentation_guide_tests {
    /// Theme API 0.2 (master 1488588b8/c8b18f327, ADR 0059) is documented where
    /// users meet themes: bounded presentation, and a Today summary Tine renders.
    #[test]
    fn declarative_theme_presentation_is_documented_in_the_bundled_guide() {
        let plugins = include_str!("templates/plugins.md");
        assert!(plugins.contains("bounded Tine-owned presentation styles"));
        assert!(plugins.contains("are chosen independently"));
        assert!(plugins.contains("The theme receives neither those tasks"));
        assert!(!plugins.contains("Token themes live under"));
    }
}

#[cfg(test)]
mod og_20d_guide_tests {
    use super::*;

    fn page(title: &str) -> &'static str {
        GUIDE_TEMPLATES
            .iter()
            .find(|t| t.title == title)
            .unwrap_or_else(|| panic!("Guide page {title:?} is not bundled"))
            .markdown
    }

    #[test]
    fn where_things_are_documents_page_width_settings_size_and_region_failures() {
        let map = page("Start/Where things are");
        for detail in [
            "**t w**",
            "**Standard page width**",
            "**Wide page width**",
            "fill the window",
            "**Retry**",
        ] {
            assert!(
                map.contains(detail),
                "missing where-things-are detail: {detail}"
            );
        }
    }

    #[test]
    fn troubleshooting_documents_the_launch_failure_card_and_failed_regions() {
        let recovery = page("Reference/Troubleshooting and recovery");
        for detail in [
            "Tine could not open your last graph",
            "**Try again**",
            "**Copy details**",
            "**Retry**",
            "Changed on disk",
            "A panel says it could not load something",
            "Couldn’t load references to this block",
            "Couldn’t load search results",
            "A red error message appears",
            "stays on screen until you close it",
            "Recent error messages",
            "kept in memory only",
        ] {
            assert!(
                recovery.contains(detail),
                "missing recovery detail: {detail}"
            );
        }
        // og has no search index: no page may describe rebuilding or waiting on one.
        for page in GUIDE_TEMPLATES {
            for forbidden in ["Rebuild the index", "Indexing…", "Managed Storage"] {
                assert!(
                    !page.markdown.contains(forbidden),
                    "{} documents a feature og does not have: {forbidden}",
                    page.title
                );
            }
        }
    }

    /// og-D D4 (master e7af4db9c): the command-line reference names every shipped
    /// command and og's create-only export contract, and the guide index links it.
    #[test]
    fn command_line_reference_covers_the_shipped_surface_and_safety_defaults() {
        let cli = page("Reference/Command line");
        for promised in [
            "tine --help",
            "tine --version",
            "tine open GRAPH",
            "tine capture",
            "tine export static GRAPH",
            "tine export live GRAPH",
            "tine doctor GRAPH",
            "--output PARENT",
            "absolute path",
            "refuses to replace",
            "man tine",
        ] {
            assert!(cli.contains(promised), "Guide omitted {promised}");
        }
        for stale in ["--replace", "graph-relative", "--home"] {
            assert!(
                !cli.contains(stale),
                "og export has no {stale}; master-only wording leaked in"
            );
        }
        assert!(page("Tine Guide").contains("[[Reference/Command line]]"));
    }

    /// og 21a: a live-draft conflict is merged at the page and survives a
    /// restart; a rename leaves a mid-merge referrer alone and says so.
    #[test]
    fn guide_describes_live_conflict_review_and_marker_referrers() {
        let recovery = include_str!("templates/troubleshooting-recovery.md");
        assert!(recovery.contains("compares **Your unsaved edits** with **The file on disk now**"));
        assert!(recovery.contains("If the file changes again before you apply, nothing is written"));
        assert!(recovery.contains("the same comparison appears on that page after the next start"));
        let files = include_str!("templates/files-external-edits-backups.md");
        assert!(files.contains("never rewritten by a rename"));
    }

    /// og 22a: external-change freshness is user-visible — always ask, the
    /// focus pass, bulk grouping, ignored noise folders, the polling fallback
    /// and the Review-only banner for a live draft.
    #[test]
    fn guide_describes_external_change_freshness() {
        let files = include_str!("templates/files-external-edits-backups.md");
        for control in [
            "**Always ask before applying an external change**",
            "**Reload from disk** / **Keep mine**",
            "Returning to Tine asks the watcher for a fresh pass",
            "grouped into one external revision",
            "`.git/`",
            "checks for external changes every 3 seconds instead",
            "shows a banner with **Review**",
            "under **Unsaved drafts**",
        ] {
            assert!(
                files.contains(control),
                "missing 22a Guide control: {control}"
            );
        }
        let recovery = include_str!("templates/troubleshooting-recovery.md");
        assert!(recovery.contains("offers only **Review**"));
    }

    /// og 22c: sheets in an export are computed by the app; the command-line
    /// export keeps the plain outline (a named divergence) and a sheet that
    /// cannot be computed degrades to the outline with a note.
    #[test]
    fn guide_describes_sheets_in_export_and_the_cli_divergence() {
        let files = include_str!("templates/files-external-edits-backups.md");
        assert!(files.contains(
            "Tine computes them with the app's own sheet code before it writes the export"
        ));
        assert!(files.contains(
            "has no app to compute them, so it writes a sheet as the plain outline of its rows"
        ));
        assert!(files.contains("A sheet that cannot be computed"));
        assert!(files.contains("the export still succeeds"));
        // A query-backed table or board exports as a sheet; the old "result list for now" is gone.
        assert!(
            files.contains("A table or board that shows a query's results is written the same way")
        );
        assert!(!files.contains("result list for now"));
        // A row on an unpublished page is left out, not the whole sheet.
        assert!(files
            .contains("a result on a page the export does not publish is left out of the table"));
        assert!(!files.contains("if any result sits on a page the export does not publish"));
        // og E: a query board can be ungrouped.
        assert!(include_str!("templates/sheets.md").contains("also offers **No grouping**"));
    }

    /// og-D: the cross-device graph verifier is user-visible: create, copy or
    /// save, compare, name the differing paths, and refuse to confirm a match
    /// from an incomplete report.
    #[test]
    fn troubleshooting_documents_cross_device_graph_verification() {
        let recovery = page("Reference/Troubleshooting and recovery");
        for detail in [
            "**Create graph verification report**",
            "**Copy graph report**",
            "**Save graph report…**",
            "**Compare reports**",
            "**Only on this device**",
            "**Different bytes**",
            "never file contents",
            "no match is confirmed",
        ] {
            assert!(
                recovery.contains(detail),
                "missing graph verification detail: {detail}"
            );
        }
    }

    #[test]
    fn guide_says_public_false_pages_are_never_exported() {
        let files = include_str!("templates/files-external-edits-backups.md");
        assert!(files.contains("A page marked `public:: false` is never exported"));
        assert!(files.contains("`:publishing/all-pages-public? true`"));
        let queries = include_str!("templates/queries.md");
        assert!(queries.contains("it still leaves out pages marked `public:: false`"));
    }
}
