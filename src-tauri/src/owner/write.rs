//! The slice-2 write verbs (CLI v0 Spec §4 step 4; design
//! `tine-agents/specs/campaigns/2026-10-cli/SLICE2-DESIGN.md`). Every write
//! is one page-host operation planned from the host's read of the page
//! (what the window shows, after its typed text is saved), through the
//! store's own serializer, and refused before admission when the planned
//! bytes would not parse back to what the command meant.

use serde::Deserialize;
use serde_json::{json, Value};
use tine_core::editor_text::{set_text_with_policy, split, HidePolicy};
use tine_core::model::{BlockDto, Format, PageDto};
use tine_store::{
    ApplyRefusal, EditKind, HostExpected, HostReadError, Input, PageId, SaveBase, StoreError, Why,
};

use super::endpoint::busy;
use super::verbs::{internal, resolve_page, whole_graph, Diag};
use super::wire::{ok, ErrorBody};
use crate::state::GraphSlot;

/// The most handles a write reply lists; `more` counts the rest, so a reply
/// is bounded before admission (PROTOCOL §2.1).
pub(crate) const REPLY_HANDLES: usize = 200;

const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// A block's fingerprint (design §2): 6 lowercase base32 characters (30
/// bits) of FNV-1a 64 over its editor text, which collapsing or a lazy
/// `id::` does not change.
pub(crate) fn fingerprint(raw: &str, org: bool) -> String {
    let visible = split(raw, org, &HidePolicy::Builtin).visible;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in visible.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (0..6)
        .map(|i| BASE32[((hash >> (59 - 5 * i)) & 31) as usize] as char)
        .collect()
}

/// The 1-based dotted outline position of `path` (0-based child indexes).
pub(crate) fn position(path: &[usize]) -> String {
    path.iter()
        .map(|i| (i + 1).to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// `pos~fp` for the block at `path`.
pub(crate) fn handle_of(doc: &PageDto, path: &[usize]) -> Option<String> {
    let block = block_at(&doc.blocks, path)?;
    Some(format!(
        "{}~{}",
        position(path),
        fingerprint(&block.raw, is_org(doc))
    ))
}

/// Put `handle` on every block of a page payload's `blocks` (the JSON form
/// of a read), so a script reads the same handles the text output prints.
pub(crate) fn annotate_handles(blocks: &mut Value, org: bool, prefix: &str) {
    let Some(list) = blocks.as_array_mut() else {
        return;
    };
    for (index, block) in list.iter_mut().enumerate() {
        let address = format!("{prefix}{}", index + 1);
        let Some(fields) = block.as_object_mut() else {
            continue;
        };
        let raw = fields.get("raw").and_then(Value::as_str).unwrap_or("");
        let handle = format!("{address}~{}", fingerprint(raw, org));
        fields.insert("handle".into(), json!(handle));
        if let Some(children) = fields.get_mut("children") {
            annotate_handles(children, org, &format!("{address}."));
        }
    }
}

fn is_org(doc: &PageDto) -> bool {
    doc.format == Format::Org
}

pub(crate) fn block_at<'a>(blocks: &'a [BlockDto], path: &[usize]) -> Option<&'a BlockDto> {
    let (first, rest) = path.split_first()?;
    let block = blocks.get(*first)?;
    if rest.is_empty() {
        Some(block)
    } else {
        block_at(&block.children, rest)
    }
}

pub(super) fn block_at_mut<'a>(
    blocks: &'a mut [BlockDto],
    path: &[usize],
) -> Option<&'a mut BlockDto> {
    let (first, rest) = path.split_first()?;
    let block = blocks.get_mut(*first)?;
    if rest.is_empty() {
        Some(block)
    } else {
        block_at_mut(&mut block.children, rest)
    }
}

fn walk(blocks: &[BlockDto], prefix: &mut Vec<usize>, visit: &mut impl FnMut(&[usize], &BlockDto)) {
    for (index, block) in blocks.iter().enumerate() {
        prefix.push(index);
        visit(prefix, block);
        walk(&block.children, prefix, visit);
        prefix.pop();
    }
}

