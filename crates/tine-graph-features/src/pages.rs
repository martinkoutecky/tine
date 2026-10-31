//! Page rename, stray rescue, merge and deletion (a deletion is the page
//! host's, STEP3 §12, after this client's identity checks). The caller names
//! the operation; this client resolves identities and hands the page host one
//! operation (or commits a guarded non-page transaction). It needs no graph path, lock,
//! cache state, or write protocol.

use std::collections::{HashMap, HashSet};
use std::io;

use tine_core::model::{Format, PageDto, PageKind};
use tine_core::refs;
#[path = "pages_rename_plan.rs"]
mod rename_plan;
use rename_plan::{plan_rename, RenamePlan};

impl RenamePlan {
    /// Input policy of the host operation (STEP3 §7).
    fn input(&self) -> Input {
        if self.moved_titles_to_rebind.is_empty() {
            Input::Flush
        } else {
            Input::Refuse
        }
    }
}
#[cfg(any(test, feature = "test-faults"))]
use tine_store::SaveOutcome;
use tine_store::{
    Area, FileId, FileRev, Input, LoadError, PageHost, PageId, PageOperation, PageRead, RenameMap,
    RenameRefusal, Resolved, RewriteEffect, SaveBase, Store, StoreError, TitleRebind,
};

/// A page read or OS source selection failed at the load, identity, or file step.
#[derive(Debug)]
pub enum PageReadError {
    Load(LoadError),
    Store(StoreError),
    EmptyAlias,
    Source(String),
}

/// Resolve a page name or alias and read its current file through the
/// store's one page-by-name door, `Store::page_named` (which opens a page
/// some file is named for before the graph is ready, GH #623). Cost
/// O(index lookup + page bytes).
pub fn get_page(
    store: &Store,
    name: &str,
    kind: PageKind,
) -> Result<Option<PageRead>, PageReadError> {
    store.page_named(name, kind).map_err(PageReadError::Store)
}

/// Select a source identity and validate the existing OS hand-off path.
/// Cost O(index lookup + path components).
pub fn source_path_for_os_handoff(
    store: &Store,
    name: &str,
    kind: PageKind,
    path: Option<&str>,
) -> Result<std::path::PathBuf, PageReadError> {
    let id = if let Some(path) = path.filter(|path| !path.trim().is_empty()) {
        let config = store.config();
        let file = [
            (Area::Pages, config.pages_dir.as_str()),
            (Area::Journals, config.journals_dir.as_str()),
        ]
        .into_iter()
        .find_map(|(area, dir)| {
            path.strip_prefix(&format!("{dir}/"))
                .and_then(|rel| store.file_id(area, rel).ok())
        })
        .ok_or_else(|| {
            PageReadError::Store(StoreError::InvalidTarget("invalid page path".into()))
        })?;
        store.as_page(&file).ok_or_else(|| {
            PageReadError::Store(StoreError::InvalidTarget("invalid page path".into()))
        })?
    } else {
        match store
            .whole_graph()
            .map_err(PageReadError::Load)?
            .resolve(name, kind == PageKind::Journal)
        {
            Resolved::Existing { id, .. } | Resolved::Absent { id } => id,
            Resolved::Alias { owners } => {
                owners.into_iter().next().ok_or(PageReadError::EmptyAlias)?
            }
        }
    };
    store
        .path_for_os_handoff(&id.file(), true)
        .map_err(|error| match error {
            StoreError::PageSource(reason) => PageReadError::Source(reason),
            other => PageReadError::Store(other),
        })
}

/// Test oracle: one-page save over tine-store's test-only `Store::save`.
/// Production saves are the page host's (STEP3 §12). Keep-mine reads the current UTF-8
/// revision and lets the save guard reject later edits. Cost O(page bytes +
/// transaction publication).
#[cfg(any(test, feature = "test-faults"))]
pub fn save_page(
    store: &Store,
    kind: tine_store::EditKind,
    id: &PageId,
    page: &PageDto,
    base_rev: Option<String>,
    force: bool,
) -> Result<SaveOutcome, StoreError> {
    if page.guide {
        return Ok(SaveOutcome::GuideEphemeral);
    }
    let base = save_base(store, id, base_rev, force)?;
    Ok(store.save(kind, id, base, page))
}

#[cfg(any(test, feature = "test-faults"))]
fn save_base(
    store: &Store,
    id: &PageId,
    base_rev: Option<String>,
    force: bool,
) -> Result<SaveBase, StoreError> {
    Ok(if force {
        match store.read(&id.file(), Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
            Ok((bytes, rev)) => {
                std::str::from_utf8(&bytes).map_err(|_| StoreError::Undecodable)?;
                SaveBase::Existing(rev)
            }
            Err(StoreError::NotFound) => SaveBase::CreateNew,
            Err(error) => return Err(error),
        }
    } else {
        base_rev
            .map(|rev| SaveBase::Existing(rev.into()))
            .unwrap_or(SaveBase::CreateNew)
    })
}

use crate::store_error;

fn error(kind: io::ErrorKind, message: &str) -> io::Error {
    io::Error::new(kind, message)
}

/// A rename still finishing holds these pages: the host rename's `Busy`,
/// and a host operation's past its bound (M6). In-scope scenario: a disk
/// error keeps failing the rename's writes, or an external editor or a sync
/// service conflicted a referrer (`docs/storage-contract.md`, I-8).
pub(crate) fn rename_busy() -> io::Error {
    error(
        io::ErrorKind::WouldBlock,
        "Another rename of these pages is still finishing; try again in a moment.",
    )
}

fn view(store: &Store) -> io::Result<tine_store::WholeGraph> {
    store.whole_graph().map_err(|failure| match failure {
        tine_store::LoadError::Failed { reason } => error(io::ErrorKind::Other, &reason),
        tine_store::LoadError::Closed => error(io::ErrorKind::BrokenPipe, "store closed"),
    })
}

fn refreshed_view(store: &Store) -> io::Result<tine_store::WholeGraph> {
    store
        .refresh(tine_store::Depth::Stamps)
        .map_err(|failure| match failure {
            tine_store::LoadError::Failed { reason } => error(io::ErrorKind::Other, &reason),
            tine_store::LoadError::Closed => error(io::ErrorKind::BrokenPipe, "store closed"),
        })?;
    view(store)
}

