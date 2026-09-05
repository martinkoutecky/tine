//! `logseq/config.edn` read AND write — one cohesive module.
//!
//! We don't full-parse config.edn (it contains arbitrary Clojure/datalog forms —
//! `(…)` lists, `fn` bodies, `:where` rules — that a small EDN model can't safely
//! represent). Instead both reads and writes use ONE shared, string/comment/escape
//! -aware scanner family (`find_keyword`/`edn_str_end`/`match_close_*`/
//! `next_value_span`) to locate just the handful of keys we care about and edit
//! values surgically — so writes preserve comments + formatting + unrelated keys,
//! and reads are immune to whatever else the file contains.

use crate::graph_text_scope::{
    MAX_HIDDEN_EDN_BYTES, MAX_HIDDEN_EDN_DEPTH, MAX_HIDDEN_EDN_ENTRIES, MAX_HIDDEN_EDN_FORMS,
};
use crate::model::Graph;
use std::collections::HashMap;
#[cfg(test)]
use std::fs; // only the tests touch the filesystem directly now (writers go via atomic_update)
use std::io;

#[derive(Debug, Clone)]
pub struct Config {
    pub journals_dir: String,
    pub pages_dir: String,
    /// OG `:hidden` graph-relative string prefixes. Interpretation belongs to
    /// the versioned graph text scope rather than individual callers.
    pub hidden: Vec<String>,
    /// A malformed or over-limit `:hidden` value cannot safely be treated as an
    /// empty exclusion list. The graph-text classifier turns this into hide-all.
    pub hidden_parse_failed_closed: bool,
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
    /// `:property/separated-by-commas #{:a :b}` — extra property keys whose plain
    /// value OG splits on commas. One of the five inputs to [`ParseConfig`].
    pub separated_by_commas: Vec<String>,
    /// `:ignored-page-references-keywords #{:a :b}` — property keys whose value OG
    /// keeps as one unparsed string. One of the five inputs to [`ParseConfig`].
    pub ignored_page_references_keywords: Vec<String>,
    /// `:ref/linked-references-collapsed-threshold` — a page's Linked References
    /// section starts collapsed once the TOTAL backlink count reaches this,
    /// which is OG's `(>= total threshold)` in `components/reference.cljs`
    /// (`6e7afa8e`). Absent or non-integer means OG's default 100. Zero is a
    /// meaningful value, not "off": it collapses the section always, which is
    /// what the discussion thread behind GH #479 asks for.
    pub linked_references_collapsed_threshold: u32,
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
    /// `:default-home {:page "Name"}` — graph-portable startup page. Other
    /// keys in the map belong to Logseq and are preserved by the writer.
    pub default_home: Option<String>,
    /// `:favorites ["Page" …]` — favorited page names (on-disk, graph-portable).
    pub favorites: Vec<String>,
    /// `:tine/favorites-page "Name"` — the page holding Tine's Favorites
    /// arrangement (groups and order). Tine-only; Logseq ignores unknown keys.
    /// Identity lives here rather than in a reserved page NAME so that a user's
    /// own page called "Favorites" is never silently treated as Tine's, and
    /// rather than in a page property so that resolving it costs nothing on the
    /// reference path (see `refs::ReferenceSourceExclusions`).
    pub favorites_page: Option<String>,
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
            hidden: Vec::new(),
            hidden_parse_failed_closed: false,
            preferred_workflow: Workflow::Now,
            shortcuts: HashMap::new(),
            all_pages_public: false,
            start_of_week: 6, // Logseq's default (Sunday) — see field doc
            block_hidden_properties: Vec::new(),
            separated_by_commas: Vec::new(),
            ignored_page_references_keywords: Vec::new(),
            linked_references_collapsed_threshold: 100, // OG default — see field doc
            property_pages_enabled: true,
            property_pages_excludelist: Vec::new(),
            default_journal_template: None,
            default_home: None,
            favorites: Vec::new(),
            favorites_page: None,
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
        match parse_hidden_paths(edn) {
            HiddenParse::Valid(hidden) => cfg.hidden = hidden,
            HiddenParse::FailedClosed => cfg.hidden_parse_failed_closed = true,
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
        cfg.separated_by_commas = parse_keyword_set(edn, ":property/separated-by-commas");
        cfg.ignored_page_references_keywords =
            parse_keyword_set(edn, ":ignored-page-references-keywords");
        if let Some(n) = int_value(edn, ":ref/linked-references-collapsed-threshold") {
            cfg.linked_references_collapsed_threshold = n;
        }
        cfg.property_pages_enabled = bool_value(edn, ":property-pages/enabled?").unwrap_or(true);
        cfg.property_pages_excludelist = parse_keyword_set(edn, ":property-pages/excludelist");
        cfg.default_journal_template =
            nested_string(edn, ":default-templates", ":journals").filter(|s| !s.is_empty());
        cfg.default_home = nested_string_in_balanced_map(edn, ":default-home", ":page")
            .filter(|s| !s.trim().is_empty());
        cfg.favorites = parse_string_vector(edn, ":favorites");
        cfg.favorites_page =
            string_value(edn, ":tine/favorites-page").filter(|s| !s.trim().is_empty());
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
/// goes through `self.write_config(&path, …)`, which also
/// makes the read NFS-safe (NotFound→`{}`, other errors abort — audit H2) and the
/// commit atomic.
static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn config_path_for_write(graph: &Graph) -> io::Result<std::path::PathBuf> {
    let path = graph.root.join("logseq").join("config.edn");
    graph.ensure_config_write_target(&path)?;
    Ok(path)
}

impl Graph {
    /// Record which page holds the Favorites arrangement, as
    /// `:tine/favorites-page "Name"`. Logseq ignores unknown keys, so this is
    /// invisible to it; `:favorites` remains the shared membership list.
    ///
    /// Surgical and key-local like `set_favorites`: unknown keys, comments and
    /// formatting elsewhere in the file survive untouched. The existing value is
    /// located with the comment/string-aware `find_keyword` and replaced only
    /// when it really is a string, so a stray non-string value is appended
    /// beside rather than mis-scanned.
    /// Publish one configuration edit.
    ///
    /// Every setter goes through here rather than calling `atomic_update`
    /// directly, so a self-write is always recorded. Without that record the
    /// configuration watcher cannot tell Tine's own settings write from an
    /// outside one, and every star toggled in the sidebar would cost a
    /// whole-graph reopen — which discards every cache the graph has built.
    fn write_config(
        &self,
        path: &std::path::Path,
        edit: impl Fn(&str) -> io::Result<String>,
    ) -> io::Result<()> {
        crate::model::atomic_update(path, &CONFIG_LOCK, edit)?;
        self.note_config_write();
        Ok(())
    }

    pub fn set_favorites_page(&self, name: &str) -> io::Result<()> {
        let path = config_path_for_write(self)?;
        let quoted = format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""));
        self.write_config(&path, |content| {
            let mut content = content.to_string();
            const KEY: &str = ":tine/favorites-page";
            if let Some(start) = find_top_level_keyword(&content, KEY) {
                let after = start + KEY.len();
                let j = skip_blank(&content, after);
                if content.as_bytes().get(j) == Some(&b'"') {
                    let end = edn_str_end(&content, j);
                    content.replace_range(start..end, &format!("{KEY} {quoted}"));
                } else {
                    content.insert_str(after, &format!(" {quoted}"));
                }
            } else if let Some(brace) = content.find('{') {
                content.insert_str(brace + 1, &format!("\n {KEY} {quoted}\n"));
            } else {
                content = format!("{{{KEY} {quoted}}}\n");
            }
            Ok(content)
        })
    }

