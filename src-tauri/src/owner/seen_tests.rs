use super::*;
use std::fs;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    repo: Repository,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        let data = temp.path().join("app-data");
        fs::create_dir_all(root.join("pages")).unwrap();
        let repo = Repository::init(&root).unwrap();
        repo.config().unwrap().set_str("user.name", "Me").unwrap();
        repo.config()
            .unwrap()
            .set_str("user.email", "me@example.test")
            .unwrap();
        Self {
            _temp: temp,
            root,
            data,
            repo,
        }
    }
    fn commit(&self, path: &str, text: Option<&str>, who: &str) -> Oid {
        let mut index = self.repo.index().unwrap();
        if let Some(text) = text {
            fs::write(self.root.join(path), text).unwrap();
            index.add_path(Path::new(path)).unwrap();
        } else {
            fs::remove_file(self.root.join(path)).unwrap();
            index.remove_path(Path::new(path)).unwrap();
        }
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = self.repo.find_tree(tree_id).unwrap();
        let sig =
            git2::Signature::now(who, &format!("{}@example.test", who.to_lowercase())).unwrap();
        let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents = parent.iter().collect::<Vec<_>>();
        self.repo
            .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &parents)
            .unwrap()
    }
    fn commit_as(&self, path: &str, text: &str, sig: &git2::Signature<'_>) -> Oid {
        let mut index = self.repo.index().unwrap();
        fs::write(self.root.join(path), text).unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
        let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = self.repo.head().unwrap().peel_to_commit().unwrap();
        self.repo
            .commit(Some("HEAD"), sig, sig, "fixture", &tree, &[&parent])
            .unwrap()
    }
    /// One commit moving `from` to `to` (a rename, as Tine or git mv makes).
    fn rename(&self, from: &str, to: &str, who: &str) -> Oid {
        let mut index = self.repo.index().unwrap();
        fs::rename(self.root.join(from), self.root.join(to)).unwrap();
        index.remove_path(Path::new(from)).unwrap();
        index.add_path(Path::new(to)).unwrap();
        index.write().unwrap();
        let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig =
            git2::Signature::now(who, &format!("{}@example.test", who.to_lowercase())).unwrap();
        let parent = self.repo.head().unwrap().peel_to_commit().unwrap();
        self.repo
            .commit(Some("HEAD"), &sig, &sig, "rename", &tree, &[&parent])
            .unwrap()
    }
    fn start(&self) {
        initialize(&self.data, &self.root).unwrap();
    }
    fn read(&self, text: &str) -> SeenChanges {
        changes(&self.data, &self.root, "p", "pages/p.md", text, "").unwrap()
    }
}

const BASE: &str = "- anchor\n- old content\n  id:: stable\n- removed content\n";
const OTHER: &str = "- anchor\n- new content\n  id:: stable\n";

#[test]
fn seen_attribution_removal_and_current_text_mapping() {
    let f = Fixture::new();
    f.commit("pages/p.md", Some(BASE), "Me");
    f.start();
    f.commit("pages/p.md", Some(OTHER), "Agent");
    let result = f.read(OTHER);
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.blocks[0].block_path, vec![1]);
    assert_eq!(result.blocks[0].persisted_id.as_deref(), Some("stable"));
    assert_eq!(result.removed.len(), 1);
    assert_eq!(result.removed[0].text, "removed content");
    assert_eq!(result.removed[0].previous_sibling_path, Some(vec![1]));
    let local = "- locally inserted\n- anchor\n- new content\n  id:: stable\n";
    assert_eq!(f.read(local).blocks[0].block_path, vec![2]);
    let local = "- anchor\n- my unsaved replacement\n  id:: stable\n";
    assert!(f.read(local).blocks.is_empty(), "local edits are mine");
    f.commit("pages/p.md", Some(local), "Me");
    assert!(
        f.read(local).blocks.is_empty(),
        "latest touching author wins"
    );
    assert_eq!(
        f.read(local).removed.len(),
        1,
        "an unrelated own edit preserves another author's removal"
    );
    mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
    assert!(f.read(local).removed.is_empty());
}

#[test]
fn seen_first_open_restart_mark_graph_forget_and_noop_writes() {
    let f = Fixture::new();
    f.commit("pages/p.md", Some(BASE), "Me");
    f.start();
    let path = record_path(&f.data, &f.root);
    let original = fs::read(&path).unwrap();
    let stamp = fs::metadata(&path).unwrap().modified().unwrap();
    mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    f.commit("pages/p.md", Some(OTHER), "Agent");
    f.start(); // restart must preserve first open, including on never-visited pages
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!f.read(OTHER).blocks.is_empty());
    mark(&f.data, &f.root, None).unwrap();
    assert!(f.read(OTHER).blocks.is_empty());
    let marked = fs::metadata(&path).unwrap().modified().unwrap();
    mark(&f.data, &f.root, None).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), marked);
    forget(&f.data, &f.root).unwrap();
    forget(&f.data, &f.root).unwrap();
    assert!(!path.exists());
    assert!(f.read(OTHER).blocks.is_empty());
    mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
    assert!(path.exists());
}

