//! Snapshot-owned explicit reference memberships. A source owns sorted compact
//! (panel, block) pairs; panels own their cardinalities. Names select panels,
//! never a second interpretation of references. Cold construction registers
//! targets before building contributions. Successors patch changed sources.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use tine_core::doc::{property_key_norm, DocBlock};
use tine_core::reference_evidence::{visit_reference_targets, ReferenceTarget};
use tine_core::refs;

const CHUNK: u32 = 128;
type Contributions = Arc<[(u32, u32)]>;

fn visit_source_blocks(
    entry: &PageEntry,
    doc: &Document,
    config: &tine_core::Config,
    mut visit: impl FnMut(&DocBlock, u32),
) {
    let parse = tine_core::query::atom::ParseConfig::from_config(config);
    for (ordinal, block) in doc
        .pre_block
        .as_deref()
        .into_iter()
        .flat_map(|pre| crate::query::pre_blocks_with_config(entry, pre, &parse))
        .enumerate()
    {
        visit(&block, ordinal as u32);
    }
    let mut stack: Vec<_> = doc.roots.iter().rev().collect();
    let mut ordinal = 2;
    while let Some(block) = stack.pop() {
        visit(block, ordinal);
        ordinal += 1;
        stack.extend(block.children.iter().rev());
    }
}

fn source_target_names(
    entry: &PageEntry,
    doc: &Document,
    config: &tine_core::Config,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    visit_source_blocks(entry, doc, config, |block, _| {
        for reference in block.projection().reference_source().explicit {
            visit_reference_targets(reference, config, |target| {
                names.insert(match target {
                    ReferenceTarget::PageKey(key) if key != refs::page_key(&reference.name) => {
                        key.to_owned()
                    }
                    _ => reference.name.trim().to_owned(),
                });
                false
            });
        }
    });
    names
}

#[derive(Clone)]
struct Chunks<T> {
    rows: SharedMap<u32, Arc<Vec<T>>>,
}
impl<T> Default for Chunks<T> {
    fn default() -> Self {
        Self {
            rows: SharedMap::new(),
        }
    }
}
impl<T: Clone + Default> Chunks<T> {
    fn get(&self, id: u32) -> Option<&T> {
        self.rows
            .get(&(id / CHUNK))
            .map(|rows| &rows[(id % CHUNK) as usize])
    }
    fn set(&mut self, id: u32, value: T) {
        let chunk = id / CHUNK;
        if !self.rows.contains_key(&chunk) {
            self.rows
                .insert(chunk, Arc::new(vec![T::default(); CHUNK as usize]));
        }
        Arc::make_mut(self.rows.get_mut(&chunk).unwrap())[(id % CHUNK) as usize] = value;
    }
}

#[derive(Clone)]
struct Panel {
    key: Arc<str>,
    own: Arc<str>,
    names: Arc<[Arc<str>]>,
    members: Members,
    journal: bool,
}

