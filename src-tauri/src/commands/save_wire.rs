use super::{SavePagesFailure, SavePagesWire};
use tine_store::{SaveOutcome, SavePagesOutcome};

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
        } => SavePagesWire::Failed {
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
            },
        },
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
