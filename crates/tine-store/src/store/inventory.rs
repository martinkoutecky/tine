//! Name inventory of a whole-graph view: file-claimed names, aliases and
//! reference-only names, the file-claimed subset a page rename needs, and the
//! members of one alias group.
use super::*;

/// Physical names, aliases, and names that occur only in references, sorted by
/// the graph's page identity key. A physical page has one entry per decoded
/// spelling, and each entry's target is exactly what [`WholeGraph::resolve`]
/// returns for that name: the canonical claimant, then every other file
/// claiming the same normalized name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryEntry {
    /// Decoded file name, alias, or referenced name.
    pub name: String,
    /// Current claimant or proposed identity.
    pub target: Resolved,
    /// Whether this entry represents a journal.
    pub is_journal: bool,
    /// Parsed journal day, if any.
    pub day: Option<Day>,
    /// What a `[[name]]` link opens ([`WholeGraph::link_target`]), only when
    /// that differs from this entry's own (name, kind): an alias (its owner),
    /// a date spelling (its day's journal), an alias spelled as a date.
    pub link: Option<LinkTarget>,
    /// Listed only because a listed name lies under it: `n` for a page file
    /// or a reference named `n/y` (model rule 1, SPEC §3.2; OG creates these
    /// pages). Such an entry has no file, no alias and no direct reference;
    /// its target is the absent page its name would create. Views that show
    /// a namespace tree read it instead of re-deriving prefixes.
    pub namespace_parent: bool,
    /// EMPTY-ALIAS (D-20261103-CMA): a file claims this name, but every
    /// claimant has no real content ([`content_free`]) and some page declares
    /// the name as an alias, so `link` is that alias owner. A named route
    /// opens the owner too, as an alias-only name does; only a route to the
    /// exact file (its path) opens the empty page itself.
    pub empty_alias: bool,
}

/// The page a link opens: its name and kind, as the router takes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    /// The name to open (a claimant's own name, or the journal title).
    pub name: String,
    /// A journal (else an ordinary page).
    pub is_journal: bool,
}

fn fileless_entry(
    answer: crate::model::fileless_names::FilelessName,
    target: Resolved,
) -> InventoryEntry {
    InventoryEntry {
        name: answer.name,
        target,
        is_journal: answer.day.is_some(),
        day: answer.day.map(|day| Day(day.ordinal_key())),
        link: None,
        namespace_parent: answer.namespace_parent,
        empty_alias: false,
    }
}

/// Whether a parsed page file has no real content: the one test for
/// EMPTY-ALIAS (D-20261103-CMA), OG's `page-empty-or-dummy?`
/// (`src/main/frontend/db/model.cljs`, `page-empty?` or exactly one block
/// whose `:block/content` is `""`; model round 9 `Model.contentFree`). No
/// text before the first bullet (page properties, a title or free text are
/// content, as OG's pre-block is), and no block or exactly one childless
/// block whose text is empty (`- `). Whitespace is content; two blank blocks
/// are content. Reads the parsed document only: O(1), no parse.
pub(crate) fn content_free(doc: &tine_core::doc::Document) -> bool {
    doc.pre_block.as_deref().is_none_or(str::is_empty)
        && match doc.roots.as_slice() {
            [] => true,
            [only] => only.children.is_empty() && only.raw().is_empty(),
            _ => false,
        }
}

/// Whether the file `id` is parsed in this generation and [`content_free`].
/// A file without a parsed document (unreadable, not yet read) counts as
/// content: it opens itself, as before EMPTY-ALIAS.
fn file_content_free(graph: &ReadSnapshot, id: &str) -> bool {
    graph
        .physical_pages
        .positions
        .get(id)
        .and_then(|&slot| graph.physical_pages.get(slot))
        .is_some_and(|(_, doc)| content_free(doc))
}

