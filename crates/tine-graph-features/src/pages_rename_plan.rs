//! Read-only rename planning, shared by the single-page and namespace paths.
use super::*;

#[cfg(test)]
mod planning_probe {
    use super::*;

    #[test]
    #[ignore = "requires a generated realistic-10k graph in TINE_RENAME_PROBE_ROOT"]
    fn realistic_10k() {
        let root = std::env::var("TINE_RENAME_PROBE_ROOT").unwrap();
        let old = std::env::var("TINE_RENAME_PROBE_OLD").unwrap();
        let store = Store::open(std::path::Path::new(&root), Default::default())
            .unwrap()
            .0;
        store.whole_graph().unwrap();
        let mut samples = Vec::new();
        for _ in 0..21 {
            let start = std::time::Instant::now();
            let plan = plan_rename(
                &store,
                &old,
                "Rename probe destination",
                None,
                RenameChoice::Rename,
                &|| {},
            )
            .unwrap();
            samples.push(start.elapsed().as_micros());
            std::hint::black_box(plan);
        }
        samples.sort_unstable();
        println!("planning_us median={} samples={samples:?}", samples[10]);
        let graph = store.whole_graph().unwrap();
        let mut guard = Vec::new();
        for _ in 0..21 {
            let start = std::time::Instant::now();
            std::hint::black_box(graph.journal_rewrite_risks(&old, "Rename probe destination"));
            guard.push(start.elapsed().as_micros());
        }
        guard.sort_unstable();
        println!("journal_guard_us median={}", guard[10]);
    }
}

/// What one rename attempt writes, planned from one published view (the
/// attempt's walk, `refreshed_view`, decides reuse) and the config captured with it;
/// commit and the host admit it only while both are current (A-W1, B2).
pub(super) struct RenamePlan {
    pub(super) graph: tine_store::WholeGraph,
    pub(super) config: tine_store::ConfigState,
    pub(super) map: RenameMap,
    pub(super) lookup: HashMap<String, String>,
    /// `map.1` keyed so: query arguments naming a merged page (ruling (a)).
    pub(super) query_lookup: HashMap<String, String>,
    pub(super) moves: HashMap<PageId, FileId>,
    /// Planned identities (including a fileless primary), in `map.0` order.
    /// The transaction adds reference-only identities from its applied edits.
    pub(super) renamed: Vec<RenamedPage>,
    pub(super) moved_titles_to_rebind: HashSet<PageId>,
    pub(super) merge: Option<(PageId, PageId)>,
    pub(super) ref_merge: bool,
    pub(super) candidates: Vec<PageId>,
    /// The candidates `pages` read to find the plan's write set, reused by
    /// the commit so it writes exactly what was planned (A-R3).
    pub(super) reads: HashMap<PageId, (String, FileRev)>,
    /// Referrers `pages` leaves byte-identical for their VCS conflict markers.
    pub(super) skipped: Vec<PageId>,
    /// The new text of each referrer `pages` found changed, computed from
    /// its `reads` bytes; the host reuses it on equal bytes (GH #623).
    pub(super) rewritten: HashMap<PageId, refs::RenameProposal>,
}

impl RenamePlan {
    /// Select from held buffers too: an unchanged unsaved referrer is not
    /// ours to flush; a changed one refuses. Admission guards this snapshot.
    pub(super) fn include_inputs(
        &mut self,
        store: &Store,
        inputs: &tine_store::HostInputs,
    ) -> io::Result<()> {
        let source = self.single(store)?;
        for (id, bytes, clean) in inputs.pages() {
            if source.as_ref() == Some(id) && bytes.is_some() && !clean {
                return Err(unsaved_error(&self.graph, id.as_str()));
            }
            if self.moves.contains_key(id) || self.moves.values().any(|to| *to == id.file()) {
                continue;
            }
            let Some(bytes) = bytes else {
                self.candidates.retain(|candidate| candidate != id);
                continue;
            };
            let text = std::str::from_utf8(bytes).map_err(|_| unwritable(store, id))?;
            let format = Format::from_path(id.as_str().as_ref());
            let changed = std::panic::catch_unwind(|| {
                refs::rename_rewrite_with_queries(
                    text,
                    format == Format::Org,
                    &self.map.0,
                    &self.map.1,
                    self.config.file_name_format,
                    None,
                )
            })
            .map_err(|_| unwritable(store, id))?
                != text;
            let marked = !tine_core::concord_queue::vcs_conflict_markers(text, format).is_empty();
            if changed && !marked && !clean {
                return Err(unsaved_error(&self.graph, id.as_str()));
            }
            if changed
                && marked
                && source.is_some()
                && format == Format::Org
                && !tine_core::org::org_editable(text)
            {
                return Err(unwritable(store, id));
            }
            if changed && marked {
                self.skipped.push(id.clone());
            }
            if marked || (!changed && !clean) {
                self.candidates.retain(|candidate| candidate != id);
            } else if changed {
                self.candidates.push(id.clone());
            }
        }
        self.candidates
            .sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        self.candidates.dedup();
        Ok(())
    }