/// A block address as a write names it (design §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Handle {
    /// A persisted `id::` UUID, graph-wide.
    Uuid(String),
    /// `page/pos~fp`, or `page/pos` (only with `--rev`).
    At {
        page: String,
        pos: Vec<usize>,
        fp: Option<String>,
    },
}

fn bad_handle(text: &str) -> ErrorBody {
    ErrorBody::new(
        "bad-args",
        format!("`{text}` is not a block handle (use an id:: UUID or PAGE/POS~FP)"),
        false,
    )
}

/// Parse a handle. The page is everything before the last `/`, since page
/// names hold `/` (namespaces).
pub(crate) fn parse_handle(text: &str) -> Result<Handle, ErrorBody> {
    let text = text.trim();
    if super::verbs::check_uuid(text).is_ok() {
        return Ok(Handle::Uuid(text.to_ascii_lowercase()));
    }
    let (page, address) = text.rsplit_once('/').ok_or_else(|| bad_handle(text))?;
    let (pos, fp) = match address.split_once('~') {
        Some((pos, fp)) => (pos, Some(fp.to_ascii_lowercase())),
        None => (address, None),
    };
    let pos: Vec<usize> = pos
        .split('.')
        .map(|part| {
            part.parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .map(|n| n - 1)
        })
        .collect::<Option<_>>()
        .ok_or_else(|| bad_handle(text))?;
    let fp_ok = fp.as_deref().map_or(true, |fp| {
        fp.len() == 6 && fp.bytes().all(|b| BASE32.contains(&b))
    });
    if page.trim().is_empty() || pos.is_empty() || !fp_ok {
        return Err(bad_handle(text));
    }
    Ok(Handle::At {
        page: page.to_string(),
        pos,
        fp,
    })
}

fn stale(page: &PageId, message: impl Into<String>, extra: Value) -> ErrorBody {
    let mut body = ErrorBody::new("stale", message, false);
    let mut detail = json!({ "admitted": false, "page": page.as_str() });
    if let (Some(detail), Some(extra)) = (detail.as_object_mut(), extra.as_object()) {
        detail.extend(extra.clone());
    }
    body.detail = Some(detail);
    body
}

pub(super) fn refused(
    reason: &str,
    page: Option<&PageId>,
    message: impl Into<String>,
) -> ErrorBody {
    let mut body = ErrorBody::new("refused", message, false);
    body.detail = Some(json!({
        "admitted": false,
        "reason": reason,
        "page": page.map(PageId::as_str),
    }));
    body
}

/// Resolve the block a handle names on `doc` (the host's current read).
/// A fingerprint resolves only when exactly one block on the page carries
/// it (design §2, review round 1 finding 1): position-first over a shared
/// fingerprint could pick the wrong block.
pub(crate) fn resolve_on(
    doc: &PageDto,
    page: &PageId,
    handle: &Handle,
    text: &str,
    rev_checked: bool,
) -> Result<Vec<usize>, ErrorBody> {
    let org = is_org(doc);
    match handle {
        Handle::Uuid(uuid) => {
            let mut found = Vec::new();
            walk(&doc.blocks, &mut Vec::new(), &mut |path, block| {
                let regions = tine_core::block_regions::parse(&block.raw, org);
                if regions
                    .id
                    .as_ref()
                    .is_some_and(|id| id.value.trim().eq_ignore_ascii_case(uuid))
                {
                    found.push(path.to_vec());
                }
            });
            match found.len() {
                1 => Ok(found.remove(0)),
                0 => Err(stale(
                    page,
                    format!("no block on the page has id:: {uuid}"),
                    json!({ "handle": text }),
                )),
                _ => Err(stale(
                    page,
                    format!("{} blocks on the page have id:: {uuid}", found.len()),
                    json!({
                        "handle": text,
                        "candidates": found.iter().filter_map(|p| handle_of(doc, p)).collect::<Vec<_>>(),
                    }),
                )),
            }
        }
        Handle::At { pos, fp: None, .. } => {
            if !rev_checked {
                return Err(ErrorBody::new(
                    "bad-args",
                    format!("`{text}` has no fingerprint; give POS~FP, or pass --rev"),
                    false,
                ));
            }
            block_at(&doc.blocks, pos)
                .map(|_| pos.clone())
                .ok_or_else(|| {
                    stale(
                        page,
                        format!("the page has no block {}", position(pos)),
                        json!({ "handle": text }),
                    )
                })
        }
        Handle::At { fp: Some(fp), .. } => {
            let mut found = Vec::new();
            walk(&doc.blocks, &mut Vec::new(), &mut |path, block| {
                if fingerprint(&block.raw, org) == *fp {
                    found.push(path.to_vec());
                }
            });
            match found.len() {
                1 => Ok(found.remove(0)),
                0 => Err(stale(
                    page,
                    format!("no block on the page matches `{text}` any more; read the page again"),
                    json!({ "handle": text }),
                )),
                _ => Err(stale(
                    page,
                    format!(
                        "{} blocks on the page match `{text}`; address one by its id:: or by position with --rev",
                        found.len()
                    ),
                    json!({
                        "handle": text,
                        "candidates": found.iter().filter_map(|p| handle_of(doc, p)).collect::<Vec<_>>(),
                    }),
                )),
            }
        }
    }
}

