//! Shared test adapter for the existing full derived-index oracle.
//! Compiled only for tests / `test-faults`; no writer or injection seam.
use super::*;

fn block_lines(out: &mut String, block: &DocBlock, depth: usize) {
    out.push_str(&format!(
        "  block {depth} {:?} {:?} {}\n",
        block.uuid,
        block.raw(),
        serde_json::to_string(block.projection()).unwrap()
    ));
    for child in &block.children {
        block_lines(out, child, depth + 1);
    }
}

/// The canonical image of one snapshot: what its capture built, then every
/// lazily built index, forced here (referenced names and their per-page
/// index, editor baselines, block hints, aliases, block-ref counts, icons,
/// the query index through queries, backlinks and unlinked references for
/// `probes` and the first pages). Forcing them is what lets a later
/// publication inherit them by per-file deltas.
#[cfg(test)]
pub(crate) fn snapshot_image(snapshot: &ReadSnapshot, probes: &[&str]) -> String {
    image(snapshot, probes, true)
}

fn image(snapshot: &ReadSnapshot, probes: &[&str], include_layout: bool) -> String {
    let config = snapshot.config.clone();
    let mut out = String::new();
    let mut rel_by_slot = HashMap::new();
    let mut pages: Vec<(String, &PageEntry, &Arc<Document>)> = Vec::new();
    for (slot, (entry, doc)) in snapshot.pages.slots() {
        let rel = entry
            .rel_path
            .as_ref()
            .map(|rel| rel.as_str().to_owned())
            .unwrap_or_default();
        rel_by_slot.insert(slot, rel.clone());
        pages.push((rel, entry, doc));
    }
    pages.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, entry, doc) in &pages {
        out.push_str(&format!(
            "page {rel:?} {:?} {:?} pre={:?}\n",
            entry.kind, entry.name, doc.pre_block
        ));
        for block in &doc.roots {
            block_lines(&mut out, block, 0);
        }
        let mut referenced = collect_document_referenced_names(doc);
        referenced.sort();
        out.push_str(&format!("  referenced {referenced:?}\n"));
        out.push_str(&format!(
            "  explicit {:?}\n",
            crate::query::document_explicit_reference_names(entry, doc, &config)
        ));
        let mut counts: Vec<_> = document_block_ref_counts(doc).into_iter().collect();
        counts.sort();
        out.push_str(&format!("  block_ref_counts {counts:?}\n"));
    }
    for (shard, map) in snapshot.explicit_index.shards.iter().enumerate() {
        for (name, paths) in map.iter() {
            let paths: Vec<_> = paths.iter().map(|(path, _)| path).collect();
            out.push_str(&format!("explicit {shard} {name:?} {paths:?}\n"));
        }
    }
    if include_layout {
        let candidates = snapshot.reference_candidate_index.read().unwrap();
        out.push_str(&format!(
            "candidates page_count={} complete={}\n",
            candidates.page_count, candidates.complete
        ));
        let mut signatures: Vec<_> = candidates
            .signatures
            .iter()
            .map(|(slot, signatures)| (rel_by_slot[slot].clone(), signatures))
            .collect();
        signatures.sort_by(|a, b| a.0.cmp(&b.0));
        for (rel, signatures) in signatures {
            out.push_str(&format!(
                "signatures {rel:?} {}\n",
                serde_json::to_string(signatures.as_ref()).unwrap()
            ));
        }
        drop(candidates);
    }
    // Sharded by absolute-path hash; flattened so the image is the same
    // wherever the graph lives.
    let mut aliases: Vec<_> = (snapshot.alias_index.get().unwrap().shards.iter())
        .flat_map(|shard| {
            shard
                .iter()
                .map(|(path, names)| (path.clone(), names.clone()))
        })
        .collect();
    aliases.sort();
    out.push_str(&format!("aliases {aliases:?}\n"));
    out.push_str(&format!(
        "block_ref_counts {}\n",
        serde_json::to_string(snapshot.block_ref_counts().as_ref()).unwrap()
    ));
    out.push_str(&format!(
        "real_page_names {}\n",
        serde_json::to_string(snapshot.real_page_names.as_ref()).unwrap()
    ));
    for (name, keys) in snapshot.transliteration_names.iter() {
        out.push_str(&format!("name_keys {name:?} {keys:?}\n"));
    }
    for (name, owners) in snapshot.transliteration_owners.iter() {
        out.push_str(&format!("name_owners {name:?} {owners}\n"));
    }
    // The icon index keys pages by slot; its slot-free projection is the
    // icon each page name resolves to.
    let names: Vec<String> = pages
        .iter()
        .map(|(_, entry, _)| entry.name.clone())
        .collect();
    let mut icons: Vec<_> = snapshot.page_icons(&names).into_iter().collect();
    icons.sort();
    out.push_str(&format!("icons {icons:?}\n"));
    lazy_lines(&mut out, snapshot, &pages, probes, include_layout);
    out
}

