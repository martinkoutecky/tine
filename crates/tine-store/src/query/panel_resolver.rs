//! Carry the panel resolver's date support through changed explicit keys.
use super::*;
use std::collections::BTreeSet;

/// The panel's resolver for many targets of one snapshot: the reads
/// [`graph_equivalent_page_names_with`] needs, gathered once, so a bulk
/// driver resolves each target as the Linked References panel does without a
/// per-target scan. The one setup the link table (`LinkGraph::build`) and the
/// All pages counter (`page_counts::referring_block_counts`) share.
///
/// Built in O(P + R) for P listed pages and R explicit reference names; holds
/// one name per journal day and the referenced day spellings. The membership
/// owner carries these persistent roots and patches changed explicit keys.
///
/// The day's journal is the FIRST journal page of the day in page-list order.
/// This agrees with batch 2's duplicate-day keeper: the snapshot's page list
/// is built from `dedup_journal_days` (claimant order `compare_page_claimants`,
/// launch build and incremental upsert alike), so it holds one journal per day,
/// the keeper, and "first" has nothing to choose between. It would disagree
/// only if the list ever held two journals of one day: then this pick follows
/// list order, not the claimant order. The panel itself asks
/// `ReadSnapshot::reference_journal_name` (claimants of the day's title key),
/// which over the same deduplicated list names the same page.
#[derive(Clone)]
pub(crate) struct PanelResolver {
    real: RealPageOwners,
    edges: std::sync::Arc<AliasEdges>,
    format: std::sync::Arc<JournalFormat>,
    first_journal: crate::model::persistent::Map<i64, String>,
    referenced_by_day: crate::model::persistent::Map<i64, Vec<String>>,
}

impl PanelResolver {
    pub(crate) fn new(graph: &ReadSnapshot) -> Self {
        let format = graph.config().journal_format();
        // The panel's `find` over the page list, once: the FIRST journal page
        // of each day in page-list order.
        let mut first_journal = crate::model::persistent::Map::new();
        for (entry, _) in graph.pages.iter() {
            if entry.kind == PageKind::Journal {
                if let Some(day) = format.parse(&entry.name) {
                    if !first_journal.contains_key(&day.ordinal_key()) {
                        first_journal.insert(day.ordinal_key(), entry.name.clone());
                    }
                }
            }
        }
        // Referenced names by the day they denote (the one date answerer),
        // once: what `referenced_day_keys` finds per target.
        let mut referenced_by_day = crate::model::persistent::Map::new();
        for name in explicit_reference_spellings(graph) {
            if let Some(day) = eval::reference_day(name, &format) {
                if !referenced_by_day.contains_key(&day) {
                    referenced_by_day.insert(day, Vec::new());
                }
                referenced_by_day
                    .get_mut(&day)
                    .unwrap()
                    .push(refs::page_key(name));
            }
        }
        PanelResolver {
            real: real_page_names(graph),
            edges: graph.alias_edges(),
            format: std::sync::Arc::new(format),
            first_journal,
            referenced_by_day,
        }
    }

    /// The graph's journal format, as the resolver reads dates.
    pub(crate) fn format(&self) -> &JournalFormat {
        &self.format
    }

    /// `(canonical, names, self_page)` for `target`, exactly
    /// [`graph_equivalent_page_names`] with `explicit` set.
    pub(crate) fn resolve(&self, target: &str) -> (String, Vec<String>, String) {
        graph_equivalent_page_names_with(
            &self.real,
            &self.edges,
            &self.format,
            target,
            |day| self.first_journal.get(&day.ordinal_key()).cloned(),
            |day| {
                self.referenced_by_day
                    .get(&day.ordinal_key())
                    .cloned()
                    .unwrap_or_default()
            },
        )
    }
}

impl PanelResolver {
    /// The same resolver over a succeeding snapshot. Date spelling changes
    /// inspect only changed source keys and the reached day buckets.
    pub(crate) fn succeeding(
        &self,
        old: &ReadSnapshot,
        graph: &ReadSnapshot,
        paths: &[String],
    ) -> Self {
        let mut next = self.clone();
        next.real = real_page_names(graph);
        next.edges = graph.alias_edges();
        let mut keys = BTreeSet::new();
        let mut days = BTreeSet::new();
        for path in paths {
            for snapshot in [old, graph] {
                let Some(&slot) = snapshot.pages.positions.get(path) else {
                    continue;
                };
                let Some((entry, doc)) = snapshot.pages.get(slot) else {
                    continue;
                };
                if entry.kind == PageKind::Journal {
                    if let Some(day) = self.format.parse(&entry.name) {
                        days.insert(day);
                    }
                }
                keys.extend(document_explicit_reference_names(
                    entry,
                    doc,
                    snapshot.config(),
                ));
            }
        }
        for day in days {
            match graph.reference_journal_name(day) {
                Some(name) => next.first_journal.insert(day.ordinal_key(), name),
                None => next.first_journal.remove(&day.ordinal_key()),
            }
        }
        for key in keys
            .into_iter()
            .filter(|key| key.bytes().any(|byte| byte.is_ascii_digit()))
        {
            let before = old.explicit_key_spelling(&key);
            let after = graph.explicit_key_spelling(&key);
            if before == after {
                continue;
            }
            for (spelling, add) in [(before, false), (after, true)] {
                let Some(spelling) = spelling else {
                    continue;
                };
                let Some(day) = eval::reference_day(&spelling, &self.format) else {
                    continue;
                };
                let mut names = next
                    .referenced_by_day
                    .get(&day)
                    .cloned()
                    .unwrap_or_default();
                let key = refs::page_key(&spelling);
                names.retain(|name| name != &key);
                if add {
                    names.push(key);
                }
                if names.is_empty() {
                    next.referenced_by_day.remove(&day);
                } else {
                    next.referenced_by_day.insert(day, names);
                }
            }
        }
        next
    }
}
