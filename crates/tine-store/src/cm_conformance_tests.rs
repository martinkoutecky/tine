//! Conceptual model conformance, step 1 (private queue card "Conceptual model:
//! conformance harness + divergence map"). Each fixture graph goes through this
//! store and through the Lean model's executable bridge (`cm-bridge`, path in
//! `CM_BRIDGE`); the test reports every place where they answer differently.
//!
//! The model's given functions come from Tine: names, titles, aliases, the
//! outline and each block's occurrences are extracted here with Tine's parser
//! and handed to the bridge, so a difference is in the meaning layer (pages,
//! owners, navigation, ...), never in parsing. The identity fold handed to the
//! model is Tine's: `page_key`, with a date-shaped spelling folded to its day.
//!
//! Step 1 records differences; it changes no production code. Without
//! `CM_BRIDGE` the test skips. `CM_REPORT` names a file for the full report.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use tine_core::config::FileNameFormat;
use tine_core::date::JournalFormat;
use tine_core::doc::{self, DocBlock};
use tine_core::model::{decode_page_name, page_title_from_preamble, Format, PageId};
use tine_core::refs::page_key;

use crate::query::document_alias_spellings;
use crate::{InventoryScope, Resolved, Store, WholeGraph};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cm-conformance");

/// Tine's identity fold, as the model's `key`.
fn fold(journal: &JournalFormat, s: &str) -> String {
    match journal.parse(s.trim()) {
        Some(day) => page_key(&journal.title(day)),
        None => page_key(s),
    }
}

fn occ_kind(rule: &str) -> &'static str {
    match rule {
        "explicit_link" | "explicit_nested_link" => "link",
        "explicit_tag" => "tag",
        "explicit_embed" => "pageEmbed",
        "explicit_property_key" => "propertyKey",
        "explicit_task_marker" => "taskMarker",
        "explicit_priority" => "priority",
        "implicit_linkable_property" => "linkableValue",
        other => panic!("reference rule {other} has no model occurrence kind"),
    }
}

fn block_json(raw: &str, level: usize, block: &DocBlock) -> Value {
    let occ: Vec<Value> = block
        .projection()
        .reference_source()
        .explicit
        .iter()
        .map(|r| json!([occ_kind(r.rule), r.name]))
        .collect();
    json!({"text": raw, "level": level, "occ": occ})
}

fn walk(blocks: &[DocBlock], level: usize, out: &mut Vec<Value>) {
    for block in blocks {
        out.push(block_json(block.raw(), level, block));
        walk(&block.children, level + 1, out);
    }
}

/// The model's view of one file: Tine's extraction as the given functions.
fn file_json(view: &WholeGraph, rel: &str, text: &str) -> Value {
    let journal_dir = rel.starts_with("journals/");
    assert!(rel.ends_with(".md"), "step 1 fixtures are Markdown: {rel}");
    let file_stem = Path::new(rel).file_stem().unwrap().to_str().unwrap();
    let stem = if journal_dir {
        // A journal's name is its day title (Tine ignores title:: there).
        let entries = view.inventory(InventoryScope::File(&PageId::from(rel)));
        entries
            .0
            .first()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| file_stem.to_owned())
    } else {
        decode_page_name(file_stem, FileNameFormat::Legacy)
    };
    let parsed = doc::parse(text);
    let mut blocks = Vec::new();
    if let Some(pre) = &parsed.pre_block {
        let header = tine_core::page_properties::property_projection_with_config(
            pre,
            false,
            &Default::default(),
        );
        blocks.push(block_json(pre, 1, &header));
    }
    walk(&parsed.roots, 1, &mut blocks);
    json!({
        "path": rel,
        "dir": if journal_dir { "journals" } else { "pages" },
        "format": "md",
        "text": text,
        "stem": stem,
        "title": page_title_from_preamble(text, Format::Md),
        "aliases": document_alias_spellings(&parsed),
        "preBlock": parsed.pre_block.is_some(),
        "blocks": blocks,
    })
}

