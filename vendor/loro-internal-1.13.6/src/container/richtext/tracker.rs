use std::ops::ControlFlow;

use generic_btree::{
    rle::{HasLength as _, Sliceable},
    LeafIndex,
};
use loro_common::{CompactId, Counter, HasId, HasIdSpan, IdFull, IdSpan, Lamport, PeerID, ID};
use rle::HasLength as _;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{cursor::AbsolutePosition, VersionVector};

use self::{crdt_rope::CrdtRope, id_to_cursor::IdToCursor};

use super::{
    fugue_span::{FugueSpan, Status},
    RichtextChunk,
};

mod crdt_rope;
mod id_to_cursor;
pub(crate) use crdt_rope::CrdtRopeDelta;

#[derive(Debug)]
pub(crate) struct Tracker {
    applied_vv: VersionVector,
    current_vv: VersionVector,
    rope: CrdtRope,
    id_to_cursor: IdToCursor,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new_with_unknown()
    }
}

pub(super) const UNKNOWN_PEER_ID: PeerID = u64::MAX;

#[derive(Debug, Serialize, Deserialize)]
struct ExternalTrackerSnapshot {
    applied_vv: VersionVector,
    current_vv: VersionVector,
    spans: Vec<ExternalTrackerSpan>,
    deletes: Vec<ExternalTrackerDelete>,
}

#[derive(Serialize)]
struct ExternalTrackerSnapshotRef<'a> {
    applied_vv: &'a VersionVector,
    current_vv: &'a VersionVector,
    spans: &'a [ExternalTrackerSpan],
    deletes: &'a [ExternalTrackerDelete],
}

#[derive(Debug, Serialize, Deserialize)]
struct ExternalTrackerSpan {
    id_peer: PeerID,
    id_counter: Counter,
    id_lamport: Lamport,
    real_id: Option<(PeerID, Counter)>,
    future: bool,
    delete_times: i16,
    origin_left: Option<(PeerID, Counter)>,
    origin_right: Option<(PeerID, Counter)>,
    len: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExternalTrackerDelete {
    op_peer: PeerID,
    op_counter: Counter,
    target_peer: PeerID,
    target_start: Counter,
    target_end: Counter,
}

impl Tracker {
    pub fn new_with_unknown() -> Self {
        let mut this = Self {
            rope: CrdtRope::new(),
            id_to_cursor: IdToCursor::default(),
            applied_vv: Default::default(),
            current_vv: Default::default(),
        };

        let result = this.rope.tree.push(FugueSpan {
            content: RichtextChunk::new_unknown(u32::MAX / 4),
            id: IdFull::new(UNKNOWN_PEER_ID, 0, 0),
            real_id: None,
            status: Status::default(),
            diff_status: None,
            origin_left: None,
            origin_right: None,
        });
        this.id_to_cursor.insert_without_split(
            ID::new(UNKNOWN_PEER_ID, 0),
            id_to_cursor::Cursor::new_insert(result.leaf, u32::MAX as usize / 4),
        );
        this
    }

    #[allow(unused)]
    fn new() -> Self {
        Self {
            rope: CrdtRope::new(),
            id_to_cursor: IdToCursor::default(),
            applied_vv: Default::default(),
            current_vv: Default::default(),
        }
    }

    pub(crate) fn new_empty_external() -> Self {
        Self::new()
    }