fn lazy_lines(
    out: &mut String,
    snapshot: &ReadSnapshot,
    pages: &[(String, &PageEntry, &Arc<Document>)],
    probes: &[&str],
    include_layout: bool,
) {
    let mut referenced = snapshot.referenced_page_names();
    referenced.sort();
    out.push_str(&format!("lazy referenced_page_names {referenced:?}\n"));
    let index = snapshot
        .referenced_name_index
        .get()
        .expect("referenced_page_names builds the index");
    for (rel, entry, _) in pages {
        let mut names = index.get(&entry.path).cloned();
        if let Some(names) = names.as_mut() {
            names.sort();
        }
        out.push_str(&format!("lazy referenced_index {rel:?} {names:?}\n"));
    }
    let stride = (pages.len() / 8).max(1);
    for (rel, entry, doc) in pages.iter().step_by(stride) {
        if let Some(block) = doc.roots.first() {
            let original = format!("[[GJ5 Baseline]] [[孤独基线]] {}", block.raw());
            let mut names =
                snapshot.referenced_page_names_for_editor(&entry.name, &block.uuid, &original);
            names.sort();
            out.push_str(&format!("lazy editor {rel:?} {names:?}\n"));
        }
    }
    fn hints(out: &mut String, snapshot: &ReadSnapshot, block: &DocBlock) {
        out.push_str(&format!(
            "lazy hint {:?} {:?}\n",
            block.uuid,
            snapshot.block_page_hint(&block.uuid)
        ));
        for child in &block.children {
            hints(out, snapshot, child);
        }
    }
    if include_layout {
        for (_, _, doc) in pages {
            for block in &doc.roots {
                hints(out, snapshot, block);
            }
        }
    }
    let mut aliases = snapshot.page_aliases_with_owners();
    aliases.sort();
    out.push_str(&format!("lazy aliases {aliases:?}\n"));
    let mut spellings = snapshot.page_alias_spellings_with_owners();
    spellings.sort();
    out.push_str(&format!("lazy alias_spellings {spellings:?}\n"));
    // The alias relation (edges, owner paths, list) as one image; the delta
    // that patches it must equal the build that walks every page.
    out.push_str(&format!(
        "lazy alias_relation\n{}",
        snapshot.alias_relation().image()
    ));
    let mut counts: Vec<_> = snapshot
        .public_block_ref_counts()
        .iter()
        .map(|(uuid, count)| (uuid.clone(), *count))
        .collect();
    counts.sort();
    out.push_str(&format!("lazy public_block_ref_counts {counts:?}\n"));
    let groups = |result: BoundedRefGroups| {
        format!(
            "{} {} {}",
            result.total,
            result.exceeded,
            serde_json::to_string(result.groups.as_ref()).unwrap()
        )
    };
    let (rows, bytes) = (
        crate::store::RESULT_BRIDGE_MAX_ROWS,
        crate::store::RESULT_BRIDGE_MAX_BYTES,
    );
    for query in QUERIES {
        out.push_str(&format!(
            "lazy query {query:?} {}\n",
            groups(snapshot.run_query_bounded(query, rows, bytes))
        ));
    }
    if !include_layout {
        // F3 observes the names the operation actually changes, before and
        // after publication. Keep the original F2/F5 layout oracle unchanged.
        for name in probes {
            let query = format!("[[{name}]]");
            out.push_str(&format!(
                "probe simple {query:?} {}\n",
                groups(snapshot.run_query_bounded(&query, rows, bytes))
            ));
            let quoted = format!("'{}'", name.replace('\'', "''"));
            for source in [
                format!("@block and [[{name}]]"),
                format!("@page and any(links, name = {quoted})"),
                format!("@page and any(linked_by, name = {quoted})"),
                // Fileless Before/After can be link elements and seeds, but
                // cannot be returned @page rows. Observe their incoming edges
                // inside links as well as physical targets' direct linked_by.
                format!("@page and any(links, name = {quoted} and any(linked_by, true))"),
                format!("@page and near(name = {quoted}, 1, 'in')"),
                format!("@page and near(name = {quoted}, 1, 'out')"),
                format!("@page and near(name = {quoted}, 1)"),
            ] {
                use tine_core::query::ir::ExecutionContext;
                use tine_core::query::registry::Registry;
                use tine_core::query::{parse_query_input, QueryInput};
                let (query, view) = parse_query_input(
                    &source,
                    QueryInput::Tql,
                    tine_core::date::JournalDate::today(),
                    Registry::none(),
                );
                assert!(!query.is_invalid(), "{source}: {:?}", query.diagnostics);
                let crate::IrAnswer::Result(result) = crate::query::exec::query_ir(
                    snapshot,
                    crate::IrRequest::Run {
                        query: &query,
                        view: &view,
                        context: &ExecutionContext::default(),
                    },
                    &|| false,
                )
                .unwrap()
                .expect("never cancelled") else {
                    panic!("Run must return rows");
                };
                assert!(!result.exceeded, "{source}");
                out.push_str(&format!(
                    "probe tql {source:?} {} {}\n",
                    result.total,
                    serde_json::to_string(&result.rows).unwrap()
                ));
            }
        }
    }
    let targets = probes.iter().map(|name| (*name).to_owned()).chain(
        pages
            .iter()
            .take(20)
            .map(|(_, entry, _)| entry.name.clone()),
    );
    for name in targets {
        out.push_str(&format!(
            "lazy backlinks {name:?} {}\n",
            groups(snapshot.backlinks_bounded(&name, rows, bytes))
        ));
        out.push_str(&format!(
            "lazy unlinked {name:?} {}\n",
            groups(snapshot.unlinked_refs_bounded(&name, rows, bytes))
        ));
    }
}

