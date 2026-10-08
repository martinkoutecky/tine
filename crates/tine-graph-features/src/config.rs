//! Graph config edits. Each operation reads one Meta file and commits one guarded
//! create or replace; conflicts retry with fresh bytes at most four times.
//! Cost: O(config.edn bytes) per attempt. Errors are I/O errors; exhausted
//! conflicts return WouldBlock. Callers need no graph path, lock, or cache state.

use std::io;

use tine_core::config::{
    balanced_map_at, edn_str_end, find_keyword_at_map_level, find_top_level_keyword,
    match_close_brace, match_close_bracket, next_value_span, root_map_bounds, skip_blank,
};

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
    if skip_blank(text, 0) < text.len() && root_map_bounds(text).is_none() {
        return Err(refuse("the top-level form is not a balanced map"));
    }
    Ok(())
}

fn update(store: &Store, edit: impl Fn(&str) -> io::Result<String>) -> io::Result<()> {
    let id = config_id(store)?;
    crate::retry_on_conflict("config changed repeatedly during update", || {
        let current = read_config(store, &id)?;
        let text = current.as_ref().map_or("{}\n", |(text, _)| text);
        check_root(text)?;
        let next = edit(text)?;
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
    update(store, |content| {
        let mut content = content.to_string();
        if let Some(page) = page {
            set_top_level(&mut content, ":tine/favorites-page", &edn_string(page))?;
        }
        let names: Vec<String> = names.iter().map(|n| edn_string(n)).collect();
        set_top_level(
            &mut content,
            ":favorites",
            &format!("[{}]", names.join(" ")),
        )?;
        Ok(content)
    })
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
    check_root(source)?;
    let mut content = source.to_owned();
    let name = name.map(str::trim).filter(|name| !name.is_empty());
    let Some((root_open, _)) = root_map_bounds(&content) else {
        // Blank or comment-only: nothing to clear, or a new map to create.
        if let Some(name) = name {
            let entry = format!(":default-home {{:page {}}}", edn_string(name));
            insert_top_level(&mut content, &entry)?;
        }
        return Ok(content);
    };
    let home = find_top_level_keyword(&content, ":default-home")
        .map(|start| {
            let open = skip_blank(&content, start + ":default-home".len());
            balanced_map_at(&content, open)
                .ok_or_else(|| refuse(":default-home exists but is not a balanced map"))
        })
        .transpose()?;
    match (home, name) {
        (None, Some(name)) => {
            let value = edn_string(name);
            content.insert_str(
                root_open + 1,
                &format!("\n :default-home {{:page {value}}}\n"),
            );
        }
        (Some((open, close)), Some(name)) => {
            let value = edn_string(name);
            if let Some(relative) = find_keyword_at_map_level(&content[open + 1..close], ":page") {
                let after = open + 1 + relative + ":page".len();
                match next_value_span(&content, after, close) {
                    Some((start, end, _)) => content.replace_range(start..end, &value),
                    None => content.insert_str(after, &format!(" {value}")),
                }
            } else {
                let separator = if content[open + 1..close].trim().is_empty() {
                    ""
                } else {
                    " "
                };
                content.insert_str(open + 1, &format!(":page {value}{separator}"));
            }
        }
        (Some((open, close)), None) => remove_entry(&mut content, open, close, ":page"),
        (None, None) => {}
    }
    Ok(content)
}

/// The config.edn replacement a page rename carries when it renames the home
/// page (OG `rename-page-aux`, page.cljs:491 at 6e7afa8eb): `(id, base rev,
/// new bytes, new home name)`. `renamed` maps the configured home name to its
/// new name, or `None`. Nothing is returned when config.edn is absent,
/// unreadable or cannot be edited safely: a malformed config never blocks a
/// rename (scenario: external-editor race / sync delivery), and home keeps
/// its old name.
pub(crate) fn home_after_rename(
    store: &Store,
    renamed: impl Fn(&str) -> Option<String>,
) -> Option<(FileId, FileRev, Vec<u8>, String)> {
    let id = config_id(store).ok()?;
    let (text, rev) = read_config(store, &id).ok()??;
    let new = renamed(&tine_core::config::Config::parse(&text).default_home?)?;
    let next = edit_default_home(&text, Some(&new)).ok()?;
    (next != text).then(|| (id, rev, next.into_bytes(), new))
}

/// Remove `key` and its value from the map `{…}` at `open..close`, with the
/// blanks and commas after it.
fn remove_entry(content: &mut String, open: usize, close: usize, key: &str) {
    let Some(relative) = find_keyword_at_map_level(&content[open + 1..close], key) else {
        return;
    };
    let start = open + 1 + relative;
    let after = start + key.len();
    let end = next_value_span(content, after, close)
        .map(|(_, end, _)| end)
        .unwrap_or(after);
    let tail = content[end..close]
        .chars()
        .take_while(|ch| ch.is_whitespace() || *ch == ',')
        .map(char::len_utf8)
        .sum::<usize>();
    content.replace_range(start..end + tail, "");
}

fn edn_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn refuse(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("config.edn: {what}; not editing it"),
    )
}

/// Set `key` to `value` in config.edn text: replace the key's whole existing
/// value (string, vector, map, set or scalar such as `nil`) or insert the key
/// into the top-level map. Never leaves a key without its value.
fn set_top_level(content: &mut String, key: &str, value: &str) -> io::Result<()> {
    let Some(start) = find_top_level_keyword(content, key) else {
        return insert_top_level(content, &format!("{key} {value}"));
    };
    let after = start + key.len();
    let j = skip_blank(content, after); // comment-aware, like the readers
    let b = content.as_bytes();
    let missing = matches!(b.get(j), None | Some(b'}'));
    let closed = |close: usize| (close < b.len()).then_some(close + 1);
    let end = match b.get(j) {
        None | Some(b'}') => None, // the key has no value: insert one
        Some(b'"') => Some(edn_str_end(content, j)).filter(|&e| b[e - 1] == b'"' && e > j + 1),
        Some(b'[') => closed(match_close_bracket(content, j)),
        Some(b'{') => closed(match_close_brace(content, j)),
        Some(b'#') if b.get(j + 1) == Some(&b'{') => closed(match_close_brace(content, j + 1)),
        Some(b'(' | b'#' | b'^' | b'\'' | b'@' | b'`' | b'~' | b']' | b')') => {
            return Err(refuse(&format!(
                "{key} has a value shape Tine does not edit"
            )));
        }
        Some(_) => next_value_span(content, j, content.len()).map(|(_, end, _)| end),
    };
    match end {
        // Key through value, as the legacy writer (byte fixture `client`).
        Some(end) => content.replace_range(start..end, &format!("{key} {value}")),
        None if missing => content.insert_str(after, &format!(" {value}")),
        None => return Err(refuse(&format!("{key} has an unterminated value"))),
    }
    Ok(())
}

/// Insert `entry` right after the top-level map's `{`: the first form after
/// blanks and `;` comments, never a `{` inside a comment or string. A file of
/// only blanks/comments gets a new map appended; any other first form is
/// refused rather than guessed at.
fn insert_top_level(content: &mut String, entry: &str) -> io::Result<()> {
    let open = skip_blank(content, 0);
    match content.as_bytes().get(open) {
        Some(b'{') => content.insert_str(open + 1, &format!("\n {entry}\n")),
        None => {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&format!("{{{entry}}}\n"));
        }
        Some(_) => return Err(refuse("the top-level form is not a map")),
    }
    Ok(())
}

