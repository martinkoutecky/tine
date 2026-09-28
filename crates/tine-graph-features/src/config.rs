//! Graph config edits. Each operation reads one Meta file and commits one guarded
//! create or replace; conflicts retry with fresh bytes at most four times.
//! Cost: O(config.edn bytes) per attempt. Errors are I/O errors; exhausted
//! conflicts return WouldBlock. Callers need no graph path, lock, or cache state.

use std::io;

use tine_core::config::{
    edn_str_end, find_keyword, match_close_brace, match_close_bracket, next_value_span, skip_blank,
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

fn update(store: &Store, edit: impl Fn(&str) -> io::Result<String>) -> io::Result<()> {
    let id = config_id(store)?;
    crate::retry_on_conflict("config changed repeatedly during update", || {
        let current = read_config(store, &id)?;
        let next = edit(current.as_ref().map_or("{}\n", |(text, _)| text))?;
        let mut tx = store.transaction(None);
        if let Some((_, rev)) = current {
            tx.replace(&id, rev, next.into_bytes());
        } else {
            tx.create(&id, Content::Bytes(next.into_bytes()));
        }
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

/// Return custom.css or an empty string if absent, unreadable or non-UTF-8.
/// Cost: O(custom.css bytes); no size cap, as in v0.6.5.
pub fn custom_css(store: &Store) -> String {
    store
        .file_id(Area::Meta, "custom.css")
        .ok()
        .and_then(|id| {
            store
                .read(&id, Some(tine_store::PARSE_INPUT_MAX_BYTES))
                .ok()
        })
        .and_then(|(bytes, _)| String::from_utf8(bytes).ok())
        .unwrap_or_default()
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
    let Some(start) = find_keyword(content, key) else {
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
        } else {
            insert_top_level(&mut content, &format!(":preferred-workflow {kw}"))?;
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
        } else {
            insert_top_level(&mut content, &format!("{key} {val}"))?;
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
        } else {
            insert_top_level(&mut content, &format!("{key} {val}"))?;
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
        } else {
            insert_top_level(&mut content, &format!("{key} {val}"))?;
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
        } else {
            insert_top_level(&mut content, &format!("{key} {val}"))?;
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
        } else {
            insert_top_level(&mut content, &format!("{key} {val}"))?;
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
        } else {
            insert_top_level(&mut content, &format!("{key} {val}"))?;
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
                        insert_top_level(
                            &mut content,
                            &format!(":default-templates {{:journals {v}}}"),
                        )?;
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
        } else {
            insert_top_level(&mut content, &format!(":start-of-week {n}"))?;
        }
        Ok(content)
    })
}
