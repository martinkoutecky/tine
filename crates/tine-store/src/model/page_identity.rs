//! Graph text discovery and effective page names. The path remains the stable
//! file identity; the preamble title is the logical page name.

use super::*;
use std::io::{BufRead, BufReader};

impl Graph {
    /// Build the page list and effective-name claimants from one cold walk.
    pub(crate) fn snapshot_name_index(
        &self,
    ) -> (
        Arc<Vec<PageEntry>>,
        HashMap<(PageKind, String), Vec<PageEntry>>,
    ) {
        let format = self.current_journal_format();
        let mut claimants: HashMap<(PageKind, String), Vec<PageEntry>> = HashMap::new();
        let entries = list_graph_pages(self);
        for entry in &entries {
            claimants
                .entry((entry.kind, tine_core::refs::page_key(&entry.name)))
                .or_default()
                .push(entry.clone());
        }
        for entries in claimants.values_mut() {
            entries.sort_by(|a, b| {
                compare_page_claimants(a, b, &format, self.current_config().file_name_format)
            });
        }
        let list = Arc::new(dedup_journal_days(
            entries,
            &format,
            self.current_config().file_name_format,
        ));
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        *self.page_list_cache.write().unwrap() = Some((gen, Arc::clone(&list)));
        (list, claimants)
    }
}

#[cfg(test)]
mod cold_index_tests {
    use super::*;

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

pub(crate) fn configured_hidden(relative: &str, config: &Config) -> bool {
    config.hidden.iter().any(|prefix| {
        if prefix.is_empty() {
            return true;
        }
        let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
        if prefix.starts_with('/')
            || prefix.starts_with(' ')
            || prefix.ends_with(' ')
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

/// Read only the preamble instead of every block of every page during name
/// discovery. A full parse is still done by the cache builder and page reader.
pub(super) fn effective_page_name(path: &Path, stem: &str, name_fmt: FileNameFormat) -> String {
    let title = (|| {
        let file = fs::File::open(path).ok()?;
        #[cfg(test)]
        super::GRAPH_PREAMBLE_READS.with(|reads| reads.set(reads.get() + 1));
        if file.metadata().ok()?.len() > PARSE_INPUT_MAX_BYTES {
            return None;
        }
        let mut reader = BufReader::new(file);
        let mut preamble = String::new();
        let mut line = String::new();
        let org = Format::from_path(path) == Format::Org;
        loop {
            line.clear();
            let n = reader.read_line(&mut line).ok()?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("- ") || trimmed == "-" || (org && trimmed.starts_with("* ")) {
                break;
            }
            preamble.push_str(&line);
        }
        tine_core::model::page_title_from_preamble(&preamble, Format::from_path(path))
    })();
    title.unwrap_or_else(|| decode_page_name(stem, name_fmt))
}

pub(super) fn list_graph_pages(graph: &Graph) -> Vec<PageEntry> {
    #[cfg(test)]
    super::GRAPH_LIST_CALLS.with(|calls| calls.set(calls.get() + 1));
    let mut entries = Vec::new();
    let root = &graph.root;
    let format = graph.current_journal_format();
    let name_format = graph.current_config().file_name_format;
    let journals = graph.journals_path();
    let config = graph.current_config();
    walk_graph_page_files(root, &config, |path| {
        if !graph_text_eligible(root, &path, &config) {
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
                effective_page_name(&path, stem, name_format),
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
    entries
}

fn walk_graph_page_files(root: &Path, config: &Config, mut visit: impl FnMut(PathBuf)) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::readdir();
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_file() && graph_text_eligible(root, &path, config) {
                visit(path);
            } else if kind.is_dir() && graph_text_directory_scannable(root, &path, config) {
                pending.push(path);
            }
        }
    }
}
