//! PDF, asset, journal, page, and conflict graph features. Each operation uses the public `tine-store`
//! boundary; callers supply device source streams, while the asset client
//! validates a selected import name without opening its path. A transaction owns every graph write. Cost is stated on each public
//! function. Callers need no graph path, lock, cache state, or write protocol.

pub mod assets;
pub mod config;
pub mod conflicts;
pub mod guide;
pub mod journals;
pub mod pages;
pub mod pdf;
pub mod print;
pub mod publish;
mod render;
pub mod search;
pub mod sources;

use std::io;
use tine_store::{Refusal, StoreError, TxOutcome, Why};

fn store_error(error: StoreError) -> io::Error {
    match error {
        StoreError::NotFound => io::Error::from(io::ErrorKind::NotFound),
        StoreError::InvalidTarget(message) => io::Error::new(io::ErrorKind::InvalidInput, message),
        StoreError::Undecodable => io::Error::new(io::ErrorKind::InvalidData, "undecodable file"),
        StoreError::Unparseable(message) => io::Error::new(io::ErrorKind::InvalidData, message),
        StoreError::TooLarge { .. } => io::Error::new(io::ErrorKind::InvalidData, "file too large"),
        StoreError::Io(error) => error,
        StoreError::Closed => io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
    }
}

fn tx_error(outcome: TxOutcome) -> io::Result<Vec<tine_store::StepResult>> {
    match outcome {
        TxOutcome::Committed { steps, .. } => Ok(steps),
        TxOutcome::NotCommitted { why, rollback, .. } => {
            if !rollback.undo_failed.is_empty() {
                let failed = rollback
                    .undo_failed
                    .iter()
                    .map(|(file, error)| {
                        format!("{} ({:?}: {})", file.as_str(), error.kind, error.message)
                    })
                    .fold(String::new(), |mut text, item| {
                        if !text.is_empty() {
                            text.push_str(", ");
                        }
                        text.push_str(&item);
                        text
                    });
                let recovery = rollback
                    .kept_external
                    .iter()
                    .filter_map(|(_, location)| location.as_ref())
                    .map(|file| file.as_str())
                    .fold(String::new(), |mut text, item| {
                        if !text.is_empty() {
                            text.push_str(", ");
                        }
                        text.push_str(item);
                        text
                    });
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

#[cfg(test)]
mod error_tests;
