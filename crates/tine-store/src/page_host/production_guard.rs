//! Store-owned path and create rules at the host's actual I/O boundary.
use super::*;

/// An admitted operation's original raw-create / DTO / move preconditions.
/// Ephemeral, bounded by its creating pages; failures keep them for retry.
pub(super) struct Creation {
    bytes: Text,
    name: Option<String>,
    source: Option<crate::FileId>,
}

impl ProductionIo {
    pub(in crate::page_host) fn note_creation(
        &mut self,
        page: &str,
        bytes: Text,
        name: Option<String>,
        source: Option<crate::FileId>,
    ) {
        self.creation_rules.insert(
            page.into(),
            Creation {
                bytes,
                name,
                source,
            },
        );
    }

    pub(super) fn creation_input(&mut self, page: &str, bytes: &Text) {
        if self
            .creation_rules
            .get(page)
            .is_some_and(|rule| rule.bytes != *bytes)
        {
            self.creation_rules.remove(page);
        }
    }

    pub(super) fn check_path(&self, path: &Path) -> IoResult<()> {
        // Shared with the Store, so attachment captures no redundant identity
        // and cannot accept a replacement since Store::open.
        crate::model::checked_write_target(
            &self.graph,
            path,
            "graph root",
            Some(&self.root_binding),
        )
        .map_err(failure)
    }

    pub(super) fn creating_owner(
        &self,
        page: &str,
        bytes: &Text,
        source: Option<&str>,
    ) -> IoResult<Option<String>> {
        let Some(store) = self.store.upgrade() else {
            // Isolated physical adapters have no logical graph inventory.
            return Ok(None);
        };
        let target = self.page_path(page);
        self.check_path(&target)?;
        let file = crate::FileId::from(self.spellings.spelling(page));
        let source = source.map(|source| crate::FileId::from(source.to_owned()));
        // A retained raw create or binary rescue keeps opaque bytes, as its
        // transaction does. Only decodable text contributes a preamble name;
        // the filename identity and unreadable-owner rules still apply.
        let text = std::str::from_utf8(bytes.as_deref().unwrap_or_default()).unwrap_or("");
        let proposed = store.graph.entry_for_path_in(&target, Some(text));
        let rule = self
            .creation_rules
            .get(page)
            .filter(|rule| rule.bytes == *bytes);
        let name = match rule {
            Some(rule) => rule.name.as_deref(),
            None => proposed.as_ref().map(|entry| entry.name.as_str()),
        };
        let source = rule
            .and_then(|rule| rule.source.as_ref())
            .or(source.as_ref());
        match store
            .transaction(None)
            .creating_save_owner(&file, name, source)
        {
            Ok(()) => Ok(None),
            Err(crate::Why::Refused(crate::Refusal::Twin { existing })) => {
                Ok(Some(existing.as_str().into()))
            }
            Err(crate::Why::Refused(crate::Refusal::UnreadableOwner { file })) => {
                Ok(Some(file.as_str().into()))
            }
            Err(_) => Err(failure(io::Error::other(
                "could not check creating save owner",
            ))),
        }
    }
}