    /// Every page the plan writes (A-R3): moved pages and their
    /// destinations, interrupted titles, a merge's survivor and source,
    /// and the referrers whose references it changes. A referrer the
    /// rename leaves byte-identical (unchanged, or carrying VCS conflict
    /// markers) is not written, so its unsaved input blocks nothing. The
    /// reads are kept for the commit, which decides on the same bytes.
    pub(super) fn pages(&mut self, store: &Store) -> io::Result<Vec<PageId>> {
        let format = self.config.file_name_format;
        let mut pages = Vec::new();
        for id in &self.candidates {
            if self.moves.contains_key(id) || self.moved_titles_to_rebind.contains(id) {
                pages.push(id.clone());
                continue;
            }
            let file = id.file();
            let (content, rev) = read_text(store, &file)
                .map_err(|e| error(e.kind(), &format!("{}: {e}", file.as_str())))?;
            // A referrer is written when the
            // transaction's rewrite (using the page's source format)
            // changes it and it carries no VCS conflict markers.
            let syntax = Format::from_path(id.as_str().as_ref());
            let org = syntax == Format::Org;
            let rewritten =
                refs::rename_proposal(&content, org, &self.lookup, &self.query_lookup, format)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let changed = rewritten.text != content;
            let marked =
                !tine_core::concord_queue::vcs_conflict_markers(&content, syntax).is_empty();
            match (changed, marked) {
                (true, false) => {
                    pages.push(id.clone());
                    self.rewritten.insert(id.clone(), rewritten);
                }
                (true, true) => self.skipped.push(id.clone()),
                _ => {}
            }
            self.reads.insert(id.clone(), (content, rev));
        }
        pages.extend(crate::retained::pages(store, self.moves.values()));
        pages.extend(
            self.merge
                .iter()
                .flat_map(|(src, dst)| [src.clone(), dst.clone()]),
        );
        Ok(pages)
    }

    /// The source of a single page's rename, which a host runs
    /// as one page operation (STEP3 §7, F10): one name, no namespace
    /// descendant, no merge, no interrupted title. A source with no file
    /// renames its references only, under the default Markdown spelling.
    pub(super) fn single(&self, store: &Store) -> io::Result<Option<PageId>> {
        if self.merge.is_some()
            || self.ref_merge
            || !self.moved_titles_to_rebind.is_empty()
            || self.map.0.len() != 1
        {
            return Ok(None);
        }
        if let Some(source) = self.moves.keys().next() {
            return Ok(Some(source.clone()));
        }
        let rel = format!(
            "{}.md",
            tine_core::model::encode_page_name(&self.map.0[0].0, self.config.file_name_format)
        );
        let file = store.file_id(Area::Pages, &rel).map_err(store_error)?;
        store
            .as_page(&file)
            .map(Some)
            .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))
    }
}

/// Whether unindexed `file` holds a reference `map` renames. A file too big or
/// too deep to hand the rewriter, or that panics it, is searched for the names
/// themselves, so one stray file cannot block every rename. Bytes past a UTF-8
/// error still spell references. A file gone holds none.
fn may_hold_reference(
    store: &Store,
    file: &FileId,
    map: &RenameMap,
    format: tine_core::config::FileNameFormat,
) -> bool {
    let bytes = match store.read(file, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
        Ok((bytes, _)) => bytes,
        Err(StoreError::NotFound(_)) => return false,
        Err(StoreError::TooLarge { .. }) => {
            return store
                .path_for_os_handoff(file, false)
                .ok()
                .and_then(|path| file_mentions_a_name(&path, map).ok())
                .unwrap_or(true)
        }
        Err(_) => return true,
    };
    let text = String::from_utf8_lossy(&bytes);
    let org = Format::from_path(file.as_str().as_ref()) == Format::Org;
    if !tine_store::parse_input_depth_within_limit(&text)
        || (org && !tine_core::org::headline_levels_within_limit(&text, 128))
    {
        return text_mentions_a_name(&text, map);
    }
    std::panic::catch_unwind(|| {
        refs::rename_rewrite_with_queries(&text, org, &map.0, &map.1, format, None) != text
    })
    .unwrap_or_else(|_| text_mentions_a_name(&text, map))
}

