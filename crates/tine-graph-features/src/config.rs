//! Graph config edits. Each operation reads one Meta file and commits one guarded
//! create or replace; conflicts retry with fresh bytes at most four times.
//! Cost: O(config.edn bytes) per attempt. Errors are I/O errors; exhausted
//! conflicts return WouldBlock. Callers need no graph path, lock, or cache state.

use std::io;

use tine_core::config::{
    find_keyword, match_close_brace, match_close_bracket, next_value_span, skip_blank,
};

use tine_store::{Area, Content, FileId, FileRev, Store, StoreError};

use crate::{is_conflict, store_error, tx_error};

fn config_error(error: StoreError) -> io::Error {
    store_error(error)
}

fn config_commit(outcome: tine_store::TxOutcome) -> io::Result<()> {
    tx_error(outcome).map(|_| ())
}

fn config_id(store: &Store) -> io::Result<FileId> {
    store
        .file_id(Area::Meta, "config.edn")
        .map_err(config_error)
}

fn read_config(store: &Store, id: &FileId) -> io::Result<Option<(String, FileRev)>> {
    match store.read(id, None) {
        Ok((bytes, rev)) => String::from_utf8(bytes)
            .map(|text| Some((text, rev)))
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stream did not contain valid UTF-8",
                )
            }),
        Err(StoreError::NotFound) => Ok(None),
        Err(error) => Err(config_error(error)),
    }
}

fn update(store: &Store, edit: impl Fn(&str) -> io::Result<String>) -> io::Result<()> {
    let id = config_id(store)?;
    for _ in 0..4 {
        let current = read_config(store, &id)?;
        let next = edit(current.as_ref().map_or("{}\n", |(text, _)| text))?;
        let mut tx = store.transaction();
        if let Some((_, rev)) = current {
            tx.replace(&id, rev, next.into_bytes());
        } else {
            tx.create(&id, Content::Bytes(next.into_bytes()));
        }
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        config_commit(outcome)?;
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "config changed repeatedly during update",
    ))
}

/// Return custom.css or an empty string if absent, unreadable or non-UTF-8.
/// Cost: O(custom.css bytes); no size cap, as in v0.6.5.
pub fn custom_css(store: &Store) -> String {
    store
        .file_id(Area::Meta, "custom.css")
        .ok()
        .and_then(|id| store.read(&id, None).ok())
        .and_then(|(bytes, _)| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

/// Persist the favorites list to `:favorites [...]`, replacing the existing
/// vector or inserting one, preserving the rest of the file.
pub fn set_favorites(store: &Store, names: &[String]) -> io::Result<()> {
    update(store, |content| {
        let mut content = content.to_string();
        let vec_str = format!(
            "[{}]",
            names
                .iter()
                .map(|n| format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(" ")
        );
        if let Some(start) = find_keyword(&content, ":favorites") {
            // Replace the existing `:favorites [...]` vector. Require its value to
            // be a vector and find the matching `]` with an EDN-aware scan so a
            // favorite NAME containing `]` (or a comment in the vector) can't
            // truncate the replacement and corrupt config.edn.
            let after = start + ":favorites".len();
            let j = skip_blank(&content, after); // comment-aware, like the readers
            if content.as_bytes().get(j) == Some(&b'[') {
                let end = match_close_bracket(&content, j) + 1;
                content.replace_range(start..end, &format!(":favorites {vec_str}"));
            } else {
                content.insert_str(after, &format!(" {vec_str}"));
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n :favorites {vec_str}\n"));
        } else {
            content = format!("{{:favorites {vec_str}}}\n");
        }
        Ok(content)
    })
}

/// Persist the task workflow to `:preferred-workflow :todo`/`:now`, replacing
/// the keyword value or inserting the key. `find_keyword` skips comments/strings
/// so a commented or in-string `:preferred-workflow` is never edited.
pub fn set_preferred_workflow(store: &Store, wf: &str) -> io::Result<()> {
    let kw = if wf == "todo" { ":todo" } else { ":now" };
    let key = ":preferred-workflow";
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            let vstart = skip_blank(&content, after); // comment-aware
            if content[vstart..].starts_with(':') {
                let vrest = &content[vstart + 1..];
                let end = vrest
                    .find(|c: char| c.is_whitespace() || c == '}' || c == ')')
                    .unwrap_or(vrest.len());
                content.replace_range(vstart..vstart + 1 + end, kw);
            } else {
                content.insert_str(after, &format!(" {kw}"));
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n :preferred-workflow {kw}\n"));
        } else {
            content = format!("{{:preferred-workflow {kw}}}\n");
        }
        Ok(content)
    })
}

/// Persist `:feature/enable-timetracking?`. OG treats an absent key as ON,
/// but writing the explicit boolean keeps the Settings toggle reversible.
pub fn set_timetracking_enabled(store: &Store, enabled: bool) -> io::Result<()> {
    let key = ":feature/enable-timetracking?";
    let val = if enabled { "true" } else { "false" };
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            match next_value_span(&content, after, content.len()) {
                Some((vstart, vend, _)) if vend > vstart => {
                    content.replace_range(vstart..vend, val)
                }
                _ => content.insert_str(after, &format!(" {val}")),
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n {key} {val}\n"));
        } else {
            content = format!("{{{key} {val}}}\n");
        }
        Ok(content)
    })
}

