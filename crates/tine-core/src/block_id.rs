//! The runtime identity of a block, held in 16 bytes when it can be.
//!
//! Almost every block's runtime key is a derived UUID in canonical spelling, and
//! its 36 text bytes would be paid per block for the life of the graph. Any other
//! text (a key taken from a frontend draft, a `page-property:` key, a spelling
//! with capitals or without hyphens) is kept as it came. Only text that formats
//! back byte for byte is packed, so `to_string` always returns the exact input,
//! and two ids are equal exactly when their texts are. This is an in-memory
//! form: serde reads and writes the text.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

const TEXT_LEN: usize = uuid::fmt::Hyphenated::LENGTH;

/// A block's runtime id; see the module docs. The default is the empty id.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct BlockId(Repr);

#[derive(Clone, Default, PartialEq, Eq, Hash)]
enum Repr {
    #[default]
    Empty,
    // Bytes, not u128: its alignment would grow every `DocBlock` to pad it.
    Uuid([u8; 16]),
    Text(Box<str>),
}

impl BlockId {
    /// Whether no id is assigned yet.
    pub fn is_empty(&self) -> bool {
        matches!(self.0, Repr::Empty)
    }

    /// Back to the unassigned state.
    pub fn clear(&mut self) {
        self.0 = Repr::Empty;
    }

    /// Length of the text form.
    #[allow(clippy::len_without_is_empty)] // `is_empty` is above.
    pub fn len(&self) -> usize {
        match &self.0 {
            Repr::Empty => 0,
            Repr::Uuid(_) => TEXT_LEN,
            Repr::Text(text) => text.len(),
        }
    }

    /// The packed value, when this id's text is a canonical UUID.
    pub fn as_uuid(&self) -> Option<Uuid> {
        match &self.0 {
            Repr::Uuid(bytes) => Some(Uuid::from_bytes(*bytes)),
            _ => None,
        }
    }

    /// The id's own text, when it holds one (not a packed canonical UUID).
    pub fn unpacked_text(&self) -> Option<&str> {
        match &self.0 {
            Repr::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The text form, on the stack. No allocation; a packed id formats here.
    pub fn text(&self) -> BlockIdText<'_> {
        match &self.0 {
            Repr::Empty => BlockIdText::Borrowed(""),
            Repr::Text(text) => BlockIdText::Borrowed(text),
            Repr::Uuid(bytes) => {
                let mut buffer = [0; TEXT_LEN];
                Uuid::from_bytes(*bytes)
                    .hyphenated()
                    .encode_lower(&mut buffer);
                BlockIdText::Packed(buffer)
            }
        }
    }
}

/// [`BlockId::text`]'s answer.
pub enum BlockIdText<'a> {
    /// The id's own text.
    Borrowed(&'a str),
    /// A packed id's formatted text.
    Packed([u8; TEXT_LEN]),
}

impl BlockIdText<'_> {
    /// The id's text.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(text) => text,
            Self::Packed(buffer) => std::str::from_utf8(buffer).expect("a formatted UUID is ASCII"),
        }
    }
}

impl std::ops::Deref for BlockIdText<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl From<Uuid> for BlockId {
    fn from(id: Uuid) -> Self {
        Self(Repr::Uuid(id.into_bytes()))
    }
}

fn packed(text: &str) -> Option<[u8; 16]> {
    if text.len() != TEXT_LEN {
        return None;
    }
    let id = Uuid::try_parse(text).ok()?;
    let mut buffer = [0; TEXT_LEN];
    // The round trip, not the parser's leniency, decides what may be packed.
    (id.hyphenated().encode_lower(&mut buffer).as_bytes() == text.as_bytes())
        .then(|| id.into_bytes())
}

impl From<&str> for BlockId {
    fn from(text: &str) -> Self {
        Self(if text.is_empty() {
            Repr::Empty
        } else if let Some(bytes) = packed(text) {
            Repr::Uuid(bytes)
        } else {
            Repr::Text(text.into())
        })
    }
}

impl From<String> for BlockId {
    fn from(text: String) -> Self {
        Self(if text.is_empty() {
            Repr::Empty
        } else if let Some(bytes) = packed(&text) {
            Repr::Uuid(bytes)
        } else {
            Repr::Text(text.into_boxed_str())
        })
    }
}

impl From<&String> for BlockId {
    fn from(text: &String) -> Self {
        text.as_str().into()
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::fmt::Debug for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&*self.text(), f)
    }
}

