//! The observed property registry (SPEC §6.1–§6.4).
//!
//! One graph-level table computed from property rows and consulted by the
//! builder (vocabulary, operators), the editor, the lowering and the walk
//! (coercion), and TQL diagnostics (suggestions). It is a **disposable,
//! in-memory, graph-scoped cache** (D-3): nothing is persisted and nothing is
//! authoritative — the property lines in the Markdown/Org tree are.
//!
//! **One producer (D-4, D-14).** `build_registry` is the only function that
//! turns property rows into registry rows; every backend state supplies the
//! same [`OwnerRow`] stream and the same same-snapshot page lookup:
//!
//! | backend state | rows | page lookup |
//! |---|---|---|
//! | Direct Files, projection ready | `SqliteGraphProjectionRead::property_facet_rows_after(false, …)` | `pages(page_id, path, name)` in the same snapshot |
//! | Direct Files, not ready | [`crate::query::property_owner_rows`] over the document cache | the page entry's `rel_path` / name |
//! | Managed Storage | `SqliteMaterializedRead::property_facet_rows_after(false, …)` masked, then the overlay iterator | masked and overlaid identically (C4) |
//!
//! The three `Vec<(key, Vec<value>)>` facet wrappers that exist today are NOT
//! sources: they aggregate owner identity away, and owner identity is what
//! gives cardinality and the distinct-owner counts (§6.2).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::config::ParseConfig;
use crate::doc::property_key_norm;
use crate::query::atom::{atom_key, Atom, AtomFormat};
use crate::query::ir::{Cardinality, ObservedType, RegistryRow, RegistrySnapshot};
use crate::refs;

/// The declaration key (Q9): `tine.type:: <type>` on the page whose name is the
/// property key.
pub const DECLARED_TYPE_KEY: &str = "tine.type";

/// Whether a property row's owner is a block or a page. Owner identity is what
/// the registry needs from the row and what today's facet wrappers throw away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OwnerType {
    Block,
    Page,
}

/// One property row exactly as `MaterializedPropertyFacetRow`
/// (`oplog::sqlite_materialization::MaterializedPropertyFacetRow`) streams it,
/// with the ids widened to
/// opaque snapshot-scoped strings so ONE producer serves all three sources:
/// Managed identifies owners by `PageId`/`BlockId` UUIDs, the Direct Files
/// projection by `[u8; 16]`, and the cold document walk has no stored id at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRow {
    pub owner_type: OwnerType,
    pub owner_id: String,
    pub page_id: String,
    /// The raw key as the source line spelled it.
    pub source_name: String,
    /// `property_key_norm(source_name)` — the key rows of one owner group by.
    pub normalized_name: String,
    /// Source order of the property line within its owner — the same number the
    /// physical producers write into `properties.ordinal` (E3).
    pub ordinal: u32,
    pub value: String,
}

/// What the same-snapshot page lookup must answer for every row's `page_id`
/// (§6.2 G3, E4): the format the atomizer parses the value with, and the page's
/// name, which is how a `tine.type::` declaration is bound to its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageMeta {
    pub format: AtomFormat,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A row named a page the snapshot does not contain. This is a
    /// snapshot-consistency defect and fails the build — never a silent
    /// fallback to Markdown (§6.2).
    UnknownPage(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::UnknownPage(page) => {
                write!(
                    formatter,
                    "property row names a page absent from the snapshot: {page}"
                )
            }
        }
    }
}

/// One coherent registry snapshot: the rows, the generation they were built at,
/// and the [`ParseConfig`] digest they were built under (G7 — the generation
/// advances unconditionally when that digest changes).
#[derive(Debug, Clone)]
pub struct Registry {
    rows: Vec<RegistryRow>,
    index: HashMap<String, usize>,
    generation: u64,
    config_digest: tine_storage::ContentDigest,
}