/// Whether a page FILENAME in the pages area that the view does not admit (a
/// non-portable legacy name such as `pages/A:B.md`, written by OG on
/// Linux/macOS) decodes to `name` (`refs::same_page`). Filename evidence only;
/// no content is read. Unlistable entries are skipped rather than refusing
/// (missing one yields at worst a duplicate-identity file, never data loss).
/// Cost O(files under pages/). Master 46a0e8c27.
fn retained_legacy_page_identity_exists(store: &Store, name: &str) -> io::Result<bool> {
    let format = store.config().file_name_format;
    let listing = store.scan_area(Area::Pages, None).map_err(store_error)?;
    Ok(listing.files.iter().any(|file| {
        let path = std::path::Path::new(&file.rel);
        file.page.is_none()
            && tine_store::is_graph_text(&file.id)
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| {
                    refs::same_page(&tine_core::model::decode_page_name(stem, format), name)
                })
    }))
}

fn existing(target: Resolved) -> Vec<PageId> {
    match target {
        Resolved::Existing { id, mut others } => {
            others.insert(0, id);
            others
        }
        Resolved::Alias { .. } | Resolved::Absent { .. } => Vec::new(),
    }
}

/// Every page a name reaches: its file claimants, or the owners of the alias.
fn claimants(target: Resolved) -> Vec<PageId> {
    match target {
        Resolved::Alias { owners } => owners,
        other => existing(other),
    }
}

/// Every page holding an export definition block whose page selector names
/// one of `olds`, found by the definitions query
/// (`tine_core::export_definition::definitions_query`) over the planning view
/// and the selectors of the blocks it returns: O(P + B) in memory, memoized
/// per generation, no file read, so a definition page the rename does not
/// touch is not read. A view with more definition blocks than one bounded
/// answer carries refuses the rename before anything is written, rather than
/// leave a definition naming the old page (scenario: malformed or generated
/// input; a later page with the old name would then be the one exported).
fn export_definition_pages(
    graph: &tine_store::WholeGraph,
    olds: &[String],
) -> io::Result<Vec<PageId>> {
    use tine_core::query::ir::{ExecutionContext, QueryRows, ViewSettings};
    use tine_store::{IrAnswer, IrRequest};
    let parsed = tine_core::query::wire_parse::parse_query_pair(
        &tine_core::export_definition::definitions_query(),
        tine_core::query::wire_parse::QueryTextDialect::Og,
        &[],
        tine_core::query::registry::Registry::none(),
    );
    let answer = graph
        .query_ir(
            IrRequest::Run {
                query: &parsed.query,
                view: &ViewSettings::default(),
                context: &ExecutionContext::default(),
            },
            &tine_store::Cancel::default(),
        )
        .map(|answer| answer.expect("never cancelled"))
        .map_err(|e| io::Error::other(e.to_string()))?;
    let IrAnswer::Result(result) = answer else {
        return Err(io::Error::other("query run returned another answer"));
    };
    if result.exceeded {
        return Err(error(
            io::ErrorKind::InvalidData,
            "too many export definitions to rename their pages safely",
        ));
    }
    let QueryRows::Block { groups } = result.rows else {
        return Ok(Vec::new());
    };
    let olds: HashSet<String> = olds.iter().map(|old| refs::normalize(old)).collect();
    Ok(groups
        .iter()
        .filter(|group| {
            group.blocks.iter().any(|block| {
                tine_core::export_definition::selected_names(&block.raw)
                    .iter()
                    .any(|name| olds.contains(&refs::normalize(name)))
            })
        })
        .flat_map(|group| existing(graph.resolve(&group.page, group.kind == PageKind::Journal)))
        .collect())
}

fn physical(target: &Resolved) -> Vec<PageId> {
    match target {
        Resolved::Existing { id, others } => {
            let mut ids = vec![id.clone()];
            ids.extend(others.iter().cloned());
            ids
        }
        Resolved::Alias { .. } | Resolved::Absent { .. } => Vec::new(),
    }
}

