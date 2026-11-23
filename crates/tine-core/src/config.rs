//! `logseq/config.edn` readers and the shared selectors used by its setters.
//!
//! Direct settings consume effective values from the shared EDN reader.
//! Surgical setters locate authored spans to preserve unrelated bytes.

use crate::edn::{self, strings_in_vector, ConfigEntry, Edn, Form, Kind, UnreadableExclusion};
use std::collections::HashMap;

/// OG shortcut values retain a single string or all vector alternatives over IPC.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum ShortcutBinding {
    /// One binding, including the legacy "false" disabled sentinel.
    Single(String),
    /// Alternative bindings, in user order; an empty vector disables the command.
    Multiple(Vec<String>),
}

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
    /// Visible diagnostics for malformed journal configuration values and
    /// journal date patterns with parts Tine does not support.
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
    /// User keybinding overrides from OG `:shortcuts`: strings, vectors or false.
    pub shortcuts: HashMap<String, ShortcutBinding>,
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
    /// `:ref/default-open-blocks-level` — the level to which a reference or
    /// query result opens by default: a block at level `>=` this that has
    /// children starts folded, level 1 being the displayed hit itself (OG
    /// `block-default-collapsed?`, `handler/editor.cljs`; GH #793). OG takes any
    /// integer (zero and negative values fold every hit that has children);
    /// absent or not an integer means 2.
    pub ref_default_open_blocks_level: i32,
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
    /// empty journal page: trimmed, and a blank name is `None` (OG
    /// `state/get-default-journal-template`).
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
    /// `:quick-capture-templates {:text "…"}` — OG's template for shared or
    /// quick-captured text (`frontend.quick-capture/quick-capture`,
    /// `frontend.mobile.intent/handle-payload`). `None` means the caller's OG
    /// default; only a string directly inside the top-level map counts.
    pub quick_capture_template_text: Option<String>,
    /// `:quick-capture-templates {:media "…"}` — OG's template for one shared
    /// image or file (`frontend.mobile.intent/embed-asset-file`).
    pub quick_capture_template_media: Option<String>,
    /// `:quick-capture-options {:default-page "Name"}` — OG's page a capture
    /// goes to when none is named (`frontend.quick-capture/quick-capture`).
    /// Only a non-blank string directly inside the top-level map counts.
    pub quick_capture_default_page: Option<String>,
    /// `:quick-capture-options {:insert-today? true}` — a capture without a
    /// named page goes to today's journal, ahead of `:default-page`. Only the
    /// literal `true` counts; anything else keeps OG's `false`.
    pub quick_capture_insert_today: bool,
    /// `:quick-capture-options {:redirect-page? true}` — after a capture the
    /// app opens the page it landed on. Only the literal `true` counts.
    pub quick_capture_redirect_page: bool,
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
    /// `:ui/enable-tooltip?` — OG default ON (`state/enable-tooltip?`); only
    /// explicit false switches the page, tag and block-reference previews off.
    pub enable_tooltip: bool,
    /// `:feature/enable-journals?` — OG default ON (`state/enable-journals?`:
    /// `(not (false? …))`); only explicit false turns the journals feature off
    /// (the sidebar offers the configured home page instead of Journals).
    pub enable_journals: bool,
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
    /// The entry as an OG `#+BEGIN_QUERY` block (its EDN map re-printed, title
    /// and options included), for seeding a today's-sections page. One-shot:
    /// Tine never writes `:default-queries` back. None for an invalid entry.
    #[serde(default)]
    pub seed: Option<String>,
}

/// OG's default when `:ref/linked-references-collapsed-threshold` is absent;
/// the one declaration the `Config` default and the graph-meta DTO share.
pub const DEFAULT_LINKED_REFERENCES_COLLAPSED_THRESHOLD: u32 = 100;

/// OG's default when `:ref/default-open-blocks-level` is absent or not an
/// integer (`state/get-ref-open-blocks-level`); shared like the one above.
pub const DEFAULT_REF_OPEN_BLOCKS_LEVEL: i32 = 2;

