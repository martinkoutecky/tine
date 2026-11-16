//! The conceptual model's check of a rename or merge, per prepared step, on
//! its final bytes (translation validation, cm-step2 P2/P3): P2, the file
//! reads as the model expects from its old reading under the plan, at the
//! path the model expects; P3, Tine's own save of the new bytes, with no
//! edit, publishes them unchanged. The expected side is `tine_core::cm_check`,
//! which shares no code with the writers it checks. A failure is a refusal
//! before any write (no repair, no retry, no skipped file). Nothing is read
//! from disk: the step's staged bytes and the writer's reading of them.

use super::*;
use tine_core::cm_check::{self, Content as Observed, Expect};
use tine_core::config::FileNameFormat;
use tine_core::model::Format;

/// The rename every step of one rename or merge is checked against, shared
/// by its steps: the model's `old` and `Plan.destination`, with the planning
/// config's journal format (no rename respells a day) and file-name codec.
/// `old: None` is the empty plan of a merge that renames nothing.
pub struct CheckPlan {
    /// The renamed name (Lean `old`); `None` renames nothing.
    pub old: Option<String>,
    /// Its new name (Lean `Plan.destination`).
    pub destination: String,
    /// The planning config's journal format.
    pub days: tine_core::date::JournalFormat,
    /// The planning config's file-name codec.
    pub names: FileNameFormat,
}

/// What the check of one rewrite or move step needs, attached by the rename
/// and merge transactions as they attach the writer's byte trace.
pub struct RenameCheck {
    /// The rename, shared by the transaction's steps.
    pub plan: Arc<CheckPlan>,
    /// The name of the file's page before the rename (Lean `P.name f`).
    pub page: String,
    /// A journal or the merge survivor: the file keeps its path.
    pub fixed: bool,
    /// The model's reading of the prepared step's old text, from the
    /// writer's parse; used only when preflight reuses that prepared text.
    pub observed: Option<Arc<Observed>>,
    /// For the merge survivor: the source's old reading (Lean `mergeContent`).
    pub source: Option<Arc<Observed>>,
}

/// Which promise a refused step breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guarantee {
    /// The new file does not read as the plan intends, or is at the wrong
    /// path: a writer bug.
    P2,
    /// Tine's own save of the new bytes would rewrite them.
    P3,
}

/// The bytes of the merge survivor and of each changed or moved file, checked
/// as `step` asks (`observed`: the old reading the step's writer made, when
/// preflight ran it). A moved file whose bytes were never read (an oversized
/// page moved as opaque bytes, which no parser reads) is refused (P3).
/// Cost: one store parse of the new text, shared by P2 and P3, the reading of
/// it, and the unchanged save's serialization (and its own parse where it
/// re-reads what it rendered); the old text is parsed again only for a
/// verbatim move (its bytes are the new ones, read once) or when no writer
/// read it.
pub(super) fn check(
    check: &RenameCheck,
    prepared: &Prepared,
    observed: Option<Arc<Observed>>,
) -> Result<(), Why> {
    let source_file = &prepared.src;
    let file = prepared.dst.as_ref().unwrap_or(source_file);
    // The file as it is on disk now: the one the user can open and save.
    let refuse = |guarantee| {
        Why::Refused(Refusal::ModelCheck {
            guarantee,
            file: source_file.clone(),
        })
    };
    // Lean `changed G H`: a file is changed unless both its path and its
    // bytes are the old ones, so a verbatim move is checked (manager ruling
    // Q6): its save at the new path must leave its bytes.
    let moved = file != source_file;
    let (Some(old), Some(new)) = (&prepared.old, &prepared.new) else {
        // Bytes never read (an oversized page moved opaque): Tine's save
        // cannot take them. A rename's planner refuses such a page before it
        // queues a move (C3W W2), so this is the store's backstop.
        return if moved {
            Err(refuse(Guarantee::P3))
        } else {
            Ok(())
        };
    };
    let unchanged = !moved && old == new;
    // Bytes that are not text cannot be read or saved: moved as they are,
    // the save refuses them (P3); changed into them, they cannot read as the
    // plan intends (P2).
    let (Ok(text), Ok(old_text)) = (std::str::from_utf8(new), std::str::from_utf8(old)) else {
        return match (unchanged, old == new) {
            (true, _) => Ok(()),
            (false, true) => Err(refuse(Guarantee::P3)),
            (false, false) => Err(refuse(Guarantee::P2)),
        };
    };
    let path = Path::new(file.as_str());
    let format = Format::from_path(path);
    let plan = &check.plan;
    let parsed = crate::model::parse_doc_with_opts(path, text);
    let reading = cm_check::observe_parsed(text, &parsed.0, format, plan.names);
    let before = match observed {
        Some(before) => before,
        None if old == new => Arc::new(reading.clone()),
        None => Arc::new(cm_check::observe(
            old_text,
            Format::from_path(Path::new(source_file.as_str())),
            plan.names,
        )),
    };
    let rename = match &plan.old {
        Some(name) => cm_check::Rename::new(name, &plan.destination, Some(&plan.days), plan.names),
        None => cm_check::Rename::none(Some(&plan.days), plan.names),
    };
    let expected_path =
        cm_check::expected_path(&rename, source_file.as_str(), &check.page, check.fixed);
    if expected_path != file.as_str() {
        return Err(refuse(Guarantee::P2));
    }
    let input = match &check.source {
        Some(source) => Expect::Survivor {
            source,
            target: &before,
        },
        None => Expect::File {
            content: &before,
            skip: cm_check::skip_file(old_text, format, source_file.as_str(), &expected_path),
        },
    };
    let expected = cm_check::expected(&rename, format, input);
    if !cm_check::matches(&expected, &reading, check.source.is_some()) {
        return Err(refuse(Guarantee::P2));
    }
    // P3: every changed file, a moved one included, saves unchanged. The one
    // exception is SPEC O4's: "a moved file with conflict markers needs no
    // rewrite (it moves verbatim)"; the save refuses it (R-VCS-MARKERS) and
    // publishes nothing, which leaves its bytes and its path (open question
    // Q9 for the manager: ruling Q6 and O4 disagree on this file).
    let saved = crate::cost_counters::model_check_save(|| {
        crate::model::unchanged_save::unchanged_save(path, Some(file.as_str()), text, parsed)
    });
    match saved {
        Ok(saved) if saved == text => Ok(()),
        Err(_) if unchanged => Ok(()),
        Err(_)
            if old == new
                && !tine_core::concord_queue::vcs_conflict_markers(text, format).is_empty() =>
        {
            Ok(())
        }
        _ => Err(refuse(Guarantee::P3)),
    }
}

/// Test fault: a writer bug in a checked step's output. Rewrite and move
/// steps whose final bytes hold the sentinel lose it before the check runs,
/// so a step the check skips would write the damaged bytes.
#[cfg(any(test, feature = "test-faults"))]
pub(super) fn writer_bug(step: &Step, prepared: &mut Prepared) {
    if !matches!(step, Step::Rewrite { .. } | Step::Move { .. }) {
        return;
    }
    let sentinel = crate::model::TEST_RENAME_WRITER_BUG_SENTINEL;
    if let Some(text) = prepared
        .new
        .as_ref()
        .and_then(|new| std::str::from_utf8(new).ok())
        .filter(|text| text.contains(sentinel))
    {
        prepared.new = Some(text.replacen(sentinel, "", 1).into_bytes());
    }
}