/// Persistent sorted leaves: eight bytes per membership, copying at most 128
/// entries and O(log memberships) branch nodes for one insertion/removal.
#[derive(Clone)]
enum Members {
    Leaf(Arc<[(u32, u32)]>),
    Fork(Arc<Fork>),
}
struct Fork {
    left: Members,
    right: Members,
    last: (u32, u32),
    height: u32,
    len: u32,
}
impl Default for Members {
    fn default() -> Self {
        Self::Leaf(Arc::from([]))
    }
}
impl Members {
    fn len(&self) -> u32 {
        match self {
            Self::Leaf(rows) => rows.len() as u32,
            Self::Fork(node) => node.len,
        }
    }
    fn height(&self) -> u32 {
        match self {
            Self::Leaf(_) => 0,
            Self::Fork(node) => node.height,
        }
    }
    fn last(&self) -> (u32, u32) {
        match self {
            Self::Leaf(rows) => *rows.last().unwrap(),
            Self::Fork(node) => node.last,
        }
    }
    fn fork(left: Self, right: Self) -> Self {
        if left.len() == 0 {
            return right;
        }
        if right.len() == 0 {
            return left;
        }
        if let (Self::Leaf(a), Self::Leaf(b)) = (&left, &right) {
            if a.len() + b.len() <= 128 {
                return Self::Leaf(a.iter().chain(b.iter()).copied().collect::<Vec<_>>().into());
            }
        }
        Self::Fork(Arc::new(Fork {
            last: right.last(),
            height: 1 + left.height().max(right.height()),
            len: left.len() + right.len(),
            left,
            right,
        }))
    }
    fn balance(left: Self, right: Self) -> Self {
        if left.height() > right.height() + 1 {
            let Self::Fork(a) = &left else { unreachable!() };
            if a.left.height() >= a.right.height() {
                return Self::fork(a.left.clone(), Self::fork(a.right.clone(), right));
            }
            let Self::Fork(b) = &a.right else {
                unreachable!()
            };
            return Self::fork(
                Self::fork(a.left.clone(), b.left.clone()),
                Self::fork(b.right.clone(), right),
            );
        }
        if right.height() > left.height() + 1 {
            let Self::Fork(a) = &right else {
                unreachable!()
            };
            if a.right.height() >= a.left.height() {
                return Self::fork(Self::fork(left, a.left.clone()), a.right.clone());
            }
            let Self::Fork(b) = &a.left else {
                unreachable!()
            };
            return Self::fork(
                Self::fork(left, b.left.clone()),
                Self::fork(b.right.clone(), a.right.clone()),
            );
        }
        Self::fork(left, right)
    }
    fn update(&self, key: (u32, u32), add: bool) -> Self {
        match self {
            Self::Leaf(rows) => {
                let mut next = rows.to_vec();
                match next.binary_search(&key) {
                    Ok(at) if !add => {
                        next.remove(at);
                    }
                    Err(at) if add => next.insert(at, key),
                    _ => return self.clone(),
                }
                if next.len() <= 128 {
                    Self::Leaf(next.into())
                } else {
                    let right = next.split_off(next.len() / 2);
                    Self::fork(Self::Leaf(next.into()), Self::Leaf(right.into()))
                }
            }
            Self::Fork(node) => {
                if key <= node.left.last() {
                    Self::balance(node.left.update(key, add), node.right.clone())
                } else {
                    Self::balance(node.left.clone(), node.right.update(key, add))
                }
            }
        }
    }
    fn visit(&self, visitor: &mut impl FnMut((u32, u32))) {
        match self {
            Self::Leaf(rows) => {
                for &row in rows.iter() {
                    visitor(row);
                }
            }
            Self::Fork(node) => {
                node.left.visit(visitor);
                node.right.visit(visitor);
            }
        }
    }
}
#[derive(Clone, Default)]
struct Reach {
    panels: Arc<[(u32, u8)]>,
    identities: [Option<u32>; 2],
}
const DIRECT: u8 = 1;
const PROPERTY: u8 = 2;
const IDENTITY: u8 = 4;
impl Reach {
    fn ids(&self, mask: u8) -> impl Iterator<Item = u32> + '_ {
        self.panels
            .iter()
            .filter_map(move |&(id, flags)| (flags & mask != 0).then_some(id))
            .chain(
                self.identities
                    .iter()
                    .filter_map(move |&id| if mask & IDENTITY != 0 { id } else { None }),
            )
    }
}

pub(crate) struct BacklinkMembership {
    sources: Chunks<Contributions>,
    panels: Chunks<Option<Panel>>,
    names: SharedMap<Arc<str>, Reach>,
    free: SharedMap<u32, ()>,
    next: u32,
    resolver: crate::query::PanelResolver,
    /// Final affected panel identities; the publication projects row keys.
    pub(crate) changed: Vec<(bool, String)>,
    base_generation: Option<u64>,
    #[cfg(test)]
    pub(crate) source_visits: usize,
    #[cfg(test)]
    pub(crate) full_builds: usize,
    #[cfg(test)]
    pub(crate) snapshot_reads: std::sync::atomic::AtomicUsize,
}

