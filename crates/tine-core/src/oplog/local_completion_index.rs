use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::ErrorKind;
use std::time::{Duration, Instant};

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::absence_decision::{
    frontier_strictly_dominates, AbsenceCompletionAnchor, AbsenceDecisionKey,
};
use super::object_store::{
    ensure_reconstructible_directory_nofollow, open_dir_nofollow, read_optional_regular,
    require_regular_entry, sync_dir_required,
};
use super::{
    ContentDigest, FrontierV2, ManagedPath, ObjectStore, PageId, ProjectionEndpointId,
    ProjectionIntent, ProjectionIntentId, ProjectionTargetKind, StoreError, WorkspaceId,
};

pub(crate) const LOCAL_COMPLETION_FLUSH_AFTER: Duration = Duration::from_secs(60);
pub(crate) const LOCAL_COMPLETION_PROJECTING_TURN_CAP: u64 = 64;
const LOCAL_COMPLETION_SCHEMA_VERSION: u32 = 1;
const ABSENCE_NAMESPACE: &str = "sweeps";
const LOCAL_COMPLETION_NAMESPACE: &str = "local-completion-index-v1";
const LOCAL_COMPLETION_PREFIX: &str = "local-completion-";
const DELTA_SUFFIX: &str = ".delta";
const COMPACTION_SUFFIX: &str = ".compaction";
const MAX_LOCAL_COMPLETION_OBJECT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalCompletionState {
    Attempted,
    Completed,
}

/// Exact post-execution evidence for one own-endpoint projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalCompletionEntry {
    pub(crate) intent_id: ProjectionIntentId,
    pub(crate) page_id: PageId,
    pub(crate) path: ManagedPath,
    pub(crate) state: LocalCompletionState,
    pub(crate) target_kind: ProjectionTargetKind,
    pub(crate) post_frontier: FrontierV2,
}

