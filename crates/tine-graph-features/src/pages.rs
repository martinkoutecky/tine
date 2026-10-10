//! Recoverable page deletion, rename, stray rescue, and merge. The caller names
//! the operation; this client resolves identities and commits one guarded store
//! transaction. It needs no graph path, lock, cache state, or write protocol.

use std::collections::{HashMap, HashSet};
use std::io;

use tine_core::model::{Format, PageDto, PageKind};
use tine_core::refs;
#[cfg(any(test, feature = "test-faults"))]
use tine_store::SaveOutcome;
use tine_store::{
    Area, FileId, FileRev, Input, LoadError, PageHost, PageId, PageRead, RenameMap, RenameRefusal,
    Resolved, RewriteEffect, SaveBase, SavePagesOutcome, Store, StoreError, TitleRebind,
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
/// Production saves use [`save_pages`]. Keep-mine reads the current UTF-8
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

/// Compute each requested base, then save every page in one store transaction.
pub fn save_pages(
    store: &Store,
    entries: &[(
        PageId,
        PageDto,
        Option<String>,
        bool,
        Vec<tine_store::EditKind>,
    )],
) -> Result<SavePagesOutcome, (usize, StoreError)> {
    let mut prepared = Vec::with_capacity(entries.len());
    for (index, (id, page, base_rev, force, kinds)) in entries.iter().enumerate() {
        if page.guide {
            prepared.push((id.clone(), SaveBase::CreateNew, page.clone(), kinds.clone()));
            continue;
        }
        prepared.push((
            id.clone(),
            save_base(store, id, base_rev.clone(), *force).map_err(|error| (index, error))?,
            page.clone(),
            kinds.clone(),
        ));
    }
    Ok(store.save_pages(&prepared))
}

use crate::{is_conflict, store_error, tx_error};

fn error(kind: io::ErrorKind, message: &str) -> io::Error {
    io::Error::new(kind, message)
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
        return Err(error(
            io::ErrorKind::AlreadyExists,
            "multiple files share this page identity; mutation is ambiguous",
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

/// Delete one named page or journal into recoverable trash. A supplied revision
/// pins the displayed version; without one the current version is read and
/// guarded. An absent reference-only page is a no-op. With an expected path,
/// an absent identity succeeds only after a fresh read confirms that file is
/// also absent (external deletion during confirmation); a live file at the
/// path or a replacement claimant still refuses as a stale target. No file is
/// written in the absent case. A retained writer (STEP3 §7) of the page and
/// the expected path: their unsaved input is saved first. Cost O(P + file
/// bytes) per try; a conflict is replanned at most four times.
pub fn delete_page_expected(
    store: &Store,
    host: Option<&PageHost>,
    name: &str,
    kind: PageKind,
    expected_path: Option<&str>,
    expected_rev: Option<&FileRev>,
) -> io::Result<()> {
    let discover = || {
        let mut pages = existing(refreshed_view(store)?.resolve(name, kind == PageKind::Journal));
        let expected = expected_path.and_then(|path| text_file(store, path).ok());
        pages.extend(crate::retained::pages(store, &expected));
        Ok(pages)
    };
    crate::retained::reserved(
        host,
        Input::Flush,
        discover,
        crate::retained::unsaved,
        |_| {
            crate::retry_on_conflict("page changed repeatedly during delete", || {
                let graph = refreshed_view(store)?;
                let ids = existing(graph.resolve(name, kind == PageKind::Journal));
                let Some(id) = ids.first() else {
                    if let Some(path) = expected_path.filter(|path| !path.trim().is_empty()) {
                        let file = text_file(store, path)?;
                        match store.read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
                            Err(StoreError::NotFound) => {}
                            Err(error) => return Err(store_error(error)),
                            Ok(_) => {
                                return Err(error(io::ErrorKind::NotFound, "stale page target"))
                            }
                        }
                    }
                    return Ok(Some(()));
                };
                validate_target(&ids, expected_path)?;
                let file = id.file();
                let (_, rev) = store
                    .read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES))
                    .map_err(store_error)?;
                if expected_rev.is_some_and(|expected| *expected != rev) {
                    return Err(error(io::ErrorKind::WouldBlock, "stale page revision"));
                }
                let mut tx = store.transaction(Some(tine_store::EditKind::DeletePage));
                tx.trash(&file, rev, tine_store::TrashIf::Any);
                let outcome = tx.commit();
                if is_conflict(&outcome) {
                    if expected_rev.is_some() {
                        return Err(error(io::ErrorKind::WouldBlock, "stale page revision"));
                    }
                    return Ok(None);
                }
                tx_error(outcome)?;
                Ok(Some(()))
            })
        },
    )
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
pub fn rename_page_expected(
    store: &Store,
    host: Option<&PageHost>,
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

/// [`rename_page_expected`] when `merge_into` is `None`. Otherwise rename onto
/// the existing page `merge_into` the user confirmed, as OG `merge-pages!`
/// (master GH #327); `merge_into` must be the sole file claiming the new name,
/// directly or as its alias. The source's blocks append to that survivor, its
/// header (Markdown `key::` lines, Org pre-headline `#+KEY:` directives) joins
/// the survivor's: aliases are united, the survivor wins another clash, and the
/// source title and clashing lines move with the blocks as one ordinary block.
/// References and namespace descendants are renamed as by
/// [`rename_page_expected`], and the source goes to graph trash in the last
/// step, so a crash never leaves a reference to a name with no live page. A
/// retry after a crash between the survivor write and the trash appends the
/// moved blocks again: visible duplicates, never loss (see `merged_survivor`).
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
/// With a page host (`host`), its own unsaved input decides instead for a
/// single page's rename, which is the host's operation; every other rename
/// is a retained writer of the pages it touches (STEP3 §7), and the host's
/// rename moves `config.edn`'s home page in its own transaction after it.
/// Otherwise every write is one `tine-store` transaction: all steps are checked before
/// the first write and a failure rolls back; only a failed rollback or
/// publication leaves partial state, and its error says to inspect disk.
pub fn rename_or_merge_page(
    store: &Store,
    host: Option<&PageHost>,
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
    host: Option<&PageHost>,
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
        let Some(host) = host else {
            return commit_rename(store, old, first, unsaved_paths);
        };
        if let Some(pair) = first.single(store)? {
            return host_rename(store, host, pair, first, &plan, unsaved_paths);
        }
        // A retained writer (STEP3 §7) of every page the plan writes, planned
        // again under the reservation: flush first, except an interrupted
        // title completion, which refuses unsaved input as today.
        let input = first.input();
        let last = std::cell::Cell::new(None);
        let discover = || {
            let plan = plan()?;
            let pages = plan.pages(store);
            last.set(Some(plan));
            Ok(pages)
        };
        let refused = |page: &str| unsaved_error(&first.graph, page);
        crate::retained::reserved(Some(host), input, discover, refused, |_| {
            let plan = last.take().expect("planned under the reservation");
            if plan.single(store)?.is_some() || plan.input() != input {
                return Ok(None);
            }
            commit_rename(store, old, plan, unsaved_paths)
        })
    })
}

/// What one rename attempt writes, planned from one refreshed view.
struct RenamePlan {
    graph: tine_store::WholeGraph,
    map: RenameMap,
    lookup: HashMap<String, String>,
    moves: HashMap<PageId, FileId>,
    moved_titles_to_rebind: HashSet<PageId>,
    merge: Option<(PageId, PageId)>,
    ref_merge: bool,
    candidates: Vec<PageId>,
}

impl RenamePlan {
    /// Every page the plan can write: referrers, moved pages and their
    /// destinations, and a merge's survivor and source.
    fn pages(&self, store: &Store) -> Vec<PageId> {
        let mut pages = self.candidates.clone();
        pages.extend(crate::retained::pages(store, self.moves.values()));
        pages.extend(
            self.merge
                .iter()
                .flat_map(|(src, dst)| [src.clone(), dst.clone()]),
        );
        pages
    }

    /// An interrupted title completion refuses unsaved input; every other
    /// retained rename flushes it first (STEP3 §7).
    fn input(&self) -> Input {
        if self.moved_titles_to_rebind.is_empty() {
            Input::Flush
        } else {
            Input::Refuse
        }
    }

    /// The source and target of a single page's rename, which a host runs
    /// as one page operation (STEP3 §7, F10): one name, no namespace
    /// descendant, no merge, no interrupted title. A source with no file
    /// renames its references only, under the default Markdown spelling.
    fn single(&self, store: &Store) -> io::Result<Option<(PageId, PageId)>> {
        if self.merge.is_some()
            || self.ref_merge
            || !self.moved_titles_to_rebind.is_empty()
            || self.map.0.len() != 1
        {
            return Ok(None);
        }
        let spelled = |name: &str| {
            let rel = format!(
                "{}.md",
                tine_core::model::encode_page_name(name, store.config().file_name_format)
            );
            store.file_id(Area::Pages, &rel).map_err(store_error)
        };
        let (source, target) = match self.moves.iter().next() {
            Some((source, to)) => (source.clone(), to.clone()),
            None => {
                let (old, new) = &self.map.0[0];
                (page(store, &spelled(old)?)?, spelled(new)?)
            }
        };
        Ok(Some((source, page(store, &target)?)))
    }
}

fn page(store: &Store, file: &FileId) -> io::Result<PageId> {
    store
        .as_page(file)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))
}