impl BacklinkMembership {
    fn register_source_targets(&mut self, graph: &ReadSnapshot, entry: &PageEntry, doc: &Document) {
        let config = graph.config();
        if config
            .favorites_page
            .as_deref()
            .is_some_and(|name| refs::page_key(name) == refs::page_key(&entry.name))
        {
            return;
        }
        visit_source_blocks(entry, doc, config, |block, _| {
            for reference in block.projection().reference_source().explicit {
                visit_reference_targets(reference, config, |target| {
                    let (key, flag) = match target {
                        ReferenceTarget::PageKey(key) => (key, DIRECT),
                        ReferenceTarget::PropertyKey(key) => (key, PROPERTY),
                    };
                    if self
                        .names
                        .get(key)
                        .is_some_and(|reach| reach.ids(flag).next().is_some())
                    {
                        return false;
                    }
                    let spelling = match target {
                        ReferenceTarget::PageKey(key) if key != refs::page_key(&reference.name) => {
                            key
                        }
                        _ => reference.name.trim(),
                    };
                    let (canonical, _, _) = self.resolver.resolve(spelling);
                    let journal = self.resolver.format().parse(spelling).is_some();
                    self.register(&canonical, journal, spelling);
                    false
                });
            }
        });
    }
    /// Membership evidence grouped by source, for the panel's materializer.
    /// O(memberships) output; the source and block ordinals are authoritative.
    pub(crate) fn sources_for_panel(&self, id: u32) -> BTreeMap<usize, Vec<u32>> {
        let mut sources = BTreeMap::<usize, Vec<u32>>::new();
        if let Some(panel) = self.panel(id) {
            panel.members.visit(&mut |(source, block)| {
                sources.entry(source as usize).or_default().push(block)
            });
        }
        sources
    }
    fn intern(&self, name: &str) -> Arc<str> {
        self.names
            .iter_from(name)
            .next()
            .filter(|(key, _)| key.as_ref() == name)
            .map_or_else(|| Arc::from(name), |(key, _)| Arc::clone(key))
    }
    fn identity(&self, key: &str, journal: bool) -> Option<u32> {
        self.names.get(key)?.identities[journal as usize]
    }
    fn panel(&self, id: u32) -> Option<&Panel> {
        self.panels.get(id)?.as_ref()
    }
    fn registration(&mut self, name: Arc<str>, id: u32, flag: u8, add: bool) {
        let mut reach = self.names.get(name.as_ref()).cloned().unwrap_or_default();
        if flag == IDENTITY {
            reach.identities[self.panel(id).unwrap().journal as usize] = add.then_some(id);
        } else {
            let mut updated = reach.panels.to_vec();
            match updated.binary_search_by_key(&id, |&(id, _)| id) {
                Ok(at) => {
                    if add {
                        updated[at].1 |= flag;
                    } else {
                        updated[at].1 &= !flag;
                    }
                    if updated[at].1 == 0 {
                        updated.remove(at);
                    }
                }
                Err(at) if add => updated.insert(at, (id, flag)),
                _ => return,
            }
            reach.panels = updated.into();
        }
        if reach.panels.is_empty() && reach.identities.iter().all(Option::is_none) {
            self.names.remove(name.as_ref());
        } else {
            self.names.insert(name, reach);
        }
    }
    fn register(&mut self, name: &str, journal: bool, opening: &str) -> u32 {
        let key = refs::page_key(name);
        if let Some(id) = self.identity(&key, journal) {
            return id;
        }
        let (_, names, own) = self.resolver.resolve(opening);
        let key = self.intern(&key);
        let own = if refs::page_key(&own) == key.as_ref() {
            Arc::clone(&key)
        } else {
            self.intern(&refs::page_key(&own))
        };
        let names: Arc<[Arc<str>]> = names
            .iter()
            .map(|name| {
                if name == key.as_ref() {
                    Arc::clone(&key)
                } else {
                    self.intern(name)
                }
            })
            .collect::<Vec<_>>()
            .into();
        let id = if let Some((&id, _)) = self.free.first() {
            self.free.remove(&id);
            id
        } else {
            let id = self.next;
            self.next += 1;
            id
        };
        self.panels.set(
            id,
            Some(Panel {
                key: Arc::clone(&key),
                own,
                names: Arc::clone(&names),
                members: Members::default(),
                journal,
            }),
        );
        self.registration(key, id, IDENTITY, true);
        for name in names.iter() {
            self.registration(Arc::clone(name), id, DIRECT, true);
            let folded = property_key_norm(name);
            let folded = if folded == name.as_ref() {
                Arc::clone(name)
            } else {
                self.intern(&folded)
            };
            self.registration(folded, id, PROPERTY, true);
        }
        id
    }
    fn unregister(&mut self, id: u32) {
        let Some(panel) = self.panel(id).cloned() else {
            return;
        };
        self.registration(Arc::clone(&panel.key), id, IDENTITY, false);
        for name in panel.names.iter() {
            self.registration(Arc::clone(name), id, DIRECT, false);
            let folded = property_key_norm(name);
            self.registration(Arc::from(folded), id, PROPERTY, false);
        }
        self.panels.set(id, None);
        self.free.insert(id, ());
        let chunk = id / CHUNK;
        if self
            .panels
            .rows
            .get(&chunk)
            .is_some_and(|rows| rows.iter().all(Option::is_none))
        {
            self.panels.rows.remove(&chunk);
            for id in chunk * CHUNK..(chunk + 1) * CHUNK {
                self.free.remove(&id);
            }
        }
    }
    /// Resolve the requested panel to its already registered membership.
    /// Alias-only openings share a real panel only when its self exclusion
    /// and complete name set agree. O(names and reached panels).
    pub(crate) fn selection(&self, names: &[String], own: &str) -> Option<u32> {
        let own = refs::page_key(own);
        self.names
            .get(names.first()?.as_str())?
            .ids(DIRECT)
            .find(|&id| {
                self.panel(id).is_some_and(|panel| {
                    panel.own.as_ref() == own
                        && panel.names.len() == names.len()
                        && panel.names.iter().zip(names).all(|(a, b)| a.as_ref() == b)
                })
            })
    }
    /// One source/block membership lookup; ordinals 0/1 are the header and
    /// preamble, roots start at 2. Binary search over one source's pairs.
    pub(crate) fn contains(&self, slot: usize, panel: u32, ordinal: u32) -> bool {
        self.sources
            .get(slot as u32)
            .is_some_and(|rows| rows.binary_search(&(panel, ordinal)).is_ok())
    }
    /// Cardinalities by the existing CM-COUNT panel identity. O(panels).
    pub(crate) fn scan_all_counts(&self) -> HashMap<(bool, String), u32> {
        #[cfg(test)]
        self.snapshot_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.panels
            .rows
            .values()
            .flat_map(|chunk| chunk.iter().flatten())
            .filter(|panel| panel.members.len() > 0)
            .map(|panel| ((panel.journal, panel.key.to_string()), panel.members.len()))
            .collect()
    }
    pub(crate) fn count(&self, journal: bool, key: &str) -> u32 {
        self.identity(key, journal)
            .and_then(|id| self.panel(id))
            .map_or(0, |panel| panel.members.len())
    }
    fn contributions(
        &self,
        graph: &ReadSnapshot,
        entry: &PageEntry,
        doc: &Document,
    ) -> Contributions {
        let config = graph.config();
        if config
            .favorites_page
            .as_deref()
            .is_some_and(|name| refs::page_key(name) == refs::page_key(&entry.name))
        {
            return Arc::from([]);
        }
        let source = refs::page_key(&entry.name);
        let mut rows = Vec::new();
        let mut collect = |block: &DocBlock, ordinal| {
            for reference in block.projection().reference_source().explicit {
                visit_reference_targets(reference, config, |target| {
                    let (key, flag) = match target {
                        ReferenceTarget::PageKey(key) => (key, DIRECT),
                        ReferenceTarget::PropertyKey(key) => (key, PROPERTY),
                    };
                    for panel in self
                        .names
                        .get(key)
                        .into_iter()
                        .flat_map(|reach| reach.ids(flag))
                    {
                        if self
                            .panel(panel)
                            .is_some_and(|panel| panel.own.as_ref() != source)
                        {
                            rows.push((panel, ordinal));
                        }
                    }
                    false
                });
            }
        };
        visit_source_blocks(entry, doc, config, &mut collect);
        rows.sort_unstable();
        rows.dedup();
        rows.into()
    }
    fn replace_source(&mut self, slot: usize, rows: Contributions, touched: &mut BTreeSet<u32>) {
        let old = self.sources.get(slot as u32).cloned().unwrap_or_default();
        if old == rows {
            return;
        }
        for (side, add) in [(&old, false), (&rows, true)] {
            for &(id, ordinal) in side.iter() {
                let other = if add { &old } else { &rows };
                if other.binary_search(&(id, ordinal)).is_ok() {
                    continue;
                }
                if let Some(mut panel) = self.panel(id).cloned() {
                    panel.members = panel.members.update((slot as u32, ordinal), add);
                    self.panels.set(id, Some(panel));
                    touched.insert(id);
                }
            }
        }
        self.sources.set(slot as u32, rows);
    }
    /// Cold graph-wide construction, invoked only by an explicit panel/count
    /// read. All reference selection is through visit_reference_targets.
    fn scan_all_build(graph: &ReadSnapshot) -> Self {
        let mut answer = Self {
            sources: Chunks::default(),
            panels: Chunks::default(),
            names: SharedMap::new(),
            free: SharedMap::new(),
            next: 0,
            resolver: crate::query::PanelResolver::new(graph),
            changed: Vec::new(),
            base_generation: None,
            #[cfg(test)]
            source_visits: 0,
            #[cfg(test)]
            full_builds: 1,
            #[cfg(test)]
            snapshot_reads: std::sync::atomic::AtomicUsize::new(0),
        };
        for (entry, _) in graph.pages.iter() {
            answer.register(&entry.name, entry.kind == PageKind::Journal, &entry.name);
        }
        for (entry, doc) in graph.pages.iter() {
            answer.register_source_targets(graph, entry, doc);
        }
        let mut touched = BTreeSet::new();
        for (slot, (entry, doc)) in graph.pages.slots() {
            let rows = answer.contributions(graph, entry, doc);
            answer.replace_source(slot, rows, &mut touched);
            #[cfg(test)]
            {
                answer.source_visits += 1;
            }
        }
        answer
    }

