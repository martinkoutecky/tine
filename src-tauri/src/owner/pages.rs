//! Page verbs call the app's host operations. Helpers use the same parser-owned
//! property edit as Copy block ref; no client writes graph files.
use serde::Deserialize;
use serde_json::{json, Value};
use tine_core::block_regions::{self, Edit};
use tine_core::model::{Format, PageDto};
use tine_store::{EditKind, HostReadError, PageId, PageOperation, Resolved};

use super::verbs::{parse, resolve_page, whole_graph};
use super::wire::{ok, ErrorBody};
use super::write::{self, apply_refused, host_failed, refused, store_refused, Planned};
use crate::state::GraphSlot;

#[derive(Deserialize)]
struct PageArgs {
    name: String,
    #[serde(default)]
    rev: Option<String>,
    #[serde(default)]
    yes: bool,
}

fn name_valid(name: &str) -> Result<(), ErrorBody> {
    if name.trim().is_empty() {
        Err(ErrorBody::new("bad-args", "page name is empty", false))
    } else {
        Ok(())
    }
}

/// Unlike a read's preferred claimant, a mutation must not choose among twins.
fn unique_page(slot: &GraphSlot, name: &str) -> Result<PageId, ErrorBody> {
    match unique_source(slot, name)? {
        Some(page) => Ok(page),
        None => resolve_page(&whole_graph(slot)?, name),
    }
}

/// A rename can also start from a reference-only name or fileless namespace.
fn unique_source(slot: &GraphSlot, name: &str) -> Result<Option<PageId>, ErrorBody> {
    let graph = whole_graph(slot)?;
    let check = |resolved| match resolved {
        Resolved::Existing { id, mut others } => {
            others.insert(0, id);
            others
        }
        Resolved::Alias { owners } => owners,
        Resolved::Absent { .. } => Vec::new(),
    };
    let mut candidates = check(graph.resolve(name, false));
    if candidates.is_empty() {
        candidates = check(graph.resolve(name, true));
    }
    if candidates.len() > 1 {
        let mut body = ErrorBody::new(
            "ambiguous",
            "several files claim this page; nothing changed",
            false,
        );
        body.detail = Some(
            json!({"admitted":false,"candidates": candidates.iter().map(PageId::as_str).collect::<Vec<_>>()}),
        );
        return Err(body);
    }
    Ok(candidates.pop())
}

pub(super) fn create_named_page(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let args: PageArgs = parse(args)?;
    name_valid(&args.name)?;
    let graph = whole_graph(slot)?;
    let Resolved::Absent { id } = graph.resolve(args.name.trim(), false) else {
        return Err(refused(
            "page-exists",
            None,
            "a page or alias already owns this name; nothing changed",
        ));
    };
    write::write_page_with_create(
        slot,
        &id,
        EditKind::CreatePage,
        args.rev.as_deref(),
        true,
        |doc, _| {
            Ok(Planned {
                doc: doc.clone(),
                changed: vec![],
            })
        },
    )
}

fn flush_window(slot: &GraphSlot, page: Option<&str>) -> Result<(), ErrorBody> {
    if let Some(window) = slot.window.get() {
        let result = match page {
            Some(name) => window.flush_page(name),
            None => window.flush_all(),
        };
        match result {
            super::window::Flushed::Saved => {}
            super::window::Flushed::Unsaved => {
                return Err(refused(
                    "unsaved",
                    None,
                    "the window has text it could not save; nothing changed",
                ))
            }
            super::window::Flushed::NoAnswer => {
                return Err(refused(
                    "page-busy",
                    None,
                    "the window did not save its typed text in time; nothing changed",
                ))
            }
        }
    }
    Ok(())
}