/// Set a top-level scalar `key` (keyword, boolean, number or string): replace
/// only the value token, keeping the key and its spacing, or insert the key
/// into the root map. A collection-shaped value is refused, never spliced.
fn set_scalar(content: &mut String, key: &str, value: &str) -> io::Result<()> {
    let Some(start) = find_top_level_keyword(content, key) else {
        return insert_top_level(content, &format!("{key} {value}"));
    };
    let after = start + key.len();
    let (_, close) = root_map_bounds(content).ok_or_else(|| refuse("no top-level map"))?;
    let j = skip_blank(content, after);
    if matches!(content.as_bytes().get(j), Some(b'{' | b'[' | b'(' | b'#')) {
        return Err(refuse(&format!(
            "{key} has a value shape Tine does not edit"
        )));
    }
    match next_value_span(content, after, close) {
        Some((vstart, vend, _)) if vend > vstart => content.replace_range(vstart..vend, value),
        _ => content.insert_str(after, &format!(" {value}")),
    }
    Ok(())
}

fn update_scalar(store: &Store, key: &str, value: &str) -> io::Result<()> {
    update(store, |content| {
        let mut content = content.to_string();
        set_scalar(&mut content, key, value)?;
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

/// Persist the new-journal default template as `:default-templates {:journals
/// "Name"}`. `Some` sets/replaces the `:journals` entry; `None` removes it.
/// Other keys in `:default-templates`, the rest of the file, and comments are
/// preserved. A non-map `:default-templates` value (`nil`) is replaced by a
/// map on `Some` and left alone on `None`.
pub fn set_default_journal_template(store: &Store, name: Option<&str>) -> io::Result<()> {
    const KEY: &str = ":default-templates";
    update(store, |content| {
        let mut content = content.to_string();
        let dt = find_top_level_keyword(&content, KEY).map(|start| {
            let j = skip_blank(&content, start + KEY.len());
            balanced_map_at(&content, j)
        });
        match (dt, name) {
            (Some(Some((open, close))), Some(name)) => {
                let v = edn_string(name);
                if let Some(jrel) =
                    find_keyword_at_map_level(&content[open + 1..close], ":journals")
                {
                    // Replace the value IMMEDIATELY after :journals, never a
                    // later key's value.
                    let after = open + 1 + jrel + ":journals".len();
                    match next_value_span(&content, after, close) {
                        Some((vstart, vend, _)) => content.replace_range(vstart..vend, &v),
                        None => content.insert_str(after, &format!(" {v}")),
                    }
                } else {
                    let sep = if content[open + 1..close].trim().is_empty() {
                        ""
                    } else {
                        " "
                    };
                    content.insert_str(open + 1, &format!(":journals {v}{sep}"));
                }
            }
            (_, Some(name)) => {
                set_top_level(
                    &mut content,
                    KEY,
                    &format!("{{:journals {}}}", edn_string(name)),
                )?;
            }
            (Some(Some((open, close))), None) => {
                remove_entry(&mut content, open, close, ":journals")
            }
            (_, None) => {}
        }
        Ok(content)
    })
}

/// Persist the first day of week to `:start-of-week N` (Logseq convention:
/// 0=Monday … 6=Sunday).
pub fn set_start_of_week(store: &Store, n: u32) -> io::Result<()> {
    update_scalar(store, ":start-of-week", &n.min(6).to_string())
}
