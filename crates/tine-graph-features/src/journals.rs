//! Journal feed, duplicate-day reconciliation, and filename migration. All
//! reads use the store's area inventory; writes are guarded transactions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io;

use tine_core::date::{JournalDate, JournalFormat};
use tine_core::model::{JournalConflict, JournalFile};
use tine_store::{Area, Day, FileEntry, FileId, Input, PageHost, PageId, Store, StoreError, Why};

use crate::{store_error, tx_error};

/// A day-based page of the journal feed. The cursor records the last examined
/// day, including a journal that disappeared between inventory and read.
pub struct FeedPage<T> {
    pub pages: Vec<T>,
    /// Referenced future days, nearest first, first request only.
    pub upcoming: Vec<T>,
    pub next_before_day: Option<i64>,
    pub done: bool,
    pub as_of_day: i64,
    /// `path: reason` for every journal this request skipped because it could
    /// not be listed or read (one bad file never blocks the feed, I-22; the
    /// caller reports these, I-2). Empty in the common case.
    pub unreadable: Vec<String>,
    /// File-backed days are usable; referenced-only days await the graph.
    pub references_pending: bool,
}

/// What one feed row load found.
enum Loaded<T> {
    Page(T),
    /// Deleted since the inventory: skipped silently, as before.
    Gone,
    /// This file cannot be read or parsed; skipped and reported with its reason.
    Unreadable(String),
}

fn collect_feed_page<T, F>(
    entries: Vec<(Day, PageId)>,
    limit: usize,
    before_day: Option<i64>,
    as_of_day: i64,
    load: F,
) -> Result<FeedPage<T>, io::Error>
where
    F: FnMut(&PageId) -> Result<Loaded<T>, io::Error>,
{
    collect_feed_iter(entries.into_iter(), limit, before_day, as_of_day, load)
}

fn collect_feed_iter<T>(
    entries: impl Iterator<Item = (Day, PageId)>,
    limit: usize,
    before_day: Option<i64>,
    as_of_day: i64,
    mut load: impl FnMut(&PageId) -> Result<Loaded<T>, io::Error>,
) -> Result<FeedPage<T>, io::Error> {
    let mut candidates = entries
        .filter(|(day, _)| before_day.is_none_or(|before| day.0 < before))
        .peekable();
    if limit == 0 {
        let done = candidates.peek().is_none();
        return Ok(FeedPage {
            pages: Vec::new(),
            upcoming: Vec::new(),
            next_before_day: None,
            done,
            as_of_day,
            unreadable: Vec::new(),
            references_pending: false,
        });
    }
    let mut out = Vec::new();
    let mut unreadable = Vec::new();
    let mut last_examined = None;
    while let Some((day, id)) = candidates.next() {
        last_examined = Some(day.0);
        match load(&id)? {
            Loaded::Page(value) => out.push(value),
            Loaded::Gone => {}
            Loaded::Unreadable(reason) => {
                tine_core::diag_line::diagnostic_line(
                    "Tine unreadable file producer=journal-feed-read",
                );
                unreadable.push(format!("{}: {reason}", id.as_str()));
            }
        }
        if out.len() == limit {
            break;
        }
    }
    let done = candidates.peek().is_none();
    Ok(FeedPage {
        pages: out,
        upcoming: Vec::new(),
        next_before_day: if done { None } else { last_examined },
        done,
        as_of_day,
        unreadable,
        references_pending: false,
    })
}

/// One feed identity and body; `doc.rev` is absent for a virtual journal day.
pub struct FeedRead {
    pub id: PageId,
    pub doc: tine_core::model::PageDto,
}

fn feed_read(read: tine_store::PageRead) -> FeedRead {
    let mut doc = read.doc;
    doc.rev = Some(read.rev.into());
    FeedRead { id: read.id, doc }
}

fn virtual_day(store: &Store, day: Day) -> FeedRead {
    let config = store.config();
    let title = format(store).title(JournalDate::from_ordinal(day.0));
    FeedRead {
        id: store.journal_id(day, true),
        doc: tine_core::model::PageDto {
            name: title.clone(),
            title,
            kind: tine_core::model::PageKind::Journal,
            pre_block: None,
            blocks: Vec::new(),
            rev: None,
            format: config.preferred_format,
            read_only: false,
            guide: false,
        },
    }
}

fn load_feed_read(
    store: &Store,
    id: &PageId,
    referenced: impl FnOnce() -> Option<Day>,
) -> io::Result<Loaded<FeedRead>> {
    Ok(match store.page(id) {
        Ok(read) => Loaded::Page(feed_read(read)),
        Err(StoreError::NotFound(_)) => {
            referenced().map_or(Loaded::Gone, |day| Loaded::Page(virtual_day(store, day)))
        }
        Err(StoreError::Closed) => return Err(store_error(StoreError::Closed)),
        Err(error) => Loaded::Unreadable(store_error(error).to_string()),
    })
}

