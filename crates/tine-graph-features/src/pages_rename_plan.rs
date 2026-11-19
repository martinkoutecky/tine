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
    }
}

/// What one rename attempt writes, planned from one published view (the
/// attempt's walk, `refreshed_view`, decides reuse) and the config captured with it;
/// commit and the host admit it only while both are current (A-W1, B2).
pub(super) struct RenamePlan {
    pub(super) graph: tine_store::WholeGraph,
    pub(super) config: tine_store::ConfigState,
    /// The planning config's journal format: no rename respells a day (O2).
    pub(super) days: tine_core::date::JournalFormat,
    pub(super) map: RenameMap,
    pub(super) lookup: HashMap<String, String>,
    pub(super) moves: HashMap<PageId, FileId>,
    /// Planned identities (including a fileless primary), in `map.0` order.
    /// The transaction adds reference-only identities from its applied edits.
    pub(super) renamed: Vec<RenamedPage>,
    pub(super) moved_titles_to_rebind: HashSet<PageId>,
    pub(super) merge: Option<(PageId, PageId)>,
    /// The survivor's bytes and revision its spelling was chosen from (O1,
    /// a day's header title), which the merge realizes from: the merge's
    /// revision guard then covers the choice, and the survivor is read once.
    pub(super) survivor_read: Option<(String, FileRev)>,
    pub(super) ref_merge: bool,
    pub(super) candidates: Vec<PageId>,
    /// The candidates `pages` read to find the plan's write set, reused by
    /// the commit so it writes exactly what was planned (A-R3).
    pub(super) reads: HashMap<PageId, (String, FileRev)>,
    /// Referrers `pages` leaves byte-identical for their VCS conflict markers.
    pub(super) skipped: Vec<PageId>,
    /// The new text of each referrer `pages` found changed, computed from
    /// its `reads` bytes; the host reuses it on equal bytes (GH #623). An
    /// unchanged referrer's proposal is kept when it holds a site O7 reports.
    pub(super) rewritten: HashMap<PageId, refs::RenameProposal>,
    /// The request names the source's own current spelling: nothing to do,
    /// on any view (the rename decides it without a walk).
    pub(super) unchanged: bool,
}

