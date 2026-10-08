//! Bundled Guide content: the canonical Guide pages, the demo-graph config and
//! the embedded assets. Pure data; the store (`tine_store::onboarding`) writes it.

use crate::model::PageDto;
use crate::projection::markdown_page_dto;
use std::collections::HashMap;

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
        title: "Workflows/Customize Tine's look",
        markdown: include_str!("templates/customize-look.md"),
    },
    GuideTemplate {
        title: "Reference/Tine query model",
        markdown: include_str!("templates/query-model.md"),
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

/// Every embedded asset the Guide templates reference: top-level names, sorted,
/// no duplicates. `asset_manifest_tests` checks this against the templates, so
/// the copy path writes the list directly without scanning them at runtime.
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

#[cfg(test)]
mod journal_guide_tests {
    #[test]
    fn reference_change_notices_offer_undo() {
        let reference = include_str!("templates/pages-links-references-search.md");
        assert!(
            reference.contains("Deleting referenced blocks or merging them with Backspace/Delete")
        );
        assert!(reference.contains("with **Undo** to restore the blocks and their ids"));
    }

    #[test]
    fn search_workspace_and_find_explain_visible_views_and_destinations() {
        let reference = include_str!("templates/pages-links-references-search.md");
        let tips = include_str!("templates/tips.md");
        assert!(reference.contains("split panes and expanded right-sidebar items"));
        assert!(reference.contains("pane order then sidebar stack order"));
        assert!(reference.contains("Clicking a result opens a new tab and keeps Search unchanged"));
        assert!(reference.contains("Page results offer the same right-click menu"));
        assert!(tips.contains("**Enter** / **Shift+Enter** move forward / backward"));
        assert!(tips.contains("In a Search tab, clicking a result opens a new tab"));
    }

    #[test]
    fn workspace_windows_are_documented() {
        let context = include_str!("templates/keep-context-visible.md");
        for detail in [
            "## Open a page in its own window",
            "**Open in new window**",
            "**Open current page in new window**",
            "It has no left or right sidebar.",
            "text you just typed is kept",
            "the ones that were open come back",
            "Switching to another graph closes them.",
        ] {
            assert!(
                context.contains(detail),
                "missing workspace-window Guide detail: {detail}"
            );
        }
    }

    #[test]
    fn selection_menu_and_keyboard_calendar_are_documented() {
        let tips = include_str!("templates/tips.md");
        for action in [
            "Copy blocks",
            "Cut blocks",
            "Copy block refs",
            "Copy block embeds",
            "Delete blocks",
        ] {
            assert!(tips.contains(action), "missing selection action: {action}");
        }
        assert!(tips.contains("one Undo step"));
        assert!(tips.contains("complete subtrees for pasting"));
        let calendar = include_str!("templates/journals-tasks-scheduling.md");
        for behavior in [
            "keyboard focus",
            "**Left/Right**",
            "**Up/Down**",
            "**Enter**",
            "**Escape**",
            "same caret",
        ] {
            assert!(
                calendar.contains(behavior),
                "missing calendar behavior: {behavior}"
            );
        }
    }

    #[test]
    fn sidebar_and_direct_search_actions_are_documented() {
        let tips = include_str!("templates/tips.md");
        for control in [
            "sidebar/grow-width",
            "sidebar/shrink-width",
            "right-sidebar/grow-width",
            "right-sidebar/shrink-width",
            "go/search-tab",
            "All four start unbound",
            "Titles that fit show no tooltip",
        ] {
            assert!(
                tips.contains(control),
                "missing sidebar/search Guide control: {control}"
            );
        }
    }

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
        // og-J2 (master d017d1afc): assets refresh in place after an outside change.
        assert!(files.contains("Files in `assets/` are watched too"));
        assert!(files.contains("An open PDF, audio, or video is not swapped while you use it"));
    }
}

