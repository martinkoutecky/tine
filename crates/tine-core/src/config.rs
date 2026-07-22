//! `logseq/config.edn` read AND write — one cohesive module.
//!
//! We don't full-parse config.edn (it contains arbitrary Clojure/datalog forms —
//! `(…)` lists, `fn` bodies, `:where` rules — that a small EDN model can't safely
//! represent). Instead both reads and writes use ONE shared, string/comment/escape
//! -aware scanner family (`find_keyword`/`edn_str_end`/`match_close_*`/
//! `next_value_span`) to locate just the handful of keys we care about and edit
//! values surgically — so writes preserve comments + formatting + unrelated keys,
//! and reads are immune to whatever else the file contains.

use crate::model::Graph;
use std::collections::HashMap;
#[cfg(test)]
use std::fs; // only the tests touch the filesystem directly now (writers go via atomic_update)
use std::io;

#[derive(Debug, Clone)]
pub struct Config {
    pub journals_dir: String,
    pub pages_dir: String,
    pub preferred_workflow: Workflow,
    /// User keybinding overrides from `:shortcuts {:cmd "binding"}` (string
    /// bindings only; vectors take the first binding, `false` disables).
    pub shortcuts: HashMap<String, String>,
    /// `:publishing/all-pages-public?` — when true, HTML export publishes every
    /// page; otherwise only pages with `public:: true`.
    pub all_pages_public: bool,
    /// `:start-of-week` — first day of the week in the date picker. Logseq's
    /// convention: 0=Monday, 1=Tuesday … 6=Sunday (default 6). The frontend
    /// converts this to a JS getDay() index via (n+1)%7.
    pub start_of_week: u32,
    /// `:block-hidden-properties #{:a :b}` — extra property keys to hide from the
    /// rendered properties area, on top of the built-in internal set.
    pub block_hidden_properties: Vec<String>,
    /// `:property-pages/enabled?` — OG creates a page reference from every
    /// eligible property key unless this is explicitly false. Absent defaults to
    /// true (`block.cljs`: `(contains? #{true nil} enabled?)`).
    pub property_pages_enabled: bool,
    /// `:property-pages/excludelist #{:a :b}` — property keys that do not create
    /// property-page references. Values retain OG keyword names here and are
    /// folded with `property_key_norm` when matched.
    pub property_pages_excludelist: Vec<String>,
    /// `:default-templates {:journals "Name"}` — template applied to a new,
    /// empty journal page.
    pub default_journal_template: Option<String>,
    /// `:favorites ["Page" …]` — favorited page names (on-disk, graph-portable).
    pub favorites: Vec<String>,
    /// `:journal/file-name-format` — Logseq's journal FILENAME format (cljs-time /
    /// Joda tokens). `None` = the default `"yyyy_MM_dd"`. Tine only synthesizes
    /// the default format, so a non-default value here means Tine must NOT create
    /// new journal files (it would duplicate the user's real journal for the day).
    pub journal_file_name_format: Option<String>,
    /// `:journal/page-title-format` — Logseq's journal TITLE format. `None` = the
    /// default `"MMM do, yyyy"`. See `journal_file_name_format`.
    pub journal_page_title_format: Option<String>,
    /// `:preferred-format` — the format ("Markdown"/"Org") for NEW pages and
    /// journals. Existing files keep their own format (decided per-file by
    /// extension). Default markdown.
    pub preferred_format: crate::model::Format,
    /// `:file/name-format` — namespace-separator encoding in page filenames.
    /// Default (absent key) is `Legacy` (`%2F`), matching OG; modern graphs pin
    /// `:triple-lowbar` (`___`). See [`FileNameFormat`].
    pub file_name_format: FileNameFormat,
    /// `:macros {"name" "template" …}` — user-defined text-substitution macros.
    /// `$1..$N` (and `$ARG`) placeholders in the template are filled with the
    /// macro's comma-separated args at render time, then the result is rendered as
    /// markdown. We only collect the string→string pairs; the frontend substitutes
    /// and recurses.
    pub macros: HashMap<String, String>,
    /// `:feature/enable-timetracking?` — OG default ON; only explicit false
    /// disables marker-driven CLOCK entries.
    pub enable_timetracking: bool,
    /// `:ui/show-brackets?` — OG default ON; only explicit false hides the
    /// brackets around page references.
    pub show_brackets: bool,
    /// `:shortcut/doc-mode-enter-for-new-block?` — when document mode is on,
    /// retain the normal Enter = new block mapping. Absent defaults to false.
    pub doc_mode_enter_for_new_block: bool,
    /// `:editor/logical-outdenting?` — leave following siblings under the old
    /// parent when a block is outdented. Absent defaults to OG's false.
    pub logical_outdenting: bool,
    /// `:logbook/settings` — OG logbook write/display settings.
    pub logbook: LogbookSettings,
    /// Tine-owned graph-local flag for the one-time bundled Guide announcement.
    /// Stored in `logseq/config.edn` so it survives WebKitGTK's ephemeral
    /// localStorage and stays scoped to the graph.
    pub guide_announced: bool,
}

