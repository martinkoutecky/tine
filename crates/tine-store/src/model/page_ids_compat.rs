//! The two calls whose signature phase A of `RUNTIME-ID-MODEL.md` changed,
//! for `page_ids_tests.rs`: the fail-before run on 16cee4693 swaps this file
//! for its baseline twin (`publish_owned(.., Option<&Document>)`,
//! `respelled(key, from)`), so the same test bodies run on both.

use super::*;

/// The owner's publication of `bytes` for held `key`, with the serializer's
/// keyed parse of exactly these bytes when it has one (an own save).
pub(super) fn publish(graph: &Graph, key: &str, bytes: &str, doc: Option<Document>) {
    let bytes: Arc<[u8]> = Arc::from(bytes.as_bytes());
    let saved = doc.map(|doc| page_ids::SaveHandoff {
        bytes: Arc::clone(&bytes),
        doc: Arc::new(doc),
    });
    assert!(graph.publish_owned(key, Some(bytes), saved).is_ok());
}

/// The host's confirmed same-file spelling move of `key` from `from`.
pub(super) fn respelled(graph: &Graph, key: &str, from: &Path) {
    graph.respelled(key, from, true);
}
