//! Page rename, stray rescue, merge and deletion (a deletion is the page
//! host's, STEP3 §12, after this client's identity checks). The caller names
//! the operation; this client resolves identities and hands the page host one
//! operation (or commits a guarded non-page transaction). It needs no graph path, lock,
//! cache state, or write protocol.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use tine_core::model::{Format, PageKind};
use tine_core::refs;
#[path = "pages_rename_plan.rs"]
mod rename_plan;
pub use rename_plan::{RenameBlocked, RenameChoice, RenamedPage};
#[path = "pages_rename_offer.rs"]
mod rename_offer;
pub use rename_offer::{rename_offer, rename_page_as, RenameOffer};
#[path = "pages_refusal.rs"]
mod refusal;
#[path = "pages_rename_report.rs"]
mod rename_report;
use refusal::unsaved_error;
pub use refusal::{save_page_unchanged, MergeRefused, RewriteBlocked};
use rename_plan::{plan_rename, RenamePlan};
use rename_report::{site_exceptions, survivor_check, IncompleteRename, StepChecks};
pub use rename_report::{
    CheckRefused, Disposition, RenameOutcome, RenameReport, SiteException, TouchedPage,
};

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
use tine_store::{
    Area, FileId, FileRev, Input, LoadError, PageHost, PageId, PageOperation, PageRead, RenameMap,
    Resolved, Store, StoreError, TitleRebind,
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

/// The view after one stat walk of graph text and config
/// (`Depth::PageStamps`; the walk waits for Ready, so the view is never the
/// launch checkpoint). A page operation takes it once (a rename once per
/// attempt, in `multi_rename`'s plan reuse check). The walk is the only
/// proof that the plan sees an external edit or sync delivery the watcher
/// has not published yet (its 200 ms debounce, a watch that reports nothing
/// on some network and FUSE mounts, Poll mode's 3 s): a referrer, a twin or a
/// claimant anywhere in the graph (B20d). After the walk, a planned page's
/// edit is refused by the commit's byte check, and a change the watcher
/// publishes before admission by the host's bound-view admission (B2); both
/// replan. A page that becomes a referrer, twin or claimant after the walk
/// and is not published by admission is not seen, the window the walk always
/// had (`docs/storage-contract.md`). Assets and `custom.css` are not walked:
/// no page plan reads them. Cost O(P stamps).
fn refreshed_view(store: &Store) -> io::Result<tine_store::WholeGraph> {
    store
        .refresh(tine_store::Depth::PageStamps)
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
///
/// Deliberately not the store's page-name answer (census Q2): that answer
/// names the files the view admits, and these files are exactly the ones it
/// does not. The question here is "would this file's name, decoded with the
/// graph's codec, claim `name`", asked of the file name alone so that a page
/// create never duplicates an identity a non-portable file still holds.
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

/// Every page with a block the rename rewrites although no reference names
/// the page (so no referrers row selects it): a `{{query}}` page-name argument
/// (D-20261031-CMQ), a `[[link]]` in another macro's argument (cloze,
/// renderer, plugin macros) or an Org `[[file:…]]` link (its label, not its
/// file, is the reference). Found by scanning the planning view's parsed
/// blocks and pre-blocks in memory, with no index (D-10; no persisted record,
/// I-25): O(B) substring checks, plus the rename's own rewrite of each text
/// that may hold a target. A page is selected when one of its texts needs
/// a write or an O7 report, including an unspellable target left unchanged.
///
/// A macro's target spells its page's name in the text itself, so a text
/// holding `{{` is rewritten (one parse) only when its folded text contains a
/// renamed page's key. That is a necessary condition except for an escaped
/// spelling (a `\` in the text, EDN `\"`; a quote in the name, SQL `''`),
/// which takes the rewrite unfiltered, as an Org `[[file:` link (its encoded
/// file stem) always does. Realistic-10k: ~10k embed and query blocks, of
/// which a rename parses the few that spell the name.
fn rewrite_target_pages(
    graph: &tine_store::WholeGraph,
    renames: &HashMap<String, String>,
    format: refs::RenameFormat<'_>,
) -> Vec<PageId> {
    // A key containing another key is found through that one.
    let mut keys: Vec<&str> = renames.keys().map(String::as_str).collect();
    keys.sort_by_key(|key| key.len());
    let mut needles: Vec<&str> = Vec::new();
    for key in keys {
        if !needles.iter().any(|needle| key.contains(needle)) {
            needles.push(key);
        }
    }
    let quoted = needles.iter().any(|key| key.contains(['\'', '"']));
    let spells_a_key = |raw: &str| {
        quoted || raw.contains('\\') || {
            let folded = refs::page_key(raw);
            needles.iter().any(|key| folded.contains(key))
        }
    };
    graph
        .corpus(None, false)
        .pages
        .into_iter()
        .filter(|page| {
            let org = Format::from_path(page.id.as_str().as_ref()) == Format::Org;
            let may_hold = |raw: &str| {
                (raw.contains("{{") && spells_a_key(raw)) || (org && raw.contains("[[file:"))
            };
            if let Some(pre) = page.document.pre_block.as_deref() {
                if may_hold(pre)
                    && refs::rename_target_needs_report_or_write(pre, org, true, renames, format)
                {
                    return true;
                }
            }
            let mut stack: Vec<_> = page.document.roots.iter().collect();
            while let Some(block) = stack.pop() {
                let raw = block.raw();
                if may_hold(raw)
                    && refs::rename_target_needs_report_or_write(raw, org, false, renames, format)
                {
                    return true;
                }
                stack.extend(block.children.iter());
            }
            false
        })
        .map(|page| page.id)
        .collect()
}

/// One text an unsaved page holds, as the window asks about it
/// ([`rename_would_change`]): its pre-block, or one block's raw body.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameProbeText {
    pub raw: String,
    pub org: bool,
    /// The page's pre-block (its page properties).
    pub pre_block: bool,
}

/// Per text, whether renaming `old` to `new` would rewrite it: the rename's
/// own name map (`old` and every namespace descendant a file owns, as
/// [`plan_rename`] pairs them) and its own rewrite, so the window's check
/// before a rename that must leave a stuck page alone (GH #535) cannot
/// disagree with what the rename writes. Read-only: one view, one inventory
/// of `old`'s namespace, then O(text bytes) per text.
pub fn rename_would_change(
    store: &Store,
    old: &str,
    new: &str,
    texts: &[RenameProbeText],
) -> io::Result<Vec<bool>> {
    let graph = view(store)?;
    let old_key = refs::normalize(old);
    let owned = graph.inventory(tine_store::InventoryScope::FilesAtOrUnder(&old_key));
    let mut renames: HashMap<String, String> = owned
        .0
        .iter()
        .filter(|entry| !entry.is_journal)
        .map(|entry| {
            let to = format!("{new}{}", namespace_suffix(&entry.name, &old_key));
            (refs::normalize(&entry.name), to)
        })
        .collect();
    renames.insert(old_key, new.to_owned());
    let config = store.config();
    let days = config.config.journal_format();
    let format = refs::RenameFormat::new(config.file_name_format, &days);
    Ok(texts
        .iter()
        .map(|text| {
            let after = if text.pre_block {
                refs::rename_pre_block(&text.raw, text.org, &renames, format, None)
            } else {
                refs::rename_block_raw(&text.raw, text.org, &renames, format, None)
            };
            after != text.raw
        })
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
        let mut paths: Vec<_> = ids.iter().map(|id| id.as_str().to_owned()).collect();
        paths.sort_unstable();
        return Err(RewriteBlocked::DuplicateFiles { files: paths }.error());
    }
    if let Some(expected) = expected_path.filter(|path| !path.trim().is_empty()) {
        if ids.first().is_none_or(|id| id.as_str() != expected) {
            return Err(RewriteBlocked::StaleSource {
                file: expected.into(),
            }
            .error());
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
/// `Pending` deletion to publish (§4.4). Cost one stat walk of the graph's
/// text files (O(P), [`refreshed_view`]) plus the file's bytes.
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
                Err(StoreError::NotFound(_)) => {}
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
/// `alias::` member follows too (D-20261105-CM13, deliberate OG difference). Non-UTF-8
/// candidates refuse (C3W W2), as does a page listed unreadable that may hold a
/// renamed name; a non-round-tripping Org referrer
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
        RenameChoice::Rename,
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
/// The page host (`host`) decides by its own unsaved input as well: every
/// rename is one host operation of every page it writes (S2), checked before
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
    rename_page_as(
        store,
        host,
        old,
        new,
        expected_path,
        merge_into.map_or(RenameChoice::Rename, RenameChoice::MergeInto),
        unsaved_paths,
    )
}

/// What [`follow_page_identity`] did to `config.edn`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct IdentityFollow {
    /// The page's name now, when it differs from the old one (by page key).
    pub name: Option<String>,
    /// The new `:default-home` name, when it moved.
    pub home_page: Option<String>,
    /// The favorites after the change, whenever the change is confirmed
    /// (also when another writer already moved them), so a window adopts
    /// the backend's list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorites: Option<crate::config::FavoritesAfter>,
    /// Why a move was not written (see [`RenameReport::config_skipped`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_skipped: Option<String>,
}

/// A page's name changed without a rename operation: its `title::` was edited
/// (in Tine, by the CLI, or in another editor), so the file at `path` that
/// was named `old` is now named otherwise. Home and favorites follow it
/// through the same config.edn step and answerer as a rename (CM step 2,
/// FAV-RETITLE ruling; OG follows only its page-title rename). Nothing moves
/// when the file at `path` still answers to `old`, is a journal or is gone,
/// or when `old` still names some page or alias (a stored favorite still
/// opens a page). Callers: the CLI owner after a page write, the window after
/// it observed the identity change. Cost: one published-view inventory scan,
/// one stat walk if the change is not yet published, plus the config step
/// (O(config.edn bytes)).
pub fn follow_page_identity(store: &Store, path: &str, old: &str) -> io::Result<IdentityFollow> {
    let named = |graph: &tine_store::WholeGraph| {
        graph
            .inventory(tine_store::InventoryScope::All)
            .0
            .iter()
            .find(|entry| {
                !entry.is_journal && physical(&entry.target).iter().any(|id| id.as_str() == path)
            })
            .map(|entry| entry.name.clone())
    };
    let mut graph = view(store)?;
    let mut name = named(&graph);
    if name
        .as_deref()
        .is_none_or(|name| refs::same_page(name, old))
    {
        graph = refreshed_view(store)?;
        name = named(&graph);
    }
    let Some(name) = name.filter(|name| !refs::same_page(name, old)) else {
        return Ok(IdentityFollow::default());
    };
    if !matches!(graph.resolve(old, false), Resolved::Absent { .. }) {
        return Ok(IdentityFollow::default());
    }
    // A title edit is not a rename request (model r6 Scope): only the one
    // name follows, exactly.
    let old_key = refs::normalize(old);
    let renamed = |stored: &str| (refs::normalize(stored) == old_key).then(|| name.clone());
    let step = commit_config(store, &renamed, None, None)?;
    let current = store.config();
    let favorites = step.favorites.or_else(|| {
        Some(crate::config::FavoritesAfter {
            names: current.config.favorites.clone(),
            page: current.config.favorites_page.clone(),
        })
    });
    Ok(IdentityFollow {
        name: Some(name),
        home_page: step.home,
        favorites,
        config_skipped: step.skipped,
    })
}

#[allow(clippy::too_many_arguments)]
fn rename_page_after_inventory(
    store: &Store,
    host: &PageHost,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    choice: RenameChoice<'_>,
    unsaved_paths: &[String],
    #[cfg(test)] after_inventory: impl Fn(),
) -> io::Result<RenameReport> {
    let old = old.trim();
    let new = new.trim();
    if new.is_empty() {
        return Err(error(io::ErrorKind::InvalidInput, "empty name"));
    }
    if old.is_empty() {
        return Ok(RenameReport::unchanged());
    }
    let plan = || {
        plan_rename(
            store,
            old,
            new,
            expected_path,
            choice,
            #[cfg(test)]
            &after_inventory,
        )
    };
    crate::retry_on_conflict("page changed repeatedly during rename", || {
        // Planned from the published view; the attempt's walk runs in
        // `multi_rename`, after the flush. A refusal (or nothing to do) is
        // never decided on an unwalked view: walk and plan again (`unchanged` needs none).
        let first = match plan() {
            Ok(first) if !first.map.0.is_empty() || first.unchanged => first,
            _ => {
                refreshed_view(store)?;
                plan()?
            }
        };
        if first.map.0.is_empty() {
            return Ok(Some(RenameReport::unchanged()));
        }
        // One host operation of every page the plan writes (host-only-writer
        // S2, DESIGN §3.1, §3.5), planned again until the host applies it:
        // flush first, except an interrupted title completion, which refuses
        // unsaved input as today. `config.edn`'s home page is a post step.
        multi_rename(store, host, old, first, &plan, unsaved_paths)
    })
}

/// Every rename (one page, a namespace, a merge or references only)
/// as one host operation, then the home page in `config.edn` as its
/// own transaction once the operation applied (DESIGN §3.4: skipped when it
/// is uncertain). A plan that changed its unsaved-input policy or selection
/// shape replans (`Ok(None)`).
fn multi_rename(
    store: &Store,
    host: &PageHost,
    old: &str,
    mut first: RenamePlan,
    plan: &dyn Fn() -> io::Result<RenamePlan>,
    unsaved_paths: &[String],
) -> io::Result<Option<RenameReport>> {
    let single = first.single(store)?.is_some();
    let input = if single { Input::Refuse } else { first.input() };
    let before_flush = host.inputs().map_err(|_| rename_busy())?;
    let mut inputs = single.then(|| before_flush.clone());
    if let Some(inputs) = &inputs {
        first.include_inputs(store, inputs)?;
    }
    let expected = first.pages(store)?;
    let graph = first.graph.clone();
    let refused = |page: &str| unsaved_error(&graph, page);
    let mut initial = Some(first);
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
            // Reuse only while view/config and held input still match. The
            // first call takes the attempt's one walk (`refreshed_view`),
            // after the flush and as near the host's admission as planning
            // gets: the first plan read the published view, so it is reused
            // only if the walk published nothing. A replan reads the
            // published view.
            let retained = match initial.take() {
                Some(first) => {
                    let current = refreshed_view(store)?;
                    (current.rev() == first.graph.rev()
                        && std::sync::Arc::ptr_eq(&store.config().config, &first.config.config)
                        && host.inputs().is_ok_and(|now| now == before_flush))
                    .then_some(first)
                }
                None => None,
            };
            let reused = retained.is_some();
            let mut plan = match retained {
                Some(first) => first,
                None => plan()?,
            };
            let now_single = plan.single(store)?.is_some();
            let now_input = if now_single {
                Input::Refuse
            } else {
                plan.input()
            };
            if now_single != single || now_input != input {
                replan.set(true);
                return Ok(None);
            }
            // The commit decides on the bytes `pages` read.
            if !reused {
                inputs = if single {
                    Some(host.inputs().map_err(|_| rename_busy())?)
                } else {
                    None
                };
                if let Some(inputs) = &inputs {
                    plan.include_inputs(store, inputs)?;
                }
                plan.pages(store)?;
            }
            match rename_transaction(store, old, plan, unsaved_paths)? {
                RenameTransaction::Unchanged { report, home } => {
                    if inputs
                        .as_ref()
                        .is_some_and(|before| !host.inputs().is_ok_and(|now| now == *before))
                    {
                        replan.set(true);
                        return Ok(None);
                    }
                    built.replace(Some((report, home)));
                    Ok(None)
                }
                RenameTransaction::Writes {
                    tx,
                    report,
                    merge,
                    home,
                } => {
                    let mut op = crate::retained::plan(tx)?;
                    op.separate_case_entries = single;
                    op.inputs = inputs.clone();
                    // A merge's survivor holds the source's blocks: a move's
                    // order, as `merge_pages` declares it.
                    if let Some((src, dst)) = merge {
                        op.moves.push((dst, src));
                    }
                    built.replace(Some((report, home)));
                    Ok(Some(op))
                }
            }
        },
    )
    .map_err(rename_operation_error)?;
    if replan.get() {
        return Ok(None);
    }
    let (mut report, (lookup, merged_old, days)) =
        built.into_inner().expect("planned before the operation");
    commit_config_after(store, &mut report, &lookup, merged_old.as_ref(), &days)?;
    Ok(Some(report))
}