/// Logseq's default journal formats (verified against
/// `logseq/deps/common/src/logseq/common/util/date_time.cljs`). Tine recognizes
/// and synthesizes only these.
pub const DEFAULT_JOURNAL_FILE_FORMAT: &str = "yyyy_MM_dd";
pub const DEFAULT_JOURNAL_TITLE_FORMAT: &str = "MMM do, yyyy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workflow {
    /// NOW / LATER
    Now,
    /// TODO / DOING
    Todo,
}

/// `:file/name-format` — how a page name's namespace separator `/` (and reserved
/// characters) are encoded in the on-disk FILENAME. Logseq's two formats:
///
/// - `Legacy` — `/` → `%2F` (URL-encoded). **This is OG's default when the key is
///   absent** (`graph_parser/cli.cljs`: `(or (:file/name-format config) :legacy)`).
/// - `TripleLowbar` — `/` → `___` (triple underscore). What modern Logseq writes
///   into a freshly-created graph's config template.
///
/// Both decode percent-escapes on read; triple-lowbar additionally maps `___`↔`/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileNameFormat {
    Legacy,
    TripleLowbar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogbookSettings {
    pub with_second_support: bool,
    pub enabled_in_timestamped_blocks: bool,
    pub enabled_in_all_blocks: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            preferred_workflow: Workflow::Now,
            shortcuts: HashMap::new(),
            all_pages_public: false,
            start_of_week: 6, // Logseq's default (Sunday) — see field doc
            block_hidden_properties: Vec::new(),
            property_pages_enabled: true,
            property_pages_excludelist: Vec::new(),
            default_journal_template: None,
            favorites: Vec::new(),
            journal_file_name_format: None,
            journal_page_title_format: None,
            preferred_format: crate::model::Format::Md,
            file_name_format: FileNameFormat::Legacy,
            macros: HashMap::new(),
            enable_timetracking: true,
            show_brackets: true,
            doc_mode_enter_for_new_block: false,
            logical_outdenting: false,
            logbook: LogbookSettings::default(),
            guide_announced: false,
        }
    }
}

impl Default for LogbookSettings {
    fn default() -> Self {
        LogbookSettings {
            with_second_support: true,
            enabled_in_timestamped_blocks: true,
            enabled_in_all_blocks: false,
        }
    }
}