#[test]
fn seen_damaged_unknown_record_absent_until_mark() {
    let f = Fixture::new();
    f.commit("pages/p.md", Some(BASE), "Me");
    f.start();
    let path = record_path(&f.data, &f.root);
    for bytes in [
        b"torn".as_slice(),
        br#"{"version":99}"#,
        br#"{"version":1,"first_open_commit":"broken","first_open_at":1,"pages":{}}"#,
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(f.read(BASE).blocks.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        mark(&f.data, &f.root, None).unwrap();
        assert!(load(&path).is_some());
    }
}

#[test]
fn seen_unborn_and_non_repository_and_renamed_path() {
    let f = Fixture::new();
    f.start();
    assert!(f.read("").blocks.is_empty());
    f.commit("pages/p.md", Some("- newly committed\n"), "Agent");
    assert_eq!(f.read("- newly committed\n").blocks.len(), 1);
    mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
    f.commit("pages/q.md", Some("- newly committed\n"), "Agent");
    f.commit("pages/p.md", None, "Agent");
    assert!(changes(
        &f.data,
        &f.root,
        "p",
        "pages/q.md",
        "- newly committed\n",
        ""
    )
    .unwrap()
    .blocks
    .is_empty());
    mark(&f.data, &f.root, Some(("p", "pages/q.md"))).unwrap();
    assert_eq!(
        load(&record_path(&f.data, &f.root)).unwrap().pages["p"].path,
        "pages/q.md"
    );
    let no_repo = tempfile::tempdir().unwrap();
    let result = changes(&f.data, no_repo.path(), "p", "pages/p.md", "", "").unwrap();
    assert!(!result.available);
    assert!(result.reason.unwrap().contains("not in a git repository"));
    assert!(changes(&f.data, &f.root, "p", "../escape.md", "", "").is_err());
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn seen_read_is_git_byte_identical_and_ignores_both_write_locks() {
    let f = Fixture::new();
    f.commit("pages/p.md", Some(BASE), "Me");
    f.start();
    f.commit("pages/p.md", Some(OTHER), "Agent");
    fs::write(f.repo.path().join("index.lock"), b"someone else's lock").unwrap();
    let before = snapshot(f.repo.path());
    let record = fs::read(record_path(&f.data, &f.root)).unwrap();
    let _run = super::super::git_run::RUN_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    assert_eq!(f.read(OTHER).blocks.len(), 1);
    assert_eq!(snapshot(f.repo.path()), before);
    assert_eq!(fs::read(record_path(&f.data, &f.root)).unwrap(), record);
}

#[test]
fn seen_two_hundred_touching_commits_then_unattributed_fallback() {
    let f = Fixture::new();
    f.commit("pages/p.md", Some("- start\n  id:: stable\n"), "Me");
    f.start();
    for i in 0..200 {
        f.commit(
            "pages/p.md",
            Some(&format!("- revision {i}\n  id:: stable\n")),
            "Me",
        );
    }
    let text = "- revision 199\n  id:: stable\n";
    let result = f.read(text);
    assert!(!result.unattributed);
    assert!(result.blocks.is_empty());
    f.commit("pages/p.md", Some("- revision 200\n  id:: stable\n"), "Me");
    let result = f.read("- revision 200\n  id:: stable\n");
    assert!(result.unattributed);
    assert_eq!(result.blocks.len(), 1);
}

#[test]
fn seen_normalization_children_and_identity_fallback() {
    let f = Fixture::new();
    let base = "- parent\n\t- child content\n\t  id:: kid\n- tail\n";
    f.commit("pages/p.md", Some(base), "Me");
    f.start();
    let changed = "- parent  \n  collapsed:: true\n\t- child changed\n\t  id:: kid\n- tail\n";
    f.commit("pages/p.md", Some(changed), "Agent");
    assert_eq!(
        f.read(changed)
            .blocks
            .iter()
            .map(|b| b.block_path.clone())
            .collect::<Vec<_>>(),
        vec![vec![0, 0]]
    );
    // "Me" is exactly the autocommit identity (ADR 0078): the repository's
    // name with the placeholder email when no email is configured, so an
    // author merely named like "Your name" is someone else.
    f.repo.config().unwrap().set_str("user.email", "").unwrap();
    let result = changes(&f.data, &f.root, "p", "pages/p.md", changed, "Agent").unwrap();
    assert!(result.identity_known);
    assert_eq!(result.blocks.len(), 1, "repo user.name wins over Your name");
    f.repo.config().unwrap().set_str("user.name", "").unwrap();
    let result = changes(&f.data, &f.root, "p", "pages/p.md", changed, "Agent").unwrap();
    assert!(result.identity_known);
    assert_eq!(
        result.blocks.len(),
        1,
        "Agent <agent@example.test> is not Agent <user@tine.invalid>"
    );
    let result = f.read(changed);
    assert!(!result.identity_known);
    assert_eq!(result.blocks.len(), 1);
}

#[test]
fn seen_me_is_exactly_the_identity_tine_commits_as() {
    // Each row: repo name, repo email, Your name. The writer's own commit
    // never counts; another unconfigured Tine (same placeholder email,
    // another name) does, unless both share the fixed fallback.
    for (name, email, yours) in [
        ("Me", "me@example.test", ""),
        ("", "", "Ada"),
        ("", "me@example.test", "Ada"),
        ("Me", "", "Ada"),
        ("", "", ""),
    ] {
        let f = Fixture::new();
        f.repo.config().unwrap().set_str("user.name", name).unwrap();
        f.repo
            .config()
            .unwrap()
            .set_str("user.email", email)
            .unwrap();
        f.commit("pages/p.md", Some(BASE), "Me");
        f.start();
        let writer = super::super::git_run::user_identity(&f.repo, yours);
        f.commit_as("pages/p.md", OTHER, &writer.signature());
        let read = changes(&f.data, &f.root, "p", "pages/p.md", OTHER, yours).unwrap();
        assert!(
            read.blocks.is_empty(),
            "{name:?}/{email:?}/{yours:?}: own commit"
        );
        assert!(read.removed.is_empty());
        assert_eq!(
            read.identity_known,
            !(name.is_empty() && email.is_empty() && yours.is_empty())
        );
        let stranger = git2::Signature::now("Bob", "user@tine.invalid").unwrap();
        let next = "- anchor\n- bob's content\n  id:: stable\n";
        f.commit_as("pages/p.md", next, &stranger);
        let read = changes(&f.data, &f.root, "p", "pages/p.md", next, yours).unwrap();
        assert_eq!(
            read.blocks.len(),
            1,
            "{name:?}/{email:?}/{yours:?}: Bob counts"
        );
    }
}

#[test]
fn seen_rename_commit_attribution_decides_new_path() {
    // A page with no record inherits the graph baseline; whoever committed
    // the rename (the creation at the new path) owns its blocks.
    for (who, shown) in [("Me", 0), ("Agent", 2)] {
        let f = Fixture::new();
        f.commit("pages/p.md", Some("- one\n- two\n"), "Me");
        f.start();
        mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
        f.rename("pages/p.md", "pages/q.md", who);
        let read = changes(&f.data, &f.root, "q", "pages/q.md", "- one\n- two\n", "").unwrap();
        assert_eq!(read.blocks.len(), shown, "renamed by {who}");
        assert!(read.removed.is_empty());
    }
}

#[test]
fn seen_forget_page_drops_only_that_page_and_writes_nothing_otherwise() {
    let f = Fixture::new();
    f.commit("pages/p.md", Some(BASE), "Me");
    f.start();
    f.commit("pages/p.md", Some(OTHER), "Agent");
    mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
    mark(&f.data, &f.root, Some(("q", "pages/q.md"))).unwrap();
    assert!(f.read(OTHER).blocks.is_empty());
    let path = record_path(&f.data, &f.root);
    forget_page(&f.data, &f.root, Some("p"), None).unwrap();
    let record = load(&path).unwrap();
    assert!(!record.pages.contains_key("p"));
    assert!(record.pages.contains_key("q"));
    assert_eq!(f.read(OTHER).blocks.len(), 1, "back on the graph baseline");
    let bytes = fs::read(&path).unwrap();
    let stamp = fs::metadata(&path).unwrap().modified().unwrap();
    forget_page(&f.data, &f.root, Some("p"), Some("pages/p.md")).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    forget_page(&f.data, &f.root, None, Some("pages/q.md")).unwrap();
    assert!(load(&path).unwrap().pages.is_empty());
    forget(&f.data, &f.root).unwrap();
    forget_page(&f.data, &f.root, Some("p"), None).unwrap();
    assert!(!path.exists(), "no record is not created");
    assert!(forget_page(&f.root.join("app-data"), &f.root, Some("p"), None).is_err());
}

#[test]
fn seen_org_properties_and_literal_fold_text() {
    let f = Fixture::new();
    let base = "* heading\n:PROPERTIES:\n:id: stable\n:END:\n";
    f.commit("pages/p.org", Some(base), "Me");
    f.start();
    let fold = "* heading\n:PROPERTIES:\n:id: stable\n:collapsed: true\n:END:\n";
    f.commit("pages/p.org", Some(fold), "Agent");
    let read = |text: &str| changes(&f.data, &f.root, "org", "pages/p.org", text, "").unwrap();
    assert!(read(fold).blocks.is_empty());
    let changed = "* heading\n:PROPERTIES:\n:id: stable\n:status: done\n:END:\n";
    f.commit("pages/p.org", Some(changed), "Agent");
    assert_eq!(read(changed).blocks.len(), 1);
    let literal_before = "- ```\n  collapsed:: false\n  ```\n  id:: literal\n";
    f.commit("pages/p.md", Some(literal_before), "Me");
    mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
    let literal_after = "- ```\n  collapsed:: true\n  ```\n  id:: literal\n";
    f.commit("pages/p.md", Some(literal_after), "Agent");
    assert_eq!(
        f.read(literal_after).blocks.len(),
        1,
        "literal collapsed text counts"
    );
}

#[test]
fn seen_nested_graph_detached_head_and_app_data_containment() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("Notes [work]/pages")).unwrap();
    let nested = f.root.join("Notes [work]");
    f.commit("Notes [work]/pages/p.md", Some(BASE), "Me");
    initialize(&f.data, &nested).unwrap();
    let tip = f.commit("Notes [work]/pages/p.md", Some(OTHER), "Agent");
    f.repo.set_head_detached(tip).unwrap();
    let result = changes(&f.data, &nested, "p", "pages/p.md", OTHER, "").unwrap();
    assert_eq!(result.blocks.len(), 1);
    assert!(initialize(&nested.join("app-data"), &nested).is_err());
    assert!(mark(&nested.join("app-data"), &nested, None).is_err());
    assert!(forget(&nested.join("app-data"), &nested).is_err());
    assert!(!nested.join("app-data").exists());
}

