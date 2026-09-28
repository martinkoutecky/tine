//! Journal feed, duplicate-day reconciliation, and filename migration. All
//! reads use the store's area inventory; writes are guarded transactions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io;

use tine_core::date::{JournalDate, JournalFormat};
use tine_core::model::{JournalConflict, JournalFile};
use tine_store::{Area, Day, FileEntry, PageId, Store, StoreError};

use crate::{store_error, tx_error};

/// A day-based page of the journal feed. The cursor records the last examined
/// day, including a journal that disappeared between inventory and read.
pub struct FeedPage<T> {
    pub pages: Vec<T>,
    pub next_before_day: Option<i64>,
    pub done: bool,
    pub as_of_day: i64,
}

fn collect_feed_page<T, F>(
    entries: Vec<(Day, PageId)>,
    limit: usize,
    before_day: Option<i64>,
    as_of_day: i64,
    mut load: F,
) -> Result<FeedPage<T>, String>
where
    F: FnMut(&PageId) -> Result<T, io::Error>,
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
        });
    }
    let mut out = Vec::new();
    let mut last_examined = None;
    let mut candidates = entries
        .into_iter()
        .filter(|(day, _)| before_day.is_none_or(|before| day.0 < before))
        .peekable();
    while let Some((day, id)) = candidates.next() {
        last_examined = Some(day.0);
        match load(&id) {
            Ok(value) => out.push(value),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
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
    })
}

/// Page the dated journal feed by day, skipping a file deleted since inventory.
/// Cost O(J log J + bytes of returned pages).
pub fn feed_page(
    store: &Store,
    limit: usize,
    before_day: Option<i64>,
) -> Result<FeedPage<tine_store::PageRead>, String> {
    let as_of_day = JournalDate::today().ordinal_key();
    let entries = feed_journals_desc_through(store, Day(as_of_day));
    collect_feed_page(entries, limit, before_day, as_of_day, |id| {
        store.page(id).map_err(|error| match error {
            StoreError::NotFound => io::Error::from(io::ErrorKind::NotFound),
            StoreError::Io(error) => error.into(),
            StoreError::InvalidTarget(_)
            | StoreError::PageSource(_)
            | StoreError::StreamSymlink(_) => io::Error::other("invalid page path"),
            StoreError::Undecodable => io::Error::other("stream did not contain valid UTF-8"),
            StoreError::Unparseable(reason) => io::Error::other(reason),
            StoreError::TooLarge { limit, .. } => {
                io::Error::other(format!("journal page exceeds {limit} byte limit"))
            }
            StoreError::Closed => io::Error::other("store closed"),
        })
    })
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
            if id.as_str() == "5" {
                Err(io::Error::from(io::ErrorKind::NotFound))
            } else {
                Ok(dto(id))
            }
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
        let second =
            collect_feed_page(entries, 3, first.next_before_day, 5, |id| Ok(dto(id))).unwrap();
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
            |id| Ok(dto(id)),
        )
        .unwrap();
        assert_eq!(first.next_before_day, Some(3));
        let second = collect_feed_page(
            [5, 4, 3, 2, 1].into_iter().map(entry).collect(),
            3,
            first.next_before_day,
            5,
            |id| {
                if id.as_str() == "2" {
                    Err(io::Error::from(io::ErrorKind::NotFound))
                } else {
                    Ok(dto(id))
                }
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
            |id| Ok(dto(id)),
        )
        .unwrap();
        assert!(empty.pages.is_empty());
        assert!(empty.done);
        let exact = collect_feed_page(
            [3, 2, 1].into_iter().map(entry).collect(),
            3,
            None,
            3,
            |id| Ok(dto(id)),
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
                Ok(dto(&PageId::from("0")))
            },
        )
        .unwrap();
        assert_eq!(loads, 0, "zero limit loads no entries");
        assert!(!zero.done);
        let hard: Result<FeedPage<PageDto>, _> =
            collect_feed_page([3].into_iter().map(entry).collect(), 1, None, 3, |_id| {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            });
        assert!(matches!(hard, Err(err) if err.contains("denied")));
    }
}

fn format(store: &Store) -> JournalFormat {
    let config = store.config();
    JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    )
}