/// Compared under page identity's own folding (case and Unicode normal form),
/// so a link in any spelling that names the page is seen.
fn text_mentions_a_name(text: &str, map: &RenameMap) -> bool {
    let text = refs::page_key(text);
    map.0
        .iter()
        .any(|(old, _)| text.contains(&refs::page_key(old)))
}

/// The same search for a file too big to hold in memory: chunks overlap by more
/// than the longest name in any spelling (decomposing grows UTF-8 at most
/// threefold), so a name across two chunks is still seen.
fn file_mentions_a_name(path: &std::path::Path, map: &RenameMap) -> io::Result<bool> {
    const CHUNK: usize = 1024 * 1024;
    let overlap = map.0.iter().map(|(old, _)| old.len()).max().unwrap_or(0) * 4;
    let mut input = std::fs::File::open(path)?;
    let mut window = vec![0u8; CHUNK + overlap];
    let mut kept = 0;
    loop {
        let read = io::Read::read(&mut input, &mut window[kept..])?;
        if read == 0 {
            return Ok(false);
        }
        let filled = kept + read;
        let text = String::from_utf8_lossy(&window[..filled]);
        if text_mentions_a_name(&text, map) {
            return Ok(true);
        }
        kept = overlap.min(filled);
        window.copy_within(filled - kept..filled, 0);
    }
}

/// Who holds a name a rename would take (the one answer the plan, its
/// dialog question [`super::rename_offer`] and the CM-OWN-CHILD "used" test
/// share): page or journal files claiming it, else the pages declaring it
/// as an alias. A journal claims its day under every date spelling the
/// graph's one date answer accepts (`JournalFormat::parse`, CM step 2 batch
/// 1; M-Q7: a rename onto a day is a merge into its journal). O(1) indexed
/// lookups (two `resolve`s, a third for a date).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Holder {
    Free,
    Files(Vec<PageId>),
    Aliases(Vec<PageId>),
}

pub(super) fn holder(
    graph: &tine_store::WholeGraph,
    journal_format: &tine_core::date::JournalFormat,
    name: &str,
) -> Holder {
    let mut files = existing(graph.resolve(name, false));
    if journal_format.parse(name).is_some() {
        for id in existing(graph.resolve(name, true)) {
            if !files.contains(&id) {
                files.push(id);
            }
        }
    }
    if !files.is_empty() {
        return Holder::Files(files);
    }
    match graph.resolve(name, false) {
        Resolved::Alias { owners } if !owners.is_empty() => Holder::Aliases(owners),
        _ => Holder::Free,
    }
}

/// The pages other than `source` that hold `new` for a rename of `source`:
/// [`holder`]'s answer, which the plan and its dialog question use, under
/// the store's current journal format. Empty when the name is free or held
/// only by `source`. A journal holds every accepted spelling of its day
/// (M-Q7), which `resolve(new, false)` alone does not see. A caller without
/// a confirmation dialog (the CLI) refuses a nonempty answer before it
/// enters the operation. O(1) indexed lookups, as `holder`.
pub fn rename_destination_holders(
    store: &Store,
    graph: &tine_store::WholeGraph,
    new: &str,
    source: Option<&PageId>,
) -> Vec<PageId> {
    let journal_format = store.config().config.journal_format();
    match holder(graph, &journal_format, new.trim()) {
        Holder::Free => Vec::new(),
        Holder::Files(ids) | Holder::Aliases(ids) => {
            ids.into_iter().filter(|id| Some(id) != source).collect()
        }
    }
}

/// CM-OWN-CHILD (D-20261103-CMA) and M-Q6 (D-20261102-MC): `new` lies
/// strictly inside `old`'s namespace and is used, i.e. some file claims it or
/// some page (`old` itself included) declares it as an alias. A name that is
/// only linked is not used (D-20261105-CM11, CM-R9-USED). Only a
/// namespace name lies inside a namespace (`refs::is_namespace_name`, OG
/// `namespace-page?`; model round 10, S15): `./x` is no child of `.`.
pub(super) fn own_child_used(
    graph: &tine_store::WholeGraph,
    journal_format: &tine_core::date::JournalFormat,
    old_key: &str,
    new: &str,
) -> bool {
    refs::is_namespace_name(new)
        && refs::normalize(new).starts_with(&format!("{old_key}/"))
        && holder(graph, journal_format, new) != Holder::Free
}

