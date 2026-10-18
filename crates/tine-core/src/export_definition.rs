//! Export definitions (`docs/contracts/export-definitions.md`): an ordinary
//! query block that carries a destination property, `export::` (a folder) or
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
//! `graph_parser/block.cljs:70-74`), so the reference rewrite of a rename never
//! sees the quoted name. A rename therefore also rewrites, in DEFINITION
//! blocks only, every quoted page name an active TQL `name` comparison holds
//! ([`crate::query::tql::page_name_literals`]), so the definition keeps
//! selecting the renamed page and a new page with the old name is not shared.
//! Deliberate scope: an ordinary (non-definition) query block keeps OG's
//! behaviour and is not rewritten; `{{query}}` text is never rewritten (it is
//! OG's syntax, and OG's rename leaves it); `like`, `namespace` and property
//! comparisons are not page selectors and stay. An alias is not a selector
//! target: `name = 'x'` compares the page's own name, so a selector spelling an
//! alias selects nothing and a rename leaves it alone.

use std::collections::HashMap;

use crate::doc::{property_key_norm, DocBlock};
use crate::outline::{self, OutlineFormat};
use crate::query::macro_text::{
    macro_safe, query_macro_extents, recognizable_macro, split_trailing_map, FormFamily,
};

/// The destination properties that make a query block an export definition.
pub const DESTINATION_KEYS: [&str; 2] = ["export", "tine.share"];

/// The OG-dialect query listing every export definition block: the exports
/// page's overview query, and how a rename finds the pages that hold one.
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

/// The rename step of [`crate::refs::rename_rewrite`]: in every export
/// definition block of `text`, replace each TQL page selector whose name
/// normalizes to a key of `renames` (normalized old name -> new name) with the
/// new name. `None` when nothing changes. Every other byte is kept, and a
/// macro whose rewritten argument would not stay one macro (a new name with a
/// `}`; the document parser's rule, [`recognizable_macro`]) is left as it was.
///
/// Cost: a page without a `{{tine-query` macro costs one O(text) scan; one
/// with such a macro adds one TQL parse per macro and, when one holds a page
/// selector, one outline parse and a property parse of each block holding one.
pub(crate) fn rename_selectors(
    text: &str,
    is_org: bool,
    renames: &HashMap<String, String>,
) -> Option<String> {
    if renames.is_empty() || !text.contains("{{") {
        return None;
    }
    // The page parse reads LF text (`doc::normalize_line_endings`): find the
    // edits there and place them in the file's own bytes. Normalizing drops the
    // `\r` of each CRLF and turns a lone `\r` into `\n`, so an LF offset moves
    // by the CRLFs before it.
    let lf = crate::doc::normalize_line_endings(text);
    let edits = selector_edits(&lf, is_org, renames);
    if edits.is_empty() {
        return None;
    }
    let crlf: Vec<usize> = lf_offsets_of_crlf(text);
    let original = |at: usize| at + crlf.partition_point(|lf_at| *lf_at < at);
    let mut out = text.to_owned();
    for (range, replacement) in edits.into_iter().rev() {
        out.replace_range(original(range.start)..original(range.end), &replacement);
    }
    Some(out)
}

/// The page names the TQL selectors of `raw` (one block's text) spell, for a
/// caller that holds the block in memory: the rename planner reads a
/// definition's page only when one of them is renamed.
pub fn selected_names(raw: &str) -> Vec<String> {
    selectors(raw)
        .into_iter()
        .flat_map(|(_, literals)| literals.into_iter().map(|(_, name)| name))
        .collect()
}