/// The page a handle writes: its own page, or the page holding the `id::`.
pub(super) fn handle_page(slot: &GraphSlot, handle: &Handle) -> Result<PageId, ErrorBody> {
    let graph = whole_graph(slot)?;
    match handle {
        Handle::At { page, .. } => resolve_page(&graph, page),
        // Graph-wide: an id on two pages is `ambiguous`, never the first match
        // (review 5 B1; design §2).
        Handle::Uuid(uuid) => match graph.block_pages(uuid).as_slice() {
            [page] => resolve_page(&graph, page),
            [] => Err(ErrorBody::new(
                "not-found",
                format!("no block has id:: {uuid}"),
                true,
            )),
            pages => {
                let mut body = ErrorBody::new(
                    "ambiguous",
                    format!("blocks on several pages have id:: {uuid}; fix the duplicate id first"),
                    false,
                );
                body.detail =
                    Some(json!({ "admitted": false, "handle": uuid, "candidates": pages }));
                Err(body)
            }
        },
    }
}

/// What a computation hands the write path: the page as the command means
/// it, and the paths of the blocks it changed (for the reply's handles).
pub(crate) struct Planned {
    pub(crate) doc: PageDto,
    pub(crate) changed: Vec<Vec<usize>>,
}

/// A host refusal as the protocol's code (design §1 table).
pub(super) fn apply_refused(refusal: ApplyRefusal, page: &PageId) -> ErrorBody {
    let with_page = |code: &str, message: &str, admitted: Value| {
        let mut body = ErrorBody::new(code, message, false);
        body.detail = Some(json!({ "admitted": admitted, "page": page.as_str() }));
        body
    };
    match refusal {
        ApplyRefusal::Changed => stale(page, "the page changed while the command ran; read it again", json!({})),
        ApplyRefusal::Unsaved(_) => refused(
            "unsaved",
            Some(page),
            "the page has typed changes Tine could not save; save or discard them in the app, then try again",
        ),
        ApplyRefusal::Busy => refused("page-busy", Some(page), "other work held the page; try again"),
        ApplyRefusal::Refused => refused("host", Some(page), "the page host refused the write; nothing changed"),
        ApplyRefusal::DraftFailed => with_page(
            "draft-failed",
            "the write's draft could not be saved (disk full or an app-data error); nothing changed",
            json!(true),
        ),
        ApplyRefusal::Unwritten(_) => with_page(
            "unwritten",
            "the write was accepted but the file could not be written yet; Tine keeps trying",
            json!(true),
        ),
        ApplyRefusal::Superseded(_) => with_page(
            "superseded",
            "the write applied, then a later edit replaced it before it was saved",
            json!(true),
        ),
        ApplyRefusal::SpellingIncomplete => with_page(
            "uncertain",
            "the file name may already be changed; the text change is not confirmed. Save or discard pending edits, then rename again to the intended name. If the page already shows that name, check its text before retrying",
            Value::Null,
        ),
        ApplyRefusal::Uncertain => {
            let mut body = ErrorBody::new(
                "uncertain",
                "no answer from the page host in time; the write may still complete. Read the page before retrying",
                false,
            );
            body.detail = Some(json!({ "admitted": Value::Null, "page": page.as_str() }));
            body
        }
    }
}

