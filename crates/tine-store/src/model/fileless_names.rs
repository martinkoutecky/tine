//! The typed answer for a name with no file (O3, CM-REFONLY, I-12).
//! Raw evidence/indexes keep their spellings; page consumers read this answer.
use super::persistent::Map as SharedMap;
use super::{PageEntry, PageKind, ReadSnapshot};
use std::collections::{BTreeMap, BTreeSet};
use tine_core::date::{JournalDate, JournalFormat};
use tine_core::refs;

/// Transient support, never checkpointed. Both levels share unchanged tree
/// nodes across generations. Namespace buckets retain every source so removing
/// the least spelling reveals the next without searching unrelated children.
#[derive(Clone, Default)]
pub(super) struct DaySupport {
    days: SharedMap<JournalDate, SharedMap<(String, bool), String>>,
    parents: SharedMap<String, SharedMap<(String, Source), ()>>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Reference(String),
    File(std::path::PathBuf),
}

impl DaySupport {
    fn spelling(&mut self, format: &JournalFormat, key: &str, parent: bool, name: &str, add: bool) {
        let Some(day) = FilelessName::new(format, name).day else {
            return;
        };
        let mut bucket = self.days.get(&day).cloned().unwrap_or_default();
        let key = (key.to_owned(), parent);
        if add {
            bucket.insert(key, name.to_owned());
        } else {
            bucket.remove(&key);
        }
        if bucket.is_empty() {
            self.days.remove(&day);
        } else {
            self.days.insert(day, bucket);
        }
    }

    fn name(&mut self, format: &JournalFormat, source: Source, name: &str, add: bool) {
        if let Source::Reference(key) = &source {
            self.spelling(format, key, false, name, add);
        }
        if !name.contains('/') {
            return;
        }
        for (prefix, key) in super::namespace_parents::parent_prefixes(name) {
            let mut bucket = self.parents.get(&key).cloned().unwrap_or_default();
            let before = bucket.first().map(|((name, _), _)| name.clone());
            let contribution = (prefix.to_owned(), source.clone());
            if add {
                bucket.insert(contribution, ());
            } else {
                bucket.remove(&contribution);
            }
            let after = bucket.first().map(|((name, _), _)| name.clone());
            if before != after {
                if let Some(name) = before {
                    self.spelling(format, &key, true, &name, false);
                }
                if let Some(name) = after {
                    self.spelling(format, &key, true, &name, true);
                }
            }
            if bucket.is_empty() {
                self.parents.remove(&key);
            } else {
                self.parents.insert(key, bucket);
            }
        }
    }

    fn build(snapshot: &ReadSnapshot) -> Self {
        let mut support = Self::default();
        let format = snapshot.config().journal_format();
        for (key, refs) in snapshot.referenced_name_table().iter() {
            support.name(
                &format,
                Source::Reference(key.clone()),
                &refs.spelling,
                true,
            );
        }
        for (_, path, name) in snapshot.real_page_names.claims_from("") {
            if name.contains('/') && snapshot.file_supplies_parents(path) {
                support.name(&format, Source::File(path.clone()), name, true);
            }
        }
        support
    }

