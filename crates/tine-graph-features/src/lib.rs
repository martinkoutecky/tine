//! PDF, asset, journal, page, and conflict graph features. Each operation uses the public `tine-store`
//! boundary; callers supply device source streams, while the asset client
//! validates a selected import name without opening its path. Store-backed
//! writes use guarded transactions or the Store's graph creation/publish paths.
//! Callers need no graph path, lock, cache state, or write protocol.

#[cfg(test)]
#[path = "../../test-support/temp_dir.rs"]
mod test_support;

pub mod assets;
mod build_graph;
pub mod config;
pub mod conflicts;
pub mod custom_css;
pub mod graph_verification;
pub mod guide;
pub mod inventory_wire;
pub mod journals;
pub mod live_conflict;
mod macro_budget;
pub mod pages;
mod parsed_text;
pub mod pdf;
pub mod print;
pub mod publish;
pub mod publish_query;
mod published_answers;
pub mod quick_share;
mod render;
mod render_query_cache;
mod retained;
pub mod run_revert;
pub use render::{SheetExport, SheetInput};
pub mod search;
pub mod templates;

use std::io;
use tine_store::{FileId, Refusal, Store, StoreError, TxOutcome, Why};

fn store_error(error: StoreError) -> io::Error {
    match error {
        StoreError::NotFound(_) => io::Error::from(io::ErrorKind::NotFound),
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

/// A write refused because `logseq/config.edn` could not be read, which keeps
/// the graph read-only ([`Refusal::ConfigUnreadable`]). The
/// refusal's type survives native adapters as the fixed wire family
/// `config-unreadable`, and the read error follows it for logs and the CLI; the
/// window shows its own catalog sentence for the family, never the detail.
/// After a partial operation ([`after_partial`]) it also carries what was
/// already saved, as `config-unreadable-after: <done>` and the detail on the
/// next line, so the window can say both.
/// Inspection/display costs O(1)/O(detail bytes), with no graph work.
#[derive(Debug)]
pub struct ConfigUnreadable {
    pub after: Option<String>,
    pub detail: String,
}
impl std::fmt::Display for ConfigUnreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.after {
            None => write!(f, "config-unreadable: {}", self.detail),
            Some(done) => write!(f, "config-unreadable-after: {done}\n{}", self.detail),
        }
    }
}
impl std::error::Error for ConfigUnreadable {}

/// Preserve a config read problem at the transaction or Settings boundary.
/// The typed family owns its display; conversion performs no I/O, O(1).
fn config_unreadable(cause: tine_store::IoError) -> io::Error {
    io::Error::new(
        cause.kind,
        ConfigUnreadable {
            after: None,
            detail: cause.message,
        },
    )
}

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
            Err(why_error(why))
        }
    }
}

/// A refusal or failure as the caller's I/O error.
fn why_error(why: Why) -> io::Error {
    match why {
        Why::Conflict { .. } => io::Error::new(io::ErrorKind::WouldBlock, "concurrent graph write"),
        Why::Failed(error) => io::Error::new(error.kind, error.message),
        Why::Refused(refusal) => match refusal {
            Refusal::TextRealization(error) => io::Error::new(io::ErrorKind::InvalidData, error),
            Refusal::ReadOnly(message) | Refusal::InvalidTarget(message) => {
                io::Error::new(io::ErrorKind::InvalidInput, message)
            }
            Refusal::Twin { .. } => io::Error::new(io::ErrorKind::AlreadyExists, "twin page"),
            Refusal::Undecodable => io::Error::new(io::ErrorKind::InvalidData, "undecodable file"),
            Refusal::ConfigUnreadable(cause) => config_unreadable(cause),
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
    }
}

/// A failure after part of an operation already completed. The error keeps
/// its kind, which callers branch on, and its message leads with what was
/// saved, so the user is not told the whole operation failed (P3 review #7).
fn after_partial(done: &str, error: io::Error) -> io::Error {
    // A config-unreadable refusal keeps its type: the window words it.
    if let Some(refusal) = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<ConfigUnreadable>())
    {
        let detail = refusal.detail.clone();
        let after = Some(done.to_owned());
        return io::Error::new(error.kind(), ConfigUnreadable { after, detail });
    }
    let text = error.to_string();
    let mut chars = text.chars();
    let rest: String = chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect())
        .unwrap_or_default();
    io::Error::new(error.kind(), format!("{done}, but {rest}"))
}

fn is_conflict(outcome: &TxOutcome) -> bool {
    matches!(outcome, TxOutcome::NotCommitted {
        why: Why::Conflict { .. }, rollback, publication_errors, ..
    } if rollback.undo_failed.is_empty() && publication_errors.is_empty())
}

// `retry_on_conflict` attempts on this thread: the bound a test counts.
#[cfg(test)]
thread_local! {
    pub(crate) static ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn retry_on_conflict<T>(
    exhausted: &'static str,
    mut attempt: impl FnMut() -> io::Result<Option<T>>,
) -> io::Result<T> {
    for _ in 0..4 {
        #[cfg(test)]
        ATTEMPTS.with(|n| n.set(n.get() + 1));
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
        Err(StoreError::NotFound(_)) => {
            return Err(io::Error::new(io::ErrorKind::NotFound, missing))
        }
        Err(error) => return Err(store_error(error)),
    };
    // Only called for a file that is not a page (`Store::as_page`); a page
    // is trashed through a host operation.
    let mut tx = store.transaction(None);
    tx.trash(id, rev, tine_store::TrashIf::Any);
    Ok(commit_retry(tx.commit())?.then_some(()))
}

#[cfg(test)]
mod error_tests;

#[cfg(test)]
use test_host::test_host;

#[cfg(test)]
use tine_store as store_crate;
#[cfg(test)]
#[path = "../../tine-store/src/test_host_writer.rs"]
mod test_host_writer;

#[cfg(test)]
mod test_host {
    /// A running page host for `store`, the only writer of page files
    /// (host-only-writer DESIGN §3.6), under its own app data; it derefs to
    /// the host, so `&test_host(&store)` is a writer's `&PageHost`. Drop it
    /// before removing the graph.
    pub(crate) struct TestHost {
        host: tine_store::PageHost,
        _app_data: tempfile::TempDir,
    }

    impl std::ops::Deref for TestHost {
        type Target = tine_store::PageHost;
        fn deref(&self) -> &tine_store::PageHost {
            &self.host
        }
    }

    pub(crate) fn test_host(store: &std::sync::Arc<tine_store::Store>) -> TestHost {
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
        TestHost {
            host,
            _app_data: app_data,
        }
    }
}