/// What the settings step needs (O8): the rename's lookup, the merged
/// source, whose home `merge-pages!` keeps, and the day format (days are
/// pinned; after a merge, one entry per page or day).
type SettingsStep = (
    HashMap<String, String>,
    Option<String>,
    tine_core::date::JournalFormat,
);

/// What a rename writes, as a store transaction of its page steps. Both
/// variants are an admitted rename, so both carry its settings step (O8
/// follows every successful rename, not only one that changed page bytes).
enum RenameTransaction<'s> {
    /// No page bytes to write (every referrer already reads the new name,
    /// cannot spell it, or carries VCS conflict markers).
    Unchanged {
        report: RenameReport,
        home: SettingsStep,
    },
    Writes {
        tx: tine_store::PagePlan<'s>,
        /// Without the home page: `config.edn` is written after the pages.
        report: RenameReport,
        /// A merge's `(source, survivor)`.
        merge: Option<(PageId, PageId)>,
        home: SettingsStep,
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
        days,
        map,
        lookup,
        moves,
        mut renamed,
        moved_titles_to_rebind,
        merge,
        ref_merge,
        candidates,
        mut reads,
        skipped,
        mut rewritten,
        unchanged: _,
    } = plan;
    let map = Arc::new(map); // shared by the steps: one move per namespace child
    let mut tx = store.page_plan(Some(tine_store::EditKind::RenamePage));
    // Every step, rewrite or not, commits only in the planning context.
    tx.expect_view(&graph);
    // Crash order (I-2): survivor, then referrer rewrites, then namespace
    // descendant moves, then the source trash, then config.edn LAST. The
    // survivor is queued first because referrer rewrites are queued as the
    // candidates are read below.
    let rewritten_names = std::cell::RefCell::new(HashSet::new());
    let merged = match &merge {
        Some((src, dst)) => Some(merged_survivor(
            store,
            src,
            dst,
            Some(MergeNames {
                links: &lookup,
                rewritten: &rewritten_names,
            }),
        )?),
        None => None,
    };
    // O7's site exceptions, from the proposals the steps write (no walk of
    // their own): the survivor's two inputs, then each rewritten file.
    let mut sites = Vec::new();
    if let (Some((src, dst)), Some(survivor)) = (&merge, &merged) {
        for (file, site) in &survivor.proposal.unrealized {
            let path = [dst, src][*file].as_str();
            sites.extend(site_exceptions(path, [site]));
        }
    }
    if let (Some((_, dst)), Some(survivor)) = (&merge, &merged) {
        tx.rewrite_refs_prepared(
            &[
                tine_store::EditKind::RenamePage,
                tine_store::EditKind::InsertBlocks,
            ],
            dst,
            survivor.dst_rev.clone(),
            (survivor.old.clone(), survivor.proposal.text.clone()),
            None,
            config.file_name_format,
            survivor.proposal.trace.clone(),
        );
    }
    // The model's check of every step's final bytes.
    let checks = StepChecks::new(&graph, old, &renamed, &days, config.file_name_format)?;
    let check = |id: &PageId, observed| checks.step(id, observed);
    if let (Some((_, dst)), Some(survivor)) = (&merge, &merged) {
        let page = checks.entry(dst).0;
        tx.check(
            &dst.file(),
            survivor_check(&checks.plan, &survivor.observed, page),
        );
    }
    let name_format = config.file_name_format;
    let spelling = refs::RenameFormat::new(name_format, &days);
    let mut edits = Vec::new();
    // Moved pages' planning reads and proposals, for their move steps.
    let mut carried = HashMap::new();
    let mut skipped: Vec<_> = skipped
        .into_iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    for id in candidates {
        let file = id.file();
        // A candidate the rename cannot read (non-UTF-8, over the size or
        // depth cap: malformed imported/synced content) fails the rename,
        // naming the file. Skipping it (v0.6.5 model.rs 3699) left a moved
        // page under Old while every referrer said New (C3W W2, I-2);
        // master page_rename.rs fails the same way (audit R15-09).
        let planned = reads.contains_key(&id);
        let (content, rev) = match reads.remove(&id) {
            Some(read) => read,
            None => read_text(store, &file)
                .map_err(|e| error(e.kind(), &format!("{}: {e}", file.as_str())))?,
        };
        let syntax = Format::from_path(id.as_str().as_ref());
        let org = syntax == Format::Org;
        let moved = moves.contains_key(&id);
        let rebind = moved_titles_to_rebind.contains(&id);
        let marked = !tine_core::concord_queue::vcs_conflict_markers(&content, syntax).is_empty();
        // Whether the move respells the file's title (the store's title check).
        let own_title = moved
            && (org || marked)
            && std::panic::catch_unwind(|| {
                tine_core::model::page_title_from_header(&content, syntax).is_some_and(|title| {
                    refs::renamed_site_name(&title, &lookup, Some(&days))
                        .is_some_and(|new| org || new != title)
                })
            })
            .map_err(|_| unwritable(store, &id))?;
        // Master a8fd4230d: a file carrying VCS conflict markers is not
        // ours to rewrite (R-VCS-MARKERS; scenario: an external merge or a
        // sync service left it mid-conflict). A referrer stays byte-identical
        // and is reported; a moved one moves verbatim if it needs no rewrite,
        // else the rename refuses (r6 O4). A moved file is rewritten in its
        // move step and a marked one is not queued before this check, so both
        // ask the shared rewriter directly. Every other referrer is queued by
        // the transaction's rewrite, computed once here, reused (GH #623).
        let mut names = HashSet::new();
        let rewrite = |names: &mut HashSet<_>| {
            refs::rename_rewrite_recording(&content, org, &map.0, spelling, Some(names))
        };
        let mut proposed = None;
        let changed = if moved && !marked {
            // The moved page's rewrite, proposed once: its move step reuses
            // it at preflight (base revision, staged bytes and certificate
            // still checked there) instead of walking the page again. One
            // that cannot be certified is left to the move step, which
            // refuses it as before.
            match refs::rename_proposal(&content, org, &lookup, spelling) {
                Ok(proposal) => {
                    names.clone_from(&proposal.rewritten);
                    sites.extend(site_exceptions(id.as_str(), &proposal.unrealized));
                    let changed = proposal.text != content;
                    proposed = Some(proposal);
                    changed
                }
                Err(_) => rewrite(&mut names) != content,
            }
        } else if moved || marked {
            rewrite(&mut names) != content || (!org && marked && own_title)
        } else if rebind {
            let new = match refs::rename_proposal(&content, org, &lookup, spelling) {
                Ok(proposal) => {
                    names.clone_from(&proposal.rewritten);
                    sites.extend(site_exceptions(id.as_str(), &proposal.unrealized));
                    proposal.text
                }
                Err(_) => rewrite(&mut names),
            };
            tx.rewrite_refs(&id, rev.clone(), None, &map, TitleRebind::Own);
            tx.check(&file, check(&id, None));
            new != content
        } else {
            let prepared = match rewritten.remove(&id) {
                Some(p) => Some(p),
                None if !planned => Some(
                    refs::rename_proposal(&content, org, &lookup, spelling)
                        .map_err(|e| MergeRefused::trace(e, id.as_str()))?,
                ),
                None => None,
            };
            if let Some(proposal) = prepared {
                sites.extend(site_exceptions(id.as_str(), &proposal.unrealized));
                names = proposal.rewritten;
                let changed = proposal.text != content;
                if changed {
                    tx.rewrite_refs_prepared(
                        &[tine_store::EditKind::RenamePage],
                        &id,
                        rev.clone(),
                        (content.clone(), proposal.text),
                        Some(&map),
                        name_format,
                        proposal.trace,
                    );
                    tx.check(&file, check(&id, proposal.observed));
                }
                changed
            } else {
                false
            }
        };
        let org_refuses = org && (changed || own_title) && !tine_core::org::org_editable(&content);
        if changed && marked && !moved {
            skipped.push(id.as_str().to_owned());
            continue;
        }
        if org_refuses {
            return Err(read_only_org(id.as_str()));
        }
        if changed && marked {
            return Err(RenameBlocked::ConflictMarkers {
                file: id.as_str().into(),
            }
            .error());
        }
        if marked && rebind {
            // References unchanged, but the interrupted rename's title
            // still needs rebinding in place.
            tx.rewrite_refs(&id, rev.clone(), None, &map, TitleRebind::Own);
            tx.check(&file, check(&id, None));
        }
        if moved || rebind || changed {
            rewritten_names.borrow_mut().extend(names);
            if let Some(proposal) = proposed {
                carried.insert(id.clone(), (content, proposal));
            }
            edits.push((id, rev, !marked));
        }
    }
    let outcome = if merge.is_some() || ref_merge {
        RenameOutcome::Merged
    } else {
        RenameOutcome::Renamed
    };
    skipped.sort_unstable();
    skipped.dedup();
    sites.sort_by(|a: &SiteException, b| (&a.path, a.start).cmp(&(&b.path, b.start)));
    let merged_old = (merge.is_some() || ref_merge).then(|| refs::normalize(old));
    if edits.is_empty() && merged.is_none() {
        // An admitted rename that changes no page bytes (every referrer is
        // left unspellable, already reads the new name, or is conflicted) is
        // still a completed rename: its settings step runs (O8), and its
        // outcome says so.
        return Ok(RenameTransaction::Unchanged {
            report: RenameReport {
                outcome,
                skipped_conflicted_referrers: skipped,
                site_exceptions: sites,
                ..RenameReport::unchanged()
            },
            home: (lookup, merged_old, days),
        });
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
        let to = &moves[&id];
        let mut observed = None;
        let rewrite = match carried.remove(&id).filter(|_| rewrite) {
            Some((old, mut proposal)) => {
                observed = proposal.observed.take();
                tine_store::MoveRewrite::Prepared {
                    renames: Arc::clone(&map),
                    old,
                    proposal,
                    format: name_format,
                }
            }
            None if rewrite => tine_store::MoveRewrite::Rewrite(Arc::clone(&map)),
            None => tine_store::MoveRewrite::Verbatim,
        };
        tx.move_file(&id.file(), rev, to, rewrite);
        tx.check(&id.file(), check(&id, observed));
    }
    if let (Some((src, _)), Some(survivor)) = (&merge, merged) {
        tx.trash(&src.file(), survivor.src_rev, tine_store::TrashIf::Any);
    }
    rename_plan::report_rewritten_names(&mut renamed, rewritten_names.into_inner(), old, &lookup);
    Ok(RenameTransaction::Writes {
        tx,
        report: RenameReport {
            outcome,
            touched,
            renamed,
            skipped_conflicted_referrers: skipped,
            site_exceptions: sites,
            home_page: None,
            favorites: None,
            config_skipped: None,
        },
        merge,
        home: (lookup, merged_old, days),
    })
}

