//! The slice-2 write verbs (CLI v0 Spec §4 step 4; design
//! `tine-agents/specs/campaigns/2026-10-cli/SLICE2-DESIGN.md`). Every write
//! is one page-host operation planned from the host's read of the page
//! (what the window shows, after its typed text is saved), through the
//! store's own serializer, and refused before admission when the planned
//! bytes would not parse back to what the command meant.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tine_core::editor_text::{set_text_with_policy, split, HidePolicy};
use tine_core::model::{BlockDto, Format, PageDto};
use tine_store::{
    ApplyRefusal, EditKind, HostExpected, HostReadError, Input, PageId, SaveBase, StoreError, Why,
};

use super::endpoint::busy;
use super::verbs::{internal, resolve_write_page, whole_graph, Diag};
use super::wire::{ok, ErrorBody};
use crate::state::GraphSlot;

/// The most handles a write reply lists; `more` counts the rest, so a reply
/// is bounded before admission (PROTOCOL §2.1).
pub(crate) const REPLY_HANDLES: usize = 200;

const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// The name a CLI write stamps on blocks or commits: explicit name, then
/// TINE_AUTHOR, then `agent`. Explicit blanks refuse; blank environment values
/// count as unset. O(name bytes), with no graph reads, writes, stats or IPC.
pub(crate) fn author_name(explicit: Option<&str>) -> Result<String, String> {
    if let Some(name) = explicit {
        let name = name.trim();
        if name.is_empty() {
            return Err(
                "--author needs a name; `tine prop remove BLOCK tine.author` removes one".into(),
            );
        }
        return Ok(name.into());
    }
    Ok(std::env::var("TINE_AUTHOR")
        .ok()
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("agent")
        .into())
}

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
/// A page name several files claim refuses (`verbs::resolve_write_page`,
/// CLI-DUPNAME), also when an `id::` names a block on such a page.
pub(super) fn handle_page(slot: &GraphSlot, handle: &Handle) -> Result<PageId, ErrorBody> {
    let graph = whole_graph(slot)?;
    match handle {
        Handle::At { page, .. } => resolve_write_page(&graph, page),
        // Graph-wide: an id on two pages is `ambiguous`, never the first match
        // (review 5 B1; design §2).
        Handle::Uuid(uuid) => match graph.block_pages(uuid).as_slice() {
            [page] => resolve_write_page(&graph, page),
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

pub(super) fn host_failed(message: String) -> ErrorBody {
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

/// How long a write waits in line before it refuses, untouched. In-scope
/// scenario: concurrent honest writers behind one wedged or very slow write (a
/// window that does not answer, a stalled disk). Below the client's reply wait
/// (`client::REPLY_WAIT`, pinned there), so a write that spends its wait only here
/// reaches its caller as this refusal. Time spent elsewhere is not bounded by it:
/// a `--run` write first waits for `RUN_LOCK`, and a write that gets its turn late
/// may still outlast the client, which then says `uncertain`.
pub(crate) const TURN_WAIT: Duration = Duration::from_secs(45);

/// What a write verb reaches: one page, or (`page rename`, which rewrites every
/// referrer) any page of the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnScope {
    Page(PageId),
    Graph,
}

impl TurnScope {
    fn overlaps(&self, other: &TurnScope) -> bool {
        match (self, other) {
            (TurnScope::Page(a), TurnScope::Page(b)) => a == b,
            _ => true,
        }
    }
}

/// The owner's writers per graph, in arrival order (GH #842). Each write plans
/// against the bytes it read, so writers that overlap on a page would all plan
/// from one base and all but one would lose: taking turns means commands never
/// race each other, and what still moves a page under a write (typing, an
/// editor, Syncthing) meets the host's guard as before. A writer goes once no
/// earlier one overlaps it. A line rather than a mutex, whose wakeups favour no
/// one, so a writer in it cannot be starved by later ones. Keyed by graph, so slots of
/// one graph share it across a host adoption, as [`super::git_run::RUN_LOCK`]
/// is shared.
///
/// Lock order: `RUN_LOCK`, then a turn, then the page host. Nothing holding a
/// turn takes `RUN_LOCK`, and a verb holds one turn at a time. So a `--run` write
/// joins the line only once it holds `RUN_LOCK`, whose own order is unfair, and
/// other `RUN_LOCK` users (autocommit, sync, app page operations) wait while it
/// stands in line.
pub(super) struct PageTurns {
    lines: Mutex<HashMap<PathBuf, VecDeque<(u64, TurnScope)>>>,
    moved: Condvar,
    next: AtomicU64,
}

static PAGE_TURNS: LazyLock<PageTurns> = LazyLock::new(PageTurns::new);

/// A verb's turn; dropping it, panics included, passes the turn on.
pub(super) struct PageTurn<'a> {
    turns: &'a PageTurns,
    graph: PathBuf,
    ticket: u64,
}

impl PageTurns {
    fn new() -> Self {
        Self {
            lines: Mutex::new(HashMap::new()),
            moved: Condvar::new(),
            next: AtomicU64::new(0),
        }
    }

    /// Wait until no earlier writer overlaps `scope`; `None` past `wait`, out
    /// of the line.
    fn wait_turn(&self, graph: PathBuf, scope: TurnScope, wait: Duration) -> Option<PageTurn<'_>> {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + wait;
        let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        lines
            .entry(graph.clone())
            .or_default()
            .push_back((ticket, scope.clone()));
        while lines[&graph]
            .iter()
            .take_while(|(earlier, _)| *earlier != ticket)
            .any(|(_, earlier)| earlier.overlaps(&scope))
        {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.leave_line(&mut lines, &graph, ticket);
                return None;
            }
            lines = self
                .moved
                .wait_timeout(lines, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        Some(PageTurn {
            turns: self,
            graph,
            ticket,
        })
    }

    fn leave_line(
        &self,
        lines: &mut HashMap<PathBuf, VecDeque<(u64, TurnScope)>>,
        graph: &PathBuf,
        ticket: u64,
    ) {
        if let Some(line) = lines.get_mut(graph) {
            line.retain(|(t, _)| *t != ticket);
            if line.is_empty() {
                lines.remove(graph);
            }
        }
        self.moved.notify_all();
    }
}

impl Drop for PageTurn<'_> {
    fn drop(&mut self) {
        let mut lines = self.turns.lines.lock().unwrap_or_else(|e| e.into_inner());
        self.turns.leave_line(&mut lines, &self.graph, self.ticket);
    }
}

/// How many verbs stand in `slot`'s graph's line, the holders included.
#[cfg(test)]
pub(super) fn tests_line_len(slot: &GraphSlot) -> usize {
    PAGE_TURNS
        .lines
        .lock()
        .unwrap()
        .get(&slot.root_key)
        .map_or(0, VecDeque::len)
}

/// `page`'s turn for one write verb, held across its read, save and commit.
pub(super) fn page_turn(slot: &GraphSlot, page: &PageId) -> Result<PageTurn<'static>, ErrorBody> {
    PAGE_TURNS
        .wait_turn(
            slot.root_key.clone(),
            TurnScope::Page(page.clone()),
            slot.turn_wait,
        )
        .ok_or_else(|| {
            refused(
                "page-busy",
                Some(page),
                "other commands writing to this page took too long; nothing changed, try again",
            )
        })
}