pub(super) fn outline_refused(
    reason: tine_core::outline_kernel::Reason,
    page: &PageId,
    verb: &str,
) -> ErrorBody {
    let name = serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    refused(
        &format!("kernel:{name}"),
        Some(page),
        format!("the outline refused the {verb}: {reason:?}"),
    )
}

fn host_failed(message: String) -> ErrorBody {
    if message == crate::state::STALE_BINDING {
        busy()
    } else {
        refused("no-host", None, message)
    }
}

/// Compare the tree the command meant with the tree its bytes parse to:
/// shape, and each block's raw up to trailing whitespace (design §1 4b).
fn same_tree(meant: &[BlockDto], parsed: &[BlockDto]) -> bool {
    meant.len() == parsed.len()
        && meant.iter().zip(parsed).all(|(a, b)| {
            a.raw.trim_end() == b.raw.trim_end() && same_tree(&a.children, &b.children)
        })
}

/// One write: flush, read through the host, compute, plan through the
/// store's serializer, reparse, apply (design §1). A page that changed
/// between the read and the operation is read and resolved once more; a
/// second change is `stale`.
pub(crate) fn write_page(
    slot: &GraphSlot,
    page: &PageId,
    kind: EditKind,
    rev: Option<&str>,
    compute: impl FnMut(&PageDto, &PageId) -> Result<Planned, ErrorBody>,
) -> Result<Value, ErrorBody> {
    write_page_with_create(slot, page, kind, rev, false, compute)
}

pub(super) fn write_page_with_create(
    slot: &GraphSlot,
    page: &PageId,
    kind: EditKind,
    rev: Option<&str>,
    create: bool,
    mut compute: impl FnMut(&PageDto, &PageId) -> Result<Planned, ErrorBody>,
) -> Result<Value, ErrorBody> {
    write_page_from_bytes(slot, page, kind, rev, create, |doc, page, _| {
        compute(doc, page)
    })
}

#[cfg(desktop)]
thread_local! {
    /// The git run the current write verb belongs to (set by the verb
    /// dispatcher for the duration of one verb), with the verb's commit subject.
    static RUN: std::cell::RefCell<Option<(super::git_run::RunCtx, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `verb` with its writes committed to the graph's git repository as
/// part of `run` (CLI slice 3, D-20261019-CL1).
#[cfg(desktop)]
pub(super) fn in_run<T>(
    run: Option<super::git_run::RunCtx>,
    subject: &str,
    verb: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            RUN.with(|r| r.borrow_mut().take());
        }
    }
    RUN.with(|r| *r.borrow_mut() = run.map(|run| (run, subject.to_string())));
    let _reset = Reset;
    verb()
}

/// Commit a successful write when it belongs to a run. A commit that fails
/// does not undo the saved write: the reply says so under `git`.
#[cfg(desktop)]
fn commit_if_in_run(
    slot: &GraphSlot,
    page: &PageId,
    old: Option<&[u8]>,
    new: &[u8],
    reply: &mut Value,
) {
    let Some((run, subject)) = RUN.with(|r| r.borrow().clone()) else {
        return;
    };
    let git = match super::git_run::commit_write(slot, &run, page, &subject, old, Some(new)) {
        Ok(done) => done.to_json(),
        Err(error) => json!({
            "committed": false,
            "error": super::git_run::reason_of(&error),
            "message": error.message,
        }),
    };
    if let Some(reply) = reply.as_object_mut() {
        reply.insert("git".into(), git);
    }
}

