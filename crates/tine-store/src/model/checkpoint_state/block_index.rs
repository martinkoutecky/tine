//! Compact owner rows, emitted directly from immutable checkpoint pages.
use super::*;
use serde::ser::SerializeSeq;

struct RuntimeBytes(Vec<u8>);
impl serde::Serialize for RuntimeBytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // One write per page: serializing a Vec<[u8; 16]> would send every
        // byte separately through the postcard -> zstd streaming writer.
        s.serialize_bytes(&self.0)
    }
}
impl<'de> serde::Deserialize<'de> for RuntimeBytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = RuntimeBytes;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("whole 16-byte runtime UUIDs")
            }
            fn visit_bytes<E: serde::de::Error>(self, bytes: &[u8]) -> Result<Self::Value, E> {
                if bytes.len() % 16 != 0 {
                    return Err(E::custom("partial runtime UUID"));
                }
                Ok(RuntimeBytes(bytes.to_vec()))
            }
        }
        d.deserialize_bytes(Visitor)
    }
}

/// Runtime ids use 16 bytes; authored ids retain their exact spelling. Each
/// page slot is written once, and duplicate owners are recovered on decoding.
pub(super) enum BlockIndexState {
    Out(Arc<Pages>),
    In(SnapshotBlockIndex),
}

impl BlockIndexState {
    pub(super) fn into_index(
        self,
        pages: &Pages,
    ) -> Result<SnapshotBlockIndex, crate::store::checkpoint::Fallback> {
        let Self::In(index) = self else {
            return Err(crate::store::checkpoint::Fallback::Decode);
        };
        let valid_slots: HashSet<usize> = pages.slots().map(|(slot, _)| slot).collect();
        if index.base.values().any(|owners| {
            owners.slots().is_empty()
                || owners.slots().windows(2).any(|pair| pair[0] >= pair[1])
                || owners
                    .slots()
                    .iter()
                    .any(|slot| !valid_slots.contains(slot))
        }) {
            return Err(crate::store::checkpoint::Fallback::Decode);
        }
        Ok(index)
    }
}

impl serde::Serialize for BlockIndexState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Out(pages) => {
                let mut seq = s.serialize_seq(Some(pages.len()))?;
                for (slot, (entry, doc)) in pages.slots() {
                    let mut runtime = RuntimeBytes(Vec::new());
                    let mut authored = Vec::new();
                    tine_core::projection::visit_doc_runtime_ids(
                        &doc.roots,
                        entry.rel_path_str(),
                        |block, id| {
                            runtime.0.extend_from_slice(id.as_bytes());
                            if let Some(id) = SnapshotBlockIndex::authored_id(block) {
                                authored.push(id);
                            }
                        },
                    );
                    seq.serialize_element(&(slot, runtime, authored))?;
                }
                seq.end()
            }
            Self::In(index) => {
                // Used when tests re-encode a decoded body. Keep the same wire
                // shape, including an invalid owner so install can decline it.
                let mut groups = std::collections::BTreeMap::<usize, Vec<&str>>::new();
                for (id, owners) in index.base.iter() {
                    for slot in owners.slots() {
                        groups.entry(*slot).or_default().push(id);
                    }
                }
                let mut seq = s.serialize_seq(Some(groups.len()))?;
                for (slot, ids) in groups {
                    seq.serialize_element(&(slot, RuntimeBytes(Vec::new()), ids))?;
                }
                seq.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for BlockIndexState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let groups = Vec::<(usize, RuntimeBytes, Vec<String>)>::deserialize(d)?;
        let mut runtime_rows = Vec::new();
        let mut authored_rows = Vec::new();
        for (slot, runtime, authored) in groups {
            runtime_rows.extend(
                runtime
                    .0
                    .chunks_exact(16)
                    .map(|bytes| (<[u8; 16]>::try_from(bytes).expect("whole UUID"), slot)),
            );
            authored_rows.extend(authored.into_iter().map(|id| (id, slot)));
        }
        // Canonical UUID strings have the same ordering as their bytes.
        // Sort inline bytes before allocating strings, then merge the much
        // smaller authored-id stream in exact (case-sensitive) string order.
        runtime_rows.sort_unstable();
        authored_rows.sort_unstable();
        let mut runtime = runtime_rows
            .into_iter()
            .map(|(bytes, slot)| (uuid::Uuid::from_bytes(bytes).to_string(), slot))
            .peekable();
        let mut authored = authored_rows.into_iter().peekable();
        let rows = std::iter::from_fn(|| match (runtime.peek(), authored.peek()) {
            (Some(a), Some(b)) if a <= b => runtime.next(),
            (Some(_), Some(_)) => authored.next(),
            (Some(_), None) => runtime.next(),
            (None, _) => authored.next(),
        });
        Ok(Self::In(SnapshotBlockIndex::from_sorted_rows(rows)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_binary_runtime_id_declines_the_checkpoint() {
        let bytes = postcard::to_stdvec(&RuntimeBytes(vec![0; 15])).unwrap();
        assert!(postcard::from_bytes::<RuntimeBytes>(&bytes).is_err());
    }

    #[test]
    fn binary_and_authored_rows_merge_with_exact_spelling_and_all_owners() {
        let id = "aaaaaaaa-0000-8000-8000-000000000001";
        let runtime = uuid::Uuid::parse_str(id).unwrap();
        let bytes = postcard::to_stdvec(&vec![
            (2usize, RuntimeBytes(runtime.as_bytes().to_vec()), vec!["z"]),
            (
                1usize,
                RuntimeBytes(runtime.as_bytes().to_vec()),
                vec![id, "A", "AAAAAAAA-0000-8000-8000-000000000001"],
            ),
        ])
        .unwrap();
        let BlockIndexState::In(index) = postcard::from_bytes(&bytes).unwrap() else {
            unreachable!()
        };
        assert_eq!(index.owners(id).unwrap().slots(), &[1, 2]);
        assert_eq!(
            index.first_owner("AAAAAAAA-0000-8000-8000-000000000001"),
            Some(1)
        );
        assert_eq!(index.first_owner("A"), Some(1));
        assert_eq!(index.first_owner("z"), Some(2));
    }
}
