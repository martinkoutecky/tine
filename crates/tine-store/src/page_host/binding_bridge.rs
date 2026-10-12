//! The driver's sink (STEP3 §5, §3.3): each delivery's edit kinds go to the
//! kinds sink (OG-RULES Rule 8), its claims, publications and evictions apply
//! to the index under the store writer, then its page mail goes to the window.
use super::*;

/// The driver's sink: the publication consumer (§5), then page mail to the
/// window (§3.3).
pub(super) struct Bridge {
    pub(super) store: Arc<Store>,
    pub(super) mail: MailSink,
    pub(super) kinds: KindsSink,
    #[cfg(test)]
    pub(super) index_faults: Arc<std::sync::atomic::AtomicU32>,
}

impl Sink for Bridge {
    /// Under the writer, which orders this against every other index writer
    /// and excludes a running reconcile: claimed keys leave the watcher,
    /// publications apply in host order unless a reservation took their key
    /// (its transaction publishes; checked under the writer, so it cannot
    /// commit before a publication applied here), and evicted keys the
    /// consumer still does not own return to the watcher.
    fn deliver(
        &mut self,
        mut delivery: Delivery,
        owner: &dyn Fn(&str) -> Owner,
        wait: bool,
    ) -> Result<Vec<(Publication, Indexing)>, Delivery> {
        let store = &*self.store;
        let mut results = Vec::new();
        // Before the writer, as the window's own callbacks may take it
        // (`Store::page`), and taken out: a delivery that comes back below
        // reports its kinds once.
        for (key, kinds) in std::mem::take(&mut delivery.kinds) {
            (self.kinds.lock().unwrap())(&key, &kinds);
        }
        let indexes = !delivery.claimed.is_empty()
            || !delivery.publications.is_empty()
            || !delivery.evicted.is_empty();
        // All or nothing: a delivery whose writer is held comes back whole.
        let writer = match indexes {
            false => None,
            true if wait => Some(store.writer.lock().unwrap()),
            true => match store.writer.try_lock() {
                Ok(writer) => Some(writer),
                Err(std::sync::TryLockError::WouldBlock) => return Err(delivery),
                Err(std::sync::TryLockError::Poisoned(error)) => panic!("{error}"),
            },
        };
        if let Some(writer) = writer {
            for (key, _, observed) in delivery.claimed {
                store.watch.hold_observed(key, observed);
            }
            for publication in delivery.publications {
                #[cfg(feature = "test-faults")]
                crate::cost_counters::host_publication_probes(1);
                let started = crate::bench_events::enabled().then(std::time::Instant::now);
                let indexing = match owner(&publication.key) {
                    Owner::Reservation => Indexing::Reserved,
                    #[cfg(test)]
                    _ if self.fault() => Indexing::Failed,
                    _ if index(store, &publication) => Indexing::Indexed,
                    _ => Indexing::Failed,
                };
                if let Some(started) = started {
                    crate::bench_events::mark("index_us", started.elapsed().as_micros() as u64);
                }
                results.push((publication, indexing));
            }
            for (key, _) in delivery.evicted {
                if owner(&key) == Owner::Watcher {
                    store.watch.release_hold(&key);
                }
            }
            store.publish_retired();
            drop(writer);
        }
        for (key, mail, facts) in delivery.mail {
            let mail = page_mail(store, key, mail, facts);
            (self.mail.lock().unwrap())(mail);
        }
        Ok(results)
    }
}

#[cfg(test)]
impl Bridge {
    fn fault(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.index_faults
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .is_ok()
    }
}

