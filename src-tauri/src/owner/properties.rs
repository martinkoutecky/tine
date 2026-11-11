//! Properties are edits of the existing block or page-header surface. Both
//! run in the same owner turn and guarded save as ordinary app edits.
use serde::Deserialize;
use serde_json::Value;
use tine_core::model::Format;
use tine_store::EditKind;

use super::verbs::{journal_day, journal_write_page, resolve_write_page, whole_graph};
use super::wire::ErrorBody;
use super::write::{
    block_at_mut, handle_page, parse_handle, refused, resolve_on, write_page, Planned,
};
use crate::state::GraphSlot;

#[derive(Deserialize)]
struct Args {
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    page: Option<String>,
    #[serde(default)]
    journal: Option<String>,
    key: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    rev: Option<String>,
}

pub(super) fn prop_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let args: Args = super::verbs::parse(args)?;
    let text = args
        .handle
        .as_deref()
        .ok_or_else(|| ErrorBody::new("bad-args", "block.prop needs a handle", false))?;
    let handle = parse_handle(text)?;
    let page = handle_page(slot, &handle)?;
    let value = if args.key.eq_ignore_ascii_case(tine_core::tine_keys::AUTHOR) {
        args.value
            .as_deref()
            .map(|name| super::write::author_name(Some(name)))
            .transpose()
            .map_err(|e| refused("kernel", Some(&page), e))?
    } else {
        args.value.clone()
    };
    write_page(
        slot,
        &page,
        EditKind::SaveBlock,
        args.rev.as_deref(),
        |doc, page| {
            let path = resolve_on(doc, page, &handle, text, args.rev.is_some())?;
            let mut next = doc.clone();
            let block = block_at_mut(&mut next.blocks, &path).expect("resolved path");
            block.raw = tine_core::block_regions::edit(
                &block.raw,
                doc.format == Format::Org,
                tine_core::block_regions::Edit::Property {
                    key: args.key.clone(),
                    value: value.clone(),
                },
            )
            .map_err(|e| refused("kernel", Some(page), e))?;
            Ok(Planned {
                doc: next,
                changed: vec![path],
            })
        },
    )
}

pub(super) fn prop_page(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let args: Args = super::verbs::parse(args)?;
    if args.handle.is_some() || args.page.is_some() == args.journal.is_some() {
        return Err(ErrorBody::new(
            "bad-args",
            "choose exactly one of page or journal",
            false,
        ));
    }
    let graph = whole_graph(slot)?;
    let page = if let Some(name) = &args.page {
        resolve_write_page(slot, &graph, name)?
    } else {
        let day = journal_day(slot, args.journal.as_deref().filter(|s| !s.is_empty()))?;
        journal_write_page(slot, &graph, day)?
    };
    write_page(
        slot,
        &page,
        EditKind::SaveBlock,
        args.rev.as_deref(),
        |doc, page| {
            let mut next = doc.clone();
            // A headerless Markdown file can still carry OG's first-bullet header.
            // Canonicalize that existing header exactly as the store does on save.
            if doc.format == Format::Md
                && doc.pre_block.as_deref().unwrap_or("").is_empty()
                && doc.blocks.first().is_some_and(|b| {
                    b.children.is_empty()
                        && tine_core::page_header::is_page_header_candidate(
                            b.raw.trim_end_matches(['\n', '\r']),
                        )
                })
            {
                next.pre_block = Some(next.blocks.remove(0).raw);
            }
            let parts = tine_core::page_property_edit::set(
                &[next.pre_block.clone().unwrap_or_default()],
                doc.format == Format::Org,
                &args.key,
                args.value.as_deref(),
            )
            .map_err(|e| refused("kernel", Some(page), e))?;
            next.pre_block = parts.into_iter().next().filter(|s| !s.trim().is_empty());
            Ok(Planned {
                doc: next,
                changed: vec![],
            })
        },
    )
}
