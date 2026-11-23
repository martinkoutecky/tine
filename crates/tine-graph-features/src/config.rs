//! Graph config edits. Each operation reads one Meta file and commits one guarded
//! create or replace; conflicts retry with fresh bytes at most four times.
//! File work: O(config.edn bytes) per attempt. Settings-only changes retain
//! parsed pages; publication may still enumerate names (reading page preambles)
//! and rebuild in-memory indexes. Changing a build-key field also re-reads and
//! re-parses the graph.
//! Errors are I/O errors; exhausted conflicts return WouldBlock. Callers need
//! no graph path, lock, or cache state.
//! A known config read problem needs no reread; fresh read failures carry the
//! same typed ConfigUnreadable family as the transaction guard.

use std::io;

use tine_core::config::{edit_entry, edit_is_complete, editable_value, read_for_edit};
use tine_core::edn::{Form, Kind};

use tine_store::{Area, Content, FileId, FileRev, Store, StoreError};

use crate::store_error;

fn config_error(error: StoreError) -> io::Error {
    store_error(error)
}

fn config_id(store: &Store) -> io::Result<FileId> {
    store
        .file_id(Area::Meta, "config.edn")
        .map_err(config_error)
}

fn read_config(store: &Store, id: &FileId) -> io::Result<Option<(String, FileRev)>> {
    match crate::parsed_text::read(store, id) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Refusal (external-editor race / sync delivery): a config.edn whose first
/// form is not a balanced map (truncated mid-delivery, half-saved by an
/// editor, not a map at all) is never rewritten by a settings change, and no
/// edit may produce one. A blank or comment-only file is not malformed.
/// Contract: docs/contracts/config-live-reload.md, refusal table.
fn check_root(text: &str) -> io::Result<()> {
    if !edit_is_complete(text) {
        return Err(refuse("the top-level form is not a balanced map"));
    }
    Ok(())
}

fn update(store: &Store, edit: impl Fn(&str) -> io::Result<String>) -> io::Result<()> {
    if let Some(problem) = store.config().problem {
        return Err(crate::config_unreadable(problem));
    }
    let id = config_id(store)?;
    crate::retry_on_conflict("config changed repeatedly during update", || {
        let current =
            read_config(store, &id).map_err(|error| crate::config_unreadable(error.into()))?;
        let text = current.as_ref().map_or("{}\n", |(text, _)| text);
        let next = edit(text)?;
        if next == text {
            return Ok(Some(()));
        }
        check_root(&next)?;
        let mut tx = store.transaction(None);
        if let Some((_, rev)) = current {
            tx.replace(&id, rev, next.into_bytes());
        } else {
            tx.create(&id, Content::Bytes(next.into_bytes()));
        }
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

/// Return bounded custom.css text, or empty text only for a missing file.
/// Read/UTF-8/size failures propagate: callers apply no CSS and report once
/// per graph open. Cost O(custom.css bytes), capped by PARSE_INPUT_MAX_BYTES.
pub fn custom_css(store: &Store) -> io::Result<String> {
    let id = store
        .file_id(Area::Meta, "custom.css")
        .map_err(store_error)?;
    match crate::parsed_text::read(store, &id) {
        Ok((text, _)) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// Persist the favorites list to `:favorites [...]`, replacing the existing
/// value whatever its shape (vector, `nil`, …) or inserting the key into the
/// top-level map, preserving the rest of the file. `page`, when given, is
/// recorded as `:tine/favorites-page "Name"` in the SAME guarded write, so
/// membership and the arrangement page's name never land apart; `None` leaves
/// that key untouched (it cannot clear it). Conflicts retry with fresh bytes
/// up to four times, then `WouldBlock`; a missing config.edn is created. A file
/// whose value or top-level form cannot be edited safely is refused
/// (`InvalidData`) rather than written unparsable.
pub fn set_favorites(store: &Store, names: &[String], page: Option<&str>) -> io::Result<()> {
    update(store, |content| edit_favorites(content, names, page))
}

/// The `:favorites` (and, when given, `:tine/favorites-page`) edit on config
/// text, shared by the favorites writer and a page rename's config step.
/// Refuses (`InvalidData`) a value or top-level form it cannot edit safely.
pub(crate) fn edit_favorites(
    source: &str,
    names: &[String],
    page: Option<&str>,
) -> io::Result<String> {
    let mut content = source.to_string();
    if let Some(page) = page {
        set_top_level(
            &mut content,
            "tine/favorites-page",
            &edn_string(page),
            false,
        )?;
    }
    let names: Vec<String> = names.iter().map(|n| edn_string(n)).collect();
    set_top_level(
        &mut content,
        "favorites",
        &format!("[{}]", names.join(" ")),
        false,
    )?;
    Ok(content)
}

/// Set or clear the graph's Logseq `:default-home {:page "Name"}` value.
/// Sibling entries (including `:sidebar`) and unrelated bytes are preserved.
/// A malformed root or non-map `:default-home` is refused with InvalidData;
/// storage conflicts retry through the usual guarded config transaction.
/// Cost: O(config.edn bytes) per attempt, up to four attempts.
pub fn set_default_home_page(store: &Store, name: Option<&str>) -> io::Result<()> {
    update(store, |source| edit_default_home(source, name))
}

/// The `:default-home {:page …}` edit on config text, shared by the Settings
/// picker and a page rename (OG `rename-page-aux` moves the home page with its
/// page). Refuses a malformed root or a non-map `:default-home`.
pub(crate) fn edit_default_home(source: &str, name: Option<&str>) -> io::Result<String> {
    let mut content = source.to_owned();
    let name = name.map(str::trim).filter(|name| !name.is_empty());
    let root = edit_map(source)?;
    let home = root
        .as_ref()
        .and_then(|root| edit_entry(root, "default-home"))
        .map(|(_, value)| {
            value
                .filter(|form| form.kind == Kind::Map)
                .ok_or_else(|| refuse(":default-home requires a map"))
        })
        .transpose()?;
    match (home, name) {
        (None, Some(name)) => {
            let value = format!("{{:page {}}}", edn_string(name));
            if let Some(root) = root {
                insert_key(&mut content, &root, "default-home", &value, true);
            } else {
                set_top_level(&mut content, "default-home", &value, false)?;
            }
        }
        (Some(map), Some(name)) => set_nested(&mut content, map, "page", &edn_string(name))?,
        (Some(map), None) => remove_entry(&mut content, map, "page")?,
        (None, None) => {}
    }
    Ok(content)
}

/// What a page rename (or an observed page-name change) does to
/// `config.edn`: the ONE answer for `:default-home`, `:favorites` and
/// `:tine/favorites-page` after it (CM step 2, census Q12). `write` is the
/// guarded replacement `(id, base rev, bytes)` when anything moved.
#[derive(Debug, Default)]
pub(crate) struct ConfigAfterRename {
    pub(crate) write: Option<(FileId, FileRev, Vec<u8>)>,
    /// The new home name, when home moved.
    pub(crate) home: Option<String>,
    /// The new favorites, when they changed.
    pub(crate) favorites: Option<FavoritesAfter>,
    /// Why a move the rename called for was not written (config.edn
    /// unreadable, or a value it cannot edit safely).
    pub(crate) skipped: Option<String>,
}

/// The favorites list and arrangement page name after a rename.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FavoritesAfter {
    /// `:favorites`, in order.
    pub names: Vec<String>,
    /// `:tine/favorites-page`, when the graph has one.
    pub page: Option<String>,
}

/// What a page a setting names is, for O8's one-entry rule: a day, or a page
/// by key (the model's `target`).
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Day(tine_core::date::JournalDate),
    Page(String),
}

/// The favorites a rename carries, or `None` when it changes nothing.
/// `renamed` maps a stored name to its new name (model r6 O1: the namespace
/// prefix substitution, days pinned). OG `rename-page-aux` (page.cljs:507 at
/// 6e7afa8eb) moves a favorite with its page; Tine keeps its position and the
/// new spelling. Deliberate OG difference (FAV-MERGE, manager ruling
/// 2026-11-03): OG's `merge-pages!` drops the merged-away page's favorite
/// (`delete!` -> `unfavorite-page!`); here it moves to the survivor, because
/// a favorite is a link and a merge rewrites links. After a merge
/// (`merged_days`: the graph's day format) the list keeps only the first
/// entry of each page or day (O8, Lean `uniqueNames`); a plain rename drops
/// none (Lean `favorites_rename`). A stored alias spelling is not a renamed
/// name and stays (FAV-ALIAS: the alias still opens the page). O(favorites)
/// page-key folds and day parses.
pub(crate) fn favorites_after_rename(
    config: &tine_core::config::Config,
    renamed: &dyn Fn(&str) -> Option<String>,
    merged_days: Option<&tine_core::date::JournalFormat>,
) -> Option<FavoritesAfter> {
    let mut names: Vec<String> = config
        .favorites
        .iter()
        .map(|name| renamed(name).unwrap_or_else(|| name.clone()))
        .collect();
    let page = config
        .favorites_page
        .as_deref()
        .and_then(|page| renamed(page));
    // After a merge the one-entry rule covers the whole list, entries the
    // merge did not move too (Lean `favoritesAfter`).
    if let Some(days) = merged_days {
        let target = |name: &str| {
            days.parse(name).map_or_else(
                || Target::Page(tine_core::refs::page_key(name)),
                Target::Day,
            )
        };
        let mut seen = std::collections::BTreeSet::new();
        names.retain(|name| seen.insert(target(name)));
    }
    if page.is_none() && names == config.favorites {
        return None;
    }
    Some(FavoritesAfter {
        names,
        page: page.or_else(|| config.favorites_page.clone()),
    })
}

/// The config.edn step a page rename carries (one guarded replacement, after
/// the page operation): home (OG `rename-page-aux`, page.cljs:491 at
/// 6e7afa8eb) via `home_renamed`, favorites via `renamed` (deduplicated after
/// a merge, `merged_days`). Nothing is written when config.edn is absent or
/// nothing moves. All of it or none (model r6 O8): a config.edn that cannot
/// be read, or any value of it that cannot be edited safely, leaves the whole
/// file unchanged and is reported in `skipped` ("pages done, settings not
/// updated"); it never blocks the rename (refusal scenario: external-editor
/// race / sync delivery, docs/contracts/config-live-reload.md).
pub(crate) fn config_after_rename(
    store: &Store,
    home_renamed: &dyn Fn(&str) -> Option<String>,
    renamed: &dyn Fn(&str) -> Option<String>,
    merged_days: Option<&tine_core::date::JournalFormat>,
) -> ConfigAfterRename {
    let Ok(id) = config_id(store) else {
        return ConfigAfterRename::default();
    };
    let (text, rev) = match read_config(store, &id) {
        Ok(Some(read)) => read,
        Ok(None) => return ConfigAfterRename::default(),
        Err(error) => {
            return ConfigAfterRename {
                skipped: Some(format!("config.edn could not be read: {error}")),
                ..Default::default()
            }
        }
    };
    let config = tine_core::config::Config::parse(&text);
    let mut step = ConfigAfterRename::default();
    let mut next = text.clone();
    let mut edit = || -> io::Result<()> {
        if let Some(new) = config.default_home.as_deref().and_then(home_renamed) {
            let edited = edit_default_home(&next, Some(&new))?;
            if edited != next {
                next = edited;
                step.home = Some(new);
            }
        }
        if let Some(after) = favorites_after_rename(&config, renamed, merged_days) {
            let page = (after.page != config.favorites_page)
                .then_some(after.page.as_deref())
                .flatten();
            let edited = edit_favorites(&next, &after.names, page)?;
            if edited != next {
                next = edited;
                step.favorites = Some(after);
            }
        }
        Ok(())
    };
    if let Err(error) = edit() {
        return ConfigAfterRename {
            skipped: Some(error.to_string()),
            ..Default::default()
        };
    }
    if next != text {
        if let Err(error) = check_root(&next) {
            return ConfigAfterRename {
                skipped: Some(error.to_string()),
                ..Default::default()
            };
        }
        step.write = Some((id, rev, next.into_bytes()));
    }
    step
}

fn edit_map(source: &str) -> io::Result<Option<Form>> {
    read_for_edit(source).ok_or_else(|| refuse("the top-level form is not an editable map"))
}

fn map_open(source: &str, map: &Form) -> usize {
    map.span.start + source[map.span.clone()].find('{').unwrap()
}

fn insert_key(content: &mut String, map: &Form, key: &str, value: &str, top_level: bool) {
    let open = map_open(content, map);
    let key = if content.as_bytes()[map.span.start] == b'#' && !key.contains('/') {
        format!(":_/{key}")
    } else {
        format!(":{key}")
    };
    let text = if top_level {
        format!("\n {key} {value}\n")
    } else {
        let space = if content[open + 1..map.span.end - 1].trim().is_empty() {
            ""
        } else {
            " "
        };
        format!("{key} {value}{space}")
    };
    content.insert_str(open + 1, &text);
}

fn remove_entry(content: &mut String, map: &Form, key: &str) -> io::Result<()> {
    let Some((token, value)) = edit_entry(map, key) else {
        return Ok(());
    };
    if let Some(value) = value {
        let span =
            editable_value(value).ok_or_else(|| refuse("the setting contains authored trivia"))?;
        content.replace_range(span, "");
    }
    content.replace_range(token.span.clone(), "");
    Ok(())
}

fn set_nested(content: &mut String, map: &Form, key: &str, value: &str) -> io::Result<()> {
    if let Some((token, existing)) = edit_entry(map, key) {
        replace_entry_value(content, token, existing, value, false)
    } else {
        insert_key(content, map, key, value, false);
        Ok(())
    }
}

fn edn_string(value: &str) -> String {
    tine_core::edn::quote(value)
}

fn refuse(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("config.edn: {what}; not editing it"),
    )
}

fn replace_entry_value(
    content: &mut String,
    token: &Form,
    existing: Option<&Form>,
    value: &str,
    compact_gap: bool,
) -> io::Result<()> {
    let end = token.span.end;
    if let Some(existing) = existing {
        let span = editable_value(existing)
            .ok_or_else(|| refuse("the setting contains authored trivia"))?;
        if compact_gap && content[end..span.start].chars().all(char::is_whitespace) {
            content.replace_range(end..span.end, &format!(" {value}"));
        } else {
            content.replace_range(span, value);
        }
    } else {
        content.insert_str(end, &format!(" {value}"));
    }
    Ok(())
}

fn set_top_level(content: &mut String, key: &str, value: &str, scalar: bool) -> io::Result<()> {
    let key = key.trim_start_matches(':');
    let Some(map) = edit_map(content)? else {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("{{:{key} {value}}}\n"));
        return Ok(());
    };
    let Some((token, existing)) = edit_entry(&map, key) else {
        insert_key(content, &map, key, value, true);
        return Ok(());
    };
    let supported = |kind| {
        matches!(kind, Kind::String | Kind::Atom)
            || !scalar && matches!(kind, Kind::Vector | Kind::Map | Kind::Set)
    };
    if existing.is_some_and(|form| !supported(form.kind)) {
        return Err(refuse(&format!(
            ":{key} has a value shape Tine does not edit"
        )));
    }
    replace_entry_value(content, token, existing, value, !scalar)
}

fn update_scalar(store: &Store, key: &str, value: &str) -> io::Result<()> {
    update(store, |content| {
        let mut content = content.to_string();
        set_top_level(&mut content, key, value, true)?;
        Ok(content)
    })
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// Persist the task workflow to `:preferred-workflow :todo`/`:now`.
pub fn set_preferred_workflow(store: &Store, wf: &str) -> io::Result<()> {
    let kw = if wf == "todo" { ":todo" } else { ":now" };
    update_scalar(store, ":preferred-workflow", kw)
}

/// Persist `:feature/enable-timetracking?`. OG treats an absent key as ON,
/// but writing the explicit boolean keeps the Settings toggle reversible.
pub fn set_timetracking_enabled(store: &Store, enabled: bool) -> io::Result<()> {
    update_scalar(store, ":feature/enable-timetracking?", bool_text(enabled))
}

/// Persist `:ui/show-brackets?`. OG treats an absent key as ON, but writing
/// the explicit boolean keeps the Settings toggle reversible.
pub fn set_show_brackets(store: &Store, enabled: bool) -> io::Result<()> {
    update_scalar(store, ":ui/show-brackets?", bool_text(enabled))
}

/// Persist `:ui/enable-tooltip?` (OG `config-handler/toggle-ui-enable-tooltip!`).
/// OG treats an absent key as ON, but writing the explicit boolean keeps the
/// Settings toggle reversible.
pub fn set_enable_tooltip(store: &Store, enabled: bool) -> io::Result<()> {
    update_scalar(store, ":ui/enable-tooltip?", bool_text(enabled))
}

/// Persist the document-mode escape hatch. OG declares the equivalent key in
/// `src/main/frontend/schema/handler/common_config.cljc:41` at `6e7afa8eb`.
pub fn set_doc_mode_enter_for_new_block(store: &Store, enabled: bool) -> io::Result<()> {
    update_scalar(
        store,
        ":shortcut/doc-mode-enter-for-new-block?",
        bool_text(enabled),
    )
}

/// Persist logical (Roam-like) outdenting. OG declares the equivalent key in
/// `src/main/frontend/schema/handler/common_config.cljc:83` at `6e7afa8eb`.
pub fn set_logical_outdenting(store: &Store, enabled: bool) -> io::Result<()> {
    update_scalar(store, ":editor/logical-outdenting?", bool_text(enabled))
}

/// Persist the one-time in-app Guide announcement flag, graph-locally.
pub fn set_guide_announced(store: &Store, announced: bool) -> io::Result<()> {
    update_scalar(store, ":tine/guide-announced?", bool_text(announced))
}

/// Persist the preferred format for new pages/journals as
/// `:preferred-format "Markdown"|"Org"` (the capitalized string OG uses),
/// replacing the whole existing value token (string or keyword).
pub fn set_preferred_format(store: &Store, fmt: tine_core::model::Format) -> io::Result<()> {
    let val = match fmt {
        tine_core::model::Format::Org => "\"Org\"",
        tine_core::model::Format::Md => "\"Markdown\"",
    };
    update_scalar(store, ":preferred-format", val)
}

/// `:journal/page-title-format "<pattern>"` — the journal *display* title
/// format (e.g. `MMM do, yyyy`). Affects how journal dates render and how new
/// journal titles/`[[date]]` references are written; the on-disk file name
/// (governed by `:journal/file-name-format`, default `yyyy_MM_dd`) is left
/// untouched, so existing journal files keep working.
pub fn set_journal_page_title_format(store: &Store, fmt: &str) -> io::Result<()> {
    update_scalar(store, ":journal/page-title-format", &edn_string(fmt))
}

/// [`set_journal_page_title_format`], then the reloaded config's journal
/// pattern diagnostics (D-20261020-CH3). An own write emits no external-config
/// event, so Settings takes the fresh notes from this result.
pub fn store_journal_page_title_format(store: &Store, fmt: &str) -> io::Result<Vec<String>> {
    set_journal_page_title_format(store, fmt)?;
    Ok(store.config().config.journal_config_diagnostics.clone())
}

/// Persist the new-journal default template as `:default-templates {:journals
/// "Name"}`. `Some` sets/replaces the `:journals` entry; `None` removes it.
/// Other keys in `:default-templates`, the rest of the file, and comments are
/// preserved. A non-map `:default-templates` value (`nil`) is replaced by a
/// map on `Some` and left alone on `None`.
pub fn set_default_journal_template(store: &Store, name: Option<&str>) -> io::Result<()> {
    update(store, |source| {
        let mut content = source.to_owned();
        let root = edit_map(source)?;
        let existing = root
            .as_ref()
            .and_then(|root| edit_entry(root, "default-templates"));
        let map = existing.and_then(|(_, form)| form.filter(|form| form.kind == Kind::Map));
        match (map, name) {
            (Some(map), Some(name)) => {
                set_nested(&mut content, map, "journals", &edn_string(name))?
            }
            (Some(map), None) => remove_entry(&mut content, map, "journals")?,
            (None, Some(name)) => {
                let value = format!("{{:journals {}}}", edn_string(name));
                match (root.as_ref(), existing) {
                    (_, Some((key, old))) => {
                        replace_entry_value(&mut content, key, old, &value, true)?
                    }
                    (Some(root), None) => {
                        insert_key(&mut content, root, "default-templates", &value, true)
                    }
                    (None, None) => {
                        set_top_level(&mut content, "default-templates", &value, false)?
                    }
                }
            }
            (None, None) => {}
        }
        Ok(content)
    })
}

/// Persist the first day of week to `:start-of-week N` (Logseq convention:
/// 0=Monday … 6=Sunday).
pub fn set_start_of_week(store: &Store, n: u32) -> io::Result<()> {
    update_scalar(store, ":start-of-week", &n.min(6).to_string())
}
