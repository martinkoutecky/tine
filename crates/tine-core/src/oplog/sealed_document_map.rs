//! Accepted document index: ONE shared sorted-table domain keyed by the
//! lossless [`DocumentKey`] bytes. Entity births and full (block, page)
//! membership pairs are rows of the same domain, distinguished by the key's
//! own address tag. There is no tuple hashing, no group descriptor, no nested
//! tree and no second document index.
use crate::oplog::cold_object_store::{
    framed_document_key, unframed_document_key, ColdLocatorV1, DOMAIN_DOCUMENT_ROSTER,
};
use crate::oplog::DocumentKey;
use std::collections::BTreeSet;

use super::{SealedGenerationStagingStore, SealedTableAccess};

/// The roster as a live row count over one sealed root.
///
/// The old authenticated root is gone with the treap (D-1): a sorted-table
/// domain has no composed digest, and the generation's identity is the sealed
/// root digest the marker already binds.
///
/// The census is the number of LIVE rows in [`DOMAIN_DOCUMENT_ROSTER`], and
/// the store -- not the caller's arithmetic -- says whether a key is already
/// one of them. That is why a resumed roster is seeded with
/// [`SealedDocumentMap::over_base`] rather than a payload's remembered count:
/// under sorted tables a new cut opens over the PREDECESSOR's tables, so the
/// rows are already there, and a census that assumed an empty substrate
/// double-counted every document it rewrote ("checkpoint image roster has
/// extra or missing documents" across the runtime checkpoint suites).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SealedDocumentMap {
    documents: u64,
}

impl SealedDocumentMap {
    pub(super) fn empty() -> Self {
        Self { documents: 0 }
    }

    pub(super) fn with_count(documents: u64) -> Self {
        Self { documents }
    }

    /// Seed a census from the rows the store already holds.
    ///
    /// A cut that continues a published generation inherits its roster rows,
    /// so its census starts at their count; a base-zero cut clears the domain
    /// first (`clear`) and starts at zero.
    pub(super) fn over_base<Store: SealedTableAccess>(store: &Store) -> Result<Self, String> {
        Ok(Self {
            documents: store.table_entries(DOMAIN_DOCUMENT_ROSTER)?.len() as u64,
        })
    }

    /// Drop every inherited roster row. A base-zero republication does not
    /// extend the predecessor's roster; it replaces it, and a stale row left
    /// behind is an "extra document" `qualify_complete_keys` would refuse.
    pub(super) fn clear(store: &mut SealedGenerationStagingStore) -> Result<Self, String> {
        let live: Vec<Vec<u8>> = store
            .table_entries(DOMAIN_DOCUMENT_ROSTER)?
            .into_keys()
            .collect();
        for framed in live {
            store.remove(DOMAIN_DOCUMENT_ROSTER, &framed)?;
        }
        Ok(Self { documents: 0 })
    }

    pub(super) fn count(self) -> u64 {
        self.documents
    }

    pub(super) fn value<Store: SealedTableAccess>(
        self,
        store: &Store,
        key: DocumentKey,
    ) -> Result<Option<ColdLocatorV1>, String> {
        let framed = framed_document_key(key.authenticated_map_key().as_slice())?;
        store
            .table_value(DOMAIN_DOCUMENT_ROSTER, &framed)?
            .map(|value| ColdLocatorV1::from_value(&value))
            .transpose()
    }

    pub(super) fn upsert(
        self,
        store: &mut SealedGenerationStagingStore,
        key: DocumentKey,
        value: ColdLocatorV1,
    ) -> Result<Self, String> {
        let framed = framed_document_key(key.authenticated_map_key().as_slice())?;
        let present = store
            .table_value(DOMAIN_DOCUMENT_ROSTER, &framed)?
            .is_some();
        store.put(DOMAIN_DOCUMENT_ROSTER, &framed, &value.to_bytes())?;
        Ok(Self {
            documents: if present {
                self.documents
            } else {
                self.documents
                    .checked_add(1)
                    .ok_or("document roster count overflowed")?
            },
        })
    }

    pub(super) fn remove(
        self,
        store: &mut SealedGenerationStagingStore,
        key: DocumentKey,
    ) -> Result<Self, String> {
        let framed = framed_document_key(key.authenticated_map_key().as_slice())?;
        let present = store
            .table_value(DOMAIN_DOCUMENT_ROSTER, &framed)?
            .is_some();
        if !present {
            return Ok(self);
        }
        store.remove(DOMAIN_DOCUMENT_ROSTER, &framed)?;
        Ok(Self {
            documents: self.documents.saturating_sub(1),
        })
    }

    /// Prove that this census is EXACTLY the accepted document key set: every
    /// named key resolves, and no other key is live in the domain.
    pub(super) fn qualify_complete_keys<Store: SealedTableAccess>(
        self,
        store: &Store,
        keys: impl Iterator<Item = DocumentKey>,
    ) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for key in keys {
            if !seen.insert(key) {
                return Err("document key census repeats an identity".into());
            }
            self.value(store, key)?
                .ok_or("document roster omits an accepted document")?;
        }
        let live = store.table_entries(DOMAIN_DOCUMENT_ROSTER)?;
        if live.len() != seen.len() {
            return Err(format!(
                "document roster is not exactly the accepted document key set: live={} accepted={}",
                live.len(),
                seen.len()
            ));
        }
        for framed in live.keys() {
            let key = DocumentKey::from_authenticated_map_key(
                tine_storage::sealed_accepted_index::AuthenticatedMapKey::new(
                    &unframed_document_key(framed)?,
                )
                .map_err(|error| error.to_string())?,
            )
            .ok_or("document roster holds a key that is not a document address")?;
            if !seen.contains(&key) {
                return Err("document roster is not exactly the accepted document key set".into());
            }
        }
        Ok(())
    }
}