pub(super) fn delete_named_page(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    #[cfg(desktop)]
    let run = super::git_run::run_of(args)?;
    let args: PageArgs = parse(args)?;
    name_valid(&args.name)?;
    if !args.yes {
        return Err(refused(
            "confirmation",
            None,
            "page deletion needs --yes (or --force); nothing changed",
        ));
    }
    let page = unique_page(slot, &args.name)?;
    #[cfg(desktop)]
    if run.is_some() {
        super::git_run::check_page(slot, &page)?;
    }
    let canonical = slot
        .store
        .page(&page)
        .map_err(|e| store_refused(e, &page))?
        .doc
        .name;
    flush_window(slot, Some(&canonical))?;
    let read = crate::page_commands::with_host(slot, |host| {
        host.flush_input(std::slice::from_ref(&page))
            .map_err(|r| apply_refused(r, &page))?;
        host.read(&page).map_err(|e| match e {
            HostReadError::Ended => super::endpoint::busy(),
            HostReadError::Store(e) => store_refused(e, &page),
        })
    })
    .map_err(host_failed)??;
    let bytes = read
        .bytes
        .ok_or_else(|| store_refused(tine_store::StoreError::NotFound, &page))?;
    let parsed = slot
        .store
        .page_of(&page, &bytes)
        .map_err(|e| store_refused(e, &page))?;
    if parsed.read_only.is_some() || parsed.doc.read_only || parsed.doc.guide {
        return Err(refused(
            "read-only",
            Some(&page),
            "the page is read-only in Tine",
        ));
    }
    if let Some(rev) = args.rev {
        if String::from(parsed.rev.clone()) != rev {
            let mut body = ErrorBody::new("stale", "the page changed; read it again", false);
            body.detail = Some(json!({"admitted":false,"rev":String::from(parsed.rev)}));
            return Err(body);
        }
    }
    // Count linked page referrers and references to every deleted block id,
    // using the same removal counter as block delete and the app's notice.
    let graph = whole_graph(slot)?;
    let page_refs = graph
        .backlinks(&parsed.doc.name)
        .map_err(|e| refused("store", Some(&page), format!("{e:?}")))?
        .iter()
        .map(|group| group.blocks.len())
        .sum::<usize>();
    fn raws<'a>(
        blocks: &'a [tine_core::model::BlockDto],
        org: bool,
        out: &mut Vec<(&'a str, bool)>,
    ) {
        for block in blocks {
            out.push((&block.raw, org));
            raws(&block.children, org, out);
        }
    }
    let mut removed = Vec::new();
    raws(
        &parsed.doc.blocks,
        parsed.doc.format == Format::Org,
        &mut removed,
    );
    let broken = page_refs
        + tine_core::page_properties::refs_broken_by_removing(&graph.block_ref_counts(), &removed);
    let result =
        crate::page_commands::with_host(slot, |host| host.delete(None, &page, Some(&bytes)))
            .map_err(host_failed)?;
    match result {
        PageOperation::Applied => {}
        PageOperation::Waiting => {
            return Err(refused(
                "page-busy",
                Some(&page),
                "the page is busy; nothing changed",
            ))
        }
        PageOperation::Refused => {
            return Err(refused(
                "host",
                Some(&page),
                "the host refused the deletion; nothing changed",
            ))
        }
        PageOperation::Pending => {
            return Err(apply_refused(
                tine_store::ApplyRefusal::Unwritten(page.clone()),
                &page,
            ))
        }
        PageOperation::Superseded => {
            return Err(apply_refused(
                tine_store::ApplyRefusal::Superseded(page.clone()),
                &page,
            ))
        }
        PageOperation::Uncertain => {
            return Err(apply_refused(tine_store::ApplyRefusal::Uncertain, &page))
        }
    }
    if let Some(window) = slot.window.get() {
        window.removed(page.as_str(), &parsed.doc.name, parsed.doc.kind);
    }
    let mut reply = ok(
        "page-deleted",
        json!({"page":page.as_str(),"name":parsed.doc.name,"broken_refs":broken}),
    );
    #[cfg(desktop)]
    write::commit_if_in_run(slot, &page, Some(&bytes), None, &mut reply);
    Ok(reply)
}

#[derive(Deserialize)]
struct RenameArgs {
    old: String,
    new: String,
    #[serde(default)]
    rev: Option<String>,
    #[serde(default)]
    run: Option<String>,
}