/// Page file-backed journal days immediately, always leaving graph-wide
/// additions pending for a separate completion request, even on a Ready graph.
/// Unreadable files are skipped and named; store/index failures fail the read.
/// First-page cost O((J + D) log(J + D) + R + returned page bytes), plus indexed
/// alias lookups for date-shaped names on completion. Loaded scroll steps seek
/// the day index and visit bounded windows; reference-name enumeration remains
/// O(R log D). J is journal files, R reference names, D referenced dates.
/// No reference-content scan or graph file write.
/// Before a name index exists, active selection also discovers ordinary
/// preambles once per invalidation (O(P)); it does not wait for graph parsing.
#[cfg(any(test, feature = "test-faults"))]
pub fn feed_page(
    store: &Store,
    limit: usize,
    before_day: Option<i64>,
) -> Result<FeedPage<FeedRead>, io::Error> {
    feed_page_through(store, limit, before_day, None, false)
}

/// As `feed_page`, extending the window through an already displayed day.
/// Late referenced days can be inserted without dropping the visible suffix.
/// `include_references` waits via the ordinary whole-graph read; false never
/// waits for that parse, using the available name indexes for active selection.
/// Only returned journal bodies are read in either mode.
pub fn feed_page_through(
    store: &Store,
    limit: usize,
    before_day: Option<i64>,
    through_day: Option<i64>,
    include_references: bool,
) -> Result<FeedPage<FeedRead>, io::Error> {
    let as_of_day = JournalDate::today().ordinal_key();
    let graph = if include_references {
        Some(
            store
                .whole_graph()
                .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))?,
        )
    } else {
        None
    };
    if let (Some(graph), Some(before)) = (&graph, before_day) {
        return indexed_feed_page(store, graph, limit, before, through_day, as_of_day);
    }
    // First-page and file-only requests retain listing failures and discover
    // files that no watcher publication has named yet.
    let (entries, listing_unreadable) = feed_days_through(store, Day(as_of_day))?;
    let referenced: std::collections::BTreeSet<_> = graph
        .as_ref()
        .map(|graph| {
            graph
                .inventory(tine_store::InventoryScope::ReferencedJournals)
                .0
                .iter()
                .filter_map(|entry| entry.day)
                .collect()
        })
        .unwrap_or_default();
    let mut days: BTreeMap<_, _> = entries.into_iter().collect();
    for day in referenced.iter().filter(|day| day.0 <= as_of_day) {
        if let Some(id) = active_journal_id(store, *day) {
            days.entry(*day).or_insert(id);
        }
    }
    let by_id: BTreeMap<_, _> = days.iter().map(|(day, id)| (id.clone(), *day)).collect();
    let entries: Vec<_> = days.into_iter().rev().collect();
    let limit = through_day.map_or(limit, |through| {
        limit.max(
            entries
                .iter()
                .filter(|(day, _)| {
                    day.0 >= through && before_day.is_none_or(|before| day.0 < before)
                })
                .count(),
        )
    });
    let mut page = collect_feed_page(entries, limit, before_day, as_of_day, |id| {
        load_feed_read(store, id, || {
            let day = by_id[id];
            referenced.contains(&day).then_some(day)
        })
    })?;
    page.references_pending = graph.is_none();
    // The listing's own unreadable entries are reported with the first page
    // only, so a paged feed names each one once per load.
    if before_day.is_none() {
        page.unreadable.splice(0..0, listing_unreadable);
        // Upcoming shows references, not journal bodies: proposed identities
        // suffice and avoid reading every future journal file on feed open.
        page.upcoming = referenced
            .into_iter()
            .filter(|day| day.0 > as_of_day)
            .filter(|day| active_journal_id(store, *day).is_some())
            .map(|day| virtual_day(store, day))
            .collect();
    }
    Ok(page)
}

