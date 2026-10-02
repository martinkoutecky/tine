//! The query memo in launch-checkpoint form (storage spec §7.6, ADR 0070):
//! the memo of the captured generation, written whole and loaded back as that
//! generation's memo. The launch diff then carries it exactly as any
//! publication carries a memo (`ReadSnapshot::carry_memos_from`); nothing
//! here decides validity.
//!
//! Result DTOs keep their JSON shape (internally tagged and flattened serde
//! forms that a positional format cannot read), so they travel as JSON text
//! inside the checkpoint; a plan's compiled patterns are rebuilt from its
//! filter. Maps are written in key order, so one memo encodes to one byte
//! string.

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use tine_core::query::ir::{Anchor, Filter, RegistrySnapshot};
use tine_core::query::registry::Registry;

use super::*;

/// `value` as JSON text (DTOs whose serde form is JSON-only).
pub(crate) fn to_json<T: Serialize, E: serde::ser::Error>(value: &T) -> Result<String, E> {
    serde_json::to_string(value).map_err(E::custom)
}

/// The inverse of [`to_json`].
pub(crate) fn from_json<T: DeserializeOwned, E: serde::de::Error>(text: &str) -> Result<T, E> {
    serde_json::from_str(text).map_err(E::custom)
}

/// A registry with the config it was built under.
#[derive(Serialize, Deserialize)]
pub(crate) struct RegistryParts(String, ParseConfig);

impl RegistryParts {
    pub(crate) fn of<E: serde::ser::Error>(registry: &Registry) -> Result<Self, E> {
        Ok(RegistryParts(
            to_json(&registry.snapshot())?,
            registry.config().clone(),
        ))
    }

    pub(crate) fn into_registry<E: serde::de::Error>(self) -> Result<Arc<Registry>, E> {
        let snapshot: RegistrySnapshot = from_json(&self.0)?;
        Ok(Arc::new(Registry::from_snapshot_under(&snapshot, &self.1)))
    }
}

#[derive(Serialize, Deserialize)]
struct PlanParts {
    anchor: Anchor,
    filter: String,
    track: bool,
    today: i64,
    remove_accents: bool,
    registry: Option<RegistryParts>,
}

#[derive(Serialize, Deserialize)]
enum AnswerParts {
    /// `None`: the statistics resource refusal.
    Result(Option<String>),
    Groups(String, usize, bool),
    Advanced(String, usize, bool),
}

#[derive(Serialize, Deserialize)]
struct EntryParts {
    key: String,
    plan: Option<PlanParts>,
    answer: AnswerParts,
    pages: Vec<String>,
    bytes: usize,
}

#[derive(Serialize, Deserialize)]
struct MemoParts {
    today: i64,
    parse_config: ParseConfig,
    entries: Vec<EntryParts>,
    lru: Vec<String>,
    bytes: usize,
}

/// One generation's query memo, as captured (clones of `Arc`s; the encoding
/// work happens when it is serialized, off the store writer).
pub(crate) struct MemoState(Memo);

impl QueryMemo {
    /// This memo as it stands, or `None` when nothing was ever memoized.
    pub(crate) fn checkpoint_capture(&self) -> Option<MemoState> {
        self.inner.read().unwrap().clone().map(MemoState)
    }

    /// Entries and retained bytes, for the checkpoint cadence
    /// (`model::LazyMarks`): a change means the memo holds something the
    /// last checkpoint may lack.
    pub(crate) fn checkpoint_marks(&self) -> (usize, usize) {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .map_or((0, 0), |memo| (memo.entries.len(), memo.bytes))
    }

    /// The memo a loaded generation starts with.
    pub(crate) fn from_checkpoint(state: Option<MemoState>) -> QueryMemo {
        QueryMemo {
            inner: RwLock::new(state.map(|MemoState(memo)| memo)),
        }
    }
}

