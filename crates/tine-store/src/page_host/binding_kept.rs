//! Kept originals at the binding boundary (GH #540): the recovery panel's
//! list, its Copy original and its Restore.
use super::retained::{self, HostState, ReplySlot, Settled, Step};
use super::*;
use io::{KeptOriginal, RestoreOutcome, RestoreRefusal};
use tine_core::model::PageKind;

impl PageHost {
    /// `page_kept_originals`: every original a cancelled rename kept
    /// (`Order::kept`), for window `session`; None for another session (its
    /// rows belong to the session that listed them). One walk over the
    /// durable pairs under the state mutex; then, per row, its created
    /// page's name from the kept text (`Graph::created_entry`: a decode and
    /// a preamble scan, no read, no copy of the text).
    pub fn kept_originals(&self, session: u64) -> Option<Vec<KeptOriginal>> {
        let read = move |state: &HostState| {
            let host = &state.progress.host;
            (state.book.session == session).then(|| {
                (host.order.kept())
                    .map(|(.., p)| (p.source.clone(), p.origin, host.fs.spelling(&p.source), p.preimage.clone()))
                    .collect::<Vec<_>>()
            })
        };
        let rows = self.driver.shared.read(read).ok()??;
        let graph = &self.store.graph;
        let rows = rows.into_iter().map(|(key, origin, path, text)| {
            let entry = graph.created_entry(&graph.root.join(&path), text.as_deref());
            KeptOriginal {
                key,
                origin,
                path,
                kind: entry.as_ref().map_or(PageKind::Page, |entry| entry.kind),
                name: entry.map(|entry| entry.name),
                len: text.map_or(0, |text| text.len()),
            }
        });
        Some(rows.collect())
    }

    /// `page_kept_text`: the kept original (`key`, `origin`) as text, for
    /// the panel's Copy original; None for another session or an original no
    /// longer kept. Text that is not UTF-8 is copied lossily.
    pub fn kept_text(&self, session: u64, key: &str, origin: u64) -> Option<String> {
        let key = key.to_owned();
        let read = move |state: &HostState| {
            let order = &state.progress.host.order;
            let at = order.kept_at(&key, origin).filter(|_| state.book.session == session)?;
            let text = order.kept_pair(&at)?.preimage.clone();
            Some(String::from_utf8_lossy(text.as_deref().unwrap_or_default()).into_owned())
        };
        self.driver.shared.read(read).ok()?
    }

