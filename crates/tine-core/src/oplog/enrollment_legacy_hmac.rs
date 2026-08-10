//! Frozen verification support for the original enrollment checkpoint format.
//!
//! New enrollment authority and checkpoint records are keyless and use the
//! versioned CRC integrity codec.  This module exists solely so existing v1
//! authority / v5 record histories remain readable without keeping keyed
//! signing on the production enrollment path.

use ring::hmac;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::enrollment::{EnrollmentBindingV1, PreparationId};
use super::ContentDigest;

pub(crate) const LEGACY_CHECKPOINT_DOMAIN: &str = "tine/enrollment-checkpoint/v2";
pub(crate) const LEGACY_AUTHORITY_KEY_BYTES: usize = 32;

/// Frozen on-disk v1 authority claim. It lives beside the legacy verifier:
/// current authority is keyless and this type only reopens existing v1/v5
/// histories.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacyAuthorityClaimV1 {
    pub(crate) schema_version: u32,
    pub(crate) authority_id: Uuid,
    pub(crate) lease_resource_id: ContentDigest,
    pub(crate) binding: EnrollmentBindingV1,
    pub(crate) initial_preparation_id: PreparationId,
    pub(crate) initial_source_inventory_digest: ContentDigest,
    pub(crate) key: [u8; LEGACY_AUTHORITY_KEY_BYTES],
}

impl LegacyAuthorityClaimV1 {
    pub(crate) fn legacy_key(&self) -> &[u8; LEGACY_AUTHORITY_KEY_BYTES] {
        &self.key
    }
}

pub(crate) fn verify(key: &[u8; 32], message: &[u8], tag: ContentDigest) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::verify(&key, message, tag.as_bytes()).is_ok()
}

#[cfg(test)]
impl LegacyAuthorityClaimV1 {
    pub(crate) fn new_for_test(
        schema_version: u32,
        authority_id: Uuid,
        lease_resource_id: ContentDigest,
        binding: EnrollmentBindingV1,
        initial_preparation_id: PreparationId,
        initial_source_inventory_digest: ContentDigest,
        key_byte: u8,
    ) -> Self {
        Self {
            schema_version,
            authority_id,
            lease_resource_id,
            binding,
            initial_preparation_id,
            initial_source_inventory_digest,
            key: [key_byte; LEGACY_AUTHORITY_KEY_BYTES],
        }
    }
}

/// Test fixtures deliberately retain the historical signer so byte-exact v1/v5
/// histories can be created and reopened.  Production has no HMAC signing
/// surface.
#[cfg(test)]
pub(crate) fn sign_for_test(key: &[u8; 32], message: &[u8]) -> ContentDigest {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    ContentDigest::from_bytes(
        hmac::sign(&key, message)
            .as_ref()
            .try_into()
            .expect("SHA-256 HMAC tag"),
    )
}
