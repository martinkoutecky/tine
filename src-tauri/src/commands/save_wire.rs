use super::{SavePagesFailure, SavePagesWire};
use tine_store::{SaveOutcome, SavePagesOutcome};

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
        } => SavePagesWire::Failed {
            failed: SavePagesFailure {
                index,
                disk_rev: match &outcome {
                    SaveOutcome::Conflict { disk } => Some(disk.clone().into()),
                    _ => None,
                },
                family: save_outcome_to_wire(outcome).expect_err("failed page outcome"),
                undo_failed,
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
