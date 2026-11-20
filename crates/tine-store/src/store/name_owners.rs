//! Which page a name opens: the one owner rule ([`NameOwners::opens`]) over
//! the name's real-file claimants and its alias declarers, and the view's
//! resolver that feeds it ([`WholeGraph::resolve`]). A reader that knows how
//! a planned request changes those inputs (a rename's moved files, a merge's
//! removed source, dropped declarations) patches a [`WholeGraph::name_owners`]
//! answer and asks the same rule, so it cannot disagree with the resolver.
use super::*;

impl WholeGraph {
    /// The inputs [`Self::resolve`] decides `name` from (as an ordinary
    /// name), the declarers included even when a file claims it. Cost: as
    /// `resolve`, plus the alias lookup and the day's spellings.
    pub fn name_owners(&self, name: &str) -> NameOwners {
        let spellings = match self.journal_format.parse_text(name) {
            Some(day) => {
                let title = self.journal_format.title(day);
                let mut keys =
                    crate::query::journal_spelling_keys(&self.journal_format, day, &title);
                keys.push(tine_core::refs::page_key(name));
                let mut seen = HashSet::new();
                keys.retain(|key| seen.insert(key.clone()));
                keys
            }
            None => vec![tine_core::refs::page_key(name)],
        };
        NameOwners {
            claimants: self.claimants_of(name, PageKind::Page),
            declarers: self.declarers_of(name),
            spellings,
            journals_dir: self.config.config.journals_dir.clone(),
        }
    }

    /// The real files claiming `name` asked as `kind`, in claimant order.
    fn claimants_of(&self, name: &str, kind: PageKind) -> Vec<PageEntry> {
        // A date in any accepted spelling is that day, held by its files in
        // any directory (CM SPEC §2.1, D-20261110-CM CM-DATE).
        let lookup = crate::model::claim_lookup(&self.journal_format, name, kind).by_key(
            &self.journal_format,
            self.key_claimants(name, PageKind::Journal),
        );
        let mut entries = self.name_claimants(&lookup.name, lookup.kind);
        entries.retain(|entry| lookup.holds(entry));
        if !entries.is_empty() && lookup.day.is_none() {
            // A name that is no day is one identity across both kinds.
            entries = self.all_name_claimants(&lookup.name);
            entries.retain(|entry| lookup.holds(entry));
        }
        entries
    }

    /// The pages declaring `name` as an alias, in path order.
    fn declarers_of(&self, name: &str) -> Vec<PageId> {
        let day = self.journal_format.parse_text(name);
        (self.graph)
            .alias_owner_paths(&tine_core::refs::page_key(name), day)
            .map(|paths| paths.into_iter().map(PageId::from).collect())
            .unwrap_or_default()
    }

    /// Resolve a name in this captured view. The caller supplies whether the
    /// name is a journal title; a name that parses as a date is that day
    /// whatever the caller says (CM SPEC §2.1, D-20261110-CM CM-DATE), held
    /// by its files in any directory, and a day with no file proposes one in
    /// the journals directory (CM-DAYFILE). A non-date name the caller calls
    /// a journal may propose a new journal file. Real files win over aliases
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
        let kind = if is_journal {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        let dir = &self.config.config.journals_dir;
        let entries = self.claimants_of(name, kind);
        if let Some(id) = owner(dir, &entries, &[]) {
            let others = entries.iter().filter_map(|entry| entry.rel_path.clone());
            return Resolved::Existing {
                id: id.clone(),
                others: others.filter(|other| other != id).collect(),
            };
        }
        if !is_journal || self.journal_format.parse_text(name).is_some() {
            let owners = self.declarers_of(name);
            if let Some(first) = owner(dir, &[], &owners) {
                debug_assert_eq!(Some(first), owners.first(), "owners in path order");
                return Resolved::Alias { owners };
            }
        }
        Resolved::Absent {
            id: self.proposed_file(name, is_journal),
        }
    }

    /// The file a new page `name` is created as ([`Self::resolve`]'s absent
    /// answer), whatever claims or declares the name now: for a name a
    /// helper frees first (a released alias). O(name bytes), no I/O.
    pub fn proposed_file(&self, name: &str, is_journal: bool) -> PageId {
        let kind = if is_journal {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        let config = &self.config.config;
        // A day with no file gets a new one in the journals directory
        // (D-20261111-CMR14 CM-DAYFILE).
        if crate::model::claim_lookup(&self.journal_format, name, kind).kind == PageKind::Journal {
            let stem = self
                .journal_format
                .parse_text(name)
                .map(|date| self.journal_format.file_stem(date))
                .unwrap_or_else(|| name.to_owned());
            return proposed_journal_id(config, stem);
        }
        inventory::proposed_page_id(config, name)
    }
}