    /// `page_restore_kept`: put the kept original (`key`, `origin`) back as its
    /// file, for window `session` only, while no window holds the page. The
    /// host creates the absent file through the page's own draft and
    /// guarded save (`Host::restore_source`), and the restored file reaches
    /// windows as any new file does. Refused, changing nothing, for another
    /// session or a host that is down (`Unavailable`), an original no
    /// longer kept, a file the graph's settings no longer make a page, a
    /// window holding the page or the page its text names, unsaved input,
    /// a file at the slot or another file that is this page, or a failed
    /// check. Admission takes the page's path lock and re-checks all of it
    /// there, waiting while the page is busy (`bounded_locked`, at most
    /// `OPERATION_WAIT` behind a rename). An admitted restore is then waited
    /// for as develop's host operations are (`settle`): `Restored` only once
    /// the host's own save published it and the index has it; `Superseded`
    /// once a Use Disk took other bytes first; `Unwritten` while it cannot
    /// be saved; `Uncertain` past the bound. The write is recorded as a
    /// page creation (OG-RULES Rule 8).
    pub fn restore_kept(&self, session: u64, key: &str, origin: u64) -> RestoreOutcome {
        let refused = RestoreOutcome::Refused;
        let (path, preimage) = {
            let key = key.to_owned();
            let read = move |state: &HostState| {
                let order = &state.progress.host.order;
                let at = (order.kept_at(&key, origin)).ok_or(RestoreRefusal::NotKept)?;
                let text = order.kept_pair(&at).map(|p| p.preimage.clone());
                Ok((state.progress.host.fs.spelling(&key), text.flatten()))
            };
            match self.driver.shared.read(read) {
                Ok(Ok(found)) => found,
                Ok(Err(refusal)) => return refused(refusal),
                Err(_) => return refused(RestoreRefusal::Unavailable),
            }
        };
        let graph = &self.store.graph;
        let Some(entry) = graph.created_entry(&graph.root.join(&path), preimage.as_deref()) else {
            return refused(RestoreRefusal::Ineligible);
        };
        // The page its text names, when that is another file (`pages/a.md`
        // titled `Named`): a readable file claiming the name is its twin,
        // and a window holding that name's absent page holds it. Read here,
        // never on the driver, without waiting for the graph's load; the
        // create guard checks the claimant again at the save.
        let mut named = None;
        if entry.kind == PageKind::Page {
            if let Some(claimant) = self.store.move_claimant(&entry.name, entry.kind) {
                if claimant.path != graph.root.join(&path) {
                    let file = graph.rel_path(&claimant.path);
                    return refused(RestoreRefusal::Twin { file });
                }
            }
            let proposed = crate::store::proposed_page_id(&self.store.config().config, &entry.name);
            if proposed.as_str() != path {
                named = self.try_identify(&proposed).map(|(key, ..)| key);
            }
        }
        let deadline = tine_core::clock::Instant::now() + retained::OPERATION_WAIT;
        let shared = Arc::downgrade(&self.driver.shared);
        let (key, name) = (key.to_owned(), entry.name);
        let keys = BTreeSet::from([key.clone()]);
        let step = move |state: &mut HostState| {
            if state.book.session != session {
                return Step::Done(Err(RestoreRefusal::Unavailable));
            }
            let host = &state.progress.host;
            let Some(at) = host.order.kept_at(&key, origin) else {
                return Step::Done(Err(RestoreRefusal::NotKept));
            };
            let held = |key: &PageKey| host.subscriptions.contains(key);
            if held(&key) || named.as_ref().is_some_and(held) {
                let name = name.clone();
                return Step::Done(Err(RestoreRefusal::Held { name }));
            }
            if !retained::unsaved(host, &keys).is_empty() {
                return Step::Done(Err(RestoreRefusal::Unsaved));
            }
            match state.progress.with_host(|host| host.restore_source(&key, &at)) {
                Ok(Disposition::Applied) => {}
                Ok(Disposition::Disabled) => return Step::Done(Err(RestoreRefusal::Unavailable)),
                Ok(_) => {
                    let renaming = state.progress.host.renaming(&keys);
                    return Step::Wait { renaming };
                }
                Err(refusal) => return Step::Done(Err(refusal)),
            }
            // Its caller's slot, answered at the version just installed:
            // `settle` reads its witness, index and supersession.
            let host = &mut state.progress.host;
            let version = host.pages[&key].version;
            let id = host.order.allocate(host.incarnation);
            host.order.register(id);
            host.order.answer(id, OperationReply::Applied(BTreeMap::from([(key.clone(), version)])));
            state.book.took([key.clone()], EditKind::CreatePage);
            Step::Done(Ok(ReplySlot::new(shared.clone(), id)))
        };
        let slot = match self.bounded_locked(deadline, step) {
            Ok(Ok(slot)) => slot,
            Ok(Err(refusal)) => return refused(refusal),
            Err(_) => return refused(RestoreRefusal::Unavailable),
        };
        match self.settle(&slot, Some(session), deadline) {
            Settled::Applied => RestoreOutcome::Restored,
            Settled::Superseded(_) => RestoreOutcome::Superseded,
            Settled::Unwritten(page) => {
                // One report of a save that cannot finish: the editor's,
                // when a window holds the page now.
                let read = move |state: &HostState| state.progress.host.subscriptions.contains(&page);
                let held = self.driver.shared.read(read).unwrap_or(false);
                RestoreOutcome::Unwritten { held }
            }
            Settled::DraftFailed | Settled::Uncertain => RestoreOutcome::Uncertain,
        }
    }
}