fn validate_target(ids: &[PageId], expected_path: Option<&str>) -> io::Result<()> {
    if ids.len() > 1 {
        let mut paths: Vec<_> = ids.iter().map(|id| id.as_str()).collect();
        paths.sort_unstable();
        let paths = paths
            .iter()
            .map(|path| format!("“{path}”"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(error(
            io::ErrorKind::AlreadyExists,
            &format!("Multiple files share this page identity:\n\n{paths}\n\nTo resolve this, merge one file into the other or remove one, then try again."),
        ));
    }
    if let Some(expected) = expected_path.filter(|path| !path.trim().is_empty()) {
        if ids.first().is_none_or(|id| id.as_str() != expected) {
            return Err(error(io::ErrorKind::NotFound, "stale page target"));
        }
    }
    Ok(())
}

fn read_text(store: &Store, file: &FileId) -> io::Result<(String, FileRev)> {
    crate::parsed_text::read(store, file)
}

fn text_file(store: &Store, rel: &str) -> io::Result<FileId> {
    let config = store.config();
    for (area, dir) in [
        (Area::Pages, &config.pages_dir),
        (Area::Journals, &config.journals_dir),
    ] {
        if let Some(tail) = rel.strip_prefix(&format!("{dir}/")) {
            if let Ok(file) = store.file_id(area, tail) {
                if tine_store::is_graph_text(&file) {
                    return Ok(file);
                }
            }
        }
    }
    Err(error(io::ErrorKind::InvalidInput, "invalid file path"))
}

/// Delete one named page or journal through the page host (GH #620, STEP3
/// §12, Q-P2b-5). The identity checks run here against a refreshed
/// inventory and a fresh read: twins refuse as ambiguous, and a displayed
/// path that no longer holds the name is a stale target, unless a fresh read
/// confirms that file is also absent (external deletion during
/// confirmation), which succeeds without a write; an absent name with no
/// displayed path is a no-op. The host's deletion is bound to the bytes the
/// check read and refuses if its own read differs. The window waits for a
/// `Pending` deletion to publish (§4.4). Cost O(P + file bytes).
pub fn delete_page_expected(
    store: &Store,
    host: &PageHost,
    session: u64,
    name: &str,
    kind: PageKind,
    expected_path: Option<&str>,
) -> io::Result<PageOperation> {
    let graph = refreshed_view(store)?;
    let ids = existing(graph.resolve(name, kind == PageKind::Journal));
    let Some(id) = ids.first() else {
        if let Some(path) = expected_path.filter(|path| !path.trim().is_empty()) {
            let file = text_file(store, path)?;
            match store.read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
                Err(StoreError::NotFound) => {}
                Err(error) => return Err(store_error(error)),
                Ok(_) => return Err(error(io::ErrorKind::NotFound, "stale page target")),
            }
            // A conflicted draft of the vanished file is kept recoverably
            // before the window discards it (L9 review R1).
            if let Some(page) = store.as_page(&file) {
                return Ok(host.delete(session, &page, None));
            }
        }
        return Ok(PageOperation::Applied);
    };
    validate_target(&ids, expected_path)?;
    let (checked, _) = store
        .read(&id.file(), Some(tine_store::PARSE_INPUT_MAX_BYTES))
        .map_err(store_error)?;
    Ok(host.delete(session, id, Some(&checked[..])))
}

/// Rename a page and its file-backed namespace descendants in one transaction.
/// Pages that explicitly reference a renamed name (`WholeGraph::explicit_referrers`,
/// OG `:block/refs` semantics: `{{query}}` arguments are not references) are
/// rewritten, including bare `tags::` members and self-references; a bare
/// `alias::` member stays, as in OG `replace-old-page!` (C3Y Y4). Non-UTF-8
/// candidates are skipped as in v0.6.5; a non-round-tripping Org referrer
/// refuses the entire rename (H1). An `old` with no file still rewrites its
/// references; journal files never move. A case-only rename updates filename
/// spelling, title and references through the same transaction. A target another
/// page owns as a file or an alias refuses with `AlreadyExists` and writes nothing; [`rename_or_merge_page`]
/// is the confirmed alternative. Planning costs O(P) plus the referrer query
/// and O(referrer bytes) reads, followed by O(touched bytes) commit. A rename
/// touching N files takes N+1 sorted path locks for one move; namespace moves
/// take one additional destination lock per moved file. Conflict retry: four
/// complete plans maximum, then `WouldBlock`.
#[cfg(any(test, feature = "test-faults"))]
pub fn rename_page_expected(
    store: &Store,
    host: &PageHost,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
) -> io::Result<()> {
    rename_page_after_inventory(
        store,
        host,
        old,
        new,
        expected_path,
        None,
        &[],
        #[cfg(test)]
        || {},
    )
    .map(|_| ())
}

/// `rename_page_expected` when `merge_into` is `None`. Otherwise rename onto
/// the existing page `merge_into` the user confirmed, as OG `merge-pages!`
/// (master GH #327); `merge_into` must be the sole file claiming the new name,
/// directly or as its alias. The source's blocks append to that survivor, its
/// header (Markdown `key::` lines, Org pre-headline `#+KEY:` directives) joins
/// the survivor's: aliases are united, the survivor wins another clash, and the
/// source title and clashing lines move with the blocks as one ordinary block.
/// References and namespace descendants are renamed as by
/// `rename_page_expected`, and the source goes to graph trash in the last
/// step, so a crash never leaves a reference to a name with no live page. A
/// crash between the survivor write and the trash completes from the host
/// operation's draft at relaunch, as `merge_pages` does; a retry of a merge
/// whose draft was lost appends the moved blocks again: visible duplicates,
/// never loss (see `merged_survivor`).
/// An `old` with no file only repoints its references at the survivor. It
/// refuses before any write when, for an `old` with a file, a page other than
/// `merge_into` claims the new name (`AlreadyExists`); when `merge_into` no
/// longer claims it (`NotFound`; also for an `old` with no file whose new name
/// another page claims); when the formats differ (`InvalidInput`); when an Org
/// file it would change does not round-trip (`PermissionDenied`, or
/// `InvalidInput` for a moved page whose Org title it rebinds); or when a
/// descendant's target exists (`AlreadyExists`). Cost: the rename's, plus
/// O(source + survivor bytes and blocks).
///
/// The report lists every page file the operation moved, trashed or rewrote
/// (GH #535), so the caller refreshes only those. `unsaved_paths` names the
/// page files (graph-relative, as [`PageId`]) whose in-memory edits the caller
/// could not save: when the plan would move, trash or rewrite one of them, the
/// whole operation refuses before any write (`WouldBlock`, naming the page),
/// because rewriting it would turn those edits into a conflict against bytes
/// the user never saw. Pages it does not touch need not be saved. `WouldBlock`
/// also reports giving up after pages kept changing under repeated replans.
/// The page host (`host`) decides by its own unsaved input as well: a single
/// page's rename is the host's rename; every other rename is one host
/// operation of every page it writes (host-only-writer S2), checked before
/// the first write and completed from its draft after a crash. Either moves
/// `config.edn`'s home page in its own transaction after it; a failure there
/// leaves the home setting on the old name and is reported:
/// [`RenameReport::applied_before_error`] exposes the already-published page
/// changes for caller reconciliation.
pub fn rename_or_merge_page(
    store: &Store,
    host: &PageHost,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    merge_into: Option<&str>,
    unsaved_paths: &[String],
) -> io::Result<RenameReport> {
    rename_page_after_inventory(
        store,
        host,
        old,
        new,
        expected_path,
        merge_into,
        unsaved_paths,
        #[cfg(test)]
        || {},
    )
}

#[allow(clippy::too_many_arguments)]
fn rename_page_after_inventory(
    store: &Store,
    host: &PageHost,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    merge_into: Option<&str>,
    unsaved_paths: &[String],
    #[cfg(test)] after_inventory: impl Fn(),
) -> io::Result<RenameReport> {
    let old = old.trim();
    let new = new.trim();
    if new.is_empty() {
        return Err(error(io::ErrorKind::InvalidInput, "empty name"));
    }
    if old.is_empty() || old == new {
        return Ok(RenameReport::unchanged());
    }
    let plan = || {
        plan_rename(
            store,
            old,
            new,
            expected_path,
            merge_into,
            #[cfg(test)]
            &after_inventory,
        )
    };
    crate::retry_on_conflict("page changed repeatedly during rename", || {
        let first = plan()?;
        if first.map.0.is_empty() {
            return Ok(Some(RenameReport::unchanged()));
        }
        if let Some(pair) = first.single(store)? {
            return host_rename(store, host, pair, first, &plan, unsaved_paths);
        }
        // One host operation of every page the plan writes (host-only-writer
        // S2, DESIGN §3.1, §3.5), planned again until the host applies it:
        // flush first, except an interrupted title completion, which refuses
        // unsaved input as today. `config.edn`'s home page is a post step.
        multi_rename(store, host, old, first, &plan, unsaved_paths)
    })
}

/// A rename of several pages (a namespace, a merge, a reference-only
/// rename) as one host operation, then the home page in `config.edn` as its
/// own transaction once the operation applied (DESIGN §3.4: skipped when it
/// is uncertain). A plan that turned into a single page's rename, or into
/// another unsaved-input policy, replans (`Ok(None)`).
fn multi_rename(
    store: &Store,
    host: &PageHost,
    old: &str,
    mut first: RenamePlan,
    plan: &dyn Fn() -> io::Result<RenamePlan>,
    unsaved_paths: &[String],
) -> io::Result<Option<RenameReport>> {
    let input = first.input();
    let expected = first.pages(store)?;
    let refused = |page: &str| unsaved_error(&first.graph, page);
    let built = std::cell::RefCell::new(None);
    let replan = std::cell::Cell::new(false);
    crate::retained::operation(
        host,
        input,
        &expected,
        "the rename",
        "page changed repeatedly during rename",
        refused,
        || {
            built.replace(None);
            let mut plan = plan()?;
            if plan.single(store)?.is_some() || plan.input() != input {
                replan.set(true);
                return Ok(None);
            }
            // The commit decides on the bytes `pages` read.
            plan.pages(store)?;
            match rename_transaction(store, old, plan, unsaved_paths)? {
                RenameTransaction::Unchanged(report) => {
                    built.replace(Some((report, None)));
                    Ok(None)
                }
                RenameTransaction::Writes {
                    tx,
                    report,
                    merge,
                    home,
                } => {
                    let mut op = crate::retained::plan(tx)?;
                    // A merge's survivor holds the source's blocks: a move's
                    // order, as `merge_pages` declares it.
                    if let Some((src, dst)) = merge {
                        op.moves.push((dst, src));
                    }
                    built.replace(Some((report, Some(home))));
                    Ok(Some(op))
                }
            }
        },
    )
    .map_err(rename_operation_error)?;
    if replan.get() {
        return Ok(None);
    }
    let (mut report, home) = built.into_inner().expect("planned before the operation");
    if let Some((lookup, merged_old)) = home {
        report.home_page = commit_home_after(store, &report, &lookup, merged_old.as_ref())?;
    }
    Ok(Some(report))
}

/// The stale-plan regression probe. Preflight is read-only; a stale context
/// is `Ok(None)`, so no feature test needs a direct page commit.
#[cfg(test)]
fn preflight_rename(
    store: &Store,
    old: &str,
    plan: RenamePlan,
    unsaved_paths: &[String],
) -> io::Result<Option<()>> {
    let tx = match rename_transaction(store, old, plan, unsaved_paths)? {
        RenameTransaction::Unchanged(_) => return Ok(Some(())),
        RenameTransaction::Writes { tx, .. } => tx,
    };
    match tx.plan() {
        Ok(_) => Ok(Some(())),
        Err(tine_store::Why::Conflict { .. }) => Ok(None),
        Err(why) => Err(crate::why_error(why)),
    }
}

/// What a rename writes, as a store transaction of its page steps.
enum RenameTransaction<'s> {
    /// Nothing to write (every referrer already reads the new name, or
    /// carries VCS conflict markers).
    Unchanged(RenameReport),
    Writes {
        tx: tine_store::PagePlan<'s>,
        /// Without the home page: `config.edn` is written after the pages.
        report: RenameReport,
        /// A merge's `(source, survivor)`.
        merge: Option<(PageId, PageId)>,
        /// What the home page step needs: the rename's lookup, and the
        /// merged source, whose home `merge-pages!` keeps.
        home: (HashMap<String, String>, Option<String>),
    },
}