fn answer_parts<E: serde::ser::Error>(answer: &Answer) -> Result<AnswerParts, E> {
    Ok(match answer {
        Answer::Result(Ok(result)) => AnswerParts::Result(Some(to_json(result.as_ref())?)),
        Answer::Result(Err(StatisticsResourceLimit)) => AnswerParts::Result(None),
        Answer::Groups(groups) => AnswerParts::Groups(
            to_json(groups.groups.as_ref())?,
            groups.total,
            groups.exceeded,
        ),
        Answer::Advanced {
            result,
            total,
            exceeded,
        } => AnswerParts::Advanced(to_json(result.as_ref())?, *total, *exceeded),
    })
}

fn answer_from<E: serde::de::Error>(parts: AnswerParts) -> Result<Answer, E> {
    Ok(match parts {
        AnswerParts::Result(Some(json)) => Answer::Result(Ok(Arc::new(from_json(&json)?))),
        AnswerParts::Result(None) => Answer::Result(Err(StatisticsResourceLimit)),
        AnswerParts::Groups(json, total, exceeded) => Answer::Groups(BoundedRefGroups {
            groups: Arc::new(from_json(&json)?),
            total,
            exceeded,
        }),
        AnswerParts::Advanced(json, total, exceeded) => Answer::Advanced {
            result: Arc::new(from_json(&json)?),
            total,
            exceeded,
        },
    })
}

impl Serialize for MemoState {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let memo = &self.0;
        let mut keys: Vec<&String> = memo.entries.keys().collect();
        keys.sort();
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            let entry = &memo.entries[key];
            let plan = match &entry.plan {
                Some(plan) => {
                    let (anchor, filter, track, today, remove_accents, registry) =
                        plan.checkpoint_parts();
                    Some(PlanParts {
                        anchor,
                        filter: to_json(filter)?,
                        track,
                        today: today.ordinal_key(),
                        remove_accents,
                        registry: registry
                            .map(|registry| RegistryParts::of(registry))
                            .transpose()?,
                    })
                }
                None => None,
            };
            let mut pages: Vec<String> = entry.pages.iter().cloned().collect();
            pages.sort();
            entries.push(EntryParts {
                key: key.clone(),
                plan,
                answer: answer_parts(&entry.answer)?,
                pages,
                bytes: entry.bytes,
            });
        }
        MemoParts {
            today: memo.today,
            parse_config: memo.parse_config.clone(),
            entries,
            lru: memo.lru.iter().cloned().collect(),
            bytes: memo.bytes,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for MemoState {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let parts = MemoParts::deserialize(d)?;
        let mut entries = HashMap::with_capacity(parts.entries.len());
        for entry in parts.entries {
            let plan = match entry.plan {
                Some(plan) => {
                    let filter: Filter = from_json(&plan.filter)?;
                    let registry = plan
                        .registry
                        .map(RegistryParts::into_registry)
                        .transpose()?;
                    Some(Arc::new(Plan::from_checkpoint_parts((
                        plan.anchor,
                        filter,
                        plan.track,
                        JournalDate::from_ordinal(plan.today),
                        plan.remove_accents,
                        registry,
                    ))))
                }
                None => None,
            };
            entries.insert(
                entry.key,
                Entry {
                    plan,
                    answer: answer_from(entry.answer)?,
                    pages: Arc::new(entry.pages.into_iter().collect()),
                    bytes: entry.bytes,
                },
            );
        }
        if parts.lru.iter().any(|key| !entries.contains_key(key)) {
            return Err(D::Error::custom("query memo order names a missing answer"));
        }
        Ok(MemoState(Memo {
            today: parts.today,
            parse_config: parts.parse_config,
            entries,
            lru: parts.lru.into(),
            bytes: parts.bytes,
        }))
    }
}

#[cfg(test)]
impl MemoState {
    /// The same memo filed under `day` (golden images must not depend on the
    /// day the test runs).
    pub(crate) fn at_day(mut self, day: i64) -> Self {
        self.0.today = day;
        for entry in self.0.entries.values_mut() {
            if let Some(plan) = &entry.plan {
                let (anchor, filter, track, _, remove_accents, registry) = plan.checkpoint_parts();
                entry.plan = Some(Arc::new(Plan::from_checkpoint_parts((
                    anchor,
                    filter.clone(),
                    track,
                    JournalDate::from_ordinal(day),
                    remove_accents,
                    registry.cloned(),
                ))));
            }
        }
        self
    }
}