impl Registry {
    /// An empty registry: what every reader sees before the first build, and
    /// what a graph with no properties produces.
    /// The registry a registry-free caller reads: no keys, so no suggestion
    /// and no coercion. Parsing is a pure function of its text plus whatever
    /// registry it is handed; this is the honest "none yet" value, shared so
    /// that handing one in costs nothing.
    pub fn none() -> &'static Registry {
        static NONE: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
        NONE.get_or_init(|| Registry::empty(&ParseConfig::default()))
    }

    pub fn empty(config: &ParseConfig) -> Registry {
        Registry {
            rows: Vec::new(),
            index: HashMap::new(),
            generation: 0,
            config_digest: config.digest(),
        }
    }

    pub fn rows(&self) -> &[RegistryRow] {
        &self.rows
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn config_digest(&self) -> tine_storage::ContentDigest {
        self.config_digest
    }

    pub fn row(&self, key: &str) -> Option<&RegistryRow> {
        self.index
            .get(&property_key_norm(key))
            .map(|index| &self.rows[*index])
    }

    /// The **effective type** of a key (§6.3): the declared `tine.type::`
    /// override if there is one, else the observed majority. `None` for a key
    /// the registry has never seen — the walk then compares as text, which is
    /// what an unknown key can honestly do.
    pub fn effective_type(&self, key: &str) -> Option<ObservedType> {
        self.row(key)
            .map(|row| row.declared.map_or(row.observed_type, |(kind, _)| kind))
    }

    /// The `UnknownIdent` suggestions of §4.2.2: registry keys within
    /// Jaro-Winkler 0.85 of the identifier, written as `prop('…')`, best first.
    pub fn suggestions(&self, ident: &str) -> Vec<String> {
        let needle = atom_key(ident);
        let mut scored: Vec<(f64, &str)> = self
            .rows
            .iter()
            .filter_map(|row| {
                let score = jaro_winkler(&needle, &row.normalized_name);
                (score >= SUGGESTION_THRESHOLD).then_some((score, row.normalized_name.as_str()))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(b.1))
        });
        scored
            .into_iter()
            .map(|(_, name)| format!("prop('{name}')"))
            .collect()
    }

    /// Rebuild a registry from a wire snapshot (§7.1). The command layer reads
    /// the snapshot through whichever storage mode it is bound to and hands it
    /// back to the parser for suggestions; the digest is not part of the wire
    /// shape, so a reconstructed registry carries the default config's digest
    /// and must never be used as a cache key.
    pub fn from_snapshot(snapshot: &RegistrySnapshot) -> Registry {
        let mut registry = Registry::empty(&ParseConfig::default());
        registry.index = snapshot
            .rows
            .iter()
            .enumerate()
            .map(|(position, row)| (row.normalized_name.clone(), position))
            .collect();
        registry.rows = snapshot.rows.clone();
        registry.generation = snapshot.generation;
        registry
    }

    /// The wire snapshot `query_registry` returns (§7.1).
    pub fn snapshot(&self) -> RegistrySnapshot {
        RegistrySnapshot {
            rows: self.rows.clone(),
            generation: self.generation,
        }
    }

    /// Whether two snapshots carry the same rows — the test the lifecycle uses
    /// to decide whether a refresh advances the generation (§6.2).
    pub fn rows_equal(&self, other: &Registry) -> bool {
        self.rows == other.rows
    }

    /// Publish this registry at `generation`.
    pub fn with_generation(mut self, generation: u64) -> Registry {
        self.generation = generation;
        self
    }
}

const SUGGESTION_THRESHOLD: f64 = 0.85;

/// At most eight top values per key (§6.1).
const MAX_TOP_VALUES: usize = 8;

/// Properties that are internal metadata and are never offered as query
/// vocabulary. Exported so the ONE exclusion set has one definition (K15): the
/// registry excludes `INTERNAL_PROPS` ∪ the user's `hidden_properties` ∪ every
/// `tine.*` key.
pub fn is_internal_key(normalized: &str, config: &ParseConfig) -> bool {
    if normalized.starts_with("tine.") {
        return true;
    }
    if crate::query::internal_property_keys()
        .iter()
        .any(|key| property_key_norm(key) == normalized)
    {
        return true;
    }
    config
        .hidden_properties
        .iter()
        .any(|key| property_key_norm(key.trim_start_matches(':')) == normalized)
}

