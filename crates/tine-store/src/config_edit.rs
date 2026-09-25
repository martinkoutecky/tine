//! config.edn writers (moved from tine-core config.rs, og batch 1 step 2).

use std::io;

use tine_core::config::{
    find_keyword, match_close_brace, match_close_bracket, next_value_span, skip_blank,
};

use crate::model::Graph;

// ---------------------------------------------------------------------------
// Writers — surgical, comment/format-preserving in-place edits of config.edn.
// (Graph.root is pub; atomic_write is pub(crate); both reachable from here.)
// ---------------------------------------------------------------------------

/// Serializes ALL config.edn writers so two concurrent setting changes (or one
/// racing a read-modify-write) can't clobber each other (audit M2). Process-global:
/// config writes are rare and there's one config per running app. Every writer below
/// goes through `crate::model::atomic_update(&path, &CONFIG_LOCK, …)`, which also
/// makes the read NFS-safe (NotFound→`{}`, other errors abort — audit H2) and the
/// commit atomic.
static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn config_path_for_write(graph: &Graph) -> io::Result<std::path::PathBuf> {
    let path = graph.root.join("logseq").join("config.edn");
    graph.ensure_write_target(&path)?;
    Ok(path)
}

impl Graph {
    /// Persist the favorites list to `:favorites [...]`, replacing the existing
    /// vector or inserting one, preserving the rest of the file.
    pub fn set_favorites(&self, names: &[String]) -> io::Result<()> {
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_preferred_workflow(&self, wf: &str) -> io::Result<()> {
        let kw = if wf == "todo" { ":todo" } else { ":now" };
        let key = ":preferred-workflow";
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_timetracking_enabled(&self, enabled: bool) -> io::Result<()> {
        let key = ":feature/enable-timetracking?";
        let val = if enabled { "true" } else { "false" };
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_show_brackets(&self, enabled: bool) -> io::Result<()> {
        let key = ":ui/show-brackets?";
        let val = if enabled { "true" } else { "false" };
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_doc_mode_enter_for_new_block(&self, enabled: bool) -> io::Result<()> {
        self.set_config_bool(":shortcut/doc-mode-enter-for-new-block?", enabled)
    }

    /// Persist logical (Roam-like) outdenting. OG declares the equivalent key in
    /// `src/main/frontend/schema/handler/common_config.cljc:83` at `6e7afa8eb`.
    pub fn set_logical_outdenting(&self, enabled: bool) -> io::Result<()> {
        self.set_config_bool(":editor/logical-outdenting?", enabled)
    }

    /// Write one graph-portable boolean through the existing config.edn atomic
    /// update path, preserving unrelated keys, comments, and formatting.
    fn set_config_bool(&self, key: &str, enabled: bool) -> io::Result<()> {
        let val = if enabled { "true" } else { "false" };
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_guide_announced(&self, announced: bool) -> io::Result<()> {
        let key = ":tine/guide-announced?";
        let val = if announced { "true" } else { "false" };
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_preferred_format(&self, fmt: crate::model::Format) -> io::Result<()> {
        let val = match fmt {
            crate::model::Format::Org => "\"Org\"",
            crate::model::Format::Md => "\"Markdown\"",
        };
        let key = ":preferred-format";
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_journal_page_title_format(&self, fmt: &str) -> io::Result<()> {
        let escaped = fmt.replace('\\', "\\\\").replace('"', "\\\"");
        let val = format!("\"{escaped}\"");
        let key = ":journal/page-title-format";
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
    pub fn set_default_journal_template(&self, name: Option<&str>) -> io::Result<()> {
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
                            if let Some(jrel) = find_keyword(&content[open + 1..close], ":journals")
                            {
                                // Replace the value IMMEDIATELY after :journals (string or
                                // not) — never scan for the next quote anywhere, which could
                                // land on a later key's value.
                                let after = open + 1 + jrel + ":journals".len();
                                match next_value_span(&content, after, close) {
                                    Some((vstart, vend, _)) => {
                                        content.replace_range(vstart..vend, &v)
                                    }
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
    pub fn set_start_of_week(&self, n: u32) -> io::Result<()> {
        let n = n.min(6);
        let key = ":start-of-week";
        let path = config_path_for_write(self)?;
        crate::model::atomic_update(&path, &CONFIG_LOCK, |content| {
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
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tine_core::config::Config;

    #[test]
    fn set_timetracking_enabled_round_trips() {
        use crate::model::Graph;
        let dir = std::env::temp_dir().join(format!("tine-cfgttrack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Markdown\"\n :start-of-week 0}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.set_timetracking_enabled(false).unwrap();
        let after = fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert!(
            after.contains(":feature/enable-timetracking? false"),
            "not written: {after}"
        );
        assert!(after.contains(":start-of-week 0"), "other keys preserved");
        assert!(!Graph::open(&dir).config.enable_timetracking);
        Graph::open(&dir).set_timetracking_enabled(true).unwrap();
        assert!(Graph::open(&dir).config.enable_timetracking);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn show_brackets_defaults_parses_and_round_trips() {
        use crate::model::Graph;

        assert!(Config::parse("{}").show_brackets);
        assert!(!Config::parse("{:ui/show-brackets? false}").show_brackets);

        let dir = std::env::temp_dir().join(format!("tine-cfgbrackets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Markdown\"\n ;; preserve this comment\n :start-of-week 0}\n",
        )
        .unwrap();

        Graph::open(&dir).set_show_brackets(false).unwrap();
        let after = fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert!(
            after.contains(":ui/show-brackets? false"),
            "not written: {after}"
        );
        assert!(after.contains(":start-of-week 0"), "other keys preserved");
        assert!(
            after.contains(";; preserve this comment"),
            "comments preserved"
        );
        assert!(!Graph::open(&dir).config.show_brackets);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_preferred_format_replaces_keyword_value() {
        use crate::model::{Format, Graph};
        // M3: the writer must replace a keyword value wholesale, not leave it
        // dangling beside the new string (which would corrupt the EDN map).
        let dir = std::env::temp_dir().join(format!("tine-cfgkw-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format :markdown\n :start-of-week 0}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.set_preferred_format(Format::Org).unwrap();
        let after = fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert!(
            after.contains(":preferred-format \"Org\""),
            "keyword not replaced: {after}"
        );
        assert!(
            !after.contains(":markdown"),
            "stale keyword left behind: {after}"
        );
        assert!(after.contains(":start-of-week 0"), "other keys preserved");
        assert_eq!(Graph::open(&dir).preferred_format(), Format::Org);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_preferred_format_round_trips() {
        use crate::model::{Format, Graph};
        let dir = std::env::temp_dir().join(format!("tine-cfgfmt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Markdown\"\n :start-of-week 0}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.set_preferred_format(Format::Org).unwrap();
        let after = fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert!(
            after.contains(":preferred-format \"Org\""),
            "value flipped: {after}"
        );
        assert!(after.contains(":start-of-week 0"), "other keys preserved");
        assert_eq!(Graph::open(&dir).preferred_format(), Format::Org);
        // Inserts the key when absent.
        fs::write(dir.join("logseq").join("config.edn"), "{}\n").unwrap();
        Graph::open(&dir).set_preferred_format(Format::Org).unwrap();
        assert_eq!(Graph::open(&dir).preferred_format(), Format::Org);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_journal_page_title_format_round_trips() {
        use crate::model::Graph;
        let dir = std::env::temp_dir().join(format!("tine-cfgdate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Markdown\"\n :start-of-week 0}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.set_journal_page_title_format("yyyy-MM-dd").unwrap();
        let after = fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert!(
            after.contains(":journal/page-title-format \"yyyy-MM-dd\""),
            "not written: {after}"
        );
        assert!(
            after.contains(":preferred-format \"Markdown\""),
            "other keys clobbered: {after}"
        );
        assert_eq!(
            Config::parse(&after).journal_page_title_format.as_deref(),
            Some("yyyy-MM-dd")
        );
        // A second set replaces the value wholesale (no stale leftover).
        g.set_journal_page_title_format("MMMM do, yyyy").unwrap();
        let after2 = fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert!(
            after2.contains(":journal/page-title-format \"MMMM do, yyyy\""),
            "value not replaced: {after2}"
        );
        assert!(
            !after2.contains("\"yyyy-MM-dd\""),
            "stale value left behind: {after2}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
