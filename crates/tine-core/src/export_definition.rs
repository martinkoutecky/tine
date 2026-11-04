//! Export definitions (`docs/contracts/export-definitions.md`): an ordinary
//! query block that carries a destination property, `tine.export::` (a folder) or
//! `tine.share::` (a quick share). The one answer to "is this block an export
//! definition", and the rename of the page names its query selects.
//!
//! **The "this page" selector is ordinary TQL**: `{{tine-query @page and name =
//! 'Project'}}`, the printer's own spelling (`@page name = 'Project'` reads the
//! same). It is page-anchored, so a page with no blocks is selected, and it
//! stays an ordinary term the query builder can combine (`@page and name =
//! 'Project' or name = 'Notes'`). OG's DSL has no page-row exact-name form
//! (`(page "x")` matches the page's BLOCKS, `query_dsl.cljs` `build-page`), and
//! a Tine-only head inside `{{query}}` would mean something else in OG Logseq
//! (D-18), so the selector has no `{{query}}` spelling.
//!
//! **Renames.** A query macro creates no page reference (OG
//! `graph_parser/block.cljs:70-74`), but a rename rewrites the page-name
//! arguments of every query like links (D-20261031-CMQ, `refs::rename_rewrite`):
//! every quoted page name an active TQL `name` comparison holds
//! ([`crate::query::tql::page_name_literals`]), so a definition keeps selecting
//! the renamed page and a new page with the old name is not shared. `like`,
//! `namespace` and property comparisons are not page selectors and stay. An
//! alias is not a selector target: `name = 'x'` compares the page's own name,
//! so a selector spelling an alias selects nothing and a rename leaves it
//! alone.

use crate::doc::{property_key_norm, DocBlock};

/// The destination properties that make a query block an export definition.
pub const DESTINATION_KEYS: [&str; 2] = [crate::tine_keys::EXPORT, "tine.share"];

/// The OG-dialect query listing every export definition block: the exports
/// page's overview query.
pub fn definitions_query() -> String {
    let terms: Vec<String> = DESTINATION_KEYS
        .iter()
        .map(|key| format!("(property {key})"))
        .collect();
    format!("(or {})", terms.join(" "))
}

/// Whether `block` carries a destination property.
pub fn is_definition(block: &DocBlock) -> bool {
    block
        .projection()
        .properties()
        .iter()
        .any(|(key, _)| DESTINATION_KEYS.contains(&property_key_norm(key).as_str()))
}

#[cfg(test)]
mod tests {
    use crate::config::FileNameFormat;
    use crate::query::print::{query_print, PrintDialect};
    use crate::query::registry::Registry;
    use crate::query::wire_parse::{parse_query_pair, QueryTextDialect};

    fn rename(text: &str, org: bool, from: &str, to: &str) -> String {
        crate::refs::rename_rewrite(
            text,
            org,
            &[(from.to_owned(), to.to_owned())],
            FileNameFormat::TripleLowbar,
        )
    }

    #[test]
    fn the_selector_is_page_anchored_tql_that_prints_back_and_has_no_og_spelling() {
        let parse =
            |text: &str| parse_query_pair(text, QueryTextDialect::MacroTql, &[], Registry::none());
        // The frozen spelling is the printer's own, so a definition the dialog
        // writes is byte-stable under the builder; the short form an author may
        // type reads as the same query.
        for (selector, typed) in [
            ("@page and name = 'Project'", "@page name = 'Project'"),
            (
                "@page and name = 'Project' or name = 'Notes'",
                "@page name = 'Project' or name = 'Notes'",
            ),
        ] {
            let parsed = parse(selector);
            assert!(parsed.query.diagnostics.is_empty(), "{selector}");
            assert_eq!(parsed.query.anchor, crate::query::ir::Anchor::Page);
            assert_eq!(parsed.query.filter, parse(typed).query.filter);
            let printed =
                query_print(&parsed.query, &parsed.view, PrintDialect::TqlMacro, false).unwrap();
            assert_eq!(printed, selector, "printQuery round-trips the selector");
            // D-18: OG's DSL has no page-row name form, so the builder can never
            // save this selector as `{{query}}` text OG would read differently.
            assert!(query_print(&parsed.query, &parsed.view, PrintDialect::Og, false).is_err());
        }
    }

    /// The share dialog (src/exportsPage.ts `pageSelectorQuery`) builds the selector as
    /// this exact wire value and has the engine print it; the TypeScript test pins the
    /// same file, so the two cannot drift.
    #[test]
    fn the_share_dialogs_builder_selector_prints_as_the_contract_selector() {
        let query: crate::query::ir::Query =
            serde_json::from_str(include_str!("query/fixtures/share/page_selector.json"))
                .expect("the fixture is a query");
        let printed = query_print(
            &query,
            &crate::query::ir::ViewSettings::default(),
            PrintDialect::TqlMacro,
            false,
        )
        .unwrap();
        assert_eq!(printed, "@page and name = 'Project'");
    }

