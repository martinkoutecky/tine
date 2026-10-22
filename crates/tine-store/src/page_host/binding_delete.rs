//! Delete at the binding boundary, retaining conflicted input until admission.
use super::*;

impl PageHost {
    /// `page_delete` (§7): the host's delete operation, for the current
    /// window `session` only (S2). Ordinary unsaved input refuses; a conflicted
    /// draft is copied recoverably before deleting the checked disk bytes.
    /// Waiting means the page is busy (retry when clean). `checked` is the
    /// file's bytes the caller's identity check read (GH #620,
    /// `pages::delete_page_expected`); the operation refuses when its own
    /// read differs (Q-P2b-5). An admitted deletion is waited for, at most
    /// `OPERATION_WAIT` (Finding B): `Applied` once its own save
    /// published it; `Refused` when its draft failed (nothing changed);
    /// `Pending` when the page cannot publish without the user (a conflict
    /// or a persistent save error; it completes once it can); `Superseded`
    /// or `Uncertain` otherwise. `checked: None` means the caller's check
    /// found the file absent: see [`Self::delete_absent`].
    pub fn delete(&self, session: u64, page: &PageId, checked: Option<&[u8]>) -> PageOperation {
        let Some(checked) = checked else {
            return self.delete_absent(session, page);
        };
        // A host that ended deletes nothing.
        let Some((key, spelling, ..)) = self.try_identify(page) else {
            return PageOperation::Refused;
        };
        // `open`'s invalid-target refusal (docs/storage-contract.md
        // `page_host::delete`): a path that is not a page file of this graph
        // (outside the root, not a graph text file) deletes nothing. In-scope
        // scenario: web content, a plugin or a frontend defect passing a path.
        if self.store.as_page(&spelling.file()).is_none() {
            return PageOperation::Refused;
        }
        if self.try_register(&key, &spelling).is_none() {
            return PageOperation::Refused;
        }
        let deadline = std::time::Instant::now() + retained::OPERATION_WAIT;
        let (shared, checked) = (Arc::downgrade(&self.driver.shared), checked.to_vec());
        let (store, file) = (self.store.clone(), spelling.file());
        let delete = move |state: &mut State<ProductionIo, SystemClock>| {
            if state.book.session != session {
                return (Disposition::Refused, None);
            }
            let (disposition, reply) = state.progress.with_host(|host| {
                let resolved = match prepare_delete(host, &store, &file, &key, &checked) {
                    Ok(resolved) => resolved,
                    Err(refusal) => return (refusal, None),
                };
                let disposition = host.delete_checked(&key, &checked, resolved);
                let pending = disposition == Disposition::Pending;
                (
                    disposition,
                    pending.then(|| host.register_reply()).flatten(),
                )
            });
            // OG-RULES Rule 8 (A-K1): the operation's write is a deletion.
            if disposition == Disposition::Pending {
                state.book.took([key.clone()], EditKind::DeletePage);
            }
            let slot = reply.map(|id| retained::ReplySlot::new(shared.clone(), id));
            (disposition, slot)
        };
        let deleted = (self.driver.shared).locked_step(Deadline::At(deadline), delete);
        match deleted {
            Ok((Disposition::Pending, Some(slot))) => {
                match self.settle(&slot, Some(session), deadline) {
                    retained::Settled::Applied => PageOperation::Applied,
                    retained::Settled::DraftFailed => PageOperation::Refused,
                    retained::Settled::Unwritten(_) => PageOperation::Pending,
                    retained::Settled::Superseded(_) => PageOperation::Superseded,
                    retained::Settled::Uncertain => PageOperation::Uncertain,
                }
            }
            // Not admitted in time: the page stayed busy (a path lock held).
            Ok((Disposition::Waiting, _)) | Err(Unanswered::Withdrawn) => PageOperation::Waiting,
            Err(Unanswered::Late) => PageOperation::Uncertain,
            _ => PageOperation::Refused,
        }
    }
}

impl PageHost {
    /// `page_delete` of a page whose file is already gone (an external
    /// delete while the page was conflicted): there is nothing to delete, but
    /// a conflicted draft is copied recoverably under the page lock first, so
    /// the window's discard after `Applied` loses nothing (L9 review R1).
    /// `Applied` when no conflicted input is held or its copy is durable;
    /// `Refused` when the file reappeared or the copy failed (input kept);
    /// `Waiting` while the page is busy.
    fn delete_absent(&self, session: u64, page: &PageId) -> PageOperation {
        let Some((key, spelling, ..)) = self.try_identify(page) else {
            return PageOperation::Refused;
        };
        if self.store.as_page(&spelling.file()).is_none() {
            return PageOperation::Refused;
        }
        if self.try_register(&key, &spelling).is_none() {
            return PageOperation::Refused;
        }
        let deadline = std::time::Instant::now() + retained::OPERATION_WAIT;
        let (store, file) = (self.store.clone(), spelling.file());
        let stage = move |state: &mut State<ProductionIo, SystemClock>| {
            if state.book.session != session {
                return Disposition::Refused;
            }
            state.progress.with_host(|host| {
                let Some(input) = host
                    .pages
                    .get(&key)
                    .filter(|page| page.conflict)
                    .map(|page| page.buf.clone())
                else {
                    return Disposition::Applied;
                };
                let keys = BTreeSet::from([key.clone()]);
                if host.worker.is_some() || host.busy(&key) || host.lacks_locks(&keys) {
                    return Disposition::Waiting;
                }
                // Still absent under the page lock, or this is not that case.
                if !host.fs.read_page(&key).is_ok_and(|now| now.is_none()) {
                    return Disposition::Refused;
                }
                match input {
                    Some(input) if store.stage_marker_copy(&file, &input).is_err() => {
                        Disposition::Refused
                    }
                    _ => Disposition::Applied,
                }
            })
        };
        match (self.driver.shared).locked_step(Deadline::At(deadline), stage) {
            Ok(Disposition::Applied) => PageOperation::Applied,
            Ok(Disposition::Waiting) | Err(Unanswered::Withdrawn) => PageOperation::Waiting,
            Err(Unanswered::Late) => PageOperation::Uncertain,
            _ => PageOperation::Refused,
        }
    }
}

fn prepare_delete(
    host: &mut Host<ProductionIo>,
    store: &Store,
    file: &FileId,
    key: &str,
    checked: &[u8],
) -> Result<Option<Page>, Disposition> {
    let Some(original) = host.pages.get(key).filter(|page| page.conflict).cloned() else {
        return Ok(None);
    };
    let keys = BTreeSet::from([key.to_owned()]);
    if host.worker.is_some()
        || host.busy(key)
        || host.allocator_busy()
        || host.order.touches(&keys)
        || host.lacks_locks(&keys)
    {
        return Err(Disposition::Waiting);
    }
    // The identity check must still name the disk file, under the page lock.
    if !host
        .fs
        .read_page(key)
        .is_ok_and(|now| now.as_deref() == Some(checked))
    {
        return Err(Disposition::Refused);
    }
    if let Some(input) = &original.buf {
        if store.stage_marker_copy(file, input).is_err() {
            return Err(Disposition::Refused);
        }
    }
    // Supply a read-only plan against disk. The live host page remains intact
    // until the operation's durable draft applies; there is no temporary discard.
    let disk = Host::<ProductionIo>::initial_page(Some(Arc::from(checked)), original.version);
    Ok(Some(disk))
}
