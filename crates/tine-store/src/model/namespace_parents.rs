//! Namespace parents: the one answer to "which pages exist because a listed
//! name lies under them" (conceptual model rule 1, SPEC §3.2, CM3-3; census
//! Q5 item 3, conformance S8). A name `n/y` makes `n` a page, whether the name
//! is a file's page name or only a referenced name.
//!
//! OG (0.10.9): `graph_parser/extract.cljc:194` gives every parsed file's
//! namespaced page name its prefix pages (`gp-util/split-namespace-pages`,
//! `util.cljs:116`), and `graph_parser/block.cljs` `with-page-refs` gives every
//! referenced namespaced name its prefix pages, so both appear in All Pages,
//! `[[n]]` opens them and page queries see them.
//!
//! Seeds (model `pageSeedKeys`): the proper namespace prefixes
//! ([`tine_core::refs::namespace_refs`], the same closure references use, Q6)
//! of every non-blank page file's name (journals and twins included; OG's
//! outer `extract` skips blank content, so an empty file supplies its own page
//! but no parents) and of every referenced name. A seed key is a parent page
//! only when no file of either kind claims it, no page declares it as an
//! alias (Q9), and no reference names it directly (then it is an ordinary
//! reference-only page): the callers apply those three rules, as they already
//! do for reference-only names.
//!
//! Spelling: the least (Rust `str` order) prefix spelling among the seeds of
//! one key, so the whole-graph answer and the one-key answer agree whatever
//! order they visit the seeds in.
//!
//! Nothing here is stored: the whole answer is one pass over the claims and
//! the resident referenced-name table; the one-key answer reads only the
//! names whose key starts with that key (a parent's key is always a prefix of
//! its child's key: the fold trims, lowercases per character and strips a
//! boundary slash, none of which crosses the `/` a prefix ends at).
use super::referenced_names::ReferencedNameTable;
use super::ReadSnapshot;
use std::collections::BTreeMap;

/// The proper namespace prefixes of `name`, each with its page key (empty
/// keys dropped: a prefix of only spaces or slashes names no page). The
/// publication's inventory delta asks it which parents a changed name moves.
pub(crate) fn parent_prefixes(name: &str) -> impl Iterator<Item = (&str, String)> {
    tine_core::refs::namespace_refs(name)
        .filter(move |prefix| prefix.len() < name.len())
        .map(|prefix| (prefix, tine_core::refs::page_key(prefix)))
        .filter(|(_, key)| !key.is_empty())
}

fn keep_least<'a>(slot: &mut Option<&'a str>, spelling: &'a str) {
    if slot.is_none_or(|kept| spelling < kept) {
        *slot = Some(spelling);
    }
}

impl ReadSnapshot {
    /// Whether the file at `path` (absolute, as the real-name table keys it)
    /// supplies its name's prefixes: a parsed page whose content is not blank.
    fn file_supplies_parents(&self, path: &std::path::Path) -> bool {
        let Some(entry) = self.list.get_path(path) else {
            return false;
        };
        let Some(&slot) = self.pages.positions.get(entry.rel_path_str()) else {
            return false;
        };
        self.pages.get(slot).is_some_and(|(_, doc)| {
            !doc.roots.is_empty()
                || doc
                    .pre_block
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty())
        })
    }

    /// Every namespace seed key of this generation with its spelling (see the
    /// module doc). O(F + R) for F file claims and R referenced names, plus a
    /// key fold per prefix of a name that contains `/`.
    pub(crate) fn namespace_seeds<'a>(
        &'a self,
        table: &'a ReferencedNameTable,
    ) -> BTreeMap<String, &'a str> {
        self.namespace_seeds_over(table.iter().map(|(_, referrers)| referrers.spelling.as_str()))
    }

    /// [`Self::namespace_seeds`] over any list of referenced names in place of
    /// the table's: `[[` completion's list is the table's after the editing
    /// block's reference baseline, and its parents follow that list.
    pub(crate) fn namespace_seeds_over<'a>(
        &'a self,
        referenced: impl Iterator<Item = &'a str>,
    ) -> BTreeMap<String, &'a str> {
        let mut seeds: BTreeMap<String, Option<&'a str>> = BTreeMap::new();
        for (_, path, name) in self.real_page_names.claims_from("") {
            if name.contains('/') && self.file_supplies_parents(path) {
                for (prefix, key) in parent_prefixes(name) {
                    keep_least(seeds.entry(key).or_default(), prefix);
                }
            }
        }
        for name in referenced {
            if name.contains('/') {
                for (prefix, key) in parent_prefixes(name) {
                    keep_least(seeds.entry(key).or_default(), prefix);
                }
            }
        }
        seeds
            .into_iter()
            .filter_map(|(key, spelling)| Some((key, spelling?)))
            .collect()
    }

    /// The namespace parents `[[` completion and page search offer (OG's page
    /// search indexes every page entity, parents included, with no ranking of
    /// their own: `search/db.cljs make-pages-title-indice!` over
    /// `db/get-pages`): the seeds over `referenced`, less the keys a file
    /// claims and the keys `referenced` names directly (those are ordinary
    /// referenced names, offered once). `referenced` must be in page-key
    /// order, as the name table and its baseline-adjusted copy are. Aliases
    /// are the caller's exclusion (it already holds every alias key).
    pub(crate) fn completion_namespace_parents<'a>(
        &'a self,
        referenced: &'a [std::borrow::Cow<'a, str>],
    ) -> Vec<&'a str> {
        let seeds = self.namespace_seeds_over(referenced.iter().map(|name| name.as_ref()));
        seeds
            .into_iter()
            .filter(|(key, _)| {
                self.real_page_names.owners(key).next().is_none()
                    && referenced
                        .binary_search_by(|name| tine_core::refs::page_key(name).as_str().cmp(key))
                        .is_err()
            })
            .map(|(_, spelling)| spelling)
            .collect()
    }

    /// [`Self::namespace_seeds`]' spelling for one key, or `None` when it is
    /// no seed: reads only the claims and referenced names whose key starts
    /// with `key`. O(log V + those names).
    pub(crate) fn namespace_seed<'a>(
        &'a self,
        table: &'a ReferencedNameTable,
        key: &str,
    ) -> Option<&'a str> {
        if key.is_empty() {
            return None;
        }
        let mut least = None;
        for (child, path, name) in self.real_page_names.claims_from(key) {
            if !child.starts_with(key) {
                break;
            }
            if name.contains('/') && self.file_supplies_parents(path) {
                for (prefix, prefix_key) in parent_prefixes(name) {
                    if prefix_key == key {
                        keep_least(&mut least, prefix);
                    }
                }
            }
        }
        for (child, referrers) in table.iter_from(key) {
            if !child.starts_with(key) {
                break;
            }
            if referrers.spelling.contains('/') {
                for (prefix, prefix_key) in parent_prefixes(&referrers.spelling) {
                    if prefix_key == key {
                        keep_least(&mut least, prefix);
                    }
                }
            }
        }
        least
    }
}
