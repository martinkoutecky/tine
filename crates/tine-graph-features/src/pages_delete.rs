//! Page deletion: by name ([`delete_page_expected`]) and by exact file
//! ([`delete_page_file`]). Both hand the page host one deletion bound to the
//! bytes their check read; the host's own read under the page lock refuses a
//! difference (Q-P2b-5), so neither adds a file-removal path.
use std::io;

use tine_core::model::PageKind;
use tine_store::{Area, FileRev, PageHost, PageOperation, Store, StoreError};

use super::{error, existing, refreshed_view, text_file, validate_target};
use crate::store_error;

/// Delete one named page or journal through the page host (GH #620, STEP3
/// §12, Q-P2b-5). The identity checks run here against a refreshed
/// inventory and a fresh read: twins refuse as ambiguous, and a displayed
/// path that no longer holds the name is a stale target, unless a fresh read
/// confirms that file is also absent (external deletion during
/// confirmation), which succeeds without a write; an absent name with no
/// displayed path is a no-op. The host's deletion is bound to the bytes the
/// check read and refuses if its own read differs. The window waits for a
/// `Pending` deletion to publish (§4.4). Cost one stat walk of the graph's
/// text files (O(P), [`refreshed_view`]) plus the file's bytes.
pub fn delete_page_expected(
    store: &Store,
    host: &PageHost,
    session: u64,
    name: &str,
    kind: PageKind,
    expected_path: Option<&str>,
) -> io::Result<PageOperation> {
    let graph = refreshed_view(store)?;
    let ids = existing(graph.resolve(name, kind == PageKind::Journal));
    let Some(id) = ids.first() else {
        if let Some(path) = expected_path.filter(|path| !path.trim().is_empty()) {
            let file = text_file(store, path)?;
            match store.read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
                Err(StoreError::NotFound(_)) => {}
                Err(error) => return Err(store_error(error)),
                Ok(_) => return Err(error(io::ErrorKind::NotFound, "stale page target")),
            }
            // A conflicted draft of the vanished file is kept recoverably
            // before the window discards it (L9 review R1).
            if let Some(page) = store.as_page(&file) {
                return Ok(host.delete(session, &page, None));
            }
        }
        return Ok(PageOperation::Applied);
    };
    validate_target(&ids, expected_path)?;
    let (checked, _) = store
        .read(&id.file(), Some(tine_store::PARSE_INPUT_MAX_BYTES))
        .map_err(store_error)?;
    Ok(host.delete(session, id, Some(&checked[..])))
}

/// Delete exactly the page file at graph-relative `path`, which the window
/// loaded at `base_rev` (GH #978): the one way to remove a single file of
/// several that claim one name, which a delete by name refuses as ambiguous.
/// It never resolves a name, so claimant order and twin detection are not
/// consulted, and nothing else is written: the remaining claimants keep their
/// bytes. Refusals: a path that is not a page file of this graph
/// (`InvalidInput`; in-scope scenario: a frontend defect or web content
/// passing a path), and bytes whose revision is not `base_rev`
/// (`NotFound`, "stale page target"; in-scope scenario: an external editor,
/// a sync delivery or another instance changed the file after the window
/// read it, so the user would delete text they have not seen). A file already
/// gone succeeds without a write, as for a delete by name (GH #620). Cost: one
/// read of the file's bytes, no graph walk.
pub fn delete_page_file(
    store: &Store,
    host: &PageHost,
    session: u64,
    path: &str,
    base_rev: &str,
) -> io::Result<PageOperation> {
    // Any graph text file can claim a page, not only files under the configured pages and journals folders.
    let file = store
        .file_id(Area::Graph, path)
        .map_err(|_| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let page = store
        .as_page(&file)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    match store.read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
        // A conflicted draft of the vanished file is kept recoverably.
        Err(StoreError::NotFound(_)) => Ok(host.delete(session, &page, None)),
        Err(error) => Err(store_error(error)),
        Ok((checked, rev)) if rev == FileRev::from(base_rev.to_owned()) => {
            Ok(host.delete(session, &page, Some(&checked[..])))
        }
        Ok(_) => Err(error(io::ErrorKind::NotFound, "stale page target")),
    }
}
