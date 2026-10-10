//! Pages a page host holds (STEP3 §5, R13, A-V4, A-H1). While a host
//! holds a page, its publication consumer (or a reservation's transaction)
//! is the page's only index writer. Held bytes are a page *source*, not an
//! overlay: every whole-graph build and every page read takes a page's
//! content from [`Graph::source`], before anything else reads that page's
//! file, so the page set, the name and the document all come from the
//! bytes that writer last indexed, never from disk: a newer disk read
//! indexed there could be overwritten by an older host event still on its
//! way to the consumer (REVIEW-3a V4, REVIEW-3a2 R2). With no host running
//! nothing is held, every source is the file, and builds and reads do the
//! I/O they did before.

use super::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

/// A page file's path under the store's resolved root: the held map's
/// only key (A-H1). Only [`Graph::page_path`] and [`Graph::page_path_of`]
/// build one, so a hold, its indexing and every lookup spell the root the
/// same way on every platform (a Windows temp dir's 8.3 or `\\?\` form, a
/// symlinked or `..` root).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PagePath(PathBuf);

impl std::ops::Deref for PagePath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl From<PagePath> for PathBuf {
    fn from(path: PagePath) -> Self {
        path.0
    }
}

/// A held path's owner key and what its index writer last indexed since
/// the hold began: `None` until the first publication, `Some(None)` for no
/// file. The bytes are the consumer's own buffer, shared, not copied.
type Entry = (String, Option<Option<Arc<[u8]>>>);

#[derive(Default)]
pub(crate) struct HeldPages {
    /// Changed only under the store writer (holds, releases, and every
    /// index publication), so a reconcile holding it sees a fixed set.
    paths: RwLock<HashMap<PagePath, Entry>>,
    /// Bumped by every new hold. A whole-graph build that read its files
    /// before a hold began declines its install, as for a cache mutation.
    epoch: AtomicU64,
}

/// Where a whole-graph build or a page read takes one page's content.
pub(crate) enum Source {
    /// No host holds it: the file.
    Disk,
    /// Held: the bytes its owner last indexed.
    Held(Arc<[u8]>),
    /// Held, and its owner indexed no file.
    HeldAbsent,
    /// Held, and its owner has not indexed it yet: its pending publication
    /// will. A build leaves it out; a read parses the file, unpublished.
    HeldUnindexed,
}

