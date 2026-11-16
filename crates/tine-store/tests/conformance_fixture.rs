use super::Store;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct Rng(pub u64);

impl Rng {
    pub(crate) fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize % n
    }

    pub(crate) fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

pub(crate) fn seeds() -> usize {
    let count = std::env::var("TINE_F3_SEEDS")
        .ok()
        .map(|n| n.parse().unwrap())
        .unwrap_or(1);
    assert!(count > 0, "TINE_F3_SEEDS must be positive");
    count
}

pub(crate) struct Graph {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) root: PathBuf,
    pub(crate) store: Arc<Store>,
}

impl Graph {
    pub(crate) fn new(files: &BTreeMap<String, Vec<u8>>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("graph");
        for area in ["pages", "journals", "assets", "logseq"] {
            fs::create_dir_all(root.join(area)).unwrap();
        }
        for (rel, bytes) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        Self { dir, root, store }
    }
}

impl Drop for Graph {
    fn drop(&mut self) {
        self.store.close();
    }
}

pub(crate) fn collect(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), out);
            } else {
                out.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Only actual live files or the declared recovery roots count. Temporary
/// files and arbitrary copies elsewhere in the graph cannot satisfy this.
pub(crate) fn conserved(
    root: &Path,
    payload: &[u8],
    live: &[String],
    recovery: &[PathBuf],
    at: &str,
) {
    let mut files = BTreeMap::new();
    for rel in live {
        if let Ok(bytes) = fs::read(root.join(rel)) {
            files.insert(root.join(rel), bytes);
        }
    }
    for dir in recovery {
        for (rel, bytes) in collect(dir) {
            if !rel.ends_with(".tmp") {
                files.insert(dir.join(rel), bytes);
            }
        }
    }
    assert!(
        files.values().any(|bytes| bytes == payload),
        "lost input payload at {at}: {:?}; locations: {:?}",
        String::from_utf8_lossy(payload),
        files.keys().collect::<Vec<_>>()
    );
}

/// Retained transaction/host namespaces, not the entire trash tree. These
/// schedules contain page moves/merges, asset trash, and undo conflict copies.
pub(crate) fn transaction_recovery(root: &Path) -> Vec<PathBuf> {
    ["pages", "assets", "conflicts"]
        .map(|kind| root.join("logseq/.tine-trash").join(kind))
        .to_vec()
}

/// Reuses F2/F5's complete snapshot image, forcing lazy indexes before each
/// operation and comparing against a physically separate cold Store afterward.
pub(crate) fn fresh(store: &Store, root: &Path, probes: &[&str], at: &str) {
    let live = store.conformance_image_for_tests(probes);
    let copy = tempfile::tempdir().unwrap();
    for (rel, bytes) in collect(root) {
        // Recovery is not live graph text. Copy only graph inputs.
        if rel.starts_with("pages/")
            || rel.starts_with("journals/")
            || rel == "logseq/config.edn"
            || (rel.starts_with("assets/") && !rel.contains("/."))
        {
            let path = copy.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
    }
    let cold = Store::open(copy.path(), Default::default()).unwrap().0;
    let rebuilt = cold.conformance_image_for_tests(probes);
    cold.close();
    if live != rebuilt {
        let line = live
            .lines()
            .zip(rebuilt.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "live/fresh index mismatch at {at}, line {line}:\nlive: {:?}\nfresh: {:?}",
            live.lines().nth(line),
            rebuilt.lines().nth(line)
        );
    }
}
