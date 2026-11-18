//! Real page-name claimants used by publication and reference queries.
use super::*;

/// Every physical claimant of each real page name, shared by snapshot
/// publication and the query fallback. Each normalized name owns a persistent
/// claimant tree keyed by path, so removing a claimant costs O(log D), where D
/// is its duplicates, without scanning unrelated pages (I-12/I-25). The tree
/// is persisted in the launch checkpoint (and rebuilt when one is installed);
/// it does not decide which claimant owns the name: [`RealPageOwners::get`]
/// asks the claimant order.
///
/// A day's files are keyed by the day, not by a label other days share
/// ([`Self::claim_key`]; CM SPEC §2.1: "the identity of a date is
/// `(journal, dayValue)`", and under `MM-dd` "typing the ambiguous,
/// non-identifying label cannot select one").
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct RealPageNames {
    claimants: crate::model::persistent::Map<
        String,
        crate::model::persistent::Map<std::path::PathBuf, PageEntry>,
    >,
    /// The format the keys were made with: not persisted, a checkpoint's
    /// table is rebuilt on install.
    #[serde(skip, default = "default_format")]
    format: std::sync::Arc<JournalFormat>,
}

fn default_format() -> std::sync::Arc<JournalFormat> {
    std::sync::Arc::new(JournalFormat::default())
}

/// The real-name table with what ranks its twins: the owner of a normalized
/// name across kinds is the least claimant by
/// [`crate::model::compare_page_claimants`], the same across-kind order that
/// `resolve` uses (R13 OWN2, I-12). Only this key's claimant bucket is visited;
/// the configured journals directory is the only setting that ranks (CM SPEC
/// §2.2), and filename-format ranks play no part.
#[derive(Clone)]
pub(crate) struct RealPageOwners {
    names: std::sync::Arc<RealPageNames>,
    journals_dir: std::sync::Arc<str>,
}
impl RealPageOwners {
    pub(crate) fn new(names: std::sync::Arc<RealPageNames>, journals_dir: &str) -> Self {
        Self {
            names,
            journals_dir: journals_dir.into(),
        }
    }
    /// No real pages (a scope that cannot change membership).
    pub(crate) fn empty() -> Self {
        Self::new(
            std::sync::Arc::new(RealPageNames::new(default_format())),
            "journals",
        )
    }
    /// The key `entry` is claimed under ([`RealPageNames::claim_key`]).
    pub(crate) fn claim_key(&self, entry: &PageEntry) -> String {
        self.names.claim_key(entry)
    }
    /// [`page_handle`] under this table's format.
    pub(crate) fn handle<'a>(&self, name: &'a str, day: Option<i64>) -> std::borrow::Cow<'a, str> {
        page_handle(&self.names.format, name, day)
    }
    /// [`RealPageNames::day_key`].
    pub(crate) fn day_key(&self, name: &str, day: Option<i64>) -> Option<String> {
        self.names.day_key(name, day)
    }
    /// The opening claimant of `day`, by the day's identifying spelling.
    /// O(log P + claimants of that key); no document reads.
    pub(crate) fn day_entry(&self, day: JournalDate) -> Option<&PageEntry> {
        self.opening_entry(&refs::page_key(
            &self.names.format.identifying_spelling(day),
        ))
        .filter(|entry| entry.date_key == Some(day.ordinal_key()))
    }
    /// The owner of a normalized real page name, with its stored name.
    pub(crate) fn get(&self, key: &str) -> Option<(&std::path::PathBuf, &String)> {
        self.opening_entry(key)
            .map(|entry| (&entry.path, &entry.name))
    }
    pub(crate) fn opening_entry(&self, key: &str) -> Option<&PageEntry> {
        let bucket = self.names.claimants.get(key)?;
        self.opening_claimant(bucket.iter().map(|(_, entry)| entry))
    }
    fn opening_claimant<'a>(
        &self,
        entries: impl Iterator<Item = &'a PageEntry>,
    ) -> Option<&'a PageEntry> {
        entries.min_by(|a, b| crate::model::compare_page_claimants(&self.journals_dir, a, b))
    }
    /// Active real-name owners under an identity prefix, in identity order.
    /// O(log P + C) for C matching physical claimants; no document reads.
    pub(crate) fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a PageEntry> + 'a {
        self.names
            .claimants
            .iter_from(prefix)
            .take_while(move |(key, _)| key.starts_with(prefix))
            .filter_map(|(_, bucket)| {
                self.opening_claimant(bucket.iter().map(|(_, entry)| {
                    #[cfg(feature = "test-faults")]
                    crate::cost_counters::namespace_name_visit();
                    entry
                }))
            })
    }
}
impl RealPageNames {
    pub(crate) fn new(format: std::sync::Arc<JournalFormat>) -> Self {
        Self {
            claimants: Default::default(),
            format,
        }
    }
    /// The key a claimant is filed under: its normalized name, except that a
    /// day's file whose name does not identify its day (the `MM-dd` label
    /// `11-02`) is filed under the day's identifying spelling
    /// ([`JournalFormat::claim_spelling`]). For a title format with a year
    /// this is `page_key(name)`. O(name + parse list) for a day, O(name)
    /// otherwise.
    pub(crate) fn claim_key(&self, entry: &PageEntry) -> String {
        self.day_key(&entry.name, entry.date_key)
            .unwrap_or_else(|| refs::page_key(&entry.name))
    }
    /// The key of day `day` (`yyyymmdd`) when its name `name` does not
    /// identify it (the `MM-dd` label `11-02`); `None` when `name` names the
    /// day itself or there is no day, so a caller keeps its own fold of
    /// `name` and a title format with a year answers exactly as before.
    /// O(name + parse list) for a day.
    pub(crate) fn day_key(&self, name: &str, day: Option<i64>) -> Option<String> {
        day_key(&self.format, name, day)
    }
    /// Every physical owner of this normalized name, including duplicates.
    /// Reuses the publication's claimant tree; no new query index is built.
    pub(crate) fn paths(&self, key: &str) -> impl Iterator<Item = &std::path::PathBuf> {
        self.claimants
            .get(key)
            .into_iter()
            .flat_map(|bucket| bucket.iter().map(|(path, _)| path))
    }
    /// Every physical owner of this normalized name with its effective name.
    pub(crate) fn owners(&self, key: &str) -> impl Iterator<Item = (&std::path::PathBuf, &String)> {
        self.claimants
            .get(key)
            .into_iter()
            .flat_map(|bucket| bucket.iter().map(|(path, entry)| (path, &entry.name)))
    }
    /// Every claim `(key, path, effective name)` whose key is at least
    /// `from`, in key order and path order within a key (`""` lists all).
    /// O(log K) to start, then O(1) amortized per claim.
    pub(crate) fn claims_from<'a>(
        &'a self,
        from: &str,
    ) -> impl Iterator<Item = (&'a String, &'a std::path::PathBuf, &'a String)> + 'a {
        self.claimants.iter_from(from).flat_map(|(key, bucket)| {
            bucket
                .iter()
                .map(move |(path, entry)| (key, path, &entry.name))
        })
    }
    fn update(&mut self, entry: &PageEntry, added: bool) {
        let key = self.claim_key(entry);
        let mut bucket = self.claimants.get(&key).cloned().unwrap_or_default();
        if added {
            bucket.insert(entry.path.clone(), entry.clone());
        } else {
            bucket.remove(&entry.path);
        }
        if !bucket.is_empty() {
            self.claimants.insert(key, bucket);
        } else {
            self.claimants.remove(&key);
        }
    }
    /// `format`: the graph's journal format, which keys days. An `old` table
    /// made with other date patterns is rebuilt, not patched.
    pub(crate) fn capture(
        old: Option<(&Self, &crate::model::persistent::Pages)>,
        pages: &crate::model::persistent::Pages,
        paths: &[String],
        format: std::sync::Arc<JournalFormat>,
    ) -> Self {
        let old = old.filter(|(old, _)| {
            old.format.title_format() == format.title_format()
                && old.format.file_format() == format.file_format()
        });
        if let Some((old, before)) = old {
            let mut next = old.clone();
            for path in paths {
                let was = before
                    .positions
                    .get(path)
                    .and_then(|&slot| before.get(slot))
                    .map(|p| &p.0);
                let now = pages
                    .positions
                    .get(path)
                    .and_then(|&slot| pages.get(slot))
                    .map(|p| &p.0);
                if was.map(|e| (&e.path, &e.name)) == now.map(|e| (&e.path, &e.name)) {
                    continue;
                }
                if let Some(e) = was {
                    next.update(e, false);
                }
                if let Some(e) = now {
                    next.update(e, true);
                }
            }
            next
        } else {
            let mut next = Self::new(format);
            for (entry, _) in pages {
                next.update(entry, true);
            }
            next
        }
    }
}