    fn succeeding(&self, old: &ReadSnapshot, graph: &ReadSnapshot, paths: &[String]) -> Self {
        let config_changed = old.config.favorites_page != graph.config.favorites_page
            || old.config.property_pages_enabled != graph.config.property_pages_enabled
            || old.config.property_pages_excludelist != graph.config.property_pages_excludelist
            || old.config.ignored_page_references_keywords
                != graph.config.ignored_page_references_keywords
            || old.config.separated_by_commas != graph.config.separated_by_commas
            || old.config.journal_file_name_format != graph.config.journal_file_name_format
            || old.config.journal_page_title_format != graph.config.journal_page_title_format;
        if config_changed || (paths.is_empty() && !Arc::ptr_eq(&old.pages, &graph.pages)) {
            return Self::scan_all_build(graph);
        }
        let mut next = Self {
            sources: self.sources.clone(),
            panels: self.panels.clone(),
            names: self.names.clone(),
            free: self.free.clone(),
            next: self.next,
            resolver: self.resolver.succeeding(old, graph, paths),
            changed: Vec::new(),
            base_generation: Some(old.cache_generation),
            #[cfg(test)]
            source_visits: 0,
            #[cfg(test)]
            full_builds: 0,
            #[cfg(test)]
            snapshot_reads: std::sync::atomic::AtomicUsize::new(0),
        };
        let mut seeds = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for path in paths {
            let page = |snapshot: &ReadSnapshot| {
                snapshot
                    .pages
                    .positions
                    .get(path)
                    .and_then(|&slot| snapshot.pages.get(slot).map(|row| (slot, row.clone())))
            };
            let before = page(old);
            let after = page(graph);
            for (slot, _) in before.iter().chain(&after) {
                sources.insert(*slot);
            }
            let identity = |page: &Option<(usize, (PageEntry, Arc<Document>))>| {
                page.as_ref().map(|(_, (entry, doc))| {
                    (
                        entry.kind,
                        entry.name.clone(),
                        crate::query::document_alias_spellings(doc),
                    )
                })
            };
            if identity(&before) != identity(&after) {
                for (_, (entry, doc)) in before.iter().chain(&after) {
                    seeds.insert(entry.name.clone());
                    seeds.extend(crate::query::document_alias_spellings(doc));
                }
            }
            let names = |page: &Option<(usize, (PageEntry, Arc<Document>))>,
                         snapshot: &ReadSnapshot|
             -> BTreeSet<String> {
                page.as_ref()
                    .map(|(_, (entry, doc))| source_target_names(entry, doc, snapshot.config()))
                    .unwrap_or_default()
            };
            let was = names(&before, old);
            let now = names(&after, graph);
            seeds.extend(was.symmetric_difference(&now).cloned());
        }
        let mut keys = BTreeSet::new();
        for name in seeds {
            keys.insert(refs::page_key(&name));
            for (_, key) in super::namespace_parents::parent_prefixes(&name) {
                keys.insert(key);
            }
            for resolver in [&self.resolver, &next.resolver] {
                keys.extend(resolver.resolve(&name).1);
            }
        }
        let mut ids = BTreeSet::new();
        for key in &keys {
            for lookup in [key.clone(), property_key_norm(key)] {
                if let Some(reach) = self.names.get(lookup.as_str()) {
                    ids.extend(reach.ids(DIRECT | PROPERTY | IDENTITY));
                }
            }
        }
        for &id in &ids {
            if let Some(panel) = self.panel(id) {
                keys.insert(panel.key.to_string());
            }
        }
        // Only keyed real claims and fileless rows; no inventory materialization.
        let mut wanted = BTreeMap::<(bool, String), String>::new();
        for key in &keys {
            for path in graph.real_page_names.paths(key) {
                if let Some(entry) = graph
                    .list
                    .get_path(path)
                    .filter(|entry| graph.pages.positions.contains_key(entry.rel_path_str()))
                {
                    wanted.insert(
                        (entry.kind == PageKind::Journal, refs::page_key(&entry.name)),
                        entry.name.clone(),
                    );
                }
            }
            if let Some(spelling) = graph.explicit_key_spelling(key) {
                let (canonical, names, own) = next.resolver.resolve(&spelling);
                let canonical_key = refs::page_key(&canonical);
                let real = graph.real_page_owners();
                let served = real.opening_entry(key).is_some()
                    || real.opening_entry(&canonical_key).is_some_and(|entry| {
                        let (_, real_names, real_own) = next.resolver.resolve(&entry.name);
                        names == real_names && own == real_own
                    });
                let journal = next.resolver.format().parse(&spelling).is_some();
                if !served && self.identity(&canonical_key, journal).is_some() {
                    wanted.entry((journal, canonical_key)).or_insert(spelling);
                }
            }
        }
        let mut remove = Vec::new();
        let mut add = BTreeMap::new();
        for &id in &ids {
            let panel = self.panel(id).unwrap();
            let identity = (panel.journal, panel.key.to_string());
            let same = wanted.get(&identity).is_some_and(|name| {
                let (_, names, own) = next.resolver.resolve(name);
                panel.own.as_ref() == refs::page_key(&own)
                    && panel
                        .names
                        .iter()
                        .map(|name| name.as_ref())
                        .eq(names.iter().map(String::as_str))
            });
            if !same {
                remove.push(id);
                if let Some(name) = wanted.get(&identity) {
                    add.insert(identity, name.clone());
                }
            }
        }
        for (identity, name) in wanted {
            if self.identity(&identity.1, identity.0).is_none() {
                add.insert(identity, name);
            }
        }
        let mut affected = BTreeSet::new();
        for &id in &remove {
            if let Some(panel) = self.panel(id) {
                affected.insert((panel.journal, panel.key.to_string()));
                panel.members.visit(&mut |(source, _)| {
                    sources.insert(source as usize);
                });
            }
        }
        // A new name can reach an existing panel through property folding.
        // Reuse its reverse evidence; this also covers alias joins/splits.
        for name in add.values() {
            let (_, names, _) = next.resolver.resolve(name);
            for name in names {
                for key in [name.clone(), property_key_norm(&name)] {
                    if let Some(reach) = self.names.get(key.as_str()) {
                        for id in reach.ids(DIRECT | PROPERTY) {
                            if let Some(panel) = self.panel(id) {
                                panel.members.visit(&mut |(source, _)| {
                                    sources.insert(source as usize);
                                });
                            }
                        }
                    }
                }
                for snapshot in [old, graph] {
                    for path in snapshot.real_page_names.paths(&name) {
                        if let Some(entry) = snapshot.list.get_path(path) {
                            if let Some(&slot) = snapshot.pages.positions.get(entry.rel_path_str())
                            {
                                sources.insert(slot);
                            }
                        }
                    }
                }
            }
        }
        let mut touched = BTreeSet::new();
        // Drop old contributions before recycling a panel slot.
        if !remove.is_empty() || !add.is_empty() {
            for &slot in &sources {
                next.replace_source(slot, Arc::from([]), &mut touched);
            }
        }
        for id in remove {
            next.unregister(id);
        }
        for ((journal, key), name) in add {
            next.register(&key, journal, &name);
            affected.insert((journal, key));
        }
        for &slot in &sources {
            if let Some((entry, doc)) = graph.pages.get(slot) {
                next.register_source_targets(graph, entry, doc);
            }
        }
        for slot in sources {
            if let Some((entry, doc)) = graph.pages.get(slot) {
                let rows = next.contributions(graph, entry, doc);
                next.replace_source(slot, rows, &mut touched);
                #[cfg(test)]
                {
                    next.source_visits += 1;
                }
            } else {
                let chunk = slot as u32 / CHUNK;
                if next
                    .sources
                    .rows
                    .get(&chunk)
                    .is_some_and(|rows| rows.iter().all(|rows| rows.is_empty()))
                {
                    next.sources.rows.remove(&chunk);
                }
            }
        }
        for id in touched {
            for answer in [self, &next] {
                if let Some(panel) = answer.panel(id) {
                    affected.insert((panel.journal, panel.key.to_string()));
                }
            }
        }
        next.changed = affected
            .into_iter()
            .filter(|(journal, key)| self.count(*journal, key) != next.count(*journal, key))
            .collect();
        next
    }
}

