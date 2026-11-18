use super::*;
use std::time::{Duration, Instant};

struct Graph {
    root: tempfile::TempDir,
    repo: Repository,
    cache: Cache,
}
impl Graph {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::init(root.path()).unwrap();
        Self {
            root,
            repo,
            cache: Cache::default(),
        }
    }
    fn commit(&self, path: &str, text: impl AsRef<[u8]>, time: i64) -> Oid {
        let parent = self.repo.head().ok().map(|h| h.peel_to_commit().unwrap());
        let old = parent.as_ref().map(|p| p.tree().unwrap());
        let pages = old
            .as_ref()
            .and_then(|t| t.get_name("pages"))
            .map(|e| self.repo.find_tree(e.id()).unwrap());
        let mut child = self.repo.treebuilder(pages.as_ref()).unwrap();
        child
            .insert(path, self.repo.blob(text.as_ref()).unwrap(), 0o100644)
            .unwrap();
        let mut tree = self.repo.treebuilder(old.as_ref()).unwrap();
        tree.insert("pages", child.write().unwrap(), 0o040000)
            .unwrap();
        let tree = self.repo.find_tree(tree.write().unwrap()).unwrap();
        let sig = git2::Signature::new(
            "Writer",
            "writer@example.invalid",
            &git2::Time::new(time, 0),
        )
        .unwrap();
        self.repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "fixture",
                &tree,
                &parent.iter().collect::<Vec<_>>(),
            )
            .unwrap()
    }
    fn read(&mut self, path: &str, text: &str) -> Arc<Vec<BlockChanged>> {
        self.cache
            .read_page_dates(self.root.path(), path, text)
            .unwrap()
    }
}
fn facts(rows: &[BlockChanged]) -> Vec<Changed> {
    rows.iter().map(|r| r.changed.clone()).collect()
}
fn counts() -> [usize; 6] {
    COUNTS.with(|c| *c.borrow())
}
fn reset_counts() {
    COUNTS.with(|c| *c.borrow_mut() = [0; 6]);
}

#[test]
fn block_dates_use_the_last_touch_not_head_and_cache_all_page_blocks() {
    let mut g = Graph::new();
    g.commit("p.md", "- untouched\n- before\n", 1_700_000_000);
    g.commit("p.md", "- untouched\n- after\n", 1_700_086_400);
    g.commit("other.md", "- unrelated\n", 1_700_172_800);
    reset_counts();
    let rows = g.read("pages/p.md", "- untouched\n- after\n");
    assert_eq!(
        facts(&rows),
        [
            Changed::Committed(1_700_000_000),
            Changed::Committed(1_700_086_400)
        ]
    );
    assert_eq!(
        rows.iter()
            .map(|r| r.block_path.clone())
            .collect::<Vec<_>>(),
        [vec![0], vec![1]]
    );
    // Tree path lookups, walked commits, blob reads, parses, alignments, HEAD refs.
    assert_eq!(counts(), [3, 3, 2, 2, 1, 1]);
    reset_counts();
    assert!(Arc::ptr_eq(
        &rows,
        &g.read("pages/p.md", "- untouched\n- after\n")
    ));
    assert_eq!(
        counts(),
        [0, 0, 0, 0, 0, 1],
        "repeat hovers only check HEAD; no historical objects, parse or alignment"
    );
    g.commit("p.md", "- untouched\n- latest\n", 1_700_259_200);
    assert_eq!(
        facts(&g.read("pages/p.md", "- untouched\n- latest\n"))[1],
        Changed::Committed(1_700_259_200)
    );
    assert!(counts()[1] > 0, "HEAD movement invalidates the cache");
    assert_eq!(
        facts(&g.read("pages/other.md", "- unrelated\n")),
        [Changed::Committed(1_700_172_800)]
    );
}

#[test]
fn block_dates_mark_local_changes_and_never_committed_blocks_honestly() {
    let mut g = Graph::new();
    g.commit("p.md", "- untouched\n- before\n", 100);
    assert_eq!(
        facts(&g.read("pages/p.md", "- untouched\n- edited\n- new\n")),
        [
            Changed::Committed(100),
            Changed::Uncommitted,
            Changed::Uncommitted
        ]
    );
    reset_counts();
    assert_eq!(
        facts(&g.read("pages/p.md", "- untouched\n- edited again\n")),
        [Changed::Committed(100), Changed::Uncommitted]
    );
    assert_eq!(
        counts(),
        [0, 0, 0, 1, 1, 1],
        "only changed working text is realigned"
    );
    let mut unborn = Graph::new();
    reset_counts();
    assert_eq!(
        facts(&unborn.read("pages/new.md", "- new\n")),
        [Changed::Uncommitted]
    );
    assert_eq!(
        counts(),
        [0, 0, 0, 1, 1, 1],
        "unborn HEAD has no history to inspect"
    );
    reset_counts();
    assert_eq!(
        facts(&g.read("pages/new.md", "- new\n")),
        [Changed::Uncommitted]
    );
    assert_eq!(
        counts(),
        [1, 0, 0, 1, 1, 1],
        "a file absent at HEAD needs one tree lookup and no walk"
    );
}