/// [`write_page_with_create`] whose `compute` also sees the page's current
/// bytes (empty for a page being created).
pub(super) fn write_page_from_bytes(
    slot: &GraphSlot,
    page: &PageId,
    kind: EditKind,
    rev: Option<&str>,
    create: bool,
    mut compute: impl FnMut(&PageDto, &PageId, &[u8]) -> Result<Planned, ErrorBody>,
) -> Result<Value, ErrorBody> {
    #[cfg(desktop)]
    if RUN.with(|r| r.borrow().is_some()) {
        super::git_run::check_page(slot, page)?;
    }
    let flush = || {
        crate::page_commands::with_host(slot, |host| host.flush_input(std::slice::from_ref(page)))
            .map_err(host_failed)?
            .map_err(|refusal| apply_refused(refusal, page))
    };
    flush()?;
    // A window bound to the graph holds typed text the host has not seen
    // (it debounces typing): it saves that first, then the page is read
    // again (owner::window). Once per write.
    let mut window = slot.window.get();
    let mut attempts = 0;
    while attempts < 2 {
        let read = crate::page_commands::with_host(slot, |host| host.read(page))
            .map_err(host_failed)?
            .map_err(|error| match error {
                HostReadError::Ended => busy(),
                HostReadError::Store(error) => store_refused(error, page),
            })?;
        if read.bytes.is_none() && !create {
            return Err(ErrorBody::new(
                "not-found",
                format!("{} does not exist", page.as_str()),
                false,
            ));
        }
        let absent = read.bytes.is_none();
        let bytes = read.bytes.unwrap_or_else(|| std::sync::Arc::from(&b""[..]));
        let current = slot
            .store
            .page_of(&read.page, &bytes)
            .map_err(|error| store_refused(error, page))?;
        if let Some(link) = window.take() {
            match link.flush_page(&current.doc.name) {
                super::window::Flushed::Saved => {}
                super::window::Flushed::Unsaved => {
                    return Err(refused(
                        "unsaved",
                        Some(&read.page),
                        "the page has typed changes Tine could not save; save or discard them in the app, then try again",
                    ))
                }
                super::window::Flushed::NoAnswer => {
                    return Err(refused(
                        "page-busy",
                        Some(&read.page),
                        "the window holding the page did not save its typed text in time; try again",
                    ))
                }
            }
            flush()?;
            continue;
        }
        attempts += 1;
        if current.read_only.is_some() || current.doc.read_only || current.doc.guide {
            return Err(refused(
                "read-only",
                Some(&read.page),
                format!(
                    "{} is read-only in Tine: {}",
                    read.page.as_str(),
                    current
                        .read_only
                        .as_deref()
                        .unwrap_or("it cannot round-trip safely")
                ),
            ));
        }
        if let Some(rev) = rev {
            if absent || String::from(current.rev.clone()) != rev {
                return Err(stale(
                    &read.page,
                    format!("the page is no longer at rev {rev}; read it again"),
                    json!({ "rev": String::from(current.rev.clone()) }),
                ));
            }
        }
        let planned = compute(&current.doc, &read.page, &bytes)?;
        let mut tx = slot.store.transaction(Some(kind));
        tx.save_page(
            &[kind],
            &read.page,
            if absent {
                SaveBase::CreateNew
            } else {
                SaveBase::Existing(current.rev.clone())
            },
            &planned.doc,
        );
        let mut op = match tx.plan() {
            Ok(op) => op,
            Err(Why::Conflict { .. }) => continue,
            Err(Why::Refused(refusal)) => {
                return Err(refused("store", Some(&read.page), format!("{refusal:?}")))
            }
            Err(Why::Failed(error)) => {
                return Err(internal(Diag::WriteFailed, format!("{error:?}")))
            }
        };
        // The store planned from its own read; the host's must be the same
        // bytes, or the page moved in between (PROTOCOL §5).
        let planned_from_host = op.pages.iter().all(|(_, _, expected)| match expected {
            HostExpected::Absent => absent,
            HostExpected::Bytes(old) => !absent && **old == *bytes,
        });
        if !planned_from_host {
            continue;
        }
        let Some(new_bytes) = op.pages.iter().find_map(|(_, effect, _)| match effect {
            tine_store::HostEffect::Write(new) => Some(new.clone()),
            tine_store::HostEffect::Delete => None,
        }) else {
            return Err(internal(Diag::WriteFailed, "a page write planned no bytes"));
        };
        let reparsed = slot
            .store
            .page_of(&read.page, &new_bytes)
            .map_err(|error| store_refused(error, page))?;
        if !same_tree(&planned.doc.blocks, &reparsed.doc.blocks)
            || planned.doc.pre_block.as_deref().map(str::trim_end)
                != reparsed.doc.pre_block.as_deref().map(str::trim_end)
        {
            return Err(refused(
                "does-not-round-trip",
                Some(&read.page),
                "the text would not read back as written (an unclosed fence, or a line that becomes a new block); nothing changed",
            ));
        }
        op.policy = Input::Flush;
        match crate::page_commands::with_host(slot, |host| host.apply(op)).map_err(host_failed)? {
            Ok(()) => {
                // A window that displays the page without holding it gets no
                // page mail: tell it as of an outside edit (owner::window).
                if let Some(link) = slot.window.get() {
                    link.changed(
                        read.page.as_str(),
                        &reparsed.doc.name,
                        reparsed.doc.kind,
                        absent,
                    );
                }
                #[allow(unused_mut)]
                let mut reply = written(&read.page, &reparsed.doc, &reparsed.rev, &planned.changed);
                #[cfg(desktop)]
                commit_if_in_run(
                    slot,
                    &read.page,
                    (!absent).then_some(&bytes[..]),
                    &new_bytes,
                    &mut reply,
                );
                return Ok(reply);
            }
            Err(ApplyRefusal::Changed) => continue,
            Err(refusal) => return Err(apply_refused(refusal, &read.page)),
        }
    }
    Err(stale(
        page,
        "the page kept changing while the command ran; read it again",
        json!({}),
    ))
}