impl Config {
    pub fn parse(edn: &str) -> Config {
        // Each key is located independently with the comment/string-aware
        // `find_keyword`, then its value read with the shared scanners — no
        // up-front comment strip and no whole-file parse.
        let mut cfg = Config::default();
        if let Some(v) = string_value(edn, ":journals-directory") {
            cfg.journals_dir = v;
        }
        if let Some(v) = string_value(edn, ":pages-directory") {
            cfg.pages_dir = v;
        }
        if let Some(v) = keyword_value(edn, ":preferred-workflow") {
            cfg.preferred_workflow = if v == "todo" {
                Workflow::Todo
            } else {
                Workflow::Now
            };
        }
        cfg.shortcuts = parse_shortcuts(edn);
        cfg.all_pages_public = bool_value(edn, ":publishing/all-pages-public?").unwrap_or(false);
        if let Some(n) = int_value(edn, ":start-of-week") {
            if n <= 6 {
                cfg.start_of_week = n;
            }
        }
        cfg.block_hidden_properties = parse_keyword_set(edn, ":block-hidden-properties");
        cfg.property_pages_enabled = bool_value(edn, ":property-pages/enabled?").unwrap_or(true);
        cfg.property_pages_excludelist = parse_keyword_set(edn, ":property-pages/excludelist");
        cfg.default_journal_template =
            nested_string(edn, ":default-templates", ":journals").filter(|s| !s.is_empty());
        cfg.favorites = parse_string_vector(edn, ":favorites");
        cfg.journal_file_name_format =
            string_value(edn, ":journal/file-name-format").filter(|s| !s.is_empty());
        cfg.journal_page_title_format =
            string_value(edn, ":journal/page-title-format").filter(|s| !s.is_empty());
        // OG stores `:preferred-format "Markdown"|"Org"` (a capitalized string), but
        // its schema also accepts the keyword form `:preferred-format :org` — read
        // both so a keyword-configured graph isn't silently treated as markdown.
        if let Some(v) = string_value(edn, ":preferred-format")
            .or_else(|| keyword_value(edn, ":preferred-format"))
        {
            if v.eq_ignore_ascii_case("org") {
                cfg.preferred_format = crate::model::Format::Org;
            }
        }
        // `:file/name-format` is a keyword (`:triple-lowbar` | `:legacy`). Absent
        // ⇒ legacy, matching OG's `(or (:file/name-format config) :legacy)`.
        cfg.file_name_format = match keyword_value(edn, ":file/name-format").as_deref() {
            Some("triple-lowbar") => FileNameFormat::TripleLowbar,
            _ => FileNameFormat::Legacy,
        };
        cfg.macros = parse_macros(edn);
        cfg.enable_timetracking = bool_value(edn, ":feature/enable-timetracking?").unwrap_or(true);
        cfg.show_brackets = bool_value(edn, ":ui/show-brackets?").unwrap_or(true);
        cfg.doc_mode_enter_for_new_block =
            bool_value(edn, ":shortcut/doc-mode-enter-for-new-block?").unwrap_or(false);
        cfg.logical_outdenting = bool_value(edn, ":editor/logical-outdenting?").unwrap_or(false);
        cfg.logbook = LogbookSettings {
            with_second_support: nested_bool(edn, ":logbook/settings", ":with-second-support?")
                .unwrap_or(true),
            enabled_in_timestamped_blocks: nested_bool(
                edn,
                ":logbook/settings",
                ":enabled-in-timestamped-blocks",
            )
            .unwrap_or(true),
            enabled_in_all_blocks: nested_bool(edn, ":logbook/settings", ":enabled-in-all-blocks")
                .unwrap_or(false),
        };
        cfg.guide_announced = bool_value(edn, ":tine/guide-announced?").unwrap_or(false);
        cfg
    }

    pub(crate) fn property_page_key_enabled(&self, key: &str) -> bool {
        self.property_pages_enabled
            && !self.property_pages_excludelist.iter().any(|excluded| {
                crate::doc::property_key_norm(excluded) == crate::doc::property_key_norm(key)
            })
    }
}

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

// ---------------------------------------------------------------------------
// Shared scanner family — byte-offset, string/comment/escape-aware. Used by
// BOTH the readers above and the writers above. `;` comment handling lives in
// `find_keyword` (so there's no separate comment-strip pass).
// ---------------------------------------------------------------------------

/// Index just past the closing quote of an EDN string opening at byte `open` (a
/// `"`), skipping `\"` / `\\`. Returns end-of-string if unterminated. (`"` is
/// ASCII → the returned index is a char boundary.)
fn edn_str_end(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let mut i = open + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    s.len()
}