#[test]
fn seen_removed_text_is_the_last_body_before_deletion_and_own_removals_do_not_count() {
    let f = Fixture::new();
    let base = "- anchor\n- original\n  id:: removed\n";
    f.commit("pages/p.md", Some(base), "Me");
    f.start();
    f.commit(
        "pages/p.md",
        Some("- anchor\n- edited before removal\n  id:: removed\n"),
        "Agent",
    );
    f.commit("pages/p.md", Some("- anchor\n"), "Agent");
    assert_eq!(
        f.read("- anchor\n").removed[0].text,
        "edited before removal\nid:: removed"
    );
    f.commit("pages/p.md", Some(base), "Me");
    f.commit("pages/p.md", Some("- anchor\n"), "Me");
    assert!(
        f.read("- anchor\n").removed.is_empty(),
        "restored then deleted by me"
    );
    mark(&f.data, &f.root, None).unwrap();
    f.commit("pages/p.md", Some(base), "Agent");
    mark(&f.data, &f.root, None).unwrap();
    f.commit("pages/p.md", Some("- anchor\n"), "Me");
    assert!(f.read("- anchor\n").removed.is_empty());
}

#[test]
fn seen_unit_cost_per_tracked_page() {
    let mut record = fresh(Some(Oid::from_str(&"a".repeat(40)).unwrap()));
    let empty = serde_json::to_vec(&record).unwrap().len();
    for i in 0..60 {
        record.pages.insert(
            format!("page-{i:03}"),
            PageRecord {
                path: format!("pages/page-{i:03}.md"),
                seen_commit: Some("b".repeat(40)),
                seen_at: now(),
            },
        );
    }
    let sixty = serde_json::to_vec(&record).unwrap().len();
    record.pages.retain(|key, _| key == "page-000");
    let one = serde_json::to_vec(&record).unwrap().len();
    eprintln!("seen unit cost: graph envelope {empty} B; one tracked page {one} B total; 60 tracked pages {sixty} B total; {} B per additional page including separator; 0 bytes/files per edit; 0 transport", (sixty-one)/59);
    assert_eq!((sixty - one) % 59, 0);
}