fn written(
    page: &PageId,
    doc: &PageDto,
    rev: &tine_store::FileRev,
    changed: &[Vec<usize>],
) -> Value {
    // Whole handles (`page/pos~fp`), so an agent chains them as printed.
    let handles: Vec<String> = changed
        .iter()
        .take(REPLY_HANDLES)
        .filter_map(|path| handle_of(doc, path))
        .map(|handle| format!("{}/{handle}", doc.name))
        .collect();
    ok(
        "written",
        json!({
            "page": page.as_str(),
            "name": doc.name,
            "rev": String::from(rev.clone()),
            "blocks": handles,
            "more": changed.len().saturating_sub(REPLY_HANDLES),
        }),
    )
}

pub(super) fn store_refused(error: StoreError, page: &PageId) -> ErrorBody {
    match error {
        StoreError::TooLarge { limit, len } => ErrorBody::new(
            "too-large",
            format!("the page is {len} bytes, over the {limit}-byte parse limit"),
            false,
        ),
        StoreError::NotFound => ErrorBody::new(
            "not-found",
            format!("{} does not exist", page.as_str()),
            false,
        ),
        StoreError::Closed => busy(),
        other => internal(Diag::StoreFailed, format!("{other:?}")),
    }
}

#[derive(Deserialize)]
struct EditArgs {
    handle: String,
    text: String,
    #[serde(default)]
    rev: Option<String>,
}

#[derive(Deserialize)]
struct MoveArgs {
    handle: String,
    /// The block the moved one lands relative to.
    to: String,
    /// `under` (last child), `after` or `before`.
    at: String,
    #[serde(default)]
    rev: Option<String>,
}

#[derive(Deserialize)]
struct PropArgs {
    handle: String,
    key: String,
    /// `None` removes the property.
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    rev: Option<String>,
}

/// The local time as the logbook writes it.
fn now_parts() -> tine_core::logbook::TimestampParts {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    tine_core::logbook::TimestampParts {
        year: now.year(),
        month: now.month(),
        day: now.day(),
        weekday: now.weekday().num_days_from_sunday(),
        hour: now.hour(),
        minute: now.minute(),
        second: now.second(),
    }
}

fn kernel_refused(page: &PageId, message: String) -> ErrorBody {
    refused("kernel", Some(page), message)
}

