//! Case-only moves on folding volumes (host-only-writer S3). Content is a
//! normal drafted HostOp, preceded by a byte-preserving driver command.
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
        let mut moves = Vec::new();
        let mut copies = Vec::new();
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
        let mut in_a_move = BTreeSet::new();
        for (dst, src) in &op.moves {
            if !matches!(planned.get(src), Some((Effect::Delete, Expected::Bytes(_))))
                || !matches!(planned.get(dst), Some((Effect::Write(_), _)))
                || !in_a_move.insert(src.clone())
                || !in_a_move.insert(dst.clone())
            {
                return Err(ApplyRefusal::Refused);
            }
        }
        let mut destinations = BTreeSet::new();
        let tx = self.store.page_plan(None);
        // Collapse each folded move onto its single existing key. A
        // case-only namespace rename needs the same primitive per entry.
        // Duplicate pages and malformed move shapes refuse before either
        // content or entry changes. Content repeats its full admission later.
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
            if old != new {
                copies.push((src.clone(), old.clone()));
            }
            // Validate the actual directory alias before changing any text.
            // A separately listed symlink/hard link is never a spelling move.
            tx.admission_slot(&dst.file(), None, Some(&src.file()))
                .map_err(|_| ApplyRefusal::Changed)?;
            planned.insert(
                src.clone(),
                (Effect::Write(new.clone()), Expected::Bytes(old.clone())),
            );
            planned.remove(&dst);
            destinations.insert(dst.clone());
            moves.push((src, dst, key, old));
        }
        op.pages = planned
            .into_iter()
            .map(|(page, (effect, expected))| (page, effect, expected))
            .collect();
        op.moves.retain(|(dst, _)| !destinations.contains(dst));
        op.creation_checks
            .retain(|(p, ..)| !destinations.contains(p));
        // Keep the transaction rename's old-byte recovery copy before the
        // entry move or ordinary content operation can change live state.
        // This also checks clean input and every expected page, even when
        // no recovery copy is needed. Content still gets full admission.
        self.retain_spelling_bytes(&op, copies)?;
        let mut moved_pages = BTreeMap::new();
        for (src, dst, key, bytes) in moves {
            let unchanged = planned_bytes_equal(&op, &src, &bytes);
            let rev = self
                .move_spelling(
                    src.clone(),
                    dst.clone(),
                    key.clone(),
                    bytes,
                    op.view.clone(),
                    op.publications.clone().map(|sink| (sink, unchanged)),
                )
                .map_err(|_| ApplyRefusal::SpellingIncomplete)?;
            // Only our own publication advances this view. A delivery
            // between commands still refuses at the next full view check.
            if let Some((view, _)) = &mut op.view {
                *view = rev;
            }
            // The entry move already published unchanged destination bytes.
            // Do not submit them again: admission must exclude every path
            // already in this target's receipt set, including our own.
            if unchanged && op.publications.is_some() {
                self.report_spelling_kinds(&key, &op);
                op.pages.retain(|(page, ..)| page != &src);
            }
            moved_pages.insert(src, dst);
        }
        for (page, ..) in &mut op.pages {
            if let Some(dst) = moved_pages.get(page) {
                *page = dst.clone();
            }
        }
        if op.pages.is_empty() {
            Ok(())
        } else {
            self.apply(op).map_err(|_| ApplyRefusal::SpellingIncomplete)
        }
    }

    /// An entry-only completion has no later content publication to deliver
    /// its kinds. Borrow the operation unchanged, and as the bridge does,
    /// call the sink outside driver/writer locks: it may query the store.
    fn report_spelling_kinds(&self, key: &PageKey, op: &HostOp) {
        (self.launch.kinds.lock().unwrap())(key, &op.kinds);
    }

    fn retain_spelling_bytes(
        &self,
        op: &HostOp,
        copies: Vec<(PageId, Arc<[u8]>)>,
    ) -> Result<(), ApplyRefusal> {
        let pages: Vec<_> = op.pages.iter().map(|(p, ..)| p.clone()).collect();
        if op.policy == Input::Flush {
            self.flush_input(&pages)?;
        }
        let mut keys = BTreeSet::new();
        for page in pages {
            let (key, spelling, _, proved) =
                self.try_identify(&page).ok_or(ApplyRefusal::Refused)?;
            if self.store.page_spot(&spelling).is_err() {
                return Err(ApplyRefusal::Refused);
            }
            if !proved {
                return Err(ApplyRefusal::Changed);
            }
            self.try_register(&key, &spelling)
                .ok_or(ApplyRefusal::Refused)?;
            if !keys.insert(key) {
                return Err(ApplyRefusal::Refused);
            }
        }
        let view = op.view.clone();
        let expected = op.pages.clone();
        let store = self.store.clone();
        let deadline = std::time::Instant::now() + OPERATION_WAIT;
        let retain = move |state: &mut State<ProductionIo, SystemClock>| {
            let Ok(_writer) = store.writer.try_lock() else {
                return Wait::Until(Some(std::time::Instant::now() + Duration::from_millis(10)));
            };
            let host = &mut state.progress.host;
            if !host.alive || !host.admission_open {
                return Wait::Ready(Err(ApplyRefusal::Refused));
            }
            if view.as_ref().is_some_and(|(rev, config)| {
                store.changes.rev() != *rev || !Arc::ptr_eq(config, &store.config().config)
            }) {
                return Wait::Ready(Err(ApplyRefusal::Changed));
            }
            if let Some(page) = super::retained::unsaved(host, &keys).into_values().next() {
                return Wait::Ready(Err(ApplyRefusal::Unsaved(page)));
            }
            if host.order.touches(&keys)
                || keys.iter().any(|key| host.busy(key))
                || host.lacks_locks(&keys)
            {
                return Wait::Until(Some(std::time::Instant::now() + Duration::from_millis(10)));
            }
            let tx = store.page_plan(None);
            for (page, _, expected) in &expected {
                let bytes = match expected {
                    Expected::Bytes(bytes) => Some(bytes.as_ref()),
                    Expected::Absent => None,
                };
                if tx.verify_spelling_input(&page.file(), bytes).is_err() {
                    return Wait::Ready(Err(ApplyRefusal::Changed));
                }
            }
            // Retain only verified source bytes, using the existing durable
            // no-replace trash-copy recipe. Refusal later may leave a harmless
            // recovery copy, but never changes live content.
            for (src, bytes) in &copies {
                if let Err(why) = tx.retain_spelling_bytes(&src.file(), bytes) {
                    return Wait::Ready(Err(match why {
                        Why::Conflict { .. } => ApplyRefusal::Changed,
                        _ => ApplyRefusal::Refused,
                    }));
                }
            }
            Wait::Ready(Ok(()))
        };
        match self
            .driver
            .shared
            .locked_wait(Deadline::At(deadline), retain)
        {
            Ok(result) => result,
            Err(Unanswered::Withdrawn) => Err(ApplyRefusal::Busy),
            Err(Unanswered::Late | Unanswered::Ended) => Err(ApplyRefusal::Uncertain),
        }
    }

    pub(super) fn move_spelling(
        &self,
        src: PageId,
        dst: PageId,
        key: PageKey,
        bytes: Arc<[u8]>,
        view: Option<(crate::GraphRev, Arc<tine_core::config::Config>)>,
        publications: Option<(Arc<Mutex<BTreeSet<PageId>>>, bool)>,
    ) -> Result<crate::GraphRev, ApplyRefusal> {
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
            // Entry publication precedes content admission. Apply the same
            // retry exclusion here, under the publication-owning state,
            // before this command can move either spelling.
            if publications.as_ref().is_some_and(|(sink, _)| {
                let paths = sink.lock().unwrap_or_else(|p| p.into_inner());
                paths.contains(&src) || paths.contains(&dst)
            }) {
                return Wait::Ready(Err(ApplyRefusal::Changed));
            }
            if view.as_ref().is_some_and(|(rev, config)| {
                store.changes.rev() != *rev || !Arc::ptr_eq(config, &store.config().config)
            }) || host.fs.spelling(&key) != src.as_str()
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
                    .page_plan(None)
                    .respell_entry(&src.file(), &dst.file(), &bytes);
            if moved {
                // The entry move publishes the source's absence. Its
                // destination already has U only for a byte-preserving move.
                // Record inside the driver, including a late command or a
                // directory-sync error after the entry was moved.
                if let Some((sink, unchanged)) = &publications {
                    let mut paths = sink.lock().unwrap_or_else(|p| p.into_inner());
                    paths.insert(src.clone());
                    if *unchanged {
                        paths.insert(dst.clone());
                    }
                }
                host.fs.spell(&key, dst.as_str());
                host.locks.insert(key.clone(), lock.clone());
                store
                    .graph
                    .respelled(&key, &store.graph.root.join(src.as_str()));
                // A clean, unopened page can belong to the watcher.
                // Publish that entry's new path with its original bytes
                // synchronously too, through the same final-file observation
                // as transaction moves, before answering the caller.
                if store
                    .graph
                    .disk_sourced(&store.graph.root.join(dst.as_str()))
                {
                    let published = store.page_plan(None).publish_respelled_entry(
                        &src.file(),
                        &dst.file(),
                        &bytes,
                    );
                    if result.is_ok() {
                        result = published;
                    }
                }
                store.publish_retired();
            }
            Wait::Ready(
                result
                    .map(|()| store.changes.rev())
                    .map_err(|why| match why {
                        Why::Conflict { .. } => ApplyRefusal::Changed,
                        _ => ApplyRefusal::Uncertain,
                    }),
            )
        };
        match self
            .driver
            .shared
            .locked_wait(Deadline::At(deadline), moved)
        {
            Ok(result) => result,
            // An entry move may be live: an expired/ended spelling command
            // cannot promise that nothing changed.
            Err(Unanswered::Withdrawn | Unanswered::Late | Unanswered::Ended) => {
                Err(ApplyRefusal::Uncertain)
            }
        }
    }
}

fn planned_bytes_equal(op: &HostOp, src: &PageId, bytes: &[u8]) -> bool {
    op.pages.iter().any(|(page, effect, _)| {
        page == src && matches!(effect, Effect::Write(new) if new.as_ref() == bytes)
    })
}
