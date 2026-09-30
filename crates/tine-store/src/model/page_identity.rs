//! Graph text discovery and effective page names. The path remains the stable
//! file identity; the preamble title is the logical page name.

use super::*;
use std::io::{BufRead, BufReader, Read};
use tine_core::model::PreambleRead;

impl Graph {
    pub(crate) fn find_claimants(&self, name: &str, kind: PageKind) -> Vec<PageEntry> {
        let key = (kind, tine_core::refs::page_key(name));
        loop {
            let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            if let Some((g, index)) = self.find_entry_cache.read().unwrap().as_ref() {
                if *g == gen && (index.has_kind(kind) || index.entries.contains_key(&key)) {
                    return index.entries.get(&key).cloned().unwrap_or_default();
                }
            }

            let mut built = FindEntryIndex::new();
            built.entries = page_claimants(self, &list_graph_pages_kind(self, Some(kind)));
            built.mark_kind_loaded(kind);

            let found = {
                let mut guard = self.find_entry_cache.write().unwrap();
                match guard.as_mut() {
                    Some((g, index)) if *g == gen => {
                        if !index.has_kind(kind) {
                            index
                                .entries
                                .retain(|(loaded_kind, _), _| *loaded_kind != kind);
                            index.entries.extend(built.entries);
                            index.mark_kind_loaded(kind);
                        }
                        index.entries.get(&key).cloned().unwrap_or_default()
                    }
                    _ => {
                        let found = built.entries.get(&key).cloned().unwrap_or_default();
                        *guard = Some((gen, built));
                        found
                    }
                }
            };
            if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == gen {
                return found;
            }
        }
    }

    /// A direct path read can observe a new or retitled file before the watcher.
    /// Rebuild claimant ordering if that live file is missing from its bucket;
    /// a proposed destination that does not exist must not become a claimant.
    pub(super) fn observe_name_entry(&self, entry: &PageEntry) {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let mut cache = self.find_entry_cache.write().unwrap();
        if let Some((g, index)) = cache.as_ref() {
            let key = (entry.kind, tine_core::refs::page_key(&entry.name));
            let known = index.entries.get(&key).is_some_and(|entries| {
                entries.iter().any(|candidate| candidate.path == entry.path)
            });
            if *g == gen
                && !known
                && fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_file())
            {
                *cache = None;
            }
        }
    }

    /// Cold journal inventory and its complete claimant index share one walk.
    /// Only called at open, before the watcher and load worker start.
    pub(crate) fn scan_journal_names(&self) -> Vec<PageEntry> {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let entries = list_graph_pages_kind(self, Some(PageKind::Journal));
        let mut index = FindEntryIndex::new();
        index.entries = page_claimants(self, &entries);
        index.mark_kind_loaded(PageKind::Journal);
        *self.find_entry_cache.write().unwrap() = Some((gen, index));
        entries
    }

    /// Build the page list and effective-name claimants from one cold walk.
    pub(crate) fn snapshot_name_index(
        &self,
    ) -> (
        Arc<Vec<PageEntry>>,
        HashMap<(PageKind, String), Vec<PageEntry>>,
    ) {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let format = self.current_journal_format();
        let entries = list_graph_pages(self);
        let claimants = page_claimants(self, &entries);
        *self.find_entry_cache.write().unwrap() = Some((
            gen,
            FindEntryIndex {
                entries: claimants.clone(),
                // Reuse known claims, but a first miss must discover files
                // that arrived after this published snapshot.
                pages_loaded: false,
                journals_loaded: false,
            },
        ));
        let list = Arc::new(dedup_journal_days(
            entries,
            &format,
            self.current_config().file_name_format,
        ));
        *self.page_list_cache.write().unwrap() = Some((gen, Arc::clone(&list)));
        (list, claimants)
    }
}

/// The same claimant ordering for cold direct reads, journal open and snapshots.
fn page_claimants(
    graph: &Graph,
    entries: &[PageEntry],
) -> HashMap<(PageKind, String), Vec<PageEntry>> {
    let mut claimants: HashMap<(PageKind, String), Vec<PageEntry>> = HashMap::new();
    for entry in entries {
        claimants
            .entry((entry.kind, tine_core::refs::page_key(&entry.name)))
            .or_default()
            .push(entry.clone());
    }
    let format = graph.current_journal_format();
    let name_format = graph.current_config().file_name_format;
    for entries in claimants.values_mut() {
        entries.sort_by(|a, b| compare_page_claimants(a, b, &format, name_format));
    }
    claimants
}