/// Persist `:ui/show-brackets?`. OG treats an absent key as ON, but writing
/// the explicit boolean keeps the Settings toggle reversible.
pub fn set_show_brackets(store: &Store, enabled: bool) -> io::Result<()> {
    let key = ":ui/show-brackets?";
    let val = if enabled { "true" } else { "false" };
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            match next_value_span(&content, after, content.len()) {
                Some((vstart, vend, _)) if vend > vstart => {
                    content.replace_range(vstart..vend, val)
                }
                _ => content.insert_str(after, &format!(" {val}")),
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n {key} {val}\n"));
        } else {
            content = format!("{{{key} {val}}}\n");
        }
        Ok(content)
    })
}

/// Persist the document-mode escape hatch. OG declares the equivalent key in
/// `src/main/frontend/schema/handler/common_config.cljc:41` at `6e7afa8eb`.
pub fn set_doc_mode_enter_for_new_block(store: &Store, enabled: bool) -> io::Result<()> {
    set_config_bool(store, ":shortcut/doc-mode-enter-for-new-block?", enabled)
}

/// Persist logical (Roam-like) outdenting. OG declares the equivalent key in
/// `src/main/frontend/schema/handler/common_config.cljc:83` at `6e7afa8eb`.
pub fn set_logical_outdenting(store: &Store, enabled: bool) -> io::Result<()> {
    set_config_bool(store, ":editor/logical-outdenting?", enabled)
}

/// Write one graph-portable boolean through the existing config.edn atomic
/// update path, preserving unrelated keys, comments, and formatting.
fn set_config_bool(store: &Store, key: &str, enabled: bool) -> io::Result<()> {
    let val = if enabled { "true" } else { "false" };
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            match next_value_span(&content, after, content.len()) {
                Some((vstart, vend, _)) if vend > vstart => {
                    content.replace_range(vstart..vend, val)
                }
                _ => content.insert_str(after, &format!(" {val}")),
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n {key} {val}\n"));
        } else {
            content = format!("{{{key} {val}}}\n");
        }
        Ok(content)
    })
}

/// Persist the one-time in-app Guide announcement flag, graph-locally.
pub fn set_guide_announced(store: &Store, announced: bool) -> io::Result<()> {
    let key = ":tine/guide-announced?";
    let val = if announced { "true" } else { "false" };
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            match next_value_span(&content, after, content.len()) {
                Some((vstart, vend, _)) if vend > vstart => {
                    content.replace_range(vstart..vend, val)
                }
                _ => content.insert_str(after, &format!(" {val}")),
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n {key} {val}\n"));
        } else {
            content = format!("{{{key} {val}}}\n");
        }
        Ok(content)
    })
}

/// Persist the preferred format for new pages/journals as
/// `:preferred-format "Markdown"|"Org"` (the capitalized string OG uses),
/// replacing the existing value or inserting the key, preserving the rest of
/// the file (comments, formatting, other keys).
pub fn set_preferred_format(store: &Store, fmt: tine_core::model::Format) -> io::Result<()> {
    let val = match fmt {
        tine_core::model::Format::Org => "\"Org\"",
        tine_core::model::Format::Md => "\"Markdown\"",
    };
    let key = ":preferred-format";
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            // Replace the FULL existing value span — whether it's a string
            // (`"Markdown"`) or a keyword (`:org`) — so a keyword value isn't left
            // dangling beside the new string (which would corrupt the map).
            match next_value_span(&content, after, content.len()) {
                Some((vstart, vend, _)) if vend > vstart => {
                    content.replace_range(vstart..vend, val)
                }
                _ => content.insert_str(after, &format!(" {val}")),
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n {key} {val}\n"));
        } else {
            content = format!("{{{key} {val}}}\n");
        }
        Ok(content)
    })
}