/// Merge bounded file windows with the existing reference answer. No resident
/// state: both answers belong to this captured view. Missing/unreadable bodies
/// consume candidates, so the iterator continues until the page really fills.
fn indexed_feed_page(
    store: &Store,
    graph: &tine_store::WholeGraph,
    limit: usize,
    before: i64,
    through: Option<i64>,
    as_of: i64,
) -> io::Result<FeedPage<FeedRead>> {
    let referenced: std::collections::BTreeSet<_> = graph
        .inventory(tine_store::InventoryScope::ReferencedJournals)
        .0
        .iter()
        .filter_map(|entry| entry.day)
        .collect();
    let references: BTreeMap<_, _> = referenced
        .iter()
        .copied()
        .filter(|day| day.0 <= as_of)
        .filter_map(|day| active_journal_id(store, day).map(|id| (day, id)))
        .collect();
    let by_id: BTreeMap<_, _> = references
        .iter()
        .map(|(day, id)| (id.clone(), *day))
        .collect();
    let mut references = references
        .into_iter()
        .rev()
        .filter(|(day, _)| day.0 < before)
        .peekable();
    let mut cursor = before.min(as_of.saturating_add(1));
    let batch = limit.saturating_add(1).max(1);
    let mut buffer = Vec::new().into_iter();
    let mut exhausted = false;
    let mut files = std::iter::from_fn(|| loop {
        if let Some(entry) = buffer.next() {
            return Some(entry);
        }
        if exhausted {
            return None;
        }
        let inventory = graph.inventory(tine_store::InventoryScope::JournalFilesBefore {
            before: Day(cursor),
            limit: batch,
        });
        exhausted = inventory.0.len() < batch;
        let entries: Vec<_> = inventory
            .0
            .iter()
            .filter_map(|entry| {
                let tine_store::Resolved::Existing { id, .. } = &entry.target else {
                    return None;
                };
                Some((entry.day?, id.clone()))
            })
            .collect();
        if let Some((day, _)) = entries.last() {
            cursor = day.0;
        }
        buffer = entries.into_iter();
    })
    .peekable();
    let mut entries = std::iter::from_fn(|| match (files.peek(), references.peek()) {
        (Some((file_day, _)), Some((ref_day, _))) if file_day >= ref_day => {
            if file_day == ref_day {
                references.next();
            }
            files.next()
        }
        (_, Some(_)) => references.next(),
        (Some(_), None) => files.next(),
        (None, None) => None,
    })
    .peekable();
    // Preserve extension semantics: the old path raised the requested count
    // by the number of candidates through the displayed suffix, even when
    // some of those candidates subsequently fail to load.
    let mut prefix = Vec::new();
    if let Some(through) = through {
        while entries.peek().is_some_and(|(day, _)| day.0 >= through) {
            prefix.push(entries.next().unwrap());
        }
    }
    let limit = limit.max(prefix.len());
    collect_feed_iter(
        prefix.into_iter().chain(entries),
        limit,
        None,
        as_of,
        |id| {
            load_feed_read(store, id, || {
                if referenced.is_empty() {
                    return None;
                }
                // Preserve the full union's id-to-greatest-day answer even when
                // a short-year format maps different centuries to the same path.
                // Only a missing body needs this lookup. It reads the file list,
                // not parsed pages: a file that failed to read keeps its day.
                let physical_day = graph
                    .inventory(tine_store::InventoryScope::JournalFile(id))
                    .0
                    .first()
                    .and_then(|entry| entry.day)
                    .filter(|day| day.0 <= as_of);
                let day = physical_day.max(by_id.get(id).copied())?;
                referenced.contains(&day).then_some(day)
            })
        },
    )
}

#[cfg(test)]
#[path = "journals_ready_tests.rs"]
mod journals_ready_tests;

#[cfg(test)]
mod journal_feed_tests {
    use super::*;
    use tine_core::model::PageDto;

    fn entry(day: i64) -> (Day, PageId) {
        (Day(day), PageId::from(day.to_string()))
    }
    fn dto(id: &PageId) -> PageDto {
        serde_json::from_value(serde_json::json!({
            "name": id.as_str(), "kind": "journal", "title": id.as_str(),
            "pre_block": null, "blocks": []
        }))
        .unwrap()
    }

    #[test]
    fn deletion_stable_day_cursor_fills_then_continues_without_duplicates() {
        let entries = [5, 4, 3, 2, 1].into_iter().map(entry).collect();
        let first = collect_feed_page(entries, 3, None, 5, |id| {
            Ok(if id.as_str() == "5" {
                Loaded::Gone
            } else {
                Loaded::Page(dto(id))
            })
        })
        .unwrap();
        assert_eq!(
            first
                .pages
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["4", "3", "2"]
        );
        assert_eq!(first.next_before_day, Some(2));
        assert!(!first.done);
        let entries = [5, 4, 3, 2, 1].into_iter().map(entry).collect();
        let second = collect_feed_page(entries, 3, first.next_before_day, 5, |id| {
            Ok(Loaded::Page(dto(id)))
        })
        .unwrap();
        assert_eq!(
            second
                .pages
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["1"]
        );
        assert!(second.done);
        assert_eq!(second.next_before_day, None);
    }

