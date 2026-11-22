//! Parser observations used when writing source publications.
use crate::render::parsed_body;
use crate::render::render_query_source::visit_inlines;
use tine_core::lsdoc::ast::Inline;

pub(crate) fn visit_macro_calls(
    raw: &str,
    org: bool,
    property: bool,
    mut call: impl FnMut(&str, &[String]),
) {
    if !raw.contains("{{") {
        return;
    }
    let mut blocks = if property {
        tine_core::render::parse_text_bounded(raw, org)
            .map(|parsed| parsed.blocks)
            .unwrap_or_default()
    } else {
        parsed_body(raw, org)
    };
    let _: Result<(), std::convert::Infallible> = visit_inlines(&mut blocks, |inlines| {
        for inline in inlines {
            if let Inline::Macro { name, args, .. } = inline {
                call(name, args);
            }
        }
        Ok(())
    });
}

pub(crate) fn visit_document_calls(
    document: &tine_core::doc::Document,
    mut call: impl FnMut(&str, &[String]),
) {
    let org = document.org;
    if let Some(pre) = document
        .pre_block
        .as_deref()
        .filter(|pre| pre.contains("{{"))
    {
        let regions = tine_core::block_regions::parse_document(pre, org);
        for (_, value) in tine_core::page_header::page_property_lines_in(&regions, org) {
            visit_macro_calls(&value, org, true, &mut call);
        }
        if let Some(opening) = tine_core::page_properties::preamble_range_in(pre, org, &regions) {
            visit_macro_calls(&pre[opening], org, false, &mut call);
        }
    }
    let mut pending: Vec<_> = document.roots.iter().rev().collect();
    while let Some(block) = pending.pop() {
        if block.raw().contains("{{") {
            visit_macro_calls(block.raw(), org, false, &mut call);
            for (_, value) in block.projection().properties() {
                visit_macro_calls(value, org, true, &mut call);
            }
        }
        pending.extend(block.children.iter().rev());
    }
}
