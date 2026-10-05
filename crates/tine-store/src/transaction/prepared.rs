//! A reference rewrite prepared from the caller's planning read and kept by
//! the transaction for its own preflight, so the rewrite is computed once per
//! file and not again under the writer and page locks (GH #623).
use super::*;
use tine_core::config::FileNameFormat;

/// One page file's reference rewrite, computed by the store's own rewriter
/// from the bytes the caller read. It never leaves the transaction: callers
/// only learn whether the rewrite changes the file.
///
/// Freshness: preflight still stages the file under the base-revision guard
/// and reuses the rewritten bytes only when the staged bytes are identical to
/// the prepared old bytes (a full byte comparison; `FileRev` is not
/// collision-resistant), the rename map is the step's and the configured
/// filename format is unchanged. The rewrite is a pure function of those
/// inputs and the path, so reuse gives exactly the bytes preflight would
/// compute; any difference falls back to recomputing, never to a refusal.
/// The read-only Org and VCS-marker refusals still run in preflight on the
/// staged bytes.
#[derive(Clone, Debug)]
pub(super) struct PreparedRewrite {
    file: FileId,
    old: String,
    new: String,
    renames: RenameMap,
    name_format: FileNameFormat,
}

impl PreparedRewrite {
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

impl Transaction<'_> {
    /// Whether rewriting `renames` changes `file`'s `text`, as the caller
    /// read it, with the configured filename format. A changing rewrite is
    /// kept for a later [`Self::rewrite_refs`] of the same file and map, whose
    /// preflight then reuses it instead of rewriting again; preflight keeps
    /// the base-revision guard and every refusal. No disk access. Cost O(text)
    /// for the reference rewrite plus one copy of `text` when it changes.
    pub fn prepare_ref_rewrite(&mut self, file: &FileId, text: &str, renames: &RenameMap) -> bool {
        let name_format = self.store.config().file_name_format;
        let is_org = Path::new(file.as_str())
            .extension()
            .and_then(|ext| ext.to_str())
            == Some("org");
        let new = validation::rewritten_text(text, is_org, renames, name_format);
        let changed = new != text;
        if changed {
            self.prepared.insert(
                file.clone(),
                PreparedRewrite {
                    file: file.clone(),
                    old: text.to_owned(),
                    new,
                    renames: renames.clone(),
                    name_format,
                },
            );
        } else {
            self.prepared.remove(file);
        }
        changed
    }

    /// The rewrite [`Self::prepare_ref_rewrite`] kept for `file` and exactly
    /// this rename map, if any.
    pub(super) fn take_prepared(
        &mut self,
        file: &FileId,
        renames: &RenameMap,
    ) -> Option<Box<PreparedRewrite>> {
        self.prepared
            .remove(file)
            .filter(|prepared| prepared.renames.0 == renames.0)
            .map(Box::new)
    }
}
