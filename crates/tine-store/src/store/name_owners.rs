//! Which page a name opens: the one owner rule ([`NameOwners::opens`]) over
//! the name's real-file claimants and its alias declarers, and the view's
//! resolver that feeds it ([`WholeGraph::resolve`]). A reader that knows how
//! a planned request changes those inputs (a rename's moved files, a merge's
//! removed source, dropped declarations) patches a [`WholeGraph::name_owners`]
//! answer and asks the same rule, so it cannot disagree with the resolver.
use super::*;
use crate::model::OpeningName;

pub(super) enum Opening<'a> {
    File(&'a PageEntry, &'a [PageEntry]),
    Alias(Vec<String>),
    Absent,
}

/// The spelling's opening in a complete snapshot. The same physical
/// selection also serves cold reads, where the alias relation is unavailable.
pub(super) fn open_name<'a>(
    claimants: &'a crate::model::ClaimantMap,
    graph: &ReadSnapshot,
    format: &JournalFormat,
    name: &str,
    as_journal: bool,
) -> (OpeningName, Opening<'a>) {
    let identity = OpeningName::new(format, name, as_journal);
    let opening = if let Some((entry, others)) = identity.claims(claimants).split_first() {
        Opening::File(entry, others)
    } else if identity.undated_journal {
        Opening::Absent
    } else if let Some(paths) = graph
        .alias_owner_paths(&identity.key, identity.day)
        .filter(|paths| !paths.is_empty())
    {
        Opening::Alias(paths)
    } else {
        Opening::Absent
    };
    (identity, opening)
}

impl WholeGraph {
    /// The inputs [`Self::resolve`] decides `name` from (as an ordinary
    /// name), the declarers included even when a file claims it. Cost: as
    /// `resolve`, plus the alias lookup and the day's spellings.
    pub fn name_owners(&self, name: &str) -> NameOwners {
        let identity = OpeningName::new(&self.journal_format, name, false);
        let spellings = match identity.day {
            Some(day) => {
                let title = self.journal_format.title(day);
                let mut keys =
                    crate::query::journal_spelling_keys(&self.journal_format, day, &title);
                keys.push(identity.key.clone());
                let mut seen = HashSet::new();
                keys.retain(|key| seen.insert(key.clone()));
                keys
            }
            None => vec![identity.key.clone()],
        };
        NameOwners {
            claimants: identity.claims(&self.claimants).to_vec(),
            declarers: self
                .graph
                .alias_owner_paths(&identity.key, identity.day)
                .map(|paths| paths.into_iter().map(PageId::from).collect())
                .unwrap_or_default(),
            spellings,
            journals_dir: self.config.config.journals_dir.clone(),
        }
    }

    /// Resolve a name in this captured view. The caller supplies whether the
    /// name is a journal title; a name that parses as a date is that day
    /// whatever the caller says (CM SPEC §2.1, D-20261110-CM CM-DATE), held
    /// by its files in any directory, and a day with no file proposes one in
    /// the journals directory (CM-DAYFILE). An explicit journal request for
    /// an undated name needs a journal claimant or proposes a new journal.
    /// Real files win over aliases
    /// ([`NameOwners::opens`], the one owner rule).
    /// Claimants rank the journals directory first, then unsigned UTF-8
    /// graph-relative paths (SPEC §2.2); the extension and matching filename
    /// have no separate rank.
    /// Ordinary page claims use a preamble `title::` or Org title
    /// directive when present, then the decoded filename. After a
    /// journal-format change, a new view uses the new title format for date
    /// claims; the parser still tries its documented fallback formats. Old
    /// custom-format links are not rewritten. If an old title no longer parses
    /// under the configured or fallback formats, resolving it as an ordinary
    /// page can propose a new page rather than the former journal. To classify a clicked journal
    /// title, use the current `JournalFormat::parse_text`, which tries the
    /// text routes, then pass that result as `is_journal`. Cost
    /// O(1) indexed lookup after the snapshot's one-time O(all aliases)
    /// alias-key build when no real file wins.
    pub fn resolve(&self, name: &str, is_journal: bool) -> Resolved {
        let (identity, opening) = open_name(
            &self.claimants,
            &self.graph,
            &self.journal_format,
            name,
            is_journal,
        );
        match opening {
            Opening::File(entry, others) => Resolved::Existing {
                id: entry.rel_path.clone().expect("physical claimant"),
                others: others
                    .iter()
                    .filter_map(|entry| entry.rel_path.clone())
                    .collect(),
            },
            Opening::Alias(paths) => Resolved::Alias {
                owners: paths.into_iter().map(PageId::from).collect(),
            },
            Opening::Absent => Resolved::Absent {
                id: self.proposed_file_for(name, is_journal, identity.day),
            },
        }
    }

    /// The file a new page `name` is created as ([`Self::resolve`]'s absent
    /// answer), whatever claims or declares the name now: for a name a
    /// helper frees first (a released alias). O(name bytes), no I/O.
    pub fn proposed_file(&self, name: &str, is_journal: bool) -> PageId {
        self.proposed_file_for(name, is_journal, self.journal_format.parse_text(name))
    }

    fn proposed_file_for(&self, name: &str, is_journal: bool, day: Option<JournalDate>) -> PageId {
        let config = &self.config.config;
        // A day with no file gets a new one in the journals directory
        // (D-20261111-CMR14 CM-DAYFILE).
        if is_journal || day.is_some() {
            let stem = day
                .map(|date| self.journal_format.file_stem(date))
                .unwrap_or_else(|| name.to_owned());
            return proposed_journal_id(config, stem);
        }
        inventory::proposed_page_id(config, name)
    }
}
