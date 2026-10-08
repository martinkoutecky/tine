//! `logseq/config.edn` readers and the shared selectors used by its setters.
//!
//! Config values are located by the root-map selector shared with graph-feature
//! setters. Only direct entries own settings, including in nested settings maps.
//! Surgical setters preserve unrelated bytes, comments and arbitrary forms;
//! readers decode string tokens through `edn::parse_strict`, the same decoder
//! used by sidecars. Missing or malformed individual values use defaults.

use std::collections::HashMap;

/// Effective graph configuration derived from `logseq/config.edn`. Missing or
/// unrecognized individual values use defaults.
#[deny(missing_docs)]
#[derive(Debug, Clone)]
pub struct Config {
    /// OG queries rendered below today's journal, in authored order.
    pub default_journal_queries: Vec<JournalQuery>,
    /// OG agenda opt-out; only literal true disables it.
    pub disable_scheduled_and_deadline_query: bool,
    /// OG agenda horizon (signed integers are accepted by OG). None when
    /// config.edn omits it: the device's Settings value (default 7) applies.
    pub scheduled_future_days: Option<i32>,
    /// Visible diagnostics for malformed journal configuration values.
    pub journal_config_diagnostics: Vec<String>,
    /// Configured journal directory, relative to the graph root.
    pub journals_dir: String,
    /// Configured ordinary-page directory, relative to the graph root.
    pub pages_dir: String,
    /// OG `:hidden` graph-relative literal path prefixes.
    pub hidden: Vec<String>,
    /// A malformed or over-limit `:hidden` vector (a torn or hand-broken
    /// `config.edn`, delivered by sync or an external editor) cannot safely
    /// read as "nothing hidden": that would admit text the owner excluded.
    /// Graph-text scope then hides all graph text (master
    /// `hidden_parse_failed_closed`; docs/storage-contract.md refusal table).
    pub hidden_parse_failed_closed: bool,
    /// Preferred task marker cycle.
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
    /// `:ref/linked-references-collapsed-threshold` — a page's Linked References
    /// section starts collapsed once the TOTAL backlink count reaches this
    /// (OG `(>= total threshold)`, `components/reference.cljs`). Absent or
    /// non-integer means OG's default 100; zero collapses always (GH #479).
    pub linked_references_collapsed_threshold: u32,
    /// `:property/separated-by-commas #{:a :b}` — keys added to the built-in
    /// `alias`/`aliases`/`tags` set. Tine already comma-splits EVERY key's plain
    /// value (Q21), so this only matters for a value that also contains refs: the
    /// value's comma-separated plain segments are kept as atoms next to its refs
    /// instead of being dropped. Keys are stored without the leading `:`; matched
    /// with `property_key_norm`. Read by `query::atom::ParseConfig` (queries and
    /// the property registry only).
    pub separated_by_commas: Vec<String>,
    /// `:ignored-page-references-keywords #{:a :b}` — keys whose values get no
    /// reference parsing: `[[x]]`/`#x` stay literal text. The value is still
    /// comma-split into plain atoms like every other key (Q21); only a
    /// double-quoted value stays one string. Wins over `separated_by_commas` for a
    /// key in both. Read by `query::atom::ParseConfig` (queries and the property
    /// registry only).
    pub ignored_page_references_keywords: Vec<String>,
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
    /// `:default-home {:page "Name"}` — the graph's home page (OG
    /// `state/get-default-home`). Only a string `:page` directly inside a
    /// top-level `:default-home` map counts; blank is `None`.
    pub default_home: Option<String>,
    /// `:favorites ["Page" …]` — favorited page names (on-disk, graph-portable).
    pub favorites: Vec<String>,
    /// `:mobile {:gestures/disabled-in-block-with-tags ["kanban"]}` — OG's
    /// opt-out of the block swipe gestures: a swipe that starts inside a block
    /// (or a descendant of a block) whose own refs contain any entry does
    /// nothing (`frontend.handler.block/target-disable-swipe?`). Only a vector
    /// of strings directly inside a top-level `:mobile` map counts.
    pub mobile_gestures_disabled_in_block_with_tags: Vec<String>,
    /// `:tine/favorites-page "Name"` — the page holding the Favorites arrangement
    /// (labels, nesting, order). Logseq ignores the key; `:favorites` stays the
    /// flat membership list Logseq reads.
    pub favorites_page: Option<String>,
    /// `:journal/file-name-format` — Logseq's journal filename format (cljs-time /
    /// Joda tokens). `None` uses `"yyyy_MM_dd"`. The store compiles a configured
    /// format and uses it to propose names for new journal files.
    pub journal_file_name_format: Option<String>,
    /// Journal TITLE format: nonempty `:journal/page-title-format`, then legacy
    /// `:date-formatter`. Missing, malformed or empty strings use the next key,
    /// then `"MMM do, yyyy"` for `None`; file naming is separate.
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
    /// OG `:feature/enable-search-remove-accents?`; default true.
    pub enable_search_remove_accents: bool,
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
    /// Stored in `logseq/config.edn` and scoped to this graph.
    // Graph config persists this instead of relying on WebKitGTK localStorage.
    pub guide_announced: bool,
}

/// A read-only configured query, sent through the ordinary macro renderer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JournalQuery {
    /// Plain text from a string or static hiccup title; never executable code.
    pub title: Option<String>,
    /// Macro body preserving the Logseq query and its inputs/options.
    pub body: String,
    /// Invalid entry diagnostic, isolated from adjacent queries.
    pub error: Option<String>,
}

/// OG's default when `:ref/linked-references-collapsed-threshold` is absent;
/// the one declaration the `Config` default and the graph-meta DTO share.
pub const DEFAULT_LINKED_REFERENCES_COLLAPSED_THRESHOLD: u32 = 100;