    #[test]
    fn a_rename_rewrites_the_selector_of_a_definition_and_of_an_ordinary_query() {
        let text = "- {{tine-query @page name = 'Project'}}\n  tine.export:: project-site\n\
                    - {{tine-query @page name = 'Project'}}\n";
        assert_eq!(
            rename(text, false, "Project", "Project archive"),
            "- {{tine-query @page name = 'Project archive'}}\n  tine.export:: project-site\n\
             - {{tine-query @page name = 'Project archive'}}\n",
            "a query's page names follow a rename like links (D-20261031-CMQ)"
        );
    }

    #[test]
    fn a_rename_rewrites_near_seeds_and_link_elements_of_a_definition() {
        // docs/contracts/tql-links-near.md "Renames": seeds and element
        // conditions are ordinary `name` comparisons.
        let text = "- {{tine-query @page and near(name in ('Project', 'Notes'), 2) and not name = 'Project' or any(links, name = 'project')}}\n  tine.export:: project-site\n";
        assert_eq!(
            rename(text, false, "Project", "Project archive"),
            "- {{tine-query @page and near(name in ('Project archive', 'Notes'), 2) and not name = 'Project archive' or any(links, name = 'Project archive')}}\n  tine.export:: project-site\n"
        );
    }

    #[test]
    fn a_rename_rewrites_the_selector_of_an_org_definition() {
        let text = "* Share\n** {{tine-query @page name = 'Project'}}\n:PROPERTIES:\n\
                    :tine.share: someone/abc\n:END:\n* {{tine-query @page name = 'Project'}}\n";
        assert_eq!(
            rename(text, true, "Project", "Project archive"),
            "* Share\n** {{tine-query @page name = 'Project archive'}}\n:PROPERTIES:\n\
             :tine.share: someone/abc\n:END:\n* {{tine-query @page name = 'Project archive'}}\n"
        );
    }

    #[test]
    fn every_page_name_comparison_follows_and_nothing_else_does() {
        let text = "- {{tine-query @page (name = 'project' or name in ('Notes', 'Project')) \
                    and name != 'PROJECT' and name like 'Project%' and prop('owner') = 'Project'}}\n  \
                    tine.export:: x\n\
                    - {{tine-query @block page.name = 'Project' and content = 'Project'}}\n  \
                    tine.export:: y\n";
        assert_eq!(
            rename(text, false, "Project", "Project archive"),
            "- {{tine-query @page (name = 'Project archive' or name in ('Notes', 'Project archive')) \
             and name != 'Project archive' and name like 'Project%' and prop('owner') = 'Project'}}\n  \
             tine.export:: x\n\
             - {{tine-query @block page.name = 'Project archive' and content = 'Project'}}\n  \
             tine.export:: y\n"
        );
    }

    #[test]
    fn a_new_name_is_quoted_for_tql_and_options_stay_verbatim() {
        let text = "- {{tine-query @page name = 'Project' {:title \"Mine\"}}}\n  tine.export:: x\n";
        assert_eq!(
            rename(text, false, "Project", "O'Brien"),
            "- {{tine-query @page name = 'O''Brien' {:title \"Mine\"}}}\n  tine.export:: x\n"
        );
    }

    #[test]
    fn og_page_names_follow_and_unwritable_names_are_left_as_they_were() {
        // A `{{query}}` page-name argument follows like a link
        // (D-20261031-CMQ supersedes the D-18 exception); search text stays.
        let og = "- {{query (and (page \"Project\") \"Project\")}}\n  tine.export:: x\n";
        assert_eq!(
            rename(og, false, "Project", "Project archive"),
            "- {{query (and (page \"Project archive\") \"Project\")}}\n  tine.export:: x\n"
        );
        // A `}` cannot sit inside a query macro (`macro_safe`): the macro stays.
        let tql = "- {{tine-query @page name = 'Project'}}\n  tine.export:: x\n";
        assert_eq!(rename(tql, false, "Project", "a}b"), tql);
    }

    #[test]
    fn a_crlf_definition_is_rewritten_in_place() {
        let text = "- {{tine-query @page name = 'Project'}}\r\n  tine.export:: x\r\n";
        assert_eq!(
            rename(text, false, "Project", "Renamed"),
            "- {{tine-query @page name = 'Renamed'}}\r\n  tine.export:: x\r\n"
        );
    }

    #[test]
    fn the_definitions_query_lists_both_destination_properties() {
        assert_eq!(
            super::definitions_query(),
            "(or (property tine.export) (property tine.share))"
        );
    }
}