/// The rename's page steps in crash order (I-2): the survivor, the referrer
/// rewrites, the namespace descendant moves, then the source trash; the
/// caller writes `config.edn` LAST.
fn rename_transaction<'s>(
    store: &'s Store,
    old: &str,
    plan: RenamePlan,
    unsaved_paths: &[String],
) -> io::Result<RenameTransaction<'s>> {
    let RenamePlan {
        graph,
        config,
        map,
        lookup,
        moves,
        moved_titles_to_rebind,
        merge,
        ref_merge,
        candidates,
        mut reads,
        skipped: _,
        rewritten: _,
    } = plan;
    let mut tx = store.page_plan(Some(tine_store::EditKind::RenamePage));
    // Every step, rewrite or not, commits only in the planning context.
    tx.expect_view(&graph);
    // Crash order (I-2): survivor, then referrer rewrites, then namespace
    // descendant moves, then the source trash, then config.edn LAST. The
    // survivor is queued first because referrer rewrites are queued as the
    // candidates are read below.
    let merged = match &merge {
        Some((src, dst)) => Some(merged_survivor(store, src, dst, Some(&lookup))?),
        None => None,
    };
    if let (Some((_, dst)), Some(survivor)) = (&merge, &merged) {
        let kinds = [
            tine_store::EditKind::RenamePage,
            tine_store::EditKind::InsertBlocks,
        ];
        tx.save_page(
            &kinds,
            dst,
            SaveBase::Existing(survivor.dst_rev.clone()),
            &survivor.doc,
        );
    }
    let name_format = config.file_name_format;
    let mut edits = Vec::new();
    let mut skipped = Vec::new();
    for id in candidates {
        let file = id.file();
        // A candidate the rename cannot read (non-UTF-8, over the size or
        // depth cap: malformed imported/synced content) fails the rename,
        // naming the file. Skipping it (v0.6.5 model.rs 3699) left a moved
        // page under Old while every referrer said New (C3W W2, I-2);
        // master page_rename.rs fails the same way (audit R15-09).
        let (content, rev) = match reads.remove(&id) {
            Some(read) => read,
            None => read_text(store, &file)
                .map_err(|e| error(e.kind(), &format!("{}: {e}", file.as_str())))?,
        };
        let org = Format::from_path(id.as_str().as_ref()) == Format::Org;
        let moved = moves.contains_key(&id);
        let rebind = moved_titles_to_rebind.contains(&id);
        let marked = !tine_core::concord_queue::vcs_conflict_markers(
            &content,
            if org { Format::Org } else { Format::Md },
        )
        .is_empty();
        // Master a8fd4230d: a file carrying VCS conflict markers is not
        // ours to rewrite (R-VCS-MARKERS; scenario: an external merge or a
        // sync service left it mid-conflict). It stays byte-identical, a
        // moved one moves verbatim, and the rename reports it. A moved
        // file is rewritten inside its move step, and a marker-bearing one
        // must not be queued before this check, so both ask the shared
        // rewriter directly. Every other referrer is queued by the
        // transaction's own rewrite, computed once from this read and
        // reused by preflight (GH #623).
        let changed = if moved || marked {
            tine_core::refs::rename_rewrite(&content, org, &map.0, name_format) != content
        } else {
            let title = if rebind {
                TitleRebind::Own
            } else {
                TitleRebind::Keep
            };
            tx.rewrite_refs(&id, rev.clone(), Some(&content), &map, title) == RewriteEffect::Changes
        };
        if changed && marked {
            skipped.push(id.as_str().to_owned());
            if moved {
                edits.push((id, rev, false));
            }
            continue;
        }
        if org && changed && !tine_core::org::org_editable(&content) {
            let display = store
                .path_for_os_handoff(&file, false)
                .map_err(store_error)?;
            return Err(error(
                io::ErrorKind::PermissionDenied,
                &format!(
                    "cannot rename: {} is a read-only .org file (does not round-trip)",
                    display.display()
                ),
            ));
        }
        if marked && rebind {
            // References unchanged, but the interrupted rename's title
            // still needs rebinding in place.
            tx.rewrite_refs(&id, rev.clone(), None, &map, TitleRebind::Own);
        }
        if moved || rebind || changed {
            edits.push((id, rev, true));
        }
    }
    let outcome = if merge.is_some() || ref_merge {
        RenameOutcome::Merged
    } else {
        RenameOutcome::Renamed
    };
    if edits.is_empty() && merged.is_none() {
        return Ok(RenameTransaction::Unchanged(RenameReport {
            skipped_conflicted_referrers: skipped,
            ..RenameReport::unchanged()
        }));
    }
    let touched: Vec<TouchedPage> = edits
        .iter()
        .map(|(id, _, _)| (id, moves.contains_key(id)))
        .chain(
            merge
                .iter()
                .flat_map(|(src, dst)| [(dst, false), (src, true)]),
        )
        .map(|(id, moved)| TouchedPage {
            path: id.as_str().to_owned(),
            moved,
        })
        .collect();
    if let Some(blocked) = touched
        .iter()
        .find(|page| unsaved_paths.contains(&page.path))
    {
        return Err(unsaved_error(&graph, &blocked.path));
    }
    let merged_old = (merge.is_some() || ref_merge).then(|| refs::normalize(old));
    // Crash order (I-2), continued: the survivor and the referrer
    // rewrites are queued above; namespace descendant moves, then the
    // source trash, then config.edn LAST (the caller's). A crash before the
    // config step leaves home naming the old page. Every boundary leaves
    // `[[Old]]` resolving to the still-live source, and a retry finds the
    // survivor already holding the source payload (see `merged_survivor`).
    for (id, rev, rewrite) in edits
        .into_iter()
        .filter(|(id, _, _)| moves.contains_key(id))
    {
        tx.move_file(&id.file(), rev, &moves[&id], rewrite.then_some(&map));
    }
    if let (Some((src, _)), Some(survivor)) = (&merge, merged) {
        tx.trash(&src.file(), survivor.src_rev, tine_store::TrashIf::Any);
    }
    Ok(RenameTransaction::Writes {
        tx,
        report: RenameReport {
            outcome,
            touched,
            skipped_conflicted_referrers: skipped,
            home_page: None,
        },
        merge,
        home: (lookup, merged_old),
    })
}

