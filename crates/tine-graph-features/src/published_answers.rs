//! The published snapshot's `answers` field, version 1
//! (`docs/contracts/quick-share.md`, "Published answers"; CM step 2, census
//! Q13). A published site answers every name question (what `[[x]]` opens,
//! which file a name reads, the inventory, the alias group, which journals
//! have content, the journal formats) by looking up these tables, which the
//! live answerers filled at publish time over the private build graph
//! (`build_graph.rs`). The viewer computes none of them itself.
//!
//! `v: 1` is frozen once shipped: every `/s/1/` viewer reads it with this
//! meaning forever. A later change to a table bumps `v`; an old viewer then
//! ignores the whole field and reads the snapshot as it did before `answers`.
//!
//! Tables (each row in the live app's wire shape, `inventory_wire.rs`):
//! - `inventory`: `WholeGraph::inventory(All)` rows, plus the synthetic home
//!   page of a multi-page publication (path `""`), which no graph holds.
//! - `resolve`: `WholeGraph::resolve(name, kind)` for a listed name and the
//!   kind its row does not answer (a date spelling asked as a journal, a
//!   journal title asked as a page), only where the answer is a file or an
//!   alias. Every other (listed name, kind) is the row's own target; an
//!   unlisted name is absent.
//! - `alias_groups`: `page_alias_group` of every exported page whose group is
//!   not empty.
//! - `journals`: the effective `:journal/page-title-format` and
//!   `:journal/file-name-format` (the two configuration values a snapshot
//!   carries on purpose: the viewer's date parser needs them) and the days
//!   whose journal has content (`WholeGraph::journal_content_days`).
use crate::inventory_wire::{alias_group_wire, entry_wire, PageInventoryEntryWire, ResolvedWire};
use serde_json::{json, Value};
use std::collections::HashSet;
use tine_core::model::GraphMeta;
use tine_core::refs::page_key;
use tine_core::Corpus;
use tine_store::{Inventory, WholeGraph};

/// The `answers.v` this writer emits.
pub(crate) const ANSWERS_VERSION: u32 = 1;

/// The `answers` value for a snapshot of `corpus` over its build graph
/// `graph` (whose metadata is `meta` and whose `inventory(All)` is
/// `inventory`). `home` is the synthetic home page's name, when the
/// publication has one. Cost O(rows + exported pages × alias group), two
/// O(1) `resolve` calls per row; no I/O.
pub(crate) fn answers(
    graph: &WholeGraph,
    meta: &GraphMeta,
    corpus: &Corpus,
    inventory: &Inventory,
    home: Option<&str>,
) -> Value {
    let mut rows: Vec<PageInventoryEntryWire> = inventory.0.iter().map(entry_wire).collect();
    let mut resolve = Vec::new();
    let answered: HashSet<(String, bool)> = rows
        .iter()
        .map(|row| (row.key.clone(), row.is_journal))
        .collect();
    let mut asked = HashSet::new();
    for row in &rows {
        for is_journal in [false, true] {
            if answered.contains(&(row.key.clone(), is_journal))
                || !asked.insert((row.key.clone(), is_journal))
            {
                continue;
            }
            let target = ResolvedWire::from(graph.resolve(&row.name, is_journal));
            if !matches!(target, ResolvedWire::Absent { .. }) {
                resolve.push(json!({ "name": row.name,
                    "kind": if is_journal { "journal" } else { "page" }, "target": target }));
            }
        }
    }
    if let Some(home) = home {
        // The synthetic home page is a file of this publication only. It
        // replaces a reference-only row of its name (the writers refuse a home
        // named like an exported page or alias).
        let key = page_key(home);
        rows.retain(|row| {
            row.is_journal || row.key != key || matches!(row.target, ResolvedWire::Existing { .. })
        });
        let at = rows
            .partition_point(|row| (row.key.as_str(), row.name.as_str()) <= (key.as_str(), home));
        rows.insert(
            at,
            PageInventoryEntryWire {
                key,
                name: home.to_owned(),
                is_journal: false,
                day: None,
                target: ResolvedWire::Existing {
                    id: String::new(),
                    others: Vec::new(),
                },
                link: None,
                namespace_parent: false,
                empty_alias: false,
            },
        );
    }
    let alias_groups: Vec<Value> = corpus
        .pages
        .iter()
        .filter_map(|page| {
            let group = alias_group_wire(graph, &page.name);
            (!group.aliases.is_empty() || !group.pages.is_empty()).then(
                || json!({ "name": page.name, "aliases": group.aliases, "pages": group.pages }),
            )
        })
        .collect();
    let mut content_days: Vec<i64> = graph
        .journal_content_days()
        .into_iter()
        .map(|day| day.0)
        .collect();
    content_days.sort_unstable();
    content_days.dedup();
    json!({
        "v": ANSWERS_VERSION,
        "inventory": rows,
        "resolve": resolve,
        "alias_groups": alias_groups,
        "journals": {
            "title_format": meta.journal_page_title_format,
            "file_name_format": meta.journal_file_name_format,
            "content_days": content_days,
        },
    })
}