    pub(crate) fn encode_external_snapshot(&self) -> Result<Vec<u8>, String> {
        self.check();
        let spans = self
            .rope
            .tree()
            .iter()
            .copied()
            .map(|span| {
                let len = u32::try_from(span.content.len())
                    .map_err(|_| "rich-text tracker span is too large".to_string())?;
                Ok(ExternalTrackerSpan {
                    id_peer: span.id.peer,
                    id_counter: span.id.counter,
                    id_lamport: span.id.lamport,
                    real_id: span.real_id.map(encode_compact_id),
                    future: span.status.future,
                    delete_times: span.status.delete_times,
                    origin_left: span.origin_left.map(encode_compact_id),
                    origin_right: span.origin_right.map(encode_compact_id),
                    len,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let deletes = self
            .id_to_cursor
            .external_delete_cursors()?
            .into_iter()
            .map(|(op, target)| ExternalTrackerDelete {
                op_peer: op.peer,
                op_counter: op.counter,
                target_peer: target.peer,
                target_start: target.counter.start,
                target_end: target.counter.end,
            })
            .collect::<Vec<_>>();
        postcard::to_allocvec(&ExternalTrackerSnapshotRef {
            applied_vv: &self.applied_vv,
            current_vv: &self.current_vv,
            spans: &spans,
            deletes: &deletes,
        })
        .map_err(|_| "failed to encode rich-text tracker".to_string())
    }

    pub(crate) fn encode_external_compact_snapshot(
        vv: &VersionVector,
        id: IdFull,
        len: u32,
    ) -> Result<Vec<u8>, String> {
        let encoded = ExternalTrackerSpan {
            id_peer: id.peer,
            id_counter: id.counter,
            id_lamport: id.lamport,
            real_id: Some((id.peer, id.counter)),
            future: false,
            delete_times: 0,
            origin_left: None,
            origin_right: None,
            len,
        };
        postcard::to_allocvec(&ExternalTrackerSnapshotRef {
            applied_vv: vv,
            current_vv: vv,
            spans: std::slice::from_ref(&encoded),
            deletes: &[],
        })
        .map_err(|_| "failed to encode compact rich-text tracker".to_string())
    }

    pub(crate) fn decode_external_snapshot(bytes: &[u8]) -> Result<Self, String> {
        let snapshot = decode_and_validate_external_snapshot(bytes)?;

        let mut tracker = Self::new();
        let mut cursors = Vec::new();
        for encoded in snapshot.spans {
            let span = FugueSpan {
                id: IdFull::new(encoded.id_peer, encoded.id_counter, encoded.id_lamport),
                real_id: encoded.real_id.map(decode_compact_id).transpose()?,
                status: Status {
                    future: encoded.future,
                    delete_times: encoded.delete_times,
                },
                diff_status: None,
                origin_left: encoded.origin_left.map(decode_compact_id).transpose()?,
                origin_right: encoded.origin_right.map(decode_compact_id).transpose()?,
                content: RichtextChunk::new_unknown(encoded.len),
            };
            let len = span.content.len();
            let id = span.id.id();
            let result = tracker.rope.tree.push(span);
            cursors.push((id, id_to_cursor::Cursor::new_insert(result.leaf, len)));
        }
        for encoded in snapshot.deletes {
            let op = ID::new(encoded.op_peer, encoded.op_counter);
            let target = IdSpan::new(
                encoded.target_peer,
                encoded.target_start,
                encoded.target_end,
            );
            cursors.push((op, id_to_cursor::Cursor::new_delete(target)));
        }
        cursors.sort_unstable_by_key(|(id, _)| *id);
        for pair in cursors.windows(2) {
            if pair[0].0.peer == pair[1].0.peer
                && pair[0].0.counter + pair[0].1.rle_len() as Counter > pair[1].0.counter
            {
                return Err("rich-text tracker cursor ranges overlap".to_string());
            }
        }
        for (id, cursor) in cursors {
            tracker.id_to_cursor.insert_without_split(id, cursor);
        }
        tracker.applied_vv = snapshot.applied_vv;
        tracker.current_vv = snapshot.current_vv;
        tracker.check();
        Ok(tracker)
    }

    pub(crate) fn validate_external_snapshot(
        bytes: &[u8],
        baseline_vv: &VersionVector,
    ) -> Result<bool, String> {
        let snapshot = decode_and_validate_external_snapshot(bytes)?;
        if !baseline_vv.includes_vv(&snapshot.applied_vv) {
            return Err("rich-text tracker version exceeds its import baseline".to_string());
        }
        Ok(is_canonical_compact_birth(&snapshot, baseline_vv))
    }

    #[inline]
    pub fn all_vv(&self) -> &VersionVector {
        &self.applied_vv
    }

    #[inline]
    pub fn current_vv(&self) -> &VersionVector {
        &self.current_vv
    }

    pub(crate) fn insert(&mut self, mut op_id: IdFull, mut pos: usize, mut content: RichtextChunk) {
        // trace!(
        //     "TrackerInsert op_id = {:#?}, pos = {:#?}, content = {:#?}",
        //     op_id,
        //     &pos,
        //     &content
        // );
        // tracing::span!(tracing::Level::INFO, "TrackerInsert");
        if let ControlFlow::Break(_) =
            self.skip_applied(op_id.id(), content.len(), |applied_counter_end| {
                // the op is partially included, need to slice the content
                let start = (applied_counter_end - op_id.counter) as usize;
                op_id.lamport += (applied_counter_end - op_id.counter) as Lamport;
                op_id.counter = applied_counter_end;
                pos += start;
                content = content.slice(start..);
            })
        {
            return;
        }

        // {
        //     tracing::span!(tracing::Level::INFO, "before insert {} pos={}", op_id, pos);
        //     debug_log::debug_dbg!(&self);
        // }
        self._insert(pos, content, op_id);
    }

    fn _insert(&mut self, pos: usize, content: RichtextChunk, op_id: IdFull) {
        let result = self.rope.insert(
            pos,
            FugueSpan {
                content,
                id: op_id,
                real_id: if op_id.peer == UNKNOWN_PEER_ID {
                    None
                } else {
                    Some(op_id.id().try_into().unwrap())
                },
                status: Status::default(),
                diff_status: None,
                origin_left: None,
                origin_right: None,
            },
            |id| self.id_to_cursor.get_insert(id).unwrap(),
        );
        self.id_to_cursor.insert(
            op_id.id(),
            id_to_cursor::Cursor::new_insert(result.leaf, content.len()),
        );
        self.update_insert_by_split(&result.splitted.arr);

        let end_id = op_id.inc(content.len() as Counter);
        self.current_vv.extend_to_include_end_id(end_id.id());
        self.applied_vv.extend_to_include_end_id(end_id.id());
    }

    fn update_insert_by_split(&mut self, split: &[LeafIndex]) {
        match split.len() {
            0 => {}
            1 => {
                let new_leaf_idx = split[0];
                let leaf = self.rope.tree().get_elem(new_leaf_idx).unwrap();
                self.id_to_cursor
                    .update_insert(leaf.id_span(), new_leaf_idx);
            }
            _ => {
                let mut updates = Vec::with_capacity(split.len());
                for &new_leaf_idx in split {
                    let leaf = self.rope.tree().get_elem(new_leaf_idx).unwrap();
                    updates.push((leaf.id_span(), new_leaf_idx));
                }
                self.id_to_cursor.update_insert_batch(&mut updates);
            }
        }
    }

    /// Delete the element from pos..pos+len
    ///
    /// If `reverse` is true, the deletion happens from the end of the range to the start.
    /// So the first op is the one that deletes element at `pos+len-1`, the last op
    /// is the one that deletes element at `pos`.
    ///
    /// - op_id: the first op that perform the deletion
    /// - target_start_id: in the target deleted span, it's the first id of the span
    /// - pos: the start pos of the deletion in the target span
    /// - len: the length of the deletion span
    /// - reverse: if true, the kth op delete the last kth element of the span
    pub(crate) fn delete(
        &mut self,
        mut op_id: ID,
        mut target_start_id: ID,
        pos: usize,
        mut len: usize,
        reverse: bool,
    ) {
        if let ControlFlow::Break(_) = self.skip_applied(op_id, len, |applied_counter_end: i32| {
            // the op is partially included, need to slice the op
            let start = (applied_counter_end - op_id.counter) as usize;
            op_id.counter = applied_counter_end;
            if !reverse {
                target_start_id = target_start_id.inc(start as i32);
            }
            // Okay, this looks pretty weird, but it's correct.
            // If it's reverse, we don't need to change the target_start_id, because target_start_id always pointing towards the
            // leftmost element of the span. After applying the initial part of the deletion, which starts from the right side,
            // the target_start_id will be still pointing towards the same leftmost element, thus no need to change.
            len -= start;
            // If reverse, don't need to change the pos, because it's deleting backwards.
            // If not reverse, we don't need to change the pos either, because the `start` chars after it are already deleted
        }) {
            return;
        }

        // tracing::info!("after forwarding pos={} len={}", pos, len);

        self._delete(target_start_id, pos, len, reverse, op_id);
    }

    fn _delete(&mut self, target_start_id: ID, pos: usize, len: usize, reverse: bool, op_id: ID) {
        let mut ans = Vec::new();
        let split = self
            .rope
            .delete(target_start_id, pos, len, reverse, &mut |span| {
                let mut id_span = span.id_span();
                if reverse {
                    id_span.reverse();
                }
                ans.push(id_span);
            });

        let mut cur_id = op_id;
        for id_span in ans {
            let len = id_span.atom_len();
            self.id_to_cursor
                .insert(cur_id, id_to_cursor::Cursor::Delete(id_span));
            cur_id = cur_id.inc(len as Counter);
        }

        debug_assert_eq!(cur_id.counter - op_id.counter, len as Counter);
        for s in split {
            self.update_insert_by_split(&s.arr);
        }

        let end_id = op_id.inc(len as Counter);
        self.current_vv.extend_to_include_end_id(end_id);
        self.applied_vv.extend_to_include_end_id(end_id);
    }

    fn skip_applied(
        &mut self,
        op_id: ID,
        len: usize,
        mut f: impl FnMut(Counter),
    ) -> ControlFlow<()> {
        let last_id = op_id.inc(len as Counter - 1);
        let applied_counter_end = self.applied_vv.get(&last_id.peer).copied().unwrap_or(0);
        if applied_counter_end > op_id.counter {
            if !self.current_vv.includes_id(last_id) {
                // PERF: may be slow
                let mut updates = Default::default();
                let cnt_start = self.current_vv.get(&op_id.peer).copied().unwrap_or(0);
                self.forward(
                    IdSpan::new(op_id.peer, cnt_start, op_id.counter + len as Counter),
                    &mut updates,
                );
                self.batch_update(updates, false);
            }

            if applied_counter_end > last_id.counter {
                self.current_vv.extend_to_include_last_id(last_id);
                return ControlFlow::Break(());
            }

            f(applied_counter_end);
        }
        ControlFlow::Continue(())
    }

    /// Internally it's delete at the src and insert at the dst.
    ///
    /// But it needs special behavior for id_to_cursor data structure
    ///
    #[instrument(skip(self))]
    pub(crate) fn move_item(
        &mut self,
        op_id: IdFull,
        deleted_id: ID,
        from_pos: usize,
        to_pos: usize,
    ) {
        if let ControlFlow::Break(_) = self.skip_applied(op_id.id(), 1, |_| unreachable!()) {
            return;
        }

        // We record the **fake** id of the deleted item, and store it in the `id_to_cursor`.
        // This is because when we retreat, we need to know the **fake** id of the deleted item,
        // so that we can look up the insert pos in `id_to_cursor`
        //
        // > `id_to_cursor` only stores the mappings from **fake** insert id to the leaf index.
        // > **Fake** means the id may be a temporary placeholder, created with UNKNOWN_PEER_ID.
        let mut fake_delete_id = None;
        let split = self.rope.delete(deleted_id, from_pos, 1, false, &mut |s| {
            debug_assert_eq!(s.rle_len(), 1);
            fake_delete_id = Some(s.id.id());
        });

        for s in split {
            self.update_insert_by_split(&s.arr);
        }

        let result = self.rope.insert(
            to_pos,
            FugueSpan {
                // we need to use the special move content to avoid
                // its merging with other [FugueSpan], which will make
                // id_to_cursor need to track its split.
                // It would be much harder to implement correctly
                content: RichtextChunk::new_move(),
                id: op_id,
                real_id: if op_id.peer == UNKNOWN_PEER_ID {
                    None
                } else {
                    Some(op_id.id().try_into().unwrap())
                },
                status: Status::default(),
                diff_status: None,
                origin_left: None,
                origin_right: None,
            },
            |id| self.id_to_cursor.get_insert(id).unwrap(),
        );
        self.update_insert_by_split(&result.splitted.arr);

        self.id_to_cursor.insert(
            op_id.id(),
            id_to_cursor::Cursor::new_move(result.leaf, fake_delete_id.unwrap()),
        );

        let end_id = op_id.inc(1);
        self.current_vv.extend_to_include_end_id(end_id.id());
        self.applied_vv.extend_to_include_end_id(end_id.id());
    }

    #[inline]
    pub(crate) fn checkout(&mut self, vv: &VersionVector) {
        self._checkout(vv, false);
    }

    pub(crate) fn checkout_processed(&mut self, vv: &VersionVector) {
        self.applied_vv.extend_to_include_vv(vv.iter());
        self.checkout(vv);
    }

    fn _checkout(&mut self, vv: &VersionVector, on_diff_status: bool) {
        // tracing::info!("Checkout to {:?} from {:?}", vv, self.current_vv);
        if on_diff_status {
            self.rope.clear_diff_status();
        }

        let current_vv = std::mem::take(&mut self.current_vv);
        let (retreat, forward) = current_vv.diff_iter(vv);
        let mut updates = Vec::new();
        for span in retreat {
            for c in self.id_to_cursor.iter(span) {
                match c {
                    id_to_cursor::IterCursor::Insert { leaf, id_span } => {
                        updates.push(crdt_rope::LeafUpdate {
                            leaf,
                            id_span,
                            set_future: Some(true),
                            delete_times_diff: 0,
                        })
                    }
                    id_to_cursor::IterCursor::Delete(span) => {
                        for to_del in self.id_to_cursor.iter(span) {
                            match to_del {
                                id_to_cursor::IterCursor::Insert { leaf, id_span } => {
                                    updates.push(crdt_rope::LeafUpdate {
                                        leaf,
                                        id_span,
                                        set_future: None,
                                        delete_times_diff: -1,
                                    })
                                }
                                id_to_cursor::IterCursor::Move {
                                    from_id: _,
                                    to_leaf,
                                    new_op_id,
                                } => updates.push(crdt_rope::LeafUpdate {
                                    leaf: to_leaf,
                                    id_span: new_op_id.to_span(1),
                                    set_future: None,
                                    delete_times_diff: -1,
                                }),
                                _ => unreachable!(),
                            }
                        }
                    }
                    id_to_cursor::IterCursor::Move {
                        from_id: from,
                        to_leaf: to,
                        new_op_id: op_id,
                    } => {
                        let mut visited = false;
                        for to_del in self.id_to_cursor.iter(IdSpan::new(
                            from.peer,
                            from.counter,
                            from.counter + 1,
                        )) {
                            visited = true;

                            match to_del {
                                id_to_cursor::IterCursor::Move {
                                    from_id: _,
                                    to_leaf: to,
                                    new_op_id: op_id,
                                } => updates.push(crdt_rope::LeafUpdate {
                                    leaf: to,
                                    id_span: op_id.to_span(1),
                                    set_future: None,
                                    delete_times_diff: -1,
                                }),
                                // Un delete the from
                                id_to_cursor::IterCursor::Insert { leaf, id_span } => {
                                    debug_assert_eq!(id_span.atom_len(), 1);
                                    debug_assert_eq!(id_span.counter.start, from.counter);
                                    updates.push(crdt_rope::LeafUpdate {
                                        leaf,
                                        id_span,
                                        set_future: None,
                                        delete_times_diff: -1,
                                    })
                                }
                                _ => unreachable!(),
                            }
                        }
                        assert!(visited);
                        // insert the new
                        updates.push(crdt_rope::LeafUpdate {
                            leaf: to,
                            id_span: IdSpan::new(op_id.peer, op_id.counter, op_id.counter + 1),
                            set_future: Some(true),
                            delete_times_diff: 0,
                        });
                    }
                }
            }
        }

        for span in forward {
            self.forward(span, &mut updates);
        }

        if !on_diff_status {
            self.current_vv = vv.clone();
        } else {
            self.current_vv = current_vv;
        }

        self.batch_update(updates, on_diff_status);
    }

    fn batch_update(&mut self, updates: Vec<crdt_rope::LeafUpdate>, on_diff_status: bool) {
        let leaf_indexes = self.rope.update(updates, on_diff_status);
        self.update_insert_by_split(&leaf_indexes);
    }

    fn forward(&mut self, span: loro_common::IdSpan, updates: &mut Vec<crdt_rope::LeafUpdate>) {
        for c in self.id_to_cursor.iter(span) {
            match c {
                id_to_cursor::IterCursor::Insert { leaf, id_span } => {
                    updates.push(crdt_rope::LeafUpdate {
                        leaf,
                        id_span,
                        set_future: Some(false),
                        delete_times_diff: 0,
                    })
                }
                id_to_cursor::IterCursor::Delete(span) => {
                    for to_del in self.id_to_cursor.iter(span) {
                        match to_del {
                            id_to_cursor::IterCursor::Insert { leaf, id_span } => {
                                updates.push(crdt_rope::LeafUpdate {
                                    leaf,
                                    id_span,
                                    set_future: None,
                                    delete_times_diff: 1,
                                })
                            }
                            id_to_cursor::IterCursor::Move {
                                from_id: _,
                                to_leaf,
                                new_op_id,
                            } => updates.push(crdt_rope::LeafUpdate {
                                leaf: to_leaf,
                                id_span: new_op_id.to_span(1),
                                set_future: None,
                                delete_times_diff: 1,
                            }),
                            _ => unreachable!(),
                        }
                    }
                }
                id_to_cursor::IterCursor::Move {
                    from_id: from,
                    to_leaf: to,
                    new_op_id: op_id,
                } => {
                    for to_del in self.id_to_cursor.iter(IdSpan::new(
                        from.peer,
                        from.counter,
                        from.counter + 1,
                    )) {
                        match to_del {
                            id_to_cursor::IterCursor::Move {
                                from_id: _,
                                to_leaf: to,
                                new_op_id: op_id,
                            } => updates.push(crdt_rope::LeafUpdate {
                                leaf: to,
                                id_span: op_id.to_span(1),
                                set_future: None,
                                delete_times_diff: 1,
                            }),
                            id_to_cursor::IterCursor::Insert { leaf, id_span } => {
                                updates.push(crdt_rope::LeafUpdate {
                                    leaf,
                                    id_span,
                                    set_future: None,
                                    delete_times_diff: 1,
                                })
                            }
                            _ => unreachable!(),
                        }
                    }

                    updates.push(crdt_rope::LeafUpdate {
                        leaf: to,
                        id_span: IdSpan::new(op_id.peer, op_id.counter, op_id.counter + 1),
                        set_future: Some(false),
                        delete_times_diff: 0,
                    });
                }
            }
        }
    }

    #[allow(unused)]
    pub(crate) fn check(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        self.check_vv_correctness();
        self.check_id_to_cursor_insertions_correctness();
    }

    fn check_vv_correctness(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        for span in self.rope.tree().iter() {
            if span.id.peer == UNKNOWN_PEER_ID {
                continue;
            }

            let id_span = span.id_span();
            assert!(self.all_vv().includes_id(id_span.id_last()));
            if span.status.future {
                assert!(
                    !self.current_vv.includes_id(id_span.id_start()),
                    "future span {id_span:?} is included by tracker version {:?}",
                    self.current_vv
                );
            } else {
                assert!(
                    self.current_vv.includes_id(id_span.id_last()),
                    "present span {id_span:?} exceeds tracker version {:?}",
                    self.current_vv
                );
            }
        }
    }

    // It can only check the correctness of insertions in id_to_cursor.
    // The deletions are not checked.
    fn check_id_to_cursor_insertions_correctness(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        for rope_elem in self.rope.tree().iter() {
            let id_span = rope_elem.id_span();
            let leaf_from_start = self
                .id_to_cursor
                .get_insert(id_span.id_start())
                .unwrap_or_else(|| panic!("tracker cursor is missing span start {id_span:?}"));
            let leaf_from_last = self
                .id_to_cursor
                .get_insert(id_span.id_last())
                .unwrap_or_else(|| panic!("tracker cursor is missing span end {id_span:?}"));
            assert_eq!(leaf_from_start, leaf_from_last);
            let elem_from_id_to_cursor_map = self.rope.tree().get_elem(leaf_from_last).unwrap();
            assert_eq!(rope_elem, elem_from_id_to_cursor_map);
        }

        for content in self.id_to_cursor.iter_all() {
            match content {
                id_to_cursor::IterCursor::Insert { leaf, id_span } => {
                    let leaf = self.rope.tree().get_elem(leaf).unwrap();
                    let span = leaf.id_span();
                    span.contains(id_span.id_start());
                    span.contains(id_span.id_last());
                }
                id_to_cursor::IterCursor::Delete(_) => {}
                id_to_cursor::IterCursor::Move { .. } => {}
            }
        }
    }

    pub(crate) fn get_target_id_latest_index_at_new_version(
        &self,
        id: ID,
    ) -> Option<AbsolutePosition> {
        // TODO: PERF this can be sped up from O(n) to O(log(n)) but I'm not sure if it's worth it
        let mut index = 0;
        for span in self.rope.tree.iter() {
            let is_activated = span.is_activated_in_diff();
            let span_id = span.real_id();
            let id_span = span_id.to_span(span.rle_len());
            if id_span.contains(id) {
                if is_activated {
                    index += (id.counter - id_span.counter.start) as usize;
                }

                return Some(AbsolutePosition {
                    pos: index,
                    side: if is_activated {
                        crate::cursor::Side::Middle
                    } else {
                        crate::cursor::Side::Left
                    },
                });
            }

            if is_activated {
                index += span.rle_len();
            }
        }

        None
    }

    // #[tracing::instrument(skip(self), level = "info")]
    pub(crate) fn diff(
        &mut self,
        from: &VersionVector,
        to: &VersionVector,
    ) -> impl Iterator<Item = CrdtRopeDelta> + '_ {
        // tracing::info!("Init: {:#?}, ", &self);
        self._checkout(from, false);
        self._checkout(to, true);
        // self.id_to_cursor.diagnose();
        // tracing::trace!("Trace::diff {:#?}, ", &self);

        self.rope.get_diff()
    }

    pub(crate) fn commit_diff_target(&mut self, to: &VersionVector) {
        self.checkout(to);
        self.rope.clear_diff_status();
    }
}

fn is_canonical_compact_birth(
    snapshot: &ExternalTrackerSnapshot,
    baseline_vv: &VersionVector,
) -> bool {
    if &snapshot.applied_vv != baseline_vv
        || &snapshot.current_vv != baseline_vv
        || !snapshot.deletes.is_empty()
        || snapshot.spans.len() != 1
    {
        return false;
    }

    let span = &snapshot.spans[0];
    span.len > 0
        && span.real_id == Some((span.id_peer, span.id_counter))
        && !span.future
        && span.delete_times == 0
        && span.origin_left.is_none()
        && span.origin_right.is_none()
}

fn decode_and_validate_external_snapshot(bytes: &[u8]) -> Result<ExternalTrackerSnapshot, String> {
    let snapshot: ExternalTrackerSnapshot = postcard::from_bytes(bytes)
        .map_err(|_| "invalid rich-text tracker encoding".to_string())?;
    if !snapshot.applied_vv.includes_vv(&snapshot.current_vv) {
        return Err("rich-text tracker current version is invalid".to_string());
    }

    let mut cursors = Vec::with_capacity(snapshot.spans.len() + snapshot.deletes.len());
    for encoded in &snapshot.spans {
        if encoded.len == 0 {
            return Err("invalid rich-text tracker span".to_string());
        }
        if encoded.id_peer == UNKNOWN_PEER_ID {
            return Err(
                "external rich-text tracker cannot contain UNKNOWN placeholder spans".to_string(),
            );
        }
        encoded.real_id.map(decode_compact_id).transpose()?;
        encoded.origin_left.map(decode_compact_id).transpose()?;
        encoded.origin_right.map(decode_compact_id).transpose()?;
        let len = Counter::try_from(encoded.len)
            .map_err(|_| "rich-text tracker span is too large".to_string())?;
        let last_counter = encoded
            .id_counter
            .checked_add(len - 1)
            .ok_or_else(|| "rich-text tracker span counter overflows".to_string())?;
        let last = ID::new(encoded.id_peer, last_counter);
        if !snapshot.applied_vv.includes_id(last) {
            return Err("rich-text tracker span exceeds its version".to_string());
        }
        if encoded.future == snapshot.current_vv.includes_id(last) {
            return Err("rich-text tracker span status is inconsistent".to_string());
        }
        cursors.push((ID::new(encoded.id_peer, encoded.id_counter), len));
    }

    let mut previous_delete = None;
    for encoded in &snapshot.deletes {
        let op = ID::new(encoded.op_peer, encoded.op_counter);
        if previous_delete.is_some_and(|previous| previous >= op) {
            return Err("rich-text tracker delete cursors are not canonical".to_string());
        }
        let target = IdSpan::new(
            encoded.target_peer,
            encoded.target_start,
            encoded.target_end,
        );
        let len = Counter::try_from(target.atom_len())
            .ok()
            .filter(|len| *len > 0)
            .ok_or_else(|| "rich-text tracker delete cursor exceeds its version".to_string())?;
        let last_counter = encoded
            .op_counter
            .checked_add(len - 1)
            .ok_or_else(|| "rich-text tracker delete cursor counter overflows".to_string())?;
        if !snapshot
            .applied_vv
            .includes_id(ID::new(encoded.op_peer, last_counter))
        {
            return Err("rich-text tracker delete cursor exceeds its version".to_string());
        }
        previous_delete = Some(op);
        cursors.push((op, len));
    }
    cursors.sort_unstable_by_key(|(id, _)| *id);
    for pair in cursors.windows(2) {
        if pair[0].0.peer == pair[1].0.peer
            && pair[0]
                .0
                .counter
                .checked_add(pair[0].1)
                .is_none_or(|end| end > pair[1].0.counter)
        {
            return Err("rich-text tracker cursor ranges overlap".to_string());
        }
    }
    Ok(snapshot)
}

fn encode_compact_id(id: CompactId) -> (PeerID, Counter) {
    let id = id.to_id();
    (id.peer, id.counter)
}

fn decode_compact_id((peer, counter): (PeerID, Counter)) -> Result<CompactId, String> {
    ID::new(peer, counter)
        .try_into()
        .map_err(|_| "rich-text tracker compact id is invalid".to_string())
}

#[cfg(test)]
mod test {
    use crate::{container::richtext::RichtextChunk, vv};
    use generic_btree::rle::HasLength;

    use super::*;
    use std::time::Instant;

    #[test]
    fn test_len() {
        let mut t = Tracker::new();
        t.insert(IdFull::new(1, 0, 0), 0, RichtextChunk::new_text(0..2));
        assert_eq!(t.rope.len(), 2);
        t.checkout(&Default::default());
        assert_eq!(t.rope.len(), 0);
        t.insert(IdFull::new(2, 0, 0), 0, RichtextChunk::new_text(2..4));
        let v = vv!(1 => 2, 2 => 2);
        t.checkout(&v);
        assert_eq!(&t.applied_vv, &v);
        assert_eq!(t.rope.len(), 4);
    }

    #[test]
    fn test_retreat_and_forward_delete() {
        let mut t = Tracker::new();
        t.insert(IdFull::new(1, 0, 0), 0, RichtextChunk::new_text(0..10));
        t.delete(ID::new(2, 0), ID::NONE_ID, 0, 10, true);
        t.checkout(&vv!(1 => 10, 2=>5));
        assert_eq!(t.rope.len(), 5);
        t.checkout(&vv!(1 => 10, 2=>0));
        assert_eq!(t.rope.len(), 10);
        t.checkout(&vv!(1 => 10, 2=>10));
        assert_eq!(t.rope.len(), 0);
        t.checkout(&vv!(1 => 10, 2=>0));
        assert_eq!(t.rope.len(), 10);
    }

    #[test]
    fn repeated_tail_splits_keep_id_to_cursor_consistent() {
        let mut t = Tracker::new();
        t.insert(IdFull::new(1, 0, 0), 0, RichtextChunk::new_text(0..300));

        for (i, pos) in [100, 201, 252, 278].into_iter().enumerate() {
            let op_id = IdFull::new(2, i as Counter, i as Lamport);
            let start = 1000 + i as u32;
            t.insert(op_id, pos, RichtextChunk::new_text(start..start + 1));
        }

        t.check();
    }

    #[test]
    fn test_checkout_in_doc_with_del_span() {
        let mut t = Tracker::new();
        t.insert(IdFull::new(1, 0, 0), 0, RichtextChunk::new_text(0..10));
        t.delete(ID::new(2, 0), ID::NONE_ID, 0, 10, false);
        t.checkout(&vv!(1 => 10, 2=>4));
        let v: Vec<FugueSpan> = t.rope.tree().iter().copied().collect();
        assert_eq!(v.len(), 2);
        assert!(!v[0].is_activated());
        assert_eq!(v[0].rle_len(), 4);
        assert!(v[1].is_activated());
        assert_eq!(v[1].rle_len(), 6);
    }

    #[test]
    #[ignore]
    fn perf_update_insert_by_split_quadratic() {
        // Run with:
        // cargo test -p loro-internal perf_update_insert_by_split_quadratic -- --ignored --nocapture
        const CHUNK_LEN: usize = 256;
        let fragments: usize = std::env::var("LORO_PERF_FRAGMENTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8192);
        const PEER_A: PeerID = 1;
        const PEER_B: PeerID = 2;

        let doc_len = CHUNK_LEN * fragments;

        let mut t = Tracker::new();
        t.insert(
            IdFull::new(PEER_A, 0, 0),
            0,
            RichtextChunk::new_text(0..doc_len as u32),
        );
        t.id_to_cursor.diagnose();

        let start = Instant::now();
        let expected_fragment_updates = (fragments as u64) * ((fragments - 1) as u64) / 2;

        for i in 0..(fragments - 1) {
            let pos = (i + 1) * CHUNK_LEN + i;
            let op_id = IdFull::new(PEER_B, i as Counter, i as Lamport);
            let chunk = RichtextChunk::new_text(
                (doc_len as u32 + i as u32)..(doc_len as u32 + i as u32 + 1),
            );
            t.insert(op_id, pos, chunk);
        }

        let elapsed = start.elapsed();
        let before_vv = vv!(PEER_A => doc_len as Counter);
        let after_vv = vv!(PEER_A => doc_len as Counter, PEER_B => (fragments - 1) as Counter);
        let diff_start = Instant::now();
        let diff_len = t.diff(&before_vv, &after_vv).count();
        let diff_elapsed = diff_start.elapsed();
        assert_eq!(t.rope.tree().iter().count(), 1 + 2 * (fragments - 1));
        println!(
            "perf_update_insert_by_split_quadratic: doc_len={}, fragments={}, expected_fragment_updates={}, insert_elapsed={:?}, diff_items={}, diff_elapsed={:?}",
            doc_len, fragments, expected_fragment_updates, elapsed, diff_len, diff_elapsed
        );
    }

    #[test]
    #[ignore]
    fn perf_update_insert_by_split_quadratic_unknown() {
        // Run with:
        // LORO_PERF_FRAGMENTS=8192 cargo test -p loro-internal perf_update_insert_by_split_quadratic_unknown -- --ignored --nocapture
        const CHUNK_LEN: usize = 256;
        let fragments: usize = std::env::var("LORO_PERF_FRAGMENTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8192);
        const PEER_B: PeerID = 2;

        let doc_len = CHUNK_LEN * fragments;

        let mut t = Tracker::new_with_unknown();
        t.checkout(&vv!());
        t.id_to_cursor.diagnose();

        let start = Instant::now();
        for i in 0..(fragments - 1) {
            let pos = (i + 1) * CHUNK_LEN + i;
            let op_id = IdFull::new(PEER_B, i as Counter, i as Lamport);
            let chunk = RichtextChunk::new_text(
                (doc_len as u32 + i as u32)..(doc_len as u32 + i as u32 + 1),
            );
            t.insert(op_id, pos, chunk);
        }

        let elapsed = start.elapsed();
        println!(
            "perf_update_insert_by_split_quadratic_unknown: doc_len={}, fragments={}, elapsed={:?}",
            doc_len, fragments, elapsed
        );
    }
}