impl LocalCompletionEntry {
    pub(crate) fn completed(intent: &ProjectionIntent) -> Result<Self, LocalCompletionError> {
        Ok(Self {
            intent_id: intent
                .id()
                .map_err(|error| LocalCompletionError::Invalid(error.to_string()))?,
            page_id: intent.page_id(),
            path: intent.path().clone(),
            state: LocalCompletionState::Completed,
            target_kind: intent.target_kind(),
            post_frontier: intent.frontier().clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionNameHorizon {
    count: u64,
    set_digest: ContentDigest,
    previous_compaction_generation: u64,
    covered_through_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LocalCompletionPayload {
    Delta {
        entries: Vec<LocalCompletionEntry>,
    },
    Compaction {
        entries: Vec<LocalCompletionEntry>,
        horizon: CompletionNameHorizon,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalCompletionObject {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    generation: u64,
    previous_digest: Option<ContentDigest>,
    payload: LocalCompletionPayload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalCompletionObjectKind {
    Delta,
    Compaction,
}

#[derive(Clone, Debug)]
struct LocalCompletionName {
    generation: u64,
    kind: LocalCompletionObjectKind,
    name: String,
}

#[derive(Clone, Debug)]
struct StoredEntry {
    entry: LocalCompletionEntry,
    generation: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LocalCompletionOpenStats {
    pub(crate) names_observed: usize,
    pub(crate) content_reads: usize,
    pub(crate) rebuilt: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LocalCompletionFlushStats {
    pub(crate) flushes: u64,
    pub(crate) immutable_installs: u64,
    pub(crate) compactions: u64,
    pub(crate) pruned_objects: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LocalCompletionPruningContext {
    pub(crate) live_page_paths: BTreeSet<(PageId, ManagedPath)>,
    pub(crate) retained_intents: BTreeSet<ProjectionIntentId>,
    /// Durable receiver completion anchors for exactly the keys this index
    /// holds, point-read one key at a time. A present key with an empty vector
    /// means "receiver evidence exists here, but no completion anchor": the
    /// R16-C2 rule still applies. An absent key means no receiver evidence.
    pub(crate) receiver_rows: BTreeMap<AbsenceDecisionKey, Vec<AbsenceCompletionAnchor>>,
}

#[derive(Debug)]
pub(crate) enum LocalCompletionError {
    Store(StoreError),
    Encode(String),
    Invalid(String),
}

impl fmt::Display for LocalCompletionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "local completion store: {error}"),
            Self::Encode(error) => write!(formatter, "local completion encoding: {error}"),
            Self::Invalid(error) => write!(formatter, "invalid local completion index: {error}"),
        }
    }
}

impl std::error::Error for LocalCompletionError {}

impl From<StoreError> for LocalCompletionError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<std::io::Error> for LocalCompletionError {
    fn from(error: std::io::Error) -> Self {
        Self::Store(StoreError::Io(error))
    }
}

pub(crate) struct LocalCompletionIndex {
    store: ObjectStore,
    directory: Dir,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    entries: BTreeMap<ProjectionIntentId, StoredEntry>,
    buffered: BTreeMap<ProjectionIntentId, LocalCompletionEntry>,
    names: BTreeMap<u64, LocalCompletionName>,
    next_generation: u64,
    latest_compaction_generation: u64,
    deltas_since_compaction: u64,
    first_unflushed_at: Option<Instant>,
    projecting_turns: u64,
    open_stats: LocalCompletionOpenStats,
    flush_stats: LocalCompletionFlushStats,
    #[cfg(test)]
    compaction_threshold_override: Option<u64>,
}

impl LocalCompletionIndex {
    pub(crate) fn open(
        store: &ObjectStore,
        endpoint_id: ProjectionEndpointId,
    ) -> Result<Self, LocalCompletionError> {
        let root = store.private_derived_root_capability()?;
        ensure_reconstructible_directory_nofollow(&root, ABSENCE_NAMESPACE)?;
        let absence = open_dir_nofollow(&root, ABSENCE_NAMESPACE)?;
        ensure_reconstructible_directory_nofollow(&absence, LOCAL_COMPLETION_NAMESPACE)?;
        let directory = open_dir_nofollow(&absence, LOCAL_COMPLETION_NAMESPACE)?;
        let store = store.duplicate_retained_capability()?;
        let workspace_id = store.workspace_id();
        let names = enumerate_names(&directory)?;
        let mut open_stats = LocalCompletionOpenStats {
            names_observed: names.len(),
            ..LocalCompletionOpenStats::default()
        };
        let reconstructed = reconstruct(
            &directory,
            workspace_id,
            endpoint_id,
            &names,
            &mut open_stats,
        )?;
        let next_generation = names
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| LocalCompletionError::Invalid("generation overflow".into()))?;
        Ok(Self {
            store,
            directory,
            workspace_id,
            endpoint_id,
            entries: reconstructed.entries,
            buffered: BTreeMap::new(),
            names,
            next_generation,
            latest_compaction_generation: reconstructed.latest_compaction_generation,
            deltas_since_compaction: reconstructed.deltas_since_compaction,
            first_unflushed_at: None,
            projecting_turns: 0,
            open_stats,
            flush_stats: LocalCompletionFlushStats::default(),
            #[cfg(test)]
            compaction_threshold_override: None,
        })
    }

    pub(crate) fn contains_completed(&self, intent_id: ProjectionIntentId) -> bool {
        self.buffered
            .get(&intent_id)
            .is_some_and(|entry| entry.state == LocalCompletionState::Completed)
            || self
                .entries
                .get(&intent_id)
                .is_some_and(|entry| entry.entry.state == LocalCompletionState::Completed)
    }

    pub(crate) fn completed_intent_ids(&self) -> BTreeSet<ProjectionIntentId> {
        self.entries
            .iter()
            .filter(|(_, stored)| stored.entry.state == LocalCompletionState::Completed)
            .map(|(intent_id, _)| *intent_id)
            .chain(
                self.buffered
                    .iter()
                    .filter(|(_, entry)| entry.state == LocalCompletionState::Completed)
                    .map(|(intent_id, _)| *intent_id),
            )
            .collect()
    }

    /// How many own-endpoint completion entries this chain retains, counting
    /// the unflushed buffer. Diagnostic attribution only.
    pub(crate) fn completed_entry_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|(intent_id, stored)| {
                stored.entry.state == LocalCompletionState::Completed
                    && !self.buffered.contains_key(intent_id)
            })
            .count()
            + self
                .buffered
                .values()
                .filter(|entry| entry.state == LocalCompletionState::Completed)
                .count()
    }

    pub(crate) fn completed_entries(&self) -> Vec<LocalCompletionEntry> {
        self.entries
            .values()
            .filter(|stored| stored.entry.state == LocalCompletionState::Completed)
            .map(|stored| stored.entry.clone())
            .chain(
                self.buffered
                    .values()
                    .filter(|entry| entry.state == LocalCompletionState::Completed)
                    .cloned(),
            )
            .collect()
    }

    pub(crate) fn stage_completed(
        &mut self,
        intent: &ProjectionIntent,
    ) -> Result<bool, LocalCompletionError> {
        let entry = LocalCompletionEntry::completed(intent)?;
        if self.contains_completed(entry.intent_id) {
            return Ok(false);
        }
        self.buffered.insert(entry.intent_id, entry);
        self.first_unflushed_at.get_or_insert_with(Instant::now);
        self.projecting_turns = self.projecting_turns.saturating_add(1);
        Ok(true)
    }

    pub(crate) fn has_buffered(&self) -> bool {
        !self.buffered.is_empty()
    }

    pub(crate) fn cap_due(&self) -> bool {
        self.has_buffered() && self.projecting_turns >= LOCAL_COMPLETION_PROJECTING_TURN_CAP
    }

    pub(crate) fn deadline_due(&self, now: Instant) -> bool {
        self.first_unflushed_at.is_some_and(|first| {
            now.saturating_duration_since(first) >= LOCAL_COMPLETION_FLUSH_AFTER
        })
    }

    pub(crate) fn deadline_remaining(&self, now: Instant) -> Option<Duration> {
        self.first_unflushed_at.map(|first| {
            LOCAL_COMPLETION_FLUSH_AFTER.saturating_sub(now.saturating_duration_since(first))
        })
    }

    pub(crate) fn flush(
        &mut self,
        pages_at_compaction: usize,
        pruning: &LocalCompletionPruningContext,
    ) -> Result<bool, LocalCompletionError> {
        if self.buffered.is_empty() {
            return Ok(false);
        }
        let delta_generation = self.take_generation()?;
        let previous_digest = self.tail_digest()?;
        let mut delta_entries = self.buffered.values().cloned().collect::<Vec<_>>();
        delta_entries.sort_by_key(|entry| entry.intent_id);
        let delta = LocalCompletionObject {
            schema_version: LOCAL_COMPLETION_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            endpoint_id: self.endpoint_id,
            generation: delta_generation,
            previous_digest,
            payload: LocalCompletionPayload::Delta {
                entries: delta_entries.clone(),
            },
        };
        let delta_bytes = encode_object(&delta)?;
        let delta_name = object_name(delta_generation, LocalCompletionObjectKind::Delta);

        // I-14: bound both dimensions independently. Counting only immutable
        // deltas lets one coalesced delta retain arbitrarily many lifetime
        // completions, while scaling the delta threshold with page count lets
        // the name chain grow by G deltas each containing O(G) entries. The
        // next open reconstructs both, so either pressure is sufficient to
        // compact; neither is a proxy for the other.
        let delta_threshold = 256_u64;
        // A completion entry embeds its exact post-frontier, so the projecting
        // turn cap is not a memory bound: retaining 2 * cap entries can retain
        // 2 * cap copies of a G-document frontier.  Compaction pruning keeps at
        // most the evidence required by current page/path state; trigger from
        // that live dimension itself, with one as the empty-graph floor.
        let entry_threshold = usize::max(1, pages_at_compaction.saturating_mul(2));
        #[cfg(test)]
        let (delta_threshold, entry_threshold) = self
            .compaction_threshold_override
            .map_or((delta_threshold, entry_threshold), |threshold| {
                (threshold, usize::try_from(threshold).unwrap_or(usize::MAX))
            });
        let prospective_entry_count = self.entries.len().saturating_add(
            delta_entries
                .iter()
                .filter(|entry| !self.entries.contains_key(&entry.intent_id))
                .count(),
        );
        let compact = self.deltas_since_compaction.saturating_add(1) >= delta_threshold
            || prospective_entry_count > entry_threshold;
        let mut artifacts = vec![(
            delta_name.as_str(),
            delta_bytes.as_slice(),
            MAX_LOCAL_COMPLETION_OBJECT_BYTES,
        )];
        let mut compaction_artifact = None;
        let mut compacted_entries = None;
        if compact {
            let mut candidate = self.entries.clone();
            apply_entries(&mut candidate, delta_generation, &delta_entries)?;
            prune_entries(&mut candidate, pruning);
            let previous_compaction_generation = self.latest_compaction_generation;
            let covered_delta_names = self
                .names
                .values()
                .filter(|name| {
                    name.kind == LocalCompletionObjectKind::Delta
                        && name.generation > previous_compaction_generation
                })
                .map(|name| name.name.clone())
                .chain(std::iter::once(delta_name.clone()))
                .collect::<Vec<_>>();
            let horizon = completion_name_horizon(
                &covered_delta_names,
                previous_compaction_generation,
                delta_generation,
            );
            let compaction_generation = self.take_generation()?;
            let compaction = LocalCompletionObject {
                schema_version: LOCAL_COMPLETION_SCHEMA_VERSION,
                workspace_id: self.workspace_id,
                endpoint_id: self.endpoint_id,
                generation: compaction_generation,
                previous_digest: Some(ContentDigest::of(&delta_bytes)),
                payload: LocalCompletionPayload::Compaction {
                    entries: candidate
                        .values()
                        .map(|stored| stored.entry.clone())
                        .collect(),
                    horizon,
                },
            };
            let bytes = encode_object(&compaction)?;
            let name = object_name(compaction_generation, LocalCompletionObjectKind::Compaction);
            compaction_artifact = Some((name, bytes, compaction_generation));
            compacted_entries = Some(candidate);
        }
        if let Some((name, bytes, _)) = &compaction_artifact {
            artifacts.push((
                name.as_str(),
                bytes.as_slice(),
                MAX_LOCAL_COMPLETION_OBJECT_BYTES,
            ));
        }
        self.store.publish_coalesced_private_derived(
            &self.directory,
            &artifacts,
            "local completion index object",
        )?;

        apply_entries(&mut self.entries, delta_generation, &delta_entries)?;
        self.names.insert(
            delta_generation,
            LocalCompletionName {
                generation: delta_generation,
                kind: LocalCompletionObjectKind::Delta,
                name: delta_name.clone(),
            },
        );
        self.flush_stats.flushes = self.flush_stats.flushes.saturating_add(1);
        self.flush_stats.immutable_installs = self
            .flush_stats
            .immutable_installs
            .saturating_add(artifacts.len() as u64);
        if let Some((name, _, generation)) = compaction_artifact {
            self.entries = compacted_entries.expect("compaction builds its replacement map");
            self.names.insert(
                generation,
                LocalCompletionName {
                    generation,
                    kind: LocalCompletionObjectKind::Compaction,
                    name,
                },
            );
            let keep_from = self
                .names
                .values()
                .rev()
                .filter(|name| name.kind == LocalCompletionObjectKind::Compaction)
                .nth(2)
                .map_or(0, |name| name.generation);
            self.latest_compaction_generation = generation;
            self.deltas_since_compaction = 0;
            self.flush_stats.compactions = self.flush_stats.compactions.saturating_add(1);
            self.prune_superseded_chain_before(keep_from)?;
        } else {
            self.deltas_since_compaction = self.deltas_since_compaction.saturating_add(1);
        }
        self.buffered.clear();
        self.first_unflushed_at = None;
        self.projecting_turns = 0;
        Ok(true)
    }

    /// How this chain was reached at open. Diagnostic attribution only.
    pub(crate) fn open_stats(&self) -> &LocalCompletionOpenStats {
        &self.open_stats
    }

    #[cfg(test)]
    pub(crate) fn flush_stats(&self) -> &LocalCompletionFlushStats {
        &self.flush_stats
    }

    pub(crate) fn observed_page_paths(&self) -> BTreeSet<(PageId, ManagedPath)> {
        self.entries
            .values()
            .map(|stored| (stored.entry.page_id, stored.entry.path.clone()))
            .chain(
                self.buffered
                    .values()
                    .map(|entry| (entry.page_id, entry.path.clone())),
            )
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn age_buffer_for_test(&mut self, age: Duration) {
        if self.has_buffered() {
            self.first_unflushed_at = Some(Instant::now() - age);
        }
    }

    #[cfg(test)]
    pub(crate) fn entry_count_for_test(&self) -> usize {
        self.entries.len() + self.buffered.len()
    }

    #[cfg(test)]
    pub(crate) fn force_compaction_threshold_for_test(&mut self, threshold: u64) {
        self.compaction_threshold_override = Some(threshold.max(1));
    }

    fn take_generation(&mut self) -> Result<u64, LocalCompletionError> {
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .ok_or_else(|| LocalCompletionError::Invalid("generation overflow".into()))?;
        Ok(generation)
    }

    fn tail_digest(&mut self) -> Result<Option<ContentDigest>, LocalCompletionError> {
        let Some(name) = self.names.values().next_back() else {
            return Ok(None);
        };
        let bytes = read_object_bytes(&self.directory, &name.name)?;
        Ok(Some(ContentDigest::of(&bytes)))
    }

    fn prune_superseded_chain_before(
        &mut self,
        keep_from: u64,
    ) -> Result<(), LocalCompletionError> {
        if keep_from == 0 {
            return Ok(());
        }
        let obsolete = self
            .names
            .range(..keep_from)
            .map(|(generation, name)| (*generation, name.name.clone()))
            .collect::<Vec<_>>();
        for (generation, name) in &obsolete {
            match self.directory.remove_file(name) {
                Ok(()) => {
                    self.names.remove(generation);
                    self.flush_stats.pruned_objects =
                        self.flush_stats.pruned_objects.saturating_add(1);
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.names.remove(generation);
                }
                Err(error) => return Err(error.into()),
            }
        }
        if !obsolete.is_empty() {
            sync_dir_required(&self.directory)?;
        }
        Ok(())
    }
}

struct ReconstructedIndex {
    entries: BTreeMap<ProjectionIntentId, StoredEntry>,
    latest_compaction_generation: u64,
    deltas_since_compaction: u64,
}

fn reconstruct(
    directory: &Dir,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    names: &BTreeMap<u64, LocalCompletionName>,
    stats: &mut LocalCompletionOpenStats,
) -> Result<ReconstructedIndex, LocalCompletionError> {
    let compactions = names
        .values()
        .filter(|name| name.kind == LocalCompletionObjectKind::Compaction)
        .cloned()
        .collect::<Vec<_>>();
    for candidate in compactions.iter().rev() {
        let bytes = read_object_bytes_counted(directory, &candidate.name, stats)?;
        let Ok(object) = decode_bound_object(
            &bytes,
            workspace_id,
            endpoint_id,
            candidate.generation,
            LocalCompletionObjectKind::Compaction,
        ) else {
            stats.rebuilt = true;
            continue;
        };
        let LocalCompletionPayload::Compaction { entries, horizon } = object.payload else {
            unreachable!("bound object kind was checked");
        };
        if !horizon_is_fresh(names, &horizon) {
            stats.rebuilt = true;
            continue;
        }
        let mut state = BTreeMap::new();
        apply_entries(&mut state, candidate.generation, &entries)?;
        let mut previous_digest = ContentDigest::of(&bytes);
        let mut deltas = 0_u64;
        let mut valid = true;
        for name in names
            .range((candidate.generation + 1)..)
            .map(|(_, name)| name)
        {
            if name.kind != LocalCompletionObjectKind::Delta {
                continue;
            }
            let bytes = read_object_bytes_counted(directory, &name.name, stats)?;
            let object = match decode_bound_object(
                &bytes,
                workspace_id,
                endpoint_id,
                name.generation,
                LocalCompletionObjectKind::Delta,
            ) {
                Ok(object) if object.previous_digest == Some(previous_digest) => object,
                _ => {
                    valid = false;
                    break;
                }
            };
            let LocalCompletionPayload::Delta { entries } = object.payload else {
                unreachable!("bound object kind was checked");
            };
            apply_entries(&mut state, name.generation, &entries)?;
            previous_digest = ContentDigest::of(&bytes);
            deltas = deltas.saturating_add(1);
        }
        if valid {
            return Ok(ReconstructedIndex {
                entries: state,
                latest_compaction_generation: candidate.generation,
                deltas_since_compaction: deltas,
            });
        }
        stats.rebuilt = true;
    }

    // No trusted summary (or the newest chain was torn/invalid): rebuild from
    // immutable delta truth. Compaction objects are disposable summaries and
    // are deliberately ignored in this path.
    stats.rebuilt |= !compactions.is_empty();
    let mut state = BTreeMap::new();
    let mut delta_count = 0_u64;
    for name in names.values() {
        if name.kind != LocalCompletionObjectKind::Delta {
            continue;
        }
        let bytes = read_object_bytes_counted(directory, &name.name, stats)?;
        let object = match decode_bound_object(
            &bytes,
            workspace_id,
            endpoint_id,
            name.generation,
            LocalCompletionObjectKind::Delta,
        ) {
            Ok(object) => object,
            Err(LocalCompletionError::Invalid(_) | LocalCompletionError::Encode(_)) => {
                stats.rebuilt = true;
                continue;
            }
            Err(error) => return Err(error),
        };
        let LocalCompletionPayload::Delta { entries } = object.payload else {
            unreachable!("bound object kind was checked");
        };
        apply_entries(&mut state, name.generation, &entries)?;
        delta_count = delta_count.saturating_add(1);
    }
    Ok(ReconstructedIndex {
        entries: state,
        latest_compaction_generation: 0,
        deltas_since_compaction: delta_count,
    })
}

fn enumerate_names(
    directory: &Dir,
) -> Result<BTreeMap<u64, LocalCompletionName>, LocalCompletionError> {
    let mut names = BTreeMap::<u64, LocalCompletionName>::new();
    for entry in directory.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| LocalCompletionError::Invalid("non-UTF-8 index entry".into()))?;
        if !name.starts_with(LOCAL_COMPLETION_PREFIX) {
            // This is a disposable private cache namespace. Foreign names do
            // not become authority and cannot make managed open fail.
            continue;
        }
        require_regular_entry(&entry.file_type()?, &name)?;
        let Ok(parsed) = parse_object_name(&name) else {
            continue;
        };
        if let Some(existing) = names.get(&parsed.generation) {
            // A generation twin is an invalid disposable chain. Prefer the
            // delta truth and let reconstruction ignore the summary twin.
            if existing.kind == LocalCompletionObjectKind::Delta
                || parsed.kind == LocalCompletionObjectKind::Compaction
            {
                continue;
            }
        }
        names.insert(parsed.generation, parsed);
    }
    Ok(names)
}

fn object_name(generation: u64, kind: LocalCompletionObjectKind) -> String {
    let suffix = match kind {
        LocalCompletionObjectKind::Delta => DELTA_SUFFIX,
        LocalCompletionObjectKind::Compaction => COMPACTION_SUFFIX,
    };
    format!("{LOCAL_COMPLETION_PREFIX}{generation:020}{suffix}")
}

fn parse_object_name(name: &str) -> Result<LocalCompletionName, LocalCompletionError> {
    let remainder = name
        .strip_prefix(LOCAL_COMPLETION_PREFIX)
        .ok_or_else(|| LocalCompletionError::Invalid("completion name prefix".into()))?;
    let (digits, kind) = if let Some(digits) = remainder.strip_suffix(DELTA_SUFFIX) {
        (digits, LocalCompletionObjectKind::Delta)
    } else if let Some(digits) = remainder.strip_suffix(COMPACTION_SUFFIX) {
        (digits, LocalCompletionObjectKind::Compaction)
    } else {
        return Err(LocalCompletionError::Invalid(format!(
            "unknown completion object suffix in {name:?}"
        )));
    };
    if digits.len() != 20 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(LocalCompletionError::Invalid(format!(
            "non-canonical completion generation in {name:?}"
        )));
    }
    let generation = digits
        .parse::<u64>()
        .map_err(|error| LocalCompletionError::Invalid(error.to_string()))?;
    if generation == 0 || object_name(generation, kind) != name {
        return Err(LocalCompletionError::Invalid(format!(
            "non-canonical completion name {name:?}"
        )));
    }
    Ok(LocalCompletionName {
        generation,
        kind,
        name: name.to_owned(),
    })
}

fn encode_object(object: &LocalCompletionObject) -> Result<Vec<u8>, LocalCompletionError> {
    postcard::to_allocvec(object).map_err(|error| LocalCompletionError::Encode(error.to_string()))
}

fn decode_bound_object(
    bytes: &[u8],
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    generation: u64,
    kind: LocalCompletionObjectKind,
) -> Result<LocalCompletionObject, LocalCompletionError> {
    let object: LocalCompletionObject = postcard::from_bytes(bytes)
        .map_err(|error| LocalCompletionError::Invalid(error.to_string()))?;
    if encode_object(&object)? != bytes
        || object.schema_version != LOCAL_COMPLETION_SCHEMA_VERSION
        || object.workspace_id != workspace_id
        || object.endpoint_id != endpoint_id
        || object.generation != generation
        || !matches!(
            (&object.payload, kind),
            (
                LocalCompletionPayload::Delta { .. },
                LocalCompletionObjectKind::Delta
            ) | (
                LocalCompletionPayload::Compaction { .. },
                LocalCompletionObjectKind::Compaction
            )
        )
    {
        return Err(LocalCompletionError::Invalid(
            "completion object binding or canonical encoding mismatch".into(),
        ));
    }
    Ok(object)
}

fn read_object_bytes(directory: &Dir, name: &str) -> Result<Vec<u8>, LocalCompletionError> {
    read_optional_regular(directory, name, MAX_LOCAL_COMPLETION_OBJECT_BYTES, None)?
        .ok_or_else(|| LocalCompletionError::Invalid(format!("missing completion object {name:?}")))
}

fn read_object_bytes_counted(
    directory: &Dir,
    name: &str,
    stats: &mut LocalCompletionOpenStats,
) -> Result<Vec<u8>, LocalCompletionError> {
    let bytes = read_object_bytes(directory, name)?;
    stats.content_reads = stats.content_reads.saturating_add(1);
    Ok(bytes)
}

fn apply_entries(
    state: &mut BTreeMap<ProjectionIntentId, StoredEntry>,
    generation: u64,
    entries: &[LocalCompletionEntry],
) -> Result<(), LocalCompletionError> {
    let mut observed = BTreeSet::new();
    for entry in entries {
        if !observed.insert(entry.intent_id) {
            return Err(LocalCompletionError::Invalid(
                "one completion object repeats an intent id".into(),
            ));
        }
        if entry.path.as_str().is_empty() {
            return Err(LocalCompletionError::Invalid(
                "completion entry has an empty path".into(),
            ));
        }
        state.insert(
            entry.intent_id,
            StoredEntry {
                entry: entry.clone(),
                generation,
            },
        );
    }
    Ok(())
}

fn prune_entries(
    entries: &mut BTreeMap<ProjectionIntentId, StoredEntry>,
    pruning: &LocalCompletionPruningContext,
) {
    let mut latest = BTreeMap::<(PageId, ManagedPath), (ProjectionIntentId, u64)>::new();
    for (intent_id, stored) in entries.iter() {
        let key = (stored.entry.page_id, stored.entry.path.clone());
        if !pruning.live_page_paths.contains(&key) {
            continue;
        }
        let replace = latest
            .get(&key)
            .is_none_or(|(_, generation)| stored.generation >= *generation);
        if replace {
            latest.insert(key, (*intent_id, stored.generation));
        }
    }
    let latest = latest
        .into_values()
        .map(|(intent_id, _)| intent_id)
        .collect::<BTreeSet<_>>();
    let local_entries = entries
        .values()
        .map(|stored| {
            (
                (stored.entry.page_id, stored.entry.path.clone()),
                stored.entry.intent_id,
                stored.entry.post_frontier.clone(),
            )
        })
        .collect::<Vec<_>>();
    entries.retain(|intent_id, stored| {
        if latest.contains(intent_id) || pruning.retained_intents.contains(intent_id) {
            return true;
        }
        let key = (stored.entry.page_id, stored.entry.path.clone());
        let Some(receiver_anchors) = pruning.receiver_rows.get(&key) else {
            return false;
        };
        let dominated_by_local =
            local_entries
                .iter()
                .any(|(other_key, other_id, other_frontier)| {
                    other_key == &key
                        && other_id != intent_id
                        && frontier_strictly_dominates(other_frontier, &stored.entry.post_frontier)
                });
        let dominated_by_receiver = receiver_anchors.iter().any(|receiver| {
            frontier_strictly_dominates(&receiver.frontier, &stored.entry.post_frontier)
        });
        // R16-C2: while receiver history exists underneath this key, retain
        // every locally maximal answer. Pruning it could expose an older
        // receiver Present completion and mis-defer a legitimate recreation.
        !(dominated_by_local || dominated_by_receiver)
    });
}

fn completion_name_horizon(
    names: &[String],
    previous_compaction_generation: u64,
    covered_through_generation: u64,
) -> CompletionNameHorizon {
    CompletionNameHorizon {
        count: names.len() as u64,
        set_digest: name_set_digest(names.iter().map(String::as_str)),
        previous_compaction_generation,
        covered_through_generation,
    }
}

fn horizon_is_fresh(
    names: &BTreeMap<u64, LocalCompletionName>,
    horizon: &CompletionNameHorizon,
) -> bool {
    let covered = names
        .values()
        .filter(|name| {
            name.kind == LocalCompletionObjectKind::Delta
                && name.generation > horizon.previous_compaction_generation
                && name.generation <= horizon.covered_through_generation
        })
        .map(|name| name.name.as_str())
        .collect::<Vec<_>>();
    covered.len() as u64 == horizon.count
        && name_set_digest(covered.iter().copied()) == horizon.set_digest
}

fn name_set_digest<'a>(names: impl Iterator<Item = &'a str>) -> ContentDigest {
    let mut names = names.collect::<Vec<_>>();
    names.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(b"tine/local-completion-name-set/v1\0");
    hasher.update((names.len() as u64).to_be_bytes());
    for name in names {
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::oplog::{
        absence_decision::{AbsenceDecision, AbsenceDecisionMap},
        BlobDescription, CrdtPeerCounter, CrdtPeerId, DocumentDependencies, DocumentId,
        ProjectionPrecondition,
    };

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-local-completion-{label}-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            crate::test_support::remove_dir_all(&self.0);
        }
    }

