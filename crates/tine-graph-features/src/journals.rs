//! Journal feed, duplicate-day reconciliation, and filename migration. All
//! reads use the store's area inventory; writes are guarded transactions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io;

use tine_core::date::{JournalDate, JournalFormat};
use tine_core::model::{JournalConflict, JournalFile};
use tine_store::{Area, Day, FileEntry, FileId, Input, PageHost, PageId, Store, StoreError};

use crate::{store_error, tx_error};

/// A day-based page of the journal feed. The cursor records the last examined
/// day, including a journal that disappeared between inventory and read.
pub struct FeedPage<T> {
    pub pages: Vec<T>,
    pub next_before_day: Option<i64>,
    pub done: bool,
    pub as_of_day: i64,
    /// `path: reason` for every journal this request skipped because it could
    /// not be listed or read (one bad file never blocks the feed, I-22; the
    /// caller reports these, I-2). Empty in the common case.
    pub unreadable: Vec<String>,
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
    mut load: F,
) -> Result<FeedPage<T>, io::Error>
where
    F: FnMut(&PageId) -> Result<Loaded<T>, io::Error>,
{
    if limit == 0 {
        let done = !entries
            .iter()
            .any(|(day, _)| before_day.is_none_or(|before| day.0 < before));
        return Ok(FeedPage {
            pages: Vec::new(),
            next_before_day: None,
            done,
            as_of_day,
            unreadable: Vec::new(),
        });
    }
    let mut out = Vec::new();
    let mut unreadable = Vec::new();
    let mut last_examined = None;
    let mut candidates = entries
        .into_iter()
        .filter(|(day, _)| before_day.is_none_or(|before| day.0 < before))
        .peekable();
    while let Some((day, id)) = candidates.next() {
        last_examined = Some(day.0);
        match load(&id)? {
            Loaded::Page(value) => out.push(value),
            Loaded::Gone => {}
            Loaded::Unreadable(reason) => unreadable.push(format!("{}: {reason}", id.as_str())),
        }
        if out.len() == limit {
            break;
        }
    }
    let done = candidates.peek().is_none();
    Ok(FeedPage {
        pages: out,
        next_before_day: if done { None } else { last_examined },
        done,
        as_of_day,
        unreadable,
    })
}

/// Page the dated journal feed by day, skipping a file deleted since inventory.
/// A journal that cannot be listed or read (undecodable, oversized, a disk
/// error, a non-UTF-8 name) is skipped and named in `unreadable`; only a
/// failure of the whole store (closed, area scan) fails the request.
/// Cost O(J log J + bytes of returned pages).
pub fn feed_page(
    store: &Store,
    limit: usize,
    before_day: Option<i64>,
) -> Result<FeedPage<tine_store::PageRead>, io::Error> {
    let as_of_day = JournalDate::today().ordinal_key();
    let (entries, listing_unreadable) = feed_days_through(store, Day(as_of_day))?;
    let mut page = collect_feed_page(entries, limit, before_day, as_of_day, |id| {
        Ok(match store.page(id) {
            Ok(read) => Loaded::Page(read),
            Err(StoreError::NotFound) => Loaded::Gone,
            Err(StoreError::Closed) => return Err(store_error(StoreError::Closed)),
            Err(error) => Loaded::Unreadable(store_error(error).to_string()),
        })
    })?;
    // The listing's own unreadable entries are reported with the first page
    // only, so a paged feed names each one once per load.
    if before_day.is_none() {
        page.unreadable.splice(0..0, listing_unreadable);
    }
    Ok(page)
}

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

fn format(store: &Store) -> JournalFormat {
    let config = store.config();
    JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    )
}

/// The journals listing: every listed file, plus `name: reason` for each entry
/// the scan could not read. A failed scan of the whole area is an error.
fn listing(store: &Store) -> io::Result<(Vec<FileEntry>, Vec<String>)> {
    crate::conflicts::area_listing(store, Area::Journals)
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
/// Cost O(J log J).
pub fn feed_journals_desc_through(store: &Store, cutoff: Day) -> io::Result<Vec<(Day, PageId)>> {
    Ok(feed_days_through(store, cutoff)?.0)
}

/// [`feed_journals_desc_through`] plus the listing's unreadable entries.
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
        .map(|day| (day, store.journal_id(day)))
        .collect();
    Ok((days, unreadable))
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
/// the journal's title). Caller takes the pre-migration backup. Each
/// still-eligible proposal is its own retained writer (STEP3 §7, A-R3) of
/// its source and target: their unsaved input is saved first, and a
/// proposal whose page cannot be saved is skipped with that reason, never
/// blocking the others. Cost O(J log J + confirmed × J + migrated file
/// bytes).
pub fn migrate_journal_filenames(
    store: &Store,
    host: Option<&PageHost>,
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
        let pages = crate::retained::pages(store, [&entry, &to]);
        let moved = crate::retained::reserved(
            host,
            Input::Flush,
            || Ok(pages.clone()),
            crate::retained::unsaved,
            |_| Ok(migrate_one(store, &entry, &to)),
        );
        match moved.and_then(|moved| moved) {
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
    let rev = match store.read(entry, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
        Ok((_, rev)) => rev,
        Err(error) => {
            return Err(io::Error::other(format!(
                "source could not be read: {error:?}"
            )))
        }
    };
    let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
    tx.move_file(entry, rev, to, None);
    tx_error(tx.commit()).map(drop).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            io::Error::other("same-day .md/.org twin would be created")
        } else {
            io::Error::other(format!("move refused: {error}"))
        }
    })
}

/// Duplicate-day files with first-line previews, canonical first. A file whose
/// preview cannot be read stays listed with `preview_error` (one bad file never
/// hides the day, I-22); listing entries that cannot be read at all are named by
/// the conflict inventory and the journal feed. A failed area scan or a closed
/// store is an error. Cost O(J log J + bytes of duplicate files).
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
                canonical: stem(&entry).is_some_and(|stem| fmt.is_canonical_stem(stem)),
                preview_error,
            });
        }
        journal_files.sort_by(|a, b| {
            b.canonical
                .cmp(&a.canonical)
                .then_with(|| a.name.cmp(&b.name))
        });
        conflicts.push(JournalConflict {
            title: fmt.title(JournalDate::from_ordinal(day.0)),
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
/// A retained writer (STEP3 §7): the page's unsaved input is saved first.
pub fn trash_journal_file(store: &Store, host: Option<&PageHost>, name: &str) -> io::Result<()> {
    journal_name(name)?;
    let id = store.file_id(Area::Journals, name).map_err(store_error)?;
    let discover = || Ok(crate::retained::pages(store, [&id]));
    crate::retained::reserved(
        host,
        Input::Flush,
        discover,
        crate::retained::unsaved,
        |_| {
            crate::retry_on_conflict("journal changed repeatedly during trash", || {
                crate::trash_current(
                    store,
                    &id,
                    Some(tine_store::PARSE_INPUT_MAX_BYTES),
                    "no such journal file",
                )
            })
        },
    )
}
