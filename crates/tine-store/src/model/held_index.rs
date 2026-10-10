//! Pages a page host holds (STEP3 §5, R13, A-V4, A-H1, A-H2). While a
//! host holds a page, its publication consumer (or a reservation's
//! transaction) is the page's only index writer, and everything installed
//! about the page comes from the bytes that writer last published
//! (`derived.rs`, (G)): a newer disk read installed there could be
//! overwritten by an older host event still on its way to the consumer
//! (REVIEW-3a V4, REVIEW-3a2 R2). A page read takes a held page's content
//! from [`Graph::source`] before anything else reads that page's file. With
//! no host running nothing is held, every source is the file, and builds
//! and reads do the I/O they did before.

use super::entry_identity::{fold_leaf, Identity};
use super::*;
use std::collections::HashSet;

/// Where a page read takes one page's content.
pub(crate) enum Source {
    /// No host holds it: the file.
    Disk,
    /// Held: the bytes its owner last indexed.
    Held(Arc<[u8]>),
    /// Held, and its owner indexed no file.
    HeldAbsent,
    /// Held and not indexed yet (its pending publication will), or a path
    /// whose identity against the held keys is unknown (B1): a read parses
    /// the file, unpublished.
    HeldUnindexed,
}

impl Graph {
    /// What `path` names among the held keys (B1, [`Graph::identify`]).
    /// With nothing held, `New` at no cost.
    pub(crate) fn held_identity(&self, path: &Path) -> Identity {
        if self.held.is_empty() {
            return Identity::New;
        }
        let found = path
            .file_name()
            .map_or_else(Vec::new, |leaf| self.held.candidates(&fold_leaf(leaf)));
        self.identify(path, &|_| found.clone())
    }

    /// The held key `path` names, if it names one.
    pub(crate) fn held_key(&self, path: &Path) -> Option<String> {
        match self.held_identity(path) {
            Identity::Key(key) => Some(key),
            _ => None,
        }
    }

    /// The source of a path whose identity against the held keys is known.
    fn source_of(&self, identity: &Identity) -> Source {
        match identity {
            Identity::New | Identity::Outside => Source::Disk,
            Identity::Unknown { .. } => Source::HeldUnindexed,
            Identity::Key(key) => match self.held.indexed_of(key) {
                None => Source::Disk,
                Some(None) => Source::HeldUnindexed,
                Some(Some(None)) => Source::HeldAbsent,
                Some(Some(Some(bytes))) => Source::Held(bytes),
            },
        }
    }

    /// The one per-page seam (A-H1): where a build or read takes the page
    /// at `path` from. With no host, `Disk` for every page.
    pub(crate) fn source(&self, path: &Path) -> Source {
        self.source_of(&self.held_identity(path))
    }