#[test]
fn seen_per_open_cost_600_blocks_20_commits() {
    let f = Fixture::new();
    let text = |revision: usize| {
        (0..600)
            .map(|i| {
                format!(
                    "- block {i} revision {}\n  id:: block-{i}\n",
                    if i == 300 { revision } else { 0 }
                )
            })
            .collect::<String>()
    };
    f.commit("pages/p.md", Some(&text(0)), "Me");
    f.start();
    for i in 1..=20 {
        f.commit("pages/p.md", Some(&text(i)), "Agent");
    }
    let current = text(20);
    let mut samples = Vec::new();
    for _ in 0..11 {
        let started = std::time::Instant::now();
        let result = f.read(&current);
        samples.push(started.elapsed());
        assert_eq!(result.blocks.len(), 1);
    }
    samples.sort();
    mark(&f.data, &f.root, Some(("p", "pages/p.md"))).unwrap();
    let size = fs::metadata(record_path(&f.data, &f.root)).unwrap().len();
    eprintln!("seen 600 blocks / 20 commits: median {:?}, min {:?}; one-page record {size} B; 0 bytes/edit", samples[5], samples[0]);
}

/// The round-1 algorithm, kept as the oracle for the faster one: parse every
/// snapshot, align every consecutive pair over whole pages (no trim), and
/// follow owners and removals forward. `snapshots[0]` is the baseline,
/// `other[i]` says step `i` was someone else's.
fn reference(
    snapshots: &[String],
    other: &[bool],
    current_text: &str,
) -> (
    Vec<(BlockPath, Option<String>)>,
    Vec<(BlockPath, String, Option<BlockPath>, Option<BlockPath>)>,
) {
    fn full_align(old: &[DocBlock], new: &[DocBlock]) -> Alignment {
        fn walk(
            rows: &[DiffRow],
            old: &[DocBlock],
            new: &[DocBlock],
            op: &[usize],
            np: &[usize],
            out: &mut Alignment,
        ) {
            let (mut oi, mut ni) = (0, 0);
            for row in rows {
                let mut a = op.to_vec();
                a.push(oi);
                let mut b = np.to_vec();
                b.push(ni);
                match (&row.theirs, &row.mine) {
                    (Some(_), Some(_)) => {
                        if old[oi].raw() != new[ni].raw() {
                            out.changed.insert(b.clone());
                        }
                        if row.children.is_empty() && old[oi] == new[ni] {
                            equal(&old[oi], a, b, out);
                        } else {
                            out.pairs.insert(a.clone(), b.clone());
                            walk(
                                &row.children,
                                &old[oi].children,
                                &new[ni].children,
                                &a,
                                &b,
                                out,
                            );
                        }
                        oi += 1;
                        ni += 1;
                    }
                    (Some(_), None) => {
                        out.removed.insert(a.clone());
                        each(&old[oi].children, &a, &mut |p| {
                            out.removed.insert(p);
                        });
                        oi += 1;
                    }
                    (None, Some(_)) => {
                        out.changed.insert(b.clone());
                        each(&new[ni].children, &b, &mut |p| {
                            out.changed.insert(p);
                        });
                        ni += 1;
                    }
                    _ => {}
                }
            }
        }
        let mut out = Alignment::default();
        walk(&diff_blocks(new, old), old, new, &[], &[], &mut out);
        out
    }
    let doc = |text: &str| {
        let mut d = parse(text, false);
        normalize(&mut d.roots, false);
        d
    };
    let old = doc(&snapshots[0]);
    let latest = doc(snapshots.last().unwrap());
    let net = full_align(&old.roots, &latest.roots);
    let mut owners = HashMap::<BlockPath, bool>::new();
    let mut origins = HashMap::<BlockPath, BlockPath>::new();
    let mut deletions = HashMap::<BlockPath, (bool, String)>::new();
    each(&old.roots, &[], &mut |p| {
        origins.insert(p.clone(), p);
    });
    let mut previous = doc(&snapshots[0]);
    for i in 1..snapshots.len() {
        let next = doc(&snapshots[i]);
        let step = full_align(&previous.roots, &next.roots);
        for deleted in &step.removed {
            if let Some(origin) = origins.get(deleted) {
                deletions.insert(
                    origin.clone(),
                    (other[i], block_at(&previous.roots, deleted).raw().into()),
                );
            }
        }
        owners = step
            .pairs
            .iter()
            .filter_map(|(a, b)| owners.get(a).map(|v| (b.clone(), *v)))
            .collect();
        origins = step
            .pairs
            .iter()
            .filter_map(|(a, b)| origins.get(a).map(|v| (b.clone(), v.clone())))
            .collect();
        if step.changed.iter().any(|p| !origins.contains_key(p)) {
            let mut used: HashSet<_> = origins.values().cloned().collect();
            for (base_path, next_path) in full_align(&old.roots, &next.roots).pairs {
                if !origins.contains_key(&next_path) && used.insert(base_path.clone()) {
                    origins.insert(next_path, base_path);
                }
            }
        }
        for changed in step.changed {
            owners.insert(changed, other[i]);
        }
        previous = next;
    }
    let current = doc(current_text);
    let working = full_align(&latest.roots, &current.roots);
    let mut blocks = Vec::new();
    for path in &net.changed {
        if !owners.get(path).copied().unwrap_or(false) {
            continue;
        }
        if let Some(mapped) = working
            .pairs
            .get(path)
            .filter(|p| !working.changed.contains(*p))
        {
            blocks.push((
                mapped.clone(),
                block_at(&current.roots, mapped).property("id"),
            ));
        }
    }
    let mapped = |p: &BlockPath| net.pairs.get(p).and_then(|p| working.pairs.get(p)).cloned();
    let mut removed = Vec::new();
    for path in &net.removed {
        if !deletions.get(path).is_some_and(|(other, _)| *other) {
            continue;
        }
        let mut parent = path[..path.len() - 1].to_vec();
        let mut slot = *path.last().unwrap();
        while !parent.is_empty() && mapped(&parent).is_none() {
            slot = parent.pop().unwrap();
        }
        let previous_sibling = (0..slot).rev().find_map(|i| {
            let mut p = parent.clone();
            p.push(i);
            mapped(&p)
        });
        let parent_path = if parent.is_empty() {
            None
        } else {
            mapped(&parent)
        };
        removed.push((
            path.clone(),
            deletions[path].1.clone(),
            parent_path,
            previous_sibling,
        ));
    }
    blocks.sort();
    removed.sort();
    (blocks, removed)
}