fn files(store: &Store) -> Vec<FileEntry> {
    store
        .scan_area(Area::Journals, None)
        .map(|listing| listing.files)
        .unwrap_or_default() // v0.6.5 skips unlistable journals.
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

fn preview(store: &Store, entry: &FileEntry) -> String {
    store
        .read(&entry.id, Some(tine_store::PARSE_INPUT_MAX_BYTES))
        .ok()
        .and_then(|(bytes, _)| String::from_utf8(bytes).ok())
        .and_then(|content| {
            content
                .lines()
                .map(|line| {
                    line.trim_start_matches(|ch| matches!(ch, '*' | '-' | ' ' | '\t'))
                        .trim()
                        .to_owned()
                })
                .find(|line| !line.is_empty())
        })
        .map(|line| line.chars().take(80).collect())
        .unwrap_or_default()
}

/// Dated journal ids newest first, once per day, through `cutoff`. Future days
/// stay addressable as pages. Unreadable scan entries are skipped. Cost O(J log J).
pub fn feed_journals_desc_through(store: &Store, cutoff: Day) -> Vec<(Day, PageId)> {
    let mut days = BTreeMap::new();
    for entry in files(store) {
        if entry.page.is_none() {
            continue;
        }
        if let Some(day) = entry.day.filter(|day| *day <= cutoff) {
            days.entry(day).or_insert(());
        }
    }
    days.into_keys()
        .rev()
        .map(|day| (day, store.journal_id(day)))
        .collect()
}

fn migration_target(entry: &FileEntry, fmt: &JournalFormat) -> Option<String> {
    let source_stem = stem(entry)?;
    if JournalDate::from_file_stem(source_stem).is_some() {
        return None;
    }
    let date = fmt.parse(source_stem)?;
    let wanted = fmt.file_stem(date);
    if wanted == source_stem {
        return None;
    }
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
    fn new(store: &Store) -> Self {
        let entries = files(store);
        let rels = entries.iter().map(|entry| entry.rel.clone()).collect();
        let mut days = BTreeMap::new();
        for day in entries.iter().filter_map(|entry| entry.day) {
            *days.entry(day).or_insert(0) += 1;
        }
        Self {
            entries,
            rels,
            days,
        }
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
/// `from`; an unlistable journals directory gives an empty list. Cost
/// O(J log J) over one directory listing; no file contents are read.
pub fn journal_filename_migrations(store: &Store) -> Vec<JournalFilenameMigration> {
    let listing = Listing::new(store);
    let fmt = format(store);
    listing
        .entries
        .iter()
        .filter_map(|entry| {
            let to = migration_plan(entry, &listing, &fmt, store)?.ok()?;
            Some(JournalFilenameMigration {
                from: entry.rel.clone(),
                to,
            })
        })
        .collect()
}

/// Best-effort one-file transactions over exactly the `confirmed` proposals
/// the user saw (from [`journal_filename_migrations`]). Each proposal renames
/// only when `migration_plan` still gives the same `to` for its `from`;
/// otherwise it is reported as skipped with the reason. A file not in
/// `confirmed` is never renamed. References are not rewritten (the name stays
/// the journal's title). Caller takes the pre-migration backup. Cost
/// O(J log J + confirmed × J + migrated file bytes).
pub fn migrate_journal_filenames(
    store: &Store,
    confirmed: &[JournalFilenameMigration],
) -> MigrationResult {
    let fmt = format(store);
    let mut listing = Listing::new(store);
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
        let rev = match store.read(&entry, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
            Ok((_, rev)) => rev,
            Err(error) => {
                result
                    .skipped
                    .push(skip(format!("source could not be read: {error:?}")));
                continue;
            }
        };
        let Ok(to) = store.file_id(Area::Journals, &target) else {
            continue; // `migration_plan` already refused an invalid name.
        };
        let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
        tx.move_file(&entry, rev, &to, None);
        match tx_error(tx.commit()) {
            Ok(_) => {
                // The day count is unchanged: the moved file keeps its day.
                listing.rels.remove(&proposal.from);
                listing.rels.insert(target);
                result.migrated += 1;
            }
            Err(error) => {
                let reason = if error.kind() == io::ErrorKind::AlreadyExists {
                    "same-day .md/.org twin would be created".to_owned()
                } else {
                    format!("move refused: {error}")
                };
                result.skipped.push(skip(reason));
            }
        }
    }
    result
}

/// Duplicate-day files with first-line previews, canonical first. Unreadable
/// scan entries and unreadable previews are skipped/empty as v0.6.5. Cost
/// O(J log J + bytes of duplicate files).
pub fn journal_conflicts(store: &Store) -> Vec<JournalConflict> {
    let mut groups: BTreeMap<Day, Vec<FileEntry>> = BTreeMap::new();
    for entry in files(store) {
        if let Some(day) = entry.day.filter(|_| stem(&entry).is_some()) {
            groups.entry(day).or_default().push(entry);
        }
    }
    let fmt = format(store);
    groups
        .into_iter()
        .filter_map(|(day, entries)| {
            if entries.len() < 2 {
                return None;
            }
            let mut journal_files: Vec<_> = entries
                .iter()
                .map(|entry| {
                    let name = entry
                        .rel
                        .rsplit('/')
                        .next()
                        .unwrap_or(&entry.rel)
                        .to_owned();
                    JournalFile {
                        name,
                        path: entry.id.as_str().to_owned(),
                        preview: preview(store, entry),
                        canonical: stem(&entry)
                            .is_some_and(|stem| JournalDate::from_file_stem(stem).is_some()),
                    }
                })
                .collect();
            journal_files.sort_by(|a, b| {
                b.canonical
                    .cmp(&a.canonical)
                    .then_with(|| a.name.cmp(&b.name))
            });
            Some(JournalConflict {
                title: fmt.title(JournalDate::from_ordinal(day.0)),
                files: journal_files,
            })
        })
        .collect()
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
pub fn trash_journal_file(store: &Store, name: &str) -> io::Result<()> {
    journal_name(name)?;
    let id = store.file_id(Area::Journals, name).map_err(store_error)?;
    crate::retry_on_conflict("journal changed repeatedly during trash", || {
        crate::trash_current(
            store,
            &id,
            Some(tine_store::PARSE_INPUT_MAX_BYTES),
            "no such journal file",
        )
    })
}
