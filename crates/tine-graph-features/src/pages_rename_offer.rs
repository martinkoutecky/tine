//! The rename dialog's question (`rename_offer`): what renaming a page asks
//! of the user, answered by the rename's own planner (one guard, M-Q6: the
//! window never re-derives it); and the rename with the user's answer
//! (`rename_page_as`). Split from `pages.rs` along this seam
//! (CM step 2 batch 12, PARITY-CAMPAIGN §3).
use std::io;

use tine_store::{PageHost, PageId, Store};

use super::rename_plan::{plan_rename, RenameBlocked, RenameChoice};
use super::{error, refreshed_view, rename_page_after_inventory, view, RenameReport};

/// What renaming `old` to `new` asks of the user, decided by the rename's own
/// planner (one guard: the dialog never re-derives it). See [`RenameOffer`].
/// Read-only: one plan from the published view, and after a refusal one
/// page-stamps walk and a second plan, as the rename itself decides a
/// refusal only on a walked view. Cost: the rename's planning cost (O(P)
/// with the walk).
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
    if old.is_empty() || old == new {
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
            RenameOffer::Merge {
                into,
                name,
                journal,
            }
        }
        RenameBlocked::AliasOf { owner, merge } => RenameOffer::AliasOf {
            name: entry(&owner).map_or_else(|| owner.clone(), |(name, _)| name),
            owner,
            merge,
        },
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
    /// (`into` its file, `name` its own name).
    Merge {
        into: String,
        name: String,
        journal: bool,
    },
    /// M-Q5 (D-20261102-MC5): the name is only an alias of `owner` (its
    /// file; `name` its own name). Ask: merge into it (when `merge`), make
    /// the name its own page (still an alias of the owner), or cancel.
    AliasOf {
        owner: String,
        name: String,
        merge: bool,
    },
    /// Refused before anything is written, with the reason to show.
    Refused {
        reason: RenameBlocked,
        message: String,
    },
}

/// [`super::rename_or_merge_page`] with the user's answer to the rename question
/// ([`rename_offer`]) as a [`RenameChoice`]; `RenameChoice::OwnPage` is
/// M-Q5's "make it its own page": the new name is only an alias of another
/// page, the renamed page moves to that name as a plain rename, and the
/// owner's `alias::` stays (D-20261102-MC5). Every refusal the user can act
/// on carries a [`RenameBlocked`] (`RenameBlocked::of`).
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