fn plan_rename(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    merge_into: Option<&str>,
    #[cfg(test)] after_inventory: &dyn Fn(),
) -> io::Result<RenamePlan> {
    let graph = refreshed_view(store)?;
    let old_key = refs::normalize(old);
    // Only file-claimed names move: the full inventory's alias and
    // reference-only name discovery is not needed here (GH #623).
    let owned = graph.inventory(tine_store::InventoryScope::FilesAtOrUnder(&old_key));
    #[cfg(test)]
    after_inventory();
    let source = existing(graph.resolve(old, false));
    validate_target(&source, expected_path)?;
    let mut pairs = Vec::new();
    let mut moves = HashMap::<PageId, FileId>::new();
    let mut moved_titles_to_rebind = HashSet::<PageId>::new();
    let mut destinations = HashSet::new();
    let mut identities = HashSet::new();
    let mut primary_is_file = false;
    let mut merge = None;
    for entry in owned.0.iter().filter(|entry| !entry.is_journal) {
        let ids = physical(&entry.target);
        let primary = refs::normalize(&entry.name) == old_key;
        let new_name = if primary {
            new.to_owned()
        } else {
            format!("{new}{}", namespace_suffix(&entry.name, &old_key))
        };
        if ids.len() > 1 {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "multiple files share this page identity; mutation is ambiguous",
            ));
        }
        let id = ids[0].clone();
        // An alias names its owner's page, so it is taken like a file name.
        let mut taken = claimants(graph.resolve(&new_name, false));
        taken.retain(|claimant| *claimant != id);
        if let (true, Some(into), [survivor]) = (primary, merge_into, taken.as_slice()) {
            if survivor.as_str() == into {
                identities.insert(refs::normalize(&new_name));
                merge = Some((id, survivor.clone()));
                primary_is_file = true;
                pairs.push((entry.name.clone(), new_name));
                continue;
            }
        }
        if !taken.is_empty() {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "target page identity already exists elsewhere in the graph",
            ));
        }
        let ext = Format::from_path(id.as_str().as_ref()).ext();
        let rel = format!(
            "{}.{}",
            tine_core::model::encode_page_name(&new_name, store.config().file_name_format),
            ext
        );
        let to = store.file_id(Area::Pages, &rel).map_err(store_error)?;
        if !destinations.insert(to.as_str().to_owned())
            || !identities.insert(refs::normalize(&new_name))
        {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "multiple pages map to the same rename target",
            ));
        }
        if to == id.file() {
            // A crash after the physical rename can leave Old's explicit
            // title in New's file. Finish that rewrite in place on retry.
            moved_titles_to_rebind.insert(id.clone());
            pairs.push((entry.name.clone(), new_name));
            primary_is_file |= primary;
            continue;
        }
        if primary {
            primary_is_file = true;
        }
        pairs.push((entry.name.clone(), new_name));
        moves.insert(id, to);
    }
    // v0.6.5 model.rs 3654: reference-only pages have no move, but refs
    // change. Onto an existing page that repoints them, which is OG
    // `merge-pages!` of a page with no blocks, so it needs the confirmation.
    let mut ref_merge = false;
    if !primary_is_file {
        let taken = claimants(graph.resolve(new, false));
        match (merge_into, taken.as_slice()) {
            (_, []) => {}
            (Some(into), [owner]) if owner.as_str() == into => ref_merge = true,
            (Some(_), _) => {}
            (None, _) => {
                return Err(error(
                    io::ErrorKind::AlreadyExists,
                    "target page identity already exists elsewhere in the graph",
                ))
            }
        }
        pairs.push((old.to_owned(), new.to_owned()));
    }
    if merge_into.is_some() && merge.is_none() && !ref_merge {
        return Err(error(
            io::ErrorKind::NotFound,
            "the page to merge into no longer owns that name",
        ));
    }
    let map = RenameMap(pairs);
    let lookup: HashMap<_, _> = map
        .0
        .iter()
        .map(|(from, to)| (refs::normalize(from), to.clone()))
        .collect();
    // v0.6.5 model.rs 3681-3690 (warm index) and OG `:block/refs`: only
    // pages that explicitly reference a renamed name are rewritten, plus
    // every moved page. A name mentioned only inside `{{query}}` stays.
    let olds: Vec<String> = map.0.iter().map(|(from, _)| from.clone()).collect();
    let mut candidates: Vec<PageId> = graph.explicit_referrers(&olds);
    candidates.extend(moves.keys().cloned());
    candidates.extend(moved_titles_to_rebind.iter().cloned());
    candidates.retain(|id| {
        merge
            .as_ref()
            .is_none_or(|(src, dst)| id != src && id != dst)
    });
    candidates.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    candidates.dedup();
    Ok(RenamePlan {
        graph,
        map,
        lookup,
        moves,
        moved_titles_to_rebind,
        merge,
        ref_merge,
        candidates,
    })
}