/// The name that identifies a page named `name` with day `day` (`yyyymmdd`,
/// `None` for an ordinary page) when it is asked a name-keyed question: `name`
/// itself, except for a day whose name does not identify it under `format`
/// (the `MM-dd` label `11-02`, CM SPEC §2.1: "typing the ambiguous,
/// non-identifying label cannot select one"), which is asked by
/// [`JournalFormat::identifying_spelling`]. For a title format with a year
/// it is always `name`. The one answer behind [`day_key`]. O(name + parse
/// list) for a day, O(1) otherwise.
pub(crate) fn page_handle<'a>(
    format: &JournalFormat,
    name: &'a str,
    day: Option<i64>,
) -> std::borrow::Cow<'a, str> {
    match day {
        Some(day) => format.claim_spelling(JournalDate::from_ordinal(day), name),
        None => std::borrow::Cow::Borrowed(name),
    }
}

/// The page key of [`page_handle`] when it is not `name` (a day its name does
/// not identify); `None` otherwise, so a caller keeps its own fold of `name`
/// and a title format with a year answers exactly as before. The one answer
/// to "what key does this day's page have" ([`RealPageNames::claim_key`],
/// the query row's page key, the link table's node key).
pub(crate) fn day_key(format: &JournalFormat, name: &str, day: Option<i64>) -> Option<String> {
    match page_handle(format, name, day) {
        std::borrow::Cow::Borrowed(_) => None,
        std::borrow::Cow::Owned(spelling) => Some(refs::page_key(&spelling)),
    }
}

pub(crate) fn real_page_names(graph: &ReadSnapshot) -> RealPageOwners {
    graph.real_page_owners()
}
