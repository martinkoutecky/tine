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
use tine_store::{Input, PageHost, PageId};

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
        .map_err(|pages| refused(pages.iter().next().map_or("", PageId::as_str)))?;
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
