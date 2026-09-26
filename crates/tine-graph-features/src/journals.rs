//! Journal feed, duplicate-day reconciliation, and filename migration. All
//! reads use the store's area inventory; writes are guarded transactions.

use std::collections::{BTreeMap, HashSet};
use std::io;

use tine_core::date::{JournalDate, JournalFormat};
use tine_core::model::{JournalConflict, JournalFile};
use tine_store::{Area, Day, FileEntry, PageId, Store, StoreError};

use crate::{is_conflict, store_error, tx_error};

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
            StoreError::Io(error) => error,
            StoreError::InvalidTarget(_) => io::Error::other("invalid page path"),
            StoreError::Undecodable => io::Error::other("stream did not contain valid UTF-8"),
            StoreError::Unparseable(reason) => io::Error::other(reason),
            StoreError::TooLarge { limit, .. } => {
                io::Error::other(format!("asset exceeds {limit} byte limit"))
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

fn stem(name: &str) -> Option<&str> {
    name.rsplit('/')
        .next()?
        .rsplit_once('.')
        .and_then(|(stem, ext)| matches!(ext, "md" | "org").then_some(stem))
}

fn preview(store: &Store, entry: &FileEntry) -> String {
    store
        .read(&entry.id, None)
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

fn migration_target(
    entry: &FileEntry,
    existing: &HashSet<String>,
    fmt: &JournalFormat,
) -> Option<String> {
    let source_stem = stem(&entry.rel)?;
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
    (!existing.contains(&target)).then_some(target)
}

/// Whether a top-level title-named journal has an available canonical name.
/// Unreadable scan entries are skipped. Cost O(J).
pub fn has_journal_filename_migrations(store: &Store) -> bool {
    let entries = files(store);
    let existing: HashSet<_> = entries.iter().map(|entry| entry.rel.clone()).collect();
    let fmt = format(store);
    entries
        .iter()
        .filter(|entry| !entry.rel.contains('/'))
        .any(|entry| migration_target(entry, &existing, &fmt).is_some())
}

/// Best-effort one-file transactions, skipping occupied targets as v0.6.5.
/// Caller takes the pre-migration backup. Cost O(J + migrated file bytes).
pub fn migrate_journal_filenames(store: &Store) -> usize {
    let entries = files(store);
    let mut existing: HashSet<_> = entries.iter().map(|entry| entry.rel.clone()).collect();
    let fmt = format(store);
    let mut count = 0;
    for entry in entries.into_iter().filter(|entry| !entry.rel.contains('/')) {
        let Some(target) = migration_target(&entry, &existing, &fmt) else {
            continue;
        };
        let Ok((_, rev)) = store.read(&entry.id, None) else {
            continue;
        };
        let Ok(to) = store.file_id(Area::Journals, &target) else {
            continue;
        };
        let mut tx = store.transaction();
        tx.move_file(&entry.id, rev, &to, None);
        if tx_error(tx.commit()).is_ok() {
            existing.insert(target);
            count += 1;
        }
    }
    count
}

/// Duplicate-day files with first-line previews, canonical first. Unreadable
/// scan entries and unreadable previews are skipped/empty as v0.6.5. Cost
/// O(J log J + bytes of duplicate files).
pub fn journal_conflicts(store: &Store) -> Vec<JournalConflict> {
    let mut groups: BTreeMap<Day, Vec<FileEntry>> = BTreeMap::new();
    for entry in files(store) {
        if let Some(day) = entry.day.filter(|_| stem(&entry.rel).is_some()) {
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
                        canonical: stem(&entry.rel)
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

/// Read exactly one top-level journal filename without a size cap, matching
/// v0.6.5. Cost O(file bytes).
pub fn read_journal_file(store: &Store, name: &str) -> io::Result<String> {
    journal_name(name)?;
    let id = store.file_id(Area::Journals, name).map_err(store_error)?;
    let (bytes, _) = store.read(&id, None).map_err(store_error)?;
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
    for _ in 0..4 {
        let rev = match store.read(&id, None) {
            Ok((_, rev)) => rev,
            Err(StoreError::NotFound) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no such journal file",
                ))
            }
            Err(error) => return Err(store_error(error)),
        };
        let mut tx = store.transaction();
        tx.trash(&id, rev);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "journal changed repeatedly during trash",
    ))
}
