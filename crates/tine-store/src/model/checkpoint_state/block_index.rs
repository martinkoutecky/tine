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
        if index
            .uuids
            .iter()
            .map(|(_, owners)| owners)
            .chain(index.base.values())
            .any(|owners| {
                owners.slots().is_empty()
                    || owners.slots().windows(2).any(|pair| pair[0] >= pair[1])
                    || owners
                        .slots()
                        .iter()
                        .any(|slot| !valid_slots.contains(slot))
            })
        {
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
                let mut groups = std::collections::BTreeMap::<usize, Vec<String>>::new();
                for (id, owners) in index.rows() {
                    for slot in owners.slots() {
                        groups.entry(*slot).or_default().push(id.clone());
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
        let count = groups
            .iter()
            .map(|(_, runtime, authored)| runtime.0.len() / 16 + authored.len())
            .sum();
        let mut runtime_rows = Vec::with_capacity(count);
        let mut authored_rows = Vec::new();
        for (slot, runtime, authored) in groups {
            runtime_rows.extend(runtime.0.chunks_exact(16).map(|bytes| {
                (
                    u128::from_be_bytes(bytes.try_into().expect("whole UUID")),
                    slot,
                )
            }));
            for id in authored {
                if let Some(id) = SnapshotBlockIndex::compact_uuid(&id) {
                    runtime_rows.push((id, slot));
                } else {
                    authored_rows.push((id, slot));
                }
            }
        }
        // Binary ids stay binary through sorting and the resident tree build.
        Ok(Self::In(SnapshotBlockIndex::from_rows(
            runtime_rows,
            authored_rows,
        )))
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
        assert_eq!(index.uuids.len(), 1);
        assert_eq!(index.base.len(), 3);
        assert_eq!(index.first_owner("aaaaaaaa000080008000000000000001"), None);
        assert_eq!(
            index.first_owner("urn:uuid:aaaaaaaa-0000-8000-8000-000000000001"),
            None
        );
    }

    #[test]
    fn uuid_spellings_and_overlay_tombstones_remain_distinct() {
        let lower = "aaaaaaaa-0000-4000-8000-000000000001";
        let upper = "AAAAAAAA-0000-4000-8000-000000000001";
        let simple = "aaaaaaaa000040008000000000000001";
        let urn = "urn:uuid:aaaaaaaa-0000-4000-8000-000000000001";
        let mut index = SnapshotBlockIndex::from_rows(
            vec![(SnapshotBlockIndex::compact_uuid(lower).unwrap(), 2)],
            vec![(upper, 1), (simple, 3), (urn, 4), ("custom", 5)],
        );
        let held = index.clone();
        index.edit(lower, |owners| owners.insert(0, 0));
        assert_eq!(index.owners(lower).unwrap().slots(), &[0, 2]);
        assert_eq!(held.owners(lower).unwrap().slots(), &[2]);
        index.edit(lower, Vec::clear);
        index.edit("custom", Vec::clear);
        let rows: std::collections::BTreeMap<_, _> = index.rows().collect();
        assert!(!rows.contains_key(lower));
        assert!(!rows.contains_key("custom"));
        for (id, slot) in [(upper, 1), (simple, 3), (urn, 4)] {
            assert_eq!(index.first_owner(id), Some(slot));
            assert_eq!(rows[id].slots(), &[slot]);
        }
    }
}