impl ReadSnapshot {
    /// Changed count identities for the immediate predecessor, or a full
    /// answer comparison when this generation rebuilt the reference policy.
    pub(crate) fn backlink_count_changes_since(&self, old: &Self) -> Option<&[(bool, String)]> {
        let answer = self.backlink_membership.get()?;
        (answer.base_generation == Some(old.cache_generation)).then_some(answer.changed.as_slice())
    }
    #[cfg(test)]
    pub(crate) fn fresh_backlink_counts(&self) -> HashMap<(bool, String), u32> {
        BacklinkMembership::scan_all_build(self).scan_all_counts()
    }
    pub(crate) fn observed_page_mtimes(&self) -> Arc<SharedMap<String, std::time::SystemTime>> {
        Arc::clone(&self.observed_mtimes)
    }
    pub(crate) fn reference_real_page_names(&self) -> Option<Arc<crate::query::RealPageNames>> {
        Some(Arc::clone(&self.real_page_names))
    }
    /// The real-name table ranked by the claimant order (OW-1).
    pub(crate) fn real_page_owners(&self) -> crate::query::RealPageOwners {
        crate::query::RealPageOwners::new(Arc::clone(&self.real_page_names))
    }
    pub(crate) fn resident_backlink_memberships(&self) -> Option<&BacklinkMembership> {
        self.backlink_membership.get()
    }
    /// Candidate source pages from the panel's reverse membership evidence.
    /// O(source pages in the panel), with keyed access to the held documents.
    pub(crate) fn explicit_panel_candidate_pages(
        &self,
        panel: Option<u32>,
    ) -> ReferenceCandidatePages {
        let rows = panel
            .map(|panel| {
                self.scan_all_backlink_memberships()
                    .sources_for_panel(panel)
            })
            .unwrap_or_default();
        ReferenceCandidatePages::unfiltered(
            rows.keys()
                .filter_map(|&slot| self.pages.get(slot).cloned())
                .collect(),
            true,
            #[cfg(test)]
            self.pages.len(),
        )
    }
    pub(super) fn carry_backlink_memberships(&self, old: &Self, paths: &[String]) {
        if let Some(before) = old.backlink_membership.get() {
            let _ = self
                .backlink_membership
                .set(before.succeeding(old, self, paths));
        }
    }
    pub(crate) fn metric_mtime(&self, path: &str) -> Option<i64> {
        i64::try_from(
            self.observed_mtimes
                .get(path)?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs(),
        )
        .ok()
    }
    /// One lazy membership owner per snapshot, inherited by publication.
    pub(crate) fn scan_all_backlink_memberships(&self) -> &BacklinkMembership {
        self.backlink_membership
            .get_or_init(|| BacklinkMembership::scan_all_build(self))
    }
    /// The active authored spelling for an explicit key, or its indexed key
    /// when no authored spelling exists (the explicit_reference_spellings rule).
    pub(crate) fn explicit_key_spelling(&self, key: &str) -> Option<String> {
        self.explicit_index.shards[SnapshotExplicitIndex::shard(key)]
            .get(key)
            .filter(|paths| paths.iter().any(|(path, _)| self.is_active_path(path)))?;
        Some(
            self.referenced_name_table()
                .get(key)
                .map_or(key, |entry| entry.spelling.as_str())
                .to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_tree(tree: &Members, expected: &BTreeSet<(u32, u32)>) {
        fn shape(tree: &Members) -> (u32, u32) {
            match tree {
                Members::Leaf(rows) => {
                    assert!(rows.len() <= 128);
                    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]));
                    (0, rows.len() as u32)
                }
                Members::Fork(node) => {
                    let (lh, ll) = shape(&node.left);
                    let (rh, rl) = shape(&node.right);
                    assert!(lh.abs_diff(rh) <= 1, "balanced persistent leaves");
                    assert_eq!((node.height, node.len), (1 + lh.max(rh), ll + rl));
                    assert_eq!(node.last, node.right.last());
                    (node.height, node.len)
                }
            }
        }
        let mut rows = Vec::new();
        tree.visit(&mut |row| rows.push(row));
        assert_eq!(rows, expected.iter().copied().collect::<Vec<_>>());
        assert_eq!(shape(tree).1 as usize, expected.len());
    }

    #[test]
    fn compact_reverse_memberships_preserve_old_roots_and_rebalance() {
        let mut tree = Members::default();
        let mut expected = BTreeSet::new();
        for source in 0..4096 {
            let key = (source, source % 11);
            tree = tree.update(key, true);
            expected.insert(key);
        }
        let held = tree.clone();
        let held_expected = expected.clone();
        let mut rng = 0x12345678u32;
        for step in 0..8192 {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let key = ((rng >> 8) % 4096, (rng >> 8) % 4096 % 11);
            let add = rng & 1 == 0;
            tree = tree.update(key, add);
            if add {
                expected.insert(key);
            } else {
                expected.remove(&key);
            }
            if step % 97 == 0 {
                assert_tree(&tree, &expected);
            }
        }
        for key in expected.clone() {
            tree = tree.update(key, false);
            expected.remove(&key);
        }
        assert_tree(&tree, &expected);
        assert_tree(&held, &held_expected);
    }
}
