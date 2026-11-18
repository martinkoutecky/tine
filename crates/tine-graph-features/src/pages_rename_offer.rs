//! The rename dialog's question (`rename_offer`): what renaming a page asks
//! of the user, answered by the rename's own planner (one guard, M-Q6: the
//! window never re-derives it); and the rename with the user's answer
//! (`rename_page_as`). Split from `pages.rs` along this seam
//! (CM step 2 batch 12, PARITY-CAMPAIGN §3).
use std::io;

use tine_core::refs;
use tine_store::{PageHost, PageId, Store};

use super::rename_helpers::{self, AliasEdit, Effect, Request, TagUnion};
use super::rename_plan::{name_of, plan_released_rename, plan_rename, RenameBlocked, RenameChoice};
use super::{error, existing, refreshed_view, rename_page_after_inventory, view, RenameReport};

/// What renaming `old` to `new` asks of the user, decided by the rename's own
/// planner (one guard: the dialog never re-derives it). See [`RenameOffer`].
/// Read-only: one plan from the published view, and after a refusal one
/// page-stamps walk and a second plan, as the rename itself decides a
/// refusal only on a walked view. Cost: the rename's planning cost (O(P)
/// with the walk); a merge adds the two files' reads for its tag union, and
/// an alias-held name the alias dialog's reads ([`alias_dialog`]).
pub fn rename_offer(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
) -> io::Result<RenameOffer> {
    let (old, new) = (old.trim(), new.trim());
    if new.is_empty() {
        return Err(error(io::ErrorKind::InvalidInput, "empty name"));
    }
    if old.is_empty() {
        return Ok(RenameOffer::Rename);
    }
    let plan = || {
        plan_rename(
            store,
            old,
            new,
            expected_path,
            RenameChoice::Rename,
            #[cfg(test)]
            &|| {},
        )
    };
    let planned = match plan() {
        Ok(planned) => Ok(planned),
        Err(_) => {
            refreshed_view(store)?;
            plan()
        }
    };
    let blocked = match planned {
        Ok(_) => return Ok(RenameOffer::Rename),
        Err(e) => match RenameBlocked::of(&e) {
            Some(blocked) => blocked.clone(),
            None => return Err(e),
        },
    };
    let graph = view(store)?;
    let entry = |path: &str| {
        graph
            .inventory(tine_store::InventoryScope::File(&PageId::from(
                path.to_owned(),
            )))
            .0
            .first()
            .map(|entry| (entry.name.clone(), entry.is_journal))
    };
    Ok(match blocked {
        RenameBlocked::Taken { into } => {
            let (name, journal) = entry(&into).unwrap_or_else(|| (new.to_owned(), false));
            let source = existing(graph.resolve(old, false))
                .into_iter()
                .find(|id| expected_path.is_none_or(|path| id.as_str() == path));
            let tags = match source {
                Some(source) => {
                    rename_helpers::tag_union(store, &PageId::from(into.clone()), &source)?
                }
                None => None,
            };
            RenameOffer::Merge {
                into,
                name,
                journal,
                tags,
            }
        }
        held @ RenameBlocked::AliasHeld { .. } => {
            RenameOffer::Alias(alias_dialog(store, &graph, old, new, expected_path, held)?)
        }
        refused => RenameOffer::Refused {
            message: refused.to_string(),
            reason: refused,
        },
    })
}

/// The rename question's answer ([`rename_offer`]), serialized for the
/// window's dialog (`kind` tag, camelCase).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RenameOffer {
    /// Rename without asking (the name is free or the page's own).
    Rename,
    /// The name is one other page's or journal day's: ask to merge into it
    /// (`into` its file, `name` its own name). `tags`: both files have a
    /// `tags::` line, which the window unites before the merge.
    Merge {
        into: String,
        name: String,
        journal: bool,
        tags: Option<TagUnion>,
    },
    /// Model r6 O3 refuses the name: it is only another page's alias. The
    /// alias dialog shows its component and the requests the model allows.
    Alias(AliasDialog),
    /// Refused before anything is written, with the reason to show.
    Refused {
        reason: RenameBlocked,
        message: String,
    },
}

