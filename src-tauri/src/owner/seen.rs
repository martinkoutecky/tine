//! Read-only git comparison for the device-local seen baseline (design rev 2).
//! No index, status refresh, mutation guard or autocommit lock is used here.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use git2::{ErrorCode, Oid, Repository};
use serde::{Deserialize, Serialize};
use tine_core::doc::{DocBlock, Document};
use tine_core::sync_diff::{diff_blocks, diff_blocks_trimmed, DiffRow, PersistedIds};

use crate::seen::{SeenBlock, SeenChanges, SeenRemoved};

const COMMIT_LIMIT: usize = 200;
static RECORD_LOCK: Mutex<()> = Mutex::new(());
type BlockPath = Vec<usize>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageRecord {
    path: String,
    seen_commit: Option<String>,
    seen_at: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    // null is the empty tree captured while HEAD was unborn, not an absent record.
    first_open_commit: Option<String>,
    first_open_at: u64,
    pages: BTreeMap<String, PageRecord>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn record_path(data: &Path, root: &Path) -> PathBuf {
    data.join("seen-git")
        .join(crate::settings::session_id(root))
}

fn outside_graph(data: &Path, root: &Path) -> Result<(), String> {
    // Resolve the existing ancestor too: app data need not have been created
    // yet, and a portable installation may place it next to a graph.
    fn resolved(path: &Path) -> std::io::Result<PathBuf> {
        if path.exists() {
            return path.canonicalize();
        }
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("no app-data ancestor"))?;
        Ok(resolved(parent)?.join(path.file_name().unwrap()))
    }
    let data = resolved(&record_path(data, root)).map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if data.starts_with(root) {
        return Err("Seen state needs an app-data directory outside the graph.".into());
    }
    Ok(())
}

fn load(path: &Path) -> Option<Record> {
    let bytes = crate::device_io::read_bounded(path, 16 * 1024 * 1024)
        .ok()
        .flatten()?;
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    let valid = |oid: &Option<String>| {
        oid.as_ref()
            .is_none_or(|s| s.len() == 40 && Oid::from_str(s).is_ok())
    };
    (record.version == 1
        && valid(&record.first_open_commit)
        && record
            .pages
            .values()
            .all(|p| valid(&p.seen_commit) && relative(&p.path).is_ok()))
    .then_some(record)
}

fn save(path: &Path, record: &Record) -> Result<(), String> {
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("seen record is too large".into());
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    crate::device_io::atomic_write(path, &bytes).map_err(|e| e.to_string())
}

fn relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || !Path::new(path)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
    {
        return Err("seen needs a graph-relative page path".into());
    }
    Ok(())
}

/// Mirror git_run's discovery and canonical worktree-prefix convention, without
/// its write-side checks (detached HEAD and index.lock do not prevent a read).
fn open(root: &Path) -> Result<(Repository, PathBuf), String> {
    let repo = super::git_run::discover(root).map_err(|e| {
        if e.code() == ErrorCode::NotFound {
            "The graph is not in a git repository.".into()
        } else {
            e.to_string()
        }
    })?;
    let work = repo.workdir().ok_or("The graph's repository is bare.")?;
    let work = work.canonicalize().map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let prefix = root
        .strip_prefix(work)
        .map_err(|_| "The graph is outside its repository work tree.")?
        .to_owned();
    Ok((repo, prefix))
}