    /// Patch the same changed reference keys as ReferencedNameTable, and the
    /// changed files' namespace seeds. No per-day scan of the graph.
    pub(super) fn carry(old: &ReadSnapshot, next: &ReadSnapshot, paths: &[String]) -> Option<Self> {
        let mut support = old.day_support.get()?.clone();
        let before = old.referenced_name_table_built()?;
        let after = next.referenced_name_table_built()?;
        if old.config.journal_file_name_format != next.config.journal_file_name_format
            || old.config.journal_page_title_format != next.config.journal_page_title_format
            || (paths.is_empty() && !std::sync::Arc::ptr_eq(&old.pages, &next.pages))
        {
            return None;
        }
        let format = next.config().journal_format();
        let mut keys = BTreeSet::new();
        for path in paths {
            for (snapshot, add) in [(old, false), (next, true)] {
                let Some((entry, _)) = super::snapshot_page_by_rel(&snapshot.pages, None, path)
                else {
                    continue;
                };
                for name in snapshot
                    .referenced_name_index()
                    .get(&entry.path)
                    .into_iter()
                    .flatten()
                {
                    keys.insert(refs::page_key(name));
                }
                if entry.name.contains('/') && snapshot.file_supplies_parents(&entry.path) {
                    support.name(&format, Source::File(entry.path.clone()), &entry.name, add);
                }
            }
        }
        for key in keys {
            let was = before.get(&key).map(|refs| refs.spelling.as_str());
            let now = after.get(&key).map(|refs| refs.spelling.as_str());
            if was == now {
                continue;
            }
            if let Some(name) = was {
                support.name(&format, Source::Reference(key.clone()), name, false);
            }
            if let Some(name) = now {
                support.name(&format, Source::Reference(key.clone()), name, true);
            }
        }
        Some(support)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FilelessName {
    pub(crate) name: String,
    /// An unclaimed spelling of the day when its display title is claimed
    /// by an ordinary file. Opening that literal title would open that file.
    pub(crate) opening_name: Option<String>,
    pub(crate) day: Option<JournalDate>,
    pub(crate) namespace_parent: bool,
}

impl FilelessName {
    /// The same fallback used by links. Real claims and aliases are checked
    /// by the caller before asking what an otherwise unclaimed name means.
    pub(crate) fn new(format: &JournalFormat, name: &str) -> Self {
        let day = name
            .bytes()
            .any(|b| b.is_ascii_digit())
            .then(|| format.parse(name))
            .flatten();
        Self {
            name: day.map_or_else(|| name.to_owned(), |day| format.title(day)),
            opening_name: None,
            day,
            namespace_parent: false,
        }
    }

    pub(crate) fn page_entry(&self) -> PageEntry {
        PageEntry {
            name: self.name.clone(),
            kind: if self.day.is_some() {
                PageKind::Journal
            } else {
                PageKind::Page
            },
            date_key: self.day.map(|day| day.ordinal_key()),
            rel_path: None,
            path: Default::default(),
        }
    }

    /// Complete an unclaimed spelling's identity: an active journal already
    /// supplies the day; an ordinary canonical claimant needs a safe opening.
    pub(crate) fn with_opening(
        mut self,
        real: &crate::query::RealPageOwners,
        spelling: &str,
    ) -> Option<Self> {
        if self.day.is_some() {
            if let Some(entry) = real.opening_entry(&refs::page_key(&self.name)) {
                if entry.kind == PageKind::Journal {
                    return None;
                }
                self.opening_name = Some(spelling.to_owned());
            }
        }
        Some(self)
    }
}

impl ReadSnapshot {
    /// No real-file reclassification: raw claims win first (CM-DATEPAGE).
    /// A date fallback is suppressed by an active journal or day alias.
    /// A canonical ordinary file claims only its own spelling (KD), so an
    /// alternate spelling can still support a distinct virtual journal.
    pub(crate) fn fileless_name(&self, format: &JournalFormat, name: &str) -> Option<FilelessName> {
        let answer = FilelessName::new(format, name);
        let unclaimed = |key: &str| {
            self.real_page_names.owners(key).next().is_none()
                && self
                    .alias_owner_paths(key, answer.day)
                    .is_none_or(|owners| owners.is_empty())
        };
        let key = refs::page_key(name);
        if !unclaimed(&key) {
            return None;
        }
        answer.with_opening(&self.real_page_owners(), name)
    }

    /// One row per fileless entity, in canonical key order. A direct
    /// reference takes precedence over namespace support for the same day.
    pub(crate) fn reference_only_pages(&self) -> Vec<FilelessName> {
        let format = self.config().journal_format();
        let table = self.referenced_name_table();
        let support = self.day_support();
        let mut rows = BTreeMap::new();
        for name in table.iter().map(|(_, refs)| refs.spelling.as_str()).chain(
            support
                .parents
                .iter()
                .filter(|(key, _)| table.get(key).is_none())
                .filter_map(|(_, bucket)| bucket.first().map(|((name, _), _)| name.as_str())),
        ) {
            let Some(answer) = self.reference_only_row(&format, table, name) else {
                continue;
            };
            rows.insert(refs::page_key(&answer.name), answer);
        }
        for (day, _) in support.days.iter() {
            if let Some(answer) = self.fileless_page_for_day(*day) {
                rows.insert(refs::page_key(&answer.name), answer);
            }
        }
        rows.into_values().collect()
    }

    /// The non-day row of [`Self::reference_only_pages`] for one spelling.
    fn reference_only_row(
        &self,
        format: &JournalFormat,
        table: &super::referenced_names::ReferencedNameTable,
        name: &str,
    ) -> Option<FilelessName> {
        let mut answer = self.fileless_name(format, name)?;
        if answer.day.is_some() {
            return None;
        }
        answer.namespace_parent = table.get(&refs::page_key(name)).is_none();
        Some(answer)
    }

    /// The row [`Self::reference_only_pages`] lists for `name`, by the same
    /// rule, for one name: a day spelling asks its day's row; any other name
    /// its key's referenced spelling, else its least namespace-child prefix.
    /// `None` when the name table or day support is not built for this
    /// generation (it is never built here: the caller cannot tell), else
    /// `Some(row)`. O(log names + spellings of one day); the change summary
    /// compares it across a publication (D-20261102-PERF row 3, review F1).
    pub(crate) fn resident_reference_only_page(&self, name: &str) -> Option<Option<FilelessName>> {
        let table = self.referenced_name_table_built()?;
        let support = self.day_support.get()?;
        let format = self.config().journal_format();
        let day = |name: &str| FilelessName::new(&format, name).day;
        if let Some(day) = day(name) {
            return Some(self.fileless_page_for_day(day));
        }
        let key = refs::page_key(name);
        let spelling = match table.get(&key) {
            Some(refs) => Some(refs.spelling.clone()),
            None => support
                .parents
                .get(&key)
                .and_then(|bucket| bucket.first().map(|((name, _), _)| name.clone())),
        };
        Some(spelling.and_then(|spelling| match day(&spelling) {
            Some(day) => self.fileless_page_for_day(day),
            None => self.reference_only_row(&format, table, &spelling),
        }))
    }

    fn day_support(&self) -> &DaySupport {
        self.day_support.get_or_init(|| DaySupport::build(self))
    }

    pub(crate) fn day_support_built(&self) -> bool {
        self.day_support.get().is_some()
    }

    /// The least namespace seed spelling of page key `key` (the answer
    /// [`Self::namespace_seed`] walks for), read from the resident parents
    /// support: its bucket orders every seed by spelling first, so its first
    /// entry is the least. `None` when the support is not resident (nothing is
    /// built here); `Some(None)` when `key` is no seed. O(log P) for P parent keys.
    pub(crate) fn resident_namespace_seed(&self, key: &str) -> Option<Option<&str>> {
        let support = self.day_support.get()?;
        #[cfg(test)]
        super::namespace_parents::seed_visit(key);
        Some(
            support
                .parents
                .get(key)
                .and_then(|bucket| bucket.first())
                .map(|((spelling, _), _)| spelling.as_str()),
        )
    }

    #[cfg(test)]
    pub(crate) fn assert_day_support_fresh(&self) {
        let rows = |support: &DaySupport| {
            support
                .days
                .iter()
                .flat_map(|(day, bucket)| {
                    bucket
                        .iter()
                        .map(move |(key, name)| (*day, key.clone(), name.clone()))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(self.day_support.get().expect("support inherited")),
            rows(&DaySupport::build(self))
        );
        let parents: BTreeMap<_, _> = self
            .day_support()
            .parents
            .iter()
            .filter_map(|(key, bucket)| Some((key.clone(), bucket.first()?.0 .0.as_str())))
            .collect();
        assert_eq!(parents, self.namespace_seeds(self.referenced_name_table()));
    }

    /// Both inventory and delta ask this answerer. O(log days + spellings of
    /// this day); namespace support reads the maintained least spelling.
    pub(crate) fn fileless_page_for_day(&self, day: JournalDate) -> Option<FilelessName> {
        let format = self.config().journal_format();
        let table = self.referenced_name_table();
        let mut result: Option<FilelessName> = None;
        for ((key, parent), name) in self.day_support().days.get(&day)?.iter() {
            if *parent && table.get(key).is_some() {
                continue;
            }
            if let Some(mut answer) = self.fileless_name(&format, name) {
                answer.namespace_parent = *parent;
                if let Some(row) = &mut result {
                    row.namespace_parent &= parent;
                } else {
                    result = Some(answer);
                }
            }
        }
        result
    }
}