/// A small deterministic generator (no extra dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// A page as (depth, body) rows; children follow their parent at depth + 1.
fn render(rows: &[(usize, String)]) -> String {
    rows.iter()
        .map(|(depth, body)| {
            let indent = "\t".repeat(*depth);
            let mut lines = body.lines();
            let mut out = format!("{indent}- {}\n", lines.next().unwrap_or(""));
            for line in lines {
                out.push_str(&format!("{indent}  {line}\n"));
            }
            out
        })
        .collect()
}

/// Read the page through the fast path (with an unsaved local edit one
/// time in three) and compare it with [`reference`]: (marks, removals).
fn assert_matches_reference(
    f: &Fixture,
    snapshots: &[String],
    other: &[bool],
    rng: &mut Rng,
    case: usize,
) -> (usize, usize) {
    let tip = snapshots.last().unwrap().clone();
    let local = if rng.next(3) == 0 {
        format!("- local first\n{tip}")
    } else {
        tip.clone()
    };
    let fast = f.read(&local);
    let blocks: Vec<_> = fast
        .blocks
        .iter()
        .map(|b| (b.block_path.clone(), b.persisted_id.clone()))
        .collect();
    let removed: Vec<_> = fast
        .removed
        .iter()
        .map(|r| {
            (
                r.baseline_block_path.clone(),
                r.text.clone(),
                r.parent_path.clone(),
                r.previous_sibling_path.clone(),
            )
        })
        .collect();
    let (want_blocks, want_removed) = reference(snapshots, other, &local);
    assert_eq!(blocks, want_blocks, "case {case}: blocks\n{snapshots:#?}");
    assert_eq!(
        removed, want_removed,
        "case {case}: removed\n{snapshots:#?}"
    );
    (blocks.len(), removed.len())
}

