//! The page-name arguments of a block's query macros, as the query readers
//! take them (D-20261031-CMQ): the one reading the rename writer
//! (`refs_rename::query_edits`) and the model check (`cm_check::observe`)
//! both iterate. A parser-level answer, not part of either.

use super::macro_text::{query_macro_extents, split_trailing_map, FormFamily};
use std::collections::HashMap;
use std::ops::Range;

/// One query macro read for its page names.
pub(crate) struct MacroPageNames {
    /// The macro's name (`query`, `tine-query`) and its whole argument.
    pub name: String,
    pub argument: String,
    pub family: FormFamily,
    /// Where `argument` starts in the block's raw text.
    pub argument_start: usize,
    /// Each page-name token (its range in the raw text) and the name it reads.
    pub names: Vec<(Range<usize>, String)>,
}

/// The query macros of a block's `raw` text with their page-name arguments:
/// what the OG DSL parse reads as a page name, and each TQL page-name literal.
/// A macro in code (in the text or in a property's value) is no macro, and a
/// macro the query readers do not take as written (over a limit, an advanced
/// query, an argument that is not where its extent says) has no names.
/// Cost: one extent scan of `raw`; each macro's form parsed once; a property
/// value read for code at most once ([`PropertyValueCode`]).
pub(crate) fn macro_page_names(
    raw: &str,
    regions: &crate::block_regions::BlockRegions,
    is_org: bool,
) -> Vec<MacroPageNames> {
    let mut out = Vec::new();
    if !raw.contains("{{") {
        return out;
    }
    let mut value_code = PropertyValueCode::new(raw, regions, is_org);
    for extent in query_macro_extents(raw) {
        if regions.literal_at(extent.start) || value_code.contains(extent.start) {
            continue;
        }
        let family = FormFamily::for_macro_name(&extent.name);
        let argument_end = extent.end - 2;
        let argument_start = argument_end - extent.argument.len();
        let (form, _) = split_trailing_map(&extent.argument, family);
        let form_start =
            argument_start + extent.argument.len() - extent.argument.trim_start().len();
        if raw.get(argument_start..argument_end) != Some(extent.argument.as_str())
            || !raw[form_start..].starts_with(form.as_str())
            || !super::query_source_within_limit(&form)
            || !super::query_nesting_within_limit(&form)
        {
            continue;
        }
        let names = match family {
            FormFamily::Tql => super::tql::page_name_literals(&form),
            FormFamily::Edn if super::is_advanced(&form) => Vec::new(),
            FormFamily::Edn => super::og::page_name_arguments(&form),
        };
        out.push(MacroPageNames {
            name: extent.name,
            argument: extent.argument,
            family,
            argument_start,
            names: (names.into_iter())
                .map(|(range, name)| (form_start + range.start..form_start + range.end, name))
                .collect(),
        });
    }
    out
}

/// Where the code is in the property values of one block, each value read once.
///
/// The outline parser reads a property line as one `Properties` node and keeps
/// each value as text, which is why the block's literal regions end at the
/// node; the app shows the value as inline text (`PagePropertyValue`), so the
/// value parsed on its own, as the parser would read it as text, says what is
/// code there (a backtick span, an Org `=…=` or `~…~` span).
///
/// Lower bound: a value holding a query macro is read once, however many
/// macros it holds (O(its bytes)), and a value without one is not read; each
/// macro is then a binary search of that read's code ranges. `properties` are
/// in source order and disjoint (`block_regions`' visitors append in source
/// order), and so are a parse's `literals` (sorted and merged).
struct PropertyValueCode<'a> {
    raw: &'a str,
    regions: &'a crate::block_regions::BlockRegions,
    is_org: bool,
    /// The code ranges of each value already read, in the value's own offsets,
    /// by its index in `regions.properties`.
    read: HashMap<usize, Vec<crate::block_regions::Range>>,
}

impl<'a> PropertyValueCode<'a> {
    fn new(raw: &'a str, regions: &'a crate::block_regions::BlockRegions, is_org: bool) -> Self {
        Self {
            raw,
            regions,
            is_org,
            read: HashMap::new(),
        }
    }

    /// Whether `at` (an offset in `raw`) is inside code written in a property's
    /// value.
    fn contains(&mut self, at: usize) -> bool {
        let (raw, is_org, properties) = (self.raw, self.is_org, &self.regions.properties);
        let index = properties.partition_point(|property| property.value_range.1 <= at);
        let Some(property) = properties
            .get(index)
            .filter(|property| property.value_range.contains(at))
        else {
            return false;
        };
        let code = self.read.entry(index).or_insert_with(|| {
            #[cfg(test)]
            VALUE_PARSES.with(|reads| reads.set(reads.get() + 1));
            crate::block_regions::parse(property.value_range.slice(raw), is_org).literals
        });
        let at = at - property.value_range.0;
        let next = code.partition_point(|range| range.1 <= at);
        code.get(next).is_some_and(|range| range.0 <= at)
    }
}

#[cfg(test)]
thread_local! {
    /// Property values read for code on this thread (tests reset it first).
    pub(crate) static VALUE_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