/// A single page's rename with a host (STEP3 §7, F10): one host operation
/// under `pages.rs`'s policy, then the home page in `config.edn` as its own
/// transaction once the operation completes. A target that is another
/// spelling of the source's own entry (a case-folding volume, Q4) moves as
/// a normal host operation refusing unsaved input, followed by a
/// byte-preserving entry move on the driver. A replan is `Ok(None)`.
fn host_rename(
    store: &Store,
    host: &PageHost,
    (source, target): (PageId, PageId),
    mut first: RenamePlan,
    plan: &dyn Fn() -> io::Result<RenamePlan>,
    unsaved_paths: &[String],
) -> io::Result<Option<RenameReport>> {
    // The planner's value (R3): referrers whose file the rewrite changes;
    // the host adds held buffers and fixes the final pages under its lock.
    let mut referrers = first.pages(store)?;
    referrers.retain(|id| *id != source && *id != target);
    // The planner's rewrite of a referrer, reused when the host reads the
    // same bytes under the page locks (GH #623: no second rewrite under the
    // driver's state lock), and only under the planner's file-name format.
    // The host runs it on its driver, so it owns the plan's reads (unused
    // after the rename).
    let reads = std::mem::take(&mut first.reads);
    let rewritten = std::mem::take(&mut first.rewritten);
    let planned = first.config.file_name_format;
    let prepared = move |page: &PageId, old: &[u8], format| {
        let (content, _) = reads.get(page)?;
        let new = rewritten.get(page)?;
        (format == planned && content.as_bytes() == old).then(|| new.clone().into_bytes())
    };
    let renamed = host.rename(
        &source,
        &target,
        &referrers,
        &first.map,
        &first.graph,
        std::sync::Arc::new(prepared),
    );
    let (changed, mut skipped) = match renamed {
        Ok(done) => done,
        Err(RenameRefusal::Alias) => {
            let expected = first.pages(store)?;
            let refused = |page: &str| unsaved_error(&first.graph, page);
            let built = std::cell::RefCell::new(None);
            let replan = std::cell::Cell::new(false);
            crate::retained::operation(
                host,
                Input::Refuse,
                &expected,
                "the rename",
                "page changed repeatedly during rename",
                refused,
                || {
                    built.replace(None);
                    let mut plan = plan()?;
                    if plan.single(store)?.as_ref() != Some(&(source.clone(), target.clone())) {
                        replan.set(true);
                        return Ok(None);
                    }
                    plan.pages(store)?;
                    match rename_transaction(store, &first.map.0[0].0, plan, unsaved_paths)? {
                        RenameTransaction::Unchanged(report) => {
                            built.replace(Some((report, None)));
                            Ok(None)
                        }
                        RenameTransaction::Writes {
                            tx, report, home, ..
                        } => {
                            let op = crate::retained::plan(tx)?;
                            built.replace(Some((report, Some(home))));
                            Ok(Some(op))
                        }
                    }
                },
            )
            .map_err(rename_operation_error)?;
            if replan.get() {
                return Ok(None);
            }
            let (mut report, home) = built.into_inner().expect("planned before the operation");
            if let Some((lookup, merged_old)) = home {
                report.home_page = commit_home_after(store, &report, &lookup, merged_old.as_ref())?;
            }
            return Ok(Some(report));
        }
        Err(RenameRefusal::Unsaved(page)) => {
            return Err(unsaved_error(&first.graph, page.as_str()))
        }
        Err(RenameRefusal::Unwritable(page)) => return Err(unwritable(store, &page)),
        Err(RenameRefusal::Unwritten(page)) => {
            return Err(error(
                io::ErrorKind::WouldBlock,
                &format!(
                    "“{}” could not be written; the rename finishes once it can be.",
                    page.as_str()
                ),
            ))
        }
        Err(RenameRefusal::Refused) => return Ok(None),
        // Finding B: a result never claims more than its evidence, and none
        // of these is a replan.
        Err(RenameRefusal::DraftFailed) => {
            return Err(error(
                io::ErrorKind::Other,
                "The rename could not be recorded; nothing changed.",
            ))
        }
        Err(RenameRefusal::Superseded(page)) => {
            return Err(error(
                io::ErrorKind::Other,
                &format!(
                    "“{}” changed before the rename finished; your later edit stands.",
                    page.as_str()
                ),
            ))
        }
        Err(RenameRefusal::Uncertain) => {
            return Err(error(
                io::ErrorKind::TimedOut,
                "Couldn't confirm the rename yet; Tine keeps trying to finish it.",
            ))
        }
        Err(RenameRefusal::Busy) => return Err(rename_busy()),
    };
    skipped.extend(first.skipped.iter().cloned());
    skipped.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    skipped.dedup();
    if changed.is_empty() && first.moves.is_empty() {
        let skipped = skipped.iter().map(|id| id.as_str().to_owned()).collect();
        return Ok(Some(RenameReport {
            skipped_conflicted_referrers: skipped,
            ..RenameReport::unchanged()
        }));
    }
    let mut touched: Vec<TouchedPage> = changed
        .iter()
        .map(|id| TouchedPage {
            path: id.as_str().to_owned(),
            moved: false,
        })
        .collect();
    if first.moves.contains_key(&source) {
        touched.push(TouchedPage {
            path: source.as_str().to_owned(),
            moved: true,
        });
    }
    // In path order, as the transaction's report lists them.
    touched.sort_by(|a, b| a.path.cmp(&b.path));
    let mut report = RenameReport {
        outcome: RenameOutcome::Renamed,
        touched,
        skipped_conflicted_referrers: skipped.iter().map(|id| id.as_str().to_owned()).collect(),
        home_page: None,
    };
    report.home_page = commit_home_after(store, &report, &first.lookup, None)?;
    Ok(Some(report))
}