/// The rename as one store transaction (`unsaved_paths`: see
/// [`rename_or_merge_page`]).
fn commit_rename(
    store: &Store,
    old: &str,
    plan: RenamePlan,
    unsaved_paths: &[String],
) -> io::Result<Option<RenameReport>> {
    let RenamePlan {
        graph,
        map,
        lookup,
        moves,
        moved_titles_to_rebind,
        merge,
        ref_merge,
        candidates,
    } = plan;
    let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
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
    let name_format = store.config().file_name_format;
    let mut edits = Vec::new();
    let mut skipped = Vec::new();
    for id in candidates {
        let file = id.file();
        // A candidate the rename cannot read (non-UTF-8, over the size or
        // depth cap: malformed imported/synced content) fails the rename,
        // naming the file. Skipping it (v0.6.5 model.rs 3699) left a moved
        // page under Old while every referrer said New (C3W W2, I-2);
        // master page_rename.rs fails the same way (audit R15-09).
        let (content, rev) = read_text(store, &file)
            .map_err(|e| error(e.kind(), &format!("{}: {e}", file.as_str())))?;
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
        return Ok(Some(RenameReport {
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
    let home = home_after(store, &lookup, merged_old.as_ref());
    // Crash order (I-2), continued: the survivor and the referrer
    // rewrites are queued above; namespace descendant moves, then the
    // source trash, then config.edn LAST. A crash before the config step
    // leaves home naming the old page. Every boundary leaves `[[Old]]`
    // resolving to the still-live source, and a retry finds the survivor
    // already holding the source payload (see `merged_survivor`).
    for (id, rev, rewrite) in edits
        .into_iter()
        .filter(|(id, _, _)| moves.contains_key(id))
    {
        tx.move_file(&id.file(), rev, &moves[&id], rewrite.then_some(&map));
    }
    if let (Some((src, _)), Some(survivor)) = (&merge, merged) {
        tx.trash(&src.file(), survivor.src_rev, tine_store::TrashIf::Any);
    }
    if let Some((id, rev, bytes, _)) = &home {
        tx.replace(id, rev.clone(), bytes.clone());
    }
    Ok(crate::commit_retry(tx.commit())?.then_some(RenameReport {
        outcome,
        touched,
        skipped_conflicted_referrers: skipped,
        home_page: home.map(|(_, _, _, name)| name),
    }))
}

/// A single page's rename with a host (STEP3 §7, F10): one host operation
/// under `pages.rs`'s policy, then the home page in `config.edn` as its own
/// transaction once the operation completes. A target that is another
/// spelling of the source's own entry (a case-folding volume, Q4) moves as
/// a retained transaction instead, refusing unsaved input, and the host
/// follows the new spelling. A replan is `Ok(None)`.
fn host_rename(
    store: &Store,
    host: &PageHost,
    (source, target): (PageId, PageId),
    first: RenamePlan,
    plan: &dyn Fn() -> io::Result<RenamePlan>,
    unsaved_paths: &[String],
) -> io::Result<Option<RenameReport>> {
    let referrers: Vec<PageId> = first
        .candidates
        .iter()
        .filter(|id| **id != source)
        .cloned()
        .collect();
    let (changed, skipped) = match host.rename(&source, &target, &referrers, &first.map) {
        Ok(done) => done,
        Err(RenameRefusal::Alias) => {
            let last = std::cell::Cell::new(None);
            let discover = || {
                let plan = plan()?;
                let pages = plan.pages(store);
                last.set(Some(plan));
                Ok(pages)
            };
            let refused = |page: &str| unsaved_error(&first.graph, page);
            return crate::retained::reserved(Some(host), Input::Refuse, discover, refused, |_| {
                let plan = last.take().expect("planned under the reservation");
                if plan.single(store)?.as_ref() != Some(&(source.clone(), target.clone())) {
                    return Ok(None);
                }
                let report = commit_rename(store, &first.map.0[0].0, plan, unsaved_paths)?;
                if report.is_some() {
                    host.respell(&source, &target);
                }
                Ok(report)
            });
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
    };
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
    let home_page =
        crate::retry_on_conflict("config.edn changed repeatedly during rename", || {
            let Some((id, rev, bytes, name)) = home_after(store, &first.lookup, None) else {
                return Ok(Some(None));
            };
            let mut tx = store.transaction(None);
            tx.replace(&id, rev, bytes);
            Ok(crate::commit_retry(tx.commit())?.then_some(Some(name)))
        })?;
    Ok(Some(RenameReport {
        outcome: RenameOutcome::Renamed,
        touched,
        skipped_conflicted_referrers: skipped.iter().map(|id| id.as_str().to_owned()).collect(),
        home_page,
    }))
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
    fn unchanged() -> Self {
        Self {
            outcome: RenameOutcome::Unchanged,
            touched: Vec::new(),
            skipped_conflicted_referrers: Vec::new(),
            home_page: None,
        }
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
/// v0.6.5 model.rs 1779. A retained writer (STEP3 §7) of the source and the
/// target: their unsaved input is saved first. Cost O(P + source bytes) per
/// try; four tries maximum.
pub fn rename_file_to_page(
    store: &Store,
    host: Option<&PageHost>,
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
    let discover = || Ok(crate::retained::pages(store, [&src, &to]));
    crate::retained::reserved(
        host,
        Input::Flush,
        discover,
        crate::retained::unsaved,
        |_| {
            crate::retry_on_conflict("page changed repeatedly during rescue", || {
                // A retained non-portable legacy filename (`pages/A:B.md`) is not a view
                // claimant but OG loads it as that page: refuse like a live claimant
                // (external-editor / multi-device graph; master 46a0e8c27).
                if !existing(refreshed_view(store)?.resolve(name, false)).is_empty()
                    || retained_legacy_page_identity_exists(store, name)?
                {
                    return Err(error(
                        io::ErrorKind::AlreadyExists,
                        "a page with that name already exists",
                    ));
                }
                let (_, rev) = store
                    .read(&src, Some(tine_store::PARSE_INPUT_MAX_BYTES))
                    .map_err(store_error)?;
                let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
                tx.move_file(&src, rev, &to, None);
                Ok(crate::commit_retry(tx.commit())?.then_some(()))
            })
        },
    )
}

/// Merge one source into a survivor, as [`rename_or_merge_page`] merges pages
/// but without renaming anything (aliases united, Org directives kept, through
/// the same `merged_survivor`), then recoverably trash the source in the same
/// commit. Org pairs must round-trip; formats must match. v0.6.5 never
/// rewrites inbound refs in this operation. A retained writer (STEP3 §7) of
/// both pages: their unsaved input is saved first. Cost O(source + survivor
/// bytes and blocks) per try; four complete attempts maximum.
pub fn merge_pages(
    store: &Store,
    host: Option<&PageHost>,
    src_rel: &str,
    dst_rel: &str,
) -> io::Result<()> {
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
    let discover = || Ok(vec![src_id.clone(), dst_id.clone()]);
    crate::retained::reserved(
        host,
        Input::Flush,
        discover,
        crate::retained::unsaved,
        |_| {
            crate::retry_on_conflict("pages changed repeatedly during merge", || {
                let survivor = merged_survivor(store, &src_id, &dst_id, None)?;
                let mut tx = store.transaction(Some(tine_store::EditKind::InsertBlocks));
                tx.save_page(
                    &[
                        tine_store::EditKind::InsertBlocks,
                        tine_store::EditKind::DeletePage,
                    ],
                    &dst_id,
                    SaveBase::Existing(survivor.dst_rev),
                    &survivor.doc,
                );
                tx.trash(&src, survivor.src_rev, tine_store::TrashIf::Any);
                Ok(crate::commit_retry(tx.commit())?.then_some(()))
            })
        },
    )
}

fn lines<'a>(mut parts: impl Iterator<Item = &'a str>) -> String {
    let first = parts.next().unwrap_or_default().to_owned();
    parts.fold(first, |text, line| text + "\n" + line)
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
    let (header, blocks) = payload(&doc, &source, renames, org, format);
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

/// Each line of a page preamble, with its lowercase key and value when the
/// parser accepts it as a page-header property (Markdown `key:: value`, Org
/// `#+KEY: value`) outside every literal container. A fenced or `#+BEGIN_…`
/// example is literal text, never a header property (I-12: the parser owns
/// regions; REG-OG-C5-L03-S2). One parse of `text`, O(text bytes).
fn header_lines(text: &str, org: bool) -> Vec<(&str, Option<(String, String)>)> {
    let regions = crate::conflicts::pre_regions(text, if org { Format::Org } else { Format::Md });
    let mut at = 0;
    text.split_inclusive('\n')
        .map(|raw| {
            let start = at;
            at += raw.len();
            let line = raw.strip_suffix('\n').unwrap_or(raw);
            let line = line.strip_suffix('\r').unwrap_or(line);
            let property = regions
                .container_of(start, at)
                .is_none()
                .then(|| regions.property(start).cloned())
                .flatten();
            (line, property)
        })
        .collect()
}

#[cfg(test)]
mod header_property_tests {
    use super::header_lines;

    fn property(text: &str, org: bool) -> Option<(String, String)> {
        header_lines(text, org)
            .into_iter()
            .next()
            .and_then(|(_, p)| p)
    }

    #[test]
    fn merge_recognizes_unicode_page_headers_in_both_formats() {
        assert_eq!(
            property("klíč:: hodnota", false),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+klíč: hodnota", true),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+a.b/c: value", true),
            Some(("a.b/c".into(), "value".into()))
        );
    }

    #[test]
    fn a_fenced_example_is_never_a_header_property() {
        let lines = header_lines("```\nnote:: keep me\n```\nkey:: v\n", false);
        assert_eq!(lines.len(), 4);
        assert!(lines[..3].iter().all(|(_, p)| p.is_none()), "{lines:?}");
        assert_eq!(lines[3].1, Some(("key".into(), "v".into())));
    }
}

/// The survivor's new header (when it changes) and the blocks `source` adds,
/// with `renames` applied to the source.
fn payload(
    survivor: &PageDto,
    source: &PageDto,
    renames: Option<&HashMap<String, String>>,
    org: bool,
    format: tine_core::config::FileNameFormat,
) -> (Option<String>, Vec<tine_core::model::BlockDto>) {
    let mut source = source.clone();
    if let Some(renames) = renames {
        rename_doc(&mut source, renames, org, format);
    }
    let mut new_header = None;
    let mut blocks = Vec::new();
    if let Some(pre) = source.pre_block.as_deref() {
        let mut header: Vec<(String, Option<(String, String)>)> = survivor
            .pre_block
            .as_deref()
            .map(|kept| {
                header_lines(kept, org)
                    .into_iter()
                    .map(|(line, property)| (line.to_owned(), property))
                    .collect()
            })
            .unwrap_or_default();
        let mut moved = Vec::new();
        for (line, property) in header_lines(pre, org) {
            let Some((key, value)) = property else {
                moved.push(line);
                continue;
            };
            let clash = header
                .iter()
                .position(|(_, kept)| kept.as_ref().is_some_and(|(k, _)| *k == key));
            let equal = clash.is_some_and(|at| {
                header[at]
                    .1
                    .as_ref()
                    .is_some_and(|(_, v)| v.trim() == value.trim())
            });
            match clash {
                Some(_) if equal => {}
                None if key != "title" => header.push((line.to_owned(), Some((key, value)))),
                Some(at) if key == "alias" => {
                    let kept_value = header[at]
                        .1
                        .as_ref()
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    // Members split like the reference evidence and OG
                    // `sep-by-comma`: `,` or `，` (C3Y Y4).
                    let known: HashSet<String> = kept_value
                        .split(refs::is_linkable_property_separator)
                        .map(refs::normalize)
                        .collect();
                    let extra: Vec<&str> = value
                        .split(refs::is_linkable_property_separator)
                        .map(str::trim)
                        .filter(|alias| {
                            !alias.is_empty() && !known.contains(&refs::normalize(alias))
                        })
                        .collect();
                    if !extra.is_empty() {
                        let join = |start: &str| {
                            extra
                                .iter()
                                .fold(start.trim_end().to_owned(), |line, alias| {
                                    format!("{line}, {alias}")
                                })
                        };
                        header[at] = (join(&header[at].0), Some((key, join(&kept_value))));
                    }
                }
                _ => moved.push(line),
            }
        }
        if !header.is_empty() {
            new_header = Some(lines(header.iter().map(|(line, _)| line.as_str())));
        }
        if moved.iter().any(|line| !line.trim().is_empty()) {
            blocks.push(tine_core::model::BlockDto {
                raw: lines(moved.into_iter()),
                ..Default::default()
            });
        }
    }
    blocks.extend(source.blocks);
    (new_header, blocks)
}

#[cfg(test)]
#[path = "pages_snapshot_tests.rs"]
mod snapshot_tests;
