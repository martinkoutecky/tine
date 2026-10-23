//! The alias relation of one generation, kept by delta (D-20261018-M2b/M2c).
//!
//! Three read shapes serve every alias consumer: the adjacency walked to find
//! an alias class (`AliasEdges`), the owner paths of one alias key
//! (`resolve`, name search) and the whole alias list (inventory, checkpoint).
//! All three are functions of the same rows, one per page that declares
//! aliases: `(path) -> AliasRow { owner name, rel path, alias keys }`. An edit
//! replaces the rows of the pages it changed: the old row's contributions are
//! taken out of the adjacency and the owner map, the new row's put in, each in
//! O(log) persistent-tree steps, so the cost of an alias edit is the edit and
//! not the graph (D-16, I-25). Classes are never stored; they are a walk over
//! the adjacency at read time, so a deleted alias that splits a class needs
//! no repair. Multi-edges and repeated spellings are counted, so removing one
//! page's contribution leaves every other page's intact.
//!
//! Exemplar: `page_names.rs` (`RealPageNames::capture`), the same delta over
//! the same changed paths. The relation is in memory only; the checkpoint
//! writes its list and owner map in the format it always had
//! (`model/checkpoint_state.rs`).
use super::*;
use crate::model::persistent::Map;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type Multiset = Map<String, u32>;

#[derive(Clone, Copy)]
enum Table {
    Neighbors,
    Originals,
}

/// The four facts one (alias, owner) pair puts in the adjacency: the two
/// directions of its edge and the spelling each end was written with. Both
/// the bulk build and the delta go through this one definition.
fn pair_facts(alias: &str, owner: &str) -> [(Table, String, String); 4] {
    let alias_norm = refs::page_key(alias);
    let owner_norm = refs::page_key(owner);
    [
        (Table::Neighbors, alias_norm.clone(), owner_norm.clone()),
        (Table::Neighbors, owner_norm.clone(), alias_norm.clone()),
        (Table::Originals, alias_norm, alias.to_owned()),
        (Table::Originals, owner_norm, owner.to_owned()),
    ]
}

fn adjust(sets: &mut Map<String, Multiset>, key: &str, member: &str, add: bool) {
    let mut inner = sets.get(key).cloned().unwrap_or_default();
    let count = inner.get(member).copied().unwrap_or(0);
    if add {
        inner.insert(member.to_owned(), count + 1);
    } else if count > 1 {
        inner.insert(member.to_owned(), count - 1);
    } else {
        inner.remove(member);
    }
    if inner.is_empty() {
        sets.remove(key);
    } else {
        sets.insert(key.to_owned(), inner);
    }
}

/// Normalized name to its alias-adjacent normalized names, and to the
/// spellings it was written with. A component walk costs O(component), not
/// O(every alias in the graph) (I-25).
#[derive(Clone, Default)]
pub(crate) struct AliasEdges {
    neighbors: Map<String, Multiset>,
    originals: Map<String, Multiset>,
}

impl AliasEdges {
    /// The edges of `(alias, owner)` pairs (tests; the relation builds its
    /// own from page rows).
    #[cfg(test)]
    pub(crate) fn new(aliases: &[(String, String)]) -> Self {
        let mut edges = Self::default();
        for (alias, owner) in aliases {
            edges.apply(alias, owner, true);
        }
        edges
    }

    fn apply(&mut self, alias: &str, owner: &str, add: bool) {
        for (table, key, member) in pair_facts(alias, owner) {
            let sets = match table {
                Table::Neighbors => &mut self.neighbors,
                Table::Originals => &mut self.originals,
            };
            adjust(sets, &key, &member, add);
        }
    }

    /// The names written next to `name` by an alias declaration.
    pub(crate) fn neighbors(&self, name: &str) -> impl Iterator<Item = &String> {
        self.neighbors
            .get(name)
            .into_iter()
            .flat_map(|set| set.iter().map(|(member, _)| member))
    }

    /// The spellings `name` was written with.
    pub(crate) fn originals(&self, name: &str) -> impl Iterator<Item = &String> {
        self.originals
            .get(name)
            .into_iter()
            .flat_map(|set| set.iter().map(|(member, _)| member))
    }
}

/// What one page contributes: its owner name, its graph-relative path and the
/// page key of each alias it declares, in spelling order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct AliasRow {
    name: String,
    rel: String,
    keys: Vec<String>,
}