    #[test]
    fn cursor_handles_second_page_loss_empty_suffix_exact_limit_zero_and_hard_errors() {
        let first = collect_feed_page(
            [5, 4, 3, 2, 1].into_iter().map(entry).collect(),
            3,
            None,
            5,
            |id| Ok(Loaded::Page(dto(id))),
        )
        .unwrap();
        assert_eq!(first.next_before_day, Some(3));
        let second = collect_feed_page(
            [5, 4, 3, 2, 1].into_iter().map(entry).collect(),
            3,
            first.next_before_day,
            5,
            |id| {
                Ok(if id.as_str() == "2" {
                    Loaded::Gone
                } else {
                    Loaded::Page(dto(id))
                })
            },
        )
        .unwrap();
        assert_eq!(
            second
                .pages
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["1"]
        );
        assert!(
            second.done,
            "a missing second-page row still exhausts the suffix"
        );
        let empty = collect_feed_page(
            [5, 4].into_iter().map(entry).collect(),
            3,
            Some(4),
            5,
            |id| Ok(Loaded::Page(dto(id))),
        )
        .unwrap();
        assert!(empty.pages.is_empty());
        assert!(empty.done);
        let exact = collect_feed_page(
            [3, 2, 1].into_iter().map(entry).collect(),
            3,
            None,
            3,
            |id| Ok(Loaded::Page(dto(id))),
        )
        .unwrap();
        assert!(exact.done, "an exactly-full final page is done");
        assert_eq!(exact.next_before_day, None);
        let mut loads = 0;
        let zero = collect_feed_page(
            [3, 2, 1].into_iter().map(entry).collect(),
            0,
            None,
            3,
            |_id| {
                loads += 1;
                Ok(Loaded::Page(dto(&PageId::from("0"))))
            },
        )
        .unwrap();
        assert_eq!(loads, 0, "zero limit loads no entries");
        assert!(!zero.done);
        let hard: Result<FeedPage<PageDto>, _> =
            collect_feed_page([3].into_iter().map(entry).collect(), 1, None, 3, |_id| {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            });
        assert!(matches!(hard, Err(err) if err.kind() == io::ErrorKind::PermissionDenied));
    }

    /// One unreadable journal is skipped and named; the page still fills to
    /// its limit from the readable days, and the cursor moves past it.
    #[test]
    fn an_unreadable_day_is_reported_and_the_page_still_fills() {
        let page = collect_feed_page(
            [4, 3, 2, 1].into_iter().map(entry).collect(),
            2,
            None,
            4,
            |id| {
                Ok(if id.as_str() == "3" {
                    Loaded::Unreadable("undecodable".into())
                } else {
                    Loaded::Page(dto(id))
                })
            },
        )
        .unwrap();
        assert_eq!(
            page.pages
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["4", "2"]
        );
        assert_eq!(page.unreadable, ["3: undecodable"]);
        assert_eq!(page.next_before_day, Some(2));
        assert!(!page.done);
    }
}

/// The file a journal day no file claims would be created as, answered
/// without the graph-wide parse (BqbeQ: saving an empty today's daily
/// template opened the page through `resolve_page`, which waited for it).
/// A journal is claimed by its files alone and no alias resolves a journal, so
/// such a day resolves to `Resolved::Absent` at its configured id, the id the
/// feed gives the same virtual day (`Store::journal_id`). `None` when `name` is
/// not a day under the current title or file-name format, or a file claims it:
/// the graph answers then, with every claimant. Cost: one `page_named` lookup
/// (a file-name listing check before Ready; one file read if a file claims the
/// day) and O(path bytes). No write.
pub fn absent_journal_id(store: &Store, name: &str) -> Result<Option<PageId>, StoreError> {
    let fmt = format(store);
    let Some(date) = fmt.parse(name) else {
        return Ok(None);
    };
    if store
        .page_named(name, tine_core::model::PageKind::Journal)?
        .is_some()
    {
        return Ok(None);
    }
    Ok(Some(store.journal_id(Day(date.ordinal_key()), false)))
}

fn format(store: &Store) -> JournalFormat {
    let config = store.config();
    config.journal_format()
}

/// The journals listing: every listed file, plus `name: reason` for each entry
/// the scan could not read. A failed scan of the whole area is an error.
fn listing(store: &Store) -> io::Result<(Vec<FileEntry>, Vec<String>)> {
    #[cfg(test)]
    JOURNAL_LISTINGS.with(|count| count.set(count.get() + 1));
    crate::conflicts::area_listing(store, Area::Journals)
}

