//! Persistent-format identity for `tine-storage`.
//!
//! This module is the single citable place for every constant that describes
//! something already written to disk. The storage release receipt and Tine's
//! storage pin receipt both quote [`FORMAT_MANIFEST`], so a format change is a
//! visible, reviewable event rather than an incidental diff.
//!
//! # The rule this module exists to enforce
//!
//! **On-disk format versions are never inferred from the crate's semver.** The
//! crate version tracks the Rust API; these constants track the bytes. They
//! move independently and for different reasons: an API-breaking refactor that
//! reads and writes identical bytes does not touch anything here, and a
//! one-field change to a stored envelope does — even in a patch release.
//!
//! # What belongs here
//!
//! A constant belongs in the manifest when a reader must agree with a writer
//! about it: envelope versions, magic values, on-disk file and directory names,
//! layout geometry, and the bounds a writer may legally have produced (a reader
//! that lowers such a bound stops being able to read older data).
//!
//! Deliberately **excluded**: in-memory budgets and read-path limits, which a
//! future version may change freely without stranding stored bytes. That is why
//! `MAX_MATERIALIZATION_QUERY_ROWS`, `MAX_MATERIALIZATION_QUERY_BYTES`,
//! `MAX_MATERIALIZATION_READ_BYTES` and
//! `MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES` are not listed: they bound one
//! process's work, not the bytes it left behind.
//!
//! # Adding a constant
//!
//! Re-export it below, add one [`FormatConstant`] row, and update the value in
//! `format_identity_is_pinned`. That test asserts exact values, so changing an
//! on-disk format cannot pass CI without an explicit edit that a reviewer sees.
//!
//! The definitions themselves stay in their owning modules; this module only
//! re-exports them, so listing a constant here can never change its value.

// --- format identity: envelope/schema versions and magic ---------------------
pub use crate::durable_batch::{
    MANIFEST_ENCODING_VERSION, OBJECT_ENVELOPE_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};
pub use crate::local_journal::LOCAL_JOURNAL_FRAME_SCHEMA_VERSION;
pub use crate::scratch::{SCRATCH_PAGE_SCHEMA_VERSION, SCRATCH_SCHEMA_VERSION};
pub use crate::sqlite_frontier::{SQLITE_APPLICATION_ID, SQLITE_SCHEMA_VERSION};

// --- on-disk layout: names and shape -----------------------------------------
pub use crate::scratch::{
    SCRATCH_BLOBS_FILE, SCRATCH_DIR, SCRATCH_LEASE_FILE, SCRATCH_LSM_LEVELS, SCRATCH_MARKER_FILE,
    SCRATCH_PAGES_FILE,
};

// --- bounds a writer may legally have produced -------------------------------
pub use crate::durable_batch::{MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES};
pub use crate::local_journal::{
    MAX_LOCAL_JOURNAL_FRAME_BYTES, MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES,
    MAX_LOCAL_JOURNAL_SEGMENT_BYTES,
};
pub use crate::scratch::{MAX_SCRATCH_BLOB_BYTES, MAX_SCRATCH_PAGE_BYTES};

// --- checkpoint fingerprint geometry -----------------------------------------
// A stored checkpoint is only comparable to a fresh one computed with the same
// geometry, so these values are part of the stored artifact's meaning.
pub use crate::sqlite_fileset::{
    MAX_SQLITE_CHECKPOINT_BYTES, SQLITE_CHECKPOINT_EDGE_BYTES,
    SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES, SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES,
};

/// What kind of compatibility obligation a constant carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    /// An envelope/schema version or magic value. A change means stored bytes
    /// of the old value must still be readable or explicitly migrated.
    Identity,
    /// A file name, directory name, or structural shape on disk.
    Layout,
    /// A limit a writer may have produced up to. Lowering one can strand data.
    WriterBound,
    /// Geometry that determines how a stored fingerprint was computed.
    CheckpointGeometry,
}

/// A constant's value, in the shape it takes on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatValue {
    Number(u64),
    Name(&'static str),
}

/// One row of the persistent-format manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatConstant {
    /// The constant's Rust name, as re-exported from this module.
    pub name: &'static str,
    /// Which on-disk artifact it governs.
    pub artifact: &'static str,
    pub kind: FormatKind,
    pub value: FormatValue,
}

const fn num(
    name: &'static str,
    artifact: &'static str,
    kind: FormatKind,
    v: u64,
) -> FormatConstant {
    FormatConstant {
        name,
        artifact,
        kind,
        value: FormatValue::Number(v),
    }
}

