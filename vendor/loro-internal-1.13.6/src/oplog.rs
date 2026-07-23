mod change_store;
pub(crate) mod loro_dag;
mod pending_changes;

use crate::sync::{AtomicUsize, Mutex};
use bytes::Bytes;
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;
use std::sync::Arc;
use tracing::trace_span;

use self::change_store::iter::MergedChangeIter;
use self::pending_changes::{PendingChanges, PendingChangesRollback};
use super::arena::{SharedArena, SharedArenaRollback};
use crate::change::{get_sys_timestamp, Change, Lamport, Timestamp};
use crate::configure::Configure;
use crate::container::list::list_op;
use crate::dag::{Dag, DagUtils};
use crate::diff_calc::{DiffMode, ExternalImportBaseline, ExternalImportDiff};
use crate::encoding::decode_oplog;
use crate::encoding::{ImportStatus, ParsedHeaderAndBody};
use crate::history_cache::ContainerHistoryCache;
use crate::id::{Counter, PeerID, ID};
use crate::kv_store::KvStoreHandle;
use crate::op::{FutureInnerContent, ListSlice, RawOpContent, RemoteOp, RichOp};
use crate::span::{HasCounterSpan, HasLamportSpan};
use crate::version::{Frontiers, ImVersionVector, VersionVector};
use crate::{LoroError, LoroResult};
use change_store::{BlockOpRef, ChangeStoreRollback};
use loro_common::{ContainerType, HasIdSpan, IdLp, IdSpan};
use rle::{HasLength, RleVec, Sliceable};
use smallvec::SmallVec;

pub use self::loro_dag::{AppDag, AppDagNode, FrontiersNotIncluded};
pub use change_store::{BlockChangeRef, ChangeStore};

/// [OpLog] store all the ops i.e. the history.
/// It allows multiple [AppState] to attach to it.
/// So you can derive different versions of the state from the [OpLog].
/// It allows us to build a version control system.
///
/// The causal graph should always be a DAG and complete. So we can always find the LCA.
/// If deps are missing, we can't import the change. It will be put into the `pending_changes`.
pub struct OpLog {
    pub(crate) dag: AppDag,
    pub(crate) arena: SharedArena,
    visible_op_count: Arc<AtomicUsize>,
    change_store: ChangeStore,
    history_cache: Mutex<ContainerHistoryCache>,
    /// Pending changes that haven't been applied to the dag.
    /// A change can be imported only when all its deps are already imported.
    /// Key is the ID of the missing dep
    pub(crate) pending_changes: PendingChanges,
    /// Whether we are importing a batch of changes.
    /// If so the Dag's frontiers won't be updated until the batch is finished.
    pub(crate) batch_importing: bool,
    pub(crate) configure: Configure,
    /// The uncommitted change, it's a placeholder for the change
    /// that is being edited in pre-commit callback.
    pub(crate) uncommitted_change: Option<Change>,
    pub(crate) import_rollback: Option<ImportRollback>,
    external_import_baseline: Option<ExternalImportBaseline>,
    external_open_causal_snapshot: Option<crate::external_store::CausalDagSnapshot>,
    external_greatest_timestamp: Option<Timestamp>,
    external_metadata_digest: Option<[u8; 32]>,
}

pub(crate) struct ImportRollback {
    old_vv: VersionVector,
    external_greatest_timestamp: Option<Timestamp>,
    arena: SharedArenaRollback,
    change_store: ChangeStoreRollback,
    pending: PendingChangesRollback,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ImportChangesPreflight {
    pub applies_to_dag: bool,
    pub has_deps_before_shallow_root: bool,
    pub needs_state_apply_rollback: bool,
}

impl std::fmt::Debug for OpLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpLog")
            .field("dag", &self.dag)
            .field("pending_changes", &self.pending_changes)
            .finish()
    }
}

impl OpLog {
    #[inline]
    pub(crate) fn new(visible_op_count: Arc<AtomicUsize>) -> Self {
        let arena = SharedArena::new();
        let cfg = Configure::default();
        let change_store = ChangeStore::new_mem(&arena, cfg.merge_interval_in_s.clone());
        Self {
            visible_op_count,
            history_cache: Mutex::new(ContainerHistoryCache::new(change_store.clone(), None)),
            dag: AppDag::new(change_store.clone()),
            change_store,
            arena,
            pending_changes: Default::default(),
            batch_importing: false,
            configure: cfg,
            uncommitted_change: None,
            import_rollback: None,
            external_import_baseline: None,
            external_open_causal_snapshot: None,
            external_greatest_timestamp: None,
            external_metadata_digest: None,
        }
    }

    pub(crate) fn new_external(
        visible_op_count: Arc<AtomicUsize>,
        external_kv: KvStoreHandle,
    ) -> Result<Self, LoroError> {
        let arena = SharedArena::new();
        let cfg = Configure::default();
        let (change_store, info) =
            ChangeStore::new_external(&arena, cfg.merge_interval_in_s.clone(), external_kv)?;
        let external_import_baseline = match info.import_baseline.clone() {
            Some(snapshot) => ExternalImportBaseline::from_snapshot(snapshot)?,
            None => ExternalImportBaseline::empty(),
        };
        let external_open_causal_snapshot = info.causal_dag.clone();
        let external_greatest_timestamp = info.greatest_timestamp;
        let external_metadata_digest = info.metadata_digest;
        let mut dag = AppDag::new(change_store.clone());
        dag.set_version_by_fast_snapshot_import(info);
        let oplog = Self {
            visible_op_count,
            history_cache: Mutex::new(ContainerHistoryCache::new(change_store.clone(), None)),
            dag,
            change_store,
            arena,
            pending_changes: Default::default(),
            batch_importing: false,
            configure: cfg,
            uncommitted_change: None,
            import_rollback: None,
            external_import_baseline: Some(external_import_baseline),
            external_open_causal_snapshot,
            external_greatest_timestamp,
            external_metadata_digest,
        };
        oplog.refresh_visible_op_count();
        Ok(oplog)
    }