/// Preferred task marker cycle.
#[deny(missing_docs)]
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
#[deny(missing_docs)]
pub enum FileNameFormat {
    /// Percent-encoded namespace separators.
    Legacy,
    /// Triple-underscore namespace separators.
    TripleLowbar,
}

/// Effective logbook display and timestamp settings.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogbookSettings {
    /// Whether timestamps include seconds.
    pub with_second_support: bool,
    /// Whether logbooks appear in timestamped blocks.
    pub enabled_in_timestamped_blocks: bool,
    /// Whether logbooks appear in every block.
    pub enabled_in_all_blocks: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            default_journal_queries: Vec::new(),
            disable_scheduled_and_deadline_query: false,
            scheduled_future_days: None,
            journal_config_diagnostics: Vec::new(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            hidden: Vec::new(),
            hidden_parse_failed_closed: false,
            preferred_workflow: Workflow::Now,
            shortcuts: HashMap::new(),
            all_pages_public: false,
            start_of_week: 6, // Logseq's default (Sunday) — see field doc
            block_hidden_properties: Vec::new(),
            linked_references_collapsed_threshold: DEFAULT_LINKED_REFERENCES_COLLAPSED_THRESHOLD,
            separated_by_commas: Vec::new(),
            ignored_page_references_keywords: Vec::new(),
            property_pages_enabled: true,
            property_pages_excludelist: Vec::new(),
            default_journal_template: None,
            default_home: None,
            favorites: Vec::new(),
            favorites_page: None,
            mobile_gestures_disabled_in_block_with_tags: Vec::new(),
            journal_file_name_format: None,
            journal_page_title_format: None,
            preferred_format: crate::model::Format::Md,
            file_name_format: FileNameFormat::Legacy,
            macros: HashMap::new(),
            enable_timetracking: true,
            enable_search_remove_accents: true,
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
    /// Parse supported EDN keys independently, defaulting missing or malformed
    /// values. This does not report a validation error for a bad format string.
    pub fn parse(edn: &str) -> Config {
        // Locate only direct root entries, as the graph-feature setters do.
        // Arbitrary unrelated forms need not fit the small sidecar value model.
        let mut cfg = Config::default();
        parse_journal_settings(edn, &mut cfg);
        if let Some(v) = string_value(edn, ":journals-directory") {
            cfg.journals_dir = v;
        }
        if let Some(v) = string_value(edn, ":pages-directory") {
            cfg.pages_dir = v;
        }
        match parse_hidden_paths(edn) {
            Ok(hidden) => cfg.hidden = hidden,
            Err(()) => cfg.hidden_parse_failed_closed = true,
        }
        if let Some(v) = keyword_value(edn, ":preferred-workflow") {
            cfg.preferred_workflow = if v == "todo" {
                Workflow::Todo
            } else {
                Workflow::Now
            };
        }
        cfg.shortcuts = parse_shortcuts(edn);
        cfg.all_pages_public =
            bool_value(edn, ":publishing/all-pages-public?").unwrap_or(cfg.all_pages_public);
        if let Some(n) = int_value(edn, ":start-of-week") {
            if n <= 6 {
                cfg.start_of_week = n;
            }
        }
        cfg.block_hidden_properties = parse_keyword_set(edn, ":block-hidden-properties");
        if let Some(n) = int_value(edn, ":ref/linked-references-collapsed-threshold") {
            cfg.linked_references_collapsed_threshold = n;
        }
        cfg.separated_by_commas = parse_keyword_set(edn, ":property/separated-by-commas");
        cfg.ignored_page_references_keywords =
            parse_keyword_set(edn, ":ignored-page-references-keywords");
        cfg.property_pages_enabled =
            bool_value(edn, ":property-pages/enabled?").unwrap_or(cfg.property_pages_enabled);
        cfg.property_pages_excludelist = parse_keyword_set(edn, ":property-pages/excludelist");
        cfg.default_journal_template =
            nested_string(edn, ":default-templates", ":journals").filter(|s| !s.is_empty());
        cfg.default_home =
            nested_string(edn, ":default-home", ":page").filter(|s| !s.trim().is_empty());
        cfg.favorites = parse_string_vector(edn, ":favorites");
        cfg.mobile_gestures_disabled_in_block_with_tags =
            nested_string_vector(edn, ":mobile", ":gestures/disabled-in-block-with-tags");
        cfg.favorites_page =
            string_value(edn, ":tine/favorites-page").filter(|s| !s.trim().is_empty());
        cfg.journal_file_name_format =
            string_value(edn, ":journal/file-name-format").filter(|s| !s.is_empty());
        cfg.journal_page_title_format = string_value(edn, ":journal/page-title-format")
            .filter(|s| !s.is_empty())
            .or_else(|| string_value(edn, ":date-formatter").filter(|s| !s.is_empty()));
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
        cfg.enable_timetracking =
            bool_value(edn, ":feature/enable-timetracking?").unwrap_or(cfg.enable_timetracking);
        cfg.enable_search_remove_accents =
            read_keyword(edn, ":feature/enable-search-remove-accents?")
                .map(|at| {
                    let from = skip_blank(edn, at + ":feature/enable-search-remove-accents?".len());
                    !edn[from..].strip_prefix("false").is_some_and(|rest| {
                        rest.chars().next().is_none_or(|ch| {
                            ch.is_whitespace() || matches!(ch, ',' | '}' | ']' | ')' | ';' | '#')
                        })
                    })
                })
                .unwrap_or(cfg.enable_search_remove_accents);
        cfg.show_brackets = bool_value(edn, ":ui/show-brackets?").unwrap_or(cfg.show_brackets);
        cfg.doc_mode_enter_for_new_block =
            bool_value(edn, ":shortcut/doc-mode-enter-for-new-block?")
                .unwrap_or(cfg.doc_mode_enter_for_new_block);
        cfg.logical_outdenting =
            bool_value(edn, ":editor/logical-outdenting?").unwrap_or(cfg.logical_outdenting);
        let default = cfg.logbook;
        cfg.logbook = LogbookSettings {
            with_second_support: nested_bool(edn, ":logbook/settings", ":with-second-support?")
                .unwrap_or(default.with_second_support),
            enabled_in_timestamped_blocks: nested_bool(
                edn,
                ":logbook/settings",
                ":enabled-in-timestamped-blocks",
            )
            .unwrap_or(default.enabled_in_timestamped_blocks),
            enabled_in_all_blocks: nested_bool(edn, ":logbook/settings", ":enabled-in-all-blocks")
                .unwrap_or(default.enabled_in_all_blocks),
        };
        cfg.guide_announced =
            bool_value(edn, ":tine/guide-announced?").unwrap_or(cfg.guide_announced);
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
// Shared scanner family — byte-offset, string/comment/escape-aware. Used by
// BOTH the readers above and the writers above. `;` comment handling lives in
// `find_keyword_at_map_level` (so there's no separate comment-strip pass).
// ---------------------------------------------------------------------------

/// Index just past the closing quote of an EDN string opening at byte `open` (a
/// `"`), skipping `\"` / `\\`. Returns end-of-string if unterminated. (`"` is
/// ASCII → the returned index is a char boundary.)
pub fn edn_str_end(s: &str, open: usize) -> usize {
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
pub fn match_close_brace(s: &str, open: usize) -> usize {
    match_close(s, open, b'{', b'}')
}

/// Matching close `]` for the vector whose `[` is at byte `open`, EDN-aware.
pub fn match_close_bracket(s: &str, open: usize) -> usize {
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

/// Byte index of `key` as a KEY of the map whose body is `s` (the text between
/// its braces): a direct entry, never inside a nested map/vector/list/set,
/// string or `;` comment, and never a VALUE that happens to spell the same
/// keyword (`{:backup :favorites :private "keep"}` has no `:favorites` key;
/// C5 L01-S1 spliced over it and the entry after it). The body is walked form
/// by form, so keys sit at even form positions. A `#_` discard consumes the
/// next form without counting it; a `#tag` prefix belongs to the form after it.
/// Linear (every step advances), so arbitrary content cannot hang or mislead it.
pub fn find_keyword_at_map_level(s: &str, key: &str) -> Option<usize> {
    let b = s.as_bytes();
    let (mut at, mut forms) = (skip_blank(s, 0), 0usize);
    let mut discard = false;
    let mut tagged = false;
    while at < b.len() {
        if b[at] == b'#' && b.get(at + 1) == Some(&b'_') {
            discard = true;
            at = skip_blank(s, at + 2);
            continue;
        }
        let end = form_end(s, at);
        if b[at] == b'#' && !matches!(b.get(at + 1), Some(b'{' | b'#')) {
            tagged = true; // `#inst "…"`: the tag and its value are one form
        } else {
            if !discard && !tagged && forms % 2 == 0 && &s[at..end] == key {
                return Some(at);
            }
            if !discard {
                forms += 1;
            }
            discard = false;
            tagged = false;
        }
        at = skip_blank(s, end);
    }
    None
}

/// End of the one EDN form starting at `at` (non-blank): a string, a balanced
/// collection (`{}`/`[]`/`()`/`#{}`), or a token up to the next delimiter. A
/// stray closer is a one-byte form so the walk always advances.
fn form_end(s: &str, at: usize) -> usize {
    let b = s.as_bytes();
    let set = b[at] == b'#' && b.get(at + 1) == Some(&b'{');
    let open = if set { at + 1 } else { at };
    match b[open] {
        b'"' => edn_str_end(s, open),
        b'{' => (match_close_brace(s, open) + 1).min(s.len()),
        b'[' => (match_close_bracket(s, open) + 1).min(s.len()),
        b'(' => (match_close(s, open, b'(', b')') + 1).min(s.len()),
        b'}' | b']' | b')' => open + 1,
        _ => {
            // `\(` and `\é`: the char after a backslash belongs to the literal.
            let mut end = open + usize::from(b[open] == b'\\');
            end += s[end..].chars().next().map_or(0, char::len_utf8);
            // Compact EDN: a `#{` set may follow a token with no blank between.
            while end < b.len()
                && !crate::edn::is_delim(b[end])
                && !(b[end] == b'#' && b.get(end + 1) == Some(&b'{'))
            {
                end += 1;
            }
            end
        }
    }
}

/// Span `[start, end)` of the value token following byte `from` (skipping leading
/// whitespace/commas) within `..close`, plus whether it is an EDN string. None if
/// there is no value before `close`. A string's end is escape-aware; a non-string
/// token ends at the next whitespace/comma/brace/quote.
pub fn next_value_span(s: &str, from: usize, close: usize) -> Option<(usize, usize, bool)> {
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
// Readers — root ownership is shared with `find_top_level_keyword`; complete
// values before a torn suffix are still readable. Strings use the EDN decoder;
// the shared scanners.
// ---------------------------------------------------------------------------

/// First non-blank byte at/after `from`, skipping whitespace, commas, and `;`
/// comments (a comment can sit between a key and its value).
pub fn skip_blank(s: &str, from: usize) -> usize {
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

/// Decode one complete string token with the shared EDN value parser.
fn read_string_at(s: &str, open: usize) -> Option<String> {
    let end = edn_str_end(s, open);
    match crate::edn::parse_strict(s.get(open..end)?)? {
        crate::edn::Edn::Str(value) => Some(value),
        _ => None,
    }
}

/// Read a direct root setting with the shared selector and EDN decoder.
fn journal_setting_value(edn: &str, key: &str) -> Option<Option<crate::edn::Edn>> {
    let start = read_keyword(edn, key)?;
    let from = skip_blank(edn, start + key.len());
    Some(if from < edn.len() {
        crate::edn::parse_strict(&edn[from..form_end(edn, from)])
    } else {
        None
    })
}

// OG query-title accepts vectors as hiccup. Extract only static text children;
// attributes, symbols, lists and tags can never become markup or graph code.
fn journal_title(value: &crate::edn::Edn) -> Option<String> {
    use crate::edn::Edn;
    match value {
        Edn::Str(s) => Some(s.clone()),
        Edn::Int(n) => Some(n.to_string()),
        Edn::Float(n) => Some(n.to_string()),
        Edn::Vec(children) if matches!(children.first(), Some(Edn::Keyword(_) | Edn::Str(_))) => {
            Some(children.iter().skip(1).filter_map(journal_title).collect())
        }
        _ => None,
    }
}

fn parse_journal_settings(edn: &str, cfg: &mut Config) {
    use crate::edn::{self, Edn};
    let horizon = ":scheduled/future-days";
    if let Some(value) = journal_setting_value(edn, horizon) {
        match value {
            Some(Edn::Int(n)) if i32::try_from(n).is_ok() => {
                cfg.scheduled_future_days = Some(n as i32)
            }
            _ => cfg.journal_config_diagnostics.push(format!(
                "{horizon} must be an integer; using the Settings value."
            )),
        }
    }
    let disable = ":feature/disable-scheduled-and-deadline-query?";
    if let Some(value) = journal_setting_value(edn, disable) {
        match value {
            Some(Edn::Bool(b)) => cfg.disable_scheduled_and_deadline_query = b,
            _ => cfg
                .journal_config_diagnostics
                .push(format!("{disable} must be true or false; using false.")),
        }
    }
    let Some(value) = journal_setting_value(edn, ":default-queries") else {
        return;
    };
    let Some(Edn::Map(_)) = value.as_ref() else {
        cfg.journal_config_diagnostics
            .push(":default-queries must be a map.".into());
        return;
    };
    let Some(journals) = value.as_ref().and_then(|v| v.get("journals")) else {
        return;
    };
    let Edn::Vec(queries) = journals else {
        cfg.journal_config_diagnostics
            .push(":default-queries :journals must be a vector.".into());
        return;
    };
    for (index, entry) in queries.iter().enumerate() {
        let title = entry.get("title").and_then(journal_title);
        let mut query = JournalQuery {
            title,
            body: String::new(),
            error: None,
        };
        match (entry, entry.get("query")) {
            (Edn::Map(pairs), Some(form @ (Edn::List(_) | Edn::Str(_) | Edn::Vec(_)))) => {
                // Hiccup has already been reduced to text; leave executable
                // options to the existing query parser's refusal boundary.
                let options: Vec<_> = pairs.iter().filter(|(k, _)| !matches!(k, Edn::Keyword(s) if s == "title" || s == "query")).cloned().collect();
                query.body = if matches!(form, Edn::Vec(_)) {
                    let mut advanced = options;
                    advanced.insert(0, (Edn::Keyword("query".into()), form.clone()));
                    format!("query {}", edn::to_string(&Edn::Map(advanced)))
                } else {
                    let source = match form { Edn::Str(s) => s.clone(), _ => edn::to_string(form) };
                    format!("query {source} {}", edn::to_string(&Edn::Map(options)))
                };
                if entry.get("title").is_some_and(|v| !matches!(v, Edn::Nil) && journal_title(v).is_none()) {
                    cfg.journal_config_diagnostics.push(format!(":default-queries :journals entry {} has no plain-text title.", index + 1));
                }
            }
            _ => query.error = Some(format!("Invalid default journal query {}: expected a map with :query (a string, list or vector).", index + 1)),
        }
        cfg.default_journal_queries.push(query);
    }
}

/// String value following `key`, e.g. `:journals-directory "journals"`.
fn string_value(edn: &str, key: &str) -> Option<String> {
    let start = read_keyword(edn, key)?;
    let from = skip_blank(edn, start + key.len());
    (edn.as_bytes().get(from) == Some(&b'"'))
        .then(|| read_string_at(edn, from))
        .flatten()
}

/// Keyword value (`:foo` → `foo`) following `key`.
fn keyword_value(edn: &str, key: &str) -> Option<String> {
    let start = read_keyword(edn, key)?;
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
    let start = read_keyword(edn, key)?;
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
    let start = read_keyword(edn, key)?;
    let from = skip_blank(edn, start + key.len());
    let digits: String = edn[from..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Read `:hidden` as a bounded vector of strings. An invalid value hides all
/// graph text rather than silently turning an intended exclusion into none.
fn parse_hidden_paths(edn: &str) -> Result<Vec<String>, ()> {
    // A torn root must still expose an authored :hidden vector to its bounded
    // validator; a missing closing brace must not turn exclusions into none.
    let Some(start) = read_keyword(edn, ":hidden") else {
        return Ok(Vec::new());
    };
    let from = skip_blank(edn, start + ":hidden".len());
    // OG treats a non-vector value as no configured hidden paths.
    if edn.as_bytes().get(from) != Some(&b'[') {
        return Ok(Vec::new());
    }
    let mut scan_end = from.saturating_add(64 * 1024).min(edn.len());
    while !edn.is_char_boundary(scan_end) {
        scan_end -= 1;
    }
    let close_relative = match_close_bracket(&edn[from..scan_end], 0);
    if close_relative == scan_end - from {
        return Err(());
    }
    let close = from + close_relative;
    let bytes = edn.as_bytes();
    let mut paths = Vec::new();
    let mut entries = 0usize;
    let mut i = from + 1;
    while i < close {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' | b',' => i += 1,
            b';' => {
                while i < close && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                let end = edn_str_end(&edn[..close + 1], i);
                entries += 1;
                if end > close || bytes.get(end - 1) != Some(&b'"') || entries > 256 {
                    return Err(());
                }
                paths.push(read_string_at(edn, i).ok_or(())?);
                i = end;
            }
            b'#' if bytes.get(i + 1) == Some(&b'_') => {
                i = skip_hidden_form(edn, skip_blank(edn, i + 2), close, 0)?;
            }
            _ => {
                entries += 1;
                if entries > 256 {
                    return Err(());
                }
                i = skip_hidden_form(edn, i, close, 0)?;
            }
        }
    }
    Ok(paths)
}

fn skip_hidden_form(edn: &str, start: usize, close: usize, depth: usize) -> Result<usize, ()> {
    if depth >= 32 || start >= close {
        return Err(());
    }
    let bytes = edn.as_bytes();
    let (open, left, right) = match bytes[start] {
        b'[' => (start, b'[', b']'),
        b'{' => (start, b'{', b'}'),
        b'(' => (start, b'(', b')'),
        b'#' if bytes.get(start + 1) == Some(&b'{') => (start + 1, b'{', b'}'),
        b'#' if bytes.get(start + 1) == Some(&b'_') => {
            return skip_hidden_form(edn, skip_blank(edn, start + 2), close, depth + 1);
        }
        b'"' => {
            let end = edn_str_end(&edn[..close + 1], start);
            return (end <= close && bytes.get(end - 1) == Some(&b'"'))
                .then_some(end)
                .ok_or(());
        }
        _ => {
            let mut end = start;
            while end < close
                && !matches!(
                    bytes[end],
                    b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}' | b')' | b';'
                )
            {
                end += 1;
            }
            return (end > start).then_some(end).ok_or(());
        }
    };
    let end = match_close(&edn[..close + 1], open, left, right);
    (end < close).then_some(end + 1).ok_or(())
}

/// Quoted strings in the vector following `key` (`:favorites ["a" "b"]`),
/// string-aware so a value containing `]` doesn't end the vector early.
fn parse_string_vector(edn: &str, key: &str) -> Vec<String> {
    let Some(start) = read_keyword(edn, key) else {
        return Vec::new();
    };
    string_vector_at(edn, skip_blank(edn, start + key.len()))
}

/// The quoted strings of the `[...]` vector opening at byte `from`; empty when
/// there is no `[` there.
fn string_vector_at(edn: &str, from: usize) -> Vec<String> {
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
                if let Some(value) = read_string_at(edn, i) {
                    out.push(value);
                }
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

/// A vector of strings for `inner` directly inside the map following `outer`
/// (`:mobile {:gestures/disabled-in-block-with-tags ["kanban"]}`). Nested
/// extension maps cannot shadow either key.
fn nested_string_vector(edn: &str, outer: &str, inner: &str) -> Vec<String> {
    let Some(key) = read_keyword(edn, outer) else {
        return Vec::new();
    };
    let Some((open, close)) = balanced_map_at(edn, skip_blank(edn, key + outer.len())) else {
        return Vec::new();
    };
    let Some(irel) = find_keyword_at_map_level(&edn[open + 1..close], inner) else {
        return Vec::new();
    };
    string_vector_at(edn, skip_blank(edn, open + 1 + irel + inner.len()))
}

/// A direct string entry in a direct root settings map. Nested extension
/// maps cannot shadow either the outer setting or its inner entry.
fn nested_string(edn: &str, outer: &str, inner: &str) -> Option<String> {
    let key = read_keyword(edn, outer)?;
    let (open, close) = balanced_map_at(edn, skip_blank(edn, key + outer.len()))?;
    let irel = find_keyword_at_map_level(&edn[open + 1..close], inner)?;
    let vfrom = skip_blank(edn, open + 1 + irel + inner.len());
    (edn.as_bytes().get(vfrom) == Some(&b'"'))
        .then(|| read_string_at(edn, vfrom))
        .flatten()
}

/// `(open, close)` of the balanced `{…}` map opening at byte `open`; `None`
/// when there is no `{` there or it never closes.
pub fn balanced_map_at(s: &str, open: usize) -> Option<(usize, usize)> {
    if s.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let close = match_close_brace(s, open);
    (close < s.len()).then_some((open, close))
}

/// `(open, close)` of the root map: the first form after blanks and `;`
/// comments, when it is a balanced `{…}`.
pub fn root_map_bounds(s: &str) -> Option<(usize, usize)> {
    balanced_map_at(s, skip_blank(s, 0))
}

/// Byte index of `key` among the DIRECT entries of the root map, never inside
/// a nested map, vector, comment or string. Every top-level config setter
/// locates its key here: the depth-blind keyword search returned a nested
/// shadow first and the setter spliced over it (master DUP-3, 4ae2f6f4f).
/// `None` when the key is absent or there is no balanced root map.
pub fn find_top_level_keyword(s: &str, key: &str) -> Option<usize> {
    root_keyword(s, key, true)
}

// Readers keep complete values before a torn later form. Writers additionally
// require a balanced root before applying any edit; both share root ownership.
fn read_keyword(s: &str, key: &str) -> Option<usize> {
    root_keyword(s, key, false)
}

fn root_keyword(s: &str, key: &str, require_balanced: bool) -> Option<usize> {
    let open = skip_blank(s, 0);
    if s.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let close = match_close_brace(s, open);
    if require_balanced && close == s.len() {
        return None;
    }
    find_keyword_at_map_level(&s[open + 1..close], key).map(|at| open + 1 + at)
}

/// Boolean value for `inner` inside the map following `outer`, e.g.
/// `:logbook/settings {:with-second-support? false}`.
fn nested_bool(edn: &str, outer: &str, inner: &str) -> Option<bool> {
    let start = read_keyword(edn, outer)?;
    let from = skip_blank(edn, start + outer.len());
    if edn.as_bytes().get(from) != Some(&b'{') {
        return None;
    }
    let close = match_close_brace(edn, from);
    let irel = find_keyword_at_map_level(&edn[from + 1..close], inner)?;
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
    let Some(start) = read_keyword(edn, key) else {
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
    let Some(start) = read_keyword(edn, ":shortcuts") else {
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
                if let Some(value) = read_string_at(edn, vfrom) {
                    map.insert(key, value);
                }
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
                        if let Some(value) = read_string_at(edn, k) {
                            map.insert(key.clone(), value);
                        }
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
    let Some(start) = read_keyword(edn, ":macros") else {
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
        let Some(key) = read_string_at(edn, i) else {
            break;
        };
        i = edn_str_end(edn, i);
        let vfrom = skip_blank(edn, i);
        if vfrom >= close || b[vfrom] != b'"' {
            break; // value must be a string
        }
        let Some(val) = read_string_at(edn, vfrom) else {
            break;
        };
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
    fn journal_query_settings_preserve_logseq_forms_and_defaults() {
        let cfg = Config::parse(
            r#"{:scheduled/future-days -2
          :feature/disable-scheduled-and-deadline-query? true
          :default-queries {:journals [{:title [:b "Tasks " [:span "today"]]
            :query (task TODO) :collapsed? true}
            {:title "Advanced" :query [:find ?b :in $ ?day :where [?b :block/scheduled ?day]] :inputs [:today]}]}}"#,
        );
        assert_eq!(cfg.scheduled_future_days, Some(-2));
        assert!(cfg.disable_scheduled_and_deadline_query);
        assert_eq!(cfg.default_journal_queries.len(), 2);
        assert_eq!(
            cfg.default_journal_queries[0].title.as_deref(),
            Some("Tasks today")
        );
        assert!(cfg.default_journal_queries[0].body.contains("(task TODO)"));
        assert!(cfg.default_journal_queries[1]
            .body
            .contains(":inputs [:today]"));
        assert_eq!(Config::default().scheduled_future_days, None);
        let title = Config::parse(
            r#"{:default-queries {:journals [{:title ["span" {:onclick "ignored"} "Count " 2 (graph-code)] :query (task TODO)}]}}"#,
        );
        assert_eq!(
            title.default_journal_queries[0].title.as_deref(),
            Some("Count 2")
        );
    }

    #[test]
    fn journal_query_bad_entries_are_diagnosed_without_losing_neighbors() {
        let cfg = Config::parse(
            r#"{:scheduled/future-days 2.5
          :feature/disable-scheduled-and-deadline-query? "yes"
          :default-queries {:journals [{:query (task TODO)} 42 {:query (task DOING)}]}}"#,
        );
        assert_eq!(cfg.scheduled_future_days, None);
        assert!(!cfg.disable_scheduled_and_deadline_query);
        assert_eq!(cfg.default_journal_queries.len(), 3);
        assert!(cfg.default_journal_queries[1].error.is_some());
        assert!(!cfg.journal_config_diagnostics.is_empty());
        let nested = Config::parse(
            r#"{:other {:scheduled/future-days 99 :default-queries {:journals []}}}"#,
        );
        assert_eq!(nested.scheduled_future_days, None);
    }

    #[test]
    fn configured_queries_use_the_shared_macro_parser_and_metadata() {
        use crate::query::{parse_query_input, QueryInput};
        let cfg = Config::parse(
            r#"{:default-queries {:journals [
            {:query (and (task TODO DOING) (priority A)) :collapsed? true}
            {:query [:find (pull ?b [*]) :where [?b :block/marker "TODO"]] :inputs []}
            {:query [:find ?b :where (graph-code ?b)]}]}}"#,
        );
        let today = crate::date::JournalDate {
            year: 2026,
            month: 10,
            day: 8,
        };
        let registry =
            crate::query::registry::Registry::empty(&crate::query::atom::ParseConfig::default());
        for (i, configured) in cfg.default_journal_queries.iter().enumerate() {
            let argument = configured.body.strip_prefix("query ").unwrap();
            let (query, _) = parse_query_input(argument, QueryInput::MacroQuery, today, &registry);
            if i == 0 {
                assert!(!query.is_invalid(), "{:?}", query.diagnostics);
            }
            if i == 1 {
                assert!(matches!(
                    query.source,
                    crate::query::ir::Source::Advanced { .. }
                ));
                let resolved = crate::query::resolve_for_execution(
                    &query,
                    &crate::query::ir::ExecutionContext::default(),
                    today,
                );
                assert!(
                    !resolved.query().is_invalid(),
                    "{:?}",
                    resolved.query().diagnostics
                );
            }
            if i == 2 {
                let resolved = crate::query::resolve_for_execution(
                    &query,
                    &crate::query::ir::ExecutionContext::default(),
                    today,
                );
                assert!(resolved.query().is_invalid());
            }
        }
        let meta = crate::model::GraphMeta::from_config(
            String::new(),
            &cfg,
            &crate::date::JournalFormat::new(None, None),
        );
        assert_eq!(meta.default_journal_queries.len(), 3);
        assert_eq!(meta.scheduled_future_days, None);
    }

    #[test]
    fn accent_removal_defaults_on_and_only_explicit_false_disables_it() {
        assert!(Config::parse("{}").enable_search_remove_accents);
        assert!(
            Config::parse("{:feature/enable-search-remove-accents? true}")
                .enable_search_remove_accents
        );
        assert!(
            !Config::parse("{:feature/enable-search-remove-accents? false}")
                .enable_search_remove_accents
        );
        assert!(
            Config::parse("{:feature/enable-search-remove-accents? :false}")
                .enable_search_remove_accents
        );
        assert!(
            Config::parse("{:feature/enable-search-remove-accents? falsehood}")
                .enable_search_remove_accents
        );
        assert!(
            Config::parse("{:feature/enable-search-remove-accents? \"false\"}")
                .enable_search_remove_accents
        );
        assert!(
            !Config::parse("{:feature/enable-search-remove-accents? ; comment\nfalse}")
                .enable_search_remove_accents
        );
        assert!(
            !Config::parse("{:feature/enable-search-remove-accents? false; comment\n}")
                .enable_search_remove_accents
        );
    }

    #[test]
    fn hidden_vector_is_decoded_and_bad_value_fails_closed() {
        let cfg = Config::parse(
            r#"{:hidden ["archive\u002fprivate" ; ignored
                                  42 #_"discard" "pages/Secret"]}"#,
        );
        assert_eq!(cfg.hidden, ["archive/private", "pages/Secret"]);
        assert!(!cfg.hidden_parse_failed_closed);
        for inert in [r#"{:hidden #{"archive"}}"#, r#"{:hidden [42]}"#, "{}"] {
            let cfg = Config::parse(inert);
            assert!(cfg.hidden.is_empty(), "{inert}");
            assert!(!cfg.hidden_parse_failed_closed, "{inert}");
        }
        let oversized = format!("{{:hidden [\"{}\"]}}", "x".repeat(64 * 1024));
        let too_many = format!("{{:hidden [{}]}}", "nil ".repeat(257));
        for bad in [
            "{:hidden [\"unfinished\"",
            r#"{:hidden ["bad\q"]}"#,
            &oversized,
            &too_many,
        ] {
            let cfg = Config::parse(bad);
            assert!(cfg.hidden.is_empty(), "{bad}");
            assert!(cfg.hidden_parse_failed_closed, "master fails closed: {bad}");
        }
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
    fn default_home_reads_only_the_page_inside_the_top_level_map() {
        let home = |edn: &str| Config::parse(edn).default_home;
        assert_eq!(
            home(r#"{:default-home {:page "Directory" :sidebar ["Contents"]}}"#).as_deref(),
            Some("Directory")
        );
        assert_eq!(home(r#"{:default-home "Wrong shape"}"#), None);
        assert_eq!(home("{}"), None);
        assert_eq!(home(r#"{:default-home {:page "  "}}"#), None);
        assert_eq!(
            home(r#"{:nested {:default-home {:page "Not home"}}}"#),
            None
        );
        assert_eq!(
            home(r#"{:default-home {:sidebar {:page "Not home"} :page "Actual home"}}"#).as_deref(),
            Some("Actual home")
        );
        assert_eq!(
            home("{;; :default-home {:page \"Commented\"}\n :default-home {:page \"Live\"}}")
                .as_deref(),
            Some("Live")
        );
        assert_eq!(home(r#"{:default-home-x {:page "Prefix"}}"#), None);
        assert_eq!(
            home("; graph settings\n  ;; more\n{:default-home {:page \"Directory\"}}").as_deref(),
            Some("Directory"),
            "leading EDN comments before the root map"
        );
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
    fn query_property_value_keys_are_read() {
        let cfg = Config::parse(
            "{:property/separated-by-commas #{:authors :Genre}\n :ignored-page-references-keywords #{:note}}",
        );
        assert_eq!(
            cfg.separated_by_commas,
            vec!["authors".to_string(), "Genre".to_string()]
        );
        assert_eq!(
            cfg.ignored_page_references_keywords,
            vec!["note".to_string()]
        );
        let absent = Config::parse("{}");
        assert!(absent.separated_by_commas.is_empty());
        assert!(absent.ignored_page_references_keywords.is_empty());
    }

    #[test]
    fn linked_references_collapsed_threshold_reads_the_og_key() {
        // GH #479. OG: `(>= total threshold)`, default 100 when the key is absent
        // or not an integer. Zero is a real setting — collapse always.
        assert_eq!(
            Config::parse("{}").linked_references_collapsed_threshold,
            100
        );
        // The graph-meta DTO's serde default and `Config::default` share one constant.
        assert_eq!(DEFAULT_LINKED_REFERENCES_COLLAPSED_THRESHOLD, 100);
        assert_eq!(
            Config::default().linked_references_collapsed_threshold,
            DEFAULT_LINKED_REFERENCES_COLLAPSED_THRESHOLD
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

#[cfg(test)]
mod key_position_tests {
    use super::*;

    fn at(body: &str, key: &str) -> Option<usize> {
        find_keyword_at_map_level(body, key)
    }

    /// C5 L01-S1: a keyword VALUE spelling the key is not the key, in every
    /// shape a value (and so a key) can take.
    #[test]
    fn a_key_is_found_only_at_a_key_position() {
        assert_eq!(at(":a :k 1", ":k"), None, "value of :a");
        assert_eq!(at(":a :k :k 2", ":k"), Some(6), "key after the value");
        assert_eq!(at(":a [1 2] :k 1", ":k"), Some(9));
        assert_eq!(at(":a {:x :k} :b #{:k} :c (:k) :d \"k\"", ":k"), None);
        assert_eq!(
            at(":a #inst \"2020\" :k 1", ":k"),
            Some(16),
            "a tag and its value are one form"
        );
        assert_eq!(
            at(":a 1 #_ :x :k 1", ":k"),
            Some(11),
            "a discarded form is not an entry"
        );
        assert_eq!(
            at(":a #_ :k :k 1", ":k"),
            None,
            "the discarded :k is not the value"
        );
        assert_eq!(at("\"a\" :k :k 1", ":k"), Some(7), "string key");
        assert_eq!(
            at(":k", ":k"),
            Some(0),
            "a bare key without a value is still the key"
        );
        assert_eq!(at(":kk 1 :k 2", ":k"), Some(6), "token boundary");
        assert_eq!(at(":a 1 ; :k 2\n :b 3", ":k"), None, "comment");
    }

    /// The readers answer through the same selector, so a keyword value never
    /// supplies a setting.
    #[test]
    fn readers_ignore_a_keyword_value_that_spells_a_setting() {
        let edn = "{:preferred-workflow :now :alias :preferred-format :preferred-format :org \
                   :favorites-backup :favorites :favorites [\"Real\"]}";
        let config = Config::parse(edn);
        assert_eq!(config.favorites, ["Real"]);
        assert_eq!(config.preferred_format, crate::model::Format::Org);
    }
}

#[cfg(test)]
mod defaults_tests {
    use super::*;

    /// C5 L01-B3 (Rust half; I-12): a setting's default is declared once, in
    /// `Config::default()`/`LogbookSettings::default()`. `Config::parse` falls
    /// back to the field it started from, never to a second literal that could
    /// drift from it (the browser's mirror of these defaults reads them from
    /// the graph metadata the backend serves).
    #[test]
    fn parse_never_restates_a_default_literal() {
        let source = include_str!("config.rs");
        let parse = source
            .split("    pub fn parse(edn: &str) -> Config {")
            .nth(1)
            .and_then(|rest| rest.split("\n        cfg\n    }\n").next())
            .expect("Config::parse body");
        for literal in [".unwrap_or(true)", ".unwrap_or(false)"] {
            assert!(
                !parse.contains(literal),
                "I-12: Config::parse restates a default {literal}; fall back to the field set by Config::default() (exemplar: show_brackets)"
            );
        }
    }

    #[test]
    fn an_empty_or_malformed_config_serves_the_declared_defaults() {
        for edn in [
            "",
            "{}",
            "{:ui/show-brackets? maybe :logbook/settings {:with-second-support? 3}}",
        ] {
            let cfg = Config::parse(edn);
            let default = Config::default();
            assert_eq!(cfg.show_brackets, default.show_brackets, "{edn}");
            assert_eq!(
                cfg.enable_timetracking, default.enable_timetracking,
                "{edn}"
            );
            assert_eq!(cfg.logbook, default.logbook, "{edn}");
            assert_eq!(cfg.start_of_week, default.start_of_week, "{edn}");
        }
    }
}

#[cfg(test)]
mod non_ascii_scan_tests {
    use super::*;

    /// I-22: every scanner indexes bytes, and a non-ASCII character anywhere in
    /// config.edn (outside strings and comments too) must never panic a reader.
    #[test]
    fn non_ascii_anywhere_never_panics_the_scanners() {
        let base = r#"{:journals-directory "j" :pages-directory "p" :hidden ["a"]
 :preferred-workflow :todo :shortcuts {:a "b" :c false} :start-of-week 2
 :block-hidden-properties #{:x} :default-templates {:journals "T"}
 :default-home {:page "H"} :favorites ["F"] :logbook/settings {:with-second-support? false}
 :macros {"m" "v"} :ui/show-brackets? false :file/name-format :triple-lowbar}"#;
        let keys = [
            ":journals-directory",
            ":hidden",
            ":favorites",
            ":ui/show-brackets?",
        ];
        for insert in ["é", "日本", "🙂"] {
            for at in (0..=base.len()).filter(|at| base.is_char_boundary(*at)) {
                let edn = format!("{}{insert}{}", &base[..at], &base[at..]);
                let _ = Config::parse(&edn);
                for key in keys {
                    let _ = find_keyword_at_map_level(&edn, key);
                }
                for from in (0..edn.len()).filter(|from| edn.is_char_boundary(*from)) {
                    let _ = next_value_span(&edn, from, edn.len());
                    let _ = skip_blank(&edn, from);
                }
            }
        }
    }
}

#[cfg(test)]
mod mobile_gestures_tests {
    use super::*;

    fn tags(edn: &str) -> Vec<String> {
        Config::parse(edn).mobile_gestures_disabled_in_block_with_tags
    }

    #[test]
    fn default_is_empty() {
        assert!(tags("{}").is_empty());
        assert!(tags("{:mobile {}}").is_empty());
    }

    #[test]
    fn reads_the_og_key() {
        assert_eq!(
            tags(r#"{:mobile {:gestures/disabled-in-block-with-tags ["kanban" "board"]}}"#),
            ["kanban", "board"]
        );
    }

    #[test]
    fn a_commented_entry_is_not_a_value() {
        let edn =
            "{:mobile {:gestures/disabled-in-block-with-tags [\n \"kanban\"\n ;; \"example\"\n]}}";
        assert_eq!(tags(edn), ["kanban"]);
    }

    #[test]
    fn a_nested_map_cannot_shadow_the_key() {
        // The key inside another setting's map is not the `:mobile` setting.
        assert!(
            tags(r#"{:other {:mobile {:gestures/disabled-in-block-with-tags ["x"]}}}"#).is_empty()
        );
        // Nor does a sibling key inside `:mobile` bleed through.
        assert!(
            tags(r#"{:mobile {:other {:gestures/disabled-in-block-with-tags ["x"]}}}"#).is_empty()
        );
    }

    #[test]
    fn a_wrong_shape_yields_nothing_and_does_not_panic() {
        assert!(tags(r#"{:mobile {:gestures/disabled-in-block-with-tags "kanban"}}"#).is_empty());
        assert!(tags(r#"{:mobile "kanban"}"#).is_empty());
        assert!(tags(r#"{:mobile {:gestures/disabled-in-block-with-tags"#).is_empty());
    }
}
