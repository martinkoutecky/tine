//! PDF, asset, journal, page, and conflict graph features. Each operation uses the public `tine-store`
//! boundary; callers supply device source streams, while the asset client
//! validates a selected import name without opening its path. Store-backed
//! writes use guarded transactions or the Store's graph creation/publish paths.
//! Callers need no graph path, lock, cache state, or write protocol.

pub mod assets;
pub mod config;
pub mod conflicts;
pub mod custom_css;
pub mod graph_verification;
pub mod guide;
pub mod journals;
pub mod live_conflict;
mod macro_budget;
pub mod pages;
mod parsed_text;
pub mod pdf;
pub mod print;
pub mod publish;
pub mod publish_query;
mod render;
mod render_query_cache;
mod retained;
pub use render::{SheetExport, SheetInput};
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

/// An incomplete transaction after disk failure. Its fixed wire family and
/// recovery evidence survive native adapters; ordinary I/O messages need not be
/// exposed. Inspection/display costs O(1)/O(detail bytes), with no graph work.
#[derive(Debug)]
pub enum IncompleteTransaction {
    Rollback(String),
    Publication(String),
}
impl std::fmt::Display for IncompleteTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (family, detail) = match self {
            Self::Rollback(detail) => ("rollback-incomplete", detail),
            Self::Publication(detail) => ("publication-incomplete", detail),
        };
        write!(f, "{family}: {detail}")
    }
}
impl std::error::Error for IncompleteTransaction {}

/// Preserve the original transaction refusal and every rollback/publication
/// failure location so callers can inspect disk before retrying.
fn tx_error(outcome: TxOutcome) -> io::Result<Vec<tine_store::StepResult>> {
    match outcome {
        TxOutcome::Committed { steps, .. } => Ok(steps),
        TxOutcome::PublicationIncomplete { files, .. } => Err(io::Error::other(IncompleteTransaction::Publication(format!(
            "disk write applied but final state could not be published for {}; inspect disk before retrying",
            <[&str]>::join(&files.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ", ")
        )))),
        TxOutcome::NotCommitted { why, rollback, publication_errors, .. } => {
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
                let publication = if publication_errors.is_empty() {
                    String::new()
                } else {
                    format!(
                        "; publication failed for {}",
                        <[String]>::join(&publication_errors
                            .iter()
                            .map(|(file, error)| format!("{} ({:?}: {})", file.as_str(), error.kind, error.message))
                            .collect::<Vec<_>>(), ", ")
                    )
                };
                return Err(io::Error::other(IncompleteTransaction::Rollback(format!(
                    "undo failed for {failed}; recovery: {recovery}{publication}; original: {why:?}; inspect disk before retrying"
                ))));
            }
            if !publication_errors.is_empty() {
                return Err(io::Error::other(IncompleteTransaction::Publication(format!(
                    "transaction refused ({why:?}); final state could not be published for {}; inspect disk before retrying",
                    <[String]>::join(&publication_errors
                        .iter()
                        .map(|(file, error)| format!("{} ({:?}: {})", file.as_str(), error.kind, error.message))
                        .collect::<Vec<_>>(), ", ")
                ))));
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
                    Refusal::AssetReferenced => {
                        io::Error::new(io::ErrorKind::InvalidInput, assets::AssetReferenced)
                    }
                    Refusal::UnreadableOwner { file } => io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{} cannot be read and may already be this page",
                            file.as_str()
                        ),
                    ),
                },
            })
        }
    }
}

fn is_conflict(outcome: &TxOutcome) -> bool {
    matches!(outcome, TxOutcome::NotCommitted {
        why: Why::Conflict { .. }, rollback, publication_errors, ..
    } if rollback.undo_failed.is_empty() && publication_errors.is_empty())
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
    tx.trash(id, rev, tine_store::TrashIf::Any);
    Ok(commit_retry(tx.commit())?.then_some(()))
}

#[cfg(test)]
mod error_tests;