    #[inline]
    fn calc_visible_op_count(&self) -> usize {
        let total = self.dag.vv().values().sum::<i32>() as usize;
        let shallow = self
            .dag
            .shallow_since_vv()
            .iter()
            .map(|(_, ops)| *ops)
            .sum::<i32>() as usize;
        total - shallow
    }

    #[inline]
    pub(crate) fn visible_op_count_exact(&self) -> usize {
        self.calc_visible_op_count()
    }

    #[inline]
    pub(crate) fn refresh_visible_op_count(&self) -> usize {
        let count = self.calc_visible_op_count();
        self.visible_op_count
            .store(count, std::sync::atomic::Ordering::Release);
        count
    }

    #[inline]
    pub fn dag(&self) -> &AppDag {
        &self.dag
    }

    pub fn change_store(&self) -> &ChangeStore {
        &self.change_store
    }

    pub(crate) fn probe_external_store(&self) -> Result<(), LoroError> {
        self.change_store.probe_external_store()
    }

    pub(crate) fn take_external_store_error(&self) -> Option<LoroError> {
        self.change_store.take_external_error()
    }

    pub(crate) fn prepare_external_import_diff(
        &self,
        from_vv: &VersionVector,
        from_frontiers: &Frontiers,
        to_vv: &VersionVector,
        to_frontiers: &Frontiers,
    ) -> LoroResult<ExternalImportDiff> {
        let baseline = self.external_import_baseline.as_ref().ok_or_else(|| {
            LoroError::ArgErr("document does not use an external change store".into())
        })?;
        let caught_up = baseline.advance(self, from_vv, from_frontiers, false)?;
        let ExternalImportDiff::Fast {
            baseline: caught_up,
            ..
        } = caught_up;
        caught_up.advance(self, to_vv, to_frontiers, true)
    }

    pub(crate) fn validate_external_import_changes(&self, changes: &[Change]) -> LoroResult<()> {
        if !self.change_store.is_external() {
            return Ok(());
        }
        self.external_import_baseline
            .as_ref()
            .ok_or_else(|| LoroError::DecodeError("external import baseline is missing".into()))?
            .validate_supported_import(changes)
    }

