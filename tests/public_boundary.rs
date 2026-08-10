//! What an external consumer can actually reach.
//!
//! Every other test in this crate is a unit test compiled *inside* it, so it
//! sees private items and whatever features the surrounding build enabled.
//! That is precisely the wrong vantage point for the question a package split
//! turns into a contract: is the published API self-sufficient from outside,
//! without `test-support`?
//!
//! An integration test is compiled as a separate crate against the built
//! library, so it can only use `pub` paths — which makes this file's *compiling*
//! the assertion. `cargo test -p tine-storage` builds it with default features,
//! so a production path that secretly needs a test seam fails here.
//!
//! This is the fixture `tine-core` would become after extraction, in miniature:
//! when the crate moves out of tree, its consumers see exactly this much.

use serde::{Deserialize, Serialize};
use tine_storage::formats::{self, FormatKind, FormatValue};
use tine_storage::{
    ContentDigest, DigestSealedError, DigestSealedPayload, DurableDirectoryPublication,
    LocalJournalAppendError, LocalJournalError, LocalJournalSegmentV2,
    LocalJournalSegmentV2Selection,
};
use uuid::Uuid;

/// A durable payload survives a canonical encode/decode round trip, using only
/// public paths. Not a redundant unit test: the unit suite proves the codec,
/// this proves the codec is *usable* by someone who is not this crate.
#[test]
fn a_sealed_payload_round_trips_through_the_public_api() {
    let payload = DigestSealedPayload::new(7, b"external consumer".to_vec());
    let digest = payload.payload_digest();

    let encoded = payload.encode_canonical().expect("canonical encode");
    let decoded = DigestSealedPayload::decode_canonical(&encoded).expect("canonical decode");

    assert_eq!(decoded.schema_version(), 7);
    assert_eq!(decoded.payload(), b"external consumer");
    assert_eq!(decoded.payload_digest(), digest);
    decoded.verify_digest().expect("digest verifies");
}

/// Corruption must be reportable to a consumer, not just detectable inside the
/// crate: `DigestSealedError` has to be public for the `Result` to be usable.
#[test]
fn a_corrupt_payload_reports_a_public_error() {
    let payload = DigestSealedPayload::new(1, b"tamper".to_vec());
    let mut encoded = payload.encode_canonical().expect("canonical encode");
    let last = encoded.len() - 1;
    encoded[last] ^= 0xff;

    let outcome: Result<DigestSealedPayload, DigestSealedError> =
        DigestSealedPayload::decode_canonical(&encoded);
    assert!(outcome.is_err(), "a tampered payload decoded as valid");
}

/// A content digest is constructible and inspectable from outside.
#[test]
fn content_digests_are_usable_from_outside_the_crate() {
    let digest = ContentDigest::of(b"bytes");
    assert_eq!(digest.as_bytes().len(), 32);
    assert_eq!(digest, ContentDigest::of(b"bytes"));
    assert_ne!(digest, ContentDigest::of(b"other bytes"));
}

/// The whole point of `formats`: a release or pin receipt is *generated* from
/// the manifest by someone outside this crate. If the manifest's row type is
/// not fully public, that consumer has to hand-transcribe values instead —
/// which is the failure mode the module exists to prevent.
#[test]
fn a_receipt_can_be_generated_from_the_public_manifest() {
    assert!(
        !formats::FORMAT_MANIFEST.is_empty(),
        "the manifest is empty; a generated receipt would claim nothing"
    );

    let mut lines = Vec::new();
    for row in formats::FORMAT_MANIFEST {
        let kind = match row.kind {
            FormatKind::Identity => "identity",
            FormatKind::Layout => "layout",
            FormatKind::WriterBound => "writer-bound",
            FormatKind::CheckpointGeometry => "checkpoint-geometry",
        };
        let value = match row.value {
            FormatValue::Number(number) => number.to_string(),
            FormatValue::Name(name) => name.to_string(),
        };
        lines.push(format!(
            "{} {} {} = {}",
            row.artifact, kind, row.name, value
        ));
    }

    assert_eq!(lines.len(), formats::FORMAT_MANIFEST.len());
    assert!(
        lines
            .iter()
            .any(|line| line.contains("SQLITE_SCHEMA_VERSION")),
        "the generated receipt is missing a known format constant"
    );
}