impl PartialEq<str> for BlockId {
    fn eq(&self, other: &str) -> bool {
        self.len() == other.len() && &*self.text() == other
    }
}
impl PartialEq<&str> for BlockId {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}
impl PartialEq<String> for BlockId {
    fn eq(&self, other: &String) -> bool {
        self == other.as_str()
    }
}
impl PartialEq<BlockId> for str {
    fn eq(&self, other: &BlockId) -> bool {
        other == self
    }
}
impl PartialEq<BlockId> for &str {
    fn eq(&self, other: &BlockId) -> bool {
        other == *self
    }
}
impl PartialEq<BlockId> for String {
    fn eq(&self, other: &BlockId) -> bool {
        other == self.as_str()
    }
}

impl Serialize for BlockId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text())
    }
}

impl<'de> Deserialize<'de> for BlockId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(String::deserialize(d)?.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_round_trips_byte_exact() {
        for text in [
            "",
            "x",
            "dup-1234",
            "runtime-only",
            "page-property:Page:foo",
            "aaaaaaaa-0000-0000-0000-000000000001",
            "00000000-0000-0000-0000-000000000000",
            "AAAAAAAA-0000-0000-0000-000000000001",
            "aaaaaaaa-0000-0000-0000-00000000000A",
            "aaaaaaaa000000000000000000000001",
            "{aaaaaaaa-0000-0000-0000-000000000001}",
            "urn:uuid:aaaaaaaa-0000-0000-0000-000000000001",
            "aaaaaaaa_0000_0000_0000_000000000001",
            "aaaaaaaa-00000-000-0000-000000000001",
            "aaaaaaaa-0000-0000-0000-00000000000g",
            "ééééééééééééééééé-00",
            " aaaaaaa-0000-0000-0000-000000000001",
        ] {
            let id = BlockId::from(text);
            assert_eq!(id.to_string(), text);
            assert_eq!(&*id.text(), text);
            assert_eq!(id.len(), text.len());
            assert_eq!(id.is_empty(), text.is_empty());
            assert!(id == text);
            assert_eq!(id, BlockId::from(text.to_string()));
            let wire = serde_json::to_string(&id).unwrap();
            assert_eq!(wire, serde_json::to_string(text).unwrap());
            assert_eq!(serde_json::from_str::<BlockId>(&wire).unwrap(), id);
        }
    }

    #[test]
    fn only_canonical_text_is_packed_and_distinct_spellings_stay_distinct() {
        let canonical = BlockId::from("aaaaaaaa-0000-0000-0000-000000000001");
        assert!(canonical.as_uuid().is_some());
        for other in [
            "AAAAAAAA-0000-0000-0000-000000000001",
            "aaaaaaaa000000000000000000000001",
        ] {
            let other = BlockId::from(other);
            assert_eq!(other.as_uuid(), None);
            assert_ne!(other, canonical);
        }
        assert_eq!(BlockId::from(canonical.as_uuid().unwrap()), canonical);
    }

    #[test]
    fn a_block_id_stays_three_words_with_no_heap_for_a_uuid() {
        assert!(std::mem::size_of::<BlockId>() <= 24);
    }

    #[test]
    fn authored_ids_keep_exact_bytes_through_document_and_dto_round_trips() {
        for id in [
            "aaaaaaaa-0000-4000-8000-000000000001",
            "AAAAAAAA-0000-4000-8000-000000000001",
            "aaaaaaaa000040008000000000000001",
            "urn:uuid:aaaaaaaa-0000-4000-8000-000000000001",
            "authored-key-é",
        ] {
            for org in [false, true] {
                let source = if org {
                    format!("* body\n:PROPERTIES:\n:ID: {id}\n:END:\n")
                } else {
                    format!("- body\n  id:: {id}\n")
                };
                for source in [source.clone(), source.replace('\n', "\r\n")] {
                    let mut doc = if org {
                        crate::org::parse_org(&source)
                    } else {
                        crate::doc::parse(&source)
                    };
                    doc.roots[0].uuid = BlockId::from(id);
                    let dto = crate::projection::block_to_dto(&doc.roots[0]);
                    assert_eq!(dto.id, id);
                    doc.roots[0] = crate::projection::dto_block_to_doc(&dto, org);
                    assert_eq!(doc.roots[0].property("id").as_deref(), Some(id));
                    let output = if org {
                        crate::org::serialize_org_detect(&doc, Some(&source))
                    } else {
                        crate::doc::serialize_with(
                            &doc,
                            &crate::doc::SerializeOpts::detect(Some(&source)),
                        )
                    };
                    // This canonical serializer emits LF; guarded saves restore
                    // the original per-block endings at the store boundary.
                    assert_eq!(output.as_bytes(), source.replace("\r\n", "\n").as_bytes());
                }
            }
        }
    }
}