/// `:journal/page-title-format "<pattern>"` — the journal *display* title
/// format (e.g. `MMM do, yyyy`). Affects how journal dates render and how new
/// journal titles/`[[date]]` references are written; the on-disk file name
/// (governed by `:journal/file-name-format`, default `yyyy_MM_dd`) is left
/// untouched, so existing journal files keep working. Replaces the existing
/// value or inserts the key, preserving the rest of the file.
pub fn set_journal_page_title_format(store: &Store, fmt: &str) -> io::Result<()> {
    let escaped = fmt.replace('\\', "\\\\").replace('"', "\\\"");
    let val = format!("\"{escaped}\"");
    let key = ":journal/page-title-format";
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            match next_value_span(&content, after, content.len()) {
                Some((vstart, vend, _)) if vend > vstart => {
                    content.replace_range(vstart..vend, &val)
                }
                _ => content.insert_str(after, &format!(" {val}")),
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n {key} {val}\n"));
        } else {
            content = format!("{{{key} {val}}}\n");
        }
        Ok(content)
    })
}

/// Persist the new-journal default template as `:default-templates {:journals
/// "Name"}`. `Some` sets/replaces the `:journals` entry; `None` removes it.
/// Other keys in `:default-templates`, the rest of the file, and comments are
/// preserved.
pub fn set_default_journal_template(store: &Store, name: Option<&str>) -> io::Result<()> {
    update(store, |content| {
        let mut content = content.to_string();

        // Locate a real `:default-templates` whose value is a map literal `{ … }`.
        let dt = find_keyword(&content, ":default-templates").and_then(|start| {
            let after = start + ":default-templates".len();
            let j = skip_blank(&content, after); // comment-aware
            if content.as_bytes().get(j) != Some(&b'{') {
                return None; // value isn't a map → don't touch it
            }
            let close = match_close_brace(&content, j);
            Some((j, close)) // byte indices of `{` and matching `}`
        });

        match name {
            Some(n) => {
                let v = format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\""));
                match dt {
                    Some((open, close)) => {
                        if let Some(jrel) = find_keyword(&content[open + 1..close], ":journals") {
                            // Replace the value IMMEDIATELY after :journals (string or
                            // not) — never scan for the next quote anywhere, which could
                            // land on a later key's value.
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
                    None => {
                        let entry = format!("\n :default-templates {{:journals {v}}}\n");
                        if let Some(brace) = content.find('{') {
                            content.insert_str(brace + 1, &entry);
                        } else {
                            content = format!("{{:default-templates {{:journals {v}}}}}\n");
                        }
                    }
                }
            }
            None => {
                if let Some((open, close)) = dt {
                    if let Some(jrel) = find_keyword(&content[open + 1..close], ":journals") {
                        let jstart = open + 1 + jrel;
                        let after = jstart + ":journals".len();
                        let end = next_value_span(&content, after, close)
                            .map(|(_, vend, _)| vend)
                            .unwrap_or(after);
                        let tail: usize = content[end..close]
                            .chars()
                            .take_while(|c| c.is_whitespace() || *c == ',')
                            .map(|c| c.len_utf8())
                            .sum();
                        content.replace_range(jstart..end + tail, "");
                    }
                }
            }
        }
        Ok(content)
    })
}

/// Persist the first day of week to `:start-of-week N` (Logseq convention:
/// 0=Monday … 6=Sunday), replacing the numeric value or inserting the key.
/// `find_keyword` is comment/string-aware, so a commented `:start-of-week` is
/// never edited (we insert a real one instead).
pub fn set_start_of_week(store: &Store, n: u32) -> io::Result<()> {
    let n = n.min(6);
    let key = ":start-of-week";
    update(store, |content| {
        let mut content = content.to_string();

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            let vstart = skip_blank(&content, after); // comment-aware
            let digits = content[vstart..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .count();
            if digits > 0 {
                content.replace_range(vstart..vstart + digits, &n.to_string());
            } else {
                content.insert_str(after, &format!(" {n}"));
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n :start-of-week {n}\n"));
        } else {
            content = format!("{{:start-of-week {n}}}\n");
        }
        Ok(content)
    })
}
