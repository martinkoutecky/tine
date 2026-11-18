//! The rename dialog's question (`rename_offer`): what renaming a page asks
//! of the user, answered by the rename's own planner (one guard, M-Q6: the
//! window never re-derives it); and the rename with the user's answer
//! (`rename_page_as`). Split from `pages.rs` along this seam
//! (CM step 2 batch 12, PARITY-CAMPAIGN §3).
use std::io;

use tine_store::{PageHost, PageId, Store};

use super::rename_helpers::{self, AliasEdit, Effect, TagUnion};
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
        day @ RenameBlocked::Day { .. } => day_move(store, &graph, old, new, expected_path, day)?,
        RenameBlocked::Contested { name, files } => contested(&graph, name, files),
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
    /// Model r6 O0/O2 refuses a day as a rename source; the window offers to
    /// move the day's notes to the typed name instead ([`DayMove`]).
    DayMove(DayMove),
    /// Several files claim `name` (O3, O4 contested): the window shows them
    /// and offers the file merge into the one the user keeps
    /// (D-20261019-M2(6)). `opens`: the file the name opens now.
    Contested {
        reason: RenameBlocked,
        message: String,
        name: String,
        files: Vec<String>,
        opens: Option<String>,
    },
    /// Refused before anything is written, with the reason to show.
    Refused {
        reason: RenameBlocked,
        message: String,
    },
}

/// The alias dialog (D-20261106-R13-UI, D-20261115-HELP, r6 "Outside the
/// model"): the typed name is only an alias (`reason`), opening `opens`; the
/// component is every page declaring it (`declarers`, the source included
/// when it does) with its alias lines, and the source. Two requests may be
/// offered, each with what it changes:
/// - `merge`: the ordinary request "rename the source to `opens`' name",
///   whose answer is the merge confirm (the dialog's button is that
///   confirm); `mergeRefused` when the model refuses that request.
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
    pub declarers: Vec<DialogPage>,
    pub source: DialogPage,
    pub merge: Option<MergeChoice>,
    pub merge_refused: Option<RenameBlocked>,
    pub own: Option<OwnChoice>,
    pub own_refused: Option<OwnRefusal>,
}

/// The day move (D-20261111-CMDF, -MA, -MOA, D-20261115-HELP): a day keeps
/// its date, so renaming it to a name offers to move its blocks to the page
/// `name` names, by the ordinary cross-page block move. The day keeps its
/// header (its `alias::` line included) and gains one `Moved to [[name]]`
/// block. Read-only; what the dialog shows:
/// - `into`: the existing page that receives the blocks (the merge is then an
///   explicit choice), or None: a new page `name`;
/// - `references`/`reference_count`: the pages whose links to the day keep
///   opening it (they do not follow the notes);
/// - `release` (MOA, `name` is the day's own alias): the alias edit made before
///   the move, so the new page can be opened, and put back by the move itself;
///   `retargeted`: the links to `name` that will open the new page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayMove {
    /// The model's refusal (`Day`, O0/O2) the move is offered instead of.
    pub reason: RenameBlocked,
    pub message: String,
    pub day: DialogPage,
    pub name: String,
    /// The destination is a journal day.
    pub journal: bool,
    pub into: Option<DialogPage>,
    pub references: Vec<String>,
    pub reference_count: usize,
    pub release: Option<AliasEdit>,
    pub retargeted: Option<rename_helpers::Retarget>,
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
/// the merge request's plan, the released rename's plan
/// ([`plan_released_rename`]), and one file read per other declarer. Cost:
/// two plans, plus O(declarers) file reads and the merge's tag reads.
fn alias_dialog(
    store: &Store,
    graph: &tine_store::WholeGraph,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    reason: RenameBlocked,
) -> io::Result<AliasDialog> {
    let RenameBlocked::AliasHeld {
        page: opens_name,
        path: opens_path,
        ..
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
    let declarers: Vec<DialogPage> = owners.iter().map(|id| dialog_page(graph, id)).collect();
    let opens = declarers
        .iter()
        .find(|page| page.path.as_deref() == Some(opens_path.as_str()))
        .cloned()
        .unwrap_or_else(|| dialog_page(graph, &PageId::from(opens_path.clone())));
    // Choice 1: the ordinary request "rename the source to the owner's name".
    let (mut merge, mut merge_refused) = (None, None);
    match rename_offer(store, old, &opens_name, expected_path)? {
        RenameOffer::Merge {
            into,
            name,
            journal,
            tags,
        } if into == opens_path => {
            match plan_rename(
                store,
                old,
                &opens_name,
                expected_path,
                RenameChoice::MergeInto(&into),
                #[cfg(test)]
                &|| {},
            ) {
                Ok(plan) => {
                    // The source's aliases join the owner: their links open
                    // it, their text unchanged.
                    let survivor = PageId::from(into.clone());
                    let own: Vec<&PageId> = source_id.iter().chain([&survivor]).collect();
                    let retargeted = source
                        .aliases
                        .iter()
                        .filter_map(|a| {
                            rename_helpers::retarget(&plan.graph, a, &source.name, &name, &own)
                        })
                        .collect();
                    let effect = rename_helpers::effect(&plan, retargeted);
                    merge = Some(MergeChoice {
                        into,
                        name,
                        journal,
                        tags,
                        effect,
                    });
                }
                Err(e) => merge_refused = RenameBlocked::of(&e).cloned(),
            }
        }
        RenameOffer::Refused { reason, .. } => merge_refused = Some(reason),
        _ => {}
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
                let mine: Vec<&PageId> = owners.iter().chain(source_id.iter()).collect();
                let retargeted =
                    rename_helpers::retarget(&plan.graph, new, &opens.name, new, &mine)
                        .into_iter()
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
        declarers,
        source,
        merge,
        merge_refused,
        own,
        own_refused,
    })
}