/// Whether every file claiming a name is [`content_free`] (model `solid`
/// negated: one claimant with content keeps the name). `false` for none.
fn claim_content_free<'a>(graph: &ReadSnapshot, mut ids: impl Iterator<Item = &'a str>) -> bool {
    let mut any = false;
    let all = ids.all(|id| {
        if graph.physical_pages.positions.contains_key(id)
            && !graph.pages.positions.contains_key(id)
        {
            return true;
        }
        any = true;
        file_content_free(graph, id)
    });
    any && all
}

/// The page the alias `key` opens, if any page declares it: the alias owner
/// rule's one answer (AL2/CI4), of the owner's own kind. A declarer is a
/// parsed page of this generation, so its path is always placed.
fn alias_owner_link(graph: &ReadSnapshot, key: &str) -> Option<LinkTarget> {
    let relation = graph.alias_relation();
    let path = relation.owner_path(key)?;
    let slot = *graph.pages.positions.get(path.as_str())?;
    let (entry, _) = graph.pages.get(slot)?;
    Some(LinkTarget {
        name: entry.name.clone(),
        is_journal: entry.kind == PageKind::Journal,
    })
}

/// What `[[name]]` opens: the one answer (CM step 2, census Q8), used for the
/// inventory's `link` field ([`WholeGraph::inventory`], the publication delta)
/// and by [`WholeGraph::link_target`].
/// - A date by the graph's one date answerer ([`JournalFormat::parse`]): the
///   day's journal file, else an ordinary page of exactly this name, else
///   the owner of an alias spelled so (model rule 4, S6), else the virtual
///   journal ([`JournalFormat::absent_link_is_journal`]).
/// - Any other name: the ordinary page claiming it, else the first alias
///   owner in path order, of the owner's own kind (a journal's alias opens
///   the journal), else an absent ordinary page.
/// - EMPTY-ALIAS (D-20261103-CMA): a claimed file (journal or page) whose
///   claimants are all [`content_free`] yields to the alias owner of the
///   name when some page declares it (whatever the declarer's kind); with no
///   declarer the empty file opens itself.
/// O(1) claimant lookups plus O(owners). `declared` is whether some page
/// declares the name as an alias, when the caller already knows (the full
/// inventory does); `None` reads the alias relation.
pub(crate) fn link_target_in(
    claimants: &SharedMap<(bool, String), Vec<PageEntry>>,
    graph: &ReadSnapshot,
    format: &JournalFormat,
    name: &str,
    declared: Option<bool>,
) -> LinkTarget {
    let key = tine_core::refs::page_key(name);
    let claimed = |journal: bool, key: String| {
        claimants
            .get(&(journal, key))
            .filter(|claimants| !claimants.is_empty())
    };
    let owner = || alias_owner_link(graph, &key);
    // EMPTY-ALIAS: the claimants of (kind, spelling) are all content-free
    // and the typed name has a declarer: the declarer opens. The declarer
    // is looked up first (most claimed names have none), then the claimants'
    // documents.
    let yielded = |claimants: &[PageEntry]| {
        if declared == Some(false) {
            return None;
        }
        let owner = owner()?;
        claim_content_free(
            graph,
            claimants
                .iter()
                .filter_map(|claimant| claimant.rel_path.as_ref().map(PageId::as_str)),
        )
        .then_some(owner)
    };
    let opens = |claimants: &[PageEntry], is_journal: bool| {
        yielded(claimants).unwrap_or_else(|| LinkTarget {
            name: claimants[0].name.clone(),
            is_journal,
        })
    };
    let fallback = crate::model::fileless_names::FilelessName::new(format, name);
    let owners = graph.real_page_owners();
    if let Some(entry) = owners.opening_entry(&key) {
        let all: Vec<_> = [false, true]
            .into_iter()
            .flat_map(|kind| claimed(kind, key.clone()).into_iter().flatten().cloned())
            .collect();
        return yielded(&all).unwrap_or_else(|| LinkTarget {
            name: entry.name.clone(),
            is_journal: entry.kind == PageKind::Journal,
        });
    }
    if fallback.day.is_some() {
        if let Some(journals) = claimed(true, tine_core::refs::page_key(&fallback.name)) {
            if let Some(entry) = owners.opening_entry(&tine_core::refs::page_key(&fallback.name)) {
                if entry.kind == PageKind::Journal {
                    return opens(journals, true);
                }
            }
        }
        if let Some(pages) = claimed(false, key.clone()) {
            return opens(pages, false);
        }
        if let Some(owner) = owner() {
            return owner;
        }
        return LinkTarget {
            name: fallback.name,
            is_journal: true,
        };
    }
    if let Some(pages) = claimed(false, key.clone()) {
        return opens(pages, false);
    }
    owner().unwrap_or_else(|| LinkTarget {
        name: fallback.name,
        is_journal: false,
    })
}