#[test]
fn block_dates_non_git_is_unknown_and_bad_paths_fail() {
    let root = tempfile::tempdir().unwrap();
    let mut cache = Cache::default();
    assert!(cache
        .read_page_dates(root.path(), "pages/p.md", "- note\n")
        .unwrap()
        .is_empty());
    assert!(cache
        .read_page_dates(root.path(), "../outside.md", "")
        .is_err());
}

#[test]
fn block_dates_malformed_history_is_unknown_and_cached() {
    let mut g = Graph::new();
    g.commit("p.md", [0xff], 100);
    g.commit("p.md", "- readable now\n", 200);
    assert!(g.read("pages/p.md", "- readable now\n").is_empty());
    reset_counts();
    assert!(g.read("pages/p.md", "- readable now\n").is_empty());
    assert_eq!(counts(), [0, 0, 0, 0, 0, 1]);
}

#[test]
fn block_dates_read_changes_no_git_bytes_and_ignores_write_locks() {
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.extend(snapshot(&p));
            } else {
                out.insert(p.clone(), std::fs::read(p).unwrap());
            }
        }
        out
    }
    let mut g = Graph::new();
    g.commit("p.md", "- note\n", 100);
    std::fs::write(g.repo.path().join("index.lock"), "another writer").unwrap();
    let before = snapshot(g.repo.path());
    let _run = super::super::super::git_run::RUN_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    assert_eq!(
        facts(&g.read("pages/p.md", "- note\n")),
        [Changed::Committed(100)]
    );
    assert_eq!(snapshot(g.repo.path()), before);
}

#[test]
fn block_dates_bound_the_walk_and_preserve_recent_dates() {
    let mut g = Graph::new();
    g.commit("p.md", "- ancient\n- before\n", 100);
    for i in 0..WALK_LIMIT {
        g.commit("other.md", &format!("- {i}\n"), 200 + i as i64);
    }
    g.commit("p.md", "- ancient\n- recent\n", 1000);
    reset_counts();
    assert_eq!(
        facts(&g.read("pages/p.md", "- ancient\n- recent\n")),
        [Changed::HistoryLimit, Changed::Committed(1000)]
    );
    assert_eq!(counts()[1], WALK_LIMIT);
    assert_eq!(counts()[0], WALK_LIMIT + 1);
}

#[test]
fn block_dates_org_nested_blocks_and_folds_share_seen_semantics() {
    let mut g = Graph::new();
    g.commit("p.org", "* Parent\n** Child\n* Other\n", 100);
    g.commit("p.org", "* Parent\n** Changed child\n* Other\n", 200);
    assert_eq!(
        facts(&g.read(
            "pages/p.org",
            "* Parent\n:PROPERTIES:\n:collapsed: true\n:END:\n** Changed child\n* Other\n"
        )),
        [
            Changed::Committed(100),
            Changed::Committed(200),
            Changed::Committed(100)
        ]
    );
}

#[test]
fn block_dates_walk_timing_budget() {
    let mut g = Graph::new();
    let text = (0..120)
        .map(|i| format!("- note {i}\n"))
        .collect::<String>();
    g.commit("p.md", &text, 100);
    for i in 0..64 {
        g.commit("other.md", &format!("- {i}\n"), 200 + i);
    }
    reset_counts();
    let start = Instant::now();
    assert_eq!(g.read("pages/p.md", &text).len(), 120);
    let elapsed = start.elapsed();
    eprintln!(
        "block dates: 120 blocks / 65 commits {elapsed:?}, counts {:?}",
        counts()
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "bounded hover walk exceeded 2s debug budget: {elapsed:?}"
    );
}

#[test]
fn block_dates_page_attribution_releases_older_snapshots() {
    let mut g = Graph::new();
    let mut lines: Vec<_> = (0..600).map(|i| format!("- note {i}\n")).collect();
    let mut blobs = vec![None];
    for i in 0..20 {
        lines[i] = format!("- edited note {i}\n");
        let text = lines.concat();
        g.commit("p.md", &text, 100 + i as i64);
        blobs.push(Some(g.repo.blob(text.as_bytes()).unwrap()));
    }
    let start = Instant::now();
    PEAK_DOCS.with(|peak| peak.set(0));
    let mut history = History::new(&g.repo, false, blobs, vec![false; 21]);
    let mut paths = Vec::new();
    each(&history.snapshot_doc(20).unwrap().roots, &[], &mut |p| {
        paths.push(p)
    });
    assert_eq!(history.last_changed_steps(paths, false).unwrap().len(), 600);
    assert!(history.docs.iter().filter(|d| d.is_some()).count() <= 2);
    assert!(
        PEAK_DOCS.with(|peak| peak.get()) <= 2,
        "retain only adjacent page versions while walking"
    );
    assert!(history.steps.iter().all(|s| s.is_none()));
    let elapsed = start.elapsed();
    eprintln!("block dates: 600 blocks / 20 touching commits {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "page attribution exceeded 2s debug budget: {elapsed:?}"
    );
    // The production entrypoint answers the same committed dates.
    assert!(facts(&g.read("pages/p.md", &lines.concat()))
        .iter()
        .all(|f| matches!(f, Changed::Committed(_))));
}