/// The home-page move after published page changes: its failure carries the
/// report of what was already applied (`RenameReport::applied_before_error`).
fn commit_home_after(
    store: &Store,
    report: &RenameReport,
    lookup: &HashMap<String, String>,
    merged_old: Option<&String>,
) -> io::Result<Option<String>> {
    commit_home(store, lookup, merged_old).map_err(|source| {
        io::Error::new(
            source.kind(),
            IncompleteRename {
                report: report.clone(),
                source,
            },
        )
    })
}

/// A rename's `:default-home` move: `logseq/config.edn` (the meta area,
/// never a page file) in its own transaction after the host's operation,
/// so no page-host fence applies; a merged source (`merged_old`) keeps it
/// (`home_after`). The new home name, if it moved.
fn commit_home(
    store: &Store,
    lookup: &HashMap<String, String>,
    merged_old: Option<&String>,
) -> io::Result<Option<String>> {
    crate::retry_on_conflict("config.edn changed repeatedly during rename", || {
        let Some((id, rev, bytes, name)) = home_after(store, lookup, merged_old) else {
            return Ok(Some(None));
        };
        let mut tx = store.transaction(None);
        tx.replace(&id, rev, bytes);
        Ok(crate::commit_retry(tx.commit())?.then_some(Some(name)))
    })
}

pub(crate) fn rename_operation_error(err: io::Error) -> io::Error {
    if err
        .get_ref()
        .is_some_and(|cause| cause.is::<crate::retained::Busy>())
    {
        rename_busy()
    } else {
        err
    }
}

/// OG `rename-page-aux` moves `:default-home` with a renamed page;
/// `merge-pages!` does not, so a merged source (`merged_old`) keeps it.
fn home_after(
    store: &Store,
    lookup: &HashMap<String, String>,
    merged_old: Option<&String>,
) -> Option<(FileId, FileRev, Vec<u8>, String)> {
    crate::config::home_after_rename(store, |home| {
        let key = refs::normalize(home);
        (merged_old != Some(&key))
            .then(|| lookup.get(&key).cloned())
            .flatten()
    })
}

/// The refusal of a rename that would rewrite a page with unsaved input,
/// naming the page by its title.
fn unsaved_error(graph: &tine_store::WholeGraph, path: &str) -> io::Error {
    let inventory = graph.inventory(tine_store::InventoryScope::All);
    let name = inventory
        .0
        .iter()
        .find(|entry| physical(&entry.target).iter().any(|id| id.as_str() == path))
        .map_or(path, |entry| entry.name.as_str());
    error(
        io::ErrorKind::WouldBlock,
        &format!(
            "“{name}” has changes Tine could not save, and this rename would rewrite it. \
             Save or discard those changes, then rename again."
        ),
    )
}