/// The page name a page file answers to in `graph` (its inventory entry).
pub(super) fn name_of(graph: &tine_store::WholeGraph, id: &PageId) -> Option<String> {
    graph
        .inventory(tine_store::InventoryScope::File(id))
        .0
        .first()
        .map(|entry| entry.name.clone())
}

/// Why a rename was not planned, when the user can act on it: the dialog
/// question ([`super::rename_offer`]) reads it, and its message is what the
/// rename's caller shows. Carried inside the `io::Error` the plan returns.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RenameBlocked {
    /// The new name is one other page's or journal day's (`into`, its file):
    /// a merge into it is possible.
    Taken { into: String },
    /// The new name is only an alias of `owner` (M-Q5, D-20261102-MC5).
    /// `merge`: whether a merge into the owner is allowed (not when the
    /// owner is one of the renamed page's own namespace children, M-Q6).
    AliasOf {
        owner: String,
        merge: bool,
        own_page: bool,
    },
    /// CM-OWN-CHILD / M-Q6: `new` is a used name inside `old`'s namespace.
    OwnChild { old: String, new: String },
    /// The destination file `file` exists and is the page `page`'s (R14).
    FileTaken { file: String, page: String },
    /// CM3-1: an owner occurrence loses a key a stationary journal owns.
    JournalReferenceLoss { journals: Vec<String> },
}

impl std::fmt::Display for RenameBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameBlocked::Taken { .. } | RenameBlocked::AliasOf { .. } => {
                f.write_str("target page identity already exists elsewhere in the graph")
            }
            RenameBlocked::OwnChild { old, new } => write!(
                f,
                "cannot rename “{old}” to “{new}”: “{new}” is already a page or an alias inside “{old}”, and every page under “{old}” moves with it"
            ),
            RenameBlocked::FileTaken { file, page } => write!(
                f,
                "cannot rename: the file {file} belongs to the page “{page}”; rename or merge that page first"
            ),
            RenameBlocked::JournalReferenceLoss { journals } => write!(
                f,
                "cannot rename: some links reach the journal {} through this name; journals keep their names, so renaming would break those links",
                journals.join(", ")
            ),
        }
    }
}

impl std::error::Error for RenameBlocked {}

impl RenameBlocked {
    fn error(self) -> io::Error {
        let kind = match self {
            RenameBlocked::OwnChild { .. } | RenameBlocked::JournalReferenceLoss { .. } => {
                io::ErrorKind::InvalidInput
            }
            _ => io::ErrorKind::AlreadyExists,
        };
        io::Error::new(kind, self)
    }

    /// The reason inside a plan's error, if it is one.
    pub fn of(error: &io::Error) -> Option<&RenameBlocked> {
        error.get_ref().and_then(|cause| cause.downcast_ref())
    }
}

/// How the user answered the rename question ([`super::rename_offer`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameChoice<'a> {
    /// A plain rename: a new name another page holds refuses.
    Rename,
    /// Merge into this confirmed page (its file path), which holds the name.
    MergeInto(&'a str),
    /// M-Q5's second answer: the new name is only an alias of another page;
    /// the renamed page becomes the real page of that name, and the owner
    /// keeps its `alias::` (D-20261102-MC5).
    OwnPage,
}

/// One page identity a rename or merge moved ([`RenameReport::renamed`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RenamedPage {
    /// The page's name before the rename, in its own spelling.
    pub from: String,
    /// The name the rename gave it (written to its file name or title, and to
    /// the references that followed it).
    pub to: String,
    /// Graph-relative path of the page's file before the rename; `None` for a
    /// renamed page with no file and for a reference-only namespace member.
    pub path: Option<String>,
}

