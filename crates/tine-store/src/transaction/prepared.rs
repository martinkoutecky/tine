//! A reference rewrite prepared from the caller's planning read and kept by
//! the queued step for its own preflight, so the rewrite is computed once per
//! file and not again under the writer and page locks (GH #623).
use super::*;
use tine_core::config::FileNameFormat;

/// One page file's reference rewrite, computed by the shared rewriter
/// (`tine_core::refs::rename_rewrite`) from the bytes the caller read and
/// passed to [`Transaction::rewrite_refs`]. It never leaves the transaction:
/// callers only learn whether the rewrite changes the file.
///
/// Freshness: preflight still stages the file under the base-revision guard
/// and reuses the rewritten bytes only when the staged bytes are identical to
/// the prepared old bytes (a full byte comparison; `FileRev` is not
/// collision-resistant) and the configured filename format is unchanged; the
/// rename map is the step's own, since both are built in the same call. The rewrite is a pure function of those
/// inputs and the path, so reuse gives exactly the bytes preflight would
/// compute; any difference falls back to recomputing, never to a refusal.
/// The read-only Org and VCS-marker refusals still run in preflight on the
/// staged bytes.
#[derive(Clone, Debug)]
pub(super) struct PreparedRewrite {
    file: FileId,
    old: String,
    new: String,
    name_format: FileNameFormat,
}

impl PreparedRewrite {
    /// The rewrite of `old`, the text the caller read for `file`; `new` is
    /// `rename_rewrite(old)` under `name_format`.
    pub(super) fn new(file: FileId, old: String, new: String, name_format: FileNameFormat) -> Self {
        PreparedRewrite {
            file,
            old,
            new,
            name_format,
        }
    }

    /// The prepared bytes when `file`'s staged `old` bytes and the filename
    /// format still match what this rewrite was prepared from; `None` asks
    /// preflight to recompute. O(file bytes) comparison, no parse.
    pub(super) fn reuse(
        &self,
        file: &FileId,
        old: &[u8],
        name_format: FileNameFormat,
    ) -> Option<Vec<u8>> {
        (self.file == *file && self.old.as_bytes() == old && self.name_format == name_format)
            .then(|| self.new.clone().into_bytes())
    }
}