/// `logseq/config.edn` for a new graph (triple-lowbar namespace filenames, the
/// welcome page pinned as a favorite). The store writes it when a new graph's
/// seed supplies none; the onboarding graph seeds it explicitly.
pub const NEW_GRAPH_CONFIG_EDN: &str = include_str!("new-graph-config.edn");

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
            ref_default_open_blocks_level: DEFAULT_REF_OPEN_BLOCKS_LEVEL,
            separated_by_commas: Vec::new(),
            ignored_page_references_keywords: Vec::new(),
            property_pages_enabled: true,
            property_pages_excludelist: Vec::new(),
            default_journal_template: None,
            default_home: None,
            favorites: Vec::new(),
            favorites_page: None,
            mobile_gestures_disabled_in_block_with_tags: Vec::new(),
            quick_capture_template_text: None,
            quick_capture_template_media: None,
            quick_capture_default_page: None,
            quick_capture_insert_today: false,
            quick_capture_redirect_page: false,
            journal_file_name_format: None,
            journal_page_title_format: None,
            preferred_format: crate::model::Format::Md,
            file_name_format: FileNameFormat::Legacy,
            macros: HashMap::new(),
            enable_timetracking: true,
            enable_search_remove_accents: true,
            show_brackets: true,
            enable_tooltip: true,
            enable_journals: true,
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
    /// Effective reading settings for a source publication, excluding defaults.
    /// O(retained settings and their values), with no access to authored text.
    pub fn publication_reading_values(&self) -> Vec<(&'static str, crate::edn::Edn)> {
        use crate::edn::Edn;

        // Exhaustive so that a new field requires a disclosure decision.
        let Self {
            journals_dir,
            pages_dir,
            start_of_week,
            block_hidden_properties,
            separated_by_commas,
            ignored_page_references_keywords,
            property_pages_enabled,
            property_pages_excludelist,
            journal_file_name_format,
            journal_page_title_format,
            preferred_format,
            file_name_format,
            // Filtered, replaced, or derived from the retained entries.
            macros: _,
            default_home: _,
            journal_config_diagnostics: _,
            // Device, publication, private graph and editing settings.
            default_journal_queries: _,
            disable_scheduled_and_deadline_query: _,
            scheduled_future_days: _,
            hidden: _,
            hidden_parse_failed_closed: _,
            preferred_workflow: _,
            shortcuts: _,
            all_pages_public: _,
            linked_references_collapsed_threshold: _,
            ref_default_open_blocks_level: _,
            default_journal_template: _,
            favorites: _,
            mobile_gestures_disabled_in_block_with_tags: _,
            quick_capture_template_text: _,
            quick_capture_template_media: _,
            quick_capture_default_page: _,
            quick_capture_insert_today: _,
            quick_capture_redirect_page: _,
            favorites_page: _,
            enable_timetracking: _,
            enable_search_remove_accents: _,
            show_brackets: _,
            enable_tooltip: _,
            enable_journals: _,
            doc_mode_enter_for_new_block: _,
            logical_outdenting: _,
            logbook: _,
            guide_announced: _,
        } = self;
        let defaults = Self::default();
        let set =
            |members: &[String]| Edn::Set(members.iter().cloned().map(Edn::Keyword).collect());
        [
            (
                ":pages-directory",
                (pages_dir != &defaults.pages_dir).then(|| Edn::Str(pages_dir.clone())),
            ),
            (
                ":journals-directory",
                (journals_dir != &defaults.journals_dir).then(|| Edn::Str(journals_dir.clone())),
            ),
            (
                ":journal/page-title-format",
                journal_page_title_format.clone().map(Edn::Str),
            ),
            (
                ":journal/file-name-format",
                journal_file_name_format.clone().map(Edn::Str),
            ),
            (
                ":preferred-format",
                (*preferred_format == crate::model::Format::Org)
                    .then(|| Edn::Keyword("org".into())),
            ),
            (
                ":file/name-format",
                (*file_name_format == FileNameFormat::TripleLowbar)
                    .then(|| Edn::Keyword("triple-lowbar".into())),
            ),
            (
                ":block-hidden-properties",
                (!block_hidden_properties.is_empty()).then(|| set(block_hidden_properties)),
            ),
            (
                ":property/separated-by-commas",
                (!separated_by_commas.is_empty()).then(|| set(separated_by_commas)),
            ),
            (
                ":property-pages/enabled?",
                (*property_pages_enabled != defaults.property_pages_enabled)
                    .then_some(Edn::Bool(*property_pages_enabled)),
            ),
            (
                ":property-pages/excludelist",
                (!property_pages_excludelist.is_empty()).then(|| set(property_pages_excludelist)),
            ),
            (
                ":ignored-page-references-keywords",
                (!ignored_page_references_keywords.is_empty())
                    .then(|| set(ignored_page_references_keywords)),
            ),
            (
                ":start-of-week",
                (*start_of_week != defaults.start_of_week)
                    .then_some(Edn::Int((*start_of_week).into())),
            ),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect()
    }

    /// The graph's journal date answerer: the one place the two config
    /// strings become a [`crate::date::JournalFormat`] (I-12, CM step 2).
    pub fn journal_format(&self) -> crate::date::JournalFormat {
        crate::date::JournalFormat::new(
            self.journal_file_name_format.as_deref(),
            self.journal_page_title_format.as_deref(),
        )
    }

    /// Parse supported EDN keys independently, defaulting missing or malformed
    /// values. A journal date pattern with parts Tine does not support keeps its
    /// value and adds a `journal_config_diagnostics` entry (card g4czVA).
    pub fn parse(edn: &str) -> Config {
        Self::parse_settings(crate::edn::config_entries(edn))
    }

    /// The config of input over the reader's byte limit, for a caller whose
    /// bounded read refused it before [`Config::parse`]: the reader's own
    /// verdict for such input (unreadable exclusion, EDN model R4a).
    pub fn over_read_limit() -> Config {
        Self::parse_settings(Err(UnreadableExclusion))
    }

    /// The typed config from Logseq's first-form framing for publication.
    /// Refuses a malformed form or a non-map. Empty source uses defaults.
    /// This reads the entire admitted form once, within the config size limit.
    pub fn parse_for_publication(source: &str) -> Option<Self> {
        let Some(form) = crate::edn::first_form(source)? else {
            return Some(Self::default());
        };
        let entries = crate::edn::map_entries(form)?;
        Some(Self::parse_settings(Ok(entries)))
    }

    fn parse_settings(entries: Result<Vec<ConfigEntry>, UnreadableExclusion>) -> Self {
        let entries = entries.as_deref().map_err(|&error| error);
        let settings = &Settings(entries.unwrap_or_default());
        let mut cfg = Self::default();
        let journal = parse_journal_settings(settings);
        cfg.default_journal_queries = journal.queries;
        cfg.disable_scheduled_and_deadline_query = journal.disable;
        cfg.scheduled_future_days = journal.future_days;
        cfg.journal_config_diagnostics = journal.diagnostics;
        if let Some(v) = settings.string(":journals-directory") {
            cfg.journals_dir = v;
        }
        if let Some(v) = settings.string(":pages-directory") {
            cfg.pages_dir = v;
        }
        match entries.and_then(edn::config_exclusions) {
            Ok(hidden) => cfg.hidden = hidden,
            Err(UnreadableExclusion) => cfg.hidden_parse_failed_closed = true,
        }
        if let Some(v) = settings.keyword(":preferred-workflow") {
            cfg.preferred_workflow = if v == "todo" {
                Workflow::Todo
            } else {
                Workflow::Now
            };
        }
        cfg.shortcuts = parse_shortcuts(settings.get(":shortcuts"));
        cfg.all_pages_public = settings
            .boolean(":publishing/all-pages-public?")
            .unwrap_or(cfg.all_pages_public);
        if let Some(n) = settings.integer(":start-of-week") {
            if n <= 6 {
                cfg.start_of_week = n;
            }
        }
        cfg.block_hidden_properties = settings.keyword_set(":block-hidden-properties");
        if let Some(n) = settings.integer(":ref/linked-references-collapsed-threshold") {
            cfg.linked_references_collapsed_threshold = n;
        }
        if let Some(n) = settings.signed(":ref/default-open-blocks-level") {
            cfg.ref_default_open_blocks_level = n;
        }
        cfg.separated_by_commas = settings.keyword_set(":property/separated-by-commas");
        cfg.ignored_page_references_keywords =
            settings.keyword_set(":ignored-page-references-keywords");
        cfg.property_pages_enabled = settings
            .boolean(":property-pages/enabled?")
            .unwrap_or(cfg.property_pages_enabled);
        cfg.property_pages_excludelist = settings.keyword_set(":property-pages/excludelist");
        cfg.default_journal_template = settings
            .nested_string(":default-templates", ":journals")
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        cfg.default_home = settings
            .nested_string(":default-home", ":page")
            .filter(|s| !s.trim().is_empty());
        cfg.favorites = settings.string_vector(":favorites");
        cfg.mobile_gestures_disabled_in_block_with_tags =
            settings.nested_string_vector(":mobile", ":gestures/disabled-in-block-with-tags");
        cfg.quick_capture_template_text =
            settings.nested_string(":quick-capture-templates", ":text");
        cfg.quick_capture_template_media =
            settings.nested_string(":quick-capture-templates", ":media");
        cfg.quick_capture_default_page = settings
            .nested_string(":quick-capture-options", ":default-page")
            .filter(|name| !name.trim().is_empty());
        cfg.quick_capture_insert_today = settings
            .nested_bool(":quick-capture-options", ":insert-today?")
            .unwrap_or(cfg.quick_capture_insert_today);
        cfg.quick_capture_redirect_page = settings
            .nested_bool(":quick-capture-options", ":redirect-page?")
            .unwrap_or(cfg.quick_capture_redirect_page);
        cfg.favorites_page = settings
            .string(":tine/favorites-page")
            .filter(|s| !s.trim().is_empty());
        let file_format = settings
            .string(":journal/file-name-format")
            .filter(|s| !s.is_empty());
        let title_format = [":journal/page-title-format", ":date-formatter"]
            .into_iter()
            .find_map(|key| {
                settings
                    .string(key)
                    .filter(|s| !s.is_empty())
                    .map(|v| (key, v))
            });
        for (key, pattern) in [
            file_format
                .as_deref()
                .map(|p| (":journal/file-name-format", p)),
            title_format.as_ref().map(|(key, p)| (*key, p.as_str())),
        ]
        .into_iter()
        .flatten()
        {
            cfg.journal_config_diagnostics
                .extend(unsupported_journal_pattern_diagnostic(key, pattern));
        }
        cfg.journal_file_name_format = file_format;
        cfg.journal_page_title_format = title_format.map(|(_, v)| v);
        // OG stores `:preferred-format "Markdown"|"Org"` (a capitalized string), but
        // its schema also accepts the keyword form `:preferred-format :org` — read
        // both so a keyword-configured graph isn't silently treated as markdown.
        if let Some(v) = settings
            .string(":preferred-format")
            .or_else(|| settings.keyword(":preferred-format"))
        {
            if v.eq_ignore_ascii_case("org") {
                cfg.preferred_format = crate::model::Format::Org;
            }
        }
        // `:file/name-format` is a keyword (`:triple-lowbar` | `:legacy`). Absent
        // ⇒ legacy, matching OG's `(or (:file/name-format config) :legacy)`.
        cfg.file_name_format = match settings.keyword(":file/name-format").as_deref() {
            Some("triple-lowbar") => FileNameFormat::TripleLowbar,
            _ => FileNameFormat::Legacy,
        };
        cfg.macros = macro_entries(settings.get(":macros"));
        cfg.enable_timetracking = settings
            .boolean(":feature/enable-timetracking?")
            .unwrap_or(cfg.enable_timetracking);
        cfg.enable_search_remove_accents = settings
            .get(":feature/enable-search-remove-accents?")
            .map(|form| form.leaf != Some(Edn::Bool(false)))
            .unwrap_or(cfg.enable_search_remove_accents);
        cfg.show_brackets = settings
            .boolean(":ui/show-brackets?")
            .unwrap_or(cfg.show_brackets);
        cfg.enable_tooltip = settings
            .boolean(":ui/enable-tooltip?")
            .unwrap_or(cfg.enable_tooltip);
        cfg.enable_journals = settings
            .boolean(":feature/enable-journals?")
            .unwrap_or(cfg.enable_journals);
        cfg.doc_mode_enter_for_new_block = settings
            .boolean(":shortcut/doc-mode-enter-for-new-block?")
            .unwrap_or(cfg.doc_mode_enter_for_new_block);
        cfg.logical_outdenting = settings
            .boolean(":editor/logical-outdenting?")
            .unwrap_or(cfg.logical_outdenting);
        let default = cfg.logbook;
        cfg.logbook = LogbookSettings {
            with_second_support: settings
                .nested_bool(":logbook/settings", ":with-second-support?")
                .unwrap_or(default.with_second_support),
            enabled_in_timestamped_blocks: settings
                .nested_bool(":logbook/settings", ":enabled-in-timestamped-blocks")
                .unwrap_or(default.enabled_in_timestamped_blocks),
            enabled_in_all_blocks: settings
                .nested_bool(":logbook/settings", ":enabled-in-all-blocks")
                .unwrap_or(default.enabled_in_all_blocks),
        };
        cfg.guide_announced = settings
            .boolean(":tine/guide-announced?")
            .unwrap_or(cfg.guide_announced);
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
/// Decode one complete string token with the shared EDN value parser.
fn journal_setting_value(settings: &Settings<'_>, key: &str) -> Option<Option<Edn>> {
    Some(settings.entry(key)?.as_ref().map(Form::value))
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

/// Card g4czVA: the visible warning for a journal date pattern with parts
/// Tine does not support. Detection only; Tine renders the pattern as before.
fn unsupported_journal_pattern_diagnostic(key: &str, pattern: &str) -> Option<String> {
    use crate::date::UnsupportedHandling;
    let mut parts: Vec<String> = Vec::new();
    for token in crate::date::unsupported_tokens(pattern) {
        let effect = match token.handling {
            UnsupportedHandling::Literal => "written as literal text",
            UnsupportedHandling::ShorterFields => "read as shorter date fields",
            UnsupportedHandling::Quoted => "quote marks kept, letters inside read as date fields",
        };
        let part = format!("{} ({effect})", token.text);
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let what = if key == ":journal/file-name-format" {
        "file names"
    } else {
        "titles"
    };
    Some(format!(
        "{key} \"{pattern}\" uses date pattern parts Tine does not support: {}. Tine may write and recognize journal {what} differently from Logseq; it renames and rewrites nothing.",
        parts.join(", ")
    ))
}

/// The three Logseq journal keys, returned rather than written into a `Config`
/// so the checkpoint-key guard does not count this parser as a build reader.
#[derive(Default)]
struct JournalSettings {
    queries: Vec<JournalQuery>,
    disable: bool,
    future_days: Option<i32>,
    diagnostics: Vec<String>,
}

fn parse_journal_settings(settings: &Settings<'_>) -> JournalSettings {
    let mut cfg = JournalSettings::default();
    use crate::edn::{self, Edn};
    let horizon = ":scheduled/future-days";
    if let Some(value) = journal_setting_value(settings, horizon) {
        match value {
            Some(Edn::Int(n)) if i32::try_from(n).is_ok() => cfg.future_days = Some(n as i32),
            _ => cfg.diagnostics.push(format!(
                "{horizon} must be an integer; using the Settings value."
            )),
        }
    }
    let disable = ":feature/disable-scheduled-and-deadline-query?";
    if let Some(value) = journal_setting_value(settings, disable) {
        match value {
            Some(Edn::Bool(b)) => cfg.disable = b,
            _ => cfg
                .diagnostics
                .push(format!("{disable} must be true or false; using false.")),
        }
    }
    let Some(value) = journal_setting_value(settings, ":default-queries") else {
        return cfg;
    };
    let Some(Edn::Map(_)) = value.as_ref() else {
        cfg.diagnostics
            .push(":default-queries must be a map.".into());
        return cfg;
    };
    let Some(journals) = value.as_ref().and_then(|v| v.get("journals")) else {
        return cfg;
    };
    let Edn::Vec(queries) = journals else {
        cfg.diagnostics
            .push(":default-queries :journals must be a vector.".into());
        return cfg;
    };
    for (index, entry) in queries.iter().enumerate() {
        let title = entry.get("title").and_then(journal_title);
        let mut query = JournalQuery {
            title,
            body: String::new(),
            error: None,
            seed: None,
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
                query.seed = Some(format!("#+BEGIN_QUERY\n{}\n#+END_QUERY", edn::to_string(entry)));
                if entry.get("title").is_some_and(|v| !matches!(v, Edn::Nil) && journal_title(v).is_none()) {
                    cfg.diagnostics.push(format!(":default-queries :journals entry {} has no plain-text title.", index + 1));
                }
            }
            _ => query.error = Some(format!("Invalid default journal query {}: expected a map with :query (a string, list or vector).", index + 1)),
        }
        cfg.queries.push(query);
    }
    cfg
}

struct Settings<'a>(&'a [ConfigEntry]);

impl Settings<'_> {
    fn entry(&self, key: &str) -> Option<&Option<Form>> {
        let key = key.strip_prefix(':').unwrap_or(key);
        self.0.iter().find_map(|(name, value)| {
            matches!(&name.leaf, Some(Edn::Keyword(name)) if name == key).then_some(value)
        })
    }

    fn get(&self, key: &str) -> Option<&Form> {
        self.entry(key)?.as_ref()
    }

    fn string(&self, key: &str) -> Option<String> {
        match self.get(key)?.leaf.as_ref()? {
            Edn::Str(value) => Some(value.clone()),
            _ => None,
        }
    }

    fn keyword(&self, key: &str) -> Option<String> {
        match self.get(key)?.leaf.as_ref()? {
            Edn::Keyword(value) => Some(value.clone()),
            _ => None,
        }
    }

    fn boolean(&self, key: &str) -> Option<bool> {
        match self.get(key)?.leaf.as_ref()? {
            Edn::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn integer(&self, key: &str) -> Option<u32> {
        self.get(key)?
            .leaf
            .as_ref()?
            .exact_integer()?
            .try_into()
            .ok()
    }

    fn signed(&self, key: &str) -> Option<i32> {
        self.get(key)?
            .leaf
            .as_ref()?
            .exact_integer()
            .map(|value| value.clamp(i32::MIN.into(), i32::MAX.into()) as i32)
    }

    fn keyword_set(&self, key: &str) -> Vec<String> {
        self.get(key)
            .filter(|form| form.kind == Kind::Set)
            .map_or_else(Vec::new, |form| {
                form.children
                    .iter()
                    .filter_map(|item| match &item.leaf {
                        Some(Edn::Keyword(name)) => Some(name.clone()),
                        _ => None,
                    })
                    .collect()
            })
    }

    fn string_vector(&self, key: &str) -> Vec<String> {
        strings_in_vector(self.get(key))
    }

    fn nested_string(&self, outer: &str, key: &str) -> Option<String> {
        match nested(self.get(outer)?, key)?.leaf.as_ref()? {
            Edn::Str(value) => Some(value.clone()),
            _ => None,
        }
    }

    fn nested_bool(&self, outer: &str, key: &str) -> Option<bool> {
        match nested(self.get(outer)?, key)?.leaf.as_ref()? {
            Edn::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn nested_string_vector(&self, outer: &str, key: &str) -> Vec<String> {
        strings_in_vector(self.get(outer).and_then(|form| nested(form, key)))
    }
}

fn nested<'a>(form: &'a Form, key: &str) -> Option<&'a Form> {
    if form.kind != Kind::Map {
        return None;
    }
    let key = key.strip_prefix(':').unwrap_or(key);
    form.children.chunks_exact(2).find_map(|pair| {
        matches!(&pair[0].leaf, Some(Edn::Keyword(name)) if name == key).then_some(&pair[1])
    })
}

/// Byte index of `key` among the DIRECT entries of the root map, never inside
/// a nested map, vector, comment or string. Every top-level config setter
/// locates its key here: the depth-blind keyword search returned a nested
/// shadow first and the setter spliced over it (master DUP-3, 4ae2f6f4f).
/// `None` when the key is absent or there is no balanced root map.
pub fn find_top_level_keyword(s: &str, key: &str) -> Option<usize> {
    let map = read_for_edit(s)??;
    edit_entry(&map, key).map(|(key, _)| key.span.start)
}

/// Config edit spans from the shared reader. Missing final values in a closed
/// map are visible for repair. Blank input is valid without a map.
pub fn read_for_edit(source: &str) -> Option<Option<Form>> {
    crate::edn::read_config_edit(source)
}

/// A settings write must leave a complete EDN map within the config bound.
pub fn edit_is_complete(source: &str) -> bool {
    crate::edn::read(source, crate::edn::MAX_INPUT_BYTES, crate::edn::Mode::Edn)
        .is_some_and(|form| form.kind == Kind::Map)
}

/// The keyword token and optional value at the effective key in this map.
pub fn edit_entry<'a>(map: &'a Form, key: &str) -> Option<(&'a Form, Option<&'a Form>)> {
    let key = key.strip_prefix(':').unwrap_or(key);
    (map.kind == Kind::Map).then_some(())?;
    map.children
        .chunks(2)
        .find_map(|pair| (pair[0].keyword() == Some(key)).then(|| (&pair[0], pair.get(1))))
}

/// The range of a value whose replacement preserves authored trivia.
pub fn editable_value(form: &Form) -> Option<std::ops::Range<usize>> {
    (!form.protected_trivia).then(|| form.span.clone())
}

/// The direct `:shortcuts` entries, with their supported binding shapes.
fn parse_shortcuts(form: Option<&Form>) -> HashMap<String, ShortcutBinding> {
    let Some(form) = form.filter(|form| form.kind == Kind::Map) else {
        return HashMap::new();
    };
    form.children
        .chunks_exact(2)
        .filter_map(|pair| {
            let Some(Edn::Keyword(key)) = &pair[0].leaf else {
                return None;
            };
            let value = &pair[1];
            let binding = match &value.leaf {
                Some(Edn::Str(value)) => ShortcutBinding::Single(value.clone()),
                Some(Edn::Bool(false)) => ShortcutBinding::Single("false".into()),
                _ if value.kind == Kind::Vector => {
                    let bindings = strings_in_vector(Some(value));
                    if bindings.len() != value.children.len() {
                        return None;
                    }
                    ShortcutBinding::Multiple(bindings)
                }
                _ => return None,
            };
            Some((key.clone(), binding))
        })
        .collect()
}

// Logseq looks up the string key before its keyword counterpart. The config
// model stops at the first pair outside the supported name/template shapes.
fn macro_entries(form: Option<&Form>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(form) = form.filter(|form| form.kind == Kind::Map) else {
        return map;
    };
    let mut string_keys = std::collections::HashSet::new();
    for pair in form.children.chunks_exact(2) {
        let (key, keyword) = match &pair[0].leaf {
            Some(Edn::Keyword(key)) => (key, true),
            Some(Edn::Str(key)) => (key, false),
            _ => break,
        };
        let Some(Edn::Str(template)) = &pair[1].leaf else {
            break;
        };
        if !keyword {
            string_keys.insert(key);
        }
        if !key.is_empty() && (!keyword || !string_keys.contains(key)) {
            map.insert(key.clone(), template.clone());
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
        // GH #605 seed: each entry re-printed whole (hiccup title, options,
        // advanced inputs) inside one OG BEGIN_QUERY container.
        for (query, original) in cfg.default_journal_queries.iter().zip([
            r#"{:title [:b "Tasks " [:span "today"]] :query (task TODO) :collapsed? true}"#,
            r#"{:title "Advanced" :query [:find ?b :in $ ?day :where [?b :block/scheduled ?day]] :inputs [:today]}"#,
        ]) {
            let seed = query.seed.as_deref().unwrap();
            let inner = seed
                .strip_prefix("#+BEGIN_QUERY\n")
                .and_then(|rest| rest.strip_suffix("\n#+END_QUERY"))
                .unwrap();
            assert_eq!(crate::edn::parse(inner), crate::edn::parse(original));
        }
        let title = Config::parse(
            r#"{:default-queries {:journals [{:title ["span" {:onclick "ignored"} "Count " 2 (graph-code)] :query (task TODO)}]}}"#,
        );
        assert_eq!(
            title.default_journal_queries[0].title.as_deref(),
            Some("Count 2")
        );
    }

    /// Card g4czVA: an unsupported journal pattern part is a visible
    /// diagnostic naming the key that supplied the pattern; the configured
    /// value itself is kept unchanged.
    #[test]
    fn unsupported_journal_pattern_parts_are_diagnosed() {
        let cfg = Config::parse(
            r#"{:journal/file-name-format "yyyy-MM-dd-ww" :journal/page-title-format "yyyy-MM-dd 'w'ww-E"}"#,
        );
        assert_eq!(
            cfg.journal_config_diagnostics,
            vec![
                ":journal/file-name-format \"yyyy-MM-dd-ww\" uses date pattern parts Tine does not support: ww (written as literal text). Tine may write and recognize journal file names differently from Logseq; it renames and rewrites nothing.".to_string(),
                ":journal/page-title-format \"yyyy-MM-dd 'w'ww-E\" uses date pattern parts Tine does not support: 'w' (quote marks kept, letters inside read as date fields), ww (written as literal text). Tine may write and recognize journal titles differently from Logseq; it renames and rewrites nothing.".to_string(),
            ]
        );
        assert_eq!(
            cfg.journal_file_name_format.as_deref(),
            Some("yyyy-MM-dd-ww")
        );
        assert_eq!(
            cfg.journal_page_title_format.as_deref(),
            Some("yyyy-MM-dd 'w'ww-E")
        );
        // The legacy key is named when it supplies the title pattern; a
        // repeated part is listed once.
        let legacy = Config::parse(r#"{:date-formatter "yyyy dddd dddd"}"#);
        assert_eq!(
            legacy.journal_config_diagnostics,
            vec![":date-formatter \"yyyy dddd dddd\" uses date pattern parts Tine does not support: dddd (read as shorter date fields). Tine may write and recognize journal titles differently from Logseq; it renames and rewrites nothing.".to_string()]
        );
        // A shadowed legacy pattern is not used, so it is not diagnosed.
        let shadowed = Config::parse(
            r#"{:journal/page-title-format "EEEE, dd.MM.yyyy" :date-formatter "ww"}"#,
        );
        assert!(shadowed.journal_config_diagnostics.is_empty());
        for edn in [
            "{}",
            r#"{:journal/file-name-format "yyyy_MM_dd" :journal/page-title-format "MMM do, yyyy"}"#,
            r#"{:journal/page-title-format "yyyy年MM月dd日"}"#,
        ] {
            assert!(
                Config::parse(edn).journal_config_diagnostics.is_empty(),
                "{edn}"
            );
        }
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
    fn unreadable_config_suffix_with_either_hidden_spelling_fails_closed() {
        for (prefix, key) in [
            ("", ":hidden"),
            ("#:demo", ":_/hidden"),
            ("^:meta #:demo", ":_/hidden"),
            ("#^:meta #:demo", ":_/hidden"),
        ] {
            let suffix = format!("{prefix}{{{key} [\"pages/Secret.md\"");
            for newline in ["\n", "\r\n"] {
                let root = format!("{prefix}{{}}{newline}");
                let cfg = Config::parse(&format!("{root}{suffix}"));
                assert!(cfg.hidden_parse_failed_closed, "{root}{suffix}");
                let cfg = Config::parse(&format!("{root}; {suffix}"));
                assert!(!cfg.hidden_parse_failed_closed, "{root}; {suffix}");
            }
        }
    }

    #[test]
    fn config_trivia_uses_defaults_and_keeps_pages_visible() {
        let mut failures = Vec::new();
        for (prefix, key) in [("", ":hidden"), ("#:demo", ":_/hidden")] {
            let map = format!(r#"{prefix}{{{key} ["pages/Secret.md"]}}"#);
            for form in [
                format!("; {map}"),
                format!("#! {map}"),
                format!("#_ {map}"),
                format!("#_ ^:meta {map}"),
                format!("#_ #^:meta {map}"),
                format!("#_ {key}"),
                format!(r#"#_ "{key}""#),
            ] {
                for end in ["", "\n", "\r", "\r\n"] {
                    let source = format!(" , {form}{end}");
                    let cfg = Config::parse(&source);
                    if cfg.hidden_parse_failed_closed || !cfg.hidden.is_empty() {
                        failures.push(format!("{source:?}"));
                    }
                    assert_eq!(cfg.pages_dir, Config::default().pages_dir, "{source:?}");
                }
            }
        }
        assert!(
            failures.is_empty(),
            "Exclusions from trivia:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn config_root_spellings_recover_the_same_entries() {
        let mut failures = Vec::new();
        for (prefix, key_ns) in [
            ("", ""),
            ("#:demo", "_/"),
            ("^:meta ", ""),
            ("#^:meta ", ""),
            ("^:meta #:demo", "_/"),
            ("#^:meta #:demo", "_/"),
            ("^:meta #^{:note true} #:demo", "_/"),
        ] {
            let hidden = format!(r#":{key_ns}hidden ["pages/Secret.md"]"#);
            let pages = format!(r#":{key_ns}pages-directory "notes""#);
            let namespaced = if key_ns.is_empty() {
                ""
            } else {
                r#":hidden [] :pages-directory "namespace-only""#
            };
            // `##` with a complete operand other than NaN/Inf/-Inf is damaged like any other value.
            let symbolic = [
                "##unknown",
                "##nil",
                "##true",
                "##1",
                "##:kw",
                r#"##"s""#,
                r"##\c",
                "##[]",
                "##()",
                "##{}",
                "###{}",
            ];
            for broken in [r#"#unknown []"#, r#""bad\q""#, r#"\malformed"#]
                .into_iter()
                .chain(symbolic)
            {
                let broken = format!(":bad {broken}");
                for [first, second] in [[&broken, &hidden], [&hidden, &broken]] {
                    for newline in ["\n", "\r\n"] {
                        let source = format!(
                            "{prefix}{{{newline}{namespaced}{newline}{first}{newline}{second}{newline}{pages}{newline}}}"
                        );
                        let cfg = Config::parse(&source);
                        if cfg.hidden != ["pages/Secret.md"]
                            || cfg.hidden_parse_failed_closed
                            || cfg.pages_dir != "notes"
                        {
                            failures.push(format!(
                                "{source:?} => hidden={:?}, closed={}, pages_dir={:?}",
                                cfg.hidden, cfg.hidden_parse_failed_closed, cfg.pages_dir
                            ));
                        }
                        assert!(
                            Config::parse_for_publication(&source).is_none(),
                            "{source:?}"
                        );
                    }
                }
            }
            for end in [r#"["pages/Secret.md""#, r#"["pages/Secret.md"]"#] {
                let source = format!("{prefix}{{:{key_ns}hidden {end}");
                if !Config::parse(&source).hidden_parse_failed_closed {
                    failures.push(format!("{source:?} => torn config did not fail closed"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "Wrong config recovery in {} cases:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn oversized_config_fails_closed() {
        let limit = crate::edn::MAX_INPUT_BYTES;
        let source = " ".repeat(limit + 1);
        let cfg = Config::parse(&source[..limit]);
        assert!(!cfg.hidden_parse_failed_closed);
        let cfg = Config::parse(&source);
        assert!(cfg.hidden_parse_failed_closed);
    }

    #[test]
    fn unreadable_config_counts_only_effective_keys_lexed_before_failure() {
        let mut failures = Vec::new();
        for (read, suffix, closed, pages_dir) in [
            ("", r#"^ {:hidden []} {:bad ["torn""#, false, "pages"),
            ("", r#"#:demo{:hidden ["torn""#, false, "pages"),
            ("", r#"{:_/hidden ["torn""#, false, "pages"),
            ("", r#"{:hidden-other ["torn""#, false, "pages"),
            ("", r#"{:nested {:hidden ["torn""#, false, "pages"),
            ("", r#"{:other :hidden :bad ["torn""#, false, "pages"),
            ("", r#"{:note ":hidden" :bad ["torn""#, false, "pages"),
            ("", r#"#_ {:hidden ["torn""#, false, "pages"),
            (
                r#"{:pages-directory "notes"}"#,
                r#" ] {:hidden []}"#,
                false,
                "notes",
            ),
            (
                r#"{:pages-directory "notes"}"#,
                r#" ":hidden" #unknown []"#,
                false,
                "notes",
            ),
            (
                r#"{:pages-directory "notes"}"#,
                r#" #_ {:hidden ["torn""#,
                false,
                "notes",
            ),
            (
                r#"{:pages-directory "notes"}"#,
                r#" #_ {:hidden []} #unknown []"#,
                false,
                "notes",
            ),
            (
                r#"{:pages-directory "notes"}"#,
                r#" ; :hidden
                 #unknown []"#,
                false,
                "notes",
            ),
            (
                r#"{:pages-directory "notes"}"#,
                r#" nil {:hidden []}"#,
                true,
                "pages",
            ),
            (
                r#"{:pages-directory "notes"}"#,
                r#" {:hidden"#,
                true,
                "pages",
            ),
            ("", r#"^:meta {:hidden"#, true, "pages"),
        ] {
            let source = format!("{read}{suffix}");
            let cfg = Config::parse(&source);
            if cfg.hidden_parse_failed_closed != closed || cfg.pages_dir != pages_dir {
                failures.push(format!(
                    "{source:?}: expected ({closed}, {pages_dir}), got ({}, {})",
                    cfg.hidden_parse_failed_closed, cfg.pages_dir
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "Wrong config recovery:\n{}",
            failures.join("\n")
        );
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

    /// GH #854: a Logseq `config.edn` names macros with keywords (`:bar`). OG
    /// looks a macro up by its name string, then by `(keyword name)`, so both
    /// forms work there; Tine used to stop at the first keyword key.
    #[test]
    fn parses_macros_with_keyword_keys() {
        let edn = r#"{:macros {:bar "[:span {:class \"score s$1\"} $1]"
                               :r "red $1"}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(
            cfg.macros.get("bar").map(String::as_str),
            Some("[:span {:class \"score s$1\"} $1]")
        );
        assert_eq!(cfg.macros.get("r").map(String::as_str), Some("red $1"));
    }

    #[test]
    fn a_keyword_macro_key_no_longer_drops_later_macros() {
        let edn = r#"{:macros {"poem" "Roses are $1"
                               :g "green $1", "greet" "Hello $1"
                               ;; a comment between entries
                               :o "orange"}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(
            cfg.macros.get("poem").map(String::as_str),
            Some("Roses are $1")
        );
        assert_eq!(cfg.macros.get("g").map(String::as_str), Some("green $1"));
        assert_eq!(
            cfg.macros.get("greet").map(String::as_str),
            Some("Hello $1")
        );
        assert_eq!(cfg.macros.get("o").map(String::as_str), Some("orange"));
    }

    /// OG tries the string name first, then `(keyword name)`: when a map has
    /// both `"x"` and `:x`, the string entry wins whatever their order. A
    /// namespaced `:ns/name` matches `{{ns/name}}`, because ClojureScript's
    /// `(keyword "ns/name")` is the keyword `:ns/name`.
    #[test]
    fn macro_string_key_wins_over_keyword_key_and_namespaced_keyword_keeps_namespace() {
        let edn = r#"{:macros {:x "from keyword" "x" "from string"
                               "y" "from string" :y "from keyword"
                               :ns/name "namespaced"}}"#;
        let cfg = Config::parse(edn);
        assert_eq!(cfg.macros.get("x").map(String::as_str), Some("from string"));
        assert_eq!(cfg.macros.get("y").map(String::as_str), Some("from string"));
        assert_eq!(
            cfg.macros.get("ns/name").map(String::as_str),
            Some("namespaced")
        );
        assert_eq!(cfg.macros.get("name"), None);
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
            cfg.shortcuts.get("go/search"),
            Some(&ShortcutBinding::Single("mod+shift+k".into()))
        );
        assert_eq!(
            cfg.shortcuts.get("ui/toggle-theme"),
            Some(&ShortcutBinding::Single("t d".into()))
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
            cfg.shortcuts.get("go/search"),
            Some(&ShortcutBinding::Single("false".into()))
        );
        assert_eq!(
            cfg.shortcuts.get("editor/indent"),
            Some(&ShortcutBinding::Multiple(vec![
                "tab".into(),
                "mod+]".into()
            ]))
        );
    }

    #[test]
    fn shortcut_vectors_preserve_sequences_comments_escapes_and_empty_disable() {
        let cfg = Config::parse(
            r#"{:shortcuts {:go/search [; first
            "mod+k" ; second
            "g s" "mod+\u006a"] :ui/toggle-theme [] :invalid ["a" false]}}"#,
        );
        assert_eq!(
            cfg.shortcuts["go/search"],
            ShortcutBinding::Multiple(vec!["mod+k".into(), "g s".into(), "mod+j".into(),])
        );
        assert_eq!(
            cfg.shortcuts["ui/toggle-theme"],
            ShortcutBinding::Multiple(vec![])
        );
        assert!(!cfg.shortcuts.contains_key("invalid"));
        let json = serde_json::to_value(&cfg.shortcuts).unwrap();
        assert_eq!(
            json["go/search"],
            serde_json::json!(["mod+k", "g s", "mod+j"])
        );
        assert_eq!(json["ui/toggle-theme"], serde_json::json!([]));
        let restored: HashMap<String, ShortcutBinding> = serde_json::from_value(json).unwrap();
        assert_eq!(restored, cfg.shortcuts);
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
    fn default_journal_template_is_trimmed_and_blank_is_none_as_in_og() {
        // OG frontend.state/get-default-journal-template: `(when-not
        // (string/blank? t) (string/trim t))`.
        let template = |edn: &str| Config::parse(edn).default_journal_template;
        assert_eq!(
            template(r#"{:default-templates {:journals " Daily "}}"#).as_deref(),
            Some("Daily")
        );
        assert_eq!(
            template(r#"{:default-templates {:journals "Daily"}}"#).as_deref(),
            Some("Daily")
        );
        assert_eq!(template(r#"{:default-templates {:journals "  "}}"#), None);
        assert_eq!(template(r#"{:default-templates {:journals ""}}"#), None);
        assert_eq!(template("{}"), None);
    }

    #[test]
    fn enable_journals_is_on_unless_explicitly_false() {
        // OG `state/enable-journals?`: `(not (false? (:feature/enable-journals? config)))`.
        for (edn, enabled) in [
            ("{}", true),
            ("{:feature/enable-journals? true}", true),
            ("{:feature/enable-journals? nil}", true),
            ("{:feature/enable-journals? false}", false),
            ("{;; :feature/enable-journals? false\n}", true),
        ] {
            assert_eq!(Config::parse(edn).enable_journals, enabled, "{edn}");
        }
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
    fn ref_default_open_blocks_level_reads_the_og_key() {
        // GH #793. OG `state/get-ref-open-blocks-level`: any integer counts
        // (zero and negative included), anything else is the default 2.
        let level = |edn: &str| Config::parse(edn).ref_default_open_blocks_level;
        assert_eq!(level("{}"), 2);
        assert_eq!(DEFAULT_REF_OPEN_BLOCKS_LEVEL, 2);
        assert_eq!(Config::default().ref_default_open_blocks_level, 2);
        assert_eq!(level("{:ref/default-open-blocks-level 5}"), 5);
        assert_eq!(level("{:ref/default-open-blocks-level 1}"), 1);
        assert_eq!(level("{:ref/default-open-blocks-level 0}"), 0);
        assert_eq!(level("{:ref/default-open-blocks-level -3}"), -3);
        assert_eq!(level("{:ref/default-open-blocks-level +4}"), 4);
        assert_eq!(
            level("{:ref/default-open-blocks-level 99999999999999}"),
            i32::MAX
        );
        assert_eq!(
            level("{:ref/default-open-blocks-level 5 :start-of-week 1}"),
            5
        );
        assert_eq!(level("{:ref/default-open-blocks-level 5}\n"), 5);
        assert_eq!(level("{:ref/default-open-blocks-level ;; deeper\n 7}"), 7);
        for integer in ["5N", "5M", "5e0"] {
            assert_eq!(
                level(&format!("{{:ref/default-open-blocks-level {integer}}}")),
                5
            );
        }
        // A nonintegral number or nonnumeric value uses the default.
        for bad in [
            "\"5\"", "5.5", "1e-3", ":five", "nil", "[5]", "true", "-", "",
        ] {
            assert_eq!(
                level(&format!("{{:ref/default-open-blocks-level {bad}}}")),
                2,
                "{bad}"
            );
        }
    }

    #[test]
    fn an_unreadable_dispatch_cannot_supply_a_later_setting_boundary() {
        let config =
            Config::parse("{:ignored #<unreadable payload :block-hidden-properties #{:Private}}");
        assert!(config.block_hidden_properties.is_empty());
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
        // A vector opener terminates a keyword, but a '#' belongs to it.
        assert_eq!(
            Config::parse("{:favorites[\"X\"]}").favorites,
            vec!["X".to_string()]
        );
        assert_eq!(
            Config::parse("{:block-hidden-properties #{:id}}").block_hidden_properties,
            ["id"]
        );
        assert!(Config::parse("{:block-hidden-properties#{:id}}")
            .block_hidden_properties
            .is_empty());
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
        find_top_level_keyword(&format!("{{{body}}}"), key).map(|at| at - 1)
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

    /// GH #824: Logseq's `:ui/enable-tooltip?` (OG `state/enable-tooltip?`)
    /// reads true, false or absent; absent means previews on, as in OG.
    #[test]
    fn enable_tooltip_reads_true_false_and_absent() {
        assert!(Config::parse("{}").enable_tooltip, "absent: on");
        assert!(
            Config::parse("{:ui/enable-tooltip? true}").enable_tooltip,
            "true: on"
        );
        assert!(
            !Config::parse("{:ui/enable-tooltip? false}").enable_tooltip,
            "false: off"
        );
        assert!(
            !Config::parse(
                "{:ui/show-brackets? true\n :ui/enable-tooltip? false,\n :start-of-week 1}"
            )
            .enable_tooltip,
            "false among other keys"
        );
    }

    #[test]
    fn an_empty_or_malformed_config_serves_the_declared_defaults() {
        for edn in [
            "",
            "{}",
            "{:ui/show-brackets? maybe :logbook/settings {:with-second-support? 3}}",
            "{:ui/enable-tooltip? maybe}",
        ] {
            let cfg = Config::parse(edn);
            let default = Config::default();
            assert_eq!(cfg.show_brackets, default.show_brackets, "{edn}");
            assert_eq!(cfg.enable_tooltip, default.enable_tooltip, "{edn}");
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
                    let _ = find_top_level_keyword(&edn, key);
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

#[cfg(test)]
mod quick_capture_template_tests {
    use super::*;

    /// OG `:quick-capture-templates {:text … :media …}` (frontend.quick-capture,
    /// frontend.mobile.intent): absent keys are `None`, so the share shaper falls
    /// back to OG's own defaults; a custom template is read verbatim.
    #[test]
    fn reads_custom_templates_and_leaves_absent_ones_to_the_og_default() {
        let config = Config::parse("{}");
        assert_eq!(config.quick_capture_template_text, None);
        assert_eq!(config.quick_capture_template_media, None);
        let config = Config::parse(
            r#"{:quick-capture-templates {:text "{date} {text} {url}" :media "> {url}"}}"#,
        );
        assert_eq!(
            config.quick_capture_template_text.as_deref(),
            Some("{date} {text} {url}")
        );
        assert_eq!(
            config.quick_capture_template_media.as_deref(),
            Some("> {url}")
        );
    }

    #[test]
    fn a_nested_map_cannot_shadow_the_template() {
        let config = Config::parse(r#"{:other {:quick-capture-templates {:text "no"}}}"#);
        assert_eq!(config.quick_capture_template_text, None);
        let config = Config::parse(r#"{:quick-capture-templates {:text :not-a-string}}"#);
        assert_eq!(config.quick_capture_template_text, None);
    }
}

#[cfg(test)]
mod quick_capture_options_tests {
    use super::*;

    /// OG `:quick-capture-options` (`frontend.quick-capture/quick-capture`):
    /// absent keys keep OG's defaults (no default page, both flags `false`).
    #[test]
    fn reads_the_three_options_and_defaults_the_rest() {
        let config = Config::parse("{}");
        assert_eq!(config.quick_capture_default_page, None);
        assert!(!config.quick_capture_insert_today);
        assert!(!config.quick_capture_redirect_page);
        let config = Config::parse(
            r#"{:quick-capture-options {:default-page "Inbox" :insert-today? true :redirect-page? true}}"#,
        );
        assert_eq!(config.quick_capture_default_page.as_deref(), Some("Inbox"));
        assert!(config.quick_capture_insert_today);
        assert!(config.quick_capture_redirect_page);
        let config = Config::parse(r#"{:quick-capture-options {:default-page "Inbox"}}"#);
        assert!(!config.quick_capture_insert_today && !config.quick_capture_redirect_page);
    }

    /// A wrong shape never panics and never enables anything: a flag is on only
    /// for the literal `true`, a page only for a non-blank string, and a nested
    /// extension map cannot shadow the key.
    #[test]
    fn a_malformed_value_keeps_todays_behaviour() {
        for edn in [
            r#"{:quick-capture-options {:default-page :Inbox}}"#,
            r#"{:quick-capture-options {:default-page "  "}}"#,
            r#"{:quick-capture-options {:default-page ""}}"#,
            r#"{:quick-capture-options {:default-page 42}}"#,
            r#"{:quick-capture-options "Inbox"}"#,
            r#"{:quick-capture-options {:default-page "Inbox""#,
            r#"{:other {:quick-capture-options {:default-page "Inbox"}}}"#,
            r#"{:quick-capture-options {:nested {:default-page "Inbox"}}}"#,
        ] {
            assert_eq!(Config::parse(edn).quick_capture_default_page, None, "{edn}");
        }
        for edn in [
            r#"{:quick-capture-options {:insert-today? "true" :redirect-page? "true"}}"#,
            r#"{:quick-capture-options {:insert-today? trueish :redirect-page? 1}}"#,
            r#"{:quick-capture-options {:insert-today? nil :redirect-page? false}}"#,
            r#"{:quick-capture-options {:insert-today?"#,
            r#"{:other {:quick-capture-options {:insert-today? true :redirect-page? true}}}"#,
            r#"{:quick-capture-options {:nested {:insert-today? true}}}"#,
            r#"{:quick-capture-options true}"#,
        ] {
            let config = Config::parse(edn);
            assert!(!config.quick_capture_insert_today, "{edn}");
            assert!(!config.quick_capture_redirect_page, "{edn}");
        }
    }

    #[test]
    fn a_flag_followed_by_the_next_entry_or_the_map_end_counts() {
        let config =
            Config::parse(r#"{:quick-capture-options {:insert-today? true,:redirect-page? true}}"#);
        assert!(config.quick_capture_insert_today && config.quick_capture_redirect_page);
        let config = Config::parse(r#"{:quick-capture-options {:redirect-page? true}}"#);
        assert!(config.quick_capture_redirect_page);
    }
}