/// Publish `publication` to the index (§5): the index part of a
/// transaction's publication (cache, names, references, `GraphChange`),
/// with the R8 Document when it matched. Bytes the index already holds only
/// align the watcher's snapshot. The caller holds the writer. False when
/// the store has closed.
pub(super) fn index(store: &Store, publication: &Publication) -> bool {
    if store.is_closed() {
        return false;
    }
    let graph = &store.graph;
    let path = graph.root.join(&publication.spelling);
    let id = FileId::from(publication.spelling.clone());
    let bytes = publication.bytes.as_deref();
    let rev = bytes.map(FileRev::from_bytes);
    let cached = graph.cached_rev(&path);
    let text = bytes.and_then(|bytes| std::str::from_utf8(bytes).ok());
    // A new hold retired the page's rows (R1): the published snapshot still
    // knows it.
    let known = cached.is_some() || store.snapshot_knows(&path);
    let kind = match (known, bytes) {
        _ if cached.is_some() && text.map(content_rev) == cached => None,
        (false, None) => None,
        (false, Some(_)) => Some(ChangeKind::Created),
        (true, None) => Some(ChangeKind::Removed),
        (true, Some(_)) => Some(ChangeKind::Modified),
    };
    let document = publication.own.as_ref().and_then(|(_, d)| d.as_ref());
    // The owner's bytes become the page's index and its row in one
    // critical section (R1); an unchanged row is not touched.
    let owned = graph.publish_owned(&publication.key, publication.bytes.clone(), document);
    // The read the bytes came from stamped the file first (GH #623).
    let stamps: HashMap<FileId, crate::watch::Stamp> = (publication.stamp.clone())
        .map(|stamp| (id.clone(), stamp))
        .into_iter()
        .collect();
    let Some(kind) = kind else {
        graph.transaction_clear_page_marker(&path);
        let raced = store.watch.note_own_observed(&[(id, rev)], &stamps);
        store.watch.reconcile_raced(&raced);
        return true;
    };
    let before = graph.cache_generation();
    let entry = match &owned {
        Ok(entry) => entry.clone(),
        Err(()) => graph.transaction_publish_page_inner(
            &path,
            bytes,
            document,
            kind != ChangeKind::Modified,
            false,
        ),
    };
    if graph.cache_generation() == before {
        graph.transaction_bump_generation();
    }
    let files = vec![(id.clone(), kind, rev)];
    let mut observations = PublishedObservations {
        stamps,
        ..Default::default()
    };
    if publication.own.is_some() {
        if let Some(entry) = entry {
            observations.entries.insert(id, entry);
        }
        store.publish_own(files, observations);
    } else {
        // Named from the owner's bytes (none for a removal, which the
        // snapshot names by its previous name); with no owner, by path.
        let entry = match &owned {
            Ok(_) => entry,
            Err(()) => entry.or_else(|| graph.entry_for_path(&path)),
        };
        let pages = entry
            .map(|entry| (id, entry.kind, entry.name))
            .into_iter()
            .collect();
        store.publish_transaction_change(Origin::External, files, pages, observations);
    }
    true
}

fn page_mail(store: &Store, key: PageKey, mail: Mail, facts: MailFacts) -> PageMail {
    let graph = &store.graph;
    let page = mail.page.map(|page| MailPage {
        version: page.version,
        conflict: page.conflict,
        risk: page.risk,
        disk: page.obs.as_ref().map(DiskToken::of),
        text: match &page.buf {
            _ if !facts.content => MailText::Unchanged {
                rev: page.buf.as_deref().map(FileRev::from_bytes),
            },
            None => MailText::NoFile,
            Some(bytes) => match crate::bench_events::mail_parse(&facts.spelling, || {
                graph.page_dto_for_bytes(&graph.root.join(&facts.spelling), bytes)
            }) {
                Ok(Some(dto)) => MailText::Page { dto: Box::new(dto) },
                Ok(None) => MailText::Unreadable {
                    message: "not a page path".into(),
                },
                Err(error) => MailText::Unreadable {
                    message: error.to_string(),
                },
            },
        },
    });
    let answer = mail.answer.map(|answer| MailAnswer {
        id: answer.id,
        version: answer.version,
        took: answer.took,
        outcome: match facts.refused {
            Some(reason) => AnswerOutcome::Refused { reason },
            None => AnswerOutcome::Applied,
        },
    });
    PageMail {
        session: facts.session,
        key,
        page,
        answer,
        notice: MailNotice {
            failures: facts.notice.failures,
            save_error: facts.notice.save_error,
            operation: facts.notice.operation,
            os_error: facts.notice.os_error,
            draft_error: facts.notice.draft_error,
            dropped: facts.notice.dropped,
            conflict_reported: facts.notice.conflict_reported,
            custody_error: !facts.notice.custody_error.is_empty(),
            index_error: facts.binding.index_error,
            observe_error: facts.binding.observe_error,
            twin: facts.binding.twin,
        },
    }
}