/// Every `{{tine-query}}` macro of `text` with its page selectors: the quoted
/// name of each active TQL `name` comparison, as a byte range of `text` and
/// the decoded name ([`crate::query::tql::page_name_literals`] over the form,
/// its options map split off).
fn selectors(
    text: &str,
) -> Vec<(
    crate::query::macro_text::MacroExtent,
    Vec<(std::ops::Range<usize>, String)>,
)> {
    let mut out = Vec::new();
    if !text.contains("{{") {
        return out;
    }
    for extent in query_macro_extents(text) {
        if FormFamily::for_macro_name(&extent.name) != FormFamily::Tql {
            continue;
        }
        let argument_end = extent.end - 2;
        let argument_start = argument_end - extent.argument.len();
        let (form, _) = split_trailing_map(&extent.argument, FormFamily::Tql);
        let form_start =
            argument_start + extent.argument.len() - extent.argument.trim_start().len();
        if text.get(argument_start..argument_end) != Some(extent.argument.as_str())
            || !text[form_start..].starts_with(form.as_str())
        {
            continue;
        }
        let literals = crate::query::tql::page_name_literals(&form)
            .into_iter()
            .map(|(range, name)| (form_start + range.start..form_start + range.end, name))
            .collect();
        out.push((extent, literals));
    }
    out
}

/// The selector edits of LF `text`, in source order.
fn selector_edits(
    text: &str,
    is_org: bool,
    renames: &HashMap<String, String>,
) -> Vec<(std::ops::Range<usize>, String)> {
    let mut edits = Vec::new();
    let macros = selectors(text);
    if macros.iter().all(|(_, literals)| literals.is_empty()) {
        return edits;
    }
    let format = if is_org {
        OutlineFormat::Org
    } else {
        OutlineFormat::Markdown
    };
    // An outline lsdoc cannot represent has no blocks (the page parse shows it
    // as page text), so it has no definition block either.
    let Ok(headers) = outline::headers(text, format) else {
        return edits;
    };
    let body = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    };
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    // Block `n` spans from its header line to the next header; the page's
    // preamble (its pre-block) is block 0 when there is one.
    let first = headers.first().map_or(lines.len(), |header| header.line);
    let mut starts = Vec::with_capacity(headers.len() + 1);
    let mut blocks = Vec::with_capacity(headers.len() + 1);
    if first > 0 {
        let mut preamble = DocBlock::new(lines[..first].join("\n"));
        preamble.is_org = is_org;
        starts.push(0);
        blocks.push(preamble);
    }
    fn preorder(into: &mut Vec<DocBlock>, roots: Vec<DocBlock>) {
        for mut block in roots {
            let children = std::mem::take(&mut block.children);
            into.push(block);
            preorder(into, children);
        }
    }
    starts.extend(headers.iter().map(|header| line_starts[header.line]));
    preorder(&mut blocks, outline::blocks(&lines, &headers, is_org));
    debug_assert_eq!(starts.len(), blocks.len());
    let mut definition: HashMap<usize, bool> = HashMap::new();
    for (extent, literals) in macros {
        let at = starts.partition_point(|start| *start <= extent.start);
        let Some(index) = at.checked_sub(1) else {
            continue;
        };
        let end = starts.get(index + 1).copied().unwrap_or(text.len());
        if literals.is_empty()
            || extent.end > end
            || !*definition
                .entry(index)
                .or_insert_with(|| is_definition(&blocks[index]))
        {
            continue;
        }
        let argument_start = extent.end - 2 - extent.argument.len();
        let mut argument = extent.argument.clone();
        let mut local = Vec::new();
        for (range, name) in literals.into_iter().rev() {
            let Some(to) = renames.get(&crate::refs::normalize(&name)) else {
                continue;
            };
            if *to == name {
                continue;
            }
            let quoted = crate::query::print::sql_string(to);
            argument.replace_range(
                range.start - argument_start..range.end - argument_start,
                &quoted,
            );
            local.push((range, quoted));
        }
        if local.is_empty()
            || macro_safe(&argument, FormFamily::Tql).is_err()
            || recognizable_macro(&extent.name, &argument).is_err()
        {
            continue;
        }
        local.reverse();
        edits.extend(local);
    }
    edits
}