/// Per-owner accumulation while the row stream is consumed.
#[derive(Default)]
struct OwnerGroup {
    /// `(ordinal, value)` of every source row of this (owner, key), so the
    /// §5.8 flattening can sort by ordinal before atomizing (E3).
    rows: Vec<(u32, String)>,
    format: AtomFormat,
    source_name: String,
}

/// The ONE registry producer (§6.2).
///
/// `page_of` must answer for every row's `page_id` **out of the same snapshot
/// the rows came from**; `None` is a snapshot-consistency defect and fails the
/// build with [`RegistryError::UnknownPage`], never a silent Markdown default.
pub fn build_registry(
    rows: impl Iterator<Item = OwnerRow>,
    page_of: &dyn Fn(&str) -> Option<PageMeta>,
    config: &ParseConfig,
) -> Result<Registry, RegistryError> {
    // (normalized key, owner type, owner id) → the owner's source rows.
    let mut groups: BTreeMap<(String, OwnerType, String), OwnerGroup> = BTreeMap::new();
    // page name key → the raw `tine.type::` value on that page.
    let mut declarations: HashMap<String, String> = HashMap::new();
    let mut page_cache: HashMap<String, PageMeta> = HashMap::new();

    for row in rows {
        let meta = match page_cache.get(&row.page_id) {
            Some(meta) => meta.clone(),
            None => {
                let meta = page_of(&row.page_id)
                    .ok_or_else(|| RegistryError::UnknownPage(row.page_id.clone()))?;
                page_cache.insert(row.page_id.clone(), meta.clone());
                meta
            }
        };
        let normalized = if row.normalized_name.is_empty() {
            property_key_norm(&row.source_name)
        } else {
            property_key_norm(&row.normalized_name)
        };
        if normalized.is_empty() {
            continue;
        }
        // The declaration is read BEFORE the internal-key exclusion, because
        // `tine.type` is itself a `tine.*` key and never becomes a registry row.
        if normalized == DECLARED_TYPE_KEY && row.owner_type == OwnerType::Page {
            declarations.insert(refs::page_key(&meta.name), row.value.clone());
        }
        if is_internal_key(&normalized, config) {
            continue;
        }
        let entry = groups
            .entry((normalized, row.owner_type, row.owner_id.clone()))
            .or_insert_with(|| OwnerGroup {
                rows: Vec::new(),
                format: meta.format,
                source_name: row.source_name.clone(),
            });
        entry.rows.push((row.ordinal, row.value));
    }

    // Per key, fold every owner's flattened atom list into the aggregate.
    let mut keys: BTreeMap<String, KeyAccumulator> = BTreeMap::new();
    for ((normalized, owner_type, _owner_id), mut group) in groups {
        group.rows.sort_by(|a, b| a.0.cmp(&b.0));
        let atoms = flatten_owner_atoms(&group, &normalized, config);
        let accumulator = keys.entry(normalized).or_default();
        accumulator.observe_owner(owner_type, &atoms, config);
    }

    let mut rows_out: Vec<RegistryRow> = Vec::new();
    for (normalized, accumulator) in keys {
        let declared = declarations
            .get(&refs::page_key(&normalized))
            .and_then(|value| parse_declaration(value));
        rows_out.push(accumulator.finish(normalized, declared));
    }
    let index = rows_out
        .iter()
        .enumerate()
        .map(|(at, row)| (row.normalized_name.clone(), at))
        .collect();
    Ok(Registry {
        rows: rows_out,
        index,
        generation: 0,
        config_digest: config.digest(),
    })
}

/// §5.8's repeated-row flattening: atomize per source row in ordinal order,
/// concatenate, de-duplicate by [`atom_key`] with first occurrence winning, and
/// renumber `0..n` — so the atom list of a key is ONE union no matter how many
/// source rows spelled it (`k::` twice, `K::` and `k::`).
fn flatten_owner_atoms(group: &OwnerGroup, normalized: &str, config: &ParseConfig) -> Vec<Atom> {
    let key = if group.source_name.is_empty() {
        normalized
    } else {
        group.source_name.as_str()
    };
    let mut out: Vec<Atom> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (_, value) in &group.rows {
        for atom in crate::query::atom::property_atoms(key, value, group.format, config) {
            if seen.insert(atom.key.clone()) {
                let ordinal = out.len() as u32;
                out.push(Atom { ordinal, ..atom });
            }
        }
    }
    out
}

