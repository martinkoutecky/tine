//! CLI page-fragment insertion. The kernel owns the tree edit; write.rs owns
//! flush/read/plan/reparse/apply, including the CreateNew creation checks.
use serde::Deserialize;
use serde_json::{json, Value};
use tine_core::block_regions::{self, Edit};
use tine_core::model::{BlockDto, Format};
use tine_core::outline_kernel::{apply, KeySource, OutlineNode, Place, Verb};
use tine_store::EditKind;

use super::verbs::{internal, journal_day, journal_write_page, whole_graph, Diag};
use super::wire::ErrorBody;
use super::write::{
    block_at_mut, creation_turn, handle_page, outline_refused, page_turn, parse_handle, refused,
    resolve_on, store_refused, write_page_in_turn, Planned,
};
use crate::state::GraphSlot;

#[derive(Deserialize)]
struct Args {
    #[serde(default)]
    under: Option<String>,
    #[serde(default)]
    after: Option<String>,
    #[serde(default)]
    before: Option<String>,
    #[serde(default)]
    page: Option<String>,
    /// Absent = no journal target; an empty string = today.
    #[serde(default)]
    journal: Option<String>,
    content: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    rev: Option<String>,
}

pub(super) fn add_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let args: Args = super::verbs::parse(args)?;
    if [
        &args.under,
        &args.after,
        &args.before,
        &args.page,
        &args.journal,
    ]
    .iter()
    .filter(|a| a.is_some())
    .count()
        != 1
    {
        return Err(ErrorBody::new(
            "bad-args",
            "choose exactly one of under, after, before, page or journal",
            false,
        ));
    }
    let anchor = args
        .under
        .as_ref()
        .or(args.after.as_ref())
        .or(args.before.as_ref());
    let handle = anchor.map(|s| parse_handle(s)).transpose()?;
    let mut graph = whole_graph(slot)?;
    // A page named by a name nothing claimed is written under the whole graph's
    // turn (`creation_turn`); any other target under its page's own.
    let (page, held) = if let Some(handle) = &handle {
        (handle_page(slot, handle)?, None)
    } else if let Some(name) = &args.page {
        if name.trim().is_empty() {
            return Err(ErrorBody::new("bad-args", "page name is empty", false));
        }
        let (named, held) = creation_turn(slot, &mut graph, name)?;
        (named.page(name)?, held)
    } else {
        let day = journal_day(slot, args.journal.as_deref().filter(|d| !d.is_empty()))?;
        (journal_write_page(slot, &graph, day)?, None)
    };
    let turn = match held {
        Some(turn) => turn,
        None => page_turn(slot, &page)?,
    };
    let author = super::write::author_name(args.author.as_deref())
        .map_err(|e| refused("kernel", Some(&page), e))?;
    write_page_in_turn(
        slot,
        turn,
        &page,
        EditKind::InsertBlocks,
        args.rev.as_deref(),
        handle.is_none(),
        |doc, page, _, creating| {
            let org = doc.format == Format::Org;
            // Use the very same bounded page parser as a file read. A fragment's
            // pre-block is its first imported block, like the app's visible header.
            let fragment = slot
                .store
                .page_of(page, args.content.as_bytes())
                .map_err(|e| store_refused(e, page))?;
            if fragment.doc.read_only || fragment.read_only.is_some() {
                return Err(refused(
                    "read-only",
                    Some(page),
                    "the fragment cannot round-trip safely",
                ));
            }
            let mut blocks = fragment.doc.blocks;
            let mut explicit_header = false;
            if let Some(raw) = fragment.doc.pre_block.filter(|raw| !raw.trim().is_empty()) {
                explicit_header = !tine_core::page_header::page_property_lines(&raw, org)
                    .is_empty()
                    && tine_core::page_properties::preamble_content_range(&raw, org).is_none();
                blocks.insert(
                    0,
                    BlockDto {
                        raw,
                        ..Default::default()
                    },
                );
            }
            if blocks.is_empty() {
                return Err(ErrorBody::new("bad-args", "content has no blocks", false));
            }
            let header = creating
                && (explicit_header
                    || (!org
                        && blocks.first().is_some_and(|b| {
                            b.children.is_empty()
                                && tine_core::page_header::is_page_header_candidate(
                                    b.raw.trim_end_matches(['\n', '\r']),
                                )
                        })));
            let graph = whole_graph(slot)?;
            let mut live_ids = Vec::new();
            fn prepare(
                blocks: &[BlockDto],
                org: bool,
                author: &str,
                graph: &tine_store::WholeGraph,
                live_ids: &mut Vec<String>,
                page: &tine_store::PageId,
                header: bool,
            ) -> Result<Vec<OutlineNode>, ErrorBody> {
                blocks
                    .iter()
                    .enumerate()
                    .map(|(index, block)| {
                        for uuid in tine_core::render::block_refs(&block.raw, org).block {
                            if graph
                                .preview_block(&uuid, 1)
                                .map_err(|e| internal(Diag::ReadFailed, format!("{e:?}")))?
                                .is_none()
                            {
                                let mut error = refused(
                                    "broken-ref",
                                    Some(page),
                                    format!("no block has id:: {uuid}"),
                                );
                                error.detail.as_mut().unwrap()["uuid"] = json!(uuid);
                                return Err(error);
                            }
                        }
                        if let Some(id) = block_regions::parse(&block.raw, org).id {
                            let id = id.value.trim().to_lowercase();
                            if graph
                                .preview_block(&id, 1)
                                .map_err(|e| internal(Diag::ReadFailed, format!("{e:?}")))?
                                .is_some()
                            {
                                live_ids.push(id);
                            }
                        }
                        let raw = if header && index == 0 {
                            block.raw.clone()
                        } else {
                            block_regions::edit(
                                &block.raw,
                                org,
                                Edit::Property {
                                    key: tine_core::tine_keys::AUTHOR.into(),
                                    value: Some(author.into()),
                                },
                            )
                            .map_err(|e| refused("kernel", Some(page), e))?
                        };
                        Ok(OutlineNode {
                            raw,
                            children: prepare(
                                &block.children,
                                org,
                                author,
                                graph,
                                live_ids,
                                page,
                                false,
                            )?,
                        })
                    })
                    .collect()
            }
            let mut blocks = prepare(&blocks, org, &author, &graph, &mut live_ids, page, header)?;
            let mut next = doc.clone();
            if header {
                next.pre_block = Some(blocks.remove(0).raw);
                if blocks.is_empty() {
                    return Ok(Planned {
                        doc: next,
                        changed: vec![],
                    });
                }
            }
            let path = handle
                .as_ref()
                .map(|h| resolve_on(doc, page, h, anchor.unwrap(), args.rev.is_some()))
                .transpose()?;
            // MC5: only message roots get a creation date; their descendants
            // remain an ordinary outline. Ordinary --under is unchanged.
            let in_thread = args.under.is_some()
                && path.as_ref().is_some_and(|path| {
                    (2..=path.len()).any(|depth| {
                        super::write::block_at(&doc.blocks, &path[..depth]).is_some_and(|block| {
                            let regions = block_regions::parse(&block.raw, org);
                            tine_core::editor_text::comment_quote(
                                regions
                                    .properties
                                    .iter()
                                    .filter(|p| p.primary)
                                    .map(|p| (p.key.as_str(), p.value.as_str())),
                            )
                            .is_some()
                        })
                    })
                });
            if in_thread {
                let date = chrono::Utc::now().format("%Y-%m-%dT%H:%MZ").to_string();
                for block in &mut blocks {
                    block.raw = block_regions::edit(
                        &block.raw,
                        org,
                        Edit::Property {
                            key: "tine.date".into(),
                            value: Some(date.clone()),
                        },
                    )
                    .map_err(|e| refused("kernel", Some(page), e))?;
                }
            }
            if let Some(path) = &path {
                let block = block_at_mut(&mut next.blocks, path).expect("resolved path");
                if block_regions::parse(&block.raw, org).id.is_none() {
                    block.raw = block_regions::edit(
                        &block.raw,
                        org,
                        Edit::Property {
                            key: "id".into(),
                            value: Some(uuid::Uuid::new_v4().to_string()),
                        },
                    )
                    .map_err(|e| refused("kernel", Some(page), e))?;
                }
            }
            let arena = super::arena::Arena::of(&next);
            let place = if let Some(path) = path {
                let key = arena.key_at(&path).expect("resolved path");
                if args.under.is_some() {
                    Place::LastChild { key }
                } else if args.after.is_some() {
                    Place::After { key }
                } else {
                    Place::Before { key }
                }
            } else {
                Place::Slot {
                    parent: None,
                    page: doc.name.clone(),
                    index: doc.blocks.len(),
                }
            };
            let change = apply(
                std::slice::from_ref(&arena.page),
                &Verb::InsertOutline {
                    place,
                    blocks,
                    live_ids,
                    reuse: None,
                },
                &KeySource {
                    prefix: "added".into(),
                    next: 0,
                },
            )
            .map_err(|r| outline_refused(r, page, "add"))?;
            let keys: Vec<_> = (0..change.keys_used).map(|n| format!("added{n}")).collect();
            let (next, mut changed) = arena.apply(&next, &change, &keys);
            changed.sort();
            Ok(Planned { doc: next, changed })
        },
    )
}