    fn workspace() -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(0xc2_1000))
    }

    fn endpoint() -> ProjectionEndpointId {
        ProjectionEndpointId::from_uuid(Uuid::from_u128(0xc2_1001))
    }

    fn intent(page: u128, path: &str, version: u64) -> ProjectionIntent {
        ProjectionIntent::new(
            workspace(),
            PageId::from_uuid(Uuid::from_u128(page)),
            ManagedPath::parse(path).unwrap(),
            FrontierV2::default(),
            Vec::new(),
            ProjectionPrecondition::Absent,
            ProjectionTargetKind::Present,
            BlobDescription::of(format!("completion target {version}").as_bytes()),
            Vec::new(),
        )
        .unwrap()
    }

    fn ordered_intent(
        page: u128,
        path: &str,
        version: u64,
        target_kind: ProjectionTargetKind,
    ) -> ProjectionIntent {
        let frontier = FrontierV2::new(vec![DocumentDependencies::new(
            DocumentId::from_uuid(Uuid::from_u128(0xc2_1002)),
            vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(17), version)],
            Vec::new(),
        )
        .unwrap()])
        .unwrap();
        ProjectionIntent::new(
            workspace(),
            PageId::from_uuid(Uuid::from_u128(page)),
            ManagedPath::parse(path).unwrap(),
            frontier,
            Vec::new(),
            ProjectionPrecondition::Absent,
            target_kind,
            if target_kind == ProjectionTargetKind::Absent {
                BlobDescription::of(&[])
            } else {
                BlobDescription::of(format!("ordered completion {version}").as_bytes())
            },
            Vec::new(),
        )
        .unwrap()
    }

    fn fixture(label: &str) -> (TestDir, ObjectStore, LocalCompletionIndex) {
        let root = TestDir::new(label);
        let store = ObjectStore::open(&root.path().join("operations"), workspace()).unwrap();
        let index = LocalCompletionIndex::open(&store, endpoint()).unwrap();
        (root, store, index)
    }

    fn live(intents: &[&ProjectionIntent]) -> LocalCompletionPruningContext {
        LocalCompletionPruningContext {
            live_page_paths: intents
                .iter()
                .map(|intent| (intent.page_id(), intent.path().clone()))
                .collect(),
            retained_intents: BTreeSet::new(),
            ..LocalCompletionPruningContext::default()
        }
    }

    #[test]
    fn one_quiet_buffered_entry_has_the_sixty_second_deadline() {
        let (_root, _store, mut index) = fixture("single-deadline");
        let intent = intent(0xc2_1100, "single.md", 1);
        assert!(index.stage_completed(&intent).unwrap());
        assert!(!index.cap_due());
        assert!(index.deadline_remaining(Instant::now()).unwrap() <= Duration::from_secs(60));
        index.age_buffer_for_test(LOCAL_COMPLETION_FLUSH_AFTER);
        assert!(index.deadline_due(Instant::now()));
    }

    #[test]
    fn sixty_four_projecting_entries_reach_the_busy_graph_cap() {
        let (_root, _store, mut index) = fixture("turn-cap");
        assert_eq!(LOCAL_COMPLETION_PROJECTING_TURN_CAP, 64);
        for ordinal in 0..64 {
            let intent = intent(
                0xc2_1200 + u128::from(ordinal),
                &format!("cap-{ordinal}.md"),
                ordinal,
            );
            index.stage_completed(&intent).unwrap();
        }
        assert!(index.cap_due());
    }

    #[test]
    fn busy_graph_flush_installs_amortize_over_the_sixty_four_turn_cap() {
        let (_root, _store, mut index) = fixture("busy-amortization");
        let mut current = Vec::new();
        for ordinal in 0..128_u64 {
            let completed = intent(
                0xc2_1250 + u128::from(ordinal),
                &format!("busy-{ordinal}.md"),
                ordinal,
            );
            index.stage_completed(&completed).unwrap();
            current.push(completed);
            if index.cap_due() {
                let pruning = live(&current.iter().collect::<Vec<_>>());
                index.flush(128, &pruning).unwrap();
            }
        }
        let stats = index.flush_stats();
        assert_eq!(stats.flushes, 2);
        assert_eq!(stats.immutable_installs, 2);
        assert_eq!(stats.compactions, 0);
        assert_eq!(stats.immutable_installs as f64 / 128.0, 1.0 / 64.0);
    }

    #[test]
    fn completion_after_summary_is_delta_read_from_the_name_set_diff() {
        let (_root, store, mut index) = fixture("summary-staleness");
        index.force_compaction_threshold_for_test(1);
        let first = intent(0xc2_1300, "first.md", 1);
        index.stage_completed(&first).unwrap();
        index.flush(1, &live(&[&first])).unwrap();

        index.force_compaction_threshold_for_test(100);
        let second = intent(0xc2_1301, "second.md", 2);
        index.stage_completed(&second).unwrap();
        index.flush(2, &live(&[&first, &second])).unwrap();
        drop(index);

        let reopened = LocalCompletionIndex::open(&store, endpoint()).unwrap();
        assert!(reopened.contains_completed(first.id().unwrap()));
        assert!(reopened.contains_completed(second.id().unwrap()));
        assert_eq!(reopened.open_stats().content_reads, 2);
        assert!(!reopened.open_stats().rebuilt);
    }

    #[test]
    fn an_invalid_latest_summary_rebuilds_from_retained_delta_truth() {
        let (_root, store, mut index) = fixture("summary-rebuild");
        index.force_compaction_threshold_for_test(1);
        let first = intent(0xc2_1400, "rebuild.md", 1);
        index.stage_completed(&first).unwrap();
        index.flush(1, &live(&[&first])).unwrap();
        let second = intent(0xc2_1400, "rebuild.md", 2);
        index.stage_completed(&second).unwrap();
        index.flush(1, &live(&[&second])).unwrap();
        let latest = index
            .names
            .values()
            .rev()
            .find(|name| name.kind == LocalCompletionObjectKind::Compaction)
            .unwrap()
            .name
            .clone();
        std::fs::write(
            store
                .root_path()
                .join(ABSENCE_NAMESPACE)
                .join(LOCAL_COMPLETION_NAMESPACE)
                .join(latest),
            b"torn derived summary",
        )
        .unwrap();
        drop(index);

        let reopened = LocalCompletionIndex::open(&store, endpoint()).unwrap();
        assert!(reopened.open_stats().rebuilt);
        assert!(reopened.contains_completed(first.id().unwrap()));
        assert!(reopened.contains_completed(second.id().unwrap()));
    }

    #[test]
    fn compaction_keeps_an_older_intent_while_a_retained_frame_references_it() {
        let (_root, _store, mut index) = fixture("retained-intent");
        index.force_compaction_threshold_for_test(1);
        let older = intent(0xc2_1500, "retained.md", 1);
        index.stage_completed(&older).unwrap();
        index.flush(1, &live(&[&older])).unwrap();

        let newer = intent(0xc2_1500, "retained.md", 2);
        index.stage_completed(&newer).unwrap();
        let mut pruning = live(&[&newer]);
        pruning.retained_intents.insert(older.id().unwrap());
        index.flush(1, &pruning).unwrap();
        assert_eq!(index.entry_count_for_test(), 2);
        assert!(index.contains_completed(older.id().unwrap()));

        let newest = intent(0xc2_1500, "retained.md", 3);
        index.stage_completed(&newest).unwrap();
        index.flush(1, &live(&[&newest])).unwrap();
        assert_eq!(index.entry_count_for_test(), 1);
        assert!(index.contains_completed(newest.id().unwrap()));
    }

    #[test]
    fn receiver_history_keeps_the_local_absent_answer_needed_for_recreation() {
        let (_root, _store, mut index) = fixture("receiver-history-recreation");
        index.force_compaction_threshold_for_test(1);
        let receiver_present = ordered_intent(
            0xc2_1550,
            "receiver-history.md",
            1,
            ProjectionTargetKind::Present,
        );
        let local_absent = ordered_intent(
            0xc2_1550,
            "receiver-history.md",
            2,
            ProjectionTargetKind::Absent,
        );
        index.stage_completed(&local_absent).unwrap();
        let receiver_anchor = AbsenceCompletionAnchor::from_intent(&receiver_present).unwrap();
        let pruning = LocalCompletionPruningContext {
            receiver_rows: [(receiver_anchor.key(), vec![receiver_anchor.clone()])]
                .into_iter()
                .collect(),
            ..LocalCompletionPruningContext::default()
        };
        index.flush(0, &pruning).unwrap();
        assert_eq!(index.entry_count_for_test(), 1);

        let mut merged = AbsenceDecisionMap::default();
        merged
            .record_receiver_completion(&receiver_present)
            .unwrap();
        for entry in index.completed_entries() {
            merged
                .record_local_completion(AbsenceCompletionAnchor {
                    intent_id: entry.intent_id,
                    page_id: entry.page_id,
                    path: entry.path,
                    target_kind: entry.target_kind,
                    frontier: entry.post_frontier,
                })
                .unwrap();
        }
        assert_eq!(
            merged
                .decision(local_absent.page_id(), local_absent.path())
                .unwrap(),
            AbsenceDecision::Create,
            "pruning the local Absent row would expose the older receiver Present completion"
        );
    }

    #[test]
    fn rename_and_delete_recreate_growth_plateau_after_compaction() {
        let (_root, _store, mut index) = fixture("growth-plateau");
        index.force_compaction_threshold_for_test(1);
        let page = 0xc2_1600;
        let mut rename_curve = Vec::new();
        for ordinal in 0..16_u64 {
            let renamed = intent(page, &format!("rename-{ordinal}.md"), ordinal);
            index.stage_completed(&renamed).unwrap();
            index.flush(1, &live(&[&renamed])).unwrap();
            assert_eq!(index.entry_count_for_test(), 1);
            assert!(
                index.names.len() <= 5,
                "chain did not plateau: {:?}",
                index.names
            );
            rename_curve.push(index.names.len());
        }
        let mut recreate_curve = Vec::new();
        for ordinal in 16..32_u64 {
            let generation = intent(page, "cycle.md", ordinal);
            index.stage_completed(&generation).unwrap();
            index.flush(1, &live(&[&generation])).unwrap();
            assert_eq!(index.entry_count_for_test(), 1);
            assert!(
                index.names.len() <= 5,
                "chain did not plateau: {:?}",
                index.names
            );
            recreate_curve.push(index.names.len());
        }
        assert!(rename_curve[2..].iter().all(|count| *count == 5));
        assert!(recreate_curve.iter().all(|count| *count == 5));
        eprintln!(
            "local_completion_growth rename={rename_curve:?} delete_recreate={recreate_curve:?}"
        );
    }

    #[test]
    fn one_large_delta_compacts_at_the_live_graph_entry_bound() {
        let (_root, _store, mut index) = fixture("large-delta-entry-bound");
        let page = 0xc2_1700;
        let mut newest = None;
        for ordinal in 0..3_u64 {
            let completion = intent(page, &format!("large-delta-{ordinal}.md"), ordinal);
            index.stage_completed(&completion).unwrap();
            newest = Some(completion);
        }
        let newest = newest.unwrap();

        index.flush(1, &live(&[&newest])).unwrap();

        assert_eq!(index.flush_stats().compactions, 1);
        assert_eq!(
            index.entry_count_for_test(),
            1,
            "one coalesced delta must not bypass the live-graph compaction bound"
        );
    }
}
