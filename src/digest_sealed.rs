use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ContentDigest;

/// A canonical postcard envelope that binds opaque payload bytes to a schema
/// version and their content digest.
///
/// Schema interpretation, byte ceilings, filenames, and payload policy belong
/// to the caller. Decoding deliberately does not verify the digest so callers
/// can apply their schema fence before authenticating the payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestSealedPayload {
    schema_version: u32,
    payload: Vec<u8>,
    payload_digest: ContentDigest,
}

impl DigestSealedPayload {
    /// Seal opaque payload bytes under the supplied caller-owned schema value.
    pub fn new(schema_version: u32, payload: Vec<u8>) -> Self {
        let payload_digest = ContentDigest::of(&payload);
        Self {
            schema_version,
            payload,
            payload_digest,
        }
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub const fn payload_digest(&self) -> ContentDigest {
        self.payload_digest
    }

    /// Encode the frozen outer field sequence using canonical postcard bytes.
    pub fn encode_canonical(&self) -> Result<Vec<u8>, DigestSealedError> {
        postcard::to_allocvec(self).map_err(|_| DigestSealedError::Encode)
    }

    /// Decode only the canonical outer envelope.
    ///
    /// Digest verification is intentionally separate so a caller-owned schema
    /// fence can retain precedence over a forged payload digest.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, DigestSealedError> {
        let value: Self = postcard::from_bytes(bytes).map_err(|_| DigestSealedError::Decode)?;
        if value.encode_canonical()? != bytes {
            return Err(DigestSealedError::NonCanonical);
        }
        Ok(value)
    }

    pub fn verify_digest(&self) -> Result<(), DigestSealedError> {
        if ContentDigest::of(&self.payload) != self.payload_digest {
            return Err(DigestSealedError::DigestMismatch);
        }
        Ok(())
    }
}

/// Failure while handling a generic digest-sealed envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestSealedError {
    Encode,
    Decode,
    NonCanonical,
    DigestMismatch,
}

impl fmt::Display for DigestSealedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Encode => "digest-sealed payload is not encodable",
            Self::Decode => "digest-sealed payload does not decode",
            Self::NonCanonical => "digest-sealed payload is not canonical",
            Self::DigestMismatch => "digest does not cover the sealed payload bytes",
        })
    }
}

impl std::error::Error for DigestSealedError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(schema_version: u32, payload: &[u8]) -> DigestSealedPayload {
        DigestSealedPayload::new(schema_version, payload.to_vec())
    }

    #[test]
    fn canonical_outer_encoding_round_trips_exactly() {
        let original = envelope(7, &[0x10, 0x20, 0x30]);
        let bytes = original.encode_canonical().unwrap();
        let decoded = DigestSealedPayload::decode_canonical(&bytes).unwrap();

        assert_eq!(decoded, original);
        assert_eq!(decoded.encode_canonical().unwrap(), bytes);
        decoded.verify_digest().unwrap();
    }

    #[test]
    fn trailing_bytes_and_alternate_postcard_encodings_are_rejected() {
        let mut trailing = envelope(2, &[1, 2, 3]).encode_canonical().unwrap();
        trailing.push(0);
        assert!(DigestSealedPayload::decode_canonical(&trailing).is_err());

        // Schema 2's canonical leading varint is 0x02. This is its overlong
        // postcard varint form followed by the otherwise unchanged envelope.
        let canonical = envelope(2, &[1, 2, 3]).encode_canonical().unwrap();
        assert_eq!(canonical[0], 0x02);
        let mut alternate = vec![0x82, 0x00];
        alternate.extend_from_slice(&canonical[1..]);
        assert!(DigestSealedPayload::decode_canonical(&alternate).is_err());
    }

    #[test]
    fn truncated_envelopes_are_rejected() {
        let bytes = envelope(2, &[1, 2, 3]).encode_canonical().unwrap();
        for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(
                DigestSealedPayload::decode_canonical(&bytes[..cut]).is_err(),
                "a {cut}-byte prefix decoded"
            );
        }
    }

    #[test]
    fn digest_mismatch_is_an_explicit_check() {
        let mut forged = envelope(2, &[1, 2, 3]);
        forged.payload_digest = ContentDigest::from_bytes([0xff; 32]);
        let decoded =
            DigestSealedPayload::decode_canonical(&forged.encode_canonical().unwrap()).unwrap();

        assert_eq!(
            decoded.verify_digest(),
            Err(DigestSealedError::DigestMismatch)
        );
    }

    #[test]
    fn unknown_schema_remains_inspectable_before_digest_verification() {
        let mut forged = envelope(99, &[1, 2, 3]);
        forged.payload_digest = ContentDigest::from_bytes([0xff; 32]);
        let decoded =
            DigestSealedPayload::decode_canonical(&forged.encode_canonical().unwrap()).unwrap();

        assert_eq!(decoded.schema_version(), 99);
        assert_eq!(decoded.payload(), &[1, 2, 3]);
        assert_eq!(
            decoded.verify_digest(),
            Err(DigestSealedError::DigestMismatch)
        );
    }

    #[test]
    fn pre_extraction_sealed_envelope_bytes_are_frozen() {
        // postcard(schema_version, payload, ContentDigest), captured before
        // this envelope moved out of tine-core.
        let expected = b"\x02\x03\x01\x02\x03\x40\
            039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81";
        let decoded = DigestSealedPayload::decode_canonical(expected).unwrap();

        assert_eq!(decoded.schema_version(), 2);
        assert_eq!(decoded.payload(), &[1, 2, 3]);
        decoded.verify_digest().unwrap();
        assert_eq!(decoded.encode_canonical().unwrap(), expected);
    }
}