/// Matching close `}` for the map whose `{` is at byte `open`, EDN-aware: skips
/// strings, `;` comments, and nested braces. End-of-string if unbalanced.
fn match_close_brace(s: &str, open: usize) -> usize {
    match_close(s, open, b'{', b'}')
}

/// Matching close `]` for the vector whose `[` is at byte `open`, EDN-aware.
fn match_close_bracket(s: &str, open: usize) -> usize {
    match_close(s, open, b'[', b']')
}

fn match_close(s: &str, open: usize, openc: u8, closec: u8) -> usize {
    let b = s.as_bytes();
    let mut i = open + 1;
    let mut depth = 1usize;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            i = edn_str_end(s, i);
            continue;
        }
        if c == b';' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == openc {
            depth += 1;
        } else if c == closec {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    s.len()
}

/// Byte index of a real `key` keyword in `s`, skipping strings + `;` comments and
/// requiring a token boundary after it. None if absent. Linear scan (always
/// advances), so arbitrary `(…)`/`#{…}`/etc. content can't hang or mislead it.
fn find_keyword(s: &str, key: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i = edn_str_end(s, i);
                continue;
            }
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            _ if s[i..].starts_with(key) => {
                let after = i + key.len();
                let boundary = after >= b.len()
                    || matches!(
                        b[after],
                        b' ' | b'\t'
                            | b'\n'
                            | b'\r'
                            | b'"'
                            | b'{'
                            | b'}'
                            | b'['
                            | b']'
                            | b'('
                            | b')'
                            | b'#'
                            | b','
                    );
                if boundary {
                    return Some(i);
                }
                i = after;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Span `[start, end)` of the value token following byte `from` (skipping leading
/// whitespace/commas) within `..close`, plus whether it is an EDN string. None if
/// there is no value before `close`. A string's end is escape-aware; a non-string
/// token ends at the next whitespace/comma/brace/quote.
fn next_value_span(s: &str, from: usize, close: usize) -> Option<(usize, usize, bool)> {
    let b = s.as_bytes();
    let mut i = from;
    loop {
        while i < close && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
            i += 1;
        }
        if i < close && b[i] == b';' {
            while i < close && b[i] != b'\n' {
                i += 1; // a `;` comment between key and value isn't the value
            }
            continue;
        }
        break;
    }
    if i >= close {
        return None;
    }
    if b[i] == b'"' {
        return Some((i, edn_str_end(s, i).min(close), true));
    }
    let start = i;
    while i < close
        && !matches!(
            b[i],
            b' ' | b'\t' | b'\n' | b'\r' | b',' | b'{' | b'}' | b'"'
        )
    {
        i += 1;
    }
    Some((start, i, false))
}

// ---------------------------------------------------------------------------
// Readers — each finds its key with `find_keyword`, then reads the value with
// the shared scanners.
// ---------------------------------------------------------------------------

/// First non-blank byte at/after `from`, skipping whitespace, commas, and `;`
/// comments (a comment can sit between a key and its value).
fn skip_blank(s: &str, from: usize) -> usize {
    let b = s.as_bytes();
    let mut i = from;
    loop {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
            i += 1;
        }
        if i < b.len() && b[i] == b';' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        break;
    }
    i
}