/// `entry` with its `link` set when the link opens something other than the
/// entry itself, and `empty_alias` when that is because the claimed file is
/// content-free. A file-claimed name without a digit is never a date, so its
/// link is itself unless some page declares it as an alias and its files are
/// content-free: one alias-relation lookup (none when `declared` is given),
/// then O(1) document checks.
fn with_link(
    claimants: &SharedMap<(bool, String), Vec<PageEntry>>,
    graph: &ReadSnapshot,
    format: &JournalFormat,
    mut entry: InventoryEntry,
    declared: Option<bool>,
) -> InventoryEntry {
    // EMPTY-ALIAS needs a declarer of the name (checked first: most claimed
    // names have none; `declared` when the caller already knows) and every
    // claimant content-free.
    let empty = match &entry.target {
        Resolved::Existing { id, others } => {
            declared.unwrap_or_else(|| {
                graph
                    .alias_relation()
                    .owner_path(&tine_core::refs::page_key(&entry.name))
                    .is_some()
            }) && claim_content_free(graph, std::iter::once(id).chain(others).map(PageId::as_str))
        }
        _ => false,
    };
    if matches!(entry.target, Resolved::Existing { .. })
        && !empty
        && !entry.name.bytes().any(|byte| byte.is_ascii_digit())
    {
        return entry;
    }
    let link = link_target_in(claimants, graph, format, &entry.name, declared);
    if link.is_journal != entry.is_journal || link.name != entry.name {
        // A content-free file whose link is its alias owner yielded to it;
        // a link that moved to the day's journal moved for the date instead.
        entry.empty_alias = empty
            && alias_owner_link(graph, &tine_core::refs::page_key(&entry.name)).as_ref()
                == Some(&link);
        entry.link = Some(link);
    }
    entry
}

/// Graph inventory entries in page-key order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Inventory(pub Vec<InventoryEntry>);

/// What one publication changed in the `All` inventory when no page file
/// claim changed: the alias and reference-only entries of some page keys
/// (cost-model audit 2026-11-02 row 1). A file-claimed entry never travels
/// here: claims change only with a page created, removed or renamed, which
/// refetches, and so does an EMPTY-ALIAS flip (a declared claimant turning
/// content-free or back, an alias edit naming a content-free claimant).
/// Applying it to the previous inventory (drop every alias and reference-only
/// entry whose key is listed, add `entries`) gives the new inventory exactly. Size O(changed keys).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct InventoryDelta {
    /// Page keys whose alias or reference-only entry changed, ascending.
    pub(crate) keys: Vec<String>,
    /// Their alias or reference-only entries now, in inventory order (a key
    /// that left the inventory, or is now only file-claimed, has none).
    pub(crate) entries: Inventory,
}

