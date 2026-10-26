//! Case-only moves on folding volumes (host-only-writer S3). Content is a
//! normal drafted HostOp, followed by a byte-preserving driver command.
use super::driver::{Deadline, Unanswered, Wait};
use super::retained::OPERATION_WAIT;
use super::*;
use std::time::Duration;

impl PageHost {
    pub(super) fn apply_spellings(
        &self,
        mut op: HostOp,
        spellings: Vec<(PageId, PageId, PageKey, PageId)>,
    ) -> Result<(), ApplyRefusal> {
        let config = op.view.as_ref().map(|(_, config)| config.clone());
        let mut moves = Vec::new();
        let mut seen = BTreeSet::new();
        let mut planned = BTreeMap::new();
        for (page, effect, expected) in &op.pages {
            if planned
                .insert(page.clone(), (effect.clone(), expected.clone()))
                .is_some()
            {
                return Err(ApplyRefusal::Refused);
            }
        }
        let mut destinations = BTreeSet::new();
        let tx = self.store.transaction(None);
        // Collapse each folded move onto its single existing key. A
        // case-only namespace rename needs the same primitive per entry.
        // Duplicate pages and unrelated malformed effects still refuse in
        // the ordinary apply path, before either content or entry changes.
        for (dst, src, key, spelling) in spellings {
            if src != spelling || src == dst || !seen.insert(key.clone()) {
                return Err(ApplyRefusal::Changed);
            }
            let (
                Some((Effect::Delete, Expected::Bytes(old))),
                Some((Effect::Write(new), Expected::Absent)),
            ) = (planned.get(&src), planned.get(&dst))
            else {
                return Err(ApplyRefusal::Refused);
            };
            let (old, new) = (old.clone(), new.clone());
            // Validate the actual directory alias before changing any text.
            // A separately listed symlink/hard link is never a spelling move.
            tx.admission_slot(&dst.file(), None, Some(&src.file()))
                .map_err(|_| ApplyRefusal::Changed)?;
            planned.insert(
                src.clone(),
                (Effect::Write(new.clone()), Expected::Bytes(old)),
            );
            planned.remove(&dst);
            destinations.insert(dst.clone());
            moves.push((src, dst, key, new));
        }
        op.pages = planned
            .into_iter()
            .map(|(page, (effect, expected))| (page, effect, expected))
            .collect();
        op.moves.retain(|(dst, _)| !destinations.contains(dst));
        op.creation_checks
            .retain(|(p, ..)| !destinations.contains(p));
        self.apply(op)?;
        for (src, dst, key, bytes) in moves {
            self.move_spelling(src, dst, key, bytes, config.clone())?;
        }
        Ok(())
    }

    pub(super) fn move_spelling(
        &self,
        src: PageId,
        dst: PageId,
        key: PageKey,
        new: Arc<[u8]>,
        config: Option<Arc<tine_core::config::Config>>,
    ) -> Result<(), ApplyRefusal> {
        // No buffer/lock handover to a caller: the driver checks clean and
        // expected bytes again under the key's path lock. A later window
        // edit, external editor or sync delivery is never overwritten.
        let store = self.store.clone();
        let lock = store.graph.page_lock(&store.graph.root.join(dst.as_str()));
        let deadline = std::time::Instant::now() + OPERATION_WAIT;
        let moved = move |state: &mut State<ProductionIo, SystemClock>| {
            // Never stall the driver on another thread's writer. The path
            // loop releases its locks on Until and tries again later.
            let Ok(_writer) = store.writer.try_lock() else {
                return Wait::Until(Some(std::time::Instant::now() + Duration::from_millis(10)));
            };
            let host = &mut state.progress.host;
            if !host.alive || !host.admission_open {
                return Wait::Ready(Err(ApplyRefusal::Uncertain));
            }
            if config
                .as_ref()
                .is_some_and(|config| !Arc::ptr_eq(config, &store.config().config))
                || host.fs.spelling(&key) != src.as_str()
            {
                return Wait::Ready(Err(ApplyRefusal::Changed));
            }
            let keys = BTreeSet::from([key.clone()]);
            if let Some(page) = super::retained::unsaved(host, &keys).into_values().next() {
                return Wait::Ready(Err(ApplyRefusal::Unsaved(page)));
            }
            if host.busy(&key) || host.order.touches(&keys) || host.lacks_locks(&keys) {
                return Wait::Until(Some(std::time::Instant::now() + Duration::from_millis(10)));
            }
            // This helper performs the existing guarded no-replace / folded
            // alias rename plus directory sync. It never rewrites bytes.
            let (moved, mut result) =
                store
                    .transaction(None)
                    .respell_entry(&src.file(), &dst.file(), &new);
            if moved {
                host.fs.spell(&key, dst.as_str());
                host.locks.insert(key.clone(), lock.clone());
                store
                    .graph
                    .respelled(&key, &store.graph.root.join(src.as_str()));
                // A clean, unopened page can have returned to the watcher
                // after its content operation. Publish that entry's new path
                // synchronously too, through the same final-file observation
                // as transaction moves, before answering the caller.
                if store
                    .graph
                    .disk_sourced(&store.graph.root.join(dst.as_str()))
                {
                    let published = store.transaction(None).publish_respelled_entry(
                        &src.file(),
                        &dst.file(),
                        &new,
                    );
                    if result.is_ok() {
                        result = published;
                    }
                }
                store.publish_retired();
            }
            Wait::Ready(result.map_err(|why| match why {
                Why::Conflict { .. } => ApplyRefusal::Changed,
                _ => ApplyRefusal::Uncertain,
            }))
        };
        match self
            .driver
            .shared
            .locked_wait(Deadline::At(deadline), moved)
        {
            Ok(result) => result,
            // Content already published: an expired/ended spelling command
            // cannot promise that nothing changed.
            Err(Unanswered::Withdrawn | Unanswered::Late | Unanswered::Ended) => {
                Err(ApplyRefusal::Uncertain)
            }
        }
    }
}