fn head(repo: &Repository) -> Result<Option<Oid>, String> {
    match repo.head() {
        Ok(r) => r
            .peel_to_commit()
            .map(|c| Some(c.id()))
            .map_err(|e| e.to_string()),
        Err(e) if matches!(e.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn fresh(commit: Option<Oid>) -> Record {
    Record {
        version: 1,
        first_open_commit: commit.map(|o| o.to_string()),
        first_open_at: now(),
        pages: BTreeMap::new(),
    }
}

/// Called at graph bind, before autocommit can move HEAD. Only this setup and
/// explicit mark/forget operations write app data; get_page_changes never does.
pub(crate) fn initialize(data: &Path, root: &Path) -> Result<(), String> {
    outside_graph(data, root)?;
    let (repo, _) = open(root)?;
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = record_path(data, root);
    if load(&path).is_none() {
        save(&path, &fresh(head(&repo)?))?;
    }
    Ok(())
}

pub(crate) fn mark(data: &Path, root: &Path, page: Option<(&str, &str)>) -> Result<(), String> {
    outside_graph(data, root)?;
    if let Some((key, path)) = page {
        if key.is_empty() {
            return Err("seen needs a page key".into());
        }
        relative(path)?;
        // A leave can arrive just after Tine's own rename, merge or delete
        // dropped this page's entry (forget_page): its file is gone then, and
        // an entry made now would hand its stale baseline to a page later
        // created at the same path. Nothing on disk is nothing to mark.
        if std::fs::symlink_metadata(root.join(path)).is_err() {
            return Ok(());
        }
    }
    let (repo, _) = open(root)?;
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = record_path(data, root);
    let tip = head(&repo)?;
    let loaded = load(&path);
    let existed = loaded.is_some();
    let mut record = loaded.unwrap_or_else(|| fresh(tip));
    let oid = tip.map(|o| o.to_string());
    if let Some((key, page_path)) = page {
        if record
            .pages
            .get(key)
            .is_some_and(|p| p.seen_commit == oid && p.path == page_path)
        {
            return Ok(());
        }
        // An inherited baseline already at HEAD needs no per-page entry.
        if !record.pages.contains_key(key) && record.first_open_commit == oid && existed {
            return Ok(());
        }
        record.pages.insert(
            key.into(),
            PageRecord {
                path: page_path.into(),
                seen_commit: oid,
                seen_at: now(),
            },
        );
    } else {
        if record.first_open_commit == oid && record.pages.is_empty() && existed {
            return Ok(());
        }
        record = fresh(tip);
    }
    save(&path, &record)
}

/// Drop one page's override (Tine's own rename, merge or delete, D-20261017-SB1
/// call 6): every entry under `key` or recorded at `path`. The page then
/// inherits the graph baseline again. Nothing to drop writes nothing.
pub(crate) fn forget_page(
    data: &Path,
    root: &Path,
    key: Option<&str>,
    path: Option<&str>,
) -> Result<(), String> {
    outside_graph(data, root)?;
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let file = record_path(data, root);
    let Some(mut record) = load(&file) else {
        return Ok(());
    };
    let before = record.pages.len();
    record
        .pages
        .retain(|k, p| Some(k.as_str()) != key && Some(p.path.as_str()) != path);
    if record.pages.len() == before {
        return Ok(());
    }
    save(&file, &record)
}

pub(crate) fn forget(data: &Path, root: &Path) -> Result<(), String> {
    outside_graph(data, root)?;
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = record_path(data, root);
    match std::fs::remove_file(&path) {
        Ok(()) => tine_store::directory_durability::sync_directory_entry(path.parent().unwrap())
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn blob_id(repo: &Repository, commit: Oid, path: &Path) -> Result<Option<Oid>, String> {
    let tree = repo
        .find_commit(commit)
        .and_then(|c| c.tree())
        .map_err(|e| e.to_string())?;
    match tree.get_path(path) {
        Ok(e) if e.kind() == Some(git2::ObjectType::Blob) => Ok(Some(e.id())),
        Ok(_) => Err("seen page is not a regular git blob".into()),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn parse(text: &str, org: bool) -> Document {
    if org {
        tine_core::org::parse_org(text)
    } else {
        tine_core::doc::parse(text)
    }
}

fn normalize(blocks: &mut [DocBlock], org: bool) {
    for block in blocks {
        // Avoid reparsing every block to remove an absent fold property.
        // Actual removals still use parser-owned regions to protect literals.
        let collapsed = block.raw().to_ascii_lowercase().contains("collapsed");
        let spaces = block.raw().lines().any(|line| line.trim_end() != line);
        if collapsed || spaces {
            let raw = if collapsed {
                tine_core::block_regions::parse(block.raw(), org)
                    .apply(
                        block.raw(),
                        org,
                        tine_core::block_regions::Edit::Property {
                            key: "collapsed".into(),
                            value: None,
                        },
                    )
                    .unwrap_or_else(|_| block.raw().to_owned())
            } else {
                block.raw().to_owned()
            };
            block.set_raw(
                raw.lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        normalize(&mut block.children, org);
    }
}

/// A block alignment of two snapshots. Root blocks before `head` and the
/// last `tail` roots are content-equal on both sides and pair by position
/// (computed, not stored); `pairs`/`back` hold the window between them.
/// Trimmed blocks are never changed or removed.
#[derive(Default)]
struct Alignment {
    head: usize,
    tail: usize,
    old_len: usize,
    new_len: usize,
    pairs: HashMap<BlockPath, BlockPath>, // old -> new, window only
    back: HashMap<BlockPath, BlockPath>,  // new -> old, window only
    changed: HashSet<BlockPath>,
    removed: HashSet<BlockPath>,
}

impl Alignment {
    fn carry(&self, p: &[usize], forward: bool) -> Option<BlockPath> {
        let (from_len, to_len, window) = if forward {
            (self.old_len, self.new_len, &self.pairs)
        } else {
            (self.new_len, self.old_len, &self.back)
        };
        let root = p[0];
        if root < self.head {
            Some(p.to_vec())
        } else if root >= from_len - self.tail {
            let mut q = p.to_vec();
            q[0] = root + to_len - from_len;
            Some(q)
        } else {
            window.get(p).cloned()
        }
    }
    /// The later snapshot's block paired with `old`.
    fn fwd(&self, old: &[usize]) -> Option<BlockPath> {
        self.carry(old, true)
    }
    /// The earlier snapshot's block paired with `new`.
    fn back(&self, new: &[usize]) -> Option<BlockPath> {
        self.carry(new, false)
    }
    /// Every pair, trimmed ends included (`old` is the earlier snapshot).
    fn all_pairs(&self, old: &[DocBlock]) -> Vec<(BlockPath, BlockPath)> {
        let mut out: Vec<_> = self
            .pairs
            .iter()
            .map(|(a, b)| (a.clone(), b.clone()))
            .collect();
        let ends = (0..self.head).chain(self.old_len - self.tail..self.old_len);
        for root in ends {
            out.push((vec![root], self.fwd(&[root]).unwrap()));
            each(&old[root].children, &[root], &mut |p| {
                let q = self.fwd(&p).unwrap();
                out.push((p, q));
            });
        }
        out
    }
}

fn each(blocks: &[DocBlock], parent: &[usize], f: &mut impl FnMut(BlockPath)) {
    for (i, block) in blocks.iter().enumerate() {
        let mut path = parent.to_vec();
        path.push(i);
        f(path.clone());
        each(&block.children, &path, f);
    }
}

/// Pair two content-equal subtrees position by position.
fn equal(old: &DocBlock, a: BlockPath, b: BlockPath, out: &mut Alignment) {
    for (i, child) in old.children.iter().enumerate() {
        let mut ca = a.clone();
        ca.push(i);
        let mut cb = b.clone();
        cb.push(i);
        equal(child, ca, cb, out);
    }
    out.pairs.insert(a, b);
}

/// `tine_core::sync_diff` alignment of two page snapshots, mapped to block
/// paths. Leading and trailing root blocks that are content-equal on both
/// sides are paired directly and only the window between them is diffed
/// (`diff_blocks_trimmed`, the usual diff trim where it provably pairs as the
/// whole-list diff): consecutive snapshots differ in a few blocks, and this
/// keeps the per-block lsdoc projections that `diff_blocks` needs to that
/// window.
fn align(old: &[DocBlock], new: &[DocBlock], ids: &mut PersistedIds) -> Alignment {
    fn walk(
        rows: &[DiffRow],
        old: &[DocBlock],
        new: &[DocBlock],
        op: &[usize],
        np: &[usize],
        offset: usize,
        out: &mut Alignment,
    ) {
        let (mut oi, mut ni) = (0, 0);
        for row in rows {
            let mut a = op.to_vec();
            a.push(offset + oi);
            let mut b = np.to_vec();
            b.push(offset + ni);
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
                            0,
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
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    // Trim only where the whole-list diff provably pairs the same (an
    // ambiguous duplicate or a reorder near the edit falls back to the whole
    // list: correctness over speed).
    let (head, tail, rows) = match diff_blocks_trimmed(new, old, head, tail, ids) {
        Some(trimmed) => (trimmed.head, trimmed.tail, trimmed.rows),
        None => (0, 0, diff_blocks(new, old)),
    };
    let mut out = Alignment {
        head,
        tail,
        old_len: old.len(),
        new_len: new.len(),
        ..Alignment::default()
    };
    let (old_mid, new_mid) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
    walk(&rows, old_mid, new_mid, &[], &[], head, &mut out);
    out.back = out
        .pairs
        .iter()
        .map(|(a, b)| (b.clone(), a.clone()))
        .collect();
    out
}

fn block_at<'a>(blocks: &'a [DocBlock], path: &[usize]) -> &'a DocBlock {
    let block = &blocks[path[0]];
    if path.len() == 1 {
        block
    } else {
        block_at(&block.children, &path[1..])
    }
}

/// False only when `body` cannot be the (normalized) body of any block of
/// the page `text`: a block body's lines are dedented, at most right-trimmed
/// copies of page lines, and normalization only drops lines, so every
/// non-empty body line occurs in the page text.
fn may_contain(text: &str, body: &str) -> bool {
    body.lines()
        .filter(|line| !line.is_empty())
        .all(|line| text.contains(line))
}

/// One open's page history along first-parent: snapshot 0 is the baseline,
/// the last is HEAD, and step `i` (snapshot `i - 1` to `i`) is one touching
/// commit. Texts, parses and step alignments are computed on demand, each at
/// most once, and dropped with the open (no cache across opens, D-10).
struct History<'r> {
    repo: &'r Repository,
    org: bool,
    blobs: Vec<Option<Oid>>,
    /// `other[i]`: step `i` was committed by someone other than me.
    other: Vec<bool>,
    texts: Vec<Option<String>>,
    docs: Vec<Option<Document>>,
    steps: Vec<Option<Alignment>>,
    /// Ids by block body, for this open only (D-10).
    ids: PersistedIds,
}

impl<'r> History<'r> {
    fn new(repo: &'r Repository, org: bool, blobs: Vec<Option<Oid>>, other: Vec<bool>) -> Self {
        let n = blobs.len();
        Self {
            repo,
            org,
            blobs,
            other,
            texts: (0..n).map(|_| None).collect(),
            docs: (0..n).map(|_| None).collect(),
            steps: (0..n).map(|_| None).collect(),
            ids: PersistedIds::new(),
        }
    }
    fn last(&self) -> usize {
        self.blobs.len() - 1
    }
    fn snapshot_text(&mut self, i: usize) -> Result<&str, String> {
        if self.texts[i].is_none() {
            let text = match self.blobs[i] {
                None => String::new(),
                Some(oid) => {
                    let blob = self.repo.find_blob(oid).map_err(|e| e.to_string())?;
                    String::from_utf8(blob.content().to_vec()).map_err(|_| NOT_UTF8)?
                }
            };
            self.texts[i] = Some(text);
        }
        Ok(self.texts[i].as_deref().unwrap())
    }
    fn snapshot_doc(&mut self, i: usize) -> Result<&Document, String> {
        if self.docs[i].is_none() {
            let org = self.org;
            let mut doc = parse(self.snapshot_text(i)?, org);
            normalize(&mut doc.roots, org);
            self.docs[i] = Some(doc);
        }
        Ok(self.docs[i].as_ref().unwrap())
    }
    fn snapshot_step(&mut self, i: usize) -> Result<&Alignment, String> {
        if self.steps[i].is_none() {
            self.snapshot_doc(i - 1)?;
            self.snapshot_doc(i)?;
            let (old, new) = (self.docs[i - 1].as_ref(), self.docs[i].as_ref());
            self.steps[i] = Some(align(
                &old.unwrap().roots,
                &new.unwrap().roots,
                &mut self.ids,
            ));
        }
        Ok(self.steps[i].as_ref().unwrap())
    }

    /// Whether the newest step that changed or added HEAD's block at `path`
    /// was someone else's: its lineage is followed back through the step
    /// alignments. A step whose earlier snapshot cannot contain the block's
    /// body changed it, whatever the alignment, so that step needs no parse.
    fn changed_by_other(&mut self, path: &BlockPath) -> Result<bool, String> {
        let mut path = path.clone();
        for i in (1..=self.last()).rev() {
            let body = block_at(&self.snapshot_doc(i)?.roots, &path)
                .raw()
                .to_owned();
            if !may_contain(self.snapshot_text(i - 1)?, &body) {
                return Ok(self.other[i]);
            }
            let step = self.snapshot_step(i)?;
            if step.changed.contains(&path) {
                return Ok(self.other[i]);
            }
            // Every block of a later snapshot is changed or paired.
            path = step.back(&path).expect("an unchanged block is paired");
        }
        Ok(false)
    }

    /// For each baseline block path, the last removal of its lineage: who
    /// removed it and its body just before. A removed block can come back
    /// (restored with its `id::`); its baseline lineage is recovered through
    /// the same aligner, so a later removal supersedes an earlier one.
    fn deletions(&mut self) -> Result<HashMap<BlockPath, (bool, String)>, String> {
        let mut origins = HashMap::<BlockPath, BlockPath>::new();
        let mut deletions = HashMap::new();
        each(&self.snapshot_doc(0)?.roots, &[], &mut |p| {
            origins.insert(p.clone(), p);
        });
        for i in 1..=self.last() {
            self.snapshot_step(i)?;
            let (step, previous) = (
                self.steps[i].as_ref().unwrap(),
                self.docs[i - 1].as_ref().unwrap(),
            );
            for deleted in &step.removed {
                if let Some(origin) = origins.get(deleted) {
                    deletions.insert(
                        origin.clone(),
                        (
                            self.other[i],
                            block_at(&previous.roots, deleted).raw().into(),
                        ),
                    );
                }
            }
            origins = origins
                .into_iter()
                .filter_map(|(a, v)| step.fwd(&a).map(|b| (b, v)))
                .collect();
            if step.changed.iter().any(|p| !origins.contains_key(p)) {
                let base = &self.docs[0].as_ref().unwrap().roots;
                let next = &self.docs[i].as_ref().unwrap().roots;
                let mut used: HashSet<_> = origins.values().cloned().collect();
                for (base_path, next_path) in align(base, next, &mut self.ids).all_pairs(base) {
                    if !origins.contains_key(&next_path) && used.insert(base_path.clone()) {
                        origins.insert(next_path, base_path);
                    }
                }
            }
        }
        Ok(deletions)
    }
}

/// First-parent commits one open examines in total, touching the page or not
/// (D-10: never the whole history). Each costs a commit read and a tree path
/// lookup, about 0.45 ms on a 3000-page `pages/` tree (release,
/// `seen_walk_cost_per_commit`), so the bound keeps one open under ~0.25 s
/// there; past it the open shows the unattributed net diff (round-3 receipt).
const WALK_LIMIT: usize = 500;
const REWRITTEN: &str = "The seen baseline is not on this branch's main line (it was \
     rewritten by a squash, rebase or amend, or merged in from a side branch); changes since \
     the last commit it shares with this branch are shown.";
const FIRST_OPEN: &str = "This page's seen baseline is not in this branch's history; \
     changes since the graph's first-open baseline are shown.";
const HISTORY_TOO_LONG: &str = "The history since the seen baseline is too long to attribute; \
     every change since then is shown. Mark the page seen to start fresh.";
const TOO_MANY_COMMITS: &str = "More than 200 commits changed this page since the seen \
     baseline; every change since then is shown.";
const NOT_UTF8: &str = "A version of this page in the git history is not UTF-8 text.";

/// One first-parent commit: the page blob at it and at its first parent.
struct Link {
    oid: Oid,
    after: Option<Oid>,
    before: Option<Oid>,
    /// Committed by someone other than me.
    other: bool,
}

/// The first-parent line from HEAD, newest first, until the baseline, the
/// root or [`WALK_LIMIT`].
struct Walk {
    links: Vec<Link>,
    tip_blob: Option<Oid>,
    /// The baseline commit the walk stopped at.
    stopped_at: Option<Oid>,
    root: bool,
}

/// Where a baseline sits relative to the walked line.
enum Located {
    /// Compare from position `k` (`links[k]`'s commit, or the commit the walk
    /// ended at when `k == links.len()`), with a note when it is not the
    /// baseline itself.
    OnChain(usize, Option<&'static str>),
    /// Beyond the walk bound: only the net diff from this baseline.
    Beyond(Option<Oid>, Option<&'static str>),
    /// Absent, or sharing no history with the walked line.
    Unrelated,
}

impl Walk {
    fn new(
        repo: &Repository,
        tip: Option<Oid>,
        tip_blob: Option<Oid>,
        base: Option<Oid>,
        path: &Path,
        me: &super::git_run::UserIdentity,
    ) -> Result<Self, String> {
        let mut walk = Walk {
            links: Vec::new(),
            tip_blob,
            stopped_at: None,
            root: false,
        };
        let (mut cursor, mut after) = (tip, tip_blob);
        loop {
            let Some(oid) = cursor else {
                walk.root = true;
                break;
            };
            if Some(oid) == base {
                walk.stopped_at = base;
                break;
            }
            if walk.links.len() == WALK_LIMIT {
                break;
            }
            let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
            let parent = if commit.parent_count() == 0 {
                None
            } else {
                Some(commit.parent_id(0).map_err(|e| e.to_string())?)
            };
            let before = parent
                .map(|o| blob_id(repo, o, path))
                .transpose()?
                .flatten();
            walk.links.push(Link {
                oid,
                after,
                before,
                other: !me.wrote(&commit.author()),
            });
            after = before;
            cursor = parent;
        }
        Ok(walk)
    }
    /// The page blob at position `k` of [`Located::OnChain`].
    fn blob_at(&self, k: usize) -> Option<Oid> {
        match self.links.get(k) {
            Some(link) => link.after,
            None => self.links.last().map_or(self.tip_blob, |l| l.before),
        }
    }
    /// The commit at position `k` (`None`: the empty tree before the root).
    fn oid_at(&self, k: usize) -> Option<Oid> {
        match self.links.get(k) {
            Some(link) => Some(link.oid),
            None => self.stopped_at,
        }
    }
    /// Place `target` (`None`: the empty tree) on the walked line. A target
    /// off the line (rewritten by a squash, rebase or amend, or merged in
    /// from a side branch) falls back to the newest line commit among its
    /// ancestors, which shows more, never less.
    fn locate(&self, repo: &Repository, target: Option<Oid>) -> Located {
        let Some(target) = target else {
            return if self.root {
                Located::OnChain(self.links.len(), None)
            } else {
                Located::Beyond(None, None)
            };
        };
        if self.stopped_at == Some(target) {
            return Located::OnChain(self.links.len(), None);
        }
        if let Some(k) = self.links.iter().position(|l| l.oid == target) {
            return Located::OnChain(k, None);
        }
        if repo.find_commit(target).is_err() {
            return Located::Unrelated; // pruned: no history to share
        }
        // The target's ancestors, bounded like the walk (D-10).
        let mut ancestors = HashSet::new();
        let mut queue = std::collections::VecDeque::from([target]);
        while let Some(oid) = queue.pop_front() {
            if ancestors.len() == WALK_LIMIT || !ancestors.insert(oid) {
                continue;
            }
            if let Ok(commit) = repo.find_commit(oid) {
                queue.extend(commit.parent_ids());
            }
        }
        if let Some(k) = self.links.iter().position(|l| ancestors.contains(&l.oid)) {
            return Located::OnChain(k, Some(REWRITTEN));
        }
        if self.stopped_at.is_some_and(|oid| ancestors.contains(&oid)) {
            return Located::OnChain(self.links.len(), Some(REWRITTEN));
        }
        if self.root {
            Located::Unrelated
        } else {
            Located::Beyond(Some(target), None)
        }
    }
}

pub(crate) fn changes(
    data: &Path,
    root: &Path,
    key: &str,
    path: &str,
    current_text: &str,
    name: &str,
) -> Result<SeenChanges, String> {
    match changes_in(data, root, key, path, current_text, name) {
        // Malformed input in history: this open has nothing to show, and
        // the command must not fail on every open of the page.
        Err(reason) if reason == NOT_UTF8 => Ok(SeenChanges::unavailable(NOT_UTF8)),
        other => other,
    }
}

fn changes_in(
    data: &Path,
    root: &Path,
    key: &str,
    path: &str,
    current_text: &str,
    name: &str,
) -> Result<SeenChanges, String> {
    relative(path)?;
    let (repo, prefix) = match open(root) {
        Ok(r) => r,
        Err(reason) => return Ok(SeenChanges::unavailable(&reason)),
    };
    let mut result = SeenChanges::available(current_text);
    let tip = head(&repo)?;
    result.head_commit = tip.map(|o| o.to_string());
    let Some(record) = load(&record_path(data, root)) else {
        return Ok(result);
    };
    let page = record.pages.get(key);
    if page.is_some_and(|p| p.path != path) {
        return Ok(result);
    } // external rename: start again on leave
    let baseline = page
        .map(|p| &p.seen_commit)
        .unwrap_or(&record.first_open_commit);
    let base = baseline.as_ref().map(|s| Oid::from_str(s).unwrap());
    result.baseline_commit = baseline.clone();
    let me = super::git_run::user_identity(&repo, name);
    result.identity_known = me.known;
    if base == tip {
        return Ok(result);
    }
    let repo_path = prefix.join(path);
    let org = path.ends_with(".org");
    let tip_blob = tip
        .map(|oid| blob_id(&repo, oid, &repo_path))
        .transpose()?
        .flatten();
    // First-parent history describes the successive page snapshots actually
    // integrated into this branch; a merge's resulting edits belong to its
    // author (ADR 0079). The walk is bounded in total (D-10).
    let walk = Walk::new(&repo, tip, tip_blob, base, &repo_path, &me)?;
    let mut located = walk.locate(&repo, base);
    if matches!(located, Located::Unrelated)
        && page.is_some()
        && record.first_open_commit != *baseline
    {
        // The page's own baseline shares no history with this branch: the
        // graph's first-open baseline still bounds what is new.
        let first = record
            .first_open_commit
            .as_ref()
            .map(|s| Oid::from_str(s).unwrap());
        located = match walk.locate(&repo, first) {
            Located::OnChain(k, _) => Located::OnChain(k, Some(FIRST_OPEN)),
            Located::Beyond(_, _) => Located::Beyond(first, Some(FIRST_OPEN)),
            Located::Unrelated => Located::Unrelated,
        };
    }
    let (blobs, other) = match located {
        Located::Unrelated => {
            return Ok(SeenChanges::unavailable(
                "The seen baseline is not in this branch's history. Mark the page seen to start fresh.",
            ));
        }
        Located::Beyond(baseline, note) => {
            // Only the net baseline-to-HEAD diff is used.
            result.unattributed = true;
            result.reason = Some(note.unwrap_or(HISTORY_TOO_LONG).into());
            result.baseline_commit = baseline.map(|o| o.to_string());
            let blob = baseline
                .map(|oid| blob_id(&repo, oid, &repo_path))
                .transpose()?
                .flatten();
            (vec![blob, tip_blob], vec![true, true])
        }
        Located::OnChain(k, note) => {
            if let Some(note) = note {
                result.reason = Some(note.into());
                result.baseline_commit = walk.oid_at(k).map(|o| o.to_string());
            }
            let touching: Vec<_> = walk.links[..k]
                .iter()
                .filter(|l| l.before != l.after)
                .collect();
            if touching.len() > COMMIT_LIMIT {
                result.unattributed = true;
                result.reason = Some(match note {
                    Some(note) => format!("{note} {TOO_MANY_COMMITS}"),
                    None => TOO_MANY_COMMITS.into(),
                });
                (vec![walk.blob_at(k), tip_blob], vec![true, true])
            } else {
                let mut blobs = vec![walk.blob_at(k)];
                let mut other = vec![false];
                for link in touching.into_iter().rev() {
                    blobs.push(link.after);
                    other.push(link.other);
                }
                (blobs, other)
            }
        }
    };
    let mut history = History::new(&repo, org, blobs, other);
    let last = history.last();
    history.snapshot_doc(0)?;
    history.snapshot_doc(last)?;
    let net = align(
        &history.docs[0].as_ref().unwrap().roots,
        &history.docs[last].as_ref().unwrap().roots,
        &mut history.ids,
    );
    // The supplied snapshot usually is HEAD's text (no unsaved edits): then
    // it maps onto HEAD block for block, with no parse or alignment.
    let current = if history.snapshot_text(last)? == current_text {
        None
    } else {
        let mut current = parse(current_text, org);
        normalize(&mut current.roots, org);
        let latest = &history.docs[last].as_ref().unwrap().roots;
        let working = align(latest, &current.roots, &mut history.ids);
        Some((current, working))
    };
    let to_current = |p: &BlockPath| -> Option<BlockPath> {
        match &current {
            None => Some(p.clone()),
            Some((_, working)) => working.fwd(p).filter(|q| !working.changed.contains(q)),
        }
    };
    let mut changed: Vec<_> = net.changed.iter().cloned().collect();
    changed.sort();
    for tip_path in changed {
        if !result.unattributed && !history.changed_by_other(&tip_path)? {
            continue;
        }
        if let Some(mapped) = to_current(&tip_path) {
            let roots = match &current {
                None => &history.docs[last].as_ref().unwrap().roots,
                Some((doc, _)) => &doc.roots,
            };
            result.blocks.push(SeenBlock {
                persisted_id: block_at(roots, &mapped).property("id"),
                block_path: mapped,
            });
        }
    }
    let deletions = if result.unattributed || net.removed.is_empty() {
        HashMap::new()
    } else {
        history.deletions()?
    };
    let old = &history.docs[0].as_ref().unwrap().roots;
    let mapped = |p: &BlockPath| -> Option<BlockPath> {
        let at_tip = net.fwd(p)?;
        match &current {
            None => Some(at_tip),
            Some((_, working)) => working.fwd(&at_tip),
        }
    };
    for path in &net.removed {
        if !result.unattributed && !deletions.get(path).is_some_and(|(other, _)| *other) {
            continue;
        }
        let mut parent = path[..path.len() - 1].to_vec();
        // Removed parents have no rendered child list; attach to the nearest
        // surviving ancestor's list, with the same preceding-sibling rule.
        let mut slot = *path.last().unwrap();
        while !parent.is_empty() && mapped(&parent).is_none() {
            slot = parent.pop().unwrap();
        }
        let previous_sibling_path = (0..slot).rev().find_map(|i| {
            let mut p = parent.clone();
            p.push(i);
            mapped(&p)
        });
        result.removed.push(SeenRemoved {
            text: deletions
                .get(path)
                .map(|(_, text)| text.clone())
                .unwrap_or_else(|| block_at(old, path).raw().into()),
            baseline_block_path: path.clone(),
            parent_path: if parent.is_empty() {
                None
            } else {
                mapped(&parent)
            },
            previous_sibling_path,
        });
    }
    result
        .blocks
        .sort_by(|a, b| a.block_path.cmp(&b.block_path));
    result
        .removed
        .sort_by(|a, b| a.baseline_block_path.cmp(&b.baseline_block_path));
    Ok(result)
}

#[cfg(test)]
#[path = "seen_tests.rs"]
mod tests;