impl RenamePlan {
    /// How this plan's rewrites spell names (file names, fixed days).
    pub(super) fn spelling(&self) -> refs::RenameFormat<'_> {
        refs::RenameFormat::new(self.config.file_name_format, &self.days)
    }

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
                refs::rename_rewrite_recording(
                    text,
                    format == Format::Org,
                    &self.map.0,
                    self.spelling(),
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
            let format = refs::RenameFormat::new(self.config.file_name_format, &self.days);
            let rewritten = refs::rename_proposal(&content, org, &self.lookup, format)
                .map_err(|e| MergeRefused::trace(e, id.as_str()))?;
            let changed = rewritten.text != content;
            let marked = self.skips(id, &content);
            match (changed, marked) {
                (true, false) => {
                    pages.push(id.clone());
                    self.rewritten.insert(id.clone(), rewritten);
                }
                (true, true) => self.skipped.push(id.clone()),
                // Not written, but its sites are reported (O7): the
                // transaction reads them from the kept proposal.
                (false, false) if !rewritten.unrealized.is_empty() => {
                    self.rewritten.insert(id.clone(), rewritten);
                }
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

    /// Whether the request leaves candidate `id`'s file (text `content`)
    /// unwritten although its references change: a page carrying VCS conflict
    /// markers, unless the plan moves or retitles it. The one answer for
    /// `pages` (the executor) and the helpers' preview, so a declaration in
    /// such a file is not respelled in either.
    pub(super) fn skips(&self, id: &PageId, content: &str) -> bool {
        !(self.moves.contains_key(id) || self.moved_titles_to_rebind.contains(id))
            && !tine_core::concord_queue::vcs_conflict_markers(
                content,
                Format::from_path(id.as_str().as_ref()),
            )
            .is_empty()
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
    format: refs::RenameFormat<'_>,
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
        refs::rename_rewrite_recording(&text, org, &map.0, format, None) != text
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

/// O4's touched names of one request (model r6; Lean `touchedNames`): the
/// source, every name under it however it exists (a file, a namespace parent,
/// a reference, an alias any page declares), the aliases the source's file
/// declares, and the images of all of these under O1, once per page key. A
/// day already written is not touched (O2, O4: it stays); a day image is.
///
/// Cost: the names under the source come from the inventory the plan already
/// holds (`FilesAtOrUnder`) and the ordered reference and alias indexes
/// (`FilelessUnder`, O(log N + k)); each touched name then costs O(depth) for
/// its namespace parents and image and O(1) indexed `resolve`s in the checks.
/// Nothing visits a name outside the source's namespace.
pub(super) struct Touched<'a> {
    graph: &'a tine_store::WholeGraph,
    journal_format: &'a tine_core::date::JournalFormat,
    old_key: &'a str,
    /// The touched names, then their images, each with its image.
    names: Vec<(String, String)>,
}

impl<'a> Touched<'a> {
    pub(super) fn new(
        graph: &'a tine_store::WholeGraph,
        journal_format: &'a tine_core::date::JournalFormat,
        old: &str,
        old_key: &'a str,
        destination: &str,
        owned: &tine_store::Inventory,
        source: Option<&PageId>,
    ) -> Self {
        let under = format!("{old_key}/");
        let mut keys = HashSet::new();
        let mut touched = Vec::new();
        // A day already written stays (O2, O4) and is not touched; an image
        // is touched even when it is a day (Lean `touchedNames` appends
        // `touched.map nameMap` unfiltered), so a day the rename newly
        // produces is checked for contest like any other image.
        let mut add = |name: &str, touched: &mut Vec<String>| {
            if keys.insert(refs::page_key(name)) {
                touched.push(name.to_owned());
            }
        };
        let mut touch = |name: &str, touched: &mut Vec<String>| {
            if journal_format.parse(name).is_none() {
                add(name, touched);
            }
        };
        touch(old, &mut touched);
        let fileless = graph.inventory(tine_store::InventoryScope::FilelessUnder(old_key));
        for entry in owned.0.iter().chain(&fileless.0) {
            for name in refs::namespace_refs(&entry.name) {
                let key = refs::page_key(name);
                if key == old_key || key.starts_with(&under) {
                    touch(name, &mut touched);
                }
            }
        }
        if let Some(source) = source {
            let aliases = graph.inventory(tine_store::InventoryScope::AliasesOf(source));
            for entry in &aliases.0 {
                touch(&entry.name, &mut touched);
            }
        }
        // O1's one map: `old` and everything under it, days pinned.
        let lookup = HashMap::from([(old_key.to_owned(), destination.to_owned())]);
        let image = |name: &str| {
            refs::renamed_site_name(name, &lookup, Some(journal_format))
                .map_or_else(|| name.to_owned(), std::borrow::Cow::into_owned)
        };
        for at in 0..touched.len() {
            let mapped = image(&touched[at]);
            add(&mapped, &mut touched);
        }
        let names = touched
            .into_iter()
            .map(|name| {
                let mapped = image(&name);
                (name, mapped)
            })
            .collect();
        Self {
            graph,
            journal_format,
            old_key,
            names,
        }
    }

    /// O4 "no touched name is contested": several files claim it.
    pub(super) fn contested(&self) -> Option<RenameBlocked> {
        self.names.iter().find_map(|(name, _)| {
            match holder(self.graph, self.journal_format, name) {
                Holder::Files(files) if files.len() > 1 => {
                    let mut files: Vec<String> =
                        files.iter().map(|id| id.as_str().to_owned()).collect();
                    files.sort_unstable();
                    Some(RenameBlocked::Contested {
                        name: name.clone(),
                        files,
                    })
                }
                _ => None,
            }
        })
    }

    /// O4 capture (Lean `captureClaimed`): a touched name whose image is
    /// another page's name and claimed (a file claims it or a page declares
    /// it) by something other than the merge `survivor`. Claims by the
    /// source's own file count only for names other than the source itself
    /// ("only the source concept may land on its own alias"), and the
    /// source's own image is O3's (holders, alias held, merge), decided by the
    /// plan, so every claimant of any other image counts.
    pub(super) fn capture(&self, survivor: Option<&PageId>) -> Option<RenameBlocked> {
        self.names.iter().find_map(|(name, image)| {
            let key = refs::page_key(name);
            if key == self.old_key || key == refs::page_key(image) {
                return None;
            }
            let claimants = match holder(self.graph, self.journal_format, image) {
                Holder::Free => return None,
                Holder::Files(files) if survivor.is_some_and(|s| files.contains(s)) => return None,
                Holder::Files(files) => files,
                Holder::Aliases(owners) => owners,
            };
            let page = claimants.first()?;
            Some(RenameBlocked::Capture {
                name: name.clone(),
                image: image.clone(),
                page: name_of(self.graph, page).unwrap_or_else(|| page.as_str().to_owned()),
            })
        })
    }
}

/// O3's refusal of `alias`, an alias `owners` declare and no file claims,
/// naming the page it opens: the least-path declarer, `owners`' first (the
/// store's one answer, `Resolved::Alias`).
fn alias_held(graph: &tine_store::WholeGraph, alias: &str, owners: &[PageId]) -> RenameBlocked {
    let path = owners[0].as_str().to_owned();
    RenameBlocked::AliasHeld {
        alias: alias.to_owned(),
        page: name_of(graph, &owners[0]).unwrap_or_else(|| path.clone()),
        path,
    }
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
    /// CM-OWN-CHILD / M-Q6: `new` is a used name inside `old`'s namespace.
    OwnChild { old: String, new: String },
    /// The destination file `file` exists and is the page `page`'s (R14).
    FileTaken { file: String, page: String },
    /// O0: `name` is no page (no file, no reference, no namespace child)
    /// and no alias, so there is nothing to rename.
    NotAPage { name: String },
    /// O0, O2: `name` is a day; a day is not a rename source.
    Day { name: String },
    /// O3: the new name `alias` is an alias another page declares and no
    /// file claims; it opens `page` (its file `path`).
    AliasHeld {
        alias: String,
        page: String,
        path: String,
    },
    /// O4: the file `file` would move, carries VCS conflict markers, and
    /// needs a rewrite, so it can neither be rewritten nor move verbatim.
    ConflictMarkers { file: String },
    /// O4 capture: the touched name `name` would become `image`, a name
    /// another page claims (a file's, or an alias it declares) that is not
    /// the merge survivor's; `page` is the page `image` opens.
    Capture {
        name: String,
        image: String,
        page: String,
    },
    /// O4 contested: several files claim the touched name `name` (`files`,
    /// graph-relative, in path order).
    Contested { name: String, files: Vec<String> },
}

impl std::fmt::Display for RenameBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameBlocked::Taken { into } => write!(f, "cannot rename: {into} now owns that name; start the rename again to review the merge choice"),
            RenameBlocked::OwnChild { old, new } => write!(
                f,
                "cannot rename “{old}” to “{new}”: “{new}” is already a page or an alias inside “{old}”, and every page under “{old}” moves with it"
            ),
            RenameBlocked::FileTaken { file, page } => write!(
                f,
                "cannot rename: the file {file} belongs to the page “{page}”; rename or merge that page first"
            ),
            RenameBlocked::NotAPage { name } => write!(
                f,
                "cannot rename “{name}”: no page or alias has that name"
            ),
            RenameBlocked::Day { name } => write!(
                f,
                "cannot rename “{name}”: it is a journal day, and days keep their names"
            ),
            RenameBlocked::AliasHeld { alias, page, .. } => write!(
                f,
                "cannot rename: “{alias}” is an alias of the page “{page}”"
            ),
            RenameBlocked::ConflictMarkers { file } => write!(
                f,
                "cannot rename: {file} would move and needs a rewrite, but it has unresolved conflict markers; resolve them first"
            ),
            RenameBlocked::Capture { name, image, page } => write!(
                f,
                "cannot rename: “{name}” would become “{image}”, which is already the page “{page}”; rename or merge that page first"
            ),
            RenameBlocked::Contested { name, files } => write!(
                f,
                "cannot rename: several files claim “{name}” ({}); merge one file into the other or remove one, then try again",
                files.join(", ")
            ),
        }
    }
}

