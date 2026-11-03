//! Conceptual model step 2, batch 6: the anonymized-graph byte comparison
//! (design note `cm-step2-rename-scanner-design.md` §7). Ignored; run with
//! `CM_ANON=<graph> CM_BYTES_LABEL=<base|candidate>` and
//! `-- --ignored cm_rename_bytes`. Each sampled name is renamed through the
//! real path (`pages::rename_or_merge_page` on a page host, the audited save
//! path) on a fresh copy of the graph; every file the rename changed is
//! recorded under the worktree's untracked `artifacts/cm/bytes/<label>.json`.
//! The sample is chosen once (`sample.json`) and reused, so the base and the
//! candidate rename the same names. The report holds graph text: it stays in
//! `artifacts/` and is never quoted; comparisons report forms and counts only.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tine_graph_features::pages;

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/cm/bytes")
}

fn graph_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for dir in ["pages", "journals"] {
        let Ok(read) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in read.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file()) {
                out.push(format!("{dir}/{}", entry.file_name().to_string_lossy()));
            }
        }
    }
    out.sort();
    out
}

fn copy_graph(from: &Path, to: &Path) {
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(to.join(dir)).unwrap();
        let Ok(read) = fs::read_dir(from.join(dir)) else {
            continue;
        };
        for entry in read.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file()) {
                fs::copy(entry.path(), to.join(dir).join(entry.file_name())).unwrap();
            }
        }
    }
}

/// The names to rename, chosen from the reference evidence of the original
/// graph: the most-referenced names, namespace parents, tag-only names and
/// every name a query macro spells (design note §7).
fn sample(root: &Path) -> Vec<String> {
    let path = out_dir().join("sample.json");
    if let Ok(text) = fs::read_to_string(&path) {
        return serde_json::from_str(&text).unwrap();
    }
    // name key -> (display name, pages, every occurrence a tag)
    let mut seen: BTreeMap<String, (String, BTreeSet<usize>, bool)> = BTreeMap::new();
    let mut query_names: BTreeSet<String> = BTreeSet::new();
    for (i, rel) in graph_files(root).iter().enumerate() {
        let Ok(text) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        let doc = if rel.ends_with(".org") {
            tine_core::org::parse_org(&text)
        } else {
            tine_core::doc::parse(&text)
        };
        let mut stack: Vec<&tine_core::DocBlock> = doc.roots.iter().collect();
        while let Some(block) = stack.pop() {
            stack.extend(block.children.iter());
            for r in block.projection().reference_source().explicit {
                if matches!(
                    r.rule,
                    "explicit_task_marker" | "explicit_priority" | "explicit_property_key"
                ) || r.name.starts_with("((")
                {
                    continue;
                }
                let entry = seen
                    .entry(tine_core::refs::page_key(&r.name))
                    .or_insert_with(|| (r.name.clone(), BTreeSet::new(), true));
                entry.1.insert(i);
                entry.2 &= r.rule == "explicit_tag";
            }
            for extent in tine_core::query::macro_text::query_macro_extents(block.raw()) {
                let arg = extent.argument.as_str();
                for piece in arg.split("[[").skip(1) {
                    if let Some(end) = piece.find("]]") {
                        query_names.insert(piece[..end].trim().to_string());
                    }
                }
                for piece in arg.split("(page ").skip(1) {
                    let piece = piece.trim_start().trim_start_matches('"');
                    if let Some(end) = piece.find(['"', ')']) {
                        query_names.insert(piece[..end].trim().to_string());
                    }
                }
            }
        }
    }
    let mut by_pages: Vec<(&String, &(String, BTreeSet<usize>, bool))> = seen.iter().collect();
    by_pages.sort_by(|a, b| b.1 .1.len().cmp(&a.1 .1.len()).then(a.0.cmp(b.0)));
    let mut names: Vec<String> = Vec::new();
    let mut push = |name: &str, names: &mut Vec<String>| {
        let key = tine_core::refs::page_key(name);
        if !key.is_empty() && !names.iter().any(|n| tine_core::refs::page_key(n) == key) {
            names.push(name.to_string());
        }
    };
    for (_, (name, _, _)) in by_pages.iter().take(30) {
        push(name, &mut names);
    }
    let mut parents: BTreeMap<String, usize> = BTreeMap::new();
    for (key, (name, pages, _)) in &seen {
        if let Some((parent, _)) = name.rsplit_once('/') {
            let _ = key;
            *parents.entry(parent.to_string()).or_default() += pages.len();
        }
    }
    let mut parents: Vec<_> = parents.into_iter().collect();
    parents.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (parent, _) in parents.iter().take(10) {
        push(parent, &mut names);
    }
    for (_, (name, _, _)) in by_pages.iter().filter(|(_, v)| v.2).take(10) {
        push(name, &mut names);
    }
    for name in &query_names {
        push(name, &mut names);
    }
    fs::create_dir_all(out_dir()).unwrap();
    fs::write(&path, serde_json::to_string_pretty(&names).unwrap()).unwrap();
    names
}

#[test]
#[ignore]
fn cm_rename_bytes() {
    let Some(root) = std::env::var_os("CM_ANON") else {
        return;
    };
    let root = PathBuf::from(root);
    let label = std::env::var("CM_BYTES_LABEL").unwrap_or_else(|_| "run".into());
    let names = sample(&root);
    let files = graph_files(&root);
    let original: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|rel| (rel.clone(), fs::read(root.join(rel)).unwrap()))
        .collect();
    let scratch = out_dir().join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let mut report: BTreeMap<String, Value> = BTreeMap::new();
    for (i, name) in names.iter().enumerate() {
        let dir = tempfile::tempdir_in(&scratch).unwrap();
        copy_graph(&root, dir.path());
        let store = std::sync::Arc::new(
            tine_store::Store::open(dir.path(), Default::default())
                .unwrap()
                .0,
        );
        store.whole_graph().unwrap();
        let to = format!("Zq renamed {i}");
        let started = std::time::Instant::now();
        let result = {
            let app_data = tempfile::tempdir().unwrap();
            let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
            pages::rename_or_merge_page(&store, &host, name, &to, None, None, &[])
                .map(|_| ())
                .map_err(|e| format!("{:?}", e.kind()))
        };
        let ms = started.elapsed().as_millis();
        drop(store);
        let after: BTreeSet<String> = graph_files(dir.path()).into_iter().collect();
        let mut changed: BTreeMap<String, Value> = BTreeMap::new();
        for rel in &after {
            let bytes = fs::read(dir.path().join(rel)).unwrap();
            if original.get(rel) != Some(&bytes) {
                changed.insert(
                    rel.clone(),
                    json!(String::from_utf8_lossy(&bytes).into_owned()),
                );
            }
        }
        for rel in original.keys() {
            if !after.contains(rel) {
                changed.insert(rel.clone(), Value::Null);
            }
        }
        report.insert(
            format!("{i:03}"),
            json!({"name": name, "result": result.err(), "ms": ms, "changed": changed}),
        );
    }
    let path = out_dir().join(format!("{label}.json"));
    fs::write(&path, serde_json::to_string(&report).unwrap()).unwrap();
    eprintln!(
        "cm_rename_bytes: {} names, report {}",
        names.len(),
        path.display()
    );
}