    /// The listed paths a whole-graph build leaves out: those a held or
    /// unknown identity names ((G)); their rows are their owners'.
    pub(super) fn withheld(&self, listed: &[PageEntry]) -> HashSet<PathBuf> {
        if self.held.is_empty() {
            return HashSet::new();
        }
        listed
            .iter()
            .filter(|entry| !self.disk_sourced(&entry.path))
            .map(|entry| entry.path.clone())
            .collect()
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
    /// Forced and on-demand builds of a held, indexed page install the
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
        let path = store.graph.root.join("pages/a.md");
        assert!(
            store
                .graph
                .list_pages()
                .iter()
                .any(|entry| entry.path == path),
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
        store.graph.hold_unhosted("pages/a.md");
        pause.0.lock().unwrap().1 = true;
        pause.1.notify_all();
        assert!(
            !build.join().unwrap(),
            "A-V4: a build installed a page it read before the page's hold began"
        );
        // A new hold retires the page's row (R1): nothing about it is
        // installed until its owner publishes.
        assert_eq!(store.graph.cached_rev(&path), None);
        *store.graph.warm_after_first_page_pause.lock().unwrap() = None;
        // Held and indexed: the next build installs the owner's bytes.
        let owner = Some(Arc::from(&b"- owner\n"[..]));
        assert!(store.graph.publish_owned("pages/a.md", owner, None).is_ok());
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
        let path = store.graph.root.join("pages/a.md");
        std::fs::remove_file(&path).unwrap();
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
        let path = store.graph.root.join("pages/a.md");
        std::fs::write(&path, "title:: Disk\n- b\n").unwrap();
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
        let path = store.graph.root.join("pages/a.md");
        std::fs::write(&path, b"title:: \xff\xfe\n- b\n").unwrap();
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

    /// Calls that read page content or name a page from its file.
    const CONTENT: &[&str] = &[
        "read_parse_input(",
        "read_parse_bytes(",
        "fs::read(",
        "read_to_string(",
        "entry_for_path(",
        "load_page(",
        "load_by_validated_path(",
        "page_target(",
        "page_entry(",
        "parse_page(",
    ];

    /// The calls a page read may make before it asks the seam: none reads
    /// page content (`page_spot`'s own body is checked for that).
    const BEFORE_SEAM: &[&str] = &[
        "matches",
        "lock",
        "unwrap",
        "is_closed",
        "Err",
        "now",
        "page_writer_wait",
        "elapsed",
        "cache_generation",
        "page_spot",
        "Some",
    ];

    /// The calls in `body` that no `held_page(` dominates: one is
    /// dominated when a `held_page(` precedes it in a block that still
    /// encloses it. Line comments are ignored.
    fn undominated_calls(body: &str) -> Vec<String> {
        let code: String = body
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut depth, mut seam) = (0usize, None::<usize>);
        let mut calls = Vec::new();
        for (at, c) in code.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    if seam == Some(depth) {
                        seam = None;
                    }
                    depth -= 1;
                }
                '(' if seam.is_none() => {
                    let before = code[..at].trim_end_matches('!');
                    let start = before
                        .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .map_or(0, |i| i + 1);
                    if start < before.len() {
                        calls.push(before[start..].to_owned());
                    }
                }
                _ => {}
            }
            if seam.is_none() && code[at..].starts_with("held_page(") {
                seam = Some(depth);
            }
        }
        calls
    }

    /// Where a page read could reach page content before it asks the seam
    /// (`held_page`): `page`, a `Store::page` body, and `spot`, the
    /// `page_spot` body it may call first.
    fn bypasses(site: &str, page: &str, spot: &str) -> Vec<String> {
        let mut found: Vec<String> = undominated_calls(page)
            .into_iter()
            .filter(|call| !BEFORE_SEAM.contains(&call.as_str()))
            .map(|call| format!("{site} calls {call}( before the seam"))
            .collect();
        for call in CONTENT {
            if spot.contains(call) {
                found.push(format!("{site} page_spot reads page content with {call}"));
            }
        }
        found
    }

    /// A-H1/A-H2: a page read asks the seam (`Store::held_page`) before
    /// anything names or reads the page's file, so a held page's read
    /// answers its owner's bytes. Builds need no such rule: they may read
    /// any file, and installation applies (G) (`derived.rs`). I-1-style
    /// census; exemplar `Store::page`.
    #[test]
    fn page_reads_ask_the_seam_before_reading_the_file() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let read = |file: &str| std::fs::read_to_string(src.join(file)).unwrap();
        let (store, page_read) = (read("store.rs"), read("store/page_read.rs"));
        let found = bypasses(
            "store.rs `Store::page`",
            body(&store, "pub fn page(&self, id: &PageId)"),
            body(&page_read, "fn page_spot("),
        );
        assert!(
            found.is_empty(),
            "A-H1: a page read reaches page content before the held-page seam: {found:?}. \
             Exemplar store.rs Store::page (page_spot, then held_page, then the file)"
        );
    }

    /// The guard catches a direct read, a helper called before the seam
    /// (which the pre-A-H2 guard, a list of named content calls, let
    /// through) and a content read inside `page_spot`.
    #[test]
    fn a_planted_bypass_of_the_seam_fails_the_guard() {
        let direct = r#"
            pub fn page(&self, id: &PageId) {
                let (id, path) = self.page_spot(id)?;
                let entry = self.page_entry(&id, &path)?;
                if let Some(doc) = self.held_page(&path)? { return doc; }
            }
        "#;
        let helper = r#"
            pub fn page(&self, id: &PageId) {
                let (id, path) = self.page_spot(id)?;
                self.warm_one(&path);
                if let Some(doc) = self.held_page(&path)? { return doc; }
                let entry = self.page_entry(&id, &path)?;
            }
        "#;
        let spot = "fn page_spot(&self) { let text = fs::read(&path)?; }";
        let page = |source| body(source, "pub fn page(&self, id: &PageId)");
        assert_eq!(
            bypasses("p.rs", page(direct), "{}"),
            ["p.rs calls page_entry( before the seam"]
        );
        assert_eq!(
            bypasses("p.rs", page(helper), "{}"),
            ["p.rs calls warm_one( before the seam"]
        );
        assert_eq!(
            bypasses(
                "p.rs",
                page(helper.replace("self.warm_one(&path);", "").as_str()),
                spot
            ),
            ["p.rs page_spot reads page content with fs::read("]
        );
    }
}