#[derive(Default)]
struct KeyAccumulator {
    count_blocks: u64,
    count_pages: u64,
    histogram: BTreeMap<u8, u64>,
    /// Every owner's atom classes, kept so `mismatch_count` can be computed
    /// against the EFFECTIVE type, which is not known until the declaration is
    /// resolved at the end.
    owner_classes: Vec<Vec<ObservedType>>,
    values: BTreeMap<String, (u64, String)>,
    cardinality_many: bool,
}

fn class_index(kind: ObservedType) -> u8 {
    match kind {
        ObservedType::Text => 0,
        ObservedType::Number => 1,
        ObservedType::Date => 2,
        ObservedType::Checkbox => 3,
        ObservedType::Ref => 4,
    }
}

fn class_of_index(index: u8) -> ObservedType {
    match index {
        1 => ObservedType::Number,
        2 => ObservedType::Date,
        3 => ObservedType::Checkbox,
        4 => ObservedType::Ref,
        _ => ObservedType::Text,
    }
}

impl KeyAccumulator {
    fn observe_owner(&mut self, owner_type: OwnerType, atoms: &[Atom], config: &ParseConfig) {
        match owner_type {
            OwnerType::Block => self.count_blocks += 1,
            OwnerType::Page => self.count_pages += 1,
        }
        if atoms.len() > 1 {
            self.cardinality_many = true;
        }
        let mut classes = Vec::with_capacity(atoms.len());
        for atom in atoms {
            let class = atom.class(config);
            classes.push(class);
            *self.histogram.entry(class_index(class)).or_default() += 1;
            let entry = self
                .values
                .entry(atom.key.clone())
                .or_insert_with(|| (0, atom.text.clone()));
            entry.0 += 1;
        }
        self.owner_classes.push(classes);
    }

    fn finish(
        self,
        normalized_name: String,
        declared: Option<(ObservedType, Cardinality)>,
    ) -> RegistryRow {
        // Observed type: the class with the most atoms, ties → text; a key whose
        // owners have zero atoms in total is `text` (K14).
        let max = self.histogram.values().copied().max().unwrap_or(0);
        let leaders: Vec<u8> = self
            .histogram
            .iter()
            .filter(|(_, count)| **count == max && max > 0)
            .map(|(index, _)| *index)
            .collect();
        let observed_type = match leaders.as_slice() {
            [only] => class_of_index(*only),
            _ => ObservedType::Text,
        };
        let effective = declared.map_or(observed_type, |(kind, _)| kind);
        let mismatch_count = self
            .owner_classes
            .iter()
            .filter(|classes| classes.iter().any(|class| *class != effective))
            .count() as u64;
        let mut histogram: Vec<(ObservedType, u64)> = self
            .histogram
            .iter()
            .map(|(index, count)| (class_of_index(*index), *count))
            .collect();
        histogram.sort_by_key(|(kind, _)| class_index(*kind));
        let mut top_values: Vec<(String, u64)> = self
            .values
            .into_iter()
            .map(|(_, (count, text))| (text, count))
            .collect();
        top_values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top_values.truncate(MAX_TOP_VALUES);
        RegistryRow {
            normalized_name,
            cardinality: if self.cardinality_many {
                Cardinality::Many
            } else {
                Cardinality::One
            },
            observed_type,
            count_blocks: self.count_blocks,
            count_pages: self.count_pages,
            histogram,
            mismatch_count,
            declared,
            top_values,
        }
    }
}