/// Extend file identities with names carried by applied edits, not a second
/// reference walk. Namespace parents follow the same lookup; the full target
/// uses the rewriter's answer (including a query's override).
pub(super) fn report_rewritten_names(
    renamed: &mut Vec<RenamedPage>,
    names: HashSet<(String, String)>,
    old: &str,
    lookup: &HashMap<String, String>,
) {
    let under = format!("{}/", refs::normalize(old));
    let mut listed: HashSet<_> = renamed
        .iter()
        .map(|page| refs::normalize(&page.from))
        .collect();
    for (from, to) in names {
        for prefix in refs::namespace_refs(&from) {
            let key = refs::normalize(prefix);
            if !key.starts_with(&under) || !listed.insert(key) {
                continue;
            }
            let destination = if prefix == from {
                to.clone()
            } else {
                let Some(destination) = refs::renamed_page_name(prefix, lookup) else {
                    continue;
                };
                destination.into_owned()
            };
            renamed.push(RenamedPage {
                from: prefix.to_owned(),
                to: destination,
                path: None,
            });
        }
    }
}

pub(super) fn plan_rename(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    choice: RenameChoice<'_>,
    #[cfg(test)] after_inventory: &dyn Fn(),
) -> io::Result<RenamePlan> {
    let merge_into = match choice {
        RenameChoice::MergeInto(into) => Some(into),
        _ => None,
    };
    // The published view: the caller walks once per attempt
    // (`refreshed_view`, in `multi_rename`'s reuse check).
    let graph = view(store)?;
    // The planning context: every format decision below uses this config,
    // and commit checks it is still the view's (B2).
    let config = store.config();
    let journal_format = config.config.journal_format();
    let old_key = refs::normalize(old);
    // Only file-claimed names move: the full inventory's alias and
    // reference-only name discovery is not needed here (GH #623).
    let owned = graph.inventory(tine_store::InventoryScope::FilesAtOrUnder(&old_key));
    #[cfg(test)]
    after_inventory();
    let source = existing(graph.resolve(old, false));
    validate_target(&source, expected_path)?;
    // One guard for every source and every answer, before target offers:
    // confirmation cannot bypass a stationary journal's reference protection.
    let journals = graph.journal_rewrite_risks(old, new);
    if !journals.is_empty() {
        return Err(RenameBlocked::JournalReferenceLoss { journals }.error());
    }
    // CM-OWN-CHILD, M-Q6: refused for a rename and a merge alike, before
    // anything else is decided (D-20261103-CMA).
    if own_child_used(&graph, &journal_format, &old_key, new) {
        return Err(RenameBlocked::OwnChild {
            old: old.to_owned(),
            new: new.to_owned(),
        }
        .error());
    }
    // Every file the rename moves: a merge into one of them merges a page
    // into its own namespace child (M-Q6).
    let moving: HashSet<PageId> = owned
        .0
        .iter()
        .filter(|entry| !entry.is_journal)
        .flat_map(|entry| physical(&entry.target))
        .collect();
    // Every identity the rename moves, with the file it moves from: the
    // rename map's pairs and the report's answer are this one list.
    let mut renamed = Vec::new();
    let renames =
        |entry: &tine_store::InventoryEntry, to: &str, from: Option<&PageId>| RenamedPage {
            from: entry.name.clone(),
            to: to.to_owned(),
            path: from.map(|id| id.as_str().to_owned()),
        };
    let mut moves = HashMap::<PageId, FileId>::new();
    let mut moved_titles_to_rebind = HashSet::<PageId>::new();
    let mut destinations = HashSet::new();
    let mut identities = HashSet::new();
    let mut primary_is_file = false;
    let mut merge = None;
    // A plain rename whose new name is held: decided after every other
    // entry, so a refusal the user must act on first is the one reported.
    let mut held = None;
    for entry in owned.0.iter().filter(|entry| !entry.is_journal) {
        let ids = physical(&entry.target);
        let primary = refs::normalize(&entry.name) == old_key;
        let new_name = if primary {
            new.to_owned()
        } else {
            format!("{new}{}", namespace_suffix(&entry.name, &old_key))
        };
        validate_target(&ids, None)?;
        let id = ids[0].clone();
        // An alias names its owner's page, so it is taken like a file name;
        // a journal day is taken by its journal (M-Q7).
        let (mut taken, alias_only) = match holder(&graph, &journal_format, &new_name) {
            Holder::Free => (Vec::new(), false),
            Holder::Files(files) => (files, false),
            Holder::Aliases(owners) => (owners, true),
        };
        if alias_only {
            validate_target(&existing(graph.resolve(&new_name, false)), None)?;
        }
        if !(primary && alias_only) {
            taken.retain(|claimant| *claimant != id);
        }
        // M-Q5: the alias stays its owners'; the renamed page becomes the
        // real page of that name (moved below).
        let own_target = primary && alias_only && choice == RenameChoice::OwnPage;
        if own_target {
            if taken.len() != 1 {
                return Err(error(
                    io::ErrorKind::AlreadyExists,
                    "the alias has no unique owner",
                ));
            }
            // Answer two fills the existing empty file through the guarded
            // merge path; without that file it is an ordinary rename.
            taken = existing(graph.resolve(&new_name, false));
            taken.retain(|claimant| *claimant != id);
        }
        if primary {
            if let [survivor] = taken.as_slice() {
                let merge_allowed = !moving.contains(survivor);
                match choice {
                    RenameChoice::MergeInto(_) | RenameChoice::OwnPage
                        if own_target
                            || matches!(choice, RenameChoice::MergeInto(into) if survivor.as_str() == into) =>
                    {
                        if !merge_allowed {
                            return Err(RenameBlocked::OwnChild {
                                old: old.to_owned(),
                                new: new.to_owned(),
                            }
                            .error());
                        }
                        identities.insert(refs::normalize(&new_name));
                        renamed.push(renames(entry, &new_name, Some(&id)));
                        merge = Some((id, survivor.clone()));
                        primary_is_file = true;
                        continue;
                    }
                    RenameChoice::Rename => {
                        held = Some(if alias_only {
                            RenameBlocked::AliasOf {
                                owner: survivor.as_str().to_owned(),
                                merge: merge_allowed,
                                own_page: true,
                            }
                        } else if merge_allowed {
                            RenameBlocked::Taken {
                                into: survivor.as_str().to_owned(),
                            }
                        } else {
                            RenameBlocked::OwnChild {
                                old: old.to_owned(),
                                new: new.to_owned(),
                            }
                        });
                        primary_is_file = true;
                        continue;
                    }
                    _ => {}
                }
            }
        }
        if !taken.is_empty() {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "target page identity already exists elsewhere in the graph",
            ));
        }
        let ext = Format::from_path(id.as_str().as_ref()).ext();
        let basename = format!(
            "{}.{}",
            tine_core::model::encode_page_name(&new_name, config.file_name_format),
            ext
        );
        // OG compute-new-file-path changes only the leaf. Each namespace
        // child keeps its own parent too; a merge keeps its survivor above.
        let rel = match id.as_str().rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{basename}"),
            None => basename,
        };
        let to = store.file_id(Area::Graph, &rel).map_err(store_error)?;
        if !destinations.insert(to.as_str().to_owned())
            || !identities.insert(refs::normalize(&new_name))
        {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "multiple pages map to the same rename target",
            ));
        }
        if to == id.file() {
            // A crash after the physical rename can leave Old's explicit
            // title in New's file. Finish that rewrite in place on retry.
            renamed.push(renames(entry, &new_name, Some(&id)));
            moved_titles_to_rebind.insert(id);
            primary_is_file |= primary;
            continue;
        }
        // R14: the destination file is listed and is another page's (a
        // `title::` names it otherwise, or a case-sensitive volume holds a
        // second spelling). Moving onto it would replace that page, which
        // the no-replace move refuses at commit; retrying cannot help, so
        // refuse now with the reason. Scenario (I-8): an external editor or
        // a sync delivery left a file at the destination name.
        if let Some(page) = store
            .as_page(&to)
            .filter(|page| !ids.contains(page))
            .and_then(|page| name_of(&graph, &page))
        {
            return Err(RenameBlocked::FileTaken {
                file: to.as_str().to_owned(),
                page,
            }
            .error());
        }
        if primary {
            primary_is_file = true;
        }
        renamed.push(renames(entry, &new_name, Some(&id)));
        moves.insert(id, to);
    }
    if let Some(held) = held {
        return Err(held.error());
    }
    // v0.6.5 model.rs 3654: reference-only pages have no move, but refs
    // change. Onto an existing page that repoints them, which is OG
    // `merge-pages!` of a page with no blocks, so it needs the confirmation.
    let mut ref_merge = false;
    let mut ref_survivor = None;
    if !primary_is_file && old != new {
        // CM12 C6: alias-only sources have no file either. OwnPage would
        // pretend to create one while merely substituting a name.
        if choice == RenameChoice::OwnPage {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "a file-less source cannot become its own page",
            ));
        }
        let destination = if refs::normalize(new) == old_key {
            // CM12 C13: respelling an alias's own key is a rename, even
            // when several pages declare it. No source owner is selected.
            Some(Holder::Free)
        } else {
            None
        };
        let (taken, alias_only) =
            match destination.unwrap_or_else(|| holder(&graph, &journal_format, new)) {
                Holder::Free => (Vec::new(), false),
                Holder::Files(files) => (files, false),
                Holder::Aliases(owners) => (owners, true),
            };
        if alias_only {
            validate_target(&existing(graph.resolve(new, false)), None)?;
        }
        match (choice, taken.as_slice()) {
            (_, []) => {}
            (RenameChoice::MergeInto(into), [owner]) if owner.as_str() == into => {
                ref_merge = true;
                ref_survivor = Some(owner.clone());
            }
            (RenameChoice::MergeInto(_), _) => {}
            (_, [owner]) => {
                return Err(if alias_only {
                    RenameBlocked::AliasOf {
                        owner: owner.as_str().to_owned(),
                        merge: true,
                        own_page: false,
                    }
                } else {
                    RenameBlocked::Taken {
                        into: owner.as_str().to_owned(),
                    }
                }
                .error())
            }
            (_, _) => {
                return Err(error(
                    io::ErrorKind::AlreadyExists,
                    "target page identity already exists elsewhere in the graph",
                ))
            }
        }
        renamed.push(RenamedPage {
            from: old.to_owned(),
            to: new.to_owned(),
            path: None,
        });
    }
    if merge_into.is_some() && merge.is_none() && !ref_merge {
        return Err(error(
            io::ErrorKind::NotFound,
            "the page to merge into no longer owns that name",
        ));
    }
    // Ruling (a), D-20261103-CMA (`Rulings9.queryMap`): a stored query
    // argument naming the merged page names the survivor by its own name;
    // links and namespace children keep the typed name (ruling (b)).
    let queries: Vec<(String, String)> = merge
        .as_ref()
        .map(|(_, dst)| dst)
        .or(ref_survivor.as_ref())
        .and_then(|survivor| name_of(&graph, survivor))
        .map(|name| vec![(old.to_owned(), name)])
        .unwrap_or_default();
    let pairs = renamed
        .iter()
        .map(|page| (page.from.clone(), page.to.clone()))
        .collect();
    let map = RenameMap(pairs, queries);
    let keyed = |pairs: &[(String, String)]| -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(from, to)| (refs::normalize(from), to.clone()))
            .collect()
    };
    let lookup = keyed(&map.0);
    let query_lookup = keyed(&map.1);
    // v0.6.5 model.rs 3681-3690 (warm index) and OG `:block/refs`: the
    // pages that explicitly reference a renamed name, plus every moved page.
    // The pages with a rewrite target no reference selects are read too: a
    // query's page-name arguments (D-20261031-CMQ), links in other macros'
    // arguments and Org file links follow the rename like links.
    let olds: Vec<String> = map.0.iter().map(|(from, _)| from.clone()).collect();
    let mut candidates: Vec<PageId> = graph.explicit_referrers(&olds);
    candidates.extend(rewrite_target_pages(
        &graph,
        &lookup,
        config.file_name_format,
    ));
    candidates.extend(moves.keys().cloned());
    candidates.extend(moved_titles_to_rebind.iter().cloned());
    candidates.retain(|id| {
        merge
            .as_ref()
            .is_none_or(|(src, dst)| id != src && id != dst)
    });
    candidates.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    candidates.dedup();
    // A page Tine could not index (malformed imported or sync-delivered
    // content) has no referrers row, so nothing above sees its references.
    // Skipping it would leave `[[Old]]` dangling with no word, and it is not
    // ours to rewrite: refuse, naming the file, before anything is written.
    for (file, _) in graph.unreadable_files() {
        let planned = candidates.iter().any(|id| id.file() == *file);
        if !planned && may_hold_reference(store, file, &map, config.file_name_format) {
            return Err(error(
                io::ErrorKind::InvalidData,
                &format!(
                    "cannot rename: {} could not be indexed and may link to this page; repair or remove it first",
                    file.as_str()
                ),
            ));
        }
    }
    Ok(RenamePlan {
        graph,
        config,
        map,
        lookup,
        query_lookup,
        moves,
        renamed,
        moved_titles_to_rebind,
        merge,
        ref_merge,
        candidates,
        reads: HashMap::new(),
        skipped: Vec::new(),
        rewritten: HashMap::new(),
    })
}