// GH #980: a graph the user never put under git raised "Could not read this
// block's git history" on bullet hover. Every state where git has nothing to
// say about the page (no repository, an unusable one, an untracked file, an
// unborn HEAD) answers "no dates" silently (threat: malformed input, external
// editors and sync leave such repositories behind; none of it is the user's
// failure). Only a failure inside a usable repository is a command error.
fn no_dates(root: &Path, path: &str, text: &str) {
    let rows = Cache::default()
        .read_page_dates(root, path, text)
        .unwrap_or_else(|e| panic!("hover must stay silent, got error: {e}"));
    assert!(rows.is_empty());
}

/// An untracked page in a usable repository answers its blocks as "not yet
/// committed", never as an error.
fn uncommitted(root: &Path, path: &str, text: &str) {
    let rows = Cache::default()
        .read_page_dates(root, path, text)
        .unwrap_or_else(|e| panic!("hover must stay silent, got error: {e}"));
    assert!(!rows.is_empty() && rows.iter().all(|r| r.changed == Changed::Uncommitted));
}

#[test]
fn no_repository_has_no_dates() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    std::fs::write(root.path().join("pages/p.md"), "- a\n").unwrap();
    no_dates(root.path(), "pages/p.md", "- a\n");
}

#[test]
fn untracked_page_and_unborn_head_have_no_dates() {
    let g = Graph::new();
    uncommitted(g.root.path(), "pages/new.md", "- a\n"); // unborn HEAD
    g.commit("p.md", "- a\n", 100);
    uncommitted(g.root.path(), "pages/untracked.md", "- b\n"); // untracked
}

#[test]
fn non_ascii_folder_is_read_like_any_other() {
    let g = Graph::new();
    let tree = |name: &str, g: &Graph| {
        let blob = g.repo.blob(b"- \xe7\xac\x94\xe8\xae\xb0\n").unwrap();
        let mut dir = g.repo.treebuilder(None).unwrap();
        dir.insert(name, blob, 0o100644).unwrap();
        dir.write().unwrap()
    };
    let inner = tree("页.md", &g);
    let mut pages = g.repo.treebuilder(None).unwrap();
    pages.insert("笔记集", inner, 0o040000).unwrap();
    let mut top = g.repo.treebuilder(None).unwrap();
    top.insert("pages", pages.write().unwrap(), 0o040000)
        .unwrap();
    let tree = g.repo.find_tree(top.write().unwrap()).unwrap();
    let sig = git2::Signature::new("W", "w@example.invalid", &git2::Time::new(100, 0)).unwrap();
    g.repo
        .commit(Some("HEAD"), &sig, &sig, "x", &tree, &[])
        .unwrap();
    let mut cache = Cache::default();
    let rows = cache
        .read_page_dates(g.root.path(), "pages/笔记集/页.md", "- 笔记\n")
        .unwrap();
    assert_eq!(facts(&rows), [Changed::Committed(100)]);
    uncommitted(g.root.path(), "pages/笔记集/新.md", "- 笔记\n");
}

#[test]
fn unusable_repository_has_no_dates() {
    // A `.git` file naming a git directory that is gone (a graph copied or
    // synced out of its repository, or a repository on another device's path).
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".git"), "gitdir: ../no-such-git-dir\n").unwrap();
    no_dates(root.path(), "pages/p.md", "- a\n");
    // A repository whose work tree does not contain the graph (core.worktree
    // pointing elsewhere, as after a synced move).
    let g = Graph::new();
    g.commit("p.md", "- a\n", 100);
    let elsewhere = tempfile::tempdir().unwrap();
    g.repo
        .config()
        .unwrap()
        .set_str("core.worktree", elsewhere.path().to_str().unwrap())
        .unwrap();
    no_dates(g.root.path(), "pages/p.md", "- a\n");
    // A bare repository.
    let bare = tempfile::tempdir().unwrap();
    Repository::init_bare(bare.path()).unwrap();
    no_dates(bare.path(), "pages/p.md", "- a\n");
}

#[test]
fn damaged_history_inside_a_usable_repository_is_still_an_error() {
    // The contract keeps I/O and corruption failures as command errors.
    let mut g = Graph::new();
    let commit = g.commit("p.md", "- a\n", 100);
    let tree = g.repo.find_commit(commit).unwrap().tree_id().to_string();
    let object = g
        .root
        .path()
        .join(".git/objects")
        .join(&tree[..2])
        .join(&tree[2..]);
    std::fs::remove_file(object).ok();
    assert!(g
        .cache
        .read_page_dates(g.root.path(), "pages/p.md", "- a\n")
        .is_err());
}
