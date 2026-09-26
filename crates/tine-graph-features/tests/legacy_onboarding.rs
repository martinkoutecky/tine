use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::config::{Config, FileNameFormat};
use tine_core::guide::{
    bundled_guide_pages, guide_copy_page_name, rewrite_bundled_guide_links, GUIDE_TEMPLATES,
};
use tine_graph_features::guide;
use tine_store::{PageId, Resolved, Store};

const TARGET_ID: &str = "7a1c0f5e-0000-4000-8000-000000000001";

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tine-onboard-port-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn open(dir: &Path) -> Store {
    Store::open(dir, Default::default()).unwrap().0
}

fn page_id(store: &Store, name: &str) -> PageId {
    match store.whole_graph().unwrap().resolve(name, false) {
        Resolved::Existing { id, .. } => id,
        _ => panic!("page {name:?} not listed"),
    }
}

fn atomic_write(root: &Path, path: &Path, bytes: &[u8]) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = root.join(format!(
        ".guide-fixture-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp, bytes).unwrap();
    fs::rename(temp, path).unwrap();
}

#[test]
fn demo_graph_scaffolds_and_resolves() {
    let dir = scratch("demo");
    guide::create_demo_graph(&dir).unwrap();
    assert!(dir.join("logseq/config.edn").is_file());
    assert!(dir.join("journals").is_dir());
    assert!(dir.join("assets/quick-capture.png").is_file());
    let cfg = Config::parse(&fs::read_to_string(dir.join("logseq/config.edn")).unwrap());
    assert_eq!(cfg.file_name_format, FileNameFormat::TripleLowbar);
    assert!(dir.join("pages/Features___Quick capture.md").is_file());
    assert!(dir.join("pages/Tine Guide.md").is_file());
    assert!(dir.join("pages/Feature showcase.md").is_file());
    assert!(dir.join("pages/Project___Roadmap.md").is_file());
    let store = open(&dir);
    for template in GUIDE_TEMPLATES {
        store
            .page(&page_id(&store, template.title))
            .unwrap_or_else(|e| panic!("page {:?} failed to load: {e:?}", template.title));
    }
    let view = store.whole_graph().unwrap();
    assert!(
        view.blocks(&[TARGET_ID.to_string()]).unwrap()[0].is_some(),
        "block-ref target missing"
    );
    assert_eq!(
        view.block_ref_counts().get(TARGET_ID).copied(),
        Some(2),
        "expected 2 referrers of the demo block"
    );
    let dto = store.page(&page_id(&store, "Welcome to Tine")).unwrap().doc;
    let parents = |needle: &str| {
        dto.blocks
            .iter()
            .any(|b| b.raw.starts_with(needle) && !b.children.is_empty())
    };
    assert!(
        parents("## Try the basics"),
        "section heading should parent its body"
    );
    assert!(
        parents("# Welcome to Tine"),
        "page heading should parent its intro"
    );
}

#[test]
fn bundled_guide_pages_are_read_only_virtual_pages() {
    let pages = bundled_guide_pages();
    let index = pages
        .iter()
        .find(|p| p.title == "Tine Guide")
        .expect("guide index is bundled");
    assert_eq!(index.page.name, "Tine-guide/Tine Guide");
    assert!(index.page.read_only);
    assert!(index.page.guide);
    let sheets = pages
        .iter()
        .find(|p| p.title == "Features/Sheets")
        .expect("sheets guide is bundled");
    assert!(sheets.markdown.contains("Create one yourself"));
    assert!(sheets
        .page
        .blocks
        .iter()
        .any(|b| b.raw.contains("Positional grid")));
    let plugins = pages
        .iter()
        .find(|p| p.title == "Features/Plugins")
        .expect("plugins guide is bundled");
    assert!(plugins.markdown.contains("installed disabled"));
    assert!(plugins.markdown.contains("not Logseq or Obsidian plugins"));
}

fn extract_page_links(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = markdown;
    while let Some(i) = rest.find("[[") {
        let after = &rest[i + 2..];
        let Some(j) = after.find("]]") else { break };
        out.push(after[..j].trim().to_string());
        rest = &after[j + 2..];
    }
    out
}

#[test]
fn guide_link_set_is_closed_over_demo_pages() {
    let guide: HashSet<String> = GUIDE_TEMPLATES
        .iter()
        .map(|t| tine_core::refs::page_key(t.title))
        .collect();
    let demo: HashSet<String> = GUIDE_TEMPLATES
        .iter()
        .map(|t| tine_core::refs::page_key(t.title))
        .collect();
    for template in GUIDE_TEMPLATES {
        for target in extract_page_links(template.markdown) {
            let key = tine_core::refs::page_key(&target);
            if demo.contains(&key) {
                assert!(guide.contains(&key), "guide page {:?} links to demo page {:?}, which is not in GUIDE_TEMPLATES — the link dangles in the in-app guide and in copy-into-graph. Add it to GUIDE_TEMPLATES.", template.title, target);
            }
        }
    }
}

#[test]
fn guide_copy_rewrites_interguide_page_refs_only() {
    let copied = [
        "Tine Guide",
        "Features/Sheets",
        "Features/Formulas",
        "Features/Quick capture",
        "Features/PDF annotation",
        "Features/Plugins",
        "Features/Tips & shortcuts",
        "Feature showcase",
    ];
    let index = GUIDE_TEMPLATES
        .iter()
        .find(|p| p.title == "Tine Guide")
        .unwrap()
        .markdown;
    let mut sample = index.to_string();
    sample.push_str("\n- [[Martin]] #demo #sheets-demo\n- [read showcase]([[Feature showcase]])\n- {{embed [[Features/Tips & shortcuts]]}}\n- {{query [[Features/Quick capture]]}}\n");
    let renames: HashMap<String, String> = copied
        .iter()
        .map(|title| {
            (
                tine_core::refs::page_key(title),
                guide_copy_page_name(title.trim()),
            )
        })
        .collect();
    let out = rewrite_bundled_guide_links(&sample, &renames);
    assert!(
        out.contains("[[tine-guide/Features/Sheets]]"),
        "index link was not rewritten: {out}"
    );
    assert!(
        out.contains("[read showcase]([[tine-guide/Feature showcase]])"),
        "labelled page link was not rewritten: {out}"
    );
    assert!(
        out.contains("{{embed [[tine-guide/Features/Tips & shortcuts]]}}"),
        "embed page link was not rewritten: {out}"
    );
    assert!(
        out.contains("{{query [[tine-guide/Features/Quick capture]]}}"),
        "query page link was not rewritten: {out}"
    );
    assert!(
        out.contains("[[Martin]] #demo #sheets-demo"),
        "non-guide refs must stay verbatim: {out}"
    );
}

#[test]
fn copy_guide_into_graph_writes_whole_lowercase_namespace_and_assets() {
    let dir = scratch("whole");
    fs::create_dir_all(dir.join("pages")).unwrap();
    let store = open(&dir);
    let copied = guide::copy_guide_into_graph(&store, "Features/Sheets").unwrap();
    assert_eq!(copied.name, "tine-guide/Features/Sheets");
    assert!(copied.created);
    assert_eq!(copied.created_pages.len(), GUIDE_TEMPLATES.len());
    assert!(copied.skipped_pages.is_empty());
    assert_eq!(copied.copied_assets, vec!["quick-capture.png".to_string()]);
    for template in bundled_guide_pages() {
        let name = guide_copy_page_name(&template.title);
        let id = page_id(&store, &name);
        assert!(
            store.path_for_os_handoff(&id.file()).unwrap().is_file(),
            "missing copied guide page {name}"
        );
        let dto = store.page(&id).unwrap().doc;
        assert_eq!(dto.name, name);
        assert!(
            !dto.read_only && !dto.guide,
            "copied guide page must be normal/editable: {name}"
        );
    }
    let index = fs::read_to_string(
        store
            .path_for_os_handoff(&page_id(&store, "tine-guide/Tine Guide").file())
            .unwrap(),
    )
    .unwrap();
    assert!(index.contains("[[tine-guide/Features/Sheets]]"));
    assert!(!index.contains("[[Features/Sheets]]"));
    assert!(dir.join("assets/quick-capture.png").is_file());
}

#[test]
fn recopy_guide_skips_existing_pages_without_clobbering_user_edits() {
    let dir = scratch("recopy");
    fs::create_dir_all(dir.join("pages")).unwrap();
    let store = open(&dir);
    guide::copy_guide_into_graph(&store, "Tine Guide").unwrap();
    let edited = guide_copy_page_name("Features/Sheets");
    let edited_path = store
        .path_for_os_handoff(&page_id(&store, &edited).file())
        .unwrap();
    atomic_write(&dir, &edited_path, b"- user edits stay\n");
    store.scan_refresh().unwrap();
    let before: HashMap<String, String> = GUIDE_TEMPLATES
        .iter()
        .map(|template| {
            let name = guide_copy_page_name(template.title);
            let path = store
                .path_for_os_handoff(&page_id(&store, &name).file())
                .unwrap();
            (name, fs::read_to_string(path).unwrap())
        })
        .collect();
    let existing = guide::copy_guide_into_graph(&store, "Features/Sheets").unwrap();
    assert_eq!(existing.name, edited);
    assert!(!existing.created);
    assert!(existing.created_pages.is_empty());
    assert_eq!(existing.skipped_pages.len(), GUIDE_TEMPLATES.len());
    for (name, body) in before {
        let path = store
            .path_for_os_handoff(&page_id(&store, &name).file())
            .unwrap();
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            body,
            "recopy clobbered {name}"
        );
    }
    assert_eq!(
        fs::read_to_string(edited_path).unwrap(),
        "- user edits stay\n"
    );
}

#[cfg(unix)]
#[test]
fn copy_guide_rejects_pages_directory_symlink_swap() {
    use std::os::unix::fs::symlink;
    let dir = scratch("pages-swap");
    let outside = scratch("pages-outside");
    fs::create_dir_all(dir.join("pages")).unwrap();
    let store = open(&dir);
    fs::remove_dir(dir.join("pages")).unwrap();
    symlink(&outside, dir.join("pages")).unwrap();
    assert!(guide::copy_guide_into_graph(&store, "Tine Guide").is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn copy_guide_rejects_assets_directory_symlink_swap() {
    use std::os::unix::fs::symlink;
    let dir = scratch("assets-swap");
    let outside = scratch("assets-outside");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    let store = open(&dir);
    fs::remove_dir(dir.join("assets")).unwrap();
    symlink(&outside, dir.join("assets")).unwrap();
    assert!(guide::copy_guide_into_graph(&store, "Tine Guide").is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}