/// Every spelling the model may key: names, aliases, occurrences and their
/// namespace prefixes, probes and rename arguments.
fn spellings(input: &Value, extra: &[String]) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = extra.iter().cloned().collect();
    for file in input["files"].as_array().unwrap() {
        for key in ["stem", "title"] {
            if let Some(s) = file[key].as_str() {
                out.insert(s.to_owned());
            }
        }
        for a in file["aliases"].as_array().unwrap() {
            out.insert(a.as_str().unwrap().to_owned());
        }
        for b in file["blocks"].as_array().unwrap() {
            for o in b["occ"].as_array().unwrap() {
                out.insert(o[1].as_str().unwrap().to_owned());
            }
        }
    }
    let mut closed = BTreeSet::new();
    for s in out {
        let parts: Vec<&str> = s.split('/').collect();
        for i in 1..parts.len() {
            closed.insert(parts[..i].join("/"));
        }
        closed.insert(s);
    }
    closed
}

/// Tine's page for a resolution, as the model's (kind, key).
fn tine_page(
    view: &WholeGraph,
    journal: &JournalFormat,
    r: &Resolved,
    name: &str,
    journal_kind: bool,
) -> (String, String) {
    let entry_of = |id: &PageId| {
        let inv = view.inventory(InventoryScope::File(id));
        let e = inv.0.first().expect("a claimant has an inventory entry");
        (
            if e.is_journal { "journal" } else { "page" }.to_owned(),
            fold(journal, &e.name),
        )
    };
    match r {
        Resolved::Existing { id, .. } => entry_of(id),
        Resolved::Alias { owners } => entry_of(&owners[0]),
        Resolved::Absent { .. } => (
            if journal_kind { "journal" } else { "page" }.to_owned(),
            fold(journal, name),
        ),
    }
}

/// What a link `[[name]]` opens: the frontend's link kind (`linkPageKind`:
/// a journal file for the day, else an ordinary file, else the day) over
/// `WholeGraph::resolve`.
fn navigate(view: &WholeGraph, journal: &JournalFormat, name: &str) -> (String, String) {
    if journal.parse(name.trim()).is_some() {
        let as_journal = view.resolve(name, true);
        if matches!(as_journal, Resolved::Existing { .. }) {
            return tine_page(view, journal, &as_journal, name, true);
        }
        let as_page = view.resolve(name, false);
        if matches!(as_page, Resolved::Existing { .. }) {
            return tine_page(view, journal, &as_page, name, false);
        }
        return tine_page(view, journal, &as_journal, name, true);
    }
    tine_page(view, journal, &view.resolve(name, false), name, false)
}

fn page_pair(v: &Value) -> (String, String) {
    (
        v[0].as_str().unwrap().to_owned(),
        v[1].as_str().unwrap().to_owned(),
    )
}

struct Outcome {
    name: String,
    diffs: Vec<String>,
}