/// Unescape `\"`→`"` and `\\`→`\` (the inverse of the writers' escaping); other
/// backslashes are kept literal.
fn unescape(inner: &str) -> String {
    let b = inner.as_bytes();
    let mut out = String::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        if b[i] == b'\\' && matches!(b.get(i + 1), Some(b'"') | Some(b'\\')) {
            out.push(b[i + 1] as char);
            i += 2;
        } else {
            let ch = inner[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Read the (unescaped) content of the EDN string whose opening `"` is at `open`.
fn read_string_at(s: &str, open: usize) -> String {
    let end = edn_str_end(s, open);
    let inner_end = if end > open + 1 && s.as_bytes()[end - 1] == b'"' {
        end - 1
    } else {
        end
    };
    unescape(&s[open + 1..inner_end])
}

/// String value following `key`, e.g. `:journals-directory "journals"`.
fn string_value(edn: &str, key: &str) -> Option<String> {
    let start = find_keyword(edn, key)?;
    let from = skip_blank(edn, start + key.len());
    (edn.as_bytes().get(from) == Some(&b'"')).then(|| read_string_at(edn, from))
}

/// Keyword value (`:foo` → `foo`) following `key`.
fn keyword_value(edn: &str, key: &str) -> Option<String> {
    let start = find_keyword(edn, key)?;
    let from = skip_blank(edn, start + key.len());
    let b = edn.as_bytes();
    if b.get(from) != Some(&b':') {
        return None;
    }
    let vstart = from + 1;
    let mut j = vstart;
    while j < b.len()
        && !matches!(
            b[j],
            b' ' | b'\t' | b'\n' | b'\r' | b',' | b'}' | b')' | b']'
        )
    {
        j += 1;
    }
    Some(edn[vstart..j].to_string())
}

/// Boolean value (`true`/`false`) following `key`.
fn bool_value(edn: &str, key: &str) -> Option<bool> {
    let start = find_keyword(edn, key)?;
    let from = skip_blank(edn, start + key.len());
    if edn[from..].starts_with("true") {
        Some(true)
    } else if edn[from..].starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Non-negative integer following `key`.
fn int_value(edn: &str, key: &str) -> Option<u32> {
    let start = find_keyword(edn, key)?;
    let from = skip_blank(edn, start + key.len());
    let digits: String = edn[from..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Quoted strings in the vector following `key` (`:favorites ["a" "b"]`),
/// string-aware so a value containing `]` doesn't end the vector early.
fn parse_string_vector(edn: &str, key: &str) -> Vec<String> {
    let Some(start) = find_keyword(edn, key) else {
        return Vec::new();
    };
    let from = skip_blank(edn, start + key.len());
    let b = edn.as_bytes();
    if b.get(from) != Some(&b'[') {
        return Vec::new();
    }
    let close = match_close_bracket(edn, from);
    let mut out = Vec::new();
    let mut i = from + 1;
    while i < close {
        match b[i] {
            b'"' => {
                out.push(read_string_at(edn, i));
                i = edn_str_end(edn, i);
            }
            b';' => {
                while i < close && b[i] != b'\n' {
                    i += 1; // skip a `;` comment (a commented-out entry isn't a value)
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// The quoted string for `inner` inside the map following `outer`, e.g.
/// `:default-templates {:journals "Daily"}` → "Daily". String/brace-aware.
fn nested_string(edn: &str, outer: &str, inner: &str) -> Option<String> {
    let start = find_keyword(edn, outer)?;
    let from = skip_blank(edn, start + outer.len());
    if edn.as_bytes().get(from) != Some(&b'{') {
        return None;
    }
    let close = match_close_brace(edn, from);
    let irel = find_keyword(&edn[from + 1..close], inner)?;
    let vfrom = skip_blank(edn, from + 1 + irel + inner.len());
    (edn.as_bytes().get(vfrom) == Some(&b'"')).then(|| read_string_at(edn, vfrom))
}

/// Boolean value for `inner` inside the map following `outer`, e.g.
/// `:logbook/settings {:with-second-support? false}`.
fn nested_bool(edn: &str, outer: &str, inner: &str) -> Option<bool> {
    let start = find_keyword(edn, outer)?;
    let from = skip_blank(edn, start + outer.len());
    if edn.as_bytes().get(from) != Some(&b'{') {
        return None;
    }
    let close = match_close_brace(edn, from);
    let irel = find_keyword(&edn[from + 1..close], inner)?;
    let vfrom = skip_blank(edn, from + 1 + irel + inner.len());
    if edn[vfrom..close].starts_with("true") {
        Some(true)
    } else if edn[vfrom..close].starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Keywords in the set following `key` (`:block-hidden-properties #{:a :b}`).
fn parse_keyword_set(edn: &str, key: &str) -> Vec<String> {
    let Some(start) = find_keyword(edn, key) else {
        return Vec::new();
    };
    let from = skip_blank(edn, start + key.len());
    let b = edn.as_bytes();
    if b.get(from) != Some(&b'#') || b.get(from + 1) != Some(&b'{') {
        return Vec::new();
    }
    let brace = from + 1;
    let close = match_close_brace(edn, brace);
    // Sets hold only keywords (no strings), so a `;` is always a comment — drop
    // the commented tail of each line before collecting keywords.
    edn[brace + 1..close]
        .lines()
        .map(|l| &l[..l.find(';').unwrap_or(l.len())])
        .flat_map(str::split_whitespace)
        .filter_map(|t| t.strip_prefix(':'))
        .map(str::to_string)
        .collect()
}

/// The `:shortcuts {…}` map as command-id → binding. Values: `"binding"` |
/// `false` (disable) | `["b1" "b2"]` (first wins). String/brace-aware.
fn parse_shortcuts(edn: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(start) = find_keyword(edn, ":shortcuts") else {
        return map;
    };
    let from = skip_blank(edn, start + ":shortcuts".len());
    let b = edn.as_bytes();
    if b.get(from) != Some(&b'{') {
        return map;
    }
    let close = match_close_brace(edn, from);
    let mut i = from + 1;
    while i < close {
        i = skip_blank(edn, i);
        if i >= close || b[i] != b':' {
            break; // not a keyword key — stop rather than desync
        }
        let kstart = i + 1;
        let mut j = kstart;
        while j < close && !matches!(b[j], b' ' | b'\t' | b'\n' | b'\r' | b',') {
            j += 1;
        }
        let key = edn[kstart..j].to_string();
        let vfrom = skip_blank(edn, j);
        if vfrom >= close {
            break;
        }
        match b[vfrom] {
            b'"' => {
                map.insert(key, read_string_at(edn, vfrom));
                i = edn_str_end(edn, vfrom);
            }
            b'[' => {
                let vclose = match_close_bracket(edn, vfrom);
                let mut k = vfrom + 1;
                while k < vclose {
                    if b[k] == b';' {
                        while k < vclose && b[k] != b'\n' {
                            k += 1; // skip a `;` comment before the first binding
                        }
                        continue;
                    }
                    if b[k] == b'"' {
                        map.insert(key.clone(), read_string_at(edn, k));
                        break;
                    }
                    k += 1;
                }
                i = vclose + 1;
            }
            _ => {
                if edn[vfrom..close].starts_with("false") {
                    map.insert(key, "false".to_string());
                }
                let mut k = vfrom;
                while k < close && !matches!(b[k], b' ' | b'\t' | b'\n' | b'\r' | b',') {
                    k += 1;
                }
                i = k;
            }
        }
    }
    map
}

/// `:macros {"name" "template" …}` — string→string map. Mirrors `parse_shortcuts`
/// but with STRING keys (macro names) rather than keyword keys. Values are template
/// strings (with `$1..$N` placeholders the frontend fills in). Stops cleanly on the
/// first non-string key/value rather than desyncing on unexpected EDN.
fn parse_macros(edn: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(start) = find_keyword(edn, ":macros") else {
        return map;
    };
    let from = skip_blank(edn, start + ":macros".len());
    let b = edn.as_bytes();
    if b.get(from) != Some(&b'{') {
        return map;
    }
    let close = match_close_brace(edn, from);
    let mut i = from + 1;
    while i < close {
        i = skip_blank(edn, i);
        if i >= close || b[i] != b'"' {
            break; // key must be a string
        }
        let key = read_string_at(edn, i);
        i = edn_str_end(edn, i);
        let vfrom = skip_blank(edn, i);
        if vfrom >= close || b[vfrom] != b'"' {
            break; // value must be a string
        }
        let val = read_string_at(edn, vfrom);
        i = edn_str_end(edn, vfrom);
        if !key.is_empty() {
            map.insert(key, val);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_macros_map() {
        let edn = r#"{:preferred-format "Markdown"
                      :macros {"poem" "Roses are $1, violets are $2"
                               "greet" "Hello, **$1**! See [[$2]]"}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(
            cfg.macros.get("poem").map(String::as_str),
            Some("Roses are $1, violets are $2")
        );
        assert_eq!(
            cfg.macros.get("greet").map(String::as_str),
            Some("Hello, **$1**! See [[$2]]")
        );
    }

    #[test]
    fn no_macros_section_is_empty() {
        assert!(Config::parse(r#"{:preferred-format "Markdown"}"#)
            .macros
            .is_empty());
    }

    #[test]
    fn parses_shortcuts_map() {
        let edn = r#"{:preferred-format "Markdown"
                      :shortcuts {:go/search "mod+shift+k"
                                  :ui/toggle-theme "t d"}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(
            cfg.shortcuts.get("go/search").map(String::as_str),
            Some("mod+shift+k")
        );
        assert_eq!(
            cfg.shortcuts.get("ui/toggle-theme").map(String::as_str),
            Some("t d")
        );
    }

    #[test]
    fn no_shortcuts_section_is_empty() {
        let cfg = Config::parse(r#"{:preferred-format "Markdown"}"#);
        assert!(cfg.shortcuts.is_empty());
    }

    #[test]
    fn parses_timetracking_defaults_and_logbook_settings() {
        let cfg = Config::parse("{}");
        assert!(cfg.enable_timetracking);
        assert!(cfg.logbook.with_second_support);
        assert!(cfg.logbook.enabled_in_timestamped_blocks);
        assert!(!cfg.logbook.enabled_in_all_blocks);

        let cfg = Config::parse(
            "{:feature/enable-timetracking? false
              :logbook/settings {:with-second-support? false
                                 :enabled-in-timestamped-blocks false
                                 :enabled-in-all-blocks true}}",
        );
        assert!(!cfg.enable_timetracking);
        assert!(!cfg.logbook.with_second_support);
        assert!(!cfg.logbook.enabled_in_timestamped_blocks);
        assert!(cfg.logbook.enabled_in_all_blocks);
    }

    #[test]
    fn property_pages_default_enabled_and_parse_og_controls() {
        let defaults = Config::parse("{}");
        assert!(defaults.property_pages_enabled);
        assert!(Config::parse("{:property-pages/enabled? nil}").property_pages_enabled);
        assert!(defaults.property_pages_excludelist.is_empty());
        assert!(defaults.property_page_key_enabled("url"));

        let configured = Config::parse(
            "{:property-pages/enabled? true
              :property-pages/excludelist #{:private_key :duration}}",
        );
        assert!(configured.property_pages_enabled);
        assert_eq!(
            configured.property_pages_excludelist,
            vec!["private_key", "duration"]
        );
        assert!(!configured.property_page_key_enabled("private-key"));
        assert!(!configured.property_page_key_enabled("duration"));
        assert!(configured.property_page_key_enabled("url"));

        let disabled = Config::parse("{:property-pages/enabled? false}");
        assert!(!disabled.property_pages_enabled);
        assert!(!disabled.property_page_key_enabled("url"));
    }

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
    fn shortcut_false_and_vector_forms() {
        let edn = r#"{:shortcuts {:go/search false
                                  :editor/indent ["tab" "mod+]"]}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(
            cfg.shortcuts.get("go/search").map(String::as_str),
            Some("false")
        );
        assert_eq!(
            cfg.shortcuts.get("editor/indent").map(String::as_str),
            Some("tab")
        );
    }

    #[test]
    fn parses_preferred_format() {
        use crate::model::Format;
        assert_eq!(
            Config::parse(r#"{:preferred-format "Org"}"#).preferred_format,
            Format::Org
        );
        assert_eq!(
            Config::parse(r#"{:preferred-format "org"}"#).preferred_format,
            Format::Org
        );
        assert_eq!(
            Config::parse(r#"{:preferred-format "Markdown"}"#).preferred_format,
            Format::Md
        );
        assert_eq!(Config::parse("{}").preferred_format, Format::Md);
    }

    #[test]
    fn parses_preferred_format_keyword_form() {
        use crate::model::Format;
        // M3: OG's schema also allows the keyword form `:preferred-format :org`.
        assert_eq!(
            Config::parse("{:preferred-format :org}").preferred_format,
            Format::Org
        );
        assert_eq!(
            Config::parse("{:preferred-format :markdown}").preferred_format,
            Format::Md
        );
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

    #[test]
    fn parses_file_name_format() {
        assert_eq!(
            Config::parse("{:file/name-format :triple-lowbar}").file_name_format,
            FileNameFormat::TripleLowbar
        );
        assert_eq!(
            Config::parse("{:file/name-format :legacy}").file_name_format,
            FileNameFormat::Legacy
        );
        // Absent ⇒ legacy (OG's default), NOT triple-lowbar.
        assert_eq!(Config::parse("{}").file_name_format, FileNameFormat::Legacy);
    }

    #[test]
    fn parses_favorites_vector() {
        let cfg = Config::parse(r#"{:favorites ["Inbox" "Reading List"]}"#);
        assert_eq!(
            cfg.favorites,
            vec!["Inbox".to_string(), "Reading List".to_string()]
        );
        assert!(Config::parse("{}").favorites.is_empty());
    }

    #[test]
    fn default_journal_template() {
        let edn = r#"{:default-templates {:journals "Daily"}}"#;
        assert_eq!(
            Config::parse(edn).default_journal_template.as_deref(),
            Some("Daily")
        );
        assert_eq!(
            Config::parse(r#"{:preferred-format "Markdown"}"#).default_journal_template,
            None
        );
    }

    #[test]
    fn start_of_week_and_hidden_properties() {
        let edn = r#"{:start-of-week 1
                      :block-hidden-properties #{:public :icon}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.start_of_week, 1);
        assert_eq!(
            cfg.block_hidden_properties,
            vec!["public".to_string(), "icon".to_string()]
        );
    }

    #[test]
    fn collection_readers_skip_comments_and_read_compact_edn() {
        // `;` comments INSIDE a vector/set are not read as values (the up-front
        // strip_edn_comments pass was removed; the collection scans skip comments).
        let cfg = Config::parse("{:favorites [\"A\" ;; \"B\"\n \"C\"]}");
        assert_eq!(cfg.favorites, vec!["A".to_string(), "C".to_string()]);
        let cfg = Config::parse("{:block-hidden-properties #{:public ;; :secret\n :icon}}");
        assert_eq!(
            cfg.block_hidden_properties,
            vec!["public".to_string(), "icon".to_string()]
        );
        // Compact (whitespace-free) EDN: the key boundary now includes `[` / `#`.
        assert_eq!(
            Config::parse("{:favorites[\"X\"]}").favorites,
            vec!["X".to_string()]
        );
        assert_eq!(
            Config::parse("{:block-hidden-properties#{:id}}").block_hidden_properties,
            vec!["id".to_string()]
        );
    }

    #[test]
    fn ignores_datalog_and_paren_forms_around_keys() {
        // A real config has (…) lists / #{…} sets / nested vectors; targeted key
        // reads must skip all of it and still find the simple keys (no hang).
        let edn = r#"{:default-queries
                       [{:query [:find (pull ?h [*]) :where [(contains? #{"NOW"} ?m)]]
                         :result-transform (fn [r] (sort-by (fn [h] (get h :x)) r))}]
                      :journals-directory "diary"
                      :start-of-week 2}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.journals_dir, "diary");
        assert_eq!(cfg.start_of_week, 2);
    }
}

#[cfg(test)]
mod commented_dirs_test {
    use super::*;
    #[test]
    fn commented_example_dirs_fall_back_to_defaults() {
        let edn = r#"{
 ;; :preferred-format ""
 ;; if not specified, notes are stored in `pages` directory
 ;; :pages-directory "your-directory"
 ;; if not specified, journals are stored in `journals` directory
 ;; :journals-directory "your-directory"
}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.journals_dir, "journals");
        assert_eq!(cfg.pages_dir, "pages");
    }
}