impl HeldPages {
    /// Hand `path`'s index to `key`'s owner. Holding it again for the same
    /// key keeps what that owner already indexed.
    pub(crate) fn hold(&self, path: PagePath, key: String) {
        let mut paths = self.paths.write().unwrap();
        if paths.get(&path).is_some_and(|(held, _)| *held == key) {
            return;
        }
        paths.insert(path, (key, None));
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// Return `path` to the watcher; true when it was held.
    pub(crate) fn release(&self, path: &PagePath) -> bool {
        self.paths.write().unwrap().remove(path).is_some()
    }

    /// Return every held path to the watcher.
    pub(crate) fn release_all(&self) -> Vec<PagePath> {
        let mut paths = self.paths.write().unwrap();
        paths.drain().map(|(path, _)| path).collect()
    }

    /// The owner's page moved to `to`, another spelling of the same entry
    /// (an alias spelling move, STEP3 Q4): what it indexed stays its own.
    pub(crate) fn respell(&self, from: &PagePath, to: PagePath) {
        let mut paths = self.paths.write().unwrap();
        if let Some(entry) = paths.remove(from) {
            paths.insert(to, entry);
        }
    }

    /// The owner key of a held path.
    pub(crate) fn key(&self, path: &PagePath) -> Option<String> {
        let paths = self.paths.read().unwrap();
        paths.get(path).map(|(key, _)| key.clone())
    }

    /// Record the bytes an index writer is about to publish for `path`, if
    /// it is held. Called before the publication moves the cache
    /// generation, so a build that read the earlier bytes declines.
    pub(crate) fn indexed(&self, path: &PagePath, bytes: impl FnOnce() -> Option<Arc<[u8]>>) {
        if let Some((_, indexed)) = self.paths.write().unwrap().get_mut(path) {
            *indexed = Some(bytes());
        }
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
}

impl Graph {
    /// The page file `rel` (a root-relative spelling) under the store's root.
    pub(crate) fn page_path(&self, rel: &str) -> PagePath {
        PagePath(self.root.join(rel))
    }

    /// The page file at `abs`, a path a listing or the watcher found under
    /// the store's root; None for one outside it, which nothing holds.
    pub(crate) fn page_path_of(&self, abs: &Path) -> Option<PagePath> {
        let rel = abs.strip_prefix(&self.root).ok()?;
        Some(PagePath(self.root.join(rel)))
    }

    /// The one per-page seam (A-H1): where a build or read takes the page
    /// at `path` from. With no host, `Disk` for every page.
    pub(crate) fn source(&self, path: &Path) -> Source {
        let Some(path) = self.page_path_of(path) else {
            return Source::Disk;
        };
        match self.held.paths.read().unwrap().get(&path) {
            None => Source::Disk,
            Some((_, None)) => Source::HeldUnindexed,
            Some((_, Some(None))) => Source::HeldAbsent,
            Some((_, Some(Some(bytes)))) => Source::Held(Arc::clone(bytes)),
        }
    }

    /// The content `source` gives the page at `path`: the file's, or its
    /// owner's validated bytes; None for a held page with none to build.
    /// The only content read of a whole-graph build.
    pub(super) fn source_content(
        &self,
        path: &Path,
        source: &Source,
    ) -> io::Result<Option<String>> {
        match source {
            Source::Disk => read_parse_input(path).map(Some),
            Source::Held(bytes) => {
                validate_parse_bytes_for_path(bytes, path)?;
                String::from_utf8(bytes.to_vec())
                    .map(Some)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            }
            Source::HeldAbsent | Source::HeldUnindexed => Ok(None),
        }
    }

    /// A whole-graph build's pages and their sources (A-H1): `entries`
    /// (the build's selection from the listing `listed`) with each page's
    /// source, plus every held page its owner indexed bytes for that the
    /// listing lacks (a removal the owner has not observed yet). The added
    /// entries are returned apart, for the build's named listing.
    pub(super) fn build_sources(
        &self,
        listed: &[PageEntry],
        entries: Vec<PageEntry>,
    ) -> (Vec<(PageEntry, Source)>, Vec<PageEntry>) {
        let mut sourced: Vec<(PageEntry, Source)> = entries
            .into_iter()
            .map(|entry| {
                let source = self.source(&entry.path);
                (entry, source)
            })
            .collect();
        let held: Vec<(PagePath, Arc<[u8]>)> = self
            .held
            .paths
            .read()
            .unwrap()
            .iter()
            .filter_map(|(path, (_, indexed))| Some((path.clone(), indexed.clone()??)))
            .collect();
        if held.is_empty() {
            return (sourced, Vec::new());
        }
        let listed: HashSet<&Path> = listed.iter().map(|entry| entry.path.as_path()).collect();
        let config = self.current_config();
        let (format, journals) = (self.current_journal_format(), self.journals_path());
        let formats = (&*format, journals.as_path(), config.file_name_format);
        let mut added = Vec::new();
        for (path, bytes) in held {
            if listed.contains(&*path)
                || path_is_sync_conflict(&path)
                || !page_identity::graph_text_eligible(&self.root, &path, &config)
            {
                continue;
            }
            if let Some(entry) = page_identity::listed_entry(self, formats, path.into()) {
                added.push(entry.clone());
                sourced.push((entry, Source::Held(bytes)));
            }
        }
        (sourced, added)
    }

    /// A page's name from the content its document is built from (A-H1),
    /// for every build and source: an ordinary page's `title::`, else its
    /// decoded file stem; a journal's is its date.
    pub(super) fn name_from(&self, entry: &PageEntry, content: &str) -> io::Result<String> {
        if entry.kind != PageKind::Page {
            return Ok(entry.name.clone());
        }
        let stem = entry
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let stem = decode_page_name(stem, self.current_config().file_name_format);
        page_identity::effective_page_name_from_text(&entry.path, &stem, content)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::{Store, TestPause};
    use crate::PageId;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    /// A build that read a page before a host held it declines its
    /// install even when the owner has not published yet (A-V4): its disk
    /// bytes could be newer than the owner's pending first observation.
    /// Forced and on-demand builds of a held, indexed page parse the
    /// owner's bytes.
    #[test]
    fn a_build_that_read_a_page_before_its_hold_declines() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        std::fs::write(temp.path().join("pages/a.md"), "- a\n").unwrap();
        // A root given in a non-canonical spelling (as Windows temp dirs
        // are): the store resolves it, and a hold is keyed as production
        // keys it, by the store's root.
        let given = temp.path().join("pages").join("..");
        let store = Arc::new(Store::open(&given, Default::default()).unwrap().0);
        let path = store.graph.page_path("pages/a.md");
        assert!(
            store
                .graph
                .list_pages()
                .iter()
                .any(|entry| entry.path.as_path() == &*path),
            "A-H1: a hold's key is not the path the whole-graph build looks up"
        );
        store.page(&PageId::from("pages/a.md")).unwrap();
        store.whole_graph().unwrap();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let build = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.graph.rebuild_cache_cancellable(|| false))
        };
        {
            let (state, ready) = &*pause;
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut state = state.lock().unwrap();
            while !state.0 {
                assert!(Instant::now() < deadline, "the build never read its page");
                state = ready
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
        }
        let gen = store
            .graph
            .cache_gen
            .load(std::sync::atomic::Ordering::Acquire);
        store.graph.held.hold(path.clone(), "pages/a.md".into());
        pause.0.lock().unwrap().1 = true;
        pause.1.notify_all();
        assert!(
            !build.join().unwrap(),
            "A-V4: a build installed a page it read before the page's hold began"
        );
        // Declined for the hold, not for a cache mutation.
        assert_eq!(
            store
                .graph
                .cache_gen
                .load(std::sync::atomic::Ordering::Acquire),
            gen
        );
        *store.graph.warm_after_first_page_pause.lock().unwrap() = None;
        // Held and indexed: the next build parses the owner's bytes.
        store
            .graph
            .held
            .indexed(&path, || Some(Arc::from(&b"- owner\n"[..])));
        assert!(store.graph.rebuild_cache_cancellable(|| false));
        assert_eq!(
            store.graph.cached_rev(&path),
            Some(crate::model::content_rev("- owner\n"))
        );
        // A failed publication drops the cache (transaction.rs); the next
        // whole-graph question builds it on demand, from the same bytes.
        store.graph.invalidate_cache();
        store.graph.with_pages(|_| ());
        assert_eq!(
            store.graph.cached_rev(&path),
            Some(crate::model::content_rev("- owner\n")),
            "A-V4: an on-demand build indexed a held page from disk"
        );
        store.close();
    }

    /// A graph with `files`, opened on a non-canonical spelling of its
    /// root, its whole graph loaded.
    fn graph(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Arc<Store>) {
        let temp = tempfile::tempdir().unwrap();
        for area in ["pages", "journals"] {
            std::fs::create_dir_all(temp.path().join(area)).unwrap();
        }
        for (rel, bytes) in files {
            std::fs::write(temp.path().join(rel), bytes).unwrap();
        }
        let given = temp.path().join("pages").join("..");
        let store = Arc::new(Store::open(&given, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        (temp, store)
    }

    fn names(store: &Store, path: &std::path::Path) -> Vec<String> {
        store.graph.with_pages(|pages| {
            pages
                .iter()
                .filter(|(entry, _)| entry.path == path)
                .map(|(entry, _)| entry.name.clone())
                .collect()
        })
    }

    /// REVIEW-3a2 R2 (membership, A-H1): a held page its owner indexed
    /// stays in every rebuild while its file is gone from disk; the owner's
    /// pending observation of the removal owns that change, not the build.
    #[test]
    fn review2_rebuild_preserves_a_held_file_missing_from_disk() {
        let (_temp, store) = graph(&[("pages/a.md", b"- a\n")]);
        let id = PageId::from("pages/a.md");
        store.hold_page(&id).unwrap();
        let path = store.graph.page_path("pages/a.md");
        std::fs::remove_file(&*path).unwrap();
        assert!(store.graph.rebuild_cache_cancellable(|| false));
        let owner = Some(crate::model::content_rev("- a\n"));
        assert_eq!(store.graph.cached_rev(&path), owner, "forced rebuild");
        store.graph.invalidate_cache();
        store.graph.with_pages(|_| ());
        assert_eq!(store.graph.cached_rev(&path), owner, "on-demand build");
        assert_eq!(names(&store, &path), ["a"]);
        store.close();
    }

    /// REVIEW-3a2 R2 (naming, A-H1): every build names a held page from
    /// the owner's bytes its document is built from, never from the file.
    #[test]
    fn review2_ondemand_names_a_held_page_from_owner_bytes() {
        let (_temp, store) = graph(&[("pages/a.md", b"title:: Owner\n- a\n")]);
        store.hold_page(&PageId::from("pages/a.md")).unwrap();
        let path = store.graph.page_path("pages/a.md");
        std::fs::write(&*path, "title:: Disk\n- b\n").unwrap();
        store.graph.invalidate_cache();
        assert_eq!(names(&store, &path), ["Owner"], "on-demand build");
        assert!(store.graph.rebuild_cache_cancellable(|| false));
        assert_eq!(names(&store, &path), ["Owner"], "forced rebuild");
        store.close();
    }

    /// REVIEW-3a2 R2 (reads, A-H1): a read of a held, indexed page opens
    /// no part of its file, not even the preamble that names it.
    #[test]
    fn review2_held_read_uses_no_disk_preamble() {
        let (_temp, store) = graph(&[("pages/a.md", b"title:: A\n- a\n")]);
        let id = PageId::from("pages/a.md");
        store.hold_page(&id).unwrap();
        let path = store.graph.page_path("pages/a.md");
        std::fs::write(&*path, b"title:: \xff\xfe\n- b\n").unwrap();
        crate::model::GRAPH_PREAMBLE_READS.with(|reads| reads.set(0));
        let read = store.page(&id).unwrap();
        assert_eq!(read.doc.name, "A");
        assert_eq!(
            String::from(read.rev),
            crate::model::content_rev("title:: A\n- a\n")
        );
        assert_eq!(
            crate::model::GRAPH_PREAMBLE_READS.with(|reads| reads.get()),
            0,
            "A-H1: a held read opened its file's preamble"
        );
        store.close();
    }

    /// The body of the function whose signature starts `signature`.
    fn body<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("no {signature}"));
        let open = source[start..].find('{').unwrap() + start;
        let mut depth = 0;
        for (offset, c) in source[open..].char_indices() {
            depth += match c {
                '{' => 1,
                '}' => -1,
                _ => 0,
            };
            if depth == 0 {
                return &source[open..=open + offset];
            }
        }
        panic!("unclosed {signature}")
    }

    /// Where whole-graph builds and page reads could reach page content
    /// other than through the seam.
    fn bypasses(sources: &[(&str, &str, &str)]) -> Vec<String> {
        const CONTENT: &[&str] = &[
            "read_parse_input(",
            "read_parse_bytes(",
            "fs::read(",
            "read_to_string(",
            "entry_for_path(",
            "load_page(",
            "load_by_validated_path(",
            "page_target(",
        ];
        let mut found = Vec::new();
        for (file, signature, source) in sources {
            let body = body(source, signature);
            let site = format!("{file} `{signature}`");
            let build = !signature.contains("page(&self, id");
            if build {
                if !body.contains("source_content(") && !body.contains("build_sources(") {
                    found.push(format!("{site} takes no content from the seam"));
                }
                for call in CONTENT {
                    if body.contains(call) {
                        found.push(format!("{site} reads page content with {call}"));
                    }
                }
            } else {
                // A read asks the seam before it names or reads the file.
                let held: Vec<_> = body.match_indices("held_page(").map(|(i, _)| i).collect();
                for call in [
                    "page_entry(",
                    "parse_page(",
                    "page_target(",
                    "entry_for_path(",
                ] {
                    for (at, _) in body.match_indices(call) {
                        if !held.iter().any(|&h| h < at) {
                            found.push(format!("{site} calls {call} before the seam"));
                        }
                    }
                }
            }
        }
        found
    }

    /// A-H1: whole-graph builds and page reads reach a page's content only
    /// through the seam (`Graph::source`/`source_content`/`build_sources`),
    /// so a held page's set membership, name and document all come from
    /// its owner's bytes (I-1-style census; exemplar `Graph::load_all_pages`).
    #[test]
    fn builds_and_reads_take_page_content_only_through_the_seam() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let read = |file: &str| std::fs::read_to_string(src.join(file)).unwrap();
        let (model, parse, store) = (
            read("model.rs"),
            read("model/page_parse.rs"),
            read("store.rs"),
        );
        let sources = [
            ("model.rs", "fn warm_page_cache_inner(", model.as_str()),
            ("model.rs", "fn load_all_pages(", model.as_str()),
            (
                "model/page_parse.rs",
                "fn parse_page_entry_isolated(",
                parse.as_str(),
            ),
            (
                "store.rs",
                "pub fn page(&self, id: &PageId)",
                store.as_str(),
            ),
        ];
        let found = bypasses(&sources);
        assert!(
            found.is_empty(),
            "A-H1: a whole-graph build or page read reaches page content outside the \
             held-page seam: {found:?}. Exemplar model.rs Graph::load_all_pages \
             (build_sources, then source_content per page)"
        );
    }

    #[test]
    fn a_planted_bypass_of_the_seam_fails_the_guard() {
        let planted = r#"
            fn load_all_pages(&self) { let t = read_parse_input(&e.path); }
            fn warm_page_cache_inner(&self) { let (e, s) = self.build_sources(l, e); self.source_content(&p, &s); }
            pub fn page(&self, id: &PageId) {
                let entry = self.page_entry(&id, &path)?;
                if let Some(doc) = self.held_page(&path)? { return doc; }
            }
        "#;
        let found = bypasses(&[
            ("p.rs", "fn load_all_pages(", planted),
            ("p.rs", "fn warm_page_cache_inner(", planted),
            ("p.rs", "pub fn page(&self, id: &PageId)", planted),
        ]);
        assert_eq!(
            found,
            [
                "p.rs `fn load_all_pages(` takes no content from the seam",
                "p.rs `fn load_all_pages(` reads page content with read_parse_input(",
                "p.rs `pub fn page(&self, id: &PageId)` calls page_entry( before the seam",
            ]
        );
    }
}