#[cfg(test)]
mod cold_index_tests {
    use super::*;

    #[test]
    fn a_late_title_claimant_seen_by_path_replaces_the_cached_winner() {
        let dir = std::env::temp_dir().join(format!("tine-late-title-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/Other.md"), "title:: Claimed\n- old owner\n").unwrap();
        let graph = Graph::open(&dir);
        graph.snapshot_name_index();
        let arrived = dir.join("pages/Claimed.md");
        fs::write(&arrived, "title:: Claimed\n- new preferred owner\n").unwrap();
        // The direct page-read path observes the title before canonical lookup.
        let entry = graph.entry_for_path(&arrived).unwrap();
        assert_eq!(graph.find_entry(&entry.name, entry.kind).unwrap().path, arrived,
            "I-12: a late title claimant observed by a direct read must use the ordinary winner order");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn journal_first_read_opens_no_ordinary_preambles_and_late_titles_resolve() {
        let dir = std::env::temp_dir().join(format!("tine-journal-first-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(dir.join(".tine-test-pause-load"), "").unwrap();
        fs::write(dir.join("journals/2026_09_29.md"), "- journal\n").unwrap();
        fs::write(
            dir.join("pages/Other.md"),
            "title:: Claimed\n- title owner\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Claimed.md"),
            "title:: Elsewhere\n- moved identity\n",
        )
        .unwrap();
        fs::write(dir.join("pages/Org.org"), "#+title: Claimed\n* duplicate\n").unwrap();
        GRAPH_PREAMBLE_READS.with(|reads| reads.set(0));
        let (store, _, _) = crate::Store::open(&dir, Default::default()).unwrap();
        let open_reads = GRAPH_PREAMBLE_READS.with(|reads| reads.replace(0));
        let journal = store.journal_id(crate::Day(
            tine_core::date::JournalDate {
                year: 2026,
                month: 9,
                day: 29,
            }
            .ordinal_key(),
        ));
        assert_eq!(store.page(&journal).unwrap().doc.blocks[0].raw, "journal");
        let feed_reads = GRAPH_PREAMBLE_READS.with(|reads| reads.get());
        fs::remove_file(dir.join(".tine-test-pause-load")).unwrap();
        let graph = store.whole_graph().unwrap();
        let crate::Resolved::Existing { id, others } = graph.resolve("Claimed", false) else {
            panic!("late title claimant missing")
        };
        assert_eq!(others.len(), 1);
        assert_eq!(id.as_str(), "pages/Other.md");
        assert_eq!(store.page(&id).unwrap().doc.name, "Claimed");
        assert!(
            matches!(graph.resolve("Elsewhere", false), crate::Resolved::Existing { id, .. } if id.as_str() == "pages/Claimed.md")
        );
        drop(graph);
        store.close();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(open_reads, 0, "I-12/E: Store::open must not open ordinary page preambles before journal paint; exemplar page_identity.rs");
        assert_eq!(feed_reads, 0, "I-12/E: a journal-kind claimant lookup must not open ordinary page files; exemplar page_identity.rs");
    }

    #[test]
    fn cold_name_snapshot_reads_each_page_preamble_once() {
        let dir =
            std::env::temp_dir().join(format!("tine-cold-name-snapshot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        for i in 0..8 {
            fs::write(
                dir.join("pages").join(format!("Page {i}.md")),
                format!("title:: Name {i}\n\n- body\n"),
            )
            .unwrap();
        }
        let graph = Graph::open(&dir);
        GRAPH_LIST_CALLS.with(|calls| calls.set(0));
        GRAPH_PREAMBLE_READS.with(|reads| reads.set(0));
        let (list, claimants) = graph.snapshot_name_index();
        assert_eq!(list.len(), 8);
        assert_eq!(claimants.len(), 8);
        assert_eq!(
            GRAPH_LIST_CALLS.with(|calls| calls.get()),
            1,
            "one graph walk"
        );
        assert_eq!(
            GRAPH_PREAMBLE_READS.with(|reads| reads.get()),
            8,
            "one preamble read per page"
        );
        assert!(graph.find_entry("Name 1", PageKind::Page).is_some());
        assert_eq!(GRAPH_PREAMBLE_READS.with(|reads| reads.get()), 8,
            "I-12/E: direct reads reuse the snapshot's complete claimant index; exemplar page_identity.rs");
        let _ = fs::remove_dir_all(&dir);
    }
}

fn portable_component(name: &str) -> bool {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.ends_with([' ', '.'])
        || name.chars().any(|character| {
            character.is_control()
                || matches!(character, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let device = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    !matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

/// Whether `:hidden` excludes graph-relative `relative`: an entry is a
/// byte-exact prefix after one optional trailing `/`; an empty entry, or a
/// `:hidden` value that failed to parse (`Config::hidden_parse_failed_closed`:
/// a torn or hand-broken config.edn, delivered by sync or an external editor),
/// hides everything. An entry with a leading `/`, leading or trailing
/// (Unicode) whitespace, or a nonportable component is inert (master
/// `GraphTextScope::new` / `lexical_components`).
pub(crate) fn configured_hidden(relative: &str, config: &Config) -> bool {
    config.hidden_parse_failed_closed
        || config.hidden.iter().any(|prefix| {
            if prefix.is_empty() {
                return true;
            }
            let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
            if prefix.starts_with('/')
                || prefix != prefix.trim()
                || prefix
                    .split('/')
                    .any(|part| !portable_component(part) || matches!(part, "." | ".."))
            {
                return false;
            }
            relative.starts_with(prefix)
        })
}

pub(crate) fn graph_text_directory_scannable(root: &Path, path: &Path, config: &Config) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            return false;
        };
        let Some(part) = part.to_str() else {
            return false;
        };
        if !portable_component(part)
            || part.starts_with('.')
            || part.eq_ignore_ascii_case("node_modules")
        {
            return false;
        }
        parts.push(part);
    }
    if let Some(first) = parts.first() {
        if ["assets", "publish", "published-queries"]
            .iter()
            .any(|excluded| first.eq_ignore_ascii_case(excluded))
        {
            return false;
        }
    }
    if parts
        .first()
        .is_some_and(|part| part.eq_ignore_ascii_case("logseq"))
    {
        if parts.get(1).is_some_and(|part| {
            ["bak", "version-files", ".recycle", ".tine-trash"]
                .iter()
                .any(|excluded| part.eq_ignore_ascii_case(excluded))
        }) {
            return false;
        }
    }
    !configured_hidden(&relative.to_string_lossy().replace('\\', "/"), config)
}

/// Files the watcher must observe, including provider conflict copies that
/// appear in the conflict list but must never become page claimants.
pub(crate) fn graph_text_watch_relevant(root: &Path, path: &Path, config: &Config) -> bool {
    if !is_page_file(path)
        || !graph_text_directory_scannable(root, path.parent().unwrap_or(root), config)
    {
        return false;
    }
    if path.strip_prefix(root).ok().is_some_and(|relative| {
        configured_hidden(&relative.to_string_lossy().replace('\\', "/"), config)
    }) {
        return false;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| !name.starts_with('.') && portable_component(name))
}

pub(crate) fn graph_text_eligible(root: &Path, path: &Path, config: &Config) -> bool {
    graph_text_watch_relevant(root, path, config)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| !is_sync_conflict(stem))
}

pub(crate) fn graph_text_relative_eligible(relative: &str, config: &Config) -> bool {
    graph_text_eligible(Path::new(""), Path::new(relative), config)
}

/// Read only as much of the page as settles its preamble
/// (`tine_core::model::preamble_read`) instead of every block of every page
/// during name discovery; the title comes from the same answerer the page
/// model agrees with. A full parse is still done by the cache builder and
/// page reader.
pub(super) fn effective_page_name(
    path: &Path,
    stem: &str,
    name_fmt: FileNameFormat,
) -> io::Result<String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(decode_page_name(stem, name_fmt))
        }
        Err(error) => return Err(error),
    };
    #[cfg(test)]
    super::GRAPH_PREAMBLE_READS.with(|reads| reads.set(reads.get() + 1));
    let len = file.metadata()?.len();
    if len > PARSE_INPUT_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            ParseInputTooLarge { len },
        ));
    }
    let mut reader = BufReader::new(file.take(PARSE_INPUT_MAX_BYTES + 1));
    let mut preamble = String::new();
    let mut line = String::new();
    let format = Format::from_path(path);
    let title = loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break tine_core::model::page_title_from_preamble(&preamble, format);
        }
        preamble.push_str(&line);
        if preamble.len() as u64 > PARSE_INPUT_MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                ParseInputTooLarge {
                    len: preamble.len() as u64,
                },
            ));
        }
        match tine_core::model::preamble_read(&preamble, format) {
            PreambleRead::Settled(title) => break title,
            PreambleRead::More => {}
            PreambleRead::Whole => {
                reader.read_to_string(&mut preamble)?;
                if preamble.len() as u64 > PARSE_INPUT_MAX_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        ParseInputTooLarge {
                            len: preamble.len() as u64,
                        },
                    ));
                }
                break tine_core::model::page_title_from_preamble(&preamble, format);
            }
        }
    };
    Ok(title.unwrap_or_else(|| decode_page_name(stem, name_fmt)))
}

