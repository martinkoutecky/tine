use serde::Serialize;
use tine_store::{IoError, SaveOutcome, SavePagesOutcome, StoreError};

/// A failed `save_pages` group. `operation` and `os_error` name the platform
/// step of an `io:` failure when the store knows it (GH #538, #590); the
/// operation is a fixed literal, never a path (I-5).
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePagesFailure {
    index: usize,
    family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_rev: Option<String>,
    undo_failed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    publication_errors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    os_error: Option<i32>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum SavePagesWire {
    Ok { ok: Vec<String> },
    Failed { failed: SavePagesFailure },
}

/// The platform step of an I/O failure, when the store named one. O(1).
fn platform_step(error: Option<&IoError>) -> (Option<&'static str>, Option<i32>) {
    error.map_or((None, None), |error| (error.operation, error.os_error))
}

/// Encode Saved and Unchanged as file-revision strings. On failure, encode
/// a fixed family and a disk revision only for Conflict; any publication
/// errors select the publication-incomplete family. Consumes the outcome,
/// does no I/O, and costs O(results + path strings). Panics if handed an
/// impossible constructed Store outcome.
pub(super) fn save_pages_outcome_to_wire(outcome: SavePagesOutcome) -> SavePagesWire {
    match outcome {
        SavePagesOutcome::Ok(outcomes) => SavePagesWire::Ok {
            ok: outcomes
                .into_iter()
                .map(|outcome| save_outcome_to_wire(outcome).expect("committed page rev"))
                .collect(),
        },
        SavePagesOutcome::Failed {
            index,
            outcome,
            undo_failed,
            publication_errors,
        } => {
            let (operation, os_error) = platform_step(match &outcome {
                SaveOutcome::Io(error) if publication_errors.is_empty() => Some(error),
                _ => None,
            });
            SavePagesWire::Failed {
                failed: SavePagesFailure {
                    index,
                    disk_rev: match &outcome {
                        SaveOutcome::Conflict { disk } => Some(disk.clone().into()),
                        _ => None,
                    },
                    family: if publication_errors.is_empty() {
                        save_outcome_to_wire(outcome).expect_err("failed page outcome")
                    } else {
                        "publication-incomplete".into()
                    },
                    undo_failed: undo_failed
                        .into_iter()
                        .map(|id| id.as_str().to_owned())
                        .collect(),
                    publication_errors: publication_errors
                        .into_iter()
                        .map(|id| id.as_str().to_owned())
                        .collect(),
                    operation,
                    os_error,
                },
            }
        }
    }
}

pub(super) fn save_outcome_to_wire(outcome: SaveOutcome) -> Result<String, String> {
    match outcome {
        SaveOutcome::Saved(rev) | SaveOutcome::Unchanged(rev) => Ok(rev.into()),
        SaveOutcome::Conflict { .. } => Err("conflict".into()),
        SaveOutcome::Deleted => Err("deleted".into()),
        SaveOutcome::ReadOnly(_) => Err("read-only".into()),
        SaveOutcome::InvalidTarget(_) => Err("invalid-target".into()),
        SaveOutcome::Twin { .. } => Err("twin".into()),
        SaveOutcome::Repeated => Err("repeated".into()),
        SaveOutcome::Io(error) => Err(format!("io:{:?}", error.kind())),
        SaveOutcome::Closed => Err("closed".into()),
        SaveOutcome::GuideEphemeral => Err("invalid-target".into()),
    }
}

/// Encode a Store error raised before the page transaction as a Failed wire
/// value at `index`: a fixed family, no disk revision, no undo or publication
/// lists. Pure; O(1).
pub(super) fn store_failure_to_wire(index: usize, error: StoreError) -> SavePagesWire {
    let (operation, os_error) = platform_step(match &error {
        StoreError::Io(error) => Some(error),
        _ => None,
    });
    let family = match error {
        StoreError::NotFound => "deleted".into(),
        StoreError::InvalidTarget(_)
        | StoreError::PageSource(_)
        | StoreError::StreamSymlink(_)
        | StoreError::Undecodable
        | StoreError::Unparseable(_) => "invalid-target".into(),
        StoreError::TooLarge { .. } => "asset-too-large".into(),
        StoreError::Io(error) => format!("io:{:?}", error.kind()),
        StoreError::Closed => "closed".into(),
    };
    SavePagesWire::Failed {
        failed: SavePagesFailure {
            index,
            family,
            disk_rev: None,
            undo_failed: Vec::new(),
            publication_errors: Vec::new(),
            operation,
            os_error,
        },
    }
}

type SaveEntry = (
    tine_store::PageId,
    tine_core::model::PageDto,
    Option<String>,
    bool,
    Vec<tine_store::EditKind>,
);

/// Save prepared page entries through `save` (the command names its store
/// route, which the Rule 8 census reads) and encode the result, as the
/// `save_pages` command does once its binding and edit kinds are checked;
/// records the fixed-shape diagnostic event. Cost is the store save's.
pub(super) fn save_pages_wire(
    store: &tine_store::Store,
    entries: &[SaveEntry],
    save: impl FnOnce(&tine_store::Store, &[SaveEntry]) -> Result<SavePagesOutcome, (usize, StoreError)>,
) -> SavePagesWire {
    let started = std::time::Instant::now();
    let wire = match save(store, entries) {
        Ok(outcome) => save_pages_outcome_to_wire(outcome),
        Err((index, error)) => store_failure_to_wire(index, error),
    };
    record_save_wire(&wire, entries.len(), started.elapsed());
    wire
}

/// Hand one finished `save_pages` result to the diagnostic recorder: only its
/// fixed failure family, the page count and the duration (GH #343, I-5).
/// Does no I/O; O(1).
pub(super) fn record_save_wire(wire: &SavePagesWire, pages: usize, elapsed: std::time::Duration) {
    let failure = match wire {
        SavePagesWire::Ok { .. } => None,
        SavePagesWire::Failed { failed } => Some(failed.family.as_str()),
    };
    crate::flight::record_save(failure, pages, elapsed);
}

#[cfg(test)]
#[path = "save_wire_tests.rs"]
mod tests;