/// Format constants are reachable at `formats::NAME`. The negative half — that
/// they are reachable *only* there — is enforced by
/// `formats::tests::no_format_constant_has_a_second_export_path`, because a
/// nonexistent path cannot be named in code that has to compile.
#[test]
fn format_constants_are_reachable_through_formats() {
    assert_eq!(formats::SCRATCH_DIR, "engine-scratch-v2");
    assert!(formats::MAX_OBJECT_BYTES > 0);
    assert!(formats::SQLITE_SCHEMA_VERSION > 0);
    assert_eq!(formats::LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION, 2);
    assert_eq!(formats::LOCAL_JOURNAL_SEGMENT_HEADER_BYTES, 136);
    assert_eq!(formats::LOCAL_JOURNAL_FRONTIER_BYTES, 240);
}

#[test]
fn journal_v2_selection_and_append_certainty_are_publicly_typed() {
    let selection = LocalJournalSegmentV2Selection::new(
        "device.journal-v2",
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        17,
    )
    .unwrap();
    assert_eq!(selection.segment_name(), "device.journal-v2");
    assert_eq!(selection.segment_id(), Uuid::from_u128(1));
    assert_eq!(selection.device_id(), Uuid::from_u128(2));
    assert_eq!(selection.base_sequence(), 17);
    assert_eq!(
        selection.segment_name_digest(),
        ContentDigest::of(b"device.journal-v2")
    );

    let failure =
        LocalJournalAppendError::DefinitelyNotAppended(LocalJournalError::SequenceExhausted);
    assert!(!failure.outcome_is_unknown());
    assert_eq!(failure.cause(), &LocalJournalError::SequenceExhausted);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum ExternalJournalKind {
    Effect,
}

#[test]
#[cfg(any(unix, windows))]
fn journal_v2_can_be_prepared_opened_and_appended_from_the_public_api() {
    let root = std::env::temp_dir().join(format!("tine-storage-public-v2-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let dir = cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
    let selection = LocalJournalSegmentV2Selection::new(
        "external.journal-v2",
        Uuid::from_u128(3),
        Uuid::from_u128(4),
        9,
    )
    .unwrap();
    LocalJournalSegmentV2::<ExternalJournalKind>::prepare(&dir, &selection).unwrap();
    let (mut segment, recovery) = LocalJournalSegmentV2::open_selected(&dir, &selection).unwrap();
    assert_eq!(recovery.frames_recovered, 0);
    let appended = segment
        .append(ExternalJournalKind::Effect, b"public")
        .unwrap();
    assert_eq!(appended.sequence, 9);
    assert_eq!(appended.data_durability_syncs, 2);
    drop(segment);
    drop(dir);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(any(unix, windows))]
fn durable_publication_exposes_create_replace_and_retire_to_a_consumer() {
    let root = std::env::temp_dir().join(format!("tine-storage-public-durable-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let dir = cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
    let publication = DurableDirectoryPublication::open(&dir).unwrap();
    publication
        .publish_new_exact("schema-2-anchor", b"old")
        .unwrap();
    publication
        .replace_exact("schema-2-anchor", b"old", b"new")
        .unwrap();
    publication
        .retire_exact("schema-2-anchor", ".retired-schema-2-anchor", b"new")
        .unwrap();
    assert!(!root.join("schema-2-anchor").exists());
    assert_eq!(
        std::fs::read(root.join(".retired-schema-2-anchor")).unwrap(),
        b"new"
    );
    drop(publication);
    drop(dir);
    std::fs::remove_dir_all(root).unwrap();
}

/// The recorded surface is itself public, so a consumer or a release process
/// can enumerate what it is pinning without parsing this crate's source.
#[test]
fn the_api_surface_is_enumerable_by_a_consumer() {
    let names = tine_storage::api_surface::exported_names();
    assert!(names.len() > 100, "the published surface looks truncated");
    assert!(
        names.iter().filter(|name| name.test_support_only).count() > 0,
        "no test-support seams recorded; a consumer cannot tell them from production API"
    );
}