#[test]
fn seen_fast_attribution_equals_the_round_one_algorithm() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut fresh = 0usize;
    let mut compared = 0usize;
    let mut removals = 0usize;
    for case in 0..120 {
        let f = Fixture::new();
        let mut rows: Vec<(usize, String)> = Vec::new();
        // Every thirtieth page is large enough for sync_diff's patience branch.
        let size = if case % 30 == 0 {
            520 + rng.next(40)
        } else {
            5 + rng.next(30)
        };
        for _ in 0..size {
            fresh += 1;
            let depth = if rows.is_empty() {
                0
            } else {
                rng.next(rows.last().unwrap().0 + 2).min(2)
            };
            let body = match rng.next(4) {
                0 => format!("block {fresh}"),
                1 => format!("block {fresh} text\nid:: id-{fresh}"),
                2 => format!("TODO task {fresh}"),
                _ => format!("note {fresh}\nstatus:: open"),
            };
            rows.push((depth, body));
        }
        let mut snapshots = vec![render(&rows)];
        let mut other = vec![false];
        f.commit("pages/p.md", Some(&snapshots[0]), "Me");
        f.start();
        for _ in 0..(1 + rng.next(8)) {
            let edits = 1 + rng.next(3);
            for _ in 0..edits {
                if rows.is_empty() {
                    rows.push((0, "seed".into()));
                }
                let at = rng.next(rows.len());
                match rng.next(6) {
                    0 | 1 => {
                        rows[at].1 = format!(
                            "{} edited {}",
                            rows[at].1.lines().next().unwrap(),
                            rng.next(1000)
                        ) + &rows[at]
                            .1
                            .lines()
                            .skip(1)
                            .map(|l| format!("\n{l}"))
                            .collect::<String>()
                    }
                    2 => {
                        fresh += 1;
                        let depth = rows[at].0;
                        rows.insert(at + 1, (depth, format!("added {fresh}")));
                    }
                    3 => {
                        // Remove a block and its children.
                        let depth = rows[at].0;
                        let end = (at + 1..rows.len())
                            .find(|&i| rows[i].0 <= depth)
                            .unwrap_or(rows.len());
                        rows.drain(at..end);
                    }
                    4 => rows[at].1.push_str("\ncollapsed:: true"),
                    _ => {
                        if rows.len() > 1
                            && at > 0
                            && rows[at].0 > 0
                            && rows[at].0 == rows[at - 1].0 + 1
                        {
                            rows[at].0 -= 1; // outdent a first child
                        }
                    }
                }
            }
            let text = render(&rows);
            if &text == snapshots.last().unwrap() {
                continue;
            }
            let by_other = rng.next(2) == 0;
            f.commit(
                "pages/p.md",
                Some(&text),
                if by_other { "Agent" } else { "Me" },
            );
            snapshots.push(text);
            other.push(by_other);
        }
        let (marks, gone) = assert_matches_reference(&f, &snapshots, &other, &mut rng, case);
        compared += marks;
        removals += gone;
    }
    assert!(
        compared > 40 && removals > 10,
        "marks {compared}, removals {removals}"
    );
}