    pub(crate) fn validate_external_import_dependencies(
        &self,
        changes: &[Change],
    ) -> LoroResult<()> {
        if !self.change_store.is_external() {
            return Ok(());
        }
        let mut checked = std::collections::BTreeSet::new();
        for change in changes {
            for dep in change.deps.iter() {
                let known_end = self.vv().get(&dep.peer).copied().unwrap_or(0);
                if dep.counter < known_end && checked.insert(dep) {
                    let loaded = self.change_store.get_change_fallible(dep)?;
                    if !loaded.contains_id(dep) {
                        return Err(LoroError::DecodeError(
                            "external import dependency header is inconsistent".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn validate_external_import_baseline_coverage(
        &mut self,
        text_containers: &std::collections::BTreeSet<crate::container::idx::ContainerIdx>,
    ) -> LoroResult<()> {
        let arena_container_count = self.arena.container_count();
        self.external_import_baseline
            .as_mut()
            .ok_or_else(|| LoroError::DecodeError("external import baseline is missing".into()))?
            .validate_text_coverage(text_containers, arena_container_count)
    }

    pub(crate) fn validate_external_metadata_semantics(&self) -> LoroResult<()> {
        let baseline = self
            .external_import_baseline
            .as_ref()
            .ok_or_else(|| LoroError::DecodeError("external import baseline is missing".into()))?;
        baseline.validate_authentication(&self.arena)?;
        match self.external_open_causal_snapshot.as_ref() {
            Some(snapshot) => self
                .change_store
                .validate_persisted_external_metadata_semantics(snapshot),
            None if self.dag.vv().is_empty() => Ok(()),
            None => Err(LoroError::DecodeError(
                "external causal metadata is missing".into(),
            )),
        }
    }

    pub(crate) fn commit_external_import_baseline(&mut self, baseline: ExternalImportBaseline) {
        self.external_import_baseline = Some(baseline);
    }

    /// Get the change with the given peer and lamport.
    ///
    /// If not found, return the change with the greatest lamport that is smaller than the given lamport.
    pub fn get_change_with_lamport_lte(
        &self,
        peer: PeerID,
        lamport: Lamport,
    ) -> Option<BlockChangeRef> {
        let ans = self
            .change_store
            .get_change_by_lamport_lte(IdLp::new(peer, lamport))?;
        debug_assert!(ans.lamport <= lamport);
        Some(ans)
    }

    pub fn get_timestamp_of_version(&self, f: &Frontiers) -> Timestamp {
        let mut timestamp = Timestamp::default();
        for id in f.iter() {
            if let Some(change) = self.lookup_change(id) {
                timestamp = timestamp.max(change.timestamp);
            }
        }

        timestamp
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dag.is_empty() && self.arena.can_import_snapshot()
    }

    /// This is the **only** place to update the `OpLog.changes`
    pub(crate) fn insert_new_change(&mut self, change: Change, from_local: bool) {
        let s = trace_span!(
            "insert_new_change",
            id = ?change.id,
            lamport = change.lamport,
            deps = ?change.deps
        );
        let _enter = s.enter();
        if let Some(timestamp) = self.external_greatest_timestamp.as_mut() {
            *timestamp = (*timestamp).max(change.timestamp);
        }
        let rollback_old_vv = self
            .import_rollback
            .as_ref()
            .and_then(|x| (!x.old_vv.is_empty()).then_some(&x.old_vv));
        self.dag
            .handle_new_change(&change, from_local, rollback_old_vv);
        self.history_cache
            .lock()
            .insert_by_new_change(&change, true, true);
        self.register_container_and_parent_link(&change);
        if let Some(rollback) = self.import_rollback.as_mut() {
            self.change_store.insert_change_with_rollback(
                change,
                true,
                from_local,
                &mut rollback.change_store,
            );
        } else {
            self.change_store.insert_change(change, true, from_local);
        }
        self.refresh_visible_op_count();
    }

    pub(crate) fn begin_import_rollback(&mut self) {
        let arena = self.arena.checkpoint_for_rollback();
        self.begin_import_rollback_with_arena(arena);
    }

    pub(crate) fn begin_import_rollback_with_arena(&mut self, arena: SharedArenaRollback) {
        debug_assert!(self.import_rollback.is_none());
        let old_vv = self.vv().clone();
        self.dag.begin_import_rollback();
        self.import_rollback = Some(ImportRollback {
            old_vv: old_vv.clone(),
            external_greatest_timestamp: self.external_greatest_timestamp,
            arena,
            change_store: ChangeStoreRollback::new(old_vv),
            pending: Default::default(),
        });
    }

    pub(crate) fn commit_import_rollback(&mut self) {
        self.dag.commit_import_rollback();
        self.import_rollback = None;
    }

    pub(crate) fn preflight_import_changes(&self, changes: &[Change]) -> ImportChangesPreflight {
        let mut ans = ImportChangesPreflight::default();
        let pending_needs_state_apply_rollback =
            self.pending_changes.has_state_apply_rollback_ops();
        for change in changes {
            if change.ctr_end() <= self.vv().get(&change.id.peer).copied().unwrap_or(0) {
                continue;
            }

            if self.dag.is_before_shallow_root(&change.deps) {
                ans.has_deps_before_shallow_root = true;
                continue;
            }

            if self
                .dag
                .get_change_lamport_from_deps(&change.deps)
                .is_none()
            {
                continue;
            }

            ans.applies_to_dag = true;
            if change.ops.iter().any(|op| {
                matches!(
                    op.container.get_type(),
                    ContainerType::List | ContainerType::Tree
                )
            }) {
                ans.needs_state_apply_rollback = true;
            }
        }

        // Any newly applied change can unlock pending changes whose ops are not
        // visible in `changes`, so include pending in the rollback decision.
        // Keep this narrow: text/map-only pending changes cannot return a
        // state-apply error, and forcing rollback there adds lock traffic to
        // small sync/import workloads.
        if ans.applies_to_dag && pending_needs_state_apply_rollback {
            ans.needs_state_apply_rollback = true;
        }

        #[cfg(test)]
        if ans.applies_to_dag {
            ans.needs_state_apply_rollback = true;
        }

        ans
    }

    pub(crate) fn rollback_import(&mut self) {
        let Some(rollback) = self.import_rollback.take() else {
            return;
        };

        self.change_store.rollback_import(rollback.change_store);
        self.dag.rollback_import();
        self.external_greatest_timestamp = rollback.external_greatest_timestamp;
        rollback.pending.rollback(&mut self.pending_changes);
        self.history_cache.lock().free_all();
        self.arena.rollback(rollback.arena);
        self.refresh_visible_op_count();
    }

    pub(crate) fn reset_to_empty_for_failed_snapshot_import(
        &mut self,
        arena_checkpoint: SharedArenaRollback,
    ) {
        let arena = self.arena.clone();
        let configure = self.configure.clone();
        arena.rollback(arena_checkpoint);
        let change_store = ChangeStore::new_mem(&arena, configure.merge_interval_in_s.clone());
        self.history_cache = Mutex::new(ContainerHistoryCache::new(change_store.clone(), None));
        self.dag = AppDag::new(change_store.clone());
        self.change_store = change_store;
        self.pending_changes = Default::default();
        self.batch_importing = false;
        self.configure = configure;
        self.uncommitted_change = None;
        self.import_rollback = None;
        self.visible_op_count
            .store(0, std::sync::atomic::Ordering::Release);
    }

    #[inline(always)]
    pub(crate) fn with_history_cache<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ContainerHistoryCache) -> R,
    {
        let mut history_cache = self.history_cache.lock();
        f(&mut history_cache)
    }

    pub fn has_history_cache(&self) -> bool {
        self.history_cache.lock().has_cache()
    }

    pub fn free_history_cache(&self) {
        let mut history_cache = self.history_cache.lock();
        history_cache.free();
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn pending_changes_len(&self) -> usize {
        self.pending_changes.len()
    }

    /// Import a change.
    ///
    /// Pending changes that haven't been applied to the dag.
    /// A change can be imported only when all its deps are already imported.
    /// Key is the ID of the missing dep
    ///
    /// # Err
    ///
    /// - Return Err(LoroError::UsedOpID) when the change's id is occupied
    /// - Return Err(LoroError::DecodeError) when the change's deps are missing
    pub(crate) fn import_local_change(&mut self, change: Change) -> Result<(), LoroError> {
        self.insert_new_change(change, true);
        Ok(())
    }

    /// Trim the known part of change
    pub(crate) fn trim_the_known_part_of_change(&self, change: Change) -> Option<Change> {
        let Some(&end) = self.dag.vv().get(&change.id.peer) else {
            return Some(change);
        };

        if change.id.counter >= end {
            return Some(change);
        }

        if change.ctr_end() <= end {
            return None;
        }

        let offset = (end - change.id.counter) as usize;
        Some(change.slice(offset, change.atom_len()))
    }

    #[allow(unused)]
    fn check_id_is_not_duplicated(&self, id: ID) -> Result<(), LoroError> {
        let cur_end = self.dag.vv().get(&id.peer).cloned().unwrap_or(0);
        if cur_end > id.counter {
            return Err(LoroError::UsedOpID { id });
        }

        Ok(())
    }

    /// Ensure the new change is greater than the last peer's id and the counter is continuous.
    ///
    /// It can be false when users use detached editing mode and use a custom peer id.
    // This method might be slow and can be optimized if needed in the future.
    pub(crate) fn check_change_greater_than_last_peer_id(
        &self,
        peer: PeerID,
        counter: Counter,
        deps: &Frontiers,
    ) -> Result<(), LoroError> {
        if counter == 0 {
            return Ok(());
        }

        if !self.configure.detached_editing() {
            return Ok(());
        }

        let mut max_last_counter = -1;
        for dep in deps.iter() {
            let dep_vv = self
                .dag
                .get_vv(dep)
                .ok_or(LoroError::FrontiersNotFound(dep))?;
            max_last_counter = max_last_counter.max(dep_vv.get(&peer).cloned().unwrap_or(0) - 1);
        }

        if counter != max_last_counter + 1 {
            return Err(LoroError::ConcurrentOpsWithSamePeerID {
                peer,
                last_counter: max_last_counter,
                current: counter,
            });
        }

        Ok(())
    }

    pub(crate) fn next_id(&self, peer: PeerID) -> ID {
        let cnt = self.dag.vv().get(&peer).copied().unwrap_or(0);
        ID::new(peer, cnt)
    }

    pub(crate) fn vv(&self) -> &VersionVector {
        self.dag.vv()
    }

    pub(crate) fn frontiers(&self) -> &Frontiers {
        self.dag.frontiers()
    }

    /// - Ordering::Less means self is less than target or parallel
    /// - Ordering::Equal means versions equal
    /// - Ordering::Greater means self's version is greater than target
    pub fn cmp_with_frontiers(&self, other: &Frontiers) -> Ordering {
        self.dag.cmp_with_frontiers(other)
    }

    /// Compare two [Frontiers] causally.
    ///
    /// If one of the [Frontiers] are not included, it will return [FrontiersNotIncluded].
    #[inline]
    pub fn cmp_frontiers(
        &self,
        a: &Frontiers,
        b: &Frontiers,
    ) -> Result<Option<Ordering>, FrontiersNotIncluded> {
        self.dag.cmp_frontiers(a, b)
    }

    pub(crate) fn get_min_lamport_at(&self, id: ID) -> Lamport {
        self.get_change_at(id).map(|c| c.lamport).unwrap_or(0)
    }

    pub(crate) fn get_lamport_at(&self, id: ID) -> Option<Lamport> {
        self.get_change_at(id)
            .map(|c| c.lamport + (id.counter - c.id.counter) as Lamport)
    }

    pub(crate) fn iter_ops(&self, id_span: IdSpan) -> impl Iterator<Item = RichOp<'static>> + '_ {
        let change_iter = self.change_store.iter_changes(id_span);
        change_iter.flat_map(move |c| RichOp::new_iter_by_cnt_range(c, id_span.counter))
    }

    pub(crate) fn get_max_lamport_at(&self, id: ID) -> Lamport {
        self.get_change_at(id)
            .map(|c| {
                let change_counter = c.id.counter as u32;
                c.lamport + c.ops().last().map(|op| op.counter).unwrap_or(0) as u32 - change_counter
            })
            .unwrap_or(Lamport::MAX)
    }

    pub fn get_change_at(&self, id: ID) -> Option<BlockChangeRef> {
        self.change_store.get_change(id)
    }

    pub(crate) fn set_uncommitted_change(&mut self, change: Change) {
        self.uncommitted_change = Some(change);
    }

    pub(crate) fn get_uncommitted_change_in_span(
        &self,
        id_span: IdSpan,
    ) -> Option<Cow<'_, Change>> {
        self.uncommitted_change.as_ref().and_then(|c| {
            if c.id_span() == id_span {
                Some(Cow::Borrowed(c))
            } else if let Some((start, end)) = id_span.get_slice_range_on(&c.id_span()) {
                Some(Cow::Owned(c.slice(start, end)))
            } else {
                None
            }
        })
    }

    pub fn get_deps_of(&self, id: ID) -> Option<Frontiers> {
        self.get_change_at(id).map(|c| {
            if c.id.counter == id.counter {
                c.deps.clone()
            } else {
                Frontiers::from_id(id.inc(-1))
            }
        })
    }

    pub fn get_remote_change_at(&self, id: ID) -> Option<Change<RemoteOp<'static>>> {
        let change = self.get_change_at(id)?;
        Some(convert_change_to_remote(&self.arena, &change))
    }

    pub(crate) fn import_unknown_lamport_pending_changes(
        &mut self,
        remote_changes: Vec<Change>,
    ) -> Result<(), LoroError> {
        self.extend_pending_changes_with_unknown_lamport(remote_changes)
    }

    /// lookup change by id.
    ///
    /// if id does not included in this oplog, return None
    pub(crate) fn lookup_change(&self, id: ID) -> Option<BlockChangeRef> {
        self.change_store.get_change(id)
    }

    #[inline(always)]
    pub(crate) fn export_change_store_from(&self, vv: &VersionVector, f: &Frontiers) -> Bytes {
        self.change_store
            .export_from(vv, f, self.vv(), self.frontiers())
    }

    #[inline(always)]
    pub(crate) fn export_change_store_in_range(
        &self,
        vv: &VersionVector,
        f: &Frontiers,
        to_vv: &VersionVector,
        to_frontiers: &Frontiers,
    ) -> Bytes {
        self.change_store.export_from(vv, f, to_vv, to_frontiers)
    }

    #[inline(always)]
    pub(crate) fn export_blocks_from<W: std::io::Write>(&self, vv: &VersionVector, w: &mut W) {
        self.change_store
            .export_blocks_from(vv, self.shallow_since_vv(), self.vv(), w)
    }

    #[inline(always)]
    pub(crate) fn export_blocks_in_range<W: std::io::Write>(&self, spans: &[IdSpan], w: &mut W) {
        self.change_store.export_blocks_in_range(spans, w)
    }

    pub(crate) fn fork_changes_up_to(&self, frontiers: &Frontiers) -> Option<Bytes> {
        let vv = self.dag.frontiers_to_vv(frontiers)?;
        Some(
            self.change_store
                .fork_changes_up_to(self.dag.shallow_since_vv(), frontiers, &vv),
        )
    }

    #[inline(always)]
    pub(crate) fn decode(&mut self, data: ParsedHeaderAndBody) -> Result<ImportStatus, LoroError> {
        decode_oplog(self, data)
    }

    /// iterates over all changes between LCA(common ancestors) to the merged version of (`from` and `to`) causally
    ///
    /// Tht iterator will include a version vector when the change is applied
    ///
    /// returns: (common_ancestor_vv, iterator)
    ///
    /// Note: the change returned by the iterator may include redundant ops at the beginning, you should trim it by yourself.
    /// You can trim it by the provided counter value. It should start with the counter.
    ///
    /// If frontiers are provided, it will be faster (because we don't need to calculate it from version vector
    #[allow(clippy::type_complexity)]
    pub(crate) fn iter_from_lca_causally(
        &self,
        from: &VersionVector,
        from_frontiers: &Frontiers,
        to: &VersionVector,
        to_frontiers: &Frontiers,
    ) -> (
        VersionVector,
        DiffMode,
        impl Iterator<
                Item = (
                    BlockChangeRef,
                    (Counter, Counter),
                    Rc<RefCell<VersionVector>>,
                ),
            > + '_,
    ) {
        let mut merged_vv = from.clone();
        merged_vv.merge(to);
        loro_common::debug!("to_frontiers={:?} vv={:?}", &to_frontiers, to);
        let (common_ancestors, mut diff_mode) =
            self.dag.find_common_ancestor(from_frontiers, to_frontiers);
        if diff_mode == DiffMode::Checkout && to > from {
            diff_mode = DiffMode::Import;
        }

        let common_ancestors_vv = self.dag.frontiers_to_vv(&common_ancestors).unwrap();
        // go from lca to merged_vv
        let diff = common_ancestors_vv.diff(&merged_vv).forward;
        let mut iter = self.dag.iter_causal(common_ancestors, diff);
        let mut node = iter.next();
        let mut cur_cnt = 0;
        let vv = Rc::new(RefCell::new(VersionVector::default()));
        (
            common_ancestors_vv.clone(),
            diff_mode,
            std::iter::from_fn(move || {
                if let Some(inner) = &node {
                    let mut inner_vv = vv.borrow_mut();
                    // FIXME: PERF: it looks slow for large vv, like 10000+ entries
                    inner_vv.clear();
                    self.dag.ensure_vv_for(&inner.data);
                    inner_vv.extend_to_include_vv(inner.data.vv.get().unwrap().iter());
                    let peer = inner.data.peer;
                    let cnt = inner
                        .data
                        .cnt
                        .max(cur_cnt)
                        .max(common_ancestors_vv.get(&peer).copied().unwrap_or(0));
                    let dag_node_end = (inner.data.cnt + inner.data.len as Counter)
                        .min(merged_vv.get(&peer).copied().unwrap_or(0));
                    let change = self.change_store.get_change(ID::new(peer, cnt)).unwrap();

                    if change.ctr_end() < dag_node_end {
                        cur_cnt = change.ctr_end();
                    } else {
                        node = iter.next();
                        cur_cnt = 0;
                    }

                    inner_vv.extend_to_include_end_id(change.id);

                    Some((change, (cnt, dag_node_end), vv.clone()))
                } else {
                    None
                }
            }),
        )
    }

    pub(crate) fn collect_changes_causally_between(
        &self,
        from: &VersionVector,
        from_frontiers: &Frontiers,
        to: &VersionVector,
    ) -> LoroResult<Vec<(BlockChangeRef, (Counter, Counter), VersionVector)>> {
        let diff = from.diff(to).forward;
        let mut output = Vec::new();
        for node in self.dag.iter_causal(from_frontiers.clone(), diff) {
            let peer = node.data.peer;
            let mut counter = node.data.cnt.max(from.get(&peer).copied().unwrap_or(0));
            let node_end =
                (node.data.cnt + node.data.len as Counter).min(to.get(&peer).copied().unwrap_or(0));
            while counter < node_end {
                let change = self
                    .change_store
                    .get_change_fallible(ID::new(peer, counter))?;
                let change_end = change.ctr_end();
                if change.id.peer != peer || change.id.counter > counter || change_end <= counter {
                    return Err(LoroError::DecodeError(
                        "external causal change header does not cover the requested counter".into(),
                    ));
                }
                let start = counter;
                counter = change_end.min(node_end);
                let mut vv = node
                    .data
                    .vv
                    .get()
                    .cloned()
                    .unwrap_or_else(|| self.dag.ensure_vv_for(&node.data));
                // A persisted block may merge several sequential changes. The
                // tracker must start at the requested slice, not at the merged
                // change's original counter, or checkout will retreat already
                // materialized text before applying the suffix.
                vv.insert(peer, start);
                output.push((change, (start, node_end), VersionVector::from_im_vv(&vv)));
            }
        }
        Ok(output)
    }

    pub fn len_changes(&self) -> usize {
        self.change_store.change_num()
    }

    pub fn diagnose_size(&self) -> SizeInfo {
        let mut total_changes = 0;
        let mut total_ops = 0;
        let mut total_atom_ops = 0;
        let total_dag_node = self.dag.total_parsed_dag_node();
        self.change_store.visit_all_changes(&mut |change| {
            total_changes += 1;
            total_ops += change.ops.len();
            total_atom_ops += change.atom_len();
        });

        println!("total changes: {}", total_changes);
        println!("total ops: {}", total_ops);
        println!("total atom ops: {}", total_atom_ops);
        println!("total dag node: {}", total_dag_node);
        SizeInfo {
            total_changes,
            total_ops,
            total_atom_ops,
            total_dag_node,
        }
    }

    pub(crate) fn iter_changes_peer_by_peer<'a>(
        &'a self,
        from: &VersionVector,
        to: &VersionVector,
    ) -> impl Iterator<Item = BlockChangeRef> + 'a {
        let spans: Vec<_> = from.diff_iter(to).1.collect();
        spans
            .into_iter()
            .flat_map(move |span| self.change_store.iter_changes(span))
    }

    #[allow(dead_code)]
    pub(crate) fn iter_changes_causally_rev<'a>(
        &'a self,
        from: &VersionVector,
        to: &VersionVector,
    ) -> impl Iterator<Item = BlockChangeRef> + 'a {
        MergedChangeIter::new_change_iter_rev(self, from, to)
    }

    pub fn get_timestamp_for_next_txn(&self) -> Timestamp {
        if self.configure.record_timestamp() {
            get_timestamp_now_txn()
        } else {
            0
        }
    }

    #[inline(never)]
    pub(crate) fn idlp_to_id(&self, id: loro_common::IdLp) -> Option<ID> {
        let change = self.change_store.get_change_by_lamport_lte(id)?;

        if change.lamport > id.lamport || change.lamport_end() <= id.lamport {
            return None;
        }

        Some(ID::new(
            change.id.peer,
            (id.lamport - change.lamport) as Counter + change.id.counter,
        ))
    }

    #[allow(unused)]
    pub(crate) fn id_to_idlp(&self, id_start: ID) -> IdLp {
        let change = self.get_change_at(id_start).unwrap();
        let lamport = change.lamport + (id_start.counter - change.id.counter) as Lamport;
        let peer = id_start.peer;
        loro_common::IdLp { peer, lamport }
    }

    /// NOTE: This may return a op that includes the given id, not necessarily start with the given id
    pub(crate) fn get_op_that_includes(&self, id: ID) -> Option<BlockOpRef> {
        let change = self.get_change_at(id)?;
        change.get_op_with_counter(id.counter)
    }

    pub(crate) fn split_span_based_on_deps(&self, id_span: IdSpan) -> Vec<(IdSpan, Frontiers)> {
        let peer = id_span.peer;
        let mut counter = id_span.counter.min();
        let span_end = id_span.counter.norm_end();
        let mut ans = Vec::new();

        while counter < span_end {
            let id = ID::new(peer, counter);
            let node = self.dag.get(id).unwrap();

            let f = if node.cnt == counter {
                node.deps.clone()
            } else if counter > 0 {
                id.inc(-1).into()
            } else {
                unreachable!()
            };

            let cur_end = node.cnt + node.len as Counter;
            let len = cur_end.min(span_end) - counter;
            ans.push((id.to_span(len as usize), f));
            counter += len;
        }

        ans
    }

    #[inline]
    pub fn compact_change_store(&mut self) {
        if self.change_store.is_external() {
            let prior_causal_snapshot = self.external_open_causal_snapshot.clone();
            self.change_store
                .verify_external_predecessor(self.external_metadata_digest)
                .expect("external authenticated predecessor validation failed");
            let causal = self
                .dag
                .encode_external_causal_snapshot()
                .expect("external causal metadata encoding failed");
            let mut causal_snapshot = crate::external_store::decode_causal_snapshot(&causal)
                .expect("external causal metadata validation failed");
            let import_baseline = self
                .refresh_external_import_baseline(self.external_metadata_digest)
                .expect("external import baseline encoding failed");
            self.change_store
                .prepare_external_metadata_semantics(
                    &mut causal_snapshot,
                    prior_causal_snapshot.as_ref(),
                    self.external_metadata_digest,
                )
                .expect("external metadata semantic validation failed");
            let causal = crate::external_store::encode_causal_snapshot(causal_snapshot.clone())
                .expect("external causal metadata proof encoding failed");
            let digest = self
                .change_store
                .try_flush_external_with_causal(
                    self.dag.vv(),
                    self.dag.frontiers(),
                    causal,
                    import_baseline,
                    self.external_greatest_timestamp.unwrap_or_default(),
                )
                .expect("external change store flush failed");
            self.external_metadata_digest = Some(digest);
            self.external_open_causal_snapshot = Some(causal_snapshot);
        } else {
            self.change_store
                .flush_and_compact(self.dag.vv(), self.dag.frontiers());
        }
    }

    pub(crate) fn flush_external_change_store(&mut self) -> Result<[u8; 32], LoroError> {
        if !self.change_store.is_external() {
            return Err(LoroError::ArgErr(
                "document does not use an external change store".into(),
            ));
        }
        self.change_store
            .verify_external_predecessor(self.external_metadata_digest)?;
        let prior_causal_snapshot = self.external_open_causal_snapshot.clone();
        let causal = self.dag.encode_external_causal_snapshot()?;
        let mut causal_snapshot = crate::external_store::decode_causal_snapshot(&causal)?;
        if causal_snapshot.vv != *self.dag.vv()
            || causal_snapshot.frontiers != *self.dag.frontiers()
        {
            return Err(LoroError::DecodeError(
                "external causal metadata does not match the live DAG".into(),
            ));
        }
        let import_baseline =
            self.refresh_external_import_baseline(self.external_metadata_digest)?;
        self.change_store.prepare_external_metadata_semantics(
            &mut causal_snapshot,
            prior_causal_snapshot.as_ref(),
            self.external_metadata_digest,
        )?;
        let causal = crate::external_store::encode_causal_snapshot(causal_snapshot.clone())?;
        let digest = self.change_store.try_flush_external_with_causal(
            self.dag.vv(),
            self.dag.frontiers(),
            causal.clone(),
            import_baseline.clone(),
            self.external_greatest_timestamp.unwrap_or_default(),
        )?;
        self.external_metadata_digest = Some(digest);
        self.external_open_causal_snapshot = Some(causal_snapshot);
        Ok(digest)
    }

    pub(crate) fn evict_external_change_store_cache(&mut self) -> Result<(), LoroError> {
        if !self.change_store.external_version_is(self.dag.vv()) {
            let _ = self.flush_external_change_store()?;
        }
        self.change_store.evict_parsed_cache();
        self.dag.evict_parsed_cache();
        self.history_cache.lock().free_all();
        Ok(())
    }

    fn refresh_external_import_baseline(
        &mut self,
        predecessor_metadata_digest: Option<[u8; 32]>,
    ) -> LoroResult<bytes::Bytes> {
        let mut old_baseline = self.external_import_baseline.take().ok_or_else(|| {
            LoroError::ArgErr("document does not use an external change store".into())
        })?;
        if !old_baseline.arena_coverage_is_current(self) {
            old_baseline.ensure_arena_coverage(self);
        }
        if let Some(encoded) = old_baseline.cached_encoding(self.dag.vv(), self.dag.frontiers()) {
            self.external_import_baseline = Some(old_baseline);
            return Ok(encoded);
        }
        let mut baseline = if old_baseline.matches_version(self.dag.vv(), self.dag.frontiers()) {
            old_baseline
        } else {
            let result = old_baseline.advance(self, self.dag.vv(), self.dag.frontiers(), false);
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    self.external_import_baseline = Some(old_baseline);
                    return Err(error);
                }
            };
            let ExternalImportDiff::Fast { baseline, .. } = result;
            baseline
        };
        if !baseline.arena_coverage_is_current(self) {
            baseline.ensure_arena_coverage(self);
        }
        let snapshot = match baseline.seal_snapshot(&self.arena, predecessor_metadata_digest) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.external_import_baseline = Some(baseline);
                return Err(error);
            }
        };
        let encoded = match crate::external_store::encode_import_baseline(&snapshot) {
            Ok(encoded) => encoded,
            Err(error) => {
                self.external_import_baseline = Some(baseline);
                return Err(error);
            }
        };
        baseline.set_encoded(encoded.clone());
        self.external_import_baseline = Some(baseline);
        Ok(encoded)
    }

    #[inline]
    pub fn change_store_kv_size(&self) -> usize {
        self.change_store.kv_size()
    }

    pub fn encode_change_store(&self) -> bytes::Bytes {
        self.change_store
            .encode_all(self.dag.vv(), self.dag.frontiers())
    }

    pub fn check_dag_correctness(&self) {
        self.dag.check_dag_correctness();
    }

    pub fn shallow_since_vv(&self) -> &ImVersionVector {
        self.dag.shallow_since_vv()
    }

    pub fn shallow_since_frontiers(&self) -> &Frontiers {
        self.dag.shallow_since_frontiers()
    }

    pub fn is_shallow(&self) -> bool {
        !self.dag.shallow_since_vv().is_empty()
    }

    pub fn get_greatest_timestamp(&self, frontiers: &Frontiers) -> Timestamp {
        if frontiers == self.frontiers() {
            if let Some(timestamp) = self.external_greatest_timestamp {
                return timestamp;
            }
        }
        let mut max_timestamp = Timestamp::default();
        for id in frontiers.iter() {
            let change = self.get_change_at(id).unwrap();
            if change.timestamp > max_timestamp {
                max_timestamp = change.timestamp;
            }
        }

        max_timestamp
    }

    pub(crate) fn external_metadata_digest(&self) -> LoroResult<[u8; 32]> {
        self.external_metadata_digest
            .ok_or_else(|| LoroError::DecodeError("external store metadata is missing".into()))
    }
}