impl AliasRow {
    /// The row of `entry` declaring `spellings`; `None` when it declares none.
    pub(crate) fn of(entry: &PageEntry, spellings: &[String]) -> Option<Self> {
        if spellings.is_empty() {
            return None;
        }
        let mut sorted: Vec<&String> = spellings.iter().collect();
        sorted.sort();
        Some(Self {
            name: entry.name.clone(),
            rel: entry.rel_path_str().to_owned(),
            keys: sorted.into_iter().map(|s| refs::page_key(s)).collect(),
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct AliasRelation {
    /// Pages that declare aliases, by absolute path (the order the alias list
    /// has always had).
    rows: Map<PathBuf, Arc<AliasRow>>,
    edges: Arc<AliasEdges>,
    /// Alias key to the pages declaring it: their relative paths, with the
    /// number of declarations on that page.
    owners: Map<String, Map<PathBuf, (String, u32)>>,
}

impl AliasRelation {
    /// The relation of `rows`, built in one pass: O(rows).
    pub(crate) fn build(rows: impl IntoIterator<Item = (PathBuf, AliasRow)>) -> Self {
        use std::collections::{BTreeMap, HashMap};
        let mut page_rows = Vec::new();
        let mut neighbors: HashMap<String, HashMap<String, u32>> = HashMap::new();
        let mut originals: HashMap<String, HashMap<String, u32>> = HashMap::new();
        let mut owners: HashMap<String, BTreeMap<PathBuf, (String, u32)>> = HashMap::new();
        for (path, row) in rows {
            for key in &row.keys {
                for (table, k, m) in pair_facts(key, &row.name) {
                    let tally = match table {
                        Table::Neighbors => &mut neighbors,
                        Table::Originals => &mut originals,
                    };
                    *tally.entry(k).or_default().entry(m).or_default() += 1;
                }
                owners
                    .entry(refs::page_key(key))
                    .or_default()
                    .entry(path.clone())
                    .or_insert_with(|| (row.rel.clone(), 0))
                    .1 += 1;
            }
            page_rows.push((path, Arc::new(row)));
        }
        let sets = |tally: HashMap<String, HashMap<String, u32>>| -> Map<String, Multiset> {
            tally
                .into_iter()
                .map(|(k, members)| (k, members.into_iter().collect()))
                .collect()
        };
        Self {
            rows: page_rows.into_iter().collect(),
            edges: Arc::new(AliasEdges {
                neighbors: sets(neighbors),
                originals: sets(originals),
            }),
            owners: owners
                .into_iter()
                .map(|(k, by_path)| (k, by_path.into_iter().collect()))
                .collect(),
        }
    }

    /// The relation a checkpoint's alias list describes. `path_of` names the
    /// page at a relative path; one the graph no longer has gives `None`
    /// (the relation is then rebuilt on first use, never guessed).
    pub(crate) fn from_list(
        list: &[(String, String, String)],
        path_of: impl Fn(&str) -> Option<PathBuf>,
    ) -> Option<Self> {
        let mut rows: Vec<(PathBuf, AliasRow)> = Vec::new();
        let mut at: std::collections::HashMap<&str, usize> = Default::default();
        for (key, name, rel) in list {
            let slot = match at.get(rel.as_str()) {
                Some(slot) => *slot,
                None => {
                    rows.push((
                        path_of(rel)?,
                        AliasRow {
                            name: name.clone(),
                            rel: rel.clone(),
                            keys: Vec::new(),
                        },
                    ));
                    at.insert(rel, rows.len() - 1);
                    rows.len() - 1
                }
            };
            rows[slot].1.keys.push(key.clone());
        }
        Some(Self::build(rows))
    }

    /// Put `row` (or nothing) at `path`, taking out what the page declared
    /// before. O(aliases of the two rows * log graph); a row equal to the old
    /// one touches nothing. Returns whether the relation changed.
    pub(crate) fn replace(&mut self, path: &Path, row: Option<AliasRow>) -> bool {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::alias_page_probe();
        let before = self.rows.get(path).cloned();
        if before.as_deref() == row.as_ref() {
            return false;
        }
        if let Some(before) = before {
            self.contribute(path, &before, false);
            self.rows.remove(path);
        }
        if let Some(row) = row {
            self.contribute(path, &row, true);
            self.rows.insert(path.to_owned(), Arc::new(row));
        }
        true
    }

    fn contribute(&mut self, path: &Path, row: &AliasRow, add: bool) {
        for key in &row.keys {
            Arc::make_mut(&mut self.edges).apply(key, &row.name, add);
            let alias_norm = refs::page_key(key);
            let mut by_path = self.owners.get(&alias_norm).cloned().unwrap_or_default();
            let count = by_path.get(path).map_or(0, |(_, n)| *n);
            if add {
                by_path.insert(path.to_owned(), (row.rel.clone(), count + 1));
            } else if count > 1 {
                by_path.insert(path.to_owned(), (row.rel.clone(), count - 1));
            } else {
                by_path.remove(path);
            }
            if by_path.is_empty() {
                self.owners.remove(&alias_norm);
            } else {
                self.owners.insert(alias_norm, by_path);
            }
        }
    }

    pub(crate) fn edges(&self) -> Arc<AliasEdges> {
        Arc::clone(&self.edges)
    }

    /// Relative paths of the pages declaring `key`, once per declaration, in
    /// plain text order of the relative path: the owner rule
    /// (D-20261019-AL2), the order the name inventory lists owners in, so
    /// the first owner is the same for every resolver. (The map's own order
    /// compares paths folder by folder: `a/x.md` before `a b.md`.)
    pub(crate) fn owner_paths(&self, key: &str) -> Option<Vec<String>> {
        let by_path = self.owners.get(key)?;
        let mut paths: Vec<String> = by_path
            .values()
            .flat_map(|(rel, n)| std::iter::repeat_n(rel.clone(), *n as usize))
            .collect();
        paths.sort();
        Some(paths)
    }

    /// Absolute paths of the pages declaring `key` (keys of the per-page
    /// indexes), in map order.
    pub(crate) fn owner_files(&self, key: &str) -> impl Iterator<Item = &PathBuf> {
        self.owners
            .get(key)
            .into_iter()
            .flat_map(|by_path| by_path.iter().map(|(path, _)| path))
    }

    /// The alias list: `(alias key, owner name, owner rel path)` in path
    /// order, then spelling order.
    pub(crate) fn list(&self) -> Vec<(String, String, String)> {
        self.rows
            .values()
            .flat_map(|row| {
                row.keys
                    .iter()
                    .map(|key| (key.clone(), row.name.clone(), row.rel.clone()))
            })
            .collect()
    }

    /// Every alias key with its owner paths, by key (the checkpoint's order).
    pub(crate) fn owner_rows(&self) -> Vec<(String, Vec<String>)> {
        self.owners
            .iter()
            .map(|(key, by_path)| {
                (
                    key.clone(),
                    by_path
                        .values()
                        .flat_map(|(rel, n)| std::iter::repeat_n(rel.clone(), *n as usize))
                        .collect(),
                )
            })
            .collect()
    }

    /// A canonical text of the whole relation, for equality tests: equal
    /// exactly when a delta-maintained relation matches a rebuilt one.
    #[cfg(any(test, feature = "test-faults"))]
    pub(crate) fn image(&self) -> String {
        let mut out = String::new();
        for (path, row) in self.rows.iter() {
            out.push_str(&format!("row {path:?} {row:?}\n"));
        }
        for (table, sets) in [
            ("neighbors", &self.edges.neighbors),
            ("originals", &self.edges.originals),
        ] {
            for (key, set) in sets.iter() {
                let members: Vec<_> = set.iter().collect();
                out.push_str(&format!("{table} {key:?} {members:?}\n"));
            }
        }
        for (key, by_path) in self.owners.iter() {
            let paths: Vec<_> = by_path.iter().collect();
            out.push_str(&format!("owners {key:?} {paths:?}\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    /// xorshift: a seeded, dependency-free sequence.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }
    }

    fn entry(path: &str, name: &str) -> PageEntry {
        PageEntry {
            name: name.to_owned(),
            kind: PageKind::Page,
            date_key: None,
            rel_path: Some(crate::PageId::from(path.to_owned())),
            path: PathBuf::from(format!("/g/{path}")),
        }
    }

    /// The model: the rows themselves, the definition every derived shape
    /// is a function of (what a full rebuild reads).
    type Model = BTreeMap<PathBuf, AliasRow>;

    /// The shapes the relation must answer, computed from the model the way
    /// the pre-delta code did: an adjacency of `Vec`s built by one pass over
    /// every pair, an owner map and a list.
    fn oracle(model: &Model) -> (Vec<(String, String, String)>, String) {
        let mut list = Vec::new();
        let mut neighbors: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in model.values() {
            for key in &row.keys {
                list.push((key.clone(), row.name.clone(), row.rel.clone()));
                let (alias, owner) = (refs::page_key(key), refs::page_key(&row.name));
                neighbors
                    .entry(alias.clone())
                    .or_default()
                    .push(owner.clone());
                neighbors.entry(owner).or_default().push(alias.clone());
                owners.entry(alias).or_default().push(row.rel.clone());
            }
        }
        let classes = classes_of(neighbors.keys().cloned(), |name| {
            neighbors.get(name).cloned().unwrap_or_default()
        });
        (list, format!("{classes:?} {owners:?}"))
    }

    fn classes_of(
        names: impl Iterator<Item = String>,
        adjacent: impl Fn(&str) -> Vec<String>,
    ) -> BTreeMap<String, BTreeSet<String>> {
        let mut out = BTreeMap::new();
        for start in names {
            let mut class = BTreeSet::new();
            let mut pending = vec![start.clone()];
            while let Some(name) = pending.pop() {
                if class.insert(name.clone()) {
                    pending.extend(adjacent(&name));
                }
            }
            out.insert(start, class);
        }
        out
    }

    fn relation_view(relation: &AliasRelation) -> (Vec<(String, String, String)>, String) {
        let edges = relation.edges();
        let names: Vec<String> = relation
            .rows
            .values()
            .flat_map(|row| {
                row.keys
                    .iter()
                    .map(|key| refs::page_key(key))
                    .chain([refs::page_key(&row.name)])
            })
            .collect();
        let classes = classes_of(names.into_iter(), |name| {
            edges.neighbors(name).cloned().collect()
        });
        let owners: BTreeMap<String, Vec<String>> = relation.owner_rows().into_iter().collect();
        (relation.list(), format!("{classes:?} {owners:?}"))
    }

    /// T4 (incremental = rebuild) on the relation itself: after every step of
    /// a random sequence of page edits (declare, drop, re-case, rename,
    /// delete, collide names and aliases) the delta-maintained relation is
    /// identical, shape by shape, to one built from the rows, and answers
    /// what the one-pass adjacency answers.
    #[test]
    fn delta_equals_rebuild_over_random_edit_sequences() {
        let pool = [
            "Ax", "ax", "AX", "Ay", "Shared", "Other", "Page A", "page b", "Z",
        ];
        for seed in 1..=12u64 {
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x1000_0000_01B3));
            let mut relation = AliasRelation::default();
            let mut model = Model::new();
            for step in 0..300 {
                let slot = rng.next(8);
                let path = format!("pages/P{slot}.md");
                // A page that is renamed keeps its slot's file or moves to
                // another: both ends are edited, as a move publishes.
                let name = ["Page A", "page b", "Page C", "PAGE A", "Z"][rng.next(5)];
                let spellings: Vec<String> = (0..rng.next(4))
                    .map(|_| pool[rng.next(pool.len())].to_owned())
                    .collect();
                let page = entry(&path, name);
                let row = AliasRow::of(&page, &spellings);
                match rng.next(10) {
                    0 | 1 => {
                        // delete
                        let changed = relation.replace(&page.path, None);
                        assert_eq!(changed, model.remove(&page.path).is_some());
                    }
                    _ => {
                        let changed = relation.replace(&page.path, row.clone());
                        let was = match &row {
                            Some(row) => model.insert(page.path.clone(), row.clone()),
                            None => model.remove(&page.path),
                        };
                        assert_eq!(changed, was != row, "seed {seed} step {step}");
                    }
                }
                let rebuilt = AliasRelation::build(model.clone());
                assert_eq!(
                    relation.image(),
                    rebuilt.image(),
                    "seed {seed} step {step}: delta vs rebuild"
                );
                assert_eq!(
                    relation_view(&relation),
                    oracle(&model),
                    "seed {seed} step {step}: relation vs the one-pass adjacency"
                );
            }
        }
    }

    /// The checkpoint's list round-trips: a relation read back from the
    /// rows it wrote equals the one that wrote them.
    #[test]
    fn the_checkpoint_list_round_trips() {
        let a = entry("pages/A.md", "Alpha");
        let b = entry("pages/B.md", "Beta");
        let rows = [
            (
                a.path.clone(),
                AliasRow::of(&a, &["Ax".into(), "Ay".into()]).unwrap(),
            ),
            (b.path.clone(), AliasRow::of(&b, &["Ax".into()]).unwrap()),
        ];
        let relation = AliasRelation::build(rows);
        let back = AliasRelation::from_list(&relation.list(), |rel| {
            Some(PathBuf::from(format!("/g/{rel}")))
        })
        .unwrap();
        assert_eq!(relation.image(), back.image());
        assert!(AliasRelation::from_list(&relation.list(), |_| None).is_none());
        let owners: HashMap<_, _> = back.owner_rows().into_iter().collect();
        assert_eq!(owners["ax"], ["pages/A.md", "pages/B.md"]);
    }
}
