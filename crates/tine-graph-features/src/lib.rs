//! PDF, asset, journal, page, and conflict graph features. Each operation uses the public `tine-store`
//! boundary; callers supply device source streams, while the asset client
//! validates a selected import name without opening its path. Store-backed
//! writes use guarded transactions or the Store's graph creation/publish paths.
//! Callers need no graph path, lock, cache state, or write protocol.

pub mod assets;
pub mod config;
pub mod conflicts;
pub mod guide;
pub mod journals;
pub mod pages;
mod parsed_text;
pub mod pdf;
pub mod print;
pub mod publish;
mod render;
pub mod search;
pub mod sources;

use std::io;
use tine_store::{FileId, Refusal, Store, StoreError, TxOutcome, Why};

fn store_error(error: StoreError) -> io::Error {
    match error {
        StoreError::NotFound => io::Error::from(io::ErrorKind::NotFound),
        StoreError::InvalidTarget(message) => io::Error::new(io::ErrorKind::InvalidInput, message),
        StoreError::PageSource(message) => io::Error::new(io::ErrorKind::InvalidInput, message),
        StoreError::StreamSymlink(file) => io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("symlink:{}", file.as_str()),
        ),
        StoreError::Undecodable => io::Error::new(io::ErrorKind::InvalidData, "undecodable file"),
        StoreError::Unparseable(message) => io::Error::new(io::ErrorKind::InvalidData, message),
        StoreError::TooLarge { .. } => io::Error::new(io::ErrorKind::InvalidData, "file too large"),
        StoreError::Io(error) => error.into(),
        StoreError::Closed => io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
    }
}

fn tx_error(outcome: TxOutcome) -> io::Result<Vec<tine_store::StepResult>> {
    match outcome {
        TxOutcome::Committed { steps, .. } => Ok(steps),
        TxOutcome::PublicationIncomplete { files, .. } => Err(io::Error::other(format!(
            "publication-incomplete: disk write applied but final state could not be published for {}; inspect disk before retrying",
            <[&str]>::join(&files.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ", ")
        ))),
        TxOutcome::NotCommitted { why, rollback, publication_errors, .. } => {
            if !publication_errors.is_empty() {
                return Err(io::Error::other(format!(
                    "publication-incomplete: transaction refused ({why:?}); final state could not be published for {}; inspect disk before retrying",
                    <[&str]>::join(&publication_errors.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ", ")
                )));
            }
            if !rollback.undo_failed.is_empty() {
                let failed: Vec<String> = rollback
                    .undo_failed
                    .iter()
                    .map(|(file, error)| {
                        format!("{} ({:?}: {})", file.as_str(), error.kind, error.message)
                    })
                    .collect();
                let failed = <[String]>::join(&failed, ", ");
                let recovery: Vec<&str> = rollback
                    .kept_external
                    .iter()
                    .filter_map(|(_, location)| location.as_ref())
                    .map(|file| file.as_str())
                    .collect();
                let recovery = <[&str]>::join(&recovery, ", ");
                return Err(io::Error::other(format!(
                    "rollback-incomplete: undo failed for {failed}; recovery: {recovery}; original: {why:?}"
                )));
            }
            Err(match why {
                Why::Conflict { .. } => {
                    io::Error::new(io::ErrorKind::WouldBlock, "concurrent graph write")
                }
                Why::Failed(error) => io::Error::new(error.kind, error.message),
                Why::Refused(refusal) => match refusal {
                    Refusal::ReadOnly(message) | Refusal::InvalidTarget(message) => {
                        io::Error::new(io::ErrorKind::InvalidInput, message)
                    }
                    Refusal::Twin { .. } => {
                        io::Error::new(io::ErrorKind::AlreadyExists, "twin page")
                    }
                    Refusal::Undecodable => {
                        io::Error::new(io::ErrorKind::InvalidData, "undecodable file")
                    }
                    Refusal::RepeatedFile(_) => {
                        io::Error::new(io::ErrorKind::InvalidInput, "repeated file")
                    }
                    Refusal::Closed => io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
                },
            })
        }
    }
}

fn is_conflict(outcome: &TxOutcome) -> bool {
    matches!(
        outcome,
        TxOutcome::NotCommitted {
            why: Why::Conflict { .. },
            ..
        }
    )
}

fn retry_on_conflict<T>(
    exhausted: &'static str,
    mut attempt: impl FnMut() -> io::Result<Option<T>>,
) -> io::Result<T> {
    for _ in 0..4 {
        if let Some(value) = attempt()? {
            return Ok(value);
        }
    }
    Err(io::Error::new(io::ErrorKind::WouldBlock, exhausted))
}

fn commit_retry(outcome: TxOutcome) -> io::Result<bool> {
    if is_conflict(&outcome) {
        Ok(false)
    } else {
        tx_error(outcome).map(|_| true)
    }
}

fn trash_current(
    store: &Store,
    id: &FileId,
    max_bytes: Option<u64>,
    missing: &'static str,
) -> io::Result<Option<()>> {
    let rev = match store.read(id, max_bytes) {
        Ok((_, rev)) => rev,
        Err(StoreError::NotFound) => return Err(io::Error::new(io::ErrorKind::NotFound, missing)),
        Err(error) => return Err(store_error(error)),
    };
    let mut tx = if store.as_page(id).is_some() {
        store.transaction(Some(tine_store::EditKind::DeletePage))
    } else {
        store.transaction(None)
    };
    tx.trash(id, rev);
    Ok(commit_retry(tx.commit())?.then_some(()))
}

#[cfg(test)]
mod error_tests;
