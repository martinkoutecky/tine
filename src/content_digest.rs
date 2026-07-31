use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

/// Generic SHA-256 identity of persisted content bytes.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ContentDigest({self})")
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, formatter)
    }
}

impl Serialize for ContentDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ContentDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse_digest(&value)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DigestParseError;

impl fmt::Display for DigestParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected exactly 64 lower-case hexadecimal characters")
    }
}

fn parse_digest(value: &str) -> Result<[u8; 32], DigestParseError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(DigestParseError);
    }
    let mut result = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        result[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(result)
}

fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => unreachable!("validated hexadecimal nibble"),
    }
}

fn write_hex(bytes: &[u8], formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        formatter.write_str(
            std::str::from_utf8(&[HEX[(byte >> 4) as usize], HEX[(byte & 0x0f) as usize]])
                .expect("hexadecimal is UTF-8"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_and_postcard_wire_bytes_are_frozen() {
        let digest = ContentDigest::from_bytes([0xab; 32]);
        let text = "abababababababababababababababababababababababababababababababab";
        assert_eq!(digest.to_string(), text);
        assert_eq!(format!("{digest:?}"), format!("ContentDigest({text})"));

        let mut expected = vec![64];
        expected.extend_from_slice(text.as_bytes());
        let encoded = postcard::to_allocvec(&digest).unwrap();
        assert_eq!(encoded, expected);
        assert_eq!(
            postcard::from_bytes::<ContentDigest>(&encoded).unwrap(),
            digest
        );
    }

    #[test]
    fn deserialization_preserves_strict_lowercase_digest_parsing() {
        let mut uppercase = vec![64];
        uppercase
            .extend_from_slice(b"ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB");
        assert!(postcard::from_bytes::<ContentDigest>(&uppercase).is_err());

        for invalid_length in [63, 65] {
            let mut bytes = vec![invalid_length as u8];
            bytes.extend(std::iter::repeat(b'a').take(invalid_length));
            assert!(postcard::from_bytes::<ContentDigest>(&bytes).is_err());
        }
    }
}
