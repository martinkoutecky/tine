//! The wire form of the store's name answers: one `page_inventory` row, one
//! name's resolution and one page's alias group. The live app's commands
//! (`src-tauri/src/commands/discovery.rs`) and the published snapshot's
//! `answers` table (`published_answers.rs`, `docs/contracts/quick-share.md`)
//! serialize through these same types, so a published site reads exactly the
//! rows the app reads (I-12: one serializer per answer).
use serde::Serialize;
use tine_store::{InventoryEntry, InventoryScope, PageId, Resolved, WholeGraph};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedWire {
    Existing { id: String, others: Vec<String> },
    Alias { owners: Vec<String> },
    Absent { id: String },
}

impl From<Resolved> for ResolvedWire {
    fn from(value: Resolved) -> Self {
        Self::from(&value)
    }
}

impl From<&Resolved> for ResolvedWire {
    fn from(value: &Resolved) -> Self {
        let id = |id: &PageId| id.as_str().to_owned();
        match value {
            Resolved::Existing { id: first, others } => Self::Existing {
                id: id(first),
                others: others.iter().map(id).collect(),
            },
            Resolved::Alias { owners } => Self::Alias {
                owners: owners.iter().map(id).collect(),
            },
            Resolved::Absent { id: absent } => Self::Absent { id: id(absent) },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PageInventoryEntryWire {
    /// `refs::page_key(name)`: the frontend looks names up by this key only.
    pub key: String,
    pub name: String,
    pub is_journal: bool,
    pub day: Option<i64>,
    pub target: ResolvedWire,
    /// What a `[[name]]` link opens when that is not this entry itself
    /// (`WholeGraph::link_target`): sent only then, so most entries cost no
    /// bytes. `PageRef` routes by it; the frontend decides nothing itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkWire>,
    /// Listed only because a listed name lies under it (`n` for `n/y`):
    /// sent only when true. The namespace views read it; none re-derives
    /// prefixes.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub namespace_parent: bool,
    /// Canonical typed immediate parent, independently of alias navigation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<LinkWire>,
    /// Retired live flag (always false). Frozen published v1 rows retain
    /// their stored content-dependent opening answer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub empty_alias: bool,
    /// The name has a `/` but is no namespace name (`./x`, `../x/y`, a URL
    /// with an origin: `refs::is_namespace_name`, model round 10 S15), so no
    /// namespace view splits or nests it. Sent only when true; the views read
    /// it and test no spelling themselves.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_namespace: bool,
    /// R10-Q2: this spelling is only an alias, outside the page universe.
    /// Keep it in completion/navigation, omit it from every namespace view.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub namespace_excluded: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LinkWire {
    pub name: String,
    pub kind: &'static str,
}

/// One inventory row in its wire shape.
pub fn entry_wire(entry: &InventoryEntry) -> PageInventoryEntryWire {
    PageInventoryEntryWire {
        key: tine_core::refs::page_key(&entry.name),
        name: entry.name.clone(),
        is_journal: entry.is_journal,
        day: entry.day.map(|day| day.0),
        target: ResolvedWire::from(&entry.target),
        link: entry.link.as_ref().map(|link| LinkWire {
            name: link.name.clone(),
            kind: if link.is_journal { "journal" } else { "page" },
        }),
        namespace_parent: entry.namespace_parent,
        parent: entry.parent.as_ref().map(|parent| LinkWire {
            name: parent.name.clone(),
            kind: if parent.is_journal { "journal" } else { "page" },
        }),
        empty_alias: entry.empty_alias,
        not_namespace: entry.name.contains('/') && !tine_core::refs::is_namespace_name(&entry.name),
        namespace_excluded: matches!(entry.target, Resolved::Alias { .. }),
    }
}

/// The page header's alias group (D-20261019-AL2): the other names with no
/// file of their own (a link opens an owner), and the other names a file
/// claims (a link opens that file).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AliasGroupWire {
    pub aliases: Vec<String>,
    pub pages: Vec<String>,
}

pub fn alias_group_wire(view: &WholeGraph, name: &str) -> AliasGroupWire {
    let (mut aliases, mut pages) = (Vec::new(), Vec::new());
    for entry in &view.inventory(InventoryScope::AliasGroup(name)).0 {
        match entry.target {
            Resolved::Existing { .. } => pages.push(entry.name.clone()),
            _ => aliases.push(entry.name.clone()),
        }
    }
    AliasGroupWire { aliases, pages }
}