/// The alias dialog (D-20261106-R13-UI, D-20261115-HELP, r6 "Outside the
/// model"): the typed name is only an alias (`reason`), opening `opens`; the
/// component is every page of the name's alias group (`InventoryScope::
/// AliasGroup`, the declarers and the pages linked to them through other
/// aliases, the source included when it is one) with its alias lines, and
/// the source. The requests offered, each with what it changes:
/// - `merges`: for each component page other than the source, the ordinary
///   request "rename the source to that page's name", whose answer is the
///   merge confirm (the dialog's button is that confirm); `mergesRefused`
///   names each component page whose merge the model refuses, and why.
/// - `own`: "use the name for this page": the ordinary edits that drop the
///   alias from every other declarer, then the plain rename to `alias`;
///   `ownRefused` says why it is not offered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasDialog {
    /// The model's refusal of the typed request (O3), and its words.
    pub reason: RenameBlocked,
    pub message: String,
    pub opens: DialogPage,
    pub component: Vec<DialogPage>,
    pub source: DialogPage,
    pub merges: Vec<MergeChoice>,
    pub merges_refused: Vec<RefusedMerge>,
    pub own: Option<OwnChoice>,
    pub own_refused: Option<OwnRefusal>,
}

/// A component page the source cannot be merged into, with the model's
/// refusal of that request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RefusedMerge {
    pub name: String,
    pub path: String,
    pub reason: RenameBlocked,
}

/// One page of the alias component: its name, file (None for a name with
/// no file), kind and the aliases it declares.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DialogPage {
    pub name: String,
    pub path: Option<String>,
    pub journal: bool,
    pub aliases: Vec<String>,
}

/// The merge request the alias dialog offers: as [`RenameOffer::Merge`],
/// with what it changes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MergeChoice {
    pub into: String,
    pub name: String,
    pub journal: bool,
    pub tags: Option<TagUnion>,
    pub effect: Effect,
}

/// "Use the name for this page": the alias edits, then the rename's effect.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OwnChoice {
    pub edits: Vec<AliasEdit>,
    pub effect: Effect,
}

/// Why "use the name for this page" is not offered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OwnRefusal {
    /// The rename after the edits would be refused for `reason`.
    Plan { reason: RenameBlocked },
    /// `page`'s alias line holding the name cannot be edited without losing
    /// other text (several lines hold it, its key repeats, or its value has
    /// text besides names): the user edits it.
    AliasLine { page: String, path: String },
    /// The rename after the edits would fail for a reason the dialog cannot
    /// word; the rename itself would report it.
    Other,
}

fn dialog_page(graph: &tine_store::WholeGraph, id: &PageId) -> DialogPage {
    let entry = graph
        .inventory(tine_store::InventoryScope::File(id))
        .0
        .first()
        .cloned();
    let aliases = graph
        .inventory(tine_store::InventoryScope::AliasesOf(id))
        .0
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    DialogPage {
        name: entry
            .as_ref()
            .map_or_else(|| id.as_str().to_owned(), |e| e.name.clone()),
        path: Some(id.as_str().to_owned()),
        journal: entry.is_some_and(|e| e.is_journal),
        aliases,
    }
}