impl std::error::Error for RenameBlocked {}

impl RenameBlocked {
    pub(super) fn error(self) -> io::Error {
        let kind = match self {
            RenameBlocked::Taken { .. }
            | RenameBlocked::FileTaken { .. }
            | RenameBlocked::AliasHeld { .. }
            | RenameBlocked::Capture { .. }
            | RenameBlocked::Contested { .. } => io::ErrorKind::AlreadyExists,
            RenameBlocked::OwnChild { .. }
            | RenameBlocked::NotAPage { .. }
            | RenameBlocked::Day { .. } => io::ErrorKind::InvalidInput,
            RenameBlocked::ConflictMarkers { .. } => io::ErrorKind::InvalidData,
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
    /// O3: the only answer besides cancel; refused where nothing was asked.
    MergeInto(&'a str),
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
/// uses the rewriter's answer.
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

/// O3: "An answer where nothing was asked is refused". Scenario: the answer
/// is stale (another instance, an external editor or a sync delivery changed
/// the names between the question and the answer), or a caller answered a
/// question the planner never asked.
fn unasked(file: &str) -> io::Error {
    RewriteBlocked::StaleMerge { file: file.into() }.error()
}

pub(super) fn plan_rename(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    choice: RenameChoice<'_>,
    #[cfg(test)] after_inventory: &dyn Fn(),
) -> io::Result<RenamePlan> {
    plan_request(
        store,
        old,
        new,
        expected_path,
        choice,
        false,
        #[cfg(test)]
        after_inventory,
    )
}

/// The alias dialog's read of its second step (D-20261115-HELP, r6
/// "Outside the model"): the plain rename of `old` to `new` as it will be
/// planned once every page other than the source has dropped `new` from its
/// alias lines (the dialog's first step, ordinary edits). Only O3's
/// alias-held refusal is not raised; every other rule decides as for the
/// rename itself, on this view. Read-only: the plan is never committed (the
/// rename after the edits plans again, on the edited files). Cost: one plan.
pub(super) fn plan_released_rename(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
) -> io::Result<RenamePlan> {
    plan_request(
        store,
        old,
        new,
        expected_path,
        RenameChoice::Rename,
        true,
        #[cfg(test)]
        &|| {},
    )
}

fn plan_request(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    choice: RenameChoice<'_>,
    released: bool,
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
    // O0, O2: a day is not a rename source, by its identity (every spelling
    // the graph's one date answer accepts), wherever its file is.
    if journal_format.parse(old).is_some() {
        return Err(RenameBlocked::Day {
            name: old.to_owned(),
        }
        .error());
    }
    // SPEC §5.3: the selected physical member defines the entire map.
    // Keep `new` for checking that this member still holds the requested
    // spelling. Descendants, referrers and reports use the selection too.
    // O1 (model `survivorSpelling`): the survivor's own title if it names
    // its claim, else its effective name. A page's effective name is its
    // title already; a day's is its title format's rendering
    // (D-20261119-YL), so a day's header is read: `title:: 2027-10-26` on
    // that day's file is its spelling, a header naming another day is not.
    // One read of the survivor: the merge realizes from these bytes, under
    // their revision (`survivor_read`).
    let mut survivor_read = None;
    let selected = merge_into
        .and_then(|into| {
            let id = PageId::from(into);
            let inventory = graph.inventory(tine_store::InventoryScope::File(&id));
            let entry = inventory.0.first()?;
            if let Some(day) = entry.day.filter(|_| entry.is_journal) {
                let day = tine_core::date::JournalDate::from_ordinal(day.0);
                survivor_read = crate::parsed_text::read(store, &id.file()).ok();
                let title = survivor_read.as_ref().and_then(|(text, _)| {
                    tine_core::model::page_title_from_header(text, Format::from_path(into.as_ref()))
                });
                if let Some(title) = title.filter(|title| journal_format.parse(title) == Some(day))
                {
                    return Some(title);
                }
            }
            Some(entry.name.clone())
        })
        .unwrap_or_else(|| new.to_owned());
    let old_key = refs::normalize(old);
    // Only file-claimed names move: the full inventory's alias and
    // reference-only name discovery is not needed here (GH #623).
    let owned = graph.inventory(tine_store::InventoryScope::FilesAtOrUnder(&old_key));
    #[cfg(test)]
    after_inventory();
    let resolved = graph.resolve(old, false);
    // O0: a page has a file, a reference or a namespace child (files under
    // it are in `owned`, referenced children in the referrers index under
    // every namespace prefix); an alias is declared. Anything else is no
    // name to rename. The referrers lookup runs only when the rest fail.
    if owned.0.is_empty()
        && matches!(resolved, Resolved::Absent { .. })
        && graph.explicit_referrers(&[old.to_owned()]).is_empty()
    {
        return Err(RenameBlocked::NotAPage {
            name: old.to_owned(),
        }
        .error());
    }
    let source = existing(resolved);
    validate_target(&source, expected_path)?;
    // The files the rename moves: no journal, and no day under the source,
    // which stays (O4) wherever its file is.
    let moves_with = |entry: &&tine_store::InventoryEntry| {
        !entry.is_journal && journal_format.parse(&entry.name).is_none()
    };
    let destination = holder(&graph, &journal_format, new);
    // O3 first (Lean `selectCore` before `authorize`): a name another page
    // declares as an alias is refused for every answer, naming the page it
    // opens. An alias only the source's own file declares is the source's
    // own name (one-step rename; its declaration stays, P1); a source with
    // no file declares nothing, except that respelling an alias's own key is
    // a rename (CM12 C13). Decided here only.
    if let Holder::Aliases(owners) = &destination {
        let own_file = owned
            .0
            .iter()
            .filter(moves_with)
            .find(|entry| refs::normalize(&entry.name) == old_key)
            .and_then(|entry| physical(&entry.target).into_iter().next());
        let held = match own_file {
            Some(id) => owners.iter().any(|owner| *owner != id),
            None => refs::normalize(new) != old_key,
        };
        if held && !released {
            return Err(alias_held(&graph, new, owners).error());
        }
    }
    // O4 over every touched name, decided once, here, for every outcome
    // below (an unchanged request too: Lean `unchanged_child_contested_refused`),
    // in the model's order: contested, own child, capture. A refusal comes
    // before any question (Lean `authorize`).
    let touched = Touched::new(
        &graph,
        &journal_format,
        old,
        &old_key,
        &selected,
        &owned,
        source.first(),
    );
    if let Some(contested) = touched.contested() {
        return Err(contested.error());
    }
    // CM-OWN-CHILD, M-Q6: refused for a rename and a merge alike
    // (D-20261103-CMA).
    if own_child_used(&graph, &journal_format, &old_key, new) {
        return Err(RenameBlocked::OwnChild {
            old: old.to_owned(),
            new: new.to_owned(),
        }
        .error());
    }
    // The merge survivor O4 excepts: the one file other than the source's
    // that holds `new` (O3's one holder), whether or not the merge is
    // answered yet, so the question is asked only of an available merge.
    let survivor = match &destination {
        Holder::Files(files) => {
            let others: Vec<PageId> = files
                .iter()
                .filter(|id| !source.contains(id))
                .cloned()
                .collect();
            match <[PageId; 1]>::try_from(others) {
                Ok([one]) => Some(one),
                Err(_) => None,
            }
        }
        Holder::Free | Holder::Aliases(_) => None,
    };
    if let Some(capture) = touched.capture(survivor.as_ref()) {
        return Err(capture.error());
    }
    // A request is "this page, that spelling": unchanged when `new` is the
    // source's own current spelling (its file's name; for a source with no
    // file, the typed name), whatever case the source was typed in. Nothing
    // to rewrite and nothing asked, so a merge answer is one nobody asked
    // for (O3).
    let own = owned
        .0
        .iter()
        .find(|entry| refs::normalize(&entry.name) == old_key)
        .map_or(old, |entry| entry.name.as_str());
    if own == new {
        if merge_into.is_some() {
            return Err(unasked(merge_into.unwrap()));
        }
        return Ok(RenamePlan {
            graph,
            config,
            days: journal_format,
            map: RenameMap(Vec::new()),
            lookup: HashMap::new(),
            moves: HashMap::new(),
            renamed: Vec::new(),
            moved_titles_to_rebind: HashSet::new(),
            merge: None,
            survivor_read: None,
            ref_merge: false,
            candidates: Vec::new(),
            reads: HashMap::new(),
            skipped: Vec::new(),
            rewritten: HashMap::new(),
            unchanged: true,
        });
    }
    // A merge into a file the rename moves merges a page into its own
    // namespace child (M-Q6).
    let moving: HashSet<PageId> = owned
        .0
        .iter()
        .filter(moves_with)
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
    for entry in owned.0.iter().filter(moves_with) {
        let ids = physical(&entry.target);
        let primary = refs::normalize(&entry.name) == old_key;
        let new_name = if primary {
            selected.clone()
        } else {
            format!("{selected}{}", namespace_suffix(&entry.name, &old_key))
        };
        validate_target(&ids, None)?;
        let id = ids[0].clone();
        // O3 decides the source's own destination here: a journal day is
        // taken by its journal (M-Q7); an alias holder was decided above
        // (here only the source's own alias is left). Every other touched
        // name is O4's, decided once above (`Touched::capture`).
        let mut taken = match primary.then_some(&destination) {
            None | Some(Holder::Free | Holder::Aliases(_)) => Vec::new(),
            Some(Holder::Files(files)) => files.clone(),
        };
        taken.retain(|claimant| *claimant != id);
        if primary {
            if let [survivor] = taken.as_slice() {
                let merge_allowed = !moving.contains(survivor);
                match choice {
                    RenameChoice::MergeInto(into) if survivor.as_str() == into => {
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
                        held = Some(if merge_allowed {
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
            if taken.len() == 1 {
                return Err(unasked(merge_into.unwrap()));
            }
            return Err(RenameBlocked::Contested {
                name: new_name,
                files: taken.iter().map(|id| id.as_str().to_owned()).collect(),
            }
            .error());
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
            return Err(CheckRefused {
                guarantee: tine_store::Guarantee::P2,
                file: id.as_str().into(),
            }
            .error());
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
    if !primary_is_file && old != new {
        // CM12 C13: respelling an alias's own key is a rename, even when
        // several pages declare it. No source owner is selected.
        let taken = match &destination {
            Holder::Files(files) if refs::normalize(new) != old_key => files.clone(),
            // An alias holder was decided above (O3).
            Holder::Free | Holder::Files(_) | Holder::Aliases(_) => Vec::new(),
        };
        match (choice, taken.as_slice()) {
            (_, []) => {}
            (RenameChoice::MergeInto(into), [owner]) if owner.as_str() == into => {
                ref_merge = true;
            }
            (RenameChoice::MergeInto(_), _) => {}
            (_, [owner]) => {
                return Err(RenameBlocked::Taken {
                    into: owner.as_str().to_owned(),
                }
                .error())
            }
            (_, _) => {
                return Err(RenameBlocked::Contested {
                    name: new.into(),
                    files: taken.iter().map(|id| id.as_str().to_owned()).collect(),
                }
                .error())
            }
        }
        renamed.push(RenamedPage {
            from: old.to_owned(),
            to: selected,
            path: None,
        });
    }
    if merge_into.is_some() && merge.is_none() && !ref_merge {
        return Err(unasked(merge_into.unwrap()));
    }
    let pairs = renamed
        .iter()
        .map(|page| (page.from.clone(), page.to.clone()))
        .collect();
    let map = RenameMap(pairs);
    let keyed = |pairs: &[(String, String)]| -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(from, to)| (refs::normalize(from), to.clone()))
            .collect()
    };
    let lookup = keyed(&map.0);
    // v0.6.5 model.rs 3681-3690 (warm index) and OG `:block/refs`: the
    // pages that explicitly reference a renamed name, plus every moved page.
    // The pages with a rewrite target no reference selects are read too: a
    // query's page-name arguments (D-20261031-CMQ), links in other macros'
    // arguments and Org file links follow the rename like links.
    let olds: Vec<String> = map.0.iter().map(|(from, _)| from.clone()).collect();
    let spelling = refs::RenameFormat::new(config.file_name_format, &journal_format);
    let mut candidates: Vec<PageId> = graph.explicit_referrers(&olds);
    candidates.extend(rewrite_target_pages(&graph, &lookup, spelling));
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
        if !planned && may_hold_reference(store, file, &map, spelling) {
            return Err(RewriteBlocked::Unindexed {
                file: file.as_str().into(),
            }
            .error());
        }
    }
    Ok(RenamePlan {
        graph,
        config,
        days: journal_format,
        map,
        lookup,
        moves,
        renamed,
        moved_titles_to_rebind,
        survivor_read: merge.as_ref().and(survivor_read),
        merge,
        ref_merge,
        candidates,
        reads: HashMap::new(),
        skipped: Vec::new(),
        rewritten: HashMap::new(),
        unchanged: false,
    })
}