#[cfg(test)]
thread_local! {
    /// Journals directory listings on this thread (cost audit row 8).
    static JOURNAL_LISTINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn journal_listings() -> usize {
    JOURNAL_LISTINGS.with(std::cell::Cell::get)
}

/// The complete journals listing, for the filename migration, which renames
/// beside the entries it sees: an unlisted entry (a disk error, or a sync
/// service delivering a non-UTF-8 name) could be a same-day twin that the
/// rename would turn into a duplicate journal day, so a partial listing
/// refuses. Read-only listings use [`listing`] and report instead.
fn complete_files(store: &Store) -> io::Result<Vec<FileEntry>> {
    let (files, unreadable) = listing(store)?;
    if let Some(first) = unreadable.first() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("journal inventory is partial ({first})"),
        ));
    }
    Ok(files)
}

fn stem(entry: &FileEntry) -> Option<&str> {
    if !tine_store::is_graph_text(&entry.id) {
        return None;
    }
    entry
        .rel
        .rsplit('/')
        .next()?
        .rsplit_once('.')
        .map(|(stem, _)| stem)
}

/// Dated journal ids newest first, once per day, through `cutoff`. Future days
/// stay addressable as pages. Listing entries that cannot be read are skipped.
/// Cost O(J log J), plus initial ordinary preamble discovery before a name
/// index exists (see `Store::journal_id`).
#[cfg(any(test, feature = "test-faults"))]
pub fn feed_journals_desc_through(store: &Store, cutoff: Day) -> io::Result<Vec<(Day, PageId)>> {
    Ok(feed_days_through(store, cutoff)?.0)
}

/// Frozen unbounded day-inventory oracle for the bounded-window tests.
#[cfg(test)]
fn indexed_days_through(graph: &tine_store::WholeGraph, cutoff: Day) -> Vec<(Day, PageId)> {
    let days: BTreeMap<Day, PageId> = graph
        .inventory(tine_store::InventoryScope::JournalFiles)
        .0
        .iter()
        .filter_map(|entry| {
            let day = entry.day.filter(|day| *day <= cutoff)?;
            let tine_store::Resolved::Existing { id, .. } = &entry.target else {
                return None;
            };
            entry.is_journal.then(|| (day, id.clone()))
        })
        .collect();
    days.into_iter().rev().collect()
}

/// `feed_journals_desc_through` plus the listing's unreadable entries.
fn feed_days_through(store: &Store, cutoff: Day) -> io::Result<(Vec<(Day, PageId)>, Vec<String>)> {
    let (files, unreadable) = listing(store)?;
    let mut days = BTreeMap::new();
    for entry in files {
        if entry.page.is_some() {
            if let Some(day) = entry.day.filter(|day| *day <= cutoff) {
                days.entry(day).or_insert(());
            }
        }
    }
    let days = days
        .into_keys()
        .rev()
        .filter_map(|day| active_journal_id(store, day).map(|id| (day, id)))
        .collect();
    Ok((days, unreadable))
}

/// A physical journal (or its proposed path) belongs in the feed only when
/// it is also the opening file. An ordinary owner masks the entire journal
/// day, including reference-only insertion; it is not an empty virtual day.
fn active_journal_id(store: &Store, day: Day) -> Option<PageId> {
    let active = store.journal_id(day, true);
    (active == store.journal_id(day, false)).then_some(active)
}

fn migration_target(entry: &FileEntry, fmt: &JournalFormat) -> Option<String> {
    let source_stem = stem(entry)?;
    if fmt.is_canonical_stem(source_stem) {
        return None;
    }
    let date = fmt.parse(source_stem)?;
    let wanted = fmt.file_stem(date);
    let ext = entry.rel.rsplit_once('.')?.1;
    let target = format!("{wanted}.{ext}");
    Some(target)
}

/// One title-named journal that could not be renamed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationSkip {
    /// Existing journal filename.
    pub file: String,
    /// Human-readable refusal or read failure.
    pub reason: String,
}

/// Result of a best-effort journal filename migration.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct MigrationResult {
    /// Number of files renamed.
    pub migrated: usize,
    /// Every eligible title-named file left in place, with its reason.
    pub skipped: Vec<MigrationSkip>,
}

/// A title-named journal file and the date name it would get. Both are file
/// names inside the journals directory, extension included (never a `/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalFilenameMigration {
    pub from: String,
    pub to: String,
}

/// The journals listing indexed for [`migration_plan`]. Cost O(J) to build.
struct Listing {
    entries: Vec<FileEntry>,
    rels: HashSet<String>,
    days: BTreeMap<Day, usize>,
}