const fn name_of(
    name: &'static str,
    artifact: &'static str,
    kind: FormatKind,
    v: &'static str,
) -> FormatConstant {
    FormatConstant {
        name,
        artifact,
        kind,
        value: FormatValue::Name(v),
    }
}

/// Every persistent-format constant this crate commits to, for mechanical
/// inclusion in a storage release receipt or a Tine storage pin receipt.
///
/// Generate a receipt section from this rather than transcribing values by
/// hand: a hand-copied receipt drifts silently, and the drift is invisible
/// exactly when it matters.
pub const FORMAT_MANIFEST: &[FormatConstant] = &[
    // identity
    num(
        "OPLOG_PROTOCOL_VERSION",
        "oplog manifest/object protocol",
        FormatKind::Identity,
        OPLOG_PROTOCOL_VERSION as u64,
    ),
    num(
        "OBJECT_ENVELOPE_SCHEMA_VERSION",
        "durable object envelope",
        FormatKind::Identity,
        OBJECT_ENVELOPE_SCHEMA_VERSION as u64,
    ),
    num(
        "MANIFEST_ENCODING_VERSION",
        "durable batch manifest",
        FormatKind::Identity,
        MANIFEST_ENCODING_VERSION as u64,
    ),
    num(
        "LOCAL_JOURNAL_FRAME_SCHEMA_VERSION",
        "local journal frame",
        FormatKind::Identity,
        LOCAL_JOURNAL_FRAME_SCHEMA_VERSION as u64,
    ),
    num(
        "SCRATCH_SCHEMA_VERSION",
        "engine scratch run",
        FormatKind::Identity,
        SCRATCH_SCHEMA_VERSION as u64,
    ),
    num(
        "SCRATCH_PAGE_SCHEMA_VERSION",
        "engine scratch page",
        FormatKind::Identity,
        SCRATCH_PAGE_SCHEMA_VERSION as u64,
    ),
    num(
        "SQLITE_APPLICATION_ID",
        "SQLite projection header",
        FormatKind::Identity,
        SQLITE_APPLICATION_ID as u64,
    ),
    num(
        "SQLITE_SCHEMA_VERSION",
        "SQLite projection schema",
        FormatKind::Identity,
        SQLITE_SCHEMA_VERSION as u64,
    ),
    // layout
    name_of(
        "SCRATCH_DIR",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_DIR,
    ),
    name_of(
        "SCRATCH_MARKER_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_MARKER_FILE,
    ),
    name_of(
        "SCRATCH_LEASE_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_LEASE_FILE,
    ),
    name_of(
        "SCRATCH_PAGES_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_PAGES_FILE,
    ),
    name_of(
        "SCRATCH_BLOBS_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_BLOBS_FILE,
    ),
    num(
        "SCRATCH_LSM_LEVELS",
        "engine scratch LSM",
        FormatKind::Layout,
        SCRATCH_LSM_LEVELS as u64,
    ),
    // writer bounds
    num(
        "MAX_MANIFEST_BYTES",
        "durable batch manifest",
        FormatKind::WriterBound,
        MAX_MANIFEST_BYTES as u64,
    ),
    num(
        "MAX_OBJECT_BYTES",
        "durable object envelope",
        FormatKind::WriterBound,
        MAX_OBJECT_BYTES as u64,
    ),
    num(
        "MAX_LOCAL_JOURNAL_FRAME_BYTES",
        "local journal frame",
        FormatKind::WriterBound,
        MAX_LOCAL_JOURNAL_FRAME_BYTES as u64,
    ),
    num(
        "MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES",
        "local journal frame header",
        FormatKind::WriterBound,
        MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES as u64,
    ),
    num(
        "MAX_LOCAL_JOURNAL_SEGMENT_BYTES",
        "local journal segment",
        FormatKind::WriterBound,
        MAX_LOCAL_JOURNAL_SEGMENT_BYTES,
    ),
    num(
        "MAX_SCRATCH_PAGE_BYTES",
        "engine scratch page",
        FormatKind::WriterBound,
        MAX_SCRATCH_PAGE_BYTES as u64,
    ),
    num(
        "MAX_SCRATCH_BLOB_BYTES",
        "engine scratch blob",
        FormatKind::WriterBound,
        MAX_SCRATCH_BLOB_BYTES as u64,
    ),
    // checkpoint geometry
    num(
        "MAX_SQLITE_CHECKPOINT_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        MAX_SQLITE_CHECKPOINT_BYTES as u64,
    ),
    num(
        "SQLITE_CHECKPOINT_EDGE_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        SQLITE_CHECKPOINT_EDGE_BYTES as u64,
    ),
    num(
        "SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES as u64,
    ),
    num(
        "SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES as u64,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins every persistent-format value. A failure here is not a broken
    /// test — it means an on-disk format changed. Update the expectation only
    /// together with the migration/compatibility story for existing graphs,
    /// and record it in the storage release receipt.
    #[test]
    fn format_identity_is_pinned() {
        assert_eq!(OPLOG_PROTOCOL_VERSION, 2);
        assert_eq!(OBJECT_ENVELOPE_SCHEMA_VERSION, 2);
        assert_eq!(MANIFEST_ENCODING_VERSION, 4);
        assert_eq!(LOCAL_JOURNAL_FRAME_SCHEMA_VERSION, 1);
        assert_eq!(SCRATCH_SCHEMA_VERSION, 13);
        assert_eq!(SCRATCH_PAGE_SCHEMA_VERSION, 1);
        assert_eq!(SQLITE_APPLICATION_ID, 0x5449_4e45);
        assert_eq!(SQLITE_SCHEMA_VERSION, 12);

        assert_eq!(SCRATCH_DIR, "engine-scratch-v2");
        assert_eq!(SCRATCH_MARKER_FILE, "marker");
        assert_eq!(SCRATCH_LEASE_FILE, "lease");
        assert_eq!(SCRATCH_PAGES_FILE, "pages.index");
        assert_eq!(SCRATCH_BLOBS_FILE, "blobs.data");
        assert_eq!(SCRATCH_LSM_LEVELS, 32);

        assert_eq!(MAX_MANIFEST_BYTES, 1024 * 1024);
        assert_eq!(MAX_OBJECT_BYTES, 256 * 1024 * 1024);
        assert_eq!(MAX_LOCAL_JOURNAL_FRAME_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES, 4 * 1024);
        assert_eq!(MAX_LOCAL_JOURNAL_SEGMENT_BYTES, 4 * 1024 * 1024 * 1024);
        assert_eq!(MAX_SCRATCH_PAGE_BYTES, 256 * 1024 * 1024);
        assert_eq!(MAX_SCRATCH_BLOB_BYTES, 256 * 1024 * 1024);

        assert_eq!(MAX_SQLITE_CHECKPOINT_BYTES, 64 * 1024);
        assert_eq!(SQLITE_CHECKPOINT_EDGE_BYTES, 64 * 1024);
        assert_eq!(SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES, 16 * 1024);
        assert_eq!(SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES, 1024 * 1024);
    }

    /// The manifest must quote the live constants, not a stale copy. Every row
    /// is checked against the constant it names, so a value edited in its
    /// owning module cannot leave a divergent value in a generated receipt.
    #[test]
    fn manifest_rows_match_the_live_constants() {
        let expected: &[(&str, FormatValue)] = &[
            (
                "OPLOG_PROTOCOL_VERSION",
                FormatValue::Number(OPLOG_PROTOCOL_VERSION as u64),
            ),
            (
                "OBJECT_ENVELOPE_SCHEMA_VERSION",
                FormatValue::Number(OBJECT_ENVELOPE_SCHEMA_VERSION as u64),
            ),
            (
                "MANIFEST_ENCODING_VERSION",
                FormatValue::Number(MANIFEST_ENCODING_VERSION as u64),
            ),
            (
                "LOCAL_JOURNAL_FRAME_SCHEMA_VERSION",
                FormatValue::Number(LOCAL_JOURNAL_FRAME_SCHEMA_VERSION as u64),
            ),
            (
                "SCRATCH_SCHEMA_VERSION",
                FormatValue::Number(SCRATCH_SCHEMA_VERSION as u64),
            ),
            (
                "SCRATCH_PAGE_SCHEMA_VERSION",
                FormatValue::Number(SCRATCH_PAGE_SCHEMA_VERSION as u64),
            ),
            (
                "SQLITE_APPLICATION_ID",
                FormatValue::Number(SQLITE_APPLICATION_ID as u64),
            ),
            (
                "SQLITE_SCHEMA_VERSION",
                FormatValue::Number(SQLITE_SCHEMA_VERSION as u64),
            ),
            ("SCRATCH_DIR", FormatValue::Name(SCRATCH_DIR)),
            (
                "SCRATCH_MARKER_FILE",
                FormatValue::Name(SCRATCH_MARKER_FILE),
            ),
            ("SCRATCH_LEASE_FILE", FormatValue::Name(SCRATCH_LEASE_FILE)),
            ("SCRATCH_PAGES_FILE", FormatValue::Name(SCRATCH_PAGES_FILE)),
            ("SCRATCH_BLOBS_FILE", FormatValue::Name(SCRATCH_BLOBS_FILE)),
            (
                "SCRATCH_LSM_LEVELS",
                FormatValue::Number(SCRATCH_LSM_LEVELS as u64),
            ),
            (
                "MAX_MANIFEST_BYTES",
                FormatValue::Number(MAX_MANIFEST_BYTES as u64),
            ),
            (
                "MAX_OBJECT_BYTES",
                FormatValue::Number(MAX_OBJECT_BYTES as u64),
            ),
            (
                "MAX_LOCAL_JOURNAL_FRAME_BYTES",
                FormatValue::Number(MAX_LOCAL_JOURNAL_FRAME_BYTES as u64),
            ),
            (
                "MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES",
                FormatValue::Number(MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES as u64),
            ),
            (
                "MAX_LOCAL_JOURNAL_SEGMENT_BYTES",
                FormatValue::Number(MAX_LOCAL_JOURNAL_SEGMENT_BYTES),
            ),
            (
                "MAX_SCRATCH_PAGE_BYTES",
                FormatValue::Number(MAX_SCRATCH_PAGE_BYTES as u64),
            ),
            (
                "MAX_SCRATCH_BLOB_BYTES",
                FormatValue::Number(MAX_SCRATCH_BLOB_BYTES as u64),
            ),
            (
                "MAX_SQLITE_CHECKPOINT_BYTES",
                FormatValue::Number(MAX_SQLITE_CHECKPOINT_BYTES as u64),
            ),
            (
                "SQLITE_CHECKPOINT_EDGE_BYTES",
                FormatValue::Number(SQLITE_CHECKPOINT_EDGE_BYTES as u64),
            ),
            (
                "SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES",
                FormatValue::Number(SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES as u64),
            ),
            (
                "SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES",
                FormatValue::Number(SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES as u64),
            ),
        ];
        assert_eq!(
            FORMAT_MANIFEST.len(),
            expected.len(),
            "a persistent-format constant was added or removed without updating both the manifest and this test"
        );
        for (row, (name, value)) in FORMAT_MANIFEST.iter().zip(expected) {
            assert_eq!(&row.name, name, "manifest order changed");
            assert_eq!(
                &row.value, value,
                "manifest row for {name} does not quote the live constant"
            );
        }
    }

    #[test]
    fn manifest_names_are_unique() {
        let mut names: Vec<&str> = FORMAT_MANIFEST.iter().map(|c| c.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate name in FORMAT_MANIFEST");
    }

    /// A persistent-format constant is reachable by exactly one path:
    /// `tine_storage::formats::NAME`.
    ///
    /// This is not tidiness. A release receipt and a Tine pin receipt are
    /// generated from [`FORMAT_MANIFEST`], and their claim is "these are the
    /// format values this build commits to". A second export path lets a
    /// consumer bind to a format constant the receipt never mentions, so the
    /// receipt stops being a complete statement about the crate's format
    /// surface. Re-exporting a manifest name from `lib.rs` therefore fails
    /// here, and the fix is to import it from `formats` at the call site.
    ///
    /// Source-level rather than type-level because Rust has no way to ask
    /// "how many public paths reach this item?" — but the check is exact
    /// about what it inspects: the `pub use` items of `lib.rs`.
    #[test]
    fn no_format_constant_has_a_second_export_path() {
        const LIB_RS: &str = include_str!("lib.rs");

        let manifest_names: Vec<&str> = FORMAT_MANIFEST.iter().map(|c| c.name).collect();

        // Collect the identifiers `lib.rs` re-exports, ignoring its own
        // `pub mod formats;` declaration and any `formats::` re-export.
        let mut exported: Vec<&str> = Vec::new();
        let mut rest = LIB_RS;
        while let Some(start) = rest.find("pub use ") {
            rest = &rest[start + "pub use ".len()..];
            let end = rest.find(';').expect("a `pub use` item must be terminated");
            let item = &rest[..end];
            rest = &rest[end..];
            if item.starts_with("crate::formats") || item.starts_with("formats") {
                continue;
            }
            for token in item.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                if !token.is_empty() {
                    exported.push(token);
                }
            }
        }

        let leaked: Vec<&str> = manifest_names
            .iter()
            .copied()
            .filter(|name| exported.contains(name))
            .collect();

        assert!(
            leaked.is_empty(),
            "persistent-format constants re-exported from lib.rs as well as `formats`: {leaked:?}\n\
             Remove them from the `lib.rs` re-exports; consumers import \
             `tine_storage::formats::NAME`."
        );

        // Guard the guard: if the parse ever stops seeing `lib.rs`'s exports,
        // the emptiness above would be vacuous rather than meaningful.
        assert!(
            exported.contains(&"ContentDigest"),
            "the lib.rs re-export parse found nothing recognizable; this test would pass vacuously"
        );
    }
}
