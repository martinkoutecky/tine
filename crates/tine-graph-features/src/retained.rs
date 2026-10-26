//! Feature page writes run as operations on the binding's page host.
//! Every writer needs a running host and uses `operation`.
//! Each refusal's in-scope scenario: `docs/storage-contract.md` I-8, row
//! `tine-graph-features::retained`.

use std::io;
use tine_store::{ApplyRefusal, HostOp, Input, PageHost, PageId};

/// The pages among `files`, for a writer whose key set is its arguments.
pub(crate) fn pages<'a>(
    store: &tine_store::Store,
    files: impl IntoIterator<Item = &'a tine_store::FileId>,
) -> Vec<PageId> {
    files
        .into_iter()
        .filter_map(|file| store.as_page(file))
        .collect()
}

/// The refusal of a flush-first writer whose page cannot be saved.
pub(crate) fn unsaved(page: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        format!("“{page}” has changes Tine could not save. Save or discard them, then try again."),
    )
}

#[derive(Debug)]
struct Replan;
impl std::fmt::Display for Replan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("concurrent graph write")
    }
}
impl std::error::Error for Replan {}

/// Preserve the admission refusal's type through io::Error so a rename
/// can retain its own busy wording without recognizing message text.
#[derive(Debug)]
pub(crate) struct Busy(String);
impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Busy {}

/// Preflight conflicts consume the same bounded retry as admission's Changed.
/// Other Why values retain the transaction path's refusal mapping.
pub(crate) fn plan(tx: tine_store::PagePlan<'_>) -> io::Result<HostOp> {
    tx.plan().map_err(|why| match why {
        tine_store::Why::Conflict { .. } => io::Error::new(io::ErrorKind::WouldBlock, Replan),
        why => crate::why_error(why),
    })
}

/// A feature's write as one host operation (host-only-writer P2, DESIGN
/// §3.1), planned from the store, under the writer's `input` policy, which
/// the operation carries. `plan` reads and computes the operation, or None
/// when there is nothing to write; the host applies it, and a page that
/// changed since the plan replans, as a store conflict does, `exhausted`
/// after four tries. `what` names the change in messages; `refused(page)`
/// is the writer's unsaved-input error.
///
/// `Flush` saves the unsaved input of `pages`, the pages the writer expects
/// to touch, before the first plan: store reads answer saved bytes, so a
/// page that exists only as typed input would plan as absent (REVIEW P2). A
/// plan that names a page outside the saved set saves that one too and
/// plans again. `Refuse` saves nothing: unsaved input on a planned page
/// refuses the operation (`apply`). Finding B: no message claims more than
/// its evidence.
pub(crate) fn operation(
    host: &PageHost,
    input: Input,
    pages: &[PageId],
    what: &str,
    exhausted: &'static str,
    refused: impl Fn(&str) -> io::Error,
    mut plan: impl FnMut() -> io::Result<Option<HostOp>>,
) -> io::Result<()> {
    let fail = |refusal| refusal_error(refusal, what, &refused);
    let mut saved: Vec<PageId> = Vec::new();
    if input == Input::Flush {
        host.flush_input(pages).map_err(fail)?;
        saved.extend_from_slice(pages);
    }
    crate::retry_on_conflict(exhausted, || {
        let planned = match plan() {
            Err(error) if error.get_ref().is_some_and(|cause| cause.is::<Replan>()) => {
                return Ok(None)
            }
            result => result?,
        };
        let Some(mut op) = planned else {
            return Ok(Some(()));
        };
        op.policy = input;
        if input == Input::Flush {
            let unsaved: Vec<PageId> = (op.pages.iter())
                .map(|(page, ..)| page.clone())
                .filter(|page| !saved.contains(page))
                .collect();
            if !unsaved.is_empty() {
                host.flush_input(&unsaved).map_err(fail)?;
                saved.extend(unsaved);
                return Ok(None);
            }
        }
        match host.apply(op) {
            Ok(()) => Ok(Some(())),
            Err(ApplyRefusal::Changed) => Ok(None),
            Err(refusal) => Err(fail(refusal)),
        }
    })
}

fn refusal_error(
    refusal: ApplyRefusal,
    what: &str,
    refused: &impl Fn(&str) -> io::Error,
) -> io::Error {
    let error = |kind, message: String| io::Error::new(kind, message);
    match refusal {
        // Only an applied operation's pages change; `operation` replans.
        ApplyRefusal::Changed => error(
            io::ErrorKind::WouldBlock,
            format!(
                "{} raced another change; nothing changed.",
                capitalized(what)
            ),
        ),
        ApplyRefusal::Unsaved(page) => refused(page.as_str()),
        // Another operation, a rename or a path lock held the
        // pages past OPERATION_WAIT, or a rename held them past a flush's
        // deadline: not only a rename (REVIEW P2 round 2, finding 3).
        ApplyRefusal::Busy => io::Error::new(
            io::ErrorKind::WouldBlock,
            Busy(format!("Tine is still finishing another change to these pages, so {what} did not start. Try again in a moment.")),
        ),
        ApplyRefusal::Refused => error(
            io::ErrorKind::Other,
            format!("Tine could not run {what}; nothing changed."),
        ),
        ApplyRefusal::DraftFailed => error(
            io::ErrorKind::Other,
            format!(
                "{} could not be recorded; nothing changed.",
                capitalized(what)
            ),
        ),
        ApplyRefusal::Unwritten(page) => error(
            io::ErrorKind::WouldBlock,
            format!(
                "“{}” could not be written; {what} finishes once it can be.",
                page.as_str()
            ),
        ),
        ApplyRefusal::Superseded(page) => error(
            io::ErrorKind::Other,
            format!(
                "“{}” changed before {what} finished; your later edit stands.",
                page.as_str()
            ),
        ),
        ApplyRefusal::Uncertain => error(
            io::ErrorKind::TimedOut,
            format!("Couldn't confirm {what} yet; Tine keeps trying to finish it."),
        ),
        ApplyRefusal::SpellingIncomplete => error(
            io::ErrorKind::TimedOut,
            "The file name may already be changed; the text change is not confirmed. Save or discard any pending edits, then open Rename and rename again to the intended name. If the page already shows that name, check its text before retrying.".into(),
        ),
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_spelling_reports_entry_first_changes_and_an_ordinary_retry() {
        let err = refusal_error(
            ApplyRefusal::SpellingIncomplete,
            "the rename",
            &|_| unreachable!(),
        );
        let message = err.to_string();
        assert!(message.contains("file name may already be changed"));
        assert!(message.contains("text change is not confirmed"));
        assert!(message.contains("rename again to the intended name"));
        assert!(!message.contains("nothing changed"));
        assert!(!message.contains("keeps trying"));
        // A case-sensitive rename still has a drafted entry operation.
        let ordinary = refusal_error(ApplyRefusal::Uncertain, "the rename", &|_| unreachable!());
        assert!(ordinary
            .to_string()
            .contains("Tine keeps trying to finish it"));
    }

    #[test]
    fn busy_host_rename_keeps_its_existing_refusal_wording() {
        let refusal = || refusal_error(ApplyRefusal::Busy, "the rename", &|_| unreachable!());
        let err = crate::pages::rename_operation_error(refusal());
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            err.to_string(),
            "Another rename of these pages is still finishing; try again in a moment."
        );
        // Other operation writers keep their existing refusal wording.
        assert_eq!(refusal_error(ApplyRefusal::Busy, "the merge", &|_| unreachable!()).to_string(),
            "Tine is still finishing another change to these pages, so the merge did not start. Try again in a moment.");
    }
}