/// Which names a [`WholeGraph::inventory`] lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryScope<'a> {
    /// The file-claimed name of this page in this view, if present.
    File(&'a PageId),
    /// Every name: file-claimed names, aliases and reference-only names.
    All,
    /// Physical files in conflicting claim buckets, opening file first in
    /// each bucket. Each row targets its own file; `None` lists every bucket,
    /// and named refreshes visit only those keys. No document or disk reads.
    ConflictingFiles(Option<&'a [String]>),
    /// Frozen published `answers` v1 lookup rows for referenced spelling keys
    /// absent from this `All` inventory. These retain the old ordinary-row
    /// shape; their `link` uses the shared link answer. Not a live entity list.
    PublishedV1Spellings(&'a Inventory),
    /// Referenced journal dates, including dates without a file, once per day.
    /// Uses the explicit-reference index; excludes ordinary date-page identity.
    ReferencedJournals,
    /// Only the file-claimed names whose page key is this key, or lies under
    /// `key/` and is a namespace name (a page and its namespace descendants):
    /// what a rename moves.
    FilesAtOrUnder(&'a str),
    /// The other members of this name's alias group (D-20261019-AL1), as `All` lists them.
    AliasGroup(&'a str),
    /// Every journal day a file claims in this view, once per day: the
    /// view's day index, not a directory listing.
    JournalFiles,
}

impl WholeGraph {
    /// Physical conflict buckets, ordered with the opening claimant first.
    /// `names` limits refreshes to affected keys (O(claimants log P)); `None`
    /// enumerates all keys once for the initial conflict view. No file reads.
    fn claim_conflicts(&self, names: Option<&[String]>) -> Vec<Vec<PageEntry>> {
        let keys: std::collections::BTreeSet<_> = match names {
            Some(names) => names
                .iter()
                .map(|name| tine_core::refs::page_key(name))
                .collect(),
            None => self
                .claimants
                .iter()
                .map(|((_, key), _)| key.clone())
                .collect(),
        };
        keys.into_iter()
            .filter_map(|key| {
                let claims = self.all_name_claimants(&key);
                (claims.len() > 1).then_some(claims)
            })
            .collect()
    }

    /// Direct declaring owners when EMPTY-ALIAS makes this a redirecting name.
    /// Rename uses the same content-free answer as navigation, but requires a
    /// unique declarer before offering a merge. Indexed lookups, no file I/O.
    pub fn redirecting_alias_owners(&self, name: &str) -> Option<Vec<PageId>> {
        let key = tine_core::refs::page_key(name);
        let owners = self.graph.alias_owner_paths(&key)?;
        if owners.is_empty() {
            return None;
        }
        let solid = |journal: bool, key: String| {
            self.claimants.get(&(journal, key)).is_some_and(|claims| {
                claims.iter().any(|claim| {
                    claim
                        .rel_path
                        .as_ref()
                        .is_none_or(|id| !file_content_free(&self.graph, id.as_str()))
                })
            })
        };
        if solid(false, key.clone())
            || solid(true, key)
            || self.journal_format.parse(name).is_some_and(|day| {
                solid(
                    true,
                    tine_core::refs::page_key(&self.journal_format.title(day)),
                )
            })
        {
            return None;
        }
        Some(owners.iter().cloned().map(PageId::from).collect())
    }

    /// Names and file claimants in this view. Physical twins with the same
    /// decoded spelling produce one name entry whose target contains the
    /// canonical id and the other claimants; `scan_area` lists physical files.
    /// Only the opening claimant contributes parsed search and backlink
    /// content. Every physical row remains available by file id; reading a
    /// losing file does not activate it or publish a change. Losing claimants
    /// appear as `others` of the name's `Resolved::Existing` target.
    /// A first call can traverse all
    /// page blocks and reference text to build indexes; later calls cost
    /// O(P + B + aliases + R + N log N) on the first call, where R is
    /// reference-only names and N is inventory entries. Alias owners are
    /// indexed once per snapshot instead of rescanned for every name.
    /// The result is stable in this view.
    ///
    /// [`InventoryScope::FilesAtOrUnder`] returns only the file-claimed
    /// entries (pages and journals, `is_journal` telling them apart) whose
    /// page key is the given key or lies under `key/` with a namespace name
    /// (`refs::is_namespace_name`): a page and its namespace descendants,
    /// each with the same file target and in the same relative order as the
    /// full inventory. An alias or reference-only name
    /// is never a file claim (the full inventory skips both for a claimed
    /// key), so this is exactly the file-claimed subset a rename moves,
    /// without building the alias and reference-name indexes (GH #623). Cost
    /// O(P) page-key computations plus O(m log m) for m matches; no parse and
    /// no disk access.
    ///
    /// [`InventoryScope::ReferencedJournals`] lists only days mentioned by
    /// explicit references, with accepted date spellings merged into the
    /// configured journal title. Uses published reference/alias keys, not
    /// document scans; O(R log D + indexed alias lookups), R reference names,
    /// D distinct dates. Ordinary date-named pages retain ordinary identity.
    ///
    /// [`InventoryScope::AliasGroup`] lists the other names of this name's
    /// alias group (the transitive closure of `alias::` declarations, AL1),
    /// each as `All` lists it: a name a file claims has an `Existing` target,
    /// a name with no file `Alias { owners }`. A name in no group lists
    /// nothing. The page header shows them (D-20261019-AL2). Cost
    /// O(component log P) over the snapshot's alias edges and real-name
    /// table; no parse and no disk access.
    /// [`InventoryScope::JournalFiles`] lists one entry per journal day that a
    /// file claims in this view (duplicate-day strays are `others` of the
    /// day's target, as in `All`), from the view's day index, which
    /// publications keep per file: O(J log J) for J journal days, no parse and
    /// no disk access (cost audit 2026-11-02 row 8: the Journals feed pages
    /// through it instead of listing the journals directory per step).
    ///
    /// [`InventoryScope::File`] uses the existing page-position map and
    /// resolves its name in this view: O(log P + C) for C name claimants,
    /// with no document traversal or file reads. An absent page returns empty.
    pub fn inventory(&self, scope: InventoryScope<'_>) -> Arc<Inventory> {
        match scope {
            InventoryScope::File(id) => Arc::new(Inventory(
                self.graph
                    .physical_pages
                    .positions
                    .get(id.as_str())
                    .and_then(|&slot| self.graph.physical_pages.get(slot))
                    .map(|(entry, _)| InventoryEntry {
                        name: entry.name.clone(),
                        target: self.resolve(&entry.name, entry.kind == PageKind::Journal),
                        is_journal: entry.kind == PageKind::Journal,
                        day: entry.date_key.map(Day),
                        link: None,
                        namespace_parent: false,
                        empty_alias: false,
                    })
                    .into_iter()
                    .collect(),
            )),
            InventoryScope::All => self.full_inventory(),
            InventoryScope::ConflictingFiles(names) => Arc::new(Inventory(
                self.claim_conflicts(names)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| {
                        Some(InventoryEntry {
                            name: entry.name,
                            target: Resolved::Existing {
                                id: entry.rel_path?,
                                others: Vec::new(),
                            },
                            is_journal: entry.kind == PageKind::Journal,
                            day: entry.date_key.map(Day),
                            link: None,
                            namespace_parent: false,
                            empty_alias: false,
                        })
                    })
                    .collect(),
            )),
            InventoryScope::PublishedV1Spellings(inventory) => {
                let listed: HashSet<_> = inventory
                    .0
                    .iter()
                    .map(|entry| tine_core::refs::page_key(&entry.name))
                    .collect();
                Arc::new(Inventory(
                    self.graph
                        .referenced_name_table()
                        .iter()
                        .filter(|(key, _)| !listed.contains(*key))
                        .map(|(_, referrers)| {
                            let name = referrers.spelling.clone();
                            with_link(
                                &self.claimants,
                                &self.graph,
                                &self.journal_format,
                                InventoryEntry {
                                    target: self.resolve(&name, false),
                                    name,
                                    is_journal: false,
                                    day: None,
                                    link: None,
                                    namespace_parent: false,
                                    empty_alias: false,
                                },
                                None,
                            )
                        })
                        .collect(),
                ))
            }
            InventoryScope::ReferencedJournals => {
                let mut entries: Vec<_> = self
                    .referenced_journal_days()
                    .into_iter()
                    .map(|day| {
                        let name = self.journal_format.title(JournalDate::from_ordinal(day.0));
                        InventoryEntry {
                            target: self.resolve(&name, true),
                            name,
                            is_journal: true,
                            day: Some(day),
                            link: None,
                            namespace_parent: false,
                            empty_alias: false,
                        }
                    })
                    .collect();
                sort_inventory(&mut entries);
                Arc::new(Inventory(entries))
            }
            InventoryScope::FilesAtOrUnder(key) => {
                Arc::new(Inventory(self.page_files_at_or_under(key)))
            }
            InventoryScope::AliasGroup(name) => Arc::new(Inventory(self.alias_group(name))),
            InventoryScope::JournalFiles => {
                let mut entries: Vec<_> = self
                    .list
                    .journal_days()
                    .filter_map(|page| {
                        self.claimed_entries(page, tine_core::refs::page_key(&page.name))
                            .into_iter()
                            .next()
                    })
                    .collect();
                sort_inventory(&mut entries);
                Arc::new(Inventory(entries))
            }
        }
    }

    /// What a `[[name]]` link opens in this view ([`link_target_in`]): the one
    /// answer; the frontend reads it from the inventory's `link` field.
    /// Production reads the same answer from the inventory's `link` field;
    /// this per-name form is the conformance harnesses' oracle.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn link_target(&self, name: &str) -> LinkTarget {
        link_target_in(
            &self.claimants,
            &self.graph,
            &self.journal_format,
            name,
            None,
        )
    }

    fn full_inventory(&self) -> Arc<Inventory> {
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        let mut page_claimed_names = HashSet::new();
        let aliases: Vec<_> = self
            .graph
            .page_aliases_with_owners()
            .into_iter()
            .map(|(alias, _, owner)| (tine_core::refs::page_key(&alias), alias, owner))
            .collect();
        // Declared alias keys, so a claimed entry's EMPTY-ALIAS check needs
        // no per-entry alias-relation lookup.
        let declared: HashSet<&str> = aliases.iter().map(|(key, _, _)| key.as_str()).collect();
        let mut claimed = Vec::new();
        for page in self.list.iter() {
            let key = tine_core::refs::page_key(&page.name);
            if !visited.insert(key.clone()) {
                continue;
            }
            for entry in self.claimed_entries(page, key) {
                let key = tine_core::refs::page_key(&entry.name);
                if !entry.is_journal {
                    page_claimed_names.insert(key.clone());
                }
                claimed.push((entry, Some(declared.contains(key.as_str()))));
            }
        }
        let references = self.graph.referenced_name_table();
        let mut alias_owners: BTreeMap<String, (String, Vec<PageId>)> = BTreeMap::new();
        for (key, alias, owner) in aliases {
            if page_claimed_names.contains(&key) {
                continue;
            }
            let spelling = references
                .get(&key)
                .map_or(alias.as_str(), |referrers| referrers.spelling.as_str())
                .to_owned();
            let slot = alias_owners
                .entry(key)
                .or_insert_with(|| (spelling, Vec::new()));
            let owner = PageId::from(owner);
            if !slot.1.contains(&owner) {
                slot.1.push(owner);
            }
        }
        for (_, (name, mut owners)) in alias_owners {
            owners.sort();
            entries.push(InventoryEntry {
                name,
                target: Resolved::Alias { owners },
                is_journal: false,
                day: None,
                link: None,
                namespace_parent: false,
                empty_alias: false,
            });
        }
        entries.extend(self.graph.reference_only_pages().into_iter().map(|answer| {
            let target = self.resolve(&answer.name, answer.day.is_some());
            fileless_entry(answer, target)
        }));
        let mut entries: Vec<_> = claimed
            .into_iter()
            .chain(entries.into_iter().map(|entry| (entry, None)))
            .map(|(entry, declared)| {
                with_link(
                    &self.claimants,
                    &self.graph,
                    &self.journal_format,
                    entry,
                    declared,
                )
            })
            .collect();
        sort_inventory(&mut entries);
        #[cfg(feature = "test-faults")]
        crate::cost_counters::name_inventory_entries(entries.len());
        Arc::new(Inventory(entries))
    }

    /// [`InventoryScope::FilesAtOrUnder`]'s entries; see [`Self::inventory`].
    /// A name under `key/` is a descendant only when it is a namespace name
    /// (`refs::is_namespace_name`, OG `namespace-page?`; model round 10, S15):
    /// `./x` is no child of `.`, `https://a.b/c` none of `https:`.
    fn page_files_at_or_under(&self, key: &str) -> Vec<InventoryEntry> {
        let prefix = format!("{key}/");
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        for page in self.list.iter() {
            let page_key = tine_core::refs::page_key(&page.name);
            if (page_key != key
                && !(page_key.starts_with(&prefix)
                    && tine_core::refs::is_namespace_name(&page.name)))
                || !visited.insert(page_key.clone())
            {
                continue;
            }
            entries.extend(self.claimed_entries(page, page_key));
        }
        sort_inventory(&mut entries);
        entries
    }

    /// [`InventoryScope::AliasGroup`]'s entries; see [`Self::inventory`].
    fn alias_group(&self, name: &str) -> Vec<InventoryEntry> {
        let mut entries = Vec::new();
        for (key, spelling) in crate::query::alias_group_members(&self.graph, name) {
            let claimant = [false, true].into_iter().find_map(|journal| {
                let claimants = self.claimants.get(&(journal, key.clone()))?;
                claimants.first().cloned()
            });
            match claimant {
                Some(page) => entries.extend(self.claimed_entries(&page, key)),
                None => entries.push(InventoryEntry {
                    target: self.resolve(&spelling, false),
                    name: spelling,
                    is_journal: false,
                    day: None,
                    link: None,
                    namespace_parent: false,
                    empty_alias: false,
                }),
            }
        }
        sort_inventory(&mut entries);
        entries
    }

    /// One entry per spelling of `page`'s claimant bucket (`key` is its page
    /// key), each with the target `resolve` gives: the bucket's first
    /// claimant, then every other claimant.
    fn claimed_entries(&self, page: &PageEntry, _key: String) -> Vec<InventoryEntry> {
        let claimants = self.all_name_claimants(&page.name);
        let winner = claimants.first().unwrap_or(page);
        let is_journal = winner.kind == PageKind::Journal;
        let mut names: Vec<String> = Vec::new();
        let mut ids: Vec<PageId> = Vec::new();
        {
            for claimant in &claimants {
                if let Some(id) = &claimant.rel_path {
                    if !names.contains(&claimant.name) {
                        names.push(claimant.name.clone());
                    }
                    ids.push(id.clone());
                }
            }
        }
        names
            .into_iter()
            .map(|name| InventoryEntry {
                name,
                target: Resolved::Existing {
                    id: ids[0].clone(),
                    others: ids[1..].to_vec(),
                },
                is_journal,
                day: winner.date_key.map(Day),
                link: None,
                namespace_parent: false,
                empty_alias: false,
            })
            .collect()
    }
}

impl Snapshot {
    /// The alias, reference-only or namespace-parent `All` entry of page key
    /// `key` in this generation, exactly as [`WholeGraph::inventory`] lists
    /// it: an alias entry unless a page file claims the key, else a
    /// reference-only entry unless any file (page or journal) claims it,
    /// else a namespace parent when a listed name lies under it.
    /// Ordinary keys: O(log V + owners), plus the names starting with `key`
    /// when no reference names it. A canonical fileless day checks the resident
    /// reference/namespace support for that day (O(log days + its spellings)).
    pub(super) fn name_entry(
        &self,
        table: &crate::model::referenced_names::ReferencedNameTable,
        key: &str,
        day: Option<JournalDate>,
    ) -> Option<InventoryEntry> {
        let claimed = |journal: bool| {
            self.claimants
                .get(&(journal, key.to_owned()))
                .is_some_and(|claimants| claimants.iter().any(|c| c.rel_path.is_some()))
        };
        if claimed(false) {
            return None;
        }
        let spelling = table.get(key).map(|referrers| referrers.spelling.as_str());
        if let Some(owners) = self
            .graph
            .alias_owner_paths(key)
            .filter(|owners| !owners.is_empty())
        {
            return Some(self.linked(InventoryEntry {
                name: spelling.unwrap_or(key).to_owned(),
                target: Resolved::Alias {
                    owners: owners.into_iter().map(PageId::from).collect(),
                },
                is_journal: false,
                day: None,
                link: None,
                namespace_parent: false,
                empty_alias: false,
            }));
        }
        if claimed(true) {
            return None;
        }
        let fallback = crate::model::fileless_names::FilelessName::new(
            &self.journal_format,
            spelling.unwrap_or(key),
        );
        if let Some(day) = day.or(fallback.day) {
            // Deltas remove/add the canonical entity key, never a raw date
            // spelling. Another spelling or a namespace child can keep it
            // alive after the edited page stops referring to it.
            if tine_core::refs::page_key(&self.journal_format.title(day)) != key {
                return None;
            }
            return self.graph.fileless_page_for_day(day).map(|answer| {
                fileless_entry(
                    answer,
                    Resolved::Absent {
                        id: proposed_journal_id(
                            &self.config.config,
                            self.journal_format.file_stem(day),
                        ),
                    },
                )
            });
        }
        // A key no reference names may still be a namespace parent.
        let (name, namespace_parent) = match spelling {
            Some(spelling) => (spelling, false),
            None => (self.graph.namespace_seed(table, key)?, true),
        };
        let mut answer = self.graph.fileless_name(&self.journal_format, name)?;
        if answer.day.is_some() {
            return None; // its canonical day key is handled above
        }
        answer.namespace_parent = namespace_parent;
        let target = Resolved::Absent {
            id: proposed_page_id(&self.config.config, &answer.name),
        };
        Some(self.linked(fileless_entry(answer, target)))
    }

    /// Whether these claimants of one name are all content-free files
    /// ([`content_free`], EMPTY-ALIAS).
    pub(super) fn claim_content_free(&self, claimants: &[PageEntry]) -> bool {
        claim_content_free(
            &self.graph,
            claimants
                .iter()
                .filter_map(|claimant| claimant.rel_path.as_ref().map(PageId::as_str)),
        )
    }

    fn linked(&self, entry: InventoryEntry) -> InventoryEntry {
        with_link(
            &self.claimants,
            &self.graph,
            &self.journal_format,
            entry,
            None,
        )
    }
}

/// The file a page named `name` would be created as: the configured pages
/// directory, the encoded name and the preferred format (`resolve`'s absent
/// answer for an ordinary page).
pub(crate) fn proposed_page_id(config: &tine_core::config::Config, name: &str) -> PageId {
    let dir = &config.pages_dir;
    let stem = tine_core::model::encode_page_name(name, config.file_name_format);
    PageId::from(format!("{dir}/{stem}.{}", config.preferred_format.ext()))
}

fn sort_inventory(entries: &mut [InventoryEntry]) {
    entries
        .sort_by_cached_key(|entry| (tine_core::refs::page_key(&entry.name), entry.name.clone()));
}