/// `block.edit`: the text replaces the block's editor text, as typing it
/// would (hidden properties kept, children kept), then the marker
/// transition the window's `setRaw` runs.
pub(crate) fn edit_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let EditArgs {
        handle: text,
        text: new_text,
        rev,
    } = super::verbs::parse(args)?;
    let handle = parse_handle(&text)?;
    let page = handle_page(slot, &handle)?;
    let config = slot.store.config();
    write_page(
        slot,
        &page,
        EditKind::SaveBlock,
        rev.as_deref(),
        |doc, page| {
            let path = resolve_on(doc, page, &handle, &text, rev.is_some())?;
            let org = is_org(doc);
            let mut next = doc.clone();
            let block = block_at_mut(&mut next.blocks, &path).expect("resolved path");
            let old = block.raw.clone();
            // Raw CLI reads may be sent back verbatim. A copied buffer from
            // another block must not silently replace this block's identity.
            // Use accepted properties so fenced examples remain plain text.
            let current = tine_core::block_regions::parse(&old, org);
            if let Some(id) = current.id {
                let submitted = tine_core::block_regions::parse(&new_text, org);
                if let Some(other) = submitted.properties.iter().find(|p| {
                    p.primary
                        && p.key.eq_ignore_ascii_case("id")
                        && !p.value.trim().eq_ignore_ascii_case(id.value.trim())
                }) {
                    return Err(refused(
                        "identity-change",
                        Some(page),
                        format!(
                            "block.edit refuses to replace existing id {} with supplied id {}",
                            id.value, other.value
                        ),
                    ));
                }
            }
            let hide = HidePolicy::Configured {
                keys: config.block_hidden_properties.clone(),
                sheet_cell: false,
            };
            let joined = set_text_with_policy(&old, org, &new_text, &hide)
                .map_err(|e| kernel_refused(page, e))?;
            block.raw = tine_core::logbook::apply_edit_transition_at(
                &old,
                &joined,
                if org {
                    tine_core::logbook::LogbookFormat::Org
                } else {
                    tine_core::logbook::LogbookFormat::Markdown
                },
                config.enable_timetracking,
                config.logbook.with_second_support,
                now_parts(),
            );
            Ok(Planned {
                doc: next,
                changed: vec![path],
            })
        },
    )
}

/// `block.prop`: set or remove one property through `block_regions::edit`,
/// the window's `setBlockProperty`.
pub(crate) fn prop_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let PropArgs {
        handle: text,
        key,
        value,
        rev,
    } = super::verbs::parse(args)?;
    let handle = parse_handle(&text)?;
    let page = handle_page(slot, &handle)?;
    write_page(
        slot,
        &page,
        EditKind::SaveBlock,
        rev.as_deref(),
        |doc, page| {
            let path = resolve_on(doc, page, &handle, &text, rev.is_some())?;
            let org = is_org(doc);
            let mut next = doc.clone();
            let block = block_at_mut(&mut next.blocks, &path).expect("resolved path");
            block.raw = tine_core::block_regions::edit(
                &block.raw,
                org,
                tine_core::block_regions::Edit::Property {
                    key: key.clone(),
                    value: value.clone(),
                },
            )
            .map_err(|e| kernel_refused(page, e))?;
            Ok(Planned {
                doc: next,
                changed: vec![path],
            })
        },
    )
}

/// `block.move`: the kernel's `MoveTo` (the window's drag and drop), with
/// its subtree. Both blocks on one page; a move across pages is refused
/// until it is one operation over both pages (design §3).
pub(crate) fn move_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    use tine_core::outline_kernel::{apply, Place, Verb};
    let MoveArgs {
        handle: text,
        to: to_text,
        at,
        rev,
    } = super::verbs::parse(args)?;
    let handle = parse_handle(&text)?;
    let target = parse_handle(&to_text)?;
    let page = handle_page(slot, &handle)?;
    if handle_page(slot, &target)? != page {
        return Err(refused(
            "cross-page",
            Some(&page),
            "moving a block to another page is not supported yet; both blocks must be on one page",
        ));
    }
    if !matches!(at.as_str(), "under" | "after" | "before") {
        return Err(ErrorBody::new(
            "bad-args",
            format!("`{at}` is not a place (use under, after or before)"),
            false,
        ));
    }
    write_page(
        slot,
        &page,
        EditKind::MoveBlocks,
        rev.as_deref(),
        |doc, page| {
            let from = resolve_on(doc, page, &handle, &text, rev.is_some())?;
            let to = resolve_on(doc, page, &target, &to_text, rev.is_some())?;
            let arena = super::arena::Arena::of(doc);
            let key = arena.key_at(&from).expect("resolved path");
            let target = arena.key_at(&to).expect("resolved path");
            let place = match at.as_str() {
                "under" => Place::LastChild {
                    key: target.clone(),
                },
                "after" => Place::After {
                    key: target.clone(),
                },
                _ => Place::Before {
                    key: target.clone(),
                },
            };
            let verb = Verb::MoveTo {
                keys: vec![key.clone()],
                to: place,
                inherit_from: Some(target),
            };
            let change = apply(
                std::slice::from_ref(&arena.page),
                &verb,
                &Default::default(),
            )
            .map_err(|reason| outline_refused(reason, page, "move"))?;
            let (next, changed) = arena.apply(doc, &change, &[key]);
            Ok(Planned { doc: next, changed })
        },
    )
}