#[cfg(test)]
mod pdf_workspace_guide_tests {
    #[test]
    fn bundled_pdf_guide_describes_tab_and_mobile_reader() {
        let pdf = include_str!("templates/pdf.md");
        for detail in [
            "Opening and reading a PDF changes no graph files",
            "created when you first highlight or annotate",
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
        let research = include_str!("templates/research-document.md");
        assert!(research.contains("`file://` link"));
        assert!(research.contains("operating system's default PDF viewer"));
        assert!(
            research.contains("without copying the file into assets or creating annotation files")
        );
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
        for contract in [
            "Query text is case-sensitive and accent-sensitive",
            "including property lines and drawers",
            "Ctrl+K, search tabs and in-page Find still ignore case",
            "Commas split values only for",
            ":property/separated-by-commas",
            "query-table:: true",
            "a trailing `table`",
            "never adds `query-table`",
        ] {
            assert!(
                queries.contains(contract),
                "missing query Guide contract: {contract}"
            );
        }
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
        assert!(tips.contains("clicking a code region edits only that region"));
        assert!(tips.contains("edit the whole bullet as source; surrounding text stays intact"));
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
    fn desktop_guide_documents_the_system_tray_settings() {
        // GH #625: three off-by-default device switches, the menu, and the
        // no-tray-host fallback have to be findable without reading the code.
        let platforms = include_str!("templates/platforms-and-mobile.md");
        for outcome in [
            "**Show Tine in the system tray**",
            "**Minimize to tray**",
            "**Start minimized to tray**",
            "**Open Tine**, **Quick Capture** and **Quit**",
            "All three are off by default",
            "Closing the main window still quits Tine",
            "has no tray",
            "Starting Tine again shows the main window if the tray had hidden it",
            "If the tray disappears while Tine is running",
            "waits a few seconds for a late tray",
            "`libayatana-appindicator3`",
        ] {
            assert!(
                platforms.contains(outcome),
                "missing system-tray Guide outcome: {outcome}"
            );
        }
    }

    #[test]
    fn desktop_guide_documents_the_beta_flatpak_download() {
        // The bundle is a manual-update download: the install command, the way to find the app
        // id, side-by-side coexistence and the update path must be on the page.
        let platforms = include_str!("templates/platforms-and-mobile.md");
        for outcome in [
            "**Install as a Flatpak (Linux, Beta).**",
            "`Tine-Beta_<version>_x86_64.flatpak`",
            "`flatpak install --user Tine-Beta_<version>_x86_64.flatpak`",
            "`flatpak list` shows its application id for `flatpak run`",
            "installs next to a stable Tine Flatpak",
            "**Updates are manual**",
            "graph folders **inside your home folder only**",
            "the Flatpak bundle carries its own",
        ] {
            assert!(
                platforms.contains(outcome),
                "missing Flatpak Guide outcome: {outcome}"
            );
        }
    }

    #[test]
    fn android_guide_requires_a_local_graph_folder() {
        let mobile = include_str!("templates/platforms-and-mobile.md");
        for outcome in [
            "device's own storage",
            "Cloud document providers",
            "sync-to-local-folder",
        ] {
            assert!(
                mobile.contains(outcome),
                "missing Android Guide outcome: {outcome}"
            );
        }
    }

    #[test]
    fn ios_guide_says_where_graphs_live_and_that_plugins_stay_off() {
        // The iOS picker opens only the app's own On My iPhone / iCloud Drive
        // folders (src/graph.ts switchGraph) and Settings hides Plugins on iOS.
        let mobile = include_str!("templates/platforms-and-mobile.md");
        for outcome in [
            "**On My iPhone → TineOutline** or **iCloud Drive → TineOutline**",
            "refuses folders from other Files providers",
            "a remembered graph reopens after an app update",
            "**Plugins do not run on iOS yet.**",
        ] {
            assert!(
                mobile.contains(outcome),
                "missing iOS Guide outcome: {outcome}"
            );
        }
        assert!(!mobile.contains("there is no public iOS build yet"));
    }

    #[test]
    fn mobile_guide_names_the_touch_gestures_and_the_one_back_ladder() {
        // GH #501 / #492: a gesture nothing announces reads as missing, so the
        // mobile page has to say what each swipe does and which one is Back.
        let mobile = include_str!("templates/platforms-and-mobile.md");
        assert!(mobile.contains("## Touch gestures on a phone"));
        for outcome in [
            "Swipe a block **right** to indent it",
            "**left a short way** to outdent it",
            "**left a longer way** to select it and open its action menu",
            "The left-swipe outdent band is 40–139 px; actions start at 140 px",
            "**Swipe in from the left edge** to open the left drawer",
            "**Back is one ladder everywhere.**",
            "snapping back if you let go early",
            "swipe sideways to move between the page's images, and drag up or down to close",
            "A single tap shows or hides the buttons instead of closing",
            ":mobile {:gestures/disabled-in-block-with-tags [",
        ] {
            assert!(
                mobile.contains(outcome),
                "missing touch-gesture Guide outcome: {outcome}"
            );
        }
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
    fn query_export_guide_explains_closed_nested_result_reporting() {
        let files = include_str!("templates/files-external-edits-backups.md");
        for outcome in [
            "published-queries/<folder>/",
            "**Replace**",
            "**Create a separate export**",
            "its path is reported",
            "default **1 GiB**",
            "**Adjust limit in Settings…**",
            "Missing assets are reported",
            "move or share the whole export folder",
        ] {
            assert!(
                files.contains(outcome),
                "query-export onboarding must explain {outcome}"
            );
        }
        assert!(
            files.contains("Queries inside a query export show only results on the exported pages")
        );
        assert!(files.contains("they do not report how many results were left out on other pages"));
    }

    #[test]
    fn diagnostics_guide_explains_launch_timings_shape_statistics_and_rescan() {
        // GH #623: the report carries launch timings and graph-shape statistics
        // (numbers only), and Settings has a "Rescan graph" button.
        let recovery = include_str!("templates/troubleshooting-recovery.md");
        for outcome in [
            "`graphs` section",
            "a slow disk or antivirus scan shows up as read time rather than parse time",
            "numbers only: never a page name, any text, or a hash of either",
            "## Rescan the graph on demand",
            "choose **Rescan graph**",
            "reads and parses every file of the open graph again from disk",
            "quick check Tine does whenever you return to the window",
            "**Last rescan finished at**",
        ] {
            assert!(
                recovery.contains(outcome),
                "troubleshooting onboarding must explain {outcome}"
            );
        }
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
    /// OG simple language, the advanced subset with its whole-query refusal, OG's
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
            "the whole query is refused",
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

    /// GH #619 (og-query-ux): the Guide names every sheet capability the
    /// design list added, with the control's own wording.
    #[test]
    fn gh619_query_sheet_ux_is_documented() {
        let queries = include_str!("templates/queries.md");
        for control in [
            "onto another group to move that condition into it",
            "**Any status**",
            "**In a journal page**",
            "*Scheduled: next 7 days*",
            "nothing the builder wrote is labelled *advanced*",
            "follow every change you make, before you save",
            "Matches that share a parent show that parent's breadcrumb once",
            "Search and List page results show the page title",
            "**Pages and blocks**",
            "`tine.result-kinds:: pages-and-blocks`",
            "press **Edit as text**",
            "press the pencil beside a result to edit its properties",
            "**Grouped by**",
            "Press **Clear** to remove grouping",
            "instead of a table containing only `(none)`",
            "the sentence then reads *Pages and blocks where …*",
            "a section that a **Sample** cut short says *More pages match than are shown*",
        ] {
            assert!(
                queries.contains(control),
                "missing GH #619 query Guide control: {control}"
            );
        }
    }

    /// GH #651: the Guide says why a task nested under a matching task has no
    /// row of its own in the Table (OG's top-level-result rule, ADR 0042).
    #[test]
    fn gh651_guide_states_the_nested_match_rule() {
        let queries = include_str!("templates/queries.md");
        assert!(
            queries.contains(
                "A block that matches directly under another block that also matches is not a result of its own, as in Logseq"
            ),
            "queries.md must state the nested-match rule"
        );
        assert!(
            queries.contains("a match under a block that does not match is its own result"),
            "queries.md must say a match below a non-matching block is its own result"
        );
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
        // Audit #1: a part Tine cannot read refuses the whole query (the old
        // "never fewer / left out whole" partial-run rule is retired).
        assert!(workflow.markdown.contains("refuses the whole query"));
        assert!(!workflow.markdown.contains("is left out whole"));
    }

    /// OG parity audit #8/#8a/#9: what a simple query returns and how a legacy
    /// table is laid out are stated where users read them.
    #[test]
    fn query_result_shape_is_documented() {
        let queries = include_str!("templates/queries.md");
        for control in [
            "`(namespace Project)` — blocks on the pages directly under a namespace",
            "journal days newest first, then the other pages by name",
            "`query-properties:: [:block :page :status]`",
            "`query-sort-desc:: false`",
        ] {
            assert!(
                queries.contains(control),
                "missing query Guide control: {control}"
            );
        }
    }

    #[test]
    fn query_display_and_type_declarations_are_documented() {
        let queries = include_str!("templates/queries.md");
        for control in [
            "press **Display**",
            "Choosing **Page** in the column picker",
            "**complete sample**",
            "**Count**, **Sum**, **Average**, or **None**",
            "Built-in columns such as Page and State have no property total",
            "leaves the total blank or shows why it is unavailable",
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
mod customize_look_guide_tests {
    use super::*;

    const TITLE: &str = "Workflows/Customize Tine's look";

    fn page() -> &'static str {
        GUIDE_TEMPLATES
            .iter()
            .find(|t| t.title == TITLE)
            .unwrap_or_else(|| panic!("Guide page {TITLE:?} is not bundled"))
            .markdown
    }

    /// GH #610: a user who wants to change one visual detail must be able to
    /// find the page from the Guide index, find the file, open it, recover from
    /// a broken stylesheet, and inspect an element, using the names the app
    /// shows. `src/themeTokens.guard.test.ts` checks the token table and the
    /// control names against the contract and the Settings source.
    #[test]
    fn the_customize_page_is_reachable_and_teaches_the_whole_loop() {
        let index = GUIDE_TEMPLATES
            .iter()
            .find(|t| t.title == "Tine Guide")
            .unwrap()
            .markdown;
        assert!(index.contains("[[Workflows/Customize Tine's look]]"));
        for other in [
            include_str!("templates/extend-tine.md"),
            include_str!("templates/plugins.md"),
        ] {
            assert!(other.contains("[[Workflows/Customize Tine's look]]"));
        }
        let page = page();
        for needed in [
            "logseq/custom.css",
            "Settings",
            "**Edit custom.css**",
            "**Disable custom CSS**",
            "**Developer tools**",
            "**Ctrl+Shift+J**",
            "Soft",
            "--tine-embed-bg: transparent",
            "--tine-bullet-color",
            "--tine-content-width",
            "--tine-content-font",
            "Nord, Solarized, Gruvbox",
        ] {
            assert!(page.contains(needed), "the page must mention {needed}");
        }
        assert!(
            page.matches("```css").count() >= 10,
            "ten copy-paste recipes"
        );
        assert!(page.contains("What you should see"));
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

    /// AP2/D11 (Martin 2026-10-04): the query-model reference states the rule,
    /// names the one "finds less" exception and the refusals, and is linked
    /// from the Guide index and the Queries page.
    #[test]
    fn query_model_reference_states_the_rule_and_every_divergence_class() {
        let model = page("Reference/Tine query model");
        for promised in [
            "never less",
            "never changes your files",
            "`cafe` does not find `café`",
            "`(task)` with no marker",
            "no datalog engine",
            "`:view` and `:result-transform` never run",
            "20,000 rows",
            "Tine 0.6 (stable)",
        ] {
            assert!(
                model.contains(promised),
                "query model page lost: {promised}"
            );
        }
        assert!(page("Tine Guide").contains("[[Reference/Tine query model]]"));
        assert!(page("Features/Queries").contains("[[Reference/Tine query model]]"));
    }

    #[test]
    fn queries_document_bare_task_open_rule_and_og_difference() {
        let queries = page("Features/Queries");
        for detail in [
            "`(task)`",
            "has a task marker",
            "`DONE`",
            "`CANCELED`",
            "`CANCELLED`",
            "`WAITING`",
            "`IN-PROGRESS`",
            "OG Logseq",
            "explicit marker lists",
        ] {
            assert!(
                queries.contains(detail),
                "missing bare task detail: {detail}"
            );
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
            // og I1f (#35): the live export's home choice and doctor's findings.
            "--home \"Page name\"",
            "configured home page when it is exported",
            "every unreadable page",
            "claimed by more than one file",
            "exits with status 1",
        ] {
            assert!(cli.contains(promised), "Guide omitted {promised}");
        }
        for stale in ["--replace", "graph-relative"] {
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

    /// GH #623: the Windows Defender hint and its opt-in exclusion are
    /// user-visible: what triggers it, the click that is required, and each
    /// outcome the user can see.
    #[test]
    fn troubleshooting_documents_the_windows_defender_hint() {
        let recovery = page("Reference/Troubleshooting and recovery");
        for detail in [
            "Opening a large graph is slow on Windows",
            "**Add an exclusion for this graph folder**",
            "Windows asks for administrator approval",
            "Tine never changes Defender without that choice",
            "the notice returns the next time this graph opens slowly",
            "`windowsDefenderRealtime`",
            "never any path",
        ] {
            assert!(
                recovery.contains(detail),
                "missing Windows Defender hint detail: {detail}"
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

/// og-I2: three master Guide sentences (GH #468/#464 sidebar link areas,
/// GH #514/#516 embed root bullet, search-fold mark examples) and the checks
/// that keep them true.
#[cfg(test)]
mod i2_guide_sentence_tests {
    #[test]
    fn sidebar_link_areas_are_named_for_both_sidebars() {
        let page = super::GUIDE_TEMPLATES
            .iter()
            .find(|template| template.title == "Workflows/Find and revisit")
            .expect("find-and-revisit workflow is registered");
        assert!(page
            .markdown
            .contains("In the left sidebar the whole row is the link"));
        assert!(page
            .markdown
            .contains("In the right sidebar, where items are parked pages"));
    }

    #[test]
    fn embed_root_bullet_drag_versus_click_is_documented() {
        let page = include_str!("templates/pages-links-references-search.md");
        assert!(page.contains("drag its visible root bullet to move the embed on the host page"));
        assert!(page.contains("clicking that bullet still zooms into the source block"));
    }

    #[test]
    fn search_guide_mark_examples_are_what_search_does() {
        use crate::search_query::canonical_fold;
        let page = include_str!("templates/pages-links-references-search.md");
        for (query, text) in [
            ("cafe", "café"),
            ("lodz", "Łódź"),
            ("Tine", "Ｔｉｎｅ"),
            ("елка", "ёлка"),
        ] {
            assert!(page.contains(&format!("`{query}` finds `{text}`")));
            assert_eq!(
                canonical_fold(query),
                canonical_fold(text),
                "{query} finds {text}"
            );
        }
        for (query, text) in [("か", "が"), ("и", "й"), ("क", "कु")] {
            assert!(page.contains(&format!("`{query}` does not find `{text}`")));
            assert_ne!(
                canonical_fold(query),
                canonical_fold(text),
                "{query} must not find {text}"
            );
        }
    }
}

#[cfg(test)]
mod external_link_guide_tests {
    #[test]
    fn guide_explains_external_copy_lazy_identity_and_missing_targets() {
        let guide = include_str!("templates/find-and-revisit.md");
        for outcome in [
            "**Copy link**",
            "`tine://`",
            "`logseq/tine-graph-id`",
            "saved ID alone",
            "missing graph or target",
            "remembers that choice",
        ] {
            assert!(
                guide.contains(outcome),
                "GH #181 Guide is missing {outcome}"
            );
        }
    }

    #[test]
    fn guide_explains_reference_breadcrumb_context() {
        // GH #526: a clickable breadcrumb nobody is told about reads as missing.
        let guide = include_str!("templates/find-and-revisit.md");
        for outcome in [
            "Click an ancestor in a **Linked References** or **Unlinked References** breadcrumb",
            "**Shift+click** opens the ancestor in the right sidebar",
            "leaving and returning resets it",
        ] {
            assert!(
                guide.contains(outcome),
                "GH #526 Guide is missing {outcome}"
            );
        }
    }
}

#[cfg(test)]
mod asset_manifest_tests {
    use super::*;

    fn collect_guide_asset_refs(markdown: &str, into: &mut std::collections::HashSet<String>) {
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

    #[test]
    fn guide_assets_are_exactly_the_sorted_top_level_assets_the_templates_reference() {
        let mut referenced = std::collections::HashSet::new();
        for template in GUIDE_TEMPLATES {
            collect_guide_asset_refs(template.markdown, &mut referenced);
        }
        let mut referenced: Vec<String> = referenced.into_iter().collect();
        referenced.sort();
        let manifest: Vec<&str> = GUIDE_ASSETS.iter().map(|asset| asset.name).collect();
        assert_eq!(
            manifest, referenced,
            "GUIDE_ASSETS must list every referenced asset, sorted, once"
        );
        assert_eq!(manifest, ["quick-capture.png"]);
        for asset in GUIDE_ASSETS {
            assert!(
                !asset.name.contains('/') && !asset.name.contains('\\'),
                "guide assets must be top-level files: {}",
                asset.name
            );
            assert!(!asset.bytes.is_empty(), "{}", asset.name);
        }
    }
}