impl Listing {
    fn new(store: &Store) -> io::Result<Self> {
        let entries = complete_files(store)?;
        let rels = entries.iter().map(|entry| entry.rel.clone()).collect();
        let mut days = BTreeMap::new();
        for day in entries.iter().filter_map(|entry| entry.day) {
            *days.entry(day).or_insert(0) += 1;
        }
        Ok(Self {
            entries,
            rels,
            days,
        })
    }
}

/// The one answer to "may this journal file be renamed to its date name now?"
/// for [`journal_filename_migrations`] and [`migrate_journal_filenames`]: a
/// top-level journal whose stem parses as a `:journal/page-title-format` title
/// gets `to` from `:journal/file-name-format`, unless that file exists,
/// another journal file (any extension, including a second title) has the
/// same day, or `to` is not a valid journal file name. `Err` is the reason;
/// `None` means the file is not a title-named journal. Cost O(log J).
fn migration_plan(
    entry: &FileEntry,
    listing: &Listing,
    fmt: &JournalFormat,
    store: &Store,
) -> Option<Result<String, String>> {
    if entry.rel.contains('/') {
        return None;
    }
    let target = migration_target(entry, fmt)?;
    let same_day = entry
        .day
        .is_some_and(|day| listing.days.get(&day).copied().unwrap_or(0) > 1);
    Some(if listing.rels.contains(&target) {
        Err(format!("target {target} already exists"))
    } else if same_day {
        Err("another same-day journal file exists (see duplicate journal days)".to_owned())
    } else if store.file_id(Area::Journals, &target).is_err() {
        Err("target filename is invalid".to_owned())
    } else {
        Ok(target)
    })
}

/// Exactly the renames [`migrate_journal_filenames`] would perform now, given
/// this list back (see `migration_plan`). Read-only; the Settings panel lists
/// them and graph open never calls this (master e6f9b6e1ceae). Sorted by
/// `from`; an incomplete listing returns an error. Cost
/// O(J log J) over one directory listing; no file contents are read.
pub fn journal_filename_migrations(store: &Store) -> io::Result<Vec<JournalFilenameMigration>> {
    let listing = Listing::new(store)?;
    let fmt = format(store);
    Ok(listing
        .entries
        .iter()
        .filter_map(|entry| {
            let to = migration_plan(entry, &listing, &fmt, store)?.ok()?;
            Some(JournalFilenameMigration {
                from: entry.rel.clone(),
                to,
            })
        })
        .collect())
}

/// Best-effort one-file transactions over exactly the `confirmed` proposals
/// the user saw (from [`journal_filename_migrations`]). Each proposal renames
/// only when `migration_plan` still gives the same `to` for its `from`;
/// otherwise it is reported as skipped with the reason. A file not in
/// `confirmed` is never renamed. References are not rewritten (the name stays
/// the journal's title). Caller takes the pre-migration backup. With a
/// host, each still-eligible proposal is its own host operation (DESIGN
/// §3.1, A-R3) of its source and target: their unsaved input is saved
/// first, and a proposal whose page cannot be saved is skipped with that
/// reason, never blocking the others. Cost O(J log J + confirmed × J +
/// migrated file bytes).
pub fn migrate_journal_filenames(
    store: &Store,
    host: &PageHost,
    confirmed: &[JournalFilenameMigration],
) -> io::Result<MigrationResult> {
    let fmt = format(store);
    let mut listing = Listing::new(store)?;
    let mut result = MigrationResult::default();
    for proposal in confirmed {
        let skip = |reason: String| MigrationSkip {
            file: proposal.from.clone(),
            reason,
        };
        let plan = listing
            .entries
            .iter()
            .find(|entry| entry.rel == proposal.from)
            .map(|entry| {
                (
                    entry.id.clone(),
                    migration_plan(entry, &listing, &fmt, store),
                )
            });
        let entry = match plan {
            Some((id, Some(Ok(to)))) if to == proposal.to => id,
            Some((_, Some(Err(reason)))) => {
                result.skipped.push(skip(reason));
                continue;
            }
            _ => {
                result
                    .skipped
                    .push(skip("changed since it was listed".to_owned()));
                continue;
            }
        };
        let target = proposal.to.clone();
        let Ok(to) = store.file_id(Area::Journals, &target) else {
            continue; // `migration_plan` already refused an invalid name.
        };
        let moved = match (store.as_page(&entry), store.as_page(&to)) {
            (Some(from), Some(page)) => {
                migrate_hosted(store, host, (&entry, from), (&to, page), &target)
            }
            // Not a page (Store::as_page): the plain move, no host fence.
            _ => migrate_one(store, &entry, &to),
        };
        match moved {
            Ok(()) => {
                // The day count is unchanged: the moved file keeps its day.
                listing.rels.remove(&proposal.from);
                listing.rels.insert(target);
                result.migrated += 1;
            }
            Err(reason) => result.skipped.push(skip(reason.to_string())),
        }
    }
    Ok(result)
}