/// `tine.type:: <type>` or `tine.type:: list of <type>` (§6.3).
pub fn parse_declaration(value: &str) -> Option<(ObservedType, Cardinality)> {
    let text = value.trim().to_ascii_lowercase();
    let (cardinality, name) = match text.strip_prefix("list of ") {
        Some(rest) => (Cardinality::Many, rest.trim()),
        None => (Cardinality::One, text.as_str()),
    };
    let kind = match name {
        "text" => ObservedType::Text,
        "number" => ObservedType::Number,
        "date" => ObservedType::Date,
        "checkbox" => ObservedType::Checkbox,
        "ref" => ObservedType::Ref,
        _ => return None,
    };
    Some((kind, cardinality))
}

/// Jaro-Winkler similarity in `[0, 1]` (§4.2.2's ≥ 0.85 suggestion threshold).
///
/// D-14: the repository's only other string-distance function is
/// `sync_diff::similarity` — a normalized *Levenshtein* over capped block first
/// lines, private to the three-way-merge hunk pairing and tuned to a different
/// threshold for a different question ("is this the same block?"). It is a
/// different metric, so the spec's named one is written here rather than the
/// diff's being bent to serve two callers.
fn jaro_winkler(a: &str, b: &str) -> f64 {
    let jaro = jaro(a, b);
    if jaro < 0.7 {
        return jaro;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let prefix = a
        .iter()
        .zip(b.iter())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count() as f64;
    jaro + prefix * 0.1 * (1.0 - jaro)
}

fn jaro(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut a_hit = vec![false; a.len()];
    let mut b_hit = vec![false; b.len()];
    let mut matches = 0usize;
    for (i, ch) in a.iter().enumerate() {
        let low = i.saturating_sub(window);
        let high = (i + window + 1).min(b.len());
        for j in low..high {
            if !b_hit[j] && b[j] == *ch {
                a_hit[i] = true;
                b_hit[j] = true;
                matches += 1;
                break;
            }
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let mut transpositions = 0usize;
    let mut k = 0usize;
    for (i, hit) in a_hit.iter().enumerate() {
        if !hit {
            continue;
        }
        while !b_hit[k] {
            k += 1;
        }
        if a[i] != b[k] {
            transpositions += 1;
        }
        k += 1;
    }
    let matches = matches as f64;
    (matches / a.len() as f64
        + matches / b.len() as f64
        + (matches - transpositions as f64 / 2.0) / matches)
        / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(owner_type: OwnerType, owner: &str, key: &str, ordinal: u32, value: &str) -> OwnerRow {
        OwnerRow {
            owner_type,
            owner_id: owner.into(),
            page_id: "p1".into(),
            source_name: key.into(),
            normalized_name: property_key_norm(key),
            ordinal,
            value: value.into(),
        }
    }

    fn markdown_page(name: &'static str) -> impl Fn(&str) -> Option<PageMeta> {
        move |_| {
            Some(PageMeta {
                format: AtomFormat::Markdown,
                name: name.to_string(),
            })
        }
    }

    fn build(rows: Vec<OwnerRow>) -> Registry {
        build_registry(
            rows.into_iter(),
            &markdown_page("Page"),
            &ParseConfig::default(),
        )
        .unwrap()
    }

    /// The G8 repeated-row flattening fixture (§5.8): `k:: a` twice, `K:: b`,
    /// `k:: a, c` → one atom list `[a, b, c]`, cardinality 3.
    #[test]
    fn repeated_and_case_colliding_rows_flatten_into_one_atom_list() {
        let registry = build(vec![
            row(OwnerType::Block, "b1", "k", 0, "a"),
            row(OwnerType::Block, "b1", "k", 1, "a"),
            row(OwnerType::Block, "b1", "K", 2, "b"),
            row(OwnerType::Block, "b1", "k", 3, "a, c"),
        ]);
        let k = registry.row("k").expect("k");
        assert_eq!(k.cardinality, Cardinality::Many);
        assert_eq!(k.count_blocks, 1);
        let values: Vec<&str> = k.top_values.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(values, vec!["a", "b", "c"]);
    }

    #[test]
    fn observed_type_is_the_majority_class_and_ties_go_to_text() {
        let registry = build(vec![
            row(OwnerType::Block, "b1", "cost", 0, "10"),
            row(OwnerType::Block, "b2", "cost", 0, "20"),
            row(OwnerType::Block, "b3", "cost", 0, "abc"),
        ]);
        assert_eq!(
            registry.row("cost").unwrap().observed_type,
            ObservedType::Number
        );

        let tie = build(vec![
            row(OwnerType::Block, "b1", "k", 0, "10"),
            row(OwnerType::Block, "b2", "k", 0, "abc"),
        ]);
        assert_eq!(tie.row("k").unwrap().observed_type, ObservedType::Text);
    }

    /// K14: a key whose owners have zero atoms in total is `text`, cardinality
    /// `one`, and still counts its owners.
    #[test]
    fn a_blank_only_key_is_text_with_its_owners_counted() {
        let registry = build(vec![
            row(OwnerType::Block, "b1", "k", 0, ""),
            row(OwnerType::Block, "b2", "k", 0, "   "),
        ]);
        let k = registry.row("k").unwrap();
        assert_eq!(k.observed_type, ObservedType::Text);
        assert_eq!(k.cardinality, Cardinality::One);
        assert_eq!(k.count_blocks, 2);
        assert!(k.top_values.is_empty());
    }

    #[test]
    fn block_and_page_owners_are_counted_separately() {
        let registry = build(vec![
            row(OwnerType::Block, "b1", "k", 0, "x"),
            row(OwnerType::Page, "p1", "k", 0, "y"),
            row(OwnerType::Page, "p2", "k", 0, "z"),
        ]);
        let k = registry.row("k").unwrap();
        assert_eq!((k.count_blocks, k.count_pages), (1, 2));
    }

    #[test]
    fn mismatch_count_counts_owners_not_of_the_effective_type() {
        let registry = build(vec![
            row(OwnerType::Block, "b1", "cost", 0, "10"),
            row(OwnerType::Block, "b2", "cost", 0, "20"),
            row(OwnerType::Block, "b3", "cost", 0, "abc"),
        ]);
        assert_eq!(registry.row("cost").unwrap().mismatch_count, 1);
    }

    /// §6.3: the declaration lives on the page whose NAME is the property key,
    /// it wins for coercion, and `mismatch_count` still counts against it.
    #[test]
    fn a_declared_type_wins_for_coercion_and_still_reports_mismatches() {
        let rows = vec![
            OwnerRow {
                owner_type: OwnerType::Page,
                owner_id: "page-cost".into(),
                page_id: "page-cost".into(),
                source_name: "tine.type".into(),
                normalized_name: "tine.type".into(),
                ordinal: 0,
                value: "number".into(),
            },
            OwnerRow {
                page_id: "b-page".into(),
                ..row(OwnerType::Block, "b1", "cost", 0, "abc")
            },
            OwnerRow {
                page_id: "b-page".into(),
                ..row(OwnerType::Block, "b2", "cost", 0, "def")
            },
        ];
        let page_of = |page_id: &str| {
            Some(PageMeta {
                format: AtomFormat::Markdown,
                name: if page_id == "page-cost" {
                    "cost"
                } else {
                    "Notes"
                }
                .to_string(),
            })
        };
        let registry = build_registry(rows.into_iter(), &page_of, &ParseConfig::default()).unwrap();
        let cost = registry.row("cost").unwrap();
        assert_eq!(
            cost.observed_type,
            ObservedType::Text,
            "observed is still text"
        );
        assert_eq!(
            cost.declared,
            Some((ObservedType::Number, Cardinality::One))
        );
        assert_eq!(
            registry.effective_type("cost"),
            Some(ObservedType::Number),
            "declared wins for coercion"
        );
        assert_eq!(
            cost.mismatch_count, 2,
            "both owners mismatch the DECLARED type"
        );
        assert!(
            registry.row("tine.type").is_none(),
            "`tine.*` is never a registry row"
        );
    }

    #[test]
    fn declaration_accepts_the_list_of_spelling() {
        assert_eq!(
            parse_declaration("list of ref"),
            Some((ObservedType::Ref, Cardinality::Many))
        );
        assert_eq!(
            parse_declaration(" Number "),
            Some((ObservedType::Number, Cardinality::One))
        );
        assert_eq!(parse_declaration("enum"), None);
    }

    #[test]
    fn internal_keys_are_excluded_by_the_union_of_three_sets() {
        let mut config = ParseConfig::default();
        config.hidden_properties = vec!["secret".into()];
        let registry = build_registry(
            vec![
                row(OwnerType::Block, "b1", "id", 0, "x"),
                row(OwnerType::Block, "b1", "secret", 0, "x"),
                row(OwnerType::Block, "b1", "tine.view", 0, "table"),
                row(OwnerType::Block, "b1", "status", 0, "open"),
            ]
            .into_iter(),
            &markdown_page("Page"),
            &config,
        )
        .unwrap();
        assert_eq!(registry.rows().len(), 1);
        assert_eq!(registry.rows()[0].normalized_name, "status");
    }

    #[test]
    fn a_row_naming_an_absent_page_fails_the_build() {
        let error = build_registry(
            vec![row(OwnerType::Block, "b1", "k", 0, "x")].into_iter(),
            &|_| None,
            &ParseConfig::default(),
        )
        .unwrap_err();
        assert_eq!(error, RegistryError::UnknownPage("p1".into()));
    }

    /// E4: the format comes from the page, so an Org page's value is atomized
    /// with the Org inline grammar.
    #[test]
    fn the_page_format_decides_the_inline_grammar() {
        let org = build_registry(
            vec![row(OwnerType::Block, "b1", "k", 0, "[[a]]")].into_iter(),
            &|_| {
                Some(PageMeta {
                    format: AtomFormat::Org,
                    name: "Outline".into(),
                })
            },
            &ParseConfig::default(),
        )
        .unwrap();
        assert_eq!(org.row("k").unwrap().observed_type, ObservedType::Ref);
    }

    #[test]
    fn top_values_are_capped_at_eight_by_count() {
        let mut rows = Vec::new();
        for index in 0..12u32 {
            for owner in 0..(12 - index) {
                rows.push(row(
                    OwnerType::Block,
                    &format!("b{index}-{owner}"),
                    "k",
                    0,
                    &format!("v{index:02}"),
                ));
            }
        }
        let registry = build(rows);
        let k = registry.row("k").unwrap();
        assert_eq!(k.top_values.len(), 8);
        assert_eq!(k.top_values[0].0, "v00");
        assert!(k.top_values.windows(2).all(|w| w[0].1 >= w[1].1));
    }

    // --- suggestions (§4.2.2) ---------------------------------------------

    #[test]
    fn suggestions_are_registry_keys_within_the_jaro_winkler_threshold() {
        let registry = build(vec![
            row(OwnerType::Block, "b1", "status", 0, "open"),
            row(OwnerType::Block, "b1", "priority", 0, "high"),
        ]);
        assert_eq!(registry.suggestions("statuz"), vec!["prop('status')"]);
        assert!(registry.suggestions("zzzzzz").is_empty());
    }

    #[test]
    fn jaro_winkler_matches_its_published_reference_values() {
        // The canonical Winkler examples.
        assert!((jaro_winkler("martha", "marhta") - 0.961).abs() < 0.001);
        assert!((jaro_winkler("dwayne", "duane") - 0.840).abs() < 0.001);
        assert!((jaro_winkler("dixon", "dicksonx") - 0.813).abs() < 0.001);
        assert_eq!(jaro_winkler("abc", "abc"), 1.0);
        assert_eq!(jaro_winkler("", ""), 1.0);
        assert_eq!(jaro_winkler("abc", ""), 0.0);
    }

    #[test]
    fn a_registry_records_the_config_digest_it_was_built_under() {
        let mut config = ParseConfig::default();
        config.separated_by_commas = vec!["k".into()];
        let registry = build_registry(
            vec![row(OwnerType::Block, "b1", "k", 0, "a, b")].into_iter(),
            &markdown_page("Page"),
            &config,
        )
        .unwrap();
        assert_eq!(registry.config_digest(), config.digest());
    }
}
