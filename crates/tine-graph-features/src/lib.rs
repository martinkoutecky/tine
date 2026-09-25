//! PDF and asset graph features. Each operation uses the public `tine-store`
//! boundary; callers supply device source streams and receive values or I/O
//! errors. A transaction owns every graph write. Cost is stated on each public
//! function. Callers need no graph path, lock, cache state, or write protocol.

pub mod assets;
pub mod pdf;

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
        TxOutcome::NotCommitted { why, .. } => Err(match why {
            Why::Conflict { .. } => {
                io::Error::new(io::ErrorKind::WouldBlock, "concurrent graph write")
            }
            Why::Failed(error) => io::Error::new(error.kind, error.message),
            Why::Refused(refusal) => match refusal {
                Refusal::ReadOnly(message) | Refusal::InvalidTarget(message) => {
                    io::Error::new(io::ErrorKind::InvalidInput, message)
                }
                Refusal::Twin { .. } => io::Error::new(io::ErrorKind::AlreadyExists, "twin page"),
                Refusal::Undecodable => {
                    io::Error::new(io::ErrorKind::InvalidData, "undecodable file")
                }
                Refusal::RepeatedFile(_) => {
                    io::Error::new(io::ErrorKind::InvalidInput, "repeated file")
                }
                Refusal::Closed => io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
            },
        }),
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