/// The whole graph's turn. `page rename` rewrites referrers it finds as it goes,
/// so a command writing one must not race it. It and `page delete` resolve their
/// name under this turn, so an earlier command's new page is seen; a page's turn
/// could only be taken after the name it needs is resolved.
pub(super) fn graph_turn(slot: &GraphSlot) -> Result<PageTurn<'static>, ErrorBody> {
    PAGE_TURNS
        .wait_turn(slot.root_key.clone(), TurnScope::Graph, slot.turn_wait)
        .ok_or_else(|| {
            refused(
                "page-busy",
                None,
                "other commands writing to this graph took too long; nothing changed, try again",
            )
        })
}

/// One write: wait for the page's turn, flush, read through the host, compute,
/// plan through the store's serializer, reparse, apply (design §1). Another
/// command cannot move the page meanwhile; a page that an outside writer changed
/// between the read and the operation is read and resolved once more, and a
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

thread_local! {
    /// The git run the current write verb belongs to (set by the verb
    /// dispatcher for the duration of one verb), with the verb's commit subject.
    static RUN: std::cell::RefCell<Option<(super::git_run::RunCtx, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `verb` with its writes committed to the graph's git repository as
/// part of `run` (CLI slice 3, D-20261019-CL1).
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
pub(super) fn commit_if_in_run(
    slot: &GraphSlot,
    page: &PageId,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    reply: &mut Value,
) {
    let Some((run, subject)) = RUN.with(|r| r.borrow().clone()) else {
        return;
    };
    let git = match super::git_run::commit_write(slot, &run, page, &subject, old, new) {
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
    let _turn = page_turn(slot, page)?;
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
        if !absent && kind == EditKind::CreatePage {
            return Err(refused(
                "page-exists",
                Some(page),
                "a file already owns this page; nothing changed",
            ));
        }
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
        let mut tx = slot.store.page_plan(Some(kind));
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
                let mut reply = written(&read.page, &reparsed.doc, &reparsed.rev, &planned.changed);
                commit_if_in_run(
                    slot,
                    &read.page,
                    (!absent).then_some(&bytes[..]),
                    Some(&new_bytes),
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
        StoreError::NotFound(_) => ErrorBody::new(
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
    /// An explicit `--author` on `tine edit`. The `author` argument every write
    /// carries also takes `TINE_AUTHOR` and names only the run's commits.
    #[serde(default)]
    author_property: Option<String>,
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

/// `block.edit`: the text replaces the block's text and every accepted property
/// not named in it stays (children kept), then the marker transition the
/// window's `setRaw` runs. Intentional difference from OG and from the app's
/// editor, whose buffer owns visible properties (#840, D-20261101-CLIE): OG has
/// no command line, and a caller naming new text has not asked to drop `src::`.
pub(crate) fn edit_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let EditArgs {
        handle: text,
        text: new_text,
        rev,
        author_property,
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
            // A parser failure leaves no way to tell retained properties from
            // text, so replacing it could lose them. `block.prop` refuses too.
            if current.quarantined {
                return Err(kernel_refused(
                    page,
                    "block.edit refuses a block its parser could not read".into(),
                ));
            }
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
            // Every accepted property and the LOGBOOK are outside the replaced
            // text; a key the text names replaces its retained row in the join.
            let joined = set_text_with_policy(&old, org, &new_text, &HidePolicy::All)
                .map_err(|e| kernel_refused(page, e))?;
            let transitioned = tine_core::logbook::apply_edit_transition_at(
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
            block.raw = match author_property.as_deref() {
                None => transitioned,
                Some(name) => tine_core::block_regions::edit(
                    &transitioned,
                    org,
                    tine_core::block_regions::Edit::Property {
                        key: tine_core::tine_keys::AUTHOR.into(),
                        value: Some(author_name(Some(name)).map_err(|e| kernel_refused(page, e))?),
                    },
                )
                .map_err(|e| kernel_refused(page, e))?,
            };
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
    let value = if key.eq_ignore_ascii_case(tine_core::tine_keys::AUTHOR) {
        value
            .as_deref()
            .map(|name| author_name(Some(name)))
            .transpose()
            .map_err(|e| kernel_refused(&page, e))?
    } else {
        value
    };
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
    #[serde(default)]
    yes: bool,
}

/// Maximum UTF-8 bytes of recovery text in a delete reply. Disclose any
/// omitted suffix; calculate this before admitting the guarded save.
const REMOVED_TEXT_BYTES: usize = 65_536;

fn removed_fragment(root: &BlockDto, org: bool) -> (String, usize) {
    fn block(dto: &BlockDto) -> tine_core::DocBlock {
        let mut out = tine_core::DocBlock::new(dto.raw.clone());
        out.children = dto.children.iter().map(block).collect();
        out
    }
    let doc = tine_core::Document {
        pre_block: None,
        roots: vec![block(root)],
    };
    // Reuse the graph serializers so properties, multiline bodies and child
    // structure can be pasted back as a standalone fragment of the same format.
    let mut text = if org {
        tine_core::org::serialize_org(&doc)
    } else {
        tine_core::doc::serialize(&doc)
    };
    let total = text.len();
    let mut end = total.min(REMOVED_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, total - end)
}

/// `delete <handle>`: the block and its subtree, with the number of
/// references the deletion leaves broken (the app's notice rule). Incoming
/// references require explicit confirmation; every success carries recovery text.
pub(crate) fn delete_block(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    use tine_core::outline_kernel::{apply, Verb};
    let DeleteArgs {
        handle: text,
        rev,
        yes,
    } = super::verbs::parse(args)?;
    let handle = parse_handle(&text)?;
    let page = handle_page(slot, &handle)?;
    let mut deleted = (String::new(), 0usize);
    let mut removed = (String::new(), 0usize);
    let mut format = Format::Md;
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
            let removed_raws: Vec<(&str, bool)> =
                raws.iter().map(|raw| (raw.as_str(), org)).collect();
            // Read the graph counts after the write path has flushed window
            // input, and again if a changed page makes it retry the plan.
            let counts = whole_graph(slot)?.block_ref_counts();
            let broken =
                tine_core::page_properties::refs_broken_by_removing(&counts, &removed_raws);
            if broken > 0 && !yes {
                let mut error = refused(
                    "confirmation",
                    Some(page),
                    format!(
                        "{} {broken}; {}",
                        crate::i18n_shell::shell_text("en", "shell-cli-delete-broken-count"),
                        crate::i18n_shell::shell_text("en", "shell-cli-delete-confirmation"),
                    ),
                );
                error.detail.as_mut().expect("refusal detail")["broken_refs"] = json!(broken);
                return Err(error);
            }
            removed = removed_fragment(root, org);
            format = doc.format;
            deleted = (
                format!(
                    "{}/{}",
                    doc.name,
                    handle_of(doc, &path).expect("resolved path")
                ),
                broken,
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
    reply["removed_text"] = json!(removed.0);
    reply["removed_text_omitted_bytes"] = json!(removed.1);
    reply["format"] = json!(format);
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(name: &str) -> TurnScope {
        TurnScope::Page(PageId::from(name))
    }

    fn graph() -> PathBuf {
        PathBuf::from("/graph")
    }

    /// Writers take a page in arrival order, whoever wakes first.
    #[test]
    fn page_turns_are_first_come_first_served() {
        let turns = PageTurns::new();
        let first = turns.wait_turn(graph(), on("p"), TURN_WAIT).unwrap();
        let order = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for i in 0..8 {
                let (turns, order) = (&turns, &order);
                scope.spawn(move || {
                    let _turn = turns.wait_turn(graph(), on("p"), TURN_WAIT).unwrap();
                    order.lock().unwrap().push(i);
                });
                // Each is in the line before the next one starts.
                while turns.lines.lock().unwrap()[&graph()].len() < i + 2 {
                    std::thread::yield_now();
                }
            }
            drop(first);
        });
        assert_eq!(*order.lock().unwrap(), (0..8).collect::<Vec<_>>());
        assert!(
            turns.lines.lock().unwrap().is_empty(),
            "no entry outlives its line"
        );
    }

    #[test]
    fn a_turn_is_per_page_and_per_graph() {
        let turns = PageTurns::new();
        let _held = turns.wait_turn(graph(), on("p"), TURN_WAIT).unwrap();
        let now = Duration::ZERO;
        assert!(turns.wait_turn(graph(), on("q"), now).is_some());
        assert!(turns
            .wait_turn(PathBuf::from("/other"), on("p"), now)
            .is_some());
        assert!(turns.wait_turn(graph(), on("p"), now).is_none());
    }

    /// A rename waits for every earlier writer and every later one waits for it,
    /// so it can be neither raced nor starved.
    #[test]
    fn a_graph_turn_excludes_every_page_in_arrival_order() {
        let turns = PageTurns::new();
        let now = Duration::ZERO;
        let page = turns.wait_turn(graph(), on("p"), TURN_WAIT).unwrap();
        assert!(turns.wait_turn(graph(), TurnScope::Graph, now).is_none());
        drop(page);
        let whole = turns.wait_turn(graph(), TurnScope::Graph, now).unwrap();
        assert!(turns.wait_turn(graph(), on("q"), now).is_none());
        drop(whole);
        std::thread::scope(|scope| {
            let held = turns.wait_turn(graph(), on("p"), TURN_WAIT).unwrap();
            let waiting = scope.spawn(|| {
                turns
                    .wait_turn(graph(), TurnScope::Graph, TURN_WAIT)
                    .map(|_| ())
                    .unwrap()
            });
            while turns.lines.lock().unwrap()[&graph()].len() < 2 {
                std::thread::yield_now();
            }
            assert!(
                turns.wait_turn(graph(), on("q"), now).is_none(),
                "a later writer to another page does not pass the waiting rename"
            );
            drop(held);
            waiting.join().unwrap();
        });
    }

    /// A writer that gives up leaves the line, and those behind it still get theirs.
    #[test]
    fn a_turn_that_timed_out_leaves_the_line() {
        let turns = PageTurns::new();
        let held = turns.wait_turn(graph(), on("p"), TURN_WAIT).unwrap();
        assert!(turns
            .wait_turn(graph(), on("p"), Duration::from_millis(20))
            .is_none());
        assert_eq!(turns.lines.lock().unwrap()[&graph()].len(), 1);
        drop(held);
        assert!(turns.wait_turn(graph(), on("p"), Duration::ZERO).is_some());
    }

    /// A write that panics passes its turn on instead of wedging the page.
    #[test]
    fn a_panicking_writer_passes_its_turn_on() {
        let turns = PageTurns::new();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _turn = turns.wait_turn(graph(), on("p"), TURN_WAIT).unwrap();
            panic!("a write failed");
        }));
        assert!(panicked.is_err());
        assert!(turns.wait_turn(graph(), on("p"), Duration::ZERO).is_some());
    }

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