/// Where the subtree rooted at `at` ends (exclusive).
fn subtree_end(rows: &[(usize, String)], at: usize) -> usize {
    (at + 1..rows.len())
        .find(|&i| rows[i].0 <= rows[at].0)
        .unwrap_or(rows.len())
}

/// The shapes the end-trimming could get wrong: sibling reorders, moves
/// across parents, duplicate bodies, duplicate and missing ids. Each history
/// is compared with the untrimmed round-1 algorithm.
#[test]
fn seen_fast_attribution_equals_round_one_under_reorders_moves_and_duplicate_ids() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut fresh = 0usize;
    let (mut compared, mut removals, mut cases) = (0usize, 0usize, 0usize);
    let (mut reorders, mut moves, mut duplicates, mut stripped) = (0usize, 0, 0, 0);
    for case in 0..400 {
        let f = Fixture::new();
        let mut rows: Vec<(usize, String)> = Vec::new();
        // Every fiftieth page is large enough for sync_diff's patience branch.
        let size = if case % 50 == 0 {
            520 + rng.next(40)
        } else {
            4 + rng.next(30)
        };
        for _ in 0..size {
            fresh += 1;
            let depth = if rows.is_empty() {
                0
            } else {
                rng.next(rows.last().unwrap().0 + 2).min(2)
            };
            // A small shared vocabulary makes duplicate bodies and ids common.
            let body = match rng.next(6) {
                0 => format!("block {fresh}"),
                1 => format!("block {fresh} text\nid:: id-{fresh}"),
                2 => format!("TODO same {}", rng.next(4)),
                3 => format!("dup {}\nid:: id-shared-{}", rng.next(3), rng.next(3)),
                4 => "same".to_owned(),
                _ => format!("note {fresh}\nstatus:: open"),
            };
            rows.push((depth, body));
        }
        let mut snapshots = vec![render(&rows)];
        let mut other = vec![false];
        f.commit("pages/p.md", Some(&snapshots[0]), "Me");
        f.start();
        for _ in 0..(1 + rng.next(8)) {
            for _ in 0..(1 + rng.next(3)) {
                if rows.is_empty() {
                    rows.push((0, "seed".into()));
                }
                let at = rng.next(rows.len());
                match rng.next(9) {
                    0 => {
                        let first = rows[at].1.lines().next().unwrap().to_owned();
                        let rest: String = rows[at]
                            .1
                            .lines()
                            .skip(1)
                            .map(|l| format!("\n{l}"))
                            .collect();
                        rows[at].1 = format!("{first} edited {}{rest}", rng.next(1000));
                    }
                    1 => {
                        // Swap the subtree at `at` with its next sibling's.
                        let end = subtree_end(&rows, at);
                        if end < rows.len() && rows[end].0 == rows[at].0 {
                            let next_end = subtree_end(&rows, end);
                            let second: Vec<_> = rows.drain(end..next_end).collect();
                            for (k, row) in second.into_iter().enumerate() {
                                rows.insert(at + k, row);
                            }
                            reorders += 1;
                        }
                    }
                    2 => {
                        // Move a subtree under another parent (or to the top).
                        let end = subtree_end(&rows, at);
                        let mut moved: Vec<_> = rows.drain(at..end).collect();
                        let root = moved[0].0;
                        let (pos, depth) = if rows.is_empty() || rng.next(4) == 0 {
                            (0, 0)
                        } else {
                            let j = rng.next(rows.len());
                            (subtree_end(&rows, j), rows[j].0 + rng.next(2))
                        };
                        for row in &mut moved {
                            row.0 = row.0 - root + depth;
                        }
                        for (k, row) in moved.into_iter().enumerate() {
                            rows.insert(pos + k, row);
                        }
                        moves += 1;
                    }
                    3 => {
                        // Copy another block's body: duplicate content and ids.
                        let from = rng.next(rows.len());
                        rows[at].1 = rows[from].1.clone();
                        duplicates += 1;
                    }
                    4 => {
                        // Drop the id (a missing id), or borrow another block's.
                        if rows[at].1.contains("\nid:: ") {
                            rows[at].1 = rows[at]
                                .1
                                .lines()
                                .filter(|l| !l.starts_with("id:: "))
                                .collect::<Vec<_>>()
                                .join("\n");
                            stripped += 1;
                        } else if let Some(id) = rows
                            .iter()
                            .flat_map(|r| r.1.lines())
                            .find(|l| l.starts_with("id:: "))
                            .map(str::to_owned)
                        {
                            rows[at].1.push_str(&format!("\n{id}"));
                            duplicates += 1;
                        }
                    }
                    5 => {
                        fresh += 1;
                        let body = if rng.next(2) == 0 {
                            "same".to_owned()
                        } else {
                            format!("added {fresh}")
                        };
                        let depth = rows[at].0;
                        rows.insert(subtree_end(&rows, at), (depth, body));
                    }
                    6 => {
                        let end = subtree_end(&rows, at);
                        rows.drain(at..end);
                    }
                    7 => rows[at].1.push_str("\ncollapsed:: true"),
                    _ => {
                        // Swap two adjacent same-depth leaves anywhere.
                        if at + 1 < rows.len() && rows[at].0 == rows[at + 1].0 {
                            rows.swap(at, at + 1);
                            reorders += 1;
                        }
                    }
                }
            }
            let text = render(&rows);
            if &text == snapshots.last().unwrap() {
                continue;
            }
            let by_other = rng.next(2) == 0;
            f.commit(
                "pages/p.md",
                Some(&text),
                if by_other { "Agent" } else { "Me" },
            );
            snapshots.push(text);
            other.push(by_other);
        }
        let (marks, gone) = assert_matches_reference(&f, &snapshots, &other, &mut rng, case);
        compared += marks;
        removals += gone;
        cases += 1;
    }
    eprintln!(
        "seen differential: {cases} histories, {compared} marks, {removals} removals; \
         {reorders} reorders, {moves} moves, {duplicates} duplicates, {stripped} ids dropped"
    );
    assert!(
        compared > 200 && removals > 40 && reorders > 100 && moves > 100 && duplicates > 100,
        "coverage: marks {compared}, removals {removals}, reorders {reorders}, moves {moves}, duplicates {duplicates}"
    );
}