#[derive(Deserialize)]
struct DeleteArgs {
    handle: String,
    #[serde(default)]
    rev: Option<String>,
}

/// `delete <handle>`: the block and its subtree, with the number of
/// references the deletion leaves broken (the app's notice rule).
pub(crate) fn delete_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    use tine_core::outline_kernel::{apply, Verb};
    let DeleteArgs { handle: text, rev } = super::verbs::parse(args)?;
    let handle = parse_handle(&text)?;
    let page = handle_page(slot, &handle)?;
    let counts = whole_graph(slot)?.block_ref_counts();
    let mut deleted = (String::new(), 0usize);
    let mut reply = write_page(
        slot,
        &page,
        EditKind::DeleteBlocks,
        rev.as_deref(),
        |doc, page| {
            let path = resolve_on(doc, page, &handle, &text, rev.is_some())?;
            let org = is_org(doc);
            let root = block_at(&doc.blocks, &path).expect("resolved path");
            let mut raws: Vec<String> = Vec::new();
            walk(
                std::slice::from_ref(root),
                &mut Vec::new(),
                &mut |_, block| raws.push(block.raw.clone()),
            );
            let removed: Vec<(&str, bool)> = raws.iter().map(|raw| (raw.as_str(), org)).collect();
            deleted = (
                format!(
                    "{}/{}",
                    doc.name,
                    handle_of(doc, &path).expect("resolved path")
                ),
                tine_core::page_properties::refs_broken_by_removing(&counts, &removed),
            );
            let arena = super::arena::Arena::of(doc);
            let key = arena.key_at(&path).expect("resolved path");
            let change = apply(
                std::slice::from_ref(&arena.page),
                &Verb::Delete {
                    keys: vec![key.clone()],
                },
                &Default::default(),
            )
            .map_err(|reason| outline_refused(reason, page, "delete"))?;
            let (next, changed) = arena.apply(doc, &change, &[key]);
            Ok(Planned { doc: next, changed })
        },
    )?;
    reply["kind"] = json!("deleted");
    reply["deleted"] = json!(deleted.0);
    reply["broken_refs"] = json!(deleted.1);
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_parse_page_names_with_slashes() {
        assert_eq!(
            parse_handle("Projects/Tine/3.1~abc234").unwrap(),
            Handle::At {
                page: "Projects/Tine".into(),
                pos: vec![2, 0],
                fp: Some("abc234".into()),
            }
        );
        assert_eq!(
            parse_handle("6512BD43-D9CA-4C95-B0B4-1C2D3E4F5A6B").unwrap(),
            Handle::Uuid("6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b".into())
        );
        for bad in [
            "nopage",
            "P/0",
            "P/1~abc",
            "P/1~ABC1!x",
            "/1~abcdef",
            "P/1..2",
        ] {
            assert!(parse_handle(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_fingerprint_ignores_hidden_properties() {
        let plain = fingerprint("TODO write", false);
        assert_eq!(plain.len(), 6);
        assert_eq!(
            fingerprint(
                "TODO write\ncollapsed:: true\nid:: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b",
                false
            ),
            plain
        );
        assert_ne!(fingerprint("TODO writes", false), plain);
        for org in [false, true] {
            assert_eq!(
                fingerprint(
                    "TODO write\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00]\n:END:",
                    org
                ),
                plain
            );
        }
    }
}
