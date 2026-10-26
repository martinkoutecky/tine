//! Settings -> Appearance -> "Edit custom.css" (GH #610): the one place the
//! stylesheet is created. Reading it stays `config::custom_css`.

use std::io;

use tine_store::{Area, Content, Store, StoreError};

use crate::store_error;

/// What a newly created `logseq/custom.css` holds: comments only, so Logseq and
/// Tine both read it as an empty stylesheet.
const CUSTOM_CSS_STARTER: &str = "/* Your Tine stylesheet. It loads after every theme and wins over it, and\n   changes apply as soon as you save. The stable variables are listed in the\n   Guide page \"Customize Tine's look\".\n\n   Example (remove the comment markers to try it):\n   :root { --tine-embed-bg: transparent; }\n*/\n";

/// The validated OS path of `logseq/custom.css`, creating the file with a short
/// commented starter through one guarded no-replace transaction when it is
/// missing. An existing file is never read, rewritten or size-checked: a user
/// must be able to open an oversized or unreadable stylesheet to repair it.
/// A concurrent creation retries and finds the file. A directory or other
/// non-file at that path is refused. Cost O(path components) plus one small
/// write when creating.
pub fn ensure_custom_css(store: &Store) -> io::Result<std::path::PathBuf> {
    let id = store
        .file_id(Area::Meta, "custom.css")
        .map_err(store_error)?;
    let path = store.path_for_os_handoff(&id, false).map_err(store_error)?;
    crate::retry_on_conflict("custom.css changed repeatedly during creation", || {
        // A zero-byte limit reads nothing: an empty file is `Ok`, a larger one
        // is `TooLarge` (still a regular file), a directory is `InvalidTarget`.
        match store.read(&id, Some(0)) {
            Ok(_) | Err(StoreError::TooLarge { .. }) => return Ok(Some(())),
            Err(StoreError::NotFound) => {}
            Err(StoreError::InvalidTarget(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    // Refusal table: docs/contracts/theme-tokens.md, "a directory, not a file".
                    "logseq/custom.css is not a regular file; not editing it",
                ));
            }
            Err(error) => return Err(store_error(error)),
        }
        let mut tx = store.transaction(None);
        tx.create(&id, Content::Bytes(CUSTOM_CSS_STARTER.as_bytes().to_vec()));
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })?;
    Ok(path)
}