/// The refusal of a rename that would change a page it cannot rewrite: a
/// read-only Org file, or one that is not UTF-8.
fn unwritable(store: &Store, page: &PageId) -> io::Error {
    let file = page.file();
    if Format::from_path(page.as_str().as_ref()) == Format::Org {
        if let Ok(display) = store.path_for_os_handoff(&file, false) {
            return error(
                io::ErrorKind::PermissionDenied,
                &format!(
                    "cannot rename: {} is a read-only .org file (does not round-trip)",
                    display.display()
                ),
            );
        }
    }
    error(
        io::ErrorKind::InvalidData,
        &format!("{}: stream did not contain valid UTF-8", file.as_str()),
    )
}

/// The namespace tail of descendant `name` below the parent whose page key is
/// `parent_key`, starting at its separating `/` (`é/x` below `é` → `/x`). The
/// descendant was matched by page key (case fold, NFC, boundary slashes), which
/// can change character counts but never the namespace separators, so the tail
/// is located by separator count in the descendant's own spelling, never by the
/// parent's character count (REG-OG-C5-L03-S1: an NFD parent with an NFC child
/// renamed `Newx` instead of `New/x`). O(name).
fn namespace_suffix<'a>(name: &'a str, parent_key: &str) -> &'a str {
    let trimmed = name.trim();
    let body = trimmed.strip_prefix('/').unwrap_or(trimmed);
    let depth = parent_key.matches('/').count();
    body.match_indices('/')
        .nth(depth)
        .map_or("", |(at, _)| &body[at..])
}

/// What [`rename_or_merge_page`] did, and the page files it wrote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RenameReport {
    pub outcome: RenameOutcome,
    /// Every page file moved, trashed or rewritten; empty when `Unchanged`.
    pub touched: Vec<TouchedPage>,
    /// Paths of referrers carrying VCS conflict markers whose references the
    /// rename left untouched (R-VCS-MARKERS); the UI says so. Reported once
    /// per file.
    pub skipped_conflicted_referrers: Vec<String>,
    /// The new `:default-home` page name when this rename moved the home page
    /// with it (in the same transaction); `None` otherwise.
    pub home_page: Option<String>,
}

impl RenameReport {
    /// Page changes already published before a later, separate home-page
    /// configuration transaction failed. Callers must refresh these pages
    /// even though the overall operation returned an error. Other errors
    /// carry no such proof and may still represent partial publication.
    pub fn applied_before_error(error: &io::Error) -> Option<&Self> {
        error
            .get_ref()?
            .downcast_ref::<IncompleteRename>()
            .map(|error| &error.report)
    }

    fn unchanged() -> Self {
        Self {
            outcome: RenameOutcome::Unchanged,
            touched: Vec::new(),
            skipped_conflicted_referrers: Vec::new(),
            home_page: None,
        }
    }
}

#[derive(Debug)]
struct IncompleteRename {
    report: RenameReport,
    source: io::Error,
}

impl std::fmt::Display for IncompleteRename {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Page files renamed, but the home-page config update failed: {}",
            self.source
        )
    }
}

impl std::error::Error for IncompleteRename {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// One page file a rename or merge wrote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TouchedPage {
    /// Graph-relative path before the operation (the frontend's page id).
    pub path: String,
    /// The file left this path (moved, or trashed by a merge); otherwise it
    /// was rewritten in place.
    pub moved: bool,
}

/// What [`rename_or_merge_page`] did; serialized lowercase for the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RenameOutcome {
    /// Files moved and/or references rewritten.
    Renamed,
    /// Merged into the confirmed page (for a file-less source: its references
    /// were repointed).
    Merged,
    /// Nothing was written: identical spelling, an empty `old`, or a name
    /// that no file or reference uses.
    Unchanged,
}

/// Rescue a stray page or journal file into a uniquely named normal page.
/// Its bytes are unchanged and inbound references are not rewritten, matching
/// v0.6.5 model.rs 1779. One host operation of the source and
/// the target (DESIGN §3.1): their unsaved input is saved first. Cost O(P +
/// source bytes) per try; four tries maximum.
pub fn rename_file_to_page(
    store: &Store,
    host: &PageHost,
    src_rel: &str,
    new_name: &str,
) -> io::Result<()> {
    let name = new_name.trim();
    if name.is_empty() {
        return Err(error(io::ErrorKind::InvalidInput, "empty page name"));
    }
    let src = text_file(store, src_rel)?;
    let ext = Format::from_path(src.as_str().as_ref()).ext();
    let rel = format!(
        "{}.{}",
        tine_core::model::encode_page_name(name, store.config().file_name_format),
        ext
    );
    let to = store.file_id(Area::Pages, &rel).map_err(store_error)?;
    // A retained non-portable legacy filename (`pages/A:B.md`) is not a view
    // claimant but OG loads it as that page: refuse like a live claimant
    // (external-editor / multi-device graph; master 46a0e8c27).
    let unclaimed = || {
        if !existing(refreshed_view(store)?.resolve(name, false)).is_empty()
            || retained_legacy_page_identity_exists(store, name)?
        {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "a page with that name already exists",
            ));
        }
        Ok(())
    };
    let max = Some(tine_store::PARSE_INPUT_MAX_BYTES);
    let transaction = |rev| {
        let mut tx = store.page_plan(Some(tine_store::EditKind::RenamePage));
        tx.move_file(&src, rev, &to, None);
        tx
    };
    let mut cleanup_rev = None;
    let plan = || {
        unclaimed()?;
        let (_, rev) = store.read(&src, max).map_err(store_error)?;
        if store.as_page(&src).is_none() {
            // C2: destination through the host, source retained until Applied.
            cleanup_rev = Some(rev.clone());
        }
        crate::retained::plan(transaction(rev)).map(Some)
    };
    let touched = crate::retained::pages(store, [&src, &to]);
    crate::retained::operation(
        host,
        Input::Flush,
        &touched,
        "renaming the file",
        "page changed repeatedly during rescue",
        crate::retained::unsaved,
        plan,
    )?;
    if let Some(rev) = cleanup_rev {
        trash_rescue_source(store, &src, rev)?;
    }
    Ok(())
}