impl Graph {
    pub(super) fn discover_page_name(
        &self,
        path: &Path,
        stem: &str,
        fmt: FileNameFormat,
    ) -> Option<String> {
        match effective_page_name(path, stem, fmt) {
            Ok(name) => {
                let id = crate::FileId::from(self.rel_path(path));
                self.discovery_errors
                    .write()
                    .unwrap()
                    .retain(|(failed, _)| *failed != id);
                Some(name)
            }
            Err(error) => {
                self.discovery_errors
                    .write()
                    .unwrap()
                    .push((crate::FileId::from(self.rel_path(path)), error.into()));
                // Physical access remains possible; page reads still return
                // their typed decode/size error. No complete name inventory
                // may use this tentative filename while discovery is partial.
                Some(decode_page_name(stem, fmt))
            }
        }
    }
}

pub(super) fn list_graph_pages(graph: &Graph) -> Vec<PageEntry> {
    list_graph_pages_kind(graph, None)
}

/// Kind selection precedes preamble reads. Journal identity depends on its
/// date filename, so a cold journal lookup never needs ordinary page titles.
pub(crate) fn list_graph_pages_kind(graph: &Graph, kind: Option<PageKind>) -> Vec<PageEntry> {
    #[cfg(test)]
    super::GRAPH_LIST_CALLS.with(|calls| calls.set(calls.get() + 1));
    let mut entries = Vec::new();
    let root = &graph.root;
    let format = graph.current_journal_format();
    let name_format = graph.current_config().file_name_format;
    let journals = graph.journals_path();
    let config = graph.current_config();
    let start = if kind == Some(PageKind::Journal) {
        &journals
    } else {
        root
    };
    if !graph_text_directory_scannable(root, start, &config) {
        return entries;
    }
    let mut failures = Vec::new();
    let walk_errors = walk_graph_page_files(root, start, &config, |path| {
        if kind.is_some_and(|kind| (kind == PageKind::Journal) != path.starts_with(&journals)) {
            return;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            return;
        };
        let (name, kind, date_key) = if path.starts_with(&journals) {
            match format.parse(stem) {
                Some(date) => (
                    format.title(date),
                    PageKind::Journal,
                    Some(date.ordinal_key()),
                ),
                None => (stem.to_owned(), PageKind::Journal, None),
            }
        } else {
            (
                match effective_page_name(&path, stem, name_format) {
                    Ok(name) => name,
                    Err(error) => {
                        failures.push((crate::FileId::from(graph.rel_path(&path)), error.into()));
                        // Keep the physical claim; the file is reported as
                        // unreadable and the readable names stay available.
                        decode_page_name(stem, name_format)
                    }
                },
                PageKind::Page,
                None,
            )
        };
        entries.push(PageEntry {
            name,
            kind,
            date_key,
            rel_path: Some(graph.rel_path(&path).into()),
            path,
        });
    });
    failures.extend(
        walk_errors
            .into_iter()
            .map(|(path, error)| (crate::FileId::from(graph.rel_path(&path)), error.into())),
    );
    let mut known = graph.discovery_errors.write().unwrap();
    known.retain(|(id, _)| match kind {
        None => false,
        Some(PageKind::Journal) => !root.join(id.as_str()).starts_with(&journals),
        Some(PageKind::Page) => root.join(id.as_str()).starts_with(&journals),
    });
    known.extend(failures);
    entries
}

fn walk_graph_page_files(
    root: &Path,
    start: &Path,
    config: &Config,
    mut visit: impl FnMut(PathBuf),
) -> Vec<(PathBuf, io::Error)> {
    let mut errors = Vec::new();
    let mut pending = vec![start.to_path_buf()];
    while let Some(dir) = pending.pop() {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::readdir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                errors.push((dir, error));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push((dir.clone(), error));
                    continue;
                }
            };
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    errors.push((path, error));
                    continue;
                }
            };
            if kind.is_file() && graph_text_eligible(root, &path, config) {
                visit(path);
            } else if kind.is_dir() && graph_text_directory_scannable(root, &path, config) {
                pending.push(path);
            }
        }
    }
    errors
}
