//! Read-only projection of the transaction's own preflight into a host op.
use super::*;
use crate::{HostEffect, HostExpected, HostOp, Input};

impl Transaction<'_> {
    /// Prepare this transaction as a page-host operation without publishing
    /// or writing anything. Runs commit's preflight and stages its guarded old
    /// bytes and serialized effects. Non-page effects belong in a separate,
    /// guarded transaction before or after the operation. A byte-preserving
    /// move from a non-page source stages only its page destination; the caller
    /// must retire the source after Applied in its own revision-guarded transaction.
    /// Other non-page steps are refused.
    pub fn plan(self) -> Result<HostOp, Why> {
        self.await_reference_publication();
        let _writer = self.store.writer.lock().unwrap();
        let names = self.preflight_names().map_err(|(_, why)| why)?;
        let locks: Vec<_> = names
            .iter()
            .map(|file| {
                self.store
                    .graph
                    .page_lock(&self.store.graph.root.join(file.as_str()))
            })
            .collect();
        let _guards: Vec<_> = locks.iter().map(|lock| lock.lock().unwrap()).collect();
        let prepared = self.prepare_steps().map_err(|(_, why)| why)?;
        let mut op = HostOp {
            pages: Vec::new(),
            kinds: self.kinds.clone(),
            policy: Input::Refuse,
            moved: None,
            creation_checks: Vec::new(),
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
                if op.moved.is_some() {
                    return Err(Why::Refused(Refusal::InvalidTarget(
                        "one move per host operation".into(),
                    )));
                }
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
                    op.moved = Some((destination, page.clone()));
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
                let effect =
                    new.map_or(HostEffect::Delete, |bytes| HostEffect::Write(bytes.into()));
                op.pages.push((page, effect, expected));
            }
        }
        Ok(op)
    }

    /// The same live slot checks as preflight; a move exempts its source,
    /// while a CreateNew save also checks its DTO name for unreadable owners.
    pub(crate) fn admission_slot(
        &self,
        file: &FileId,
        name: Option<&str>,
        source: Option<&FileId>,
    ) -> Result<(), Why> {
        if let Some(source) = source {
            self.available_move_destination(source, file)?;
            self.twin(file, Some(source))
        } else if let Some(name) = name {
            self.page_create_checks(file, name)
        } else {
            self.raw_create_slot(file)
        }
    }
}