/// The LF-text offset of the `\n` of every CRLF in `text`.
fn lf_offsets_of_crlf(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (removed, (at, _)) in text.match_indices("\r\n").enumerate() {
        out.push(at - removed);
    }
    out
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
    fn a_rename_rewrites_the_selector_of_a_markdown_definition_only() {
        let text = "- {{tine-query @page name = 'Project'}}\n  export:: project-site\n\
                    - {{tine-query @page name = 'Project'}}\n";
        assert_eq!(
            rename(text, false, "Project", "Project archive"),
            "- {{tine-query @page name = 'Project archive'}}\n  export:: project-site\n\
             - {{tine-query @page name = 'Project'}}\n",
            "the second block is an ordinary query: OG's rename leaves it, and so does Tine"
        );
    }

    #[test]
    fn a_rename_rewrites_near_seeds_and_link_elements_of_a_definition() {
        // docs/contracts/tql-links-near.md "Renames": seeds and element
        // conditions are ordinary `name` comparisons.
        let text = "- {{tine-query @page and near(name in ('Project', 'Notes'), 2, through => not name = 'Project') or any(links, name = 'project')}}\n  export:: project-site\n";
        assert_eq!(
            rename(text, false, "Project", "Project archive"),
            "- {{tine-query @page and near(name in ('Project archive', 'Notes'), 2, through => not name = 'Project archive') or any(links, name = 'Project archive')}}\n  export:: project-site\n"
        );
    }

    #[test]
    fn a_rename_rewrites_the_selector_of_an_org_definition() {
        let text = "* Share\n** {{tine-query @page name = 'Project'}}\n:PROPERTIES:\n\
                    :tine.share: someone/abc\n:END:\n* {{tine-query @page name = 'Project'}}\n";
        assert_eq!(
            rename(text, true, "Project", "Project archive"),
            "* Share\n** {{tine-query @page name = 'Project archive'}}\n:PROPERTIES:\n\
             :tine.share: someone/abc\n:END:\n* {{tine-query @page name = 'Project'}}\n"
        );
    }

    #[test]
    fn every_page_name_comparison_follows_and_nothing_else_does() {
        let text = "- {{tine-query @page (name = 'project' or name in ('Notes', 'Project')) \
                    and name != 'PROJECT' and name like 'Project%' and prop('owner') = 'Project'}}\n  \
                    export:: x\n\
                    - {{tine-query @block page.name = 'Project' and content = 'Project'}}\n  \
                    export:: y\n";
        assert_eq!(
            rename(text, false, "Project", "Project archive"),
            "- {{tine-query @page (name = 'Project archive' or name in ('Notes', 'Project archive')) \
             and name != 'Project archive' and name like 'Project%' and prop('owner') = 'Project'}}\n  \
             export:: x\n\
             - {{tine-query @block page.name = 'Project archive' and content = 'Project'}}\n  \
             export:: y\n"
        );
    }

    #[test]
    fn a_new_name_is_quoted_for_tql_and_options_stay_verbatim() {
        let text = "- {{tine-query @page name = 'Project' {:title \"Mine\"}}}\n  export:: x\n";
        assert_eq!(
            rename(text, false, "Project", "O'Brien"),
            "- {{tine-query @page name = 'O''Brien' {:title \"Mine\"}}}\n  export:: x\n"
        );
    }

    #[test]
    fn og_syntax_and_unwritable_names_are_left_as_they_were() {
        // `{{query}}` is OG's syntax, which OG's rename does not rewrite (D-18).
        let og = "- {{query (page \"Project\")}}\n  export:: x\n";
        assert_eq!(rename(og, false, "Project", "Project archive"), og);
        // A `}` cannot sit inside a query macro (`macro_safe`): the macro stays.
        let tql = "- {{tine-query @page name = 'Project'}}\n  export:: x\n";
        assert_eq!(rename(tql, false, "Project", "a}b"), tql);
    }

    #[test]
    fn a_crlf_definition_is_rewritten_in_place() {
        let text = "- {{tine-query @page name = 'Project'}}\r\n  export:: x\r\n";
        assert_eq!(
            rename(text, false, "Project", "Renamed"),
            "- {{tine-query @page name = 'Renamed'}}\r\n  export:: x\r\n"
        );
    }

    #[test]
    fn the_definitions_query_lists_both_destination_properties() {
        assert_eq!(
            super::definitions_query(),
            "(or (property export) (property tine.share))"
        );
    }
}