/// Move one eligible journal file to `to` as its own transaction; the
/// error is the reason the proposal is skipped.
fn migrate_one(store: &Store, entry: &FileId, to: &FileId) -> io::Result<()> {
    let (_, rev) = migration_source(store, entry)?;
    let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
    tx.move_file(entry, rev, to, None);
    tx_error(tx.commit()).map(drop).map_err(migration_error)
}

fn migration_error(error: io::Error) -> io::Error {
    // A read-only graph (config.edn unreadable) keeps its wire family: the
    // window words it, so a prefix here would hide it.
    if error
        .get_ref()
        .is_some_and(|cause| cause.is::<crate::ConfigUnreadable>())
    {
        error
    } else if error.kind() == io::ErrorKind::AlreadyExists {
        io::Error::other("same-day .md/.org twin would be created")
    } else {
        io::Error::other(format!("move refused: {error}"))
    }
}

fn migration_plan_error(why: Why) -> io::Error {
    let busy = matches!(&why, Why::Failed(error) if error.kind == io::ErrorKind::WouldBlock);
    let error = crate::why_error(why);
    if busy {
        error
    } else {
        migration_error(error)
    }
}

#[cfg(test)]
#[test]
fn planning_busy_migration_keeps_retryable_kind_and_message() {
    let message = "Tine is still finishing another change to these pages. Try again in a moment.";
    let error = migration_plan_error(Why::Failed(tine_store::IoError {
        kind: io::ErrorKind::WouldBlock,
        message: message.into(),
    }));
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(error.to_string(), message);
    // A revision conflict keeps the migration's existing classification.
    let conflict = migration_plan_error(Why::Conflict {
        file: FileId::from("journals/old.md".to_owned()),
        disk: None,
    });
    assert_eq!(conflict.kind(), io::ErrorKind::Other);
    assert_eq!(conflict.to_string(), "move refused: concurrent graph write");
    // Card HVgII: a graph read-only for an unreadable config.edn keeps its wire
    // family, so the window can word the skip reason from its catalog.
    let read_only = migration_plan_error(Why::Refused(tine_store::Refusal::ConfigUnreadable(
        tine_store::IoError {
            kind: io::ErrorKind::InvalidData,
            message: "invalid utf-8 sequence of 1 bytes from index 35".into(),
        },
    )));
    assert_eq!(
        read_only.to_string(),
        "config-unreadable: invalid utf-8 sequence of 1 bytes from index 35"
    );
}

/// One eligible journal file's move to `to` as one host operation (DESIGN
/// §3.1): its bytes unchanged, `to` written over no file, then the source
/// deleted from the bytes the move was planned from. A target that appeared
/// since the listing refuses, never overwritten; the error is the reason
/// the proposal is skipped.
fn migrate_hosted(
    store: &Store,
    host: &PageHost,
    (entry, from): (&FileId, PageId),
    (to, page): (&FileId, PageId),
    _target: &str,
) -> io::Result<()> {
    let plan = || {
        let (_, rev) = migration_source(store, entry)?;
        let mut tx = store.page_plan(Some(tine_store::EditKind::RenamePage));
        tx.move_file(entry, rev, to, None);
        tx.plan().map(Some).map_err(migration_plan_error)
    };
    crate::retained::operation(
        host,
        Input::Flush,
        &[from.clone(), page.clone()],
        "renaming the journal file",
        "journal changed repeatedly during migration",
        crate::retained::unsaved,
        plan,
    )
}

/// A migrating journal's bytes and revision, or the reason it is skipped.
fn migration_source(store: &Store, entry: &FileId) -> io::Result<(Vec<u8>, tine_store::FileRev)> {
    store
        .read(entry, Some(tine_store::PARSE_INPUT_MAX_BYTES))
        .map_err(|error| io::Error::other(format!("source could not be read: {error:?}")))
}

