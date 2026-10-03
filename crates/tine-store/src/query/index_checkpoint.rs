//! The query index in launch-checkpoint form (storage spec §7.6, ADR 0070):
//! a generation's index slot written as it stands (built, seeded, or empty)
//! and loaded back as that generation's slot. The first query after a launch
//! diff then patches or reuses it exactly as it would in memory
//! (`QueryIndexSlot::succeeding`); nothing here decides validity.
//!
//! The facts map and the two inverted tables are written flat (base with the
//! delta applied) and in key order, so one index encodes to one byte string
//! and reads back with the same answers; registries travel as their wire
//! snapshot plus parse config. A loaded index takes the snapshot's own
//! path -> slot map at install.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::*;
use crate::query::memo::checkpoint::RegistryParts;

/// One page's facts, field for field.
#[derive(Serialize, Deserialize)]
struct FactsParts(
    Vec<(String, String)>,
    Vec<String>,
    Vec<(String, u64)>,
    Option<String>,
    Vec<String>,
    /// The header page-property block as raw text, format flag and its
    /// page-scoped identity; the projection is rebuilt on first use.
    Option<(String, bool, String)>,
);

#[derive(Serialize, Deserialize)]
struct IndexParts {
    facts: Vec<(String, Option<FactsParts>)>,
    key_pages: Vec<(String, Vec<String>)>,
    declarers: Vec<(String, Vec<String>)>,
    parse_config: ParseConfig,
    generation: u64,
    registry: Option<RegistryParts>,
    /// The inherited registry, the keys and the declared page keys to patch.
    pending: Option<(RegistryParts, Vec<String>, Vec<String>)>,
}

/// One built index (an `Arc` clone at capture; encoded when serialized).
pub(crate) struct IndexState(Arc<QueryIndex>);

/// A generation's index slot as captured.
#[derive(Serialize, Deserialize)]
pub(crate) struct SlotState {
    built: Option<IndexState>,
    seed: Option<(IndexState, Vec<String>)>,
}

impl QueryIndexSlot {
    /// This slot as it stands.
    pub(crate) fn checkpoint_capture(&self) -> SlotState {
        SlotState {
            built: self.built.get().cloned().map(IndexState),
            seed: self
                .seed
                .lock()
                .unwrap()
                .as_ref()
                .map(|(base, changed)| (IndexState(Arc::clone(base)), changed.clone())),
        }
    }

    /// The slot a loaded generation starts with; `positions` is that
    /// generation's path -> slot map.
    pub(crate) fn from_checkpoint(
        state: SlotState,
        positions: &Arc<SharedMap<String, usize>>,
    ) -> QueryIndexSlot {
        let place = |IndexState(index): IndexState| {
            let mut index = Arc::try_unwrap(index)
                .unwrap_or_else(|_| unreachable!("a loaded index has one owner"));
            index.positions = Arc::clone(positions);
            Arc::new(index)
        };
        let built = OnceLock::new();
        if let Some(index) = state.built {
            let _ = built.set(place(index));
        }
        QueryIndexSlot {
            built,
            seed: Mutex::new(state.seed.map(|(base, changed)| (place(base), changed))),
        }
    }

    /// Whether this generation's index is built (the checkpoint cadence's
    /// `model::LazyMarks`; the seed arrives with a publication, not a read).
    pub(crate) fn is_built(&self) -> bool {
        self.built.get().is_some()
    }
}

#[cfg(test)]
impl SlotState {
    pub(crate) fn is_built(&self) -> bool {
        self.built.is_some()
    }
}

/// `(key, value)` of a base map with its delta applied, in key order.
fn flat<V: Clone>(
    base: &HashMap<String, V>,
    delta: &HashMap<String, Option<V>>,
) -> Vec<(String, Option<V>)> {
    let mut all: BTreeMap<&String, Option<V>> = base
        .iter()
        .map(|(key, value)| (key, Some(value.clone())))
        .collect();
    for (key, value) in delta {
        all.insert(key, value.clone());
    }
    all.into_iter()
        .map(|(key, value)| (key.clone(), value))
        .collect()
}

fn postings_parts(postings: &Postings) -> Vec<(String, Vec<String>)> {
    let delta: HashMap<String, Option<Arc<BTreeSet<String>>>> = postings
        .delta
        .iter()
        .map(|(key, set)| (key.clone(), Some(Arc::clone(set))))
        .collect();
    flat(&postings.base, &delta)
        .into_iter()
        .filter_map(|(key, set)| set.map(|set| (key, set.iter().cloned().collect())))
        .collect()
}

fn postings_from(parts: Vec<(String, Vec<String>)>) -> Postings {
    Postings {
        base: Arc::new(
            parts
                .into_iter()
                .map(|(key, paths)| (key, Arc::new(paths.into_iter().collect())))
                .collect(),
        ),
        delta: HashMap::new(),
    }
}

impl Serialize for IndexState {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let index = &self.0;
        let facts = flat(&index.facts.base, &index.facts.delta)
            .into_iter()
            .map(|(path, facts)| {
                let facts = facts.map(|f| {
                    FactsParts(
                        f.properties.to_vec(),
                        f.tags.to_vec(),
                        f.keys.to_vec(),
                        f.declares.clone(),
                        f.refs.to_vec(),
                        f.page_property_block.as_ref().map(|block| {
                            (block.raw().to_owned(), block.is_org(), block.uuid.clone())
                        }),
                    )
                });
                (path, facts)
            })
            .collect();
        let pending = match &index.pending {
            Some(pending) => Some((
                RegistryParts::of(&pending.base)?,
                pending.keys.iter().cloned().collect(),
                pending.declared.iter().cloned().collect(),
            )),
            None => None,
        };
        IndexParts {
            facts,
            key_pages: postings_parts(&index.key_pages),
            declarers: postings_parts(&index.declarers),
            parse_config: index.parse_config.clone(),
            generation: index.generation,
            registry: index
                .registry
                .get()
                .map(|r| RegistryParts::of(r))
                .transpose()?,
            pending,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for IndexState {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let parts = IndexParts::deserialize(d)?;
        let facts = parts
            .facts
            .into_iter()
            .filter_map(|(path, facts)| {
                let FactsParts(properties, tags, keys, declares, refs, page_property_block) =
                    facts?;
                let page_property_block = page_property_block.map(|(raw, is_org, uuid)| {
                    let mut block = DocBlock::new(raw);
                    block.set_org(is_org);
                    block.uuid = uuid;
                    block
                });
                Some((
                    path,
                    Arc::new(PageFacts {
                        properties: properties.into_boxed_slice(),
                        tags: tags.into_boxed_slice(),
                        keys: keys.into_boxed_slice(),
                        declares,
                        refs: refs.into_boxed_slice(),
                        page_property_block,
                    }),
                ))
            })
            .collect();
        let registry = OnceLock::new();
        if let Some(parts) = parts.registry {
            let _ = registry.set(parts.into_registry()?);
        }
        let pending = match parts.pending {
            Some((base, keys, declared)) => Some(Pending {
                base: base.into_registry()?,
                keys: keys.into_iter().collect(),
                declared: declared.into_iter().collect(),
            }),
            None => None,
        };
        Ok(IndexState(Arc::new(QueryIndex {
            facts: FactsMap {
                base: Arc::new(facts),
                delta: HashMap::new(),
            },
            key_pages: postings_from(parts.key_pages),
            declarers: postings_from(parts.declarers),
            parse_config: parts.parse_config,
            generation: parts.generation,
            positions: Arc::new(SharedMap::new()),
            registry,
            pending,
        })))
    }
}
