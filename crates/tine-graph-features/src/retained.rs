//! Retained writers (STEP3 §7, R6): graph-features writes to page files
//! that run outside the binding's page host. With a host, a writer reserves
//! every page it touches, so the host neither opens, saves nor drafts them
//! meanwhile; the host checks unsaved input under that final reservation
//! (Q3) with the writer's own policy, the writer's transaction runs, and the
//! pages go back to the host, which observes each. With no host the write
//! is the plain call it was.
//! Each refusal's in-scope scenario: `docs/storage-contract.md` I-8, row
//! `tine-graph-features::retained`.

use std::io;
use tine_store::{ApplyRefusal, HostOp, Input, PageHost, PageId, ReserveRefusal};

/// Run `write` as a retained writer of the pages `discover` names.
/// `discover` runs again under the reservation (a grown set starts over);
/// `write` gets the pages discovered under the final reservation, or `None`
/// with no host. A page whose unsaved input refuses the write (`Refuse`),
/// or whose save cannot complete (`Flush`), becomes `refused(page)`.
pub(crate) fn reserved<T>(
    host: Option<&PageHost>,
    input: Input,
    mut discover: impl FnMut() -> io::Result<Vec<PageId>>,
    refused: impl FnOnce(&str) -> io::Error,
    write: impl FnOnce(Option<&[PageId]>) -> io::Result<T>,
) -> io::Result<T> {
    let Some(host) = host else {
        return write(None);
    };
    let mut found = Ok(Vec::new());
    let reservation = host
        .reserve(
            || {
                found = discover();
                found.as_ref().cloned().unwrap_or_default()
            },
            input,
        )
        .map_err(|refusal| match refusal {
            ReserveRefusal::Unsaved(pages) => {
                refused(pages.iter().next().map_or("", PageId::as_str))
            }
            ReserveRefusal::Busy => crate::pages::rename_busy(),
        })?;
    // Released when it drops, on a panic's unwind too (A-R1).
    let _reservation = reservation;
    found.and_then(|pages| write(Some(&pages)))
}

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

/// Preflight conflicts consume the same bounded retry as admission's Changed.
/// Other Why values retain the transaction path's refusal mapping.
pub(crate) fn plan(tx: tine_store::Transaction<'_>) -> io::Result<HostOp> {
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
        // Another operation, a reservation, a rename or a path lock held the
        // pages past OPERATION_WAIT, or a rename held them past a flush's
        // deadline: not only a rename (REVIEW P2 round 2, finding 3).
        ApplyRefusal::Busy => error(
            io::ErrorKind::WouldBlock,
            format!("Tine is still finishing another change to these pages, so {what} did not start. Try again in a moment."),
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
    use std::cell::Cell;

    /// With no host the write is the plain call: discovery never runs, the
    /// write runs once with no reserved set, and its result is returned.
    #[test]
    fn with_no_host_a_retained_writer_is_the_plain_call() {
        let discovered = Cell::new(0);
        let written = Cell::new(0);
        let result = reserved(
            None,
            Input::Refuse,
            || {
                discovered.set(discovered.get() + 1);
                Ok(vec![PageId::from("pages/a.md")])
            },
            |_| unreachable!("no host refuses nothing"),
            |pages| {
                written.set(written.get() + 1);
                assert!(pages.is_none());
                Ok(7)
            },
        );
        assert_eq!(result.unwrap(), 7);
        assert_eq!((discovered.get(), written.get()), (0, 1));
    }

    /// A-R1, REVIEW-3a2 R1 probe 1: a writer that panics releases its
    /// reservation on the unwind, so the next writer reserves the page.
    #[test]
    fn a_panicking_writer_releases_its_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let graph = dir.path().join("graph");
        std::fs::create_dir_all(graph.join("pages")).unwrap();
        std::fs::write(graph.join("pages/a.md"), "- a\n").unwrap();
        let app = dir.path().join("app");
        std::fs::create_dir_all(&app).unwrap();
        let store = std::sync::Arc::new(
            tine_store::Store::open(&graph, Default::default())
                .unwrap()
                .0,
        );
        store.whole_graph().unwrap();
        let host = std::sync::Arc::new(PageHost::start_for_tests(&store, &app).unwrap());
        let page = PageId::from("pages/a.md");
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reserved(
                Some(&host),
                Input::Flush,
                || Ok(vec![page.clone()]),
                |_| unreachable!("nothing unsaved"),
                |_| -> io::Result<()> { panic!("the writer panics") },
            )
        }));
        assert!(unwound.is_err());
        let (done, next) = std::sync::mpsc::channel();
        let (other, page) = (std::sync::Arc::clone(&host), page.clone());
        std::thread::spawn(move || {
            let reservation = other.reserve(|| vec![page.clone()], Input::Refuse);
            let _ = done.send(reservation.is_ok());
        });
        assert_eq!(
            next.recv_timeout(std::time::Duration::from_millis(750)),
            Ok(true),
            "A-R1: a panicking writer leaked its reservation; the next writer cannot reserve"
        );
    }
}
