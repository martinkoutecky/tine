//! Read-only projection of the transaction's own preflight into a host op.
use super::*;
use crate::{HostEffect, HostExpected, HostOp, Input};

fn planning_lock<T>(
    lock: &std::sync::Mutex<T>,
    deadline: std::time::Instant,
) -> Result<std::sync::MutexGuard<'_, T>, Why> {
    loop {
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => panic!("poisoned planning lock"),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if std::time::Instant::now() >= deadline {
            // One deadline mapping for every PagePlan caller, including
            // preflights that do not go through retained feature writes.
            return Err(failed(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Tine is still finishing another change to these pages. Try again in a moment.",
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_deadline_reports_retryable_busy_in_user_words() {
        let lock = std::sync::Mutex::new(());
        let _held = lock.lock().unwrap();
        let why = planning_lock(&lock, std::time::Instant::now()).unwrap_err();
        let Why::Failed(error) = why else {
            panic!("a planning deadline must be retryable: {why:?}");
        };
        assert_eq!(error.kind, io::ErrorKind::WouldBlock);
        assert_eq!(
            error.message,
            "Tine is still finishing another change to these pages. Try again in a moment."
        );
    }
}

impl PagePlan<'_> {
    /// Prepare this transaction as a page-host operation without publishing
    /// or writing anything. Runs commit's preflight and stages its guarded old
    /// bytes and serialized effects. Non-page effects belong in a separate,
    /// guarded transaction before or after the operation. A byte-preserving
    /// move from a non-page source stages only its page destination; the caller
    /// must retire the source after Applied in its own revision-guarded transaction.
    /// Other non-page steps are refused.
    /// Writer/path acquisition that reaches the planning deadline returns
    /// `Why::Failed` with `WouldBlock` and a shared user-facing retry message.
    /// Callers preserve that busy condition rather than report a failed write.
    pub fn plan(mut self) -> Result<HostOp, Why> {
        self.for_host = true;
        self.await_reference_publication();
        let deadline = std::time::Instant::now() + crate::PageHost::PLANNING_WAIT;
        let _writer = planning_lock(&self.store.writer, deadline)?;
        let names = self.preflight_names().map_err(|(_, why)| why)?;
        let locks: Vec<_> = names
            .iter()
            .map(|file| {
                self.store
                    .graph
                    .page_lock(&self.store.graph.root.join(file.as_str()))
            })
            .collect();
        let _guards: Vec<_> = locks
            .iter()
            .map(|lock| planning_lock(lock, deadline))
            .collect::<Result<_, _>>()?;
        let prepared = self.prepare_steps().map_err(|(_, why)| why)?;
        let mut op = HostOp {
            separate_case_entries: false,
            inputs: None,
            pages: Vec::new(),
            kinds: self.kinds.clone(),
            policy: Input::Refuse,
            moves: Vec::new(),
            publications: None,
            creation_checks: Vec::new(),
            view: self.view.clone(),
            saved: Vec::new(),
        };
        for (step, prepared) in self.steps.iter().zip(prepared) {
            let page = self.store.as_page(&prepared.src);
            let non_page_move = page.is_none() && matches!(step, Step::Move { renames: None, .. });
            if page.is_none() && !non_page_move {
                return Err(Why::Refused(Refusal::InvalidTarget(
                    "a host operation requires page steps only".into(),
                )));
            }
            let old = prepared
                .old
                .map(|bytes| std::sync::Arc::<[u8]>::from(bytes));
            let expected = old
                .clone()
                .map_or(HostExpected::Absent, HostExpected::Bytes);
            let mut new = prepared.new;
            if let Step::Create { content, .. } | Step::Unique { content, .. } = step {
                new = Some(validation::page_content_bytes(&prepared.src, content)?.into_owned());
            }
            if let Some(dst) = prepared.dst {
                let destination = self
                    .store
                    .as_page(&dst)
                    .ok_or_else(|| Why::Refused(Refusal::InvalidTarget(dst.as_str().into())))?;
                let bytes = new.ok_or_else(|| {
                    Why::Refused(Refusal::InvalidTarget(
                        "a host operation requires staged page bytes".into(),
                    ))
                })?;
                op.creation_checks
                    .push((destination.clone(), None, Some(prepared.src)));
                op.pages.push((
                    destination.clone(),
                    HostEffect::Write(bytes.into()),
                    HostExpected::Absent,
                ));
                if let Some(page) = page {
                    op.moves.push((destination, page.clone()));
                    op.pages.push((page, HostEffect::Delete, expected));
                }
            } else {
                let page = page.expect("non-page steps refused above");
                if matches!(step, Step::Expect { .. }) {
                    return Err(Why::Refused(Refusal::InvalidTarget(
                        "a host operation requires effects, not read-only expectations".into(),
                    )));
                }
                if old.is_none() && new.is_some() {
                    let name = match step {
                        Step::Save { doc, .. } => Some(doc.name.clone()),
                        _ => None,
                    };
                    op.creation_checks.push((page.clone(), name, None));
                }
                if prepared.opaque_rev.is_some() {
                    return Err(Why::Refused(Refusal::InvalidTarget(
                        "a host operation requires staged page bytes".into(),
                    )));
                }
                if let (Some((path, document)), Some(bytes)) = (prepared.parsed, new.as_deref()) {
                    // Named from the bytes as a read of them would be (`Store::page_of`),
                    // at the path serialization already validated.
                    let entry = std::str::from_utf8(bytes)
                        .ok()
                        .and_then(|text| self.store.graph.page_entry_in(&path, Some(text)));
                    if let Some(entry) = entry {
                        let (name, kind) = (entry.name, entry.kind);
                        op.saved.push((
                            page.clone(),
                            crate::HostSaved {
                                name,
                                kind,
                                document,
                            },
                        ));
                    }
                }
                let effect =
                    new.map_or(HostEffect::Delete, |bytes| HostEffect::Write(bytes.into()));
                op.pages.push((page, effect, expected));
            }
        }
        Ok(op)
    }

    /// Live preflight slot checks, returning a move's local absence proof.
    /// Moves exempt their source; CreateNew checks its DTO name for unreadable owners.
    pub(crate) fn admission_slot(
        &self,
        file: &FileId,
        name: Option<&str>,
        source: Option<&FileId>,
    ) -> Result<bool, Why> {
        if let Some(source) = source {
            let absent = self.available_move_destination(source, file)?;
            self.twin_with_absence(file, Some(source), absent)?;
            Ok(absent)
        } else if let Some(name) = name {
            self.page_create_checks(file, name).map(|()| false)
        } else {
            self.raw_create_slot(file).map(|()| false)
        }
    }
}
