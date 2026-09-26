use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use tine_graph_features::{print, publish};
use tine_store::Store;

fn atomic_fixture_write(path: &Path, bytes: impl AsRef<[u8]>) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = path.parent().unwrap().parent().unwrap();
    let temp = root.join(format!(
        ".print-fixture-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp, bytes).unwrap();
    fs::rename(temp, path).unwrap();
}

fn scratch(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/b14a2-golden/test-copies")
        .join(format!(
            "{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&path).unwrap();
    fs::canonicalize(path).unwrap()
}

fn copy_fixture(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_fixture(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn site_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), out);
            } else if entry.file_type().unwrap().is_file() {
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
    walk(root, root, &mut out);
    out
}

fn dump(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let (store, _, _) = Store::open(root, Default::default()).unwrap();
    let (site, _) = publish::publish_html(&store).unwrap();
    let mut out = site_files(Path::new(&site))
        .into_iter()
        .map(|(path, bytes)| (format!("site/{path}"), bytes))
        .collect::<BTreeMap<_, _>>();
    let mut pages = [tine_store::Area::Pages, tine_store::Area::Journals]
        .into_iter()
        .flat_map(|area| store.scan_area(area, None).unwrap().files)
        .filter_map(|file| file.page.map(|id| (file.id, id)))
        .collect::<Vec<_>>();
    pages.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    let stride = pages.len().div_ceil(30).max(1);
    for (index, page) in pages.iter().enumerate().step_by(stride) {
        let name = store.page(&page.1).unwrap().doc.name;
        let html = print::page_print_html(&store, &name, print::PrintOpts::default())
            .unwrap()
            .expect("listed page");
        out.insert(format!("print/{index:06}.html"), html.into_bytes());
    }
    out
}

fn differences(a: &BTreeMap<String, Vec<u8>>, b: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    a.keys()
        .chain(b.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|key| a.get(*key) != b.get(*key))
        .cloned()
        .collect()
}

#[test]
fn fixture_print_and_publish_match_golden() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/publish_print");
    let old_root = scratch("fixture-old");
    let new_root = scratch("fixture-new");
    copy_fixture(&fixture, &old_root);
    copy_fixture(&fixture, &new_root);
    let first = dump(&old_root);
    let new = dump(&new_root);
    assert!(
        differences(&first, &new).is_empty(),
        "different paths: {:?}",
        differences(&first, &new)
    );
    let manifest = include_str!("fixtures/publish_print.sha256");
    let actual = new
        .iter()
        .map(|(path, bytes)| format!("{:x}  {path}\n", Sha256::digest(bytes)))
        .collect::<String>();
    assert_eq!(actual, manifest);
}

#[test]
fn external_visibility_edit_is_seen_after_corpus_warmup() {
    let root = scratch("visibility-edit");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(
        root.join("pages/Alpha.md"),
        "public:: true\n- before edit\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), 1);
    atomic_fixture_write(
        &root.join("pages/Alpha.md"),
        "public:: false\n- after edit\n",
    );
    let (site, count) = publish::publish_html(&store).unwrap();
    assert_eq!(count, 0);
    assert!(!Path::new(&site).join("alpha.html").exists());
    let html = print::page_print_html(&store, "Alpha", print::PrintOpts::default())
        .unwrap()
        .unwrap();
    assert!(html.contains("after edit"));
}

#[test]
fn colliding_public_identity_does_not_publish_private_twin() {
    let root = scratch("public-twin");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/Twin.md"), "public:: true\n- visible\n").unwrap();
    fs::write(root.join("pages/twin.md"), "- private twin token\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let (site, count) = publish::publish_html(&store).unwrap();
    assert_eq!(count, 0);
    let files = site_files(Path::new(&site));
    assert!(!files.keys().any(|name| name == "twin.html"));
    assert!(!files.values().any(|bytes| bytes
        .windows(b"private twin token".len())
        .any(|window| window == b"private twin token")));
}

#[test]
#[ignore = "requires TINE_CORPUS and two cp -a graph copies"]
fn corpus_print_and_publish_match_golden() {
    let source = PathBuf::from(std::env::var_os("TINE_CORPUS").expect("TINE_CORPUS"));
    let base = scratch("corpus");
    let old_root = base.join("old");
    let new_root = base.join("new");
    for target in [&old_root, &new_root] {
        let status = Command::new("cp")
            .arg("-a")
            .arg(&source)
            .arg(target)
            .status()
            .unwrap();
        assert!(status.success(), "cp -a failed");
    }
    let old = dump(&old_root);
    let new = dump(&new_root);
    let oracle = site_files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/b14a2-golden/old/corpus"),
    );
    assert!(
        differences(&oracle, &old).is_empty(),
        "different paths: {:?}",
        differences(&oracle, &old)
    );
    assert!(
        differences(&oracle, &new).is_empty(),
        "different paths: {:?}",
        differences(&oracle, &new)
    );
}