fn run_fixture(bridge: &Path, path: &Path) -> Outcome {
    let fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("journals")).unwrap();
    let files: BTreeMap<String, String> = fixture["files"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
        .collect();
    for (rel, text) in &files {
        let p = dir.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let view = store.whole_graph().expect("fixture graph loads");
    let journal = JournalFormat::default();

    let probes: Vec<String> = fixture["navigate"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default();
    let mut input = json!({
        "files": files.iter().map(|(rel, text)| file_json(&view, rel, text)).collect::<Vec<_>>(),
        "navigate": probes,
    });
    let keyed = spellings(&input, &probes);
    input["keys"] = json!(keyed
        .iter()
        .map(|s| (s.clone(), fold(&journal, s)))
        .collect::<BTreeMap<_, _>>());
    input["dateKeys"] = json!(keyed
        .iter()
        .filter(|s| journal.parse(s.trim()).is_some())
        .map(|s| fold(&journal, s))
        .collect::<BTreeSet<_>>());

    let out = Command::new(bridge)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())?;
            child.wait_with_output()
        })
        .expect("cm-bridge runs");
    assert!(
        out.status.success(),
        "cm-bridge failed on {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let model: Value = serde_json::from_slice(&out.stdout).unwrap();

    let mut diffs = Vec::new();
    for field in ["missingKeys", "nonLocal"] {
        for s in model[field].as_array().unwrap() {
            diffs.push(format!("harness: {field} {s}"));
        }
    }

    // Pages: real pages and reference-only pages; alias-only names are not pages.
    let model_pages: BTreeSet<_> = model["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(page_pair)
        .collect();
    let inventory = view.inventory(InventoryScope::All);
    let mut tine_pages = BTreeSet::new();
    let mut tine_conflicts = BTreeSet::new();
    let mut tine_owner = BTreeMap::new();
    for entry in &inventory.0 {
        let kind = if entry.is_journal { "journal" } else { "page" }.to_owned();
        let key = fold(&journal, &entry.name);
        match &entry.target {
            Resolved::Existing { id, others } => {
                if !others.is_empty() {
                    tine_conflicts.insert(key.clone());
                }
                tine_owner.insert((kind.clone(), key.clone()), id.to_string());
                tine_pages.insert((kind, key));
            }
            Resolved::Alias { .. } => {}
            Resolved::Absent { .. } => {
                tine_pages.insert((kind, key));
            }
        }
    }
    for p in model_pages.difference(&tine_pages) {
        diffs.push(format!("pages: model has {p:?}, Tine does not"));
    }
    for p in tine_pages.difference(&model_pages) {
        diffs.push(format!("pages: Tine has {p:?}, model does not"));
    }

    // Conflicts: keys claimed by several files. Owners are compared only off them (O1).
    let model_conflicts: BTreeSet<String> = model["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    if model_conflicts != tine_conflicts {
        diffs.push(format!(
            "conflicts: model {model_conflicts:?}, Tine {tine_conflicts:?}"
        ));
    }
    for o in model["owners"].as_array().unwrap() {
        let page = page_pair(&o["page"]);
        let Some(model_path) = o["path"].as_str() else {
            continue;
        };
        if model_conflicts.contains(&page.1) {
            continue;
        }
        match tine_owner.get(&page) {
            Some(path) if path == model_path => {}
            other => diffs.push(format!(
                "owner of {page:?}: model {model_path}, Tine {other:?}"
            )),
        }
    }

    // Navigation: what [[s]] opens, for every name in play. A conflicted key's
    // target is the model's totalisation of an invalid state (kindPref, O1).
    for (s, page) in model["navigate"].as_object().unwrap() {
        let model_page = page_pair(page);
        if model_conflicts.contains(&model_page.1) {
            continue;
        }
        let tine = navigate(&view, &journal, s);
        if tine != model_page {
            diffs.push(format!(
                "navigate [[{s}]]: model {model_page:?}, Tine {tine:?}"
            ));
        }
    }
    Outcome { name, diffs }
}

#[test]
fn conceptual_model_conformance() {
    let Ok(bridge) = std::env::var("CM_BRIDGE") else {
        eprintln!("CM_BRIDGE unset: conceptual model conformance skipped");
        return;
    };
    let mut paths: Vec<_> = fs::read_dir(FIXTURES)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let outcomes: Vec<Outcome> = paths
        .iter()
        .map(|p| run_fixture(Path::new(&bridge), p))
        .collect();
    let report: BTreeMap<_, _> = outcomes
        .iter()
        .map(|o| (o.name.clone(), o.diffs.clone()))
        .collect();
    let text = serde_json::to_string_pretty(&report).unwrap();
    if let Ok(dest) = std::env::var("CM_REPORT") {
        fs::write(dest, &text).unwrap();
    }
    eprintln!("{text}");
}
