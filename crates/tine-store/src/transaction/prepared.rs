//! A reference rewrite prepared from the caller's planning read and kept by
//! the queued step for its own preflight, so the rewrite is computed once per
//! file and not again under the writer and page locks (GH #623).
use super::*;
use tine_core::config::FileNameFormat;

impl PagePlan<'_> {
    pub(super) fn record_kinds(&mut self, kinds: &[crate::EditKind]) {
        assert!(!kinds.is_empty(),"OG-RULES Rule 8: page save needs a kind; exemplar crates/tine-graph-features/src/pages.rs");
        for kind in kinds {
            if !self.kinds.contains(kind) {
                self.kinds.push(*kind);
            }
        }
    }
    /// Queue certified physical bytes from the planning read. `Some(renames)`
    /// permits a stale reference rewrite to be recomputed; `None` is a merge
    /// survivor and refuses a mismatch rather than recomputing only references.
    /// Preflight checks the revision, exact old bytes, filename format and
    /// complete-text certificate under the Org and conflict-marker firewalls.
    /// O(file bytes), no additional I/O beyond the ordinary staged rewrite.
    pub fn rewrite_refs_prepared(
        &mut self,
        kinds: &[crate::EditKind],
        id: &PageId,
        expected: FileRev,
        text: (String, String),
        renames: Option<&RenameMap>,
        format: FileNameFormat,
        certificate: tine_core::text_trace::TextTrace,
    ) {
        self.record_kinds(kinds);
        self.steps.push(Step::Rewrite {
            id: id.clone(),
            expected,
            renames: renames.cloned().unwrap_or_default(),
            rebind_title: false,
            prepared: Some(Box::new(PreparedRewrite::new(
                id.file(),
                text.0,
                text.1,
                format,
                certificate,
                renames.is_none(),
            ))),
        });
    }
}

/// One page file's reference rewrite or certified merge survivor, prepared
/// from the bytes the caller read. It never leaves the transaction:
/// callers only learn whether the rewrite changes the file.
///
/// Freshness: preflight still stages the file under the base-revision guard
/// and reuses the rewritten bytes only when the staged bytes are identical to
/// the prepared old bytes (a full byte comparison; `FileRev` is not
/// collision-resistant) and the configured filename format is unchanged; the
/// rename map is the step's own, since both are built in the same call. The rewrite is a pure function of those
/// inputs and the path, so reuse gives exactly the bytes preflight would
/// compute. Reference rewrites recompute on a mismatch; strict merge survivors
/// refuse because recomputing references alone cannot reproduce the merge.
/// The read-only Org and VCS-marker refusals still run in preflight on the
/// staged bytes.
#[derive(Clone, Debug)]
pub(super) struct PreparedRewrite {
    file: FileId,
    old: String,
    new: String,
    name_format: FileNameFormat,
    certificate: tine_core::text_trace::TextTrace,
    strict: bool,
}

impl PreparedRewrite {
    pub(super) fn is_merge(&self) -> bool {
        self.strict
    }
    /// Prepared output for the caller's `old` bytes and filename format.
    pub(super) fn new(
        file: FileId,
        old: String,
        new: String,
        name_format: FileNameFormat,
        certificate: tine_core::text_trace::TextTrace,
        strict: bool,
    ) -> Self {
        PreparedRewrite {
            file,
            old,
            new,
            name_format,
            certificate,
            strict,
        }
    }

    /// The prepared bytes when `file`'s staged `old` bytes and the filename
    /// format still match what this rewrite was prepared from. On mismatch,
    /// reference rewrites return `None` for recomputation; merges refuse.
    /// O(file bytes) comparison, no parse.
    pub(super) fn reuse(
        &self,
        file: &FileId,
        old: &[u8],
        name_format: FileNameFormat,
    ) -> Result<Option<Vec<u8>>, Why> {
        let matches =
            self.file == *file && self.old.as_bytes() == old && self.name_format == name_format;
        if !matches {
            if self.strict {
                return Err(Why::Refused(Refusal::TextRealization(
                    tine_core::text_trace::TraceError::Partition,
                )));
            }
            return Ok(None);
        }
        let trace = &self.certificate;
        if trace.inputs.first().map(String::as_str) != Some(self.old.as_str()) {
            return Err(Why::Refused(Refusal::TextRealization(
                tine_core::text_trace::TraceError::Partition,
            )));
        }
        trace
            .check(&self.new, &trace.substitution)
            .map_err(|e| Why::Refused(Refusal::TextRealization(e)))?;
        Ok(Some(self.new.clone().into_bytes()))
    }
}