pub(super) fn rename_named_page(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let args: RenameArgs = parse(args)?;
    name_valid(&args.old)?;
    name_valid(&args.new)?;
    // The app operation is graph-wide, including config and namespace moves.
    // Single-page run commits / revision guards cannot cover that operation.
    if args.rev.is_some() || args.run.as_ref().is_some_and(|s| !s.is_empty()) {
        return Err(refused(
            "rename-options",
            None,
            "page rename does not support --rev or --run; nothing changed",
        ));
    }
    let page = unique_source(slot, &args.old)?;
    flush_window(slot, None)?;
    let parsed = page
        .as_ref()
        .map(|page| slot.store.page(page).map_err(|e| store_refused(e, page)))
        .transpose()?;
    let old = parsed.as_ref().map_or_else(
        || args.old.trim().to_owned(),
        |parsed| parsed.doc.name.clone(),
    );
    let kind = parsed
        .as_ref()
        .map_or(tine_core::model::PageKind::Page, |parsed| parsed.doc.kind);
    let new = args.new.trim();
    // This inventory check proves an occupied destination refusal before
    // entering the shared operation. Errors from that operation alone do not
    // establish whether it already published files.
    let graph = whole_graph(slot)?;
    let occupied = match graph.resolve(new, false) {
        Resolved::Existing { id, others } => page.as_ref() != Some(&id) || !others.is_empty(),
        Resolved::Alias { owners } => owners.iter().any(|id| page.as_ref() != Some(id)),
        Resolved::Absent { .. } => false,
    };
    if occupied {
        return Err(refused(
            "page-exists",
            page.as_ref(),
            "a page or alias already owns the destination; nothing changed",
        ));
    }
    let path = page.as_ref().map(PageId::as_str);
    let notify = |report: &tine_graph_features::pages::RenameReport| {
        if let Some(window) = slot.window.get() {
            window.renamed(&old, new, path, kind, report);
        }
    };
    let report = crate::page_commands::with_host(slot, |host| {
        tine_graph_features::pages::rename_or_merge_page(
            &slot.store,
            host,
            &old,
            new,
            path,
            None,
            &[],
        )
    })
    .map_err(host_failed)?
    .map_err(|e| {
        let mut body = ErrorBody::new("uncertain", e.to_string(), false);
        if let Some(report) = tine_graph_features::pages::RenameReport::applied_before_error(&e) {
            notify(report);
            body.detail = Some(json!({"admitted":true,"page":path,"report":report}));
        } else {
            // Any I/O kind can also follow admission or incomplete rollback.
            body.detail = Some(json!({"admitted":null,"page":path}));
        }
        body
    })?;
    notify(&report);
    Ok(ok(
        "page-renamed",
        json!({"old":old,"name":new,"report":report}),
    ))
}

#[derive(Deserialize)]
struct RefArgs {
    handle: String,
    #[serde(default)]
    rev: Option<String>,
}

pub(super) fn block_reference(
    slot: &GraphSlot,
    args: &Value,
    embed: bool,
) -> Result<Value, ErrorBody> {
    let args: RefArgs = parse(args)?;
    let handle = write::parse_handle(&args.handle)?;
    let page = write::handle_page(slot, &handle)?;
    let mut uuid = String::new();
    let mut reply = write::write_page(
        slot,
        &page,
        EditKind::SaveBlock,
        args.rev.as_deref(),
        |doc, page| {
            let path = write::resolve_on(doc, page, &handle, &args.handle, args.rev.is_some())?;
            let mut next: PageDto = doc.clone();
            let block = write::block_at_mut(&mut next.blocks, &path).expect("resolved block");
            let org = doc.format == Format::Org;
            let existing = block_regions::parse(&block.raw, org)
                .id
                .map(|id| id.value.trim().to_lowercase());
            uuid = existing
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            super::verbs::check_uuid(&uuid)?;
            let owners = whole_graph(slot)?.block_pages(&uuid);
            if owners.len() > 1 {
                return Err(ErrorBody::new(
                    "ambiguous",
                    "several pages claim this block id",
                    false,
                ));
            }
            if existing.is_none() {
                block.raw = block_regions::edit(
                    &block.raw,
                    org,
                    Edit::Property {
                        key: "id".into(),
                        value: Some(uuid.clone()),
                    },
                )
                .map_err(|e| refused("kernel", Some(page), e))?;
            } else {
                write::resolve_on(doc, page, &write::parse_handle(&uuid)?, &uuid, true)?;
            }
            Ok(Planned {
                doc: next,
                changed: vec![path],
            })
        },
    )?;
    reply["kind"] = json!(if embed { "embed" } else { "ref" });
    reply["uuid"] = json!(uuid);
    reply["text"] = json!(if embed {
        format!("{{{{embed (({uuid}))}}}}")
    } else {
        format!("(({uuid}))")
    });
    Ok(reply)
}