#[derive(Debug)]
pub struct SizeInfo {
    pub total_changes: usize,
    pub total_ops: usize,
    pub total_atom_ops: usize,
    pub total_dag_node: usize,
}

pub(crate) fn convert_change_to_remote(
    arena: &SharedArena,
    change: &Change,
) -> Change<RemoteOp<'static>> {
    let mut ops = RleVec::new();
    for op in change.ops.iter() {
        for op in local_op_to_remote(arena, op) {
            ops.push(op);
        }
    }

    Change {
        ops,
        id: change.id,
        deps: change.deps.clone(),
        lamport: change.lamport,
        timestamp: change.timestamp,
        commit_msg: change.commit_msg.clone(),
    }
}

pub(crate) fn local_op_to_remote(
    arena: &SharedArena,
    op: &crate::op::Op,
) -> SmallVec<[RemoteOp<'static>; 1]> {
    let container = arena.get_container_id(op.container).unwrap();
    let mut contents: SmallVec<[_; 1]> = SmallVec::new();
    match &op.content {
        crate::op::InnerContent::List(list) => match list {
            list_op::InnerListOp::Insert { slice, pos } => match container.container_type() {
                loro_common::ContainerType::Text => {
                    let str = arena
                        .slice_str_by_unicode_range(slice.0.start as usize..slice.0.end as usize);
                    contents.push(RawOpContent::List(list_op::ListOp::Insert {
                        slice: ListSlice::RawStr {
                            unicode_len: str.chars().count(),
                            str: Cow::Owned(str),
                        },
                        pos: *pos,
                    }));
                }
                loro_common::ContainerType::List | loro_common::ContainerType::MovableList => {
                    contents.push(RawOpContent::List(list_op::ListOp::Insert {
                        slice: ListSlice::RawData(Cow::Owned(
                            arena.get_values(slice.0.start as usize..slice.0.end as usize),
                        )),
                        pos: *pos,
                    }))
                }
                _ => unreachable!(),
            },
            list_op::InnerListOp::InsertText {
                slice,
                unicode_len: len,
                unicode_start: _,
                pos,
            } => match container.container_type() {
                loro_common::ContainerType::Text => {
                    contents.push(RawOpContent::List(list_op::ListOp::Insert {
                        slice: ListSlice::RawStr {
                            unicode_len: *len as usize,
                            str: Cow::Owned(std::str::from_utf8(slice).unwrap().to_owned()),
                        },
                        pos: *pos as usize,
                    }));
                }
                _ => unreachable!(),
            },
            list_op::InnerListOp::Delete(del) => {
                contents.push(RawOpContent::List(list_op::ListOp::Delete(*del)))
            }
            list_op::InnerListOp::StyleStart {
                start,
                end,
                key,
                value,
                info,
            } => contents.push(RawOpContent::List(list_op::ListOp::StyleStart {
                start: *start,
                end: *end,
                key: key.clone(),
                value: value.clone(),
                info: *info,
            })),
            list_op::InnerListOp::StyleEnd => {
                contents.push(RawOpContent::List(list_op::ListOp::StyleEnd))
            }
            list_op::InnerListOp::Move {
                from,
                elem_id: from_id,
                to,
            } => contents.push(RawOpContent::List(list_op::ListOp::Move {
                from: *from,
                elem_id: *from_id,
                to: *to,
            })),
            list_op::InnerListOp::Set { elem_id, value } => {
                contents.push(RawOpContent::List(list_op::ListOp::Set {
                    elem_id: *elem_id,
                    value: value.clone(),
                }))
            }
        },
        crate::op::InnerContent::Map(map) => {
            let value = map.value.clone();
            contents.push(RawOpContent::Map(crate::container::map::MapSet {
                key: map.key.clone(),
                value,
            }))
        }
        crate::op::InnerContent::Tree(tree) => contents.push(RawOpContent::Tree(tree.clone())),
        crate::op::InnerContent::Future(f) => match f {
            #[cfg(feature = "counter")]
            crate::op::FutureInnerContent::Counter(c) => contents.push(RawOpContent::Counter(*c)),
            FutureInnerContent::Unknown { prop, value } => {
                contents.push(crate::op::RawOpContent::Unknown {
                    prop: *prop,
                    value: (**value).clone(),
                })
            }
        },
    };

    let mut ans = SmallVec::with_capacity(contents.len());
    for content in contents {
        ans.push(RemoteOp {
            container: container.clone(),
            content,
            counter: op.counter,
        })
    }
    ans
}

pub(crate) fn get_timestamp_now_txn() -> Timestamp {
    (get_sys_timestamp() as Timestamp + 500) / 1000
}