    /// Persist the favorites list to `:favorites [...]`, replacing the existing
    /// vector or inserting one, preserving the rest of the file.
    pub fn set_favorites(&self, names: &[String]) -> io::Result<()> {
        let path = config_path_for_write(self)?;
        self.write_config(&path, |content| {
            let mut content = content.to_string();
            let vec_str = format!(
                "[{}]",
                names
                    .iter()
                    .map(|n| format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\"")))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            if let Some(start) = find_top_level_keyword(&content, ":favorites") {
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
        self.write_config(&path, |content| {
            let mut content = content.to_string();

            if let Some(start) = find_top_level_keyword(&content, key) {
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
        self.set_config_bool(":feature/enable-timetracking?", enabled)
    }

    /// Persist `:ui/show-brackets?`. OG treats an absent key as ON, but writing
    /// the explicit boolean keeps the Settings toggle reversible.
    pub fn set_show_brackets(&self, enabled: bool) -> io::Result<()> {
        self.set_config_bool(":ui/show-brackets?", enabled)
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
        self.write_config(&path, |content| {
            let mut content = content.to_string();

            if let Some(start) = find_top_level_keyword(&content, key) {
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
        self.set_config_bool(":tine/guide-announced?", announced)
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
        self.write_config(&path, |content| {
            let mut content = content.to_string();

            if let Some(start) = find_top_level_keyword(&content, key) {
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
        self.write_config(&path, |content| {
            let mut content = content.to_string();

            if let Some(start) = find_top_level_keyword(&content, key) {
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
        self.write_config(&path, |content| {
            let mut content = content.to_string();

            // Locate a real `:default-templates` whose value is a map literal `{ … }`.
            let dt = find_top_level_keyword(&content, ":default-templates").and_then(|start| {
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

    /// Persist the graph's startup page in Logseq's `:default-home {:page
    /// "Name"}` map. Sibling keys (notably OG's `:sidebar`) and the rest of the
    /// file remain byte-for-byte untouched. A malformed/non-map `:default-home`
    /// is refused rather than replaced, so an automatic legacy migration can
    /// never destroy graph-owned configuration it does not understand.
    pub fn set_default_home_page(&self, name: Option<&str>) -> io::Result<()> {
        let path = config_path_for_write(self)?;
        self.write_config(&path, |content| {
            let mut content = content.to_string();
            let (root_open, root_close) = root_map_bounds(&content).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "config.edn is not a balanced root map",
                )
            })?;
            let existing =
                find_keyword_at_map_level(&content[root_open + 1..root_close], ":default-home")
                    .map(|relative| {
                        let start = root_open + 1 + relative;
                        let after = start + ":default-home".len();
                        let open = skip_blank(&content, after);
                        if content.as_bytes().get(open) != Some(&b'{') {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                ":default-home exists but is not a map",
                            ));
                        }
                        let close = match_close_brace(&content, open);
                        if close >= content.len() || content.as_bytes().get(close) != Some(&b'}') {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                ":default-home map is not balanced",
                            ));
                        }
                        Ok((open, close))
                    });
            let existing = match existing {
                Some(result) => Some(result?),
                None => None,
            };

            match name.map(str::trim).filter(|name| !name.is_empty()) {
                Some(name) => {
                    let value = format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""));
                    match existing {
                        Some((open, close)) => {
                            if let Some(relative) =
                                find_keyword_at_map_level(&content[open + 1..close], ":page")
                            {
                                let after = open + 1 + relative + ":page".len();
                                match next_value_span(&content, after, close) {
                                    Some((value_start, value_end, _)) => {
                                        content.replace_range(value_start..value_end, &value)
                                    }
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
                        None => {
                            let entry = format!("\n :default-home {{:page {value}}}\n");
                            content.insert_str(root_open + 1, &entry);
                        }
                    }
                }
                None => {
                    if let Some((open, close)) = existing {
                        if let Some(relative) =
                            find_keyword_at_map_level(&content[open + 1..close], ":page")
                        {
                            let start = open + 1 + relative;
                            let after = start + ":page".len();
                            let end = next_value_span(&content, after, close)
                                .map(|(_, value_end, _)| value_end)
                                .unwrap_or(after);
                            let tail: usize = content[end..close]
                                .chars()
                                .take_while(|c| c.is_whitespace() || *c == ',')
                                .map(char::len_utf8)
                                .sum();
                            content.replace_range(start..end + tail, "");
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
        self.write_config(&path, |content| {
            let mut content = content.to_string();

            if let Some(start) = find_top_level_keyword(&content, key) {
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

/// Find a keyword only among the direct entries of an already-sliced map body.
/// Nested maps/vectors/lists may legally contain the same keyword and are not
/// the setting being read or edited.
fn find_keyword_at_map_level(s: &str, key: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut index = 0usize;
    let mut depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index = edn_str_end(s, index);
                continue;
            }
            b';' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                continue;
            }
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && s[index..].starts_with(key) => {
                let after = index + key.len();
                let boundary = after >= bytes.len()
                    || matches!(
                        bytes[after],
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
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Bounds of the root EDN map. Config writers may create entries in the empty
/// `{}` supplied for a missing file, but must not replace non-map or unbalanced
/// bytes that may belong to a newer/partially-written configuration shape.
fn root_map_bounds(s: &str) -> Option<(usize, usize)> {
    let open = skip_blank(s, 0);
    if s.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let close = match_close_brace(s, open);
    (close < s.len() && s.as_bytes().get(close) == Some(&b'}')).then_some((open, close))
}

/// Locate `key` among the DIRECT entries of the root config map (byte index
/// into the full string), never inside a nested map/vector/list. The
/// depth-blind `find_keyword` returns the FIRST occurrence anywhere — so a
/// `:favorites` nested inside `:default-templates` shadowed the real top-level
/// entry, and a setter splicing its replacement over the nested hit corrupted
/// config.edn (DUP-3, 2026-08-25 duplication audit). Every top-level SETTER
/// must locate its key through this helper; `None` (absent at top level, or no
/// balanced root map yet) sends callers to their ordinary insert/create path,
/// which inserts at the root map's opening brace — BEFORE any nested shadow in
/// byte order, so the depth-blind readers still see the top-level entry first.
fn find_top_level_keyword(s: &str, key: &str) -> Option<usize> {
    let (open, close) = root_map_bounds(s)?;
    find_keyword_at_map_level(&s[open + 1..close], key).map(|relative| open + 1 + relative)
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

/// The `:hidden` value is security-relevant input: treating a malformed value as
/// absent can admit a file the graph owner meant to exclude. Keep its reader
/// independent from the deliberately permissive scanners used for display
/// preferences, and bound the encoded value, recursive form depth, total form
/// count, and number of top-level collection entries before allocating decoded
/// strings.
enum HiddenParse {
    Valid(Vec<String>),
    FailedClosed,
}

fn parse_hidden_paths(edn: &str) -> HiddenParse {
    parse_hidden_paths_inner(edn).unwrap_or(HiddenParse::FailedClosed)
}

fn parse_hidden_paths_inner(edn: &str) -> Result<HiddenParse, ()> {
    let mut reader = EdnReader::new(edn);
    reader.skip_interstitial_and_discards()?;
    if reader.peek() != Some(b'{') {
        return Err(());
    }
    reader.begin_form()?;
    reader.pos += 1;
    let mut hidden = None;
    let result = (|| {
        loop {
            reader.skip_interstitial_and_discards()?;
            match reader.peek() {
                Some(b'}') => {
                    reader.pos += 1;
                    break;
                }
                None => return Err(()),
                _ => {}
            }

            let start = reader.pos;
            let keyword = reader.peek() == Some(b':');
            reader.skip_form()?;
            let key = keyword.then_some(&edn[start..reader.pos]);
            reader.skip_interstitial_and_discards()?;
            if matches!(reader.peek(), None | Some(b'}')) {
                return Err(());
            }
            if key == Some(":hidden") {
                hidden = Some(reader.read_hidden_value()?);
            } else {
                reader.skip_form()?;
            }
        }
        reader.skip_interstitial_and_discards()?;
        (reader.pos == edn.len()).then_some(()).ok_or(())
    })();
    reader.end_form();
    result?;
    Ok(HiddenParse::Valid(hidden.unwrap_or_default()))
}

struct EdnReader<'a> {
    source: &'a str,
    pos: usize,
    limit: usize,
    depth: usize,
    forms: usize,
}

impl<'a> EdnReader<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            pos: 0,
            limit: source.len(),
            depth: 0,
            forms: 0,
        }
    }

    fn bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }

    fn peek(&self) -> Option<u8> {
        (self.pos < self.limit)
            .then(|| self.bytes().get(self.pos).copied())
            .flatten()
    }

    fn skip_interstitial(&mut self) -> Result<(), ()> {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b',')) {
                self.pos += 1;
            }
            if self.peek() == Some(b';') {
                while self.peek().is_some_and(|byte| byte != b'\n') {
                    self.pos += 1;
                }
                continue;
            }
            return Ok(());
        }
    }

    fn skip_interstitial_and_discards(&mut self) -> Result<(), ()> {
        loop {
            self.skip_interstitial()?;
            if self.peek() != Some(b'#') || self.bytes().get(self.pos + 1) != Some(&b'_') {
                return Ok(());
            }
            self.begin_form()?;
            self.pos += 2;
            let result = self.skip_form();
            self.end_form();
            result?;
        }
    }

    fn begin_form(&mut self) -> Result<(), ()> {
        self.forms = self.forms.checked_add(1).ok_or(())?;
        if self.forms > MAX_HIDDEN_EDN_FORMS {
            return Err(());
        }
        self.depth = self.depth.checked_add(1).ok_or(())?;
        if self.depth > MAX_HIDDEN_EDN_DEPTH {
            return Err(());
        }
        Ok(())
    }

    fn end_form(&mut self) {
        self.depth -= 1;
    }

    fn skip_form(&mut self) -> Result<(), ()> {
        self.skip_interstitial_and_discards()?;
        self.begin_form()?;
        let result = self.skip_form_body();
        self.end_form();
        result
    }

    fn skip_form_body(&mut self) -> Result<(), ()> {
        match self.peek().ok_or(())? {
            b'"' => self.scan_string(false).map(|_| ()),
            b'[' => self.skip_collection(b']', false),
            b'{' => self.skip_collection(b'}', true),
            b'(' => self.skip_collection(b')', false),
            b'#' if self.bytes().get(self.pos + 1) == Some(&b'{') => {
                self.pos += 1;
                self.skip_collection(b'}', false)
            }
            b'#' if self.bytes().get(self.pos + 1) == Some(&b'(') => {
                self.pos += 1;
                self.skip_collection(b')', false)
            }
            b'#' if self.bytes().get(self.pos + 1) == Some(&b'"') => {
                self.pos += 1;
                self.scan_string(false).map(|_| ())
            }
            b'#' if self.bytes().get(self.pos + 1) == Some(&b'\'') => {
                self.pos += 2;
                self.skip_form()
            }
            b'#' if self.bytes().get(self.pos + 1) == Some(&b'#') => self.skip_atom(),
            b'#' => {
                self.pos += 1;
                self.skip_atom()?;
                self.skip_form()
            }
            b'\'' | b'`' | b'@' => {
                self.pos += 1;
                self.skip_form()
            }
            b'~' => {
                self.pos += 1;
                if self.peek() == Some(b'@') {
                    self.pos += 1;
                }
                self.skip_form()
            }
            b'^' => {
                self.pos += 1;
                self.skip_form()?;
                self.skip_form()
            }
            b'\\' => self.skip_character(),
            b']' | b'}' | b')' => Err(()),
            _ => self.skip_atom(),
        }
    }

    fn skip_collection(&mut self, close: u8, map: bool) -> Result<(), ()> {
        self.pos += 1;
        let mut forms = 0usize;
        loop {
            self.skip_interstitial_and_discards()?;
            match self.peek() {
                Some(byte) if byte == close => {
                    self.pos += 1;
                    if map && forms % 2 != 0 {
                        return Err(());
                    }
                    return Ok(());
                }
                None => return Err(()),
                _ => {
                    self.skip_form()?;
                    forms = forms.checked_add(1).ok_or(())?;
                }
            }
        }
    }

    fn skip_atom(&mut self) -> Result<(), ()> {
        let start = self.pos;
        while self.peek().is_some_and(|byte| {
            !matches!(
                byte,
                b' ' | b'\t'
                    | b'\n'
                    | b'\r'
                    | b','
                    | b';'
                    | b'"'
                    | b'['
                    | b']'
                    | b'{'
                    | b'}'
                    | b'('
                    | b')'
            )
        }) {
            self.pos += 1;
        }
        (self.pos != start).then_some(()).ok_or(())
    }

    fn skip_character(&mut self) -> Result<(), ()> {
        self.pos += 1;
        let character = self.source[self.pos..].chars().next().ok_or(())?;
        self.pos = self.pos.checked_add(character.len_utf8()).ok_or(())?;
        while self.peek().is_some_and(|byte| {
            !matches!(
                byte,
                b' ' | b'\t'
                    | b'\n'
                    | b'\r'
                    | b','
                    | b';'
                    | b'['
                    | b']'
                    | b'{'
                    | b'}'
                    | b'('
                    | b')'
            )
        }) {
            self.pos += 1;
        }
        Ok(())
    }

    fn scan_string(&mut self, decode: bool) -> Result<Option<String>, ()> {
        if self.peek() != Some(b'"') {
            return Err(());
        }
        self.pos += 1;
        let mut output = decode.then(String::new);
        loop {
            let byte = self.peek().ok_or(())?;
            if byte == b'"' {
                self.pos += 1;
                return Ok(output);
            }
            if byte == b'\\' {
                self.pos += 1;
                let escaped = self.peek().ok_or(())?;
                self.pos += 1;
                let character = match escaped {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'b' => '\u{0008}',
                    b'f' => '\u{000c}',
                    b'u' => {
                        let end = self.pos.checked_add(4).ok_or(())?;
                        if end > self.limit || end > self.source.len() {
                            return Err(());
                        }
                        let digits = &self.source[self.pos..end];
                        let value = u32::from_str_radix(digits, 16).map_err(|_| ())?;
                        self.pos = end;
                        char::from_u32(value).ok_or(())?
                    }
                    _ => return Err(()),
                };
                if let Some(output) = output.as_mut() {
                    output.push(character);
                }
                continue;
            }
            let character = self.source[self.pos..]
                .chars()
                .next()
                .filter(|character| self.pos + character.len_utf8() <= self.limit)
                .ok_or(())?;
            self.pos += character.len_utf8();
            if let Some(output) = output.as_mut() {
                output.push(character);
            }
        }
    }

    fn read_hidden_value(&mut self) -> Result<Vec<String>, ()> {
        let start = self.pos;
        let previous_limit = self.limit;
        self.limit = previous_limit.min(start.checked_add(MAX_HIDDEN_EDN_BYTES).ok_or(())?);
        let result = (|| {
            self.skip_interstitial_and_discards()?;
            if self.peek() != Some(b'[') {
                self.skip_form()?;
                return Ok(Vec::new());
            }
            self.begin_form()?;
            self.pos += 1;
            let value = (|| {
                let mut entries = 0usize;
                let mut values = Vec::new();
                loop {
                    self.skip_interstitial_and_discards()?;
                    match self.peek() {
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(values);
                        }
                        None => return Err(()),
                        Some(b'"') => {
                            entries = entries.checked_add(1).ok_or(())?;
                            if entries > MAX_HIDDEN_EDN_ENTRIES {
                                return Err(());
                            }
                            self.begin_form()?;
                            let string = self.scan_string(true);
                            self.end_form();
                            values.push(string?.ok_or(())?);
                        }
                        Some(_) => {
                            entries = entries.checked_add(1).ok_or(())?;
                            if entries > MAX_HIDDEN_EDN_ENTRIES {
                                return Err(());
                            }
                            self.skip_form()?;
                        }
                    }
                }
            })();
            self.end_form();
            value
        })();
        self.limit = previous_limit;
        result
    }
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

/// Like `nested_string`, but rejects an unbalanced outer map. Preferences may
/// fall back when malformed; a graph-owner migration must not mistake a partial
/// form for authority and then rewrite it.
fn nested_string_in_balanced_map(edn: &str, outer: &str, inner: &str) -> Option<String> {
    let (root_open, root_close) = root_map_bounds(edn)?;
    let relative = find_keyword_at_map_level(&edn[root_open + 1..root_close], outer)?;
    let start = root_open + 1 + relative;
    let from = skip_blank(edn, start + outer.len());
    if edn.as_bytes().get(from) != Some(&b'{') {
        return None;
    }
    let close = match_close_brace(edn, from);
    if close >= edn.len() || edn.as_bytes().get(close) != Some(&b'}') {
        return None;
    }
    let inner_relative = find_keyword_at_map_level(&edn[from + 1..close], inner)?;
    let value_start = skip_blank(edn, from + 1 + inner_relative + inner.len());
    (edn.as_bytes().get(value_start) == Some(&b'"')).then(|| read_string_at(edn, value_start))
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

/// The five graph-config facts that decide **projected page facts and property
/// atomization** (SPEC §5.8 M21, C3).
///
/// Why exactly these five and why they travel together: `JournalFormat::new(
/// file_name_format, title_format)` and `decode_page_name(stem, file_name_format)`
/// decide every page's name, kind and `date_key`, and the atomizer's comma-split
/// and unparsed-key rules read the two keyword sets. Direct reconciliation
/// compares only source revisions, so an omitted field would leave unchanged
/// files with stale `pages` rows after a config edit — which is why the digest
/// over these five (Wave B, §5.8 H6) is what forces a projection rebuild.
///
/// This is a read-only projection of [`Config`], never a second source of truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseConfig {
    pub separated_by_commas: Vec<String>,
    pub ignored_page_references_keywords: Vec<String>,
    pub journal_page_title_format: Option<String>,
    pub journal_file_name_format: Option<String>,
    pub file_name_format: FileNameFormat,
}

impl Default for ParseConfig {
    fn default() -> Self {
        Config::default().parse_config()
    }
}

impl Config {
    /// The parse-relevant slice of this config (SPEC §5.8). `ParseConfig::digest()`
    /// — the on-disk stamp that forces a rebuild — is Wave B of P0-rust; it needs
    /// tine-storage's `ContentDigest` and the atom-key normalization, neither of
    /// which exists on this path yet.
    pub fn parse_config(&self) -> ParseConfig {
        ParseConfig {
            separated_by_commas: self.separated_by_commas.clone(),
            ignored_page_references_keywords: self.ignored_page_references_keywords.clone(),
            journal_page_title_format: self.journal_page_title_format.clone(),
            journal_file_name_format: self.journal_file_name_format.clone(),
            file_name_format: self.file_name_format,
        }
    }
}

#[cfg(test)]
mod parse_config_tests {
    use super::*;

    #[test]
    fn both_new_keyword_sets_are_read_from_config_edn() {
        let cfg = Config::parse(
            "{:property/separated-by-commas #{:tags :authors}\n :ignored-page-references-keywords #{:url :source}}",
        );
        assert_eq!(cfg.separated_by_commas, vec!["tags", "authors"]);
        assert_eq!(cfg.ignored_page_references_keywords, vec!["url", "source"]);
        let parse = cfg.parse_config();
        assert_eq!(parse.separated_by_commas, vec!["tags", "authors"]);
        assert_eq!(
            parse.ignored_page_references_keywords,
            vec!["url", "source"]
        );
    }

    #[test]
    fn absent_keyword_sets_are_empty_never_defaulted() {
        let cfg = Config::parse("{:journals-directory \"journals\"}");
        assert!(cfg.separated_by_commas.is_empty());
        assert!(cfg.ignored_page_references_keywords.is_empty());
    }

    #[test]
    fn parse_config_carries_all_five_projected_fact_inputs() {
        let cfg = Config::parse(
            "{:journal/page-title-format \"yyyy-MM-dd\"\n :journal/file-name-format \"yyyy_MM_dd\"\n :file/name-format :triple-lowbar}",
        );
        let parse = cfg.parse_config();
        assert_eq!(
            parse.journal_page_title_format.as_deref(),
            Some("yyyy-MM-dd")
        );
        assert_eq!(
            parse.journal_file_name_format.as_deref(),
            Some("yyyy_MM_dd")
        );
        assert_eq!(parse.file_name_format, FileNameFormat::TripleLowbar);
    }
}

#[cfg(test)]
mod tests {

    /// DUP-3 (2026-08-25 duplication audit): setters located their key with the
    /// depth-blind `find_keyword`, which returns the FIRST occurrence anywhere.
    /// With a `:favorites` nested inside `:default-templates` ahead of the real
    /// top-level entry, `set_favorites` spliced its vector over the NESTED one,
    /// corrupting the map. Setters must edit only direct root-map entries.
    #[test]
    fn setters_edit_the_top_level_key_never_a_nested_shadow() {
        let dir = std::env::temp_dir().join(format!(
            "tine-config-nested-shadow-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("logseq")).unwrap();
        let path = dir.join("logseq").join("config.edn");
        std::fs::write(
            &path,
            "{:default-templates {:journals \"J\" :favorites [\"nested\"]}\n :favorites [\"real\"]}\n",
        )
        .unwrap();

        let g = crate::model::Graph::open(&dir);
        g.set_favorites(&["Replaced".to_owned()]).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains(":default-templates {:journals \"J\" :favorites [\"nested\"]}"),
            "the nested shadow must survive byte-for-byte, got: {after}"
        );
        assert!(
            after.contains(":favorites [\"Replaced\"]"),
            "the real top-level entry must be the one replaced, got: {after}"
        );
        assert!(
            !after.contains("[\"real\"]"),
            "old top-level value gone, got: {after}"
        );

        // A key that exists ONLY nested gets a NEW top-level entry; the nested
        // copy is not the setting and must not be edited.
        std::fs::write(&path, "{:default-queries {:preferred-format :org}}\n").unwrap();
        let g = crate::model::Graph::open(&dir);
        g.set_preferred_format(crate::model::Format::Md).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains(":default-queries {:preferred-format :org}"),
            "nested copy untouched, got: {after}"
        );
        assert!(
            after.contains(":preferred-format \"Markdown\""),
            "new top-level entry inserted, got: {after}"
        );
    }

    // :tine/favorites-page identifies the page holding the Favorites
    // arrangement. Logseq ignores unknown keys, so the round trip must leave
    // every other key, comment and bit of formatting exactly as it found it —
    // a malformed favorites value once invalidated Logseq's whole config parse,
    // and this writer runs on the user's real config.edn.
    /// The watcher's whole economy rests on this: it may only pay for a
    /// whole-graph reopen when the configuration on disk differs from the bytes
    /// the running `Graph` was opened with. A settings write Tine performed
    /// itself already refreshed the graph, so it must read as unchanged.
    #[test]
    fn a_graph_reports_whether_config_edn_moved_since_it_was_opened() {
        let dir = std::env::temp_dir().join(format!(
            "tine-config-witness-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("logseq")).unwrap();
        std::fs::write(
            dir.join("logseq").join("config.edn"),
            "{:favorites [\"Alpha\"]}\n",
        )
        .unwrap();

        let g = crate::model::Graph::open(&dir);
        assert_eq!(
            g.open_config_description(),
            crate::model::config_file_description(&dir),
            "nothing has touched the file"
        );

        // An outside write — Logseq, an editor, Syncthing delivering a peer's.
        std::fs::write(
            dir.join("logseq").join("config.edn"),
            "{:favorites [\"Alpha\" \"Beta\"]}\n",
        )
        .unwrap();
        assert_ne!(
            g.open_config_description(),
            crate::model::config_file_description(&dir),
            "the running graph is now serving stale configuration"
        );

        // Reopening is what the watcher does about it, and settles it.
        let reopened = crate::model::Graph::open(&dir);
        assert_eq!(
            reopened.open_config_description(),
            crate::model::config_file_description(&dir)
        );

        // A graph with no configuration file at all agrees with the absence,
        // rather than reporting a change on every single cycle.
        let bare = std::env::temp_dir().join(format!(
            "tine-config-witness-bare-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        let empty = crate::model::Graph::open(&bare);
        assert_eq!(empty.open_config_description(), None);
        assert_eq!(crate::model::config_file_description(&bare), None);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    /// The other half of the watcher's economy. A settings write leaves the
    /// running graph's parsed view stale (it always has), so the byte gate
    /// alone would read every star toggled in the sidebar as an outside change
    /// and reopen the whole graph — discarding every cache it has built.
    #[test]
    fn a_settings_write_tine_performed_itself_does_not_read_as_an_outside_change() {
        let dir = std::env::temp_dir().join(format!(
            "tine-config-selfwrite-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("logseq")).unwrap();
        std::fs::write(dir.join("logseq").join("config.edn"), "{}\n").unwrap();

        let g = crate::model::Graph::open(&dir);
        assert_eq!(g.recent_config_write(), None, "nothing published yet");

        g.set_favorites(&["Alpha".to_owned()]).unwrap();
        let disk = crate::model::config_file_description(&dir);

        assert_ne!(
            g.open_config_description(),
            disk,
            "the parsed view is stale after a write, as it has always been"
        );
        assert_eq!(
            g.recent_config_write(),
            disk,
            "but the bytes on disk are the ones this instance published"
        );

        // An outside edit after our own write is still an outside edit.
        std::fs::write(
            dir.join("logseq").join("config.edn"),
            "{:favorites [\"Alpha\" \"AddedInLogseq\"]}\n",
        )
        .unwrap();
        let disk = crate::model::config_file_description(&dir);
        assert_ne!(g.open_config_description(), disk);
        assert_ne!(g.recent_config_write(), disk);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_graph_s_own_config_edn_is_recognized_as_configuration() {
        use crate::model::is_config_file_path;
        let root = std::path::Path::new("/graph");
        assert!(is_config_file_path(
            root,
            std::path::Path::new("/graph/logseq/config.edn")
        ));
        // A case-folding filesystem may hand back either spelling; the open
        // path resolves it case-insensitively, so this must too.
        assert!(is_config_file_path(
            root,
            std::path::Path::new("/graph/Logseq/Config.edn")
        ));
        for other in [
            "/graph/config.edn",
            "/graph/logseq/custom.css",
            "/graph/logseq/pages-metadata.edn",
            "/graph/pages/logseq/config.edn",
            "/elsewhere/logseq/config.edn",
            "/graph",
        ] {
            assert!(
                !is_config_file_path(root, std::path::Path::new(other)),
                "{other} is not this graph's configuration"
            );
        }
    }

    #[test]
    fn favorites_page_key_round_trips_and_preserves_the_rest_of_the_file() {
        let dir = std::env::temp_dir().join(format!("tine-favpage-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("logseq")).unwrap();
        let original =
            "{;; a leading comment\n :favorites [\"Alpha\"]\n :journals-directory \"journals\"}\n";
        std::fs::write(dir.join("logseq").join("config.edn"), original).unwrap();

        let g = crate::model::Graph::open(&dir);
        assert_eq!(g.config.favorites_page, None);
        g.set_favorites_page("Favorites").unwrap();

        let written = std::fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert!(written.contains(";; a leading comment"), "{written}");
        assert!(written.contains(":favorites [\"Alpha\"]"), "{written}");
        assert!(
            written.contains(":journals-directory \"journals\""),
            "{written}"
        );
        assert_eq!(
            Config::parse(&written).favorites_page.as_deref(),
            Some("Favorites")
        );

        // Rewriting replaces the value in place rather than accumulating keys.
        let g = crate::model::Graph::open(&dir);
        g.set_favorites_page("My Favourites").unwrap();
        let rewritten = std::fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert_eq!(
            rewritten.matches(":tine/favorites-page").count(),
            1,
            "{rewritten}"
        );
        assert_eq!(
            Config::parse(&rewritten).favorites_page.as_deref(),
            Some("My Favourites")
        );
        assert!(rewritten.contains(":favorites [\"Alpha\"]"), "{rewritten}");

        // A quoted name containing a quote survives the round trip.
        let g = crate::model::Graph::open(&dir);
        g.set_favorites_page("od\"d").unwrap();
        let odd = std::fs::read_to_string(dir.join("logseq").join("config.edn")).unwrap();
        assert_eq!(Config::parse(&odd).favorites_page.as_deref(), Some("od\"d"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    use super::*;

    #[test]
    fn default_home_reads_only_the_page_inside_the_logseq_map() {
        assert_eq!(
            Config::parse(r#"{:default-home {:page "Directory" :sidebar ["Contents"]}}"#)
                .default_home
                .as_deref(),
            Some("Directory")
        );
        assert_eq!(
            Config::parse(r#"{:default-home "Wrong shape"}"#).default_home,
            None
        );
        assert_eq!(Config::parse("{}").default_home, None);
        assert_eq!(
            Config::parse(r#"{:nested {:default-home {:page "Not home"}}}"#).default_home,
            None
        );
        assert_eq!(
            Config::parse(r#"{:default-home {:sidebar {:page "Not home"} :page "Actual home"}}"#)
                .default_home
                .as_deref(),
            Some("Actual home")
        );
    }

    #[test]
    fn default_home_writer_preserves_siblings_clears_only_page_and_refuses_malformed_owner() {
        use crate::model::Graph;

        let dir = std::env::temp_dir().join(format!(
            "tine-default-home-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        let path = dir.join("logseq/config.edn");
        fs::write(
            &path,
            "{:default-home {:sidebar [\"Contents\"] :page \"Old\"}\n ;; keep me\n :start-of-week 2}\n",
        )
        .unwrap();

        Graph::open(&dir)
            .set_default_home_page(Some("New \"Home\""))
            .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains(":page \"New \\\"Home\\\"\""), "{written}");
        assert!(written.contains(":sidebar [\"Contents\"]"), "{written}");
        assert!(written.contains(";; keep me"), "{written}");
        assert!(written.contains(":start-of-week 2"), "{written}");
        assert_eq!(
            Graph::open(&dir).config.default_home.as_deref(),
            Some("New \"Home\"")
        );

        Graph::open(&dir).set_default_home_page(None).unwrap();
        let cleared = fs::read_to_string(&path).unwrap();
        assert!(!cleared.contains(":page \"New"), "{cleared}");
        assert!(cleared.contains(":sidebar [\"Contents\"]"), "{cleared}");
        assert_eq!(Graph::open(&dir).config.default_home, None);

        fs::write(&path, "{:start-of-week 2}\n").unwrap();
        Graph::open(&dir)
            .set_default_home_page(Some("Inserted"))
            .unwrap();
        let inserted = fs::read_to_string(&path).unwrap();
        assert!(
            inserted.contains(":default-home {:page \"Inserted\"}"),
            "{inserted}"
        );
        assert!(inserted.contains(":start-of-week 2"), "{inserted}");

        let malformed = "{:default-home \"do not replace\" :start-of-week 2}\n";
        fs::write(&path, malformed).unwrap();
        let error = Graph::open(&dir)
            .set_default_home_page(Some("Migration"))
            .expect_err("a non-map owner must be left untouched");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&path).unwrap(), malformed);

        fs::write(&path, "[:not-a-config-map]\n").unwrap();
        let before = fs::read(&path).unwrap();
        let error = Graph::open(&dir)
            .set_default_home_page(Some("Never replace the file"))
            .expect_err("a non-map config must be left untouched");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), before);

        let _ = fs::remove_dir_all(&dir);
    }

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
    fn parses_og_hidden_string_collection_and_ignores_wrong_value_shapes() {
        let cfg = Config::parse(
            r#"{:other {:hidden ["nested-map"]}
                 :predicate #(not= % \])
                 :hidden ["/archive/private"
                          #_ "discarded"
                          ["nested-vector"]
                          {:nested "map-value"}
                          42 nil :keyword
                          "scratch" ;; "commented"
                          "../assets/archived"]}"#,
        );
        assert_eq!(
            cfg.hidden,
            vec![
                "/archive/private".to_string(),
                "scratch".to_string(),
                "../assets/archived".to_string()
            ]
        );
        assert!(!cfg.hidden_parse_failed_closed);
        assert!(Config::parse(r#"{:hidden #{"/not-a-vector"}}"#)
            .hidden
            .is_empty());
        assert!(Config::parse(r#"{:hidden [42 :keyword]}"#)
            .hidden
            .is_empty());
    }

    #[test]
    fn hidden_reader_decodes_edn_strings_and_fails_closed_on_malformed_or_over_limit_values() {
        let decoded = Config::parse(r#"{:hidden ["archive\u002fprivate" "tab\tpath"]}"#);
        assert_eq!(
            decoded.hidden,
            vec!["archive/private".to_owned(), "tab\tpath".to_owned()]
        );
        assert!(!decoded.hidden_parse_failed_closed);

        for malformed in [
            r#"{:hidden ["private"]"#,
            r#"{:hidden ["private\q"]}"#,
            r#"{:hidden ["private" {:odd}]}"#,
        ] {
            let config = Config::parse(malformed);
            assert!(
                config.hidden_parse_failed_closed,
                "malformed hidden EDN must fail closed: {malformed}"
            );
        }

        let oversized = format!("{{:hidden [\"{}\"]}}", "x".repeat(MAX_HIDDEN_EDN_BYTES));
        assert!(Config::parse(&oversized).hidden_parse_failed_closed);

        let too_many = format!(
            "{{:hidden [{}]}}",
            std::iter::repeat_n("nil", MAX_HIDDEN_EDN_ENTRIES + 1)
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(Config::parse(&too_many).hidden_parse_failed_closed);
    }

    #[test]
    fn hidden_reader_counts_every_structured_form_kind() {
        for (source, expected_forms) in [
            ("nil", 1),
            ("[nil]", 2),
            ("(nil)", 2),
            ("#{nil}", 2),
            ("{:key nil}", 3),
            ("#tag nil", 2),
            ("'nil", 2),
            ("^:meta nil", 3),
            ("#_ nil kept", 3),
        ] {
            let mut reader = EdnReader::new(source);
            reader.skip_form().unwrap();
            reader.skip_interstitial_and_discards().unwrap();
            assert_eq!(reader.pos, source.len(), "{source}");
            assert_eq!(reader.depth, 0, "{source}");
            assert_eq!(reader.forms, expected_forms, "{source}");
        }
    }

    #[test]
    fn hidden_reader_accepts_exact_depth_and_form_limits_and_rejects_plus_one() {
        fn nested_hidden_collections(collections: usize) -> String {
            format!(
                "{{:hidden [{}nil{}]}}",
                "[".repeat(collections),
                "]".repeat(collections)
            )
        }
        let exact_depth = nested_hidden_collections(MAX_HIDDEN_EDN_DEPTH - 3);
        assert!(!Config::parse(&exact_depth).hidden_parse_failed_closed);
        let over_depth = nested_hidden_collections(MAX_HIDDEN_EDN_DEPTH - 2);
        assert!(Config::parse(&over_depth).hidden_parse_failed_closed);

        fn hidden_nested_list(entries: usize) -> String {
            format!(
                "{{:hidden [({})]}}",
                std::iter::repeat_n("x", entries)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
        let exact_forms = hidden_nested_list(MAX_HIDDEN_EDN_FORMS - 4);
        assert!(!Config::parse(&exact_forms).hidden_parse_failed_closed);
        let over_forms = hidden_nested_list(MAX_HIDDEN_EDN_FORMS - 3);
        assert!(Config::parse(&over_forms).hidden_parse_failed_closed);
    }

    #[test]
    fn hidden_reader_bounds_deep_discard_tag_and_mixed_collection_paths() {
        let discard_depth = MAX_HIDDEN_EDN_DEPTH / 2;
        let discarded = format!(
            "{{:hidden [{}{}\"kept\"]}}",
            "#_ ".repeat(discard_depth),
            "nil ".repeat(discard_depth)
        );
        let config = Config::parse(&discarded);
        assert_eq!(config.hidden, vec!["kept"]);
        assert!(!config.hidden_parse_failed_closed);

        let tagged = format!("{{:hidden [{}nil]}}", "#deep ".repeat(MAX_HIDDEN_EDN_DEPTH));
        assert!(Config::parse(&tagged).hidden_parse_failed_closed);

        let mixed = Config::parse(
            r#"{:hidden [[(#{:leaf})]
                         {:map ['quoted ^:meta #tag nil]}
                         #_ [#tag {:discarded (nil)}]
                         "visible"]}"#,
        );
        assert_eq!(mixed.hidden, vec!["visible"]);
        assert!(!mixed.hidden_parse_failed_closed);
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
    fn linked_references_collapsed_threshold_reads_the_og_key() {
        // GH #479. OG: `(>= total threshold)`, default 100 when the key is absent
        // or not an integer (`state.cljs` get-linked-references-collapsed-threshold
        // at `6e7afa8e`). Zero is a real setting — collapse always — not "unset".
        assert_eq!(
            Config::parse("{}").linked_references_collapsed_threshold,
            100
        );
        assert_eq!(
            Config::parse("{:ref/linked-references-collapsed-threshold 0}")
                .linked_references_collapsed_threshold,
            0
        );
        assert_eq!(
            Config::parse("{:ref/linked-references-collapsed-threshold 50}")
                .linked_references_collapsed_threshold,
            50
        );
        // A non-integer value keeps OG's default rather than collapsing everything.
        assert_eq!(
            Config::parse("{:ref/linked-references-collapsed-threshold \"50\"}")
                .linked_references_collapsed_threshold,
            100
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