/// Post-Applied non-page cleanup. A crash keeps both copies; a changed
/// source stays recoverable at its original name.
fn trash_rescue_source(store: &Store, src: &FileId, rev: FileRev) -> io::Result<()> {
    debug_assert!(store.as_page(src).is_none());
    let mut tx = store.transaction(None);
    tx.trash(src, rev, tine_store::TrashIf::Any);
    crate::tx_error(tx.commit()).map(drop)
}

/// Merge one source into a survivor, as [`rename_or_merge_page`] merges pages
/// but without renaming anything (aliases united, Org directives kept, through
/// the same `merged_survivor`), then recoverably trash the source in the same
/// commit. Org pairs must round-trip; formats must match. v0.6.5 never
/// rewrites inbound refs in this operation. One host operation
/// of both pages (DESIGN §3.1): their unsaved input is saved first. Cost O(source + survivor
/// bytes and blocks) per try; four complete attempts maximum.
pub fn merge_pages(store: &Store, host: &PageHost, src_rel: &str, dst_rel: &str) -> io::Result<()> {
    let src = text_file(store, src_rel)?;
    let dst = text_file(store, dst_rel)?;
    if src == dst {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "cannot merge a file into itself",
        ));
    }
    let src_id = store
        .as_page(&src)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let dst_id = store
        .as_page(&dst)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let transaction = |survivor: &Survivor| {
        let mut tx = store.page_plan(Some(tine_store::EditKind::InsertBlocks));
        tx.save_page(
            &[
                tine_store::EditKind::InsertBlocks,
                tine_store::EditKind::DeletePage,
            ],
            &dst_id,
            SaveBase::Existing(survivor.dst_rev.clone()),
            &survivor.doc,
        );
        tx.trash(&src, survivor.src_rev.clone(), tine_store::TrashIf::Any);
        tx
    };
    // One host operation (DESIGN §3.1): the survivor's write, then the
    // source's deletion, a move's order (the survivor holds the source's
    // blocks), from one draft, so a crash between them completes the
    // deletion at relaunch rather than appending the blocks twice.
    let plan = || {
        let survivor = merged_survivor(store, &src_id, &dst_id, None)?;
        let mut op = crate::retained::plan(transaction(&survivor))?;
        op.moves.push((dst_id.clone(), src_id.clone()));
        Ok(Some(op))
    };
    crate::retained::operation(
        host,
        Input::Flush,
        &[dst_id.clone(), src_id.clone()],
        "the merge",
        "pages changed repeatedly during merge",
        crate::retained::unsaved,
        plan,
    )
}

struct Survivor {
    doc: PageDto,
    src_rev: FileRev,
    dst_rev: FileRev,
}

/// The survivor page after appending `src`, with `renames` applied to both.
/// OG `merge-pages!` moves the source's property block with its blocks; here
/// each source header property (Markdown `key::`, Org `#+KEY:`) the survivor
/// lacks joins its header, alias values are united, a line equal to a survivor
/// header line (same key, same trimmed value) is already there, and the rest
/// (a clash, the source title, free text, an Org drawer) moves as one ordinary
/// block, so the source identity never renames the survivor and every source
/// alias keeps resolving (I-4, I-12).
///
/// The source payload (the moved block, if any, then the source's top-level
/// blocks) is always appended, as OG `merge-pages!` moves every block. A first
/// merge cannot be told from a retry after a crash between the survivor write
/// and the source trash without durable evidence, so such a retry appends the
/// payload a second time: visible duplicates, never loss (Martin, 2026-10-05,
/// option (a); the old "survivor already ends with it" skip silently dropped
/// legitimate duplicate blocks on a first merge). Header joins stay idempotent
/// by the equal-line rule.
fn merged_survivor(
    store: &Store,
    src: &PageId,
    dst: &PageId,
    renames: Option<&HashMap<String, String>>,
) -> io::Result<Survivor> {
    let org = Format::from_path(src.as_str().as_ref()) == Format::Org;
    if org != (Format::from_path(dst.as_str().as_ref()) == Format::Org) {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "files are in different formats",
        ));
    }
    let (src_text, src_rev) = read_text(store, &src.file())?;
    let (dst_text, dst_rev) = read_text(store, &dst.file())?;
    if org && (!tine_core::org::org_editable(&src_text) || !tine_core::org::org_editable(&dst_text))
    {
        return Err(error(
            io::ErrorKind::PermissionDenied,
            "an org file in this pair does not round-trip; not merging",
        ));
    }
    // Parsed from the bytes whose revisions guard the write (A-V4b).
    let page = |id, text: &str| store.page_of(id, text.as_bytes()).map_err(store_error);
    let source = page(src, &src_text)?.doc;
    let mut doc = page(dst, &dst_text)?.doc;
    let format = store.config().file_name_format;
    if let Some(renames) = renames {
        rename_doc(&mut doc, renames, org, format);
    }
    let (header, blocks) = merge_payload::payload(&doc, &source, renames, org, format);
    if let Some(header) = header {
        doc.pre_block = Some(header);
    }
    doc.blocks.extend(blocks);
    Ok(Survivor {
        doc,
        src_rev,
        dst_rev,
    })
}

fn rename_doc(
    doc: &mut PageDto,
    renames: &HashMap<String, String>,
    org: bool,
    format: tine_core::config::FileNameFormat,
) {
    fn rewrite(
        raw: &mut String,
        renames: &HashMap<String, String>,
        org: bool,
        format: tine_core::config::FileNameFormat,
    ) {
        *raw = refs::rename_tags_property_multi(
            &refs::rename_refs_multi(raw, renames, org, format),
            renames,
            org,
        );
    }
    fn walk(
        blocks: &mut [tine_core::model::BlockDto],
        renames: &HashMap<String, String>,
        org: bool,
        format: tine_core::config::FileNameFormat,
    ) {
        for block in blocks {
            rewrite(&mut block.raw, renames, org, format);
            walk(&mut block.children, renames, org, format);
        }
    }
    if let Some(pre) = doc.pre_block.as_mut() {
        rewrite(pre, renames, org, format);
    }
    walk(&mut doc.blocks, renames, org, format);
}

mod merge_payload;

#[cfg(test)]
#[path = "pages_snapshot_tests.rs"]
mod snapshot_tests;