/// Queries over every fact class the query index keeps: tasks, page tags,
/// page and block properties, page references (Han included).
const QUERIES: &[&str] = &[
    "(task TODO)",
    "(task TODO DONE)",
    "(page-tags hub0)",
    "(page-tags gj5tag)",
    "(property status open)",
    "(property custom_key)",
    "(property gj5key)",
    "(page-property tags hub1)",
    "[[Shared]]",
    "[[Topic 3]]",
    "[[中文页面]]",
    "[[孤独测试]]",
    "[[GJ5 Probe]]",
    "(and (task TODO) [[Shared]])",
    "(or [[GJ5 Probe]] #gj5edited)",
];

#[cfg(feature = "test-faults")]
impl crate::Store {
    /// Full F2/F5 snapshot oracle reused by integration tests. Runtime revisions,
    /// allocation slots and graph-root spelling are not semantic state.
    pub fn conformance_image_for_tests(&self, probes: &[&str]) -> String {
        let view = self.whole_graph().unwrap();
        // The original F2 oracle also compares optimization layout. Restore
        // invalidates candidate signatures (safe full-scan fallback), and
        // runtime block hints are optional. Compare their semantic consumers
        // instead; retain the original exact-layout oracle for its old tests.
        let mut image = image(&view.graph, probes, false);
        fn previews(view: &crate::WholeGraph, out: &mut String, blocks: &[DocBlock]) {
            for block in blocks {
                out.push_str(&format!(
                    "preview {:?} {}\n",
                    block.uuid,
                    serde_json::to_string(&view.preview_block(&block.uuid, 10000).unwrap())
                        .unwrap()
                ));
                previews(view, out, &block.children);
            }
        }
        let mut pages: Vec<_> = view.graph.pages.iter().collect();
        pages.sort_by(|a, b| a.0.rel_path.cmp(&b.0.rel_path));
        for (_, doc) in &pages {
            previews(&view, &mut image, &doc.roots);
        }
        // Merge intentionally preserves authored blocks' runtime locators;
        // a cold parse assigns locators at their new structural positions.
        // Persisted id properties and reference targets remain byte-exact.
        fn ids(image: &mut String, blocks: &[DocBlock], prefix: &str) {
            for (i, block) in blocks.iter().enumerate() {
                let path = format!("{prefix}/{i}");
                *image = image.replace(&block.uuid, &format!("<runtime {path}>"));
                ids(image, &block.children, &path);
            }
        }
        for (_, (entry, doc)) in view.graph.pages.slots() {
            ids(
                &mut image,
                &doc.roots,
                entry.rel_path.as_ref().unwrap().as_str(),
            );
        }
        let mut listing: Vec<_> = view
            .list
            .iter()
            .map(|entry| format!("{:?} {:?} {:?}", entry.rel_path, entry.name, entry.kind))
            .collect();
        listing.sort();
        image.push_str(&format!("listing {listing:?}\n"));
        let mut days: Vec<_> = view
            .journal_content_days()
            .iter()
            .map(|day| day.0)
            .collect();
        days.sort();
        image.push_str(&format!("journal_days {days:?}\n"));
        let root = self.graph.root.to_string_lossy();
        let image = image
            .replace(&*root, "<root>")
            .replace(&root.replace('\\', "\\\\"), "<root>");
        // F2/F5: a patched referenced-name set may retain a different case
        // representative from a cold build. Compare that set case-folded.
        image
            .lines()
            .map(|line| {
                if [
                    "lazy referenced_page_names ",
                    "lazy referenced_index ",
                    "lazy editor ",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
                {
                    let (Some(open), Some(close)) = (line.find('['), line.rfind(']')) else {
                        return line.to_owned();
                    };
                    let mut names: Vec<_> = line[open + 1..close]
                        .split(", ")
                        .map(str::to_lowercase)
                        .collect();
                    names.sort();
                    format!(
                        "{}[{}]{}",
                        &line[..open],
                        names.join(", "),
                        &line[close + 1..]
                    )
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