/// The config.edn step after published page changes: its failure carries the
/// report of what was already applied (`RenameReport::applied_before_error`).
fn commit_config_after(
    store: &Store,
    report: &mut RenameReport,
    lookup: &HashMap<String, String>,
    merged_old: Option<&String>,
    days: &tine_core::date::JournalFormat,
) -> io::Result<()> {
    // O8: settings follow O1, the rename's one name map (namespace prefix,
    // days pinned), so a name under the source that only a setting holds
    // follows too.
    let renamed = |name: &str| {
        refs::renamed_site_name(name, lookup, Some(days)).map(std::borrow::Cow::into_owned)
    };
    match commit_config(store, &renamed, merged_old, merged_old.map(|_| days)) {
        Ok(step) => {
            report.home_page = step.home;
            report.favorites = step.favorites;
            report.config_skipped = step.skipped;
            Ok(())
        }
        Err(source) => Err(io::Error::new(
            source.kind(),
            IncompleteRename {
                report: report.clone(),
                source,
            },
        )),
    }
}

/// A rename's `config.edn` step: `logseq/config.edn` (the meta area, never a
/// page file) in ONE guarded transaction after the host's operation, so no
/// page-host fence applies. It moves `:default-home` (a merged source,
/// `merged_old`, keeps it: OG `merge-pages!`), `:favorites` (one entry per
/// page after a merge, `merged_days`) and `:tine/favorites-page` together
/// (`config::config_after_rename`, the one answerer), all or none.
/// `renamed` is the stored name's new spelling. Conflicts retry with fresh
/// bytes.
fn commit_config(
    store: &Store,
    renamed: &dyn Fn(&str) -> Option<String>,
    merged_old: Option<&String>,
    merged_days: Option<&tine_core::date::JournalFormat>,
) -> io::Result<crate::config::ConfigAfterRename> {
    let home_renamed = |home: &str| {
        (merged_old != Some(&refs::normalize(home)))
            .then(|| renamed(home))
            .flatten()
    };
    crate::retry_on_conflict("config.edn changed repeatedly during rename", || {
        let mut step =
            crate::config::config_after_rename(store, &home_renamed, renamed, merged_days);
        let Some((id, rev, bytes)) = step.write.take() else {
            return Ok(Some(step));
        };
        let mut tx = store.transaction(None);
        tx.replace(&id, rev, bytes);
        Ok(crate::commit_retry(tx.commit())?.then_some(step))
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

/// O4's unchanged-save restriction, naming the input file.
fn read_only_org(file: &str) -> io::Error {
    RewriteBlocked::ReadOnlyOrg {
        file: tine_core::graph_paths::normalize_graph_path(file),
    }
    .error()
}

/// The refusal of a rename that would change a page it cannot rewrite: a
/// read-only Org file, or one that is not UTF-8.
fn unwritable(_store: &Store, page: &PageId) -> io::Error {
    if Format::from_path(page.as_str().as_ref()) == Format::Org {
        return read_only_org(page.as_str());
    }
    RewriteBlocked::Unindexed {
        file: page.as_str().into(),
    }
    .error()
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

/// Rescue a stray page or journal file into a uniquely named normal page.
/// Its bytes are unchanged and inbound references are not rewritten, matching
/// v0.6.5 model.rs 1779. One host operation of the source and
/// the target (DESIGN §3.1): their unsaved input is saved first. Cost one
/// stat walk and one listing of the pages area (O(P)), then O(source bytes)
/// per try; four tries maximum.
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
    let taken = || {
        error(
            io::ErrorKind::AlreadyExists,
            "a page with that name already exists",
        )
    };
    // Once per operation (`refreshed_view`): the walk, and the
    // listing that alone sees a retained non-portable legacy filename
    // (`pages/A:B.md`), which is no view claimant but OG loads as that page:
    // refuse like a live claimant (external-editor / multi-device graph;
    // master 46a0e8c27). A try then resolves on the published view; the
    // host rechecks the destination's name slot at admission.
    refreshed_view(store)?;
    if retained_legacy_page_identity_exists(store, name)? {
        return Err(taken());
    }
    let unclaimed = || {
        if !existing(view(store)?.resolve(name, false)).is_empty() {
            return Err(taken());
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
        return Err(RewriteBlocked::SameFile {
            file: src_rel.into(),
        }
        .error());
    }
    let src_id = store
        .as_page(&src)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let dst_id = store
        .as_page(&dst)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let transaction = |survivor: &Survivor| {
        let mut tx = store.page_plan(Some(tine_store::EditKind::InsertBlocks));
        let config = store.config();
        tx.rewrite_refs_prepared(
            &[
                tine_store::EditKind::InsertBlocks,
                tine_store::EditKind::DeletePage,
            ],
            &dst_id,
            survivor.dst_rev.clone(),
            (survivor.old.clone(), survivor.proposal.text.clone()),
            None,
            config.file_name_format,
            survivor.proposal.trace.clone(),
        );
        // A merge that renames nothing: the model's empty plan.
        let plan = Arc::new(tine_store::CheckPlan {
            old: None,
            destination: String::new(),
            days: config.config.journal_format(),
            names: config.file_name_format,
        });
        tx.check(
            &dst,
            survivor_check(&plan, &survivor.observed, String::new()),
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
    old: String,
    proposal: merge_payload::MergeProposal,
    src_rev: FileRev,
    dst_rev: FileRev,
    /// The model's readings of the source and the survivor's old text.
    observed: (
        Arc<tine_core::cm_check::Content>,
        Arc<tine_core::cm_check::Content>,
    ),
}

/// Realize the survivor from revision-guarded input bytes; no DTO serialization.
fn merged_survivor(
    store: &Store,
    src: &PageId,
    dst: &PageId,
    renames: Option<MergeNames<'_>>,
) -> io::Result<Survivor> {
    let org = Format::from_path(src.as_str().as_ref()) == Format::Org;
    if org != (Format::from_path(dst.as_str().as_ref()) == Format::Org) {
        return Err(RewriteBlocked::DifferentFormats {
            source: src.as_str().into(),
            target: dst.as_str().into(),
        }
        .error());
    }
    let (src_text, src_rev) = read_text(store, &src.file())?;
    let (dst_text, dst_rev) = read_text(store, &dst.file())?;
    let format = Format::from_path(src.as_str().as_ref());
    for (id, text) in [(src, &src_text), (dst, &dst_text)] {
        if tine_core::concord_queue::has_vcs_anchor(text.as_bytes())
            && !tine_core::concord_queue::vcs_conflict_markers(text, format).is_empty()
        {
            return Err(RewriteBlocked::MergeConflictMarkers {
                file: id.as_str().into(),
            }
            .error());
        }
    }
    if org {
        for (id, text) in [(src, &src_text), (dst, &dst_text)] {
            if !tine_core::org::org_editable(text) {
                return Err(read_only_org(id.as_str()));
            }
        }
    }
    // Parsed from the bytes whose revisions guard the write (A-V4b).
    let page = |id, text: &str| store.page_of(id, text.as_bytes()).map_err(store_error);
    let source = page(src, &src_text)?.doc;
    let doc = page(dst, &dst_text)?.doc;
    let config = store.config();
    let days = config.config.journal_format();
    let empty = HashMap::new();
    let proposal = merge_payload::propose(
        &doc,
        &dst_text,
        &source,
        &src_text,
        renames.map_or(&empty, |n| n.links),
        org,
        refs::RenameFormat::new(config.file_name_format, &days),
    )
    .map_err(|e| MergeRefused::error(e, src, dst))?;
    if let Some(names) = renames {
        names
            .rewritten
            .borrow_mut()
            .extend(proposal.rewritten.iter().cloned());
    }
    // The model check's readings of both inputs. The merge writer reads them
    // through the page DTO, not the parser's document, so they are read here.
    let read = |text: &str| {
        Arc::new(tine_core::cm_check::observe(
            text,
            format,
            config.file_name_format,
        ))
    };
    Ok(Survivor {
        observed: (read(&src_text), read(&dst_text)),
        old: dst_text,
        proposal,
        src_rev,
        dst_rev,
    })
}

/// A merge's selected-member substitution, passed to the shared reference
/// encoder.
#[derive(Clone, Copy)]
struct MergeNames<'a> {
    links: &'a HashMap<String, String>,
    rewritten: &'a std::cell::RefCell<HashSet<(String, String)>>,
}

pub mod merge_payload;

#[cfg(test)]
#[path = "pages_snapshot_tests.rs"]
mod snapshot_tests;

#[cfg(test)]
#[path = "pages_rename_cost_tests.rs"]
mod rename_cost_tests;