/// Duplicate-day files with first-line previews, the keeper first. The keeper
/// (`canonical`, one per day) is the file the day opens: the claimant order's
/// owner that `Store::page_named` reads (CM step 2 OW-2, PQ-1 2026-11-02, I-12),
/// so a merge keeps the page the user sees. Remaining paths use the shared
/// claimant order. When its owner could not be read, the first of that order
/// is the keeper. If an ordinary page owns the name, no journal is canonical;
/// the complete bucket appears in the name-conflict overview. A file
/// whose preview cannot be read stays listed with `preview_error` (one bad file
/// never hides the day, I-22); listing entries that cannot be read at all are
/// named by the conflict inventory and the journal feed. A failed area scan or
/// a closed store is an error. Cost O(J log J + bytes of duplicate files), plus
/// one page read per duplicate day.
pub fn journal_conflicts(store: &Store) -> io::Result<Vec<JournalConflict>> {
    let mut groups: BTreeMap<Day, Vec<FileEntry>> = BTreeMap::new();
    for entry in listing(store)?.0 {
        if let Some(day) = entry.day.filter(|_| stem(&entry).is_some()) {
            groups.entry(day).or_default().push(entry);
        }
    }
    let fmt = format(store);
    let mut conflicts = Vec::new();
    for (day, entries) in groups {
        if entries.len() < 2 {
            continue;
        }
        let mut journal_files = Vec::new();
        for entry in entries {
            let (preview, preview_error) = match crate::conflicts::preview(store, &entry.id) {
                Ok(preview) => (preview, None),
                Err(error) if crate::conflicts::store_failed(&error) => return Err(error),
                Err(error) => (String::new(), Some(error.to_string())),
            };
            journal_files.push(JournalFile {
                name: entry
                    .rel
                    .rsplit('/')
                    .next()
                    .unwrap_or(&entry.rel)
                    .to_owned(),
                path: entry.id.as_str().to_owned(),
                preview,
                canonical: false,
                preview_error,
            });
        }
        let title = fmt.title(JournalDate::from_ordinal(day.0));
        // The file the day opens. A read error here only loses the ranking
        // (recovery, not refusal): the listing order below still names a keeper.
        let opened = store
            .page_named(&title, tine_core::model::PageKind::Journal)
            .ok()
            .flatten()
            .map(|read| read.id.as_str().to_owned());
        journal_files.sort_by(|a, b| {
            let opens = |file: &JournalFile| opened.as_deref() == Some(file.path.as_str());
            let claimant = |file: &JournalFile| tine_core::model::PageEntry {
                name: title.clone(),
                kind: tine_core::model::PageKind::Journal,
                date_key: Some(day.0),
                rel_path: Some(tine_store::PageId::from(file.path.as_str())),
                path: Default::default(),
            };
            opens(b)
                .cmp(&opens(a))
                .then_with(|| tine_core::model::compare_page_claimants(&claimant(a), &claimant(b)))
        });
        for (i, file) in journal_files.iter_mut().enumerate() {
            file.canonical = opened.as_deref().map_or(i == 0, |path| path == file.path);
        }
        conflicts.push(JournalConflict {
            title,
            files: journal_files,
        });
    }
    Ok(conflicts)
}

fn journal_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad journal file name",
        ))
    } else {
        Ok(())
    }
}

/// Read exactly one top-level journal filename, capped by the shared parse
/// input byte limit. Cost O(file bytes).
pub fn read_journal_file(store: &Store, name: &str) -> io::Result<String> {
    journal_name(name)?;
    let id = store.file_id(Area::Journals, name).map_err(store_error)?;
    let (bytes, _) = store
        .read(&id, Some(tine_store::PARSE_INPUT_MAX_BYTES))
        .map_err(store_error)?;
    String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })
}

/// Trash one top-level journal with a revision guard, retrying an external
/// conflict four times. Cost O(file bytes) per attempt.
/// One host operation deletes the page (DESIGN §3.1); the host
/// keeps the deleted bytes in graph trash. The page's unsaved input is saved
/// first (`Input::Flush`).
pub fn trash_journal_file(store: &Store, host: &PageHost, name: &str) -> io::Result<()> {
    journal_name(name)?;
    let id = store.file_id(Area::Journals, name).map_err(store_error)?;
    let max = Some(tine_store::PARSE_INPUT_MAX_BYTES);
    let Some(page) = store.as_page(&id) else {
        // Not a page (Store::as_page): the guarded trash, no host fence.
        return crate::retry_on_conflict("journal changed repeatedly during trash", || {
            crate::trash_current(store, &id, max, "no such journal file")
        });
    };
    let plan = || {
        let rev = match store.read(&id, max) {
            Ok((_, rev)) => rev,
            Err(StoreError::NotFound(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no such journal file",
                ))
            }
            Err(error) => return Err(store_error(error)),
        };
        let mut tx = store.page_plan(Some(tine_store::EditKind::DeletePage));
        tx.trash(&id, rev, tine_store::TrashIf::Any);
        crate::retained::plan(tx).map(Some)
    };
    crate::retained::operation(
        host,
        Input::Flush,
        std::slice::from_ref(&page),
        "moving the journal to trash at logseq/.tine-trash/pages",
        "journal changed repeatedly during trash",
        crate::retained::unsaved,
        plan,
    )
}
