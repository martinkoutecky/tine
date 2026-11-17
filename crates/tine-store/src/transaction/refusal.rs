//! Transaction refusals, including the file context attached by preflight.
use super::*;

/// A transaction step rejected without authorization to overwrite user data.
#[derive(Debug)]
pub enum Refusal {
    /// Malformed writer evidence could lose imported/externally edited text.
    TextRealization(tine_core::text_trace::TraceError),
    /// A preflight text failure with the on-disk input file preserved.
    FileTextRealization {
        /// The writer's exact certificate failure.
        reason: tine_core::text_trace::TraceError,
        /// The input file, before any move.
        file: FileId,
    },
    /// Source page cannot be round-tripped safely; reason is for display.
    ReadOnly(String),
    /// Invalid or unsafe destination or page content; reason is for display.
    InvalidTarget(String),
    /// Another page file claims the same name or journal day.
    Twin {
        /// Existing claimant.
        existing: PageId,
    },
    /// Page bytes are not UTF-8.
    Undecodable,
    /// `logseq/config.edn` could not be read when the graph last loaded it
    /// (not UTF-8, too large to parse, or an undownloaded iCloud
    /// placeholder), so the graph is read-only: its `:file/name-format` is
    /// unknown and a page file would be written under another name. The
    /// cause is the read error, for diagnostics and logs, never for display.
    /// Fixing or replacing config.edn lifts it. Every transaction is refused
    /// with this before anything changes (`PagePlan::config_write_failure`).
    ConfigUnreadable(IoError),
    /// A name-only page creation was refused because a graph-text file Tine
    /// cannot read (`file`) could already be that page
    /// (R-CREATE-UNREADABLE-OWNER, docs/storage-contract.md).
    UnreadableOwner {
        /// The unreadable file or directory that could own the name.
        file: FileId,
    },
    /// A transaction named the same file more than once. Editing a page and
    /// moving that same file require separate transactions.
    RepeatedFile(FileId),
    /// The store was closed before commit.
    Closed,
    /// An orphan-only asset trash found the published graph still references
    /// the asset. The asset is kept; callers report this as a normal outcome
    /// (the block edit that dropped one reference stands), not as a failure.
    AssetReferenced,
    /// A rename's or merge's file `file` fails the conceptual model's check
    /// (R-RENAME-CHECK, docs/storage-contract.md): `P2`, it would not read as
    /// the plan intends or not sit where it should (a writer defect); `P3`,
    /// Tine's own save would rewrite its bytes (malformed input: content an
    /// external editor, a sync delivery or an import wrote that Tine's save
    /// normalizes). Nothing is written.
    ModelCheck {
        /// The promise the file breaks.
        guarantee: Guarantee,
        /// The file, at the path it has before the rename.
        file: FileId,
    },
}