/// The day move offer for the refused request `old` (a day) → `new`, or the
/// refusal that stays: the day has no file, `new` is the same day, or `new`
/// opens no single page the notes could go to. The destination is the store's
/// one name answer (`resolve`): a file (the receiving page; several files: the
/// contested dialog), the day's own alias (MOA: a new page), another page's
/// alias (the page it opens), or absent (a new page). Cost: two name
/// resolutions, one referrers lookup, and for MOA one file read.
fn day_move(
    store: &Store,
    graph: &tine_store::WholeGraph,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    reason: RenameBlocked,
) -> io::Result<RenameOffer> {
    let refused = |reason: RenameBlocked| RenameOffer::Refused {
        message: reason.to_string(),
        reason,
    };
    let days = store.config().config.journal_format();
    let same_day = days
        .parse(new)
        .is_some_and(|day| days.parse(old) == Some(day));
    let Some(day_id) = existing(graph.resolve(old, true))
        .into_iter()
        .find(|id| expected_path.is_none_or(|path| id.as_str() == path))
    else {
        return Ok(refused(reason));
    };
    if same_day {
        return Ok(refused(reason));
    }
    let day = dialog_page(graph, &day_id);
    let mut release = None;
    let mut retargeted = None;
    let into = match graph.resolve(new, false) {
        tine_store::Resolved::Existing { id, others } => {
            if !others.is_empty() {
                let mut files: Vec<String> = std::iter::once(&id)
                    .chain(&others)
                    .map(|id| id.as_str().to_owned())
                    .collect();
                files.sort_unstable();
                return Ok(contested(graph, new.to_owned(), files));
            }
            if id == day_id {
                return Ok(refused(reason));
            }
            Some(dialog_page(graph, &id))
        }
        tine_store::Resolved::Alias { owners } => match owners.first() {
            // MOA: the day's own alias becomes a page; the day keeps the line.
            Some(first) if *first == day_id => {
                if owners.len() > 1 {
                    let other = &owners[1];
                    return Ok(refused(RenameBlocked::AliasHeld {
                        alias: new.to_owned(),
                        page: name_of(graph, other).unwrap_or_else(|| other.as_str().to_owned()),
                        path: other.as_str().to_owned(),
                    }));
                }
                match rename_helpers::alias_release(store, graph, &day_id, new)? {
                    Some(edit) => release = Some(edit),
                    None => return Ok(refused(reason)),
                }
                retargeted = rename_helpers::retarget(graph, new, &day.name, new, &[&day_id]);
                None
            }
            Some(first) => Some(dialog_page(graph, first)),
            None => None,
        },
        tine_store::Resolved::Absent { .. } => None,
    };
    let mut referrers = graph.explicit_referrers(std::slice::from_ref(&day.name));
    referrers.retain(|id| *id != day_id);
    let (references, reference_count) = rename_helpers::names(graph, referrers.iter());
    Ok(RenameOffer::DayMove(DayMove {
        journal: into
            .as_ref()
            .map_or_else(|| days.parse(new).is_some(), |page| page.journal),
        name: into
            .as_ref()
            .map_or_else(|| new.to_owned(), |page| page.name.clone()),
        message: reason.to_string(),
        reason,
        day,
        into,
        references,
        reference_count,
        release,
        retargeted,
    }))
}

/// The contested dialog's answer: the claiming files and the one `name`
/// opens now. Cost: one name resolution.
fn contested(graph: &tine_store::WholeGraph, name: String, files: Vec<String>) -> RenameOffer {
    let opens = match graph.resolve(&name, false) {
        tine_store::Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
        _ => None,
    };
    let reason = RenameBlocked::Contested {
        name: name.clone(),
        files: files.clone(),
    };
    RenameOffer::Contested {
        message: reason.to_string(),
        reason,
        name,
        files,
        opens,
    }
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
