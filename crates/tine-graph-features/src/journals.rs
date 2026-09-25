//! Journal feed, duplicate-day reconciliation, and filename migration. All
//! reads use the store's area inventory; writes are guarded transactions.

use std::collections::{BTreeMap, HashSet};
use std::io;

use tine_core::date::{JournalDate, JournalFormat};
use tine_core::model::{JournalConflict, JournalFile};
use tine_store::{Area, Day, FileEntry, PageId, Store, StoreError};

use crate::{is_conflict, store_error, tx_error};

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