/// Build the alias dialog for the refused request `old` → `new` (O3's
/// `held`: the alias, the page it opens and that page's file). Read-only:
/// the component from the alias-group inventory, each merge request's plan,
/// the released rename's plan ([`plan_released_rename`]), and one file read
/// per other declarer. Cost: two plans per component page other than the
/// source plus one, O(declarers) file reads and each merge's tag reads.
fn alias_dialog(
    store: &Store,
    graph: &tine_store::WholeGraph,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    reason: RenameBlocked,
) -> io::Result<AliasDialog> {
    let RenameBlocked::AliasHeld {
        path: opens_path, ..
    } = reason.clone()
    else {
        unreachable!("the alias dialog answers O3's alias-held refusal only");
    };
    let owners = match graph.resolve(new, false) {
        tine_store::Resolved::Alias { owners } => owners,
        _ => Vec::new(),
    };
    let source_id = existing(graph.resolve(old, false))
        .into_iter()
        .find(|id| expected_path.is_none_or(|path| id.as_str() == path));
    let source = match &source_id {
        Some(id) => dialog_page(graph, id),
        None => DialogPage {
            name: old.to_owned(),
            path: None,
            journal: false,
            aliases: Vec::new(),
        },
    };
    // The component: every file of the name's alias group (its declarers
    // included), once, by path.
    let mut ids: Vec<PageId> = graph
        .inventory(tine_store::InventoryScope::AliasGroup(new))
        .0
        .iter()
        .filter_map(|entry| match &entry.target {
            tine_store::Resolved::Existing { id, .. } => Some(id.clone()),
            _ => None,
        })
        .chain(owners.iter().cloned())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let component: Vec<DialogPage> = ids.iter().map(|id| dialog_page(graph, id)).collect();
    let opens = component
        .iter()
        .find(|page| page.path.as_deref() == Some(opens_path.as_str()))
        .cloned()
        .unwrap_or_else(|| dialog_page(graph, &PageId::from(opens_path.clone())));
    // Choice 1, per component page: the ordinary request "rename the source
    // to that page's name".
    let (mut merges, mut merges_refused) = (Vec::new(), Vec::new());
    for target in component
        .iter()
        .filter(|page| page.path.is_some() && page.path != source.path)
    {
        let path = target.path.clone().unwrap_or_default();
        match rename_offer(store, old, &target.name, expected_path)? {
            RenameOffer::Merge {
                into,
                name,
                journal,
                tags,
            } if into == path => match plan_rename(
                store,
                old,
                &name,
                expected_path,
                RenameChoice::MergeInto(&into),
                #[cfg(test)]
                &|| {},
            ) {
                Ok(plan) => {
                    let survivor = PageId::from(into.clone());
                    let retargeted = source_id.iter().flat_map(|source_id| {
                        let request = Request::Merge {
                            source: source_id,
                            survivor: &survivor,
                            name: &name,
                        };
                        source
                            .aliases
                            .iter()
                            .filter_map(|alias| {
                                rename_helpers::retarget(&plan.graph, alias, &request)
                            })
                            .collect::<Vec<_>>()
                    });
                    let effect = rename_helpers::effect(&plan, retargeted.collect());
                    merges.push(MergeChoice {
                        into,
                        name,
                        journal,
                        tags,
                        effect,
                    });
                }
                Err(e) => {
                    if let Some(reason) = RenameBlocked::of(&e) {
                        merges_refused.push(RefusedMerge {
                            name: target.name.clone(),
                            path,
                            reason: reason.clone(),
                        });
                    }
                }
            },
            RenameOffer::Refused { reason, .. } => merges_refused.push(RefusedMerge {
                name: target.name.clone(),
                path,
                reason,
            }),
            _ => {}
        }
    }
    // Choice 2: drop the alias from every other declarer, then rename.
    let mut edits = Vec::new();
    let mut own_refused = None;
    for id in owners.iter().filter(|id| Some(*id) != source_id.as_ref()) {
        match rename_helpers::alias_release(store, graph, id, new)? {
            Some(edit) => edits.push(edit),
            None => {
                own_refused = Some(OwnRefusal::AliasLine {
                    page: name_of(graph, id).unwrap_or_else(|| id.as_str().to_owned()),
                    path: id.as_str().to_owned(),
                });
                break;
            }
        }
    }
    let mut own = None;
    if own_refused.is_none() {
        match plan_released_rename(store, old, new, expected_path) {
            Ok(plan) if !plan.unchanged => {
                // A file-less source has no file to move: an id no file has.
                let (from, to) = match &source_id {
                    Some(id) => (
                        id.clone(),
                        plan.moves
                            .get(id)
                            .map_or_else(|| id.clone(), |to| PageId::from(to.as_str())),
                    ),
                    None => (PageId::from(""), PageId::from("")),
                };
                let request = Request::Own {
                    source: &from,
                    to: &to,
                    name: new,
                };
                let retargeted = std::iter::once(new)
                    .chain(
                        source
                            .aliases
                            .iter()
                            .map(String::as_str)
                            .filter(|alias| !refs::same_page(alias, new)),
                    )
                    .filter_map(|name| rename_helpers::retarget(&plan.graph, name, &request))
                    .collect();
                let mut effect = rename_helpers::effect(&plan, retargeted);
                for edit in &edits {
                    if !effect.pages.contains(&edit.page) {
                        effect.page_count += 1;
                        if effect.pages.len() < 8 {
                            effect.pages.push(edit.page.clone());
                        }
                    }
                }
                own = Some(OwnChoice { edits, effect });
            }
            Ok(_) => own_refused = Some(OwnRefusal::Other),
            Err(e) => {
                own_refused = Some(match RenameBlocked::of(&e) {
                    Some(reason) => OwnRefusal::Plan {
                        reason: reason.clone(),
                    },
                    None => OwnRefusal::Other,
                })
            }
        }
    }
    Ok(AliasDialog {
        message: reason.to_string(),
        reason,
        opens,
        component,
        source,
        merges,
        merges_refused,
        own,
        own_refused,
    })
}

/// [`super::rename_or_merge_page`] with the user's answer to the rename question
/// ([`rename_offer`]) as a [`RenameChoice`]. Planner, file-context, check and
/// realization refusals retain their types inside the returned I/O error.
pub fn rename_page_as(
    store: &Store,
    host: &PageHost,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    choice: RenameChoice<'_>,
    unsaved_paths: &[String],
) -> io::Result<RenameReport> {
    rename_page_after_inventory(
        store,
        host,
        old,
        new,
        expected_path,
        choice,
        unsaved_paths,
        #[cfg(test)]
        || {},
    )
}