/// The same 600-block / 20-commit page, in the two slower shapes: an unsaved
/// local edit (one more parse and alignment), and a removal by another
/// author (the forward lineage pass parses every touching snapshot).
#[test]
fn seen_per_open_cost_with_local_edit_and_removal() {
    let f = Fixture::new();
    let text = |revision: usize, removed: bool| {
        (0..600)
            .filter(|&i| !(removed && i == 450))
            .map(|i| {
                format!(
                    "- block {i} revision {}\n  id:: block-{i}\n",
                    if i == 300 { revision } else { 0 }
                )
            })
            .collect::<String>()
    };
    f.commit("pages/p.md", Some(&text(0, false)), "Me");
    f.start();
    for i in 1..=20 {
        f.commit("pages/p.md", Some(&text(i, i >= 10)), "Agent");
    }
    let measure = |current: &str, blocks: usize, removed: usize| {
        let mut samples = Vec::new();
        for _ in 0..11 {
            let started = std::time::Instant::now();
            let result = f.read(current);
            samples.push(started.elapsed());
            assert_eq!(
                (result.blocks.len(), result.removed.len()),
                (blocks, removed)
            );
        }
        samples.sort();
        samples[5]
    };
    let head = text(20, true);
    let removal = measure(&head, 1, 1);
    let local = measure(&format!("- unsaved\n{head}"), 1, 1);
    eprintln!("seen 600 blocks / 20 commits with a removal: median {removal:?}; plus an unsaved local edit: median {local:?}");
}
