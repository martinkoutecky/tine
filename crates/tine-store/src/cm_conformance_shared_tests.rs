//! Conceptual model conformance: the part shared by the store harness
//! (`tine-store`, `cm_conformance_tests.rs`) and the rename harness
//! (`tine-graph-features`, `tests/cm_conformance_rename.rs`). Both crates
//! include this one file by `#[path]`; it is test-only code. It names the
//! store crate only as `super::store_crate`, which each includer defines, and
//! otherwise depends on `tine_core` and `serde_json` alone.
//!
//! What lives here: the extraction of Tine's given functions into the bridge's
//! input (names, titles, aliases, outline, occurrences), the bridge call, the
//! comparison of every meaning question that a public `WholeGraph` answers
//! (pages, conflicts, owners, navigation, Linked References, backlink counts,
//! alias groups, queries), the seeded random graph generator, and the
//! expected-differences file.
//!
//! `alias_spellings` is a test-side copy of the store's crate-private
//! `query::document_alias_spellings` (the rename harness cannot call it); the
//! store harness asserts the copy equals the original on every file it sees.
#![allow(dead_code)]

#[path = "cm_conformance_native_shared_tests.rs"]
pub mod native;
#[path = "cm_conformance_r13_shared_tests.rs"]
pub mod r13;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use tine_core::config::{Config, FileNameFormat};
use tine_core::date::{JournalDate, JournalFormat};
use tine_core::doc::{self, property_key_norm, DocBlock, Document};
use tine_core::model::{
    decode_page_name, encode_page_name, page_title_from_preamble, Format, PageId, RefGroup,
};
use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::{parse_query_text, QueryDialect};
use tine_core::reference_evidence::visit_reference_targets;
use tine_core::refs::page_key;

use super::store_crate::{Cancel, InventoryScope, IrAnswer, IrRequest, Resolved, WholeGraph};

/// The fixture directory (tine-store's).
pub fn fixture_dir(manifest_dir: &str) -> std::path::PathBuf {
    let here = Path::new(manifest_dir);
    let store = if here.ends_with("tine-store") {
        here.to_path_buf()
    } else {
        here.parent().unwrap().join("tine-store")
    };
    store.join("tests/fixtures/cm-conformance")
}

/// Tine's identity fold, as the model's `key`.
/// Whether each of `rels`, once written under `root`, is listed by its
/// directory under exactly that name. A case-insensitive volume (NTFS, APFS)
/// folds `Abc.md` and `abc.md` into one file, and APFS also folds NFC and NFD
/// spellings: a fixture with such twins describes a graph that cannot exist on
/// that volume, so a harness skips its answers there ([`Section::skip_on_volume`])
/// instead of comparing a different graph. One listing per directory.
pub fn listed_apart<'a>(root: &Path, rels: impl IntoIterator<Item = &'a str>) -> bool {
    let mut listed: BTreeMap<std::path::PathBuf, BTreeSet<std::ffi::OsString>> = BTreeMap::new();
    rels.into_iter().all(|rel| {
        let path = root.join(rel);
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return false;
        };
        listed
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                fs::read_dir(dir)
                    .map(|entries| entries.filter_map(|e| Some(e.ok()?.file_name())).collect())
                    .unwrap_or_default()
            })
            .contains(name)
    })
}

pub fn fold(journal: &JournalFormat, s: &str) -> String {
    match journal.parse(s.trim()) {
        Some(day) => page_key(&journal.title(day)),
        None => page_key(s),
    }
}

fn occ_kind(rule: &str) -> &'static str {
    match rule {
        "explicit_link" | "explicit_nested_link" => "link",
        "explicit_tag" => "tag",
        "explicit_embed" => "pageEmbed",
        "explicit_property_key" => "propertyKey",
        "explicit_task_marker" => "taskMarker",
        "explicit_priority" => "priority",
        "implicit_linkable_property" => "linkableValue",
        other => panic!("reference rule {other} has no model occurrence kind"),
    }
}

/// A block's occurrences for the model: the parser-owned explicit references
/// that reach a target under the graph's config (`visit_reference_targets`,
/// the one rule behind Linked References and the link table: a property key
/// the graph's `:property-pages/enabled?` / `:property-pages/excludelist`
/// turns off names no page, as in OG `get-page-refs-from-property-names`).
fn block_json(raw: &str, level: usize, block: &DocBlock, config: &Config) -> Value {
    let occ: Vec<Value> = block
        .projection()
        .reference_source()
        .explicit
        .iter()
        .filter(|r| visit_reference_targets(r, config, |_| true))
        .map(|r| json!([occ_kind(r.rule), r.name]))
        .collect();
    json!({"text": raw, "level": level, "occ": occ, "stored": stored_queries(raw, block.is_org())})
}

/// Parser-owned query syntax, independent of reference occurrences. Leaves
/// outside the model's name algebra retain their complete IR as opaque tokens.
fn stored_queries(raw: &str, org: bool) -> Vec<Value> {
    use tine_core::query::ir::{Attr, CmpOp, Filter, Leaf, Quant, Rel};
    use tine_core::query::macro_text::{query_macro_extents, FormFamily};
    fn page_scope(q: Value) -> Value {
        for op in ["and", "or"] {
            if let Some(a) = q[op].as_array() {
                return json!({op: a.iter().cloned().map(page_scope).collect::<Vec<_>>()});
            }
        }
        if let Some(n) = q.get("not") {
            return json!({"not": page_scope(n.clone())});
        }
        json!({"page": q})
    }
    fn syntax(f: &Filter) -> Value {
        match f {
            Filter::And { items } | Filter::Or { items } if !items.is_empty() => {
                let op = if matches!(f, Filter::And { .. }) {
                    "and"
                } else {
                    "or"
                };
                items
                    .iter()
                    .map(syntax)
                    .reduce(|a, b| json!({op: [a, b]}))
                    .unwrap()
            }
            Filter::Not { inner } => json!({"not": syntax(inner)}),
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Name,
                        op: CmpOp::Eq,
                        value,
                    },
            } if value.as_text().is_some() => json!({"name": value.as_text().unwrap()}),
            Filter::Leaf {
                leaf:
                    Leaf::Rel {
                        rel,
                        quant: Quant::Any,
                        pred,
                    },
            } => {
                let p = syntax(pred);
                match rel {
                    Rel::Page => page_scope(p),
                    Rel::Refs if p["name"].is_string() => json!({"refs": p["name"]}),
                    _ => json!({"given": serde_json::to_string(f).unwrap()}),
                }
            }
            _ => json!({"given": serde_json::to_string(f).unwrap()}),
        }
    }
    let regions = tine_core::block_regions::parse(raw, org);
    query_macro_extents(raw)
        .into_iter()
        .filter(|extent| !regions.literal_at(extent.start))
        .map(|extent| {
            let dialect = match FormFamily::for_macro_name(&extent.name) {
                FormFamily::Edn => QueryDialect::Og,
                FormFamily::Tql => QueryDialect::Tql,
            };
            let (q, _) = parse_query_text(&extent.argument, dialect, JournalDate::from_days(0));
            syntax(&q.filter)
        })
        .collect()
}

fn walk(blocks: &[DocBlock], level: usize, out: &mut Vec<Value>, config: &Config) {
    for block in blocks {
        out.push(block_json(block.raw(), level, block, config));
        walk(&block.children, level + 1, out, config);
    }
}

fn is_properties_only(raw: &str) -> bool {
    tine_core::page_header::is_page_header_candidate(raw)
}

/// Test-side copy of `tine_store::query::document_alias_spellings` (crate
/// private). The store harness checks the two agree on every file.
pub fn alias_spellings(doc: &Document) -> Vec<String> {
    let is_org = tine_core::page_properties::page_document_is_org(doc);
    let alias_text: Option<&str> = match &doc.pre_block {
        Some(pre) => Some(pre.as_str()),
        None => doc
            .roots
            .first()
            .filter(|b| is_properties_only(b.raw()))
            .map(|b| b.raw()),
    };
    let Some(text) = alias_text else {
        return Vec::new();
    };
    let mut aliases = Vec::new();
    for (k, v) in tine_core::page_properties::page_property_lines(text, is_org) {
        let key = property_key_norm(&k);
        if key == "alias" || key == "aliases" {
            let trimmed = v.trim();
            if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
                continue;
            }
            aliases.extend(tine_core::property_refs::linkable_names(&v, is_org));
        }
    }
    aliases.sort_unstable();
    let mut seen = std::collections::HashSet::new();
    aliases.retain(|alias| seen.insert(page_key(alias)));
    aliases
}

/// The extractor of a file's alias declarations the harness uses.
pub type AliasFn = fn(&Document) -> Vec<String>;

/// The page format of a fixture file (`.md` or `.org`).
pub fn file_format(rel: &str) -> Format {
    if rel.ends_with(".org") {
        Format::Org
    } else {
        // `.markdown` is Markdown too; the claimant order ranks `.md` first
        // (owner fixtures o1-*).
        assert!(
            rel.ends_with(".md") || rel.ends_with(".markdown"),
            "fixtures are Markdown or Org: {rel}"
        );
        Format::Md
    }
}

/// Tine's parse of a fixture file in its format.
pub fn parse_file(rel: &str, text: &str) -> Document {
    match file_format(rel) {
        Format::Org => tine_core::org::parse_org(text),
        Format::Md => doc::parse(text),
    }
}

/// The model's view of one file: Tine's extraction as the given functions.
/// `name_format` is the graph's `:file/name-format` (the stem codec's
/// parameter; census Q2: the harness used to decode every stem as Legacy).
pub fn file_json(
    rel: &str,
    text: &str,
    aliases: AliasFn,
    name_format: FileNameFormat,
    config: &Config,
) -> Value {
    let journal_dir = rel.starts_with("journals/");
    let format = file_format(rel);
    let org = format == Format::Org;
    let file_stem = Path::new(rel).file_stem().unwrap().to_str().unwrap();
    let stem = if journal_dir {
        let journal = config.journal_format();
        journal
            .parse(file_stem)
            .map(|day| journal.title(day))
            .unwrap_or_else(|| file_stem.to_owned())
    } else {
        decode_page_name(file_stem, name_format)
    };
    let parsed = parse_file(rel, text);
    let mut blocks = Vec::new();
    let mut pre_block_kinds = Vec::new();
    if let Some(pre) = &parsed.pre_block {
        use tine_core::page_properties::{pre_block_projections, PreBlockKind};
        for (kind, block) in pre_block_projections(pre, org) {
            pre_block_kinds.push(match kind {
                PreBlockKind::Properties => "properties",
                PreBlockKind::Preamble => "preamble",
            });
            blocks.push(block_json(block.raw(), 1, &block, config));
        }
    }
    let pre_block = !blocks.is_empty();
    walk(&parsed.roots, 1, &mut blocks, config);
    json!({
        "path": rel,
        "dir": if journal_dir { "journals" } else { "pages" },
        "format": if org { "org" } else { "md" },
        "text": text,
        "stem": stem,
        "title": page_title_from_preamble(text, format),
        "aliases": aliases(&parsed),
        "preBlock": pre_block,
        "preBlockKinds": pre_block_kinds,
        "blocks": blocks,
    })
}

/// Every spelling the model may key: names, aliases, occurrences and their
/// namespace prefixes, probes, query arguments and rename arguments.
pub fn spellings(input: &Value, extra: &[String]) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = extra.iter().cloned().collect();
    for file in input["files"].as_array().unwrap() {
        for key in ["stem", "title"] {
            if let Some(s) = file[key].as_str() {
                out.insert(s.to_owned());
            }
        }
        for a in file["aliases"].as_array().unwrap() {
            out.insert(a.as_str().unwrap().to_owned());
        }
        for b in file["blocks"].as_array().unwrap() {
            for o in b["occ"].as_array().unwrap() {
                out.insert(o[1].as_str().unwrap().to_owned());
            }
        }
    }
    let mut closed = BTreeSet::new();
    for s in out {
        let parts: Vec<&str> = s.split('/').collect();
        for i in 1..parts.len() {
            closed.insert(parts[..i].join("/"));
        }
        closed.insert(s);
    }
    closed
}

/// One query: the bridge's syntax plus Tine's text and dialect.
#[derive(Clone, Debug)]
pub struct QuerySpec {
    pub anchor: String,
    pub q: Value,
    pub tine: String,
    /// "og" (the simple `{{query}}` DSL) or "tql".
    pub dialect: String,
}

/// A graph and the questions asked of it.
#[derive(Clone, Debug, Default)]
pub struct GraphSpec {
    pub name: String,
    pub files: BTreeMap<String, String>,
    pub favorites_page: Option<String>,
    /// `:journal/page-title-format` and `:journal/file-name-format` in the
    /// graph's `config.edn` (fixture `config.journalTitleFormat` /
    /// `config.journalFileFormat`); `None` leaves the Logseq default.
    pub journal_title_format: Option<String>,
    pub journal_file_format: Option<String>,
    /// `:file/name-format` (fixture `config.fileNameFormat`: `"triple-lowbar"`
    /// or `"legacy"`); `None` leaves OG's default, Legacy.
    pub file_name_format: Option<String>,
    /// Census Q2 oracle: per file, the page name OG gives it (fixture
    /// `ogNames`, written by hand from OG's source, each with its citation in
    /// the fixture's `ogSource`). Tine's codec and title parser are the
    /// model's inputs, so only this independent oracle checks them.
    pub og_names: BTreeMap<String, String>,
    /// Further `config.edn` entries, verbatim EDN (fixture `config.edn`, e.g.
    /// `":property-pages/excludelist #{:secret}"`).
    pub extra_config: Vec<String>,
    pub navigate: Vec<String>,
    pub queries: Vec<QuerySpec>,
    pub renames: Vec<(String, String)>,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default()
}

impl GraphSpec {
    pub fn from_fixture(name: &str, v: &Value) -> GraphSpec {
        let files = v["files"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let queries = v["queries"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|q| {
                        let anchor = q["anchor"].as_str().unwrap().to_owned();
                        let (tine, dialect) = match q["tine"].as_str() {
                            Some(t) => (
                                t.to_owned(),
                                q["dialect"].as_str().unwrap_or("og").to_owned(),
                            ),
                            None => tine_query_text(&anchor, &q["q"])
                                .unwrap_or_else(|| panic!("{name}: no Tine text for {q}")),
                        };
                        QuerySpec {
                            anchor,
                            q: q["q"].clone(),
                            tine,
                            dialect,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let renames = v["renames"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        (
                            r["old"].as_str().unwrap().to_owned(),
                            r["new"].as_str().unwrap().to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        GraphSpec {
            name: name.to_owned(),
            files,
            favorites_page: v["favoritesPage"].as_str().map(str::to_owned),
            journal_title_format: v["config"]["journalTitleFormat"]
                .as_str()
                .map(str::to_owned),
            journal_file_format: v["config"]["journalFileFormat"].as_str().map(str::to_owned),
            file_name_format: v["config"]["fileNameFormat"].as_str().map(str::to_owned),
            og_names: v["ogNames"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                        .collect()
                })
                .unwrap_or_default(),
            extra_config: strings(&v["config"]["edn"]),
            navigate: strings(&v["navigate"]),
            queries,
            renames,
        }
    }

    /// The fixture form, used for the smallest examples of random classes.
    pub fn to_json(&self) -> Value {
        let mut v = json!({"files": self.files});
        if let Some(f) = &self.favorites_page {
            v["favoritesPage"] = json!(f);
        }
        if let Some(f) = &self.journal_title_format {
            v["config"]["journalTitleFormat"] = json!(f);
        }
        if let Some(f) = &self.journal_file_format {
            v["config"]["journalFileFormat"] = json!(f);
        }
        if let Some(f) = &self.file_name_format {
            v["config"]["fileNameFormat"] = json!(f);
        }
        if !self.extra_config.is_empty() {
            v["config"]["edn"] = json!(self.extra_config);
        }
        if !self.queries.is_empty() {
            v["queries"] = json!(self
                .queries
                .iter()
                .map(
                    |q| json!({"anchor": q.anchor, "q": q.q, "tine": q.tine, "dialect": q.dialect})
                )
                .collect::<Vec<_>>());
        }
        if !self.renames.is_empty() {
            v["renames"] = json!(self
                .renames
                .iter()
                .map(|(o, n)| json!({"old": o, "new": n}))
                .collect::<Vec<_>>());
        }
        v
    }

    pub fn size(&self) -> usize {
        self.files.values().map(|t| t.len() + 8).sum::<usize>() + self.files.len() * 16
    }

    /// The graph's journal formats, as the store builds them from its config
    /// (`Config::journal_format`: `JournalFormat::new(file, title)`). The
    /// harness's date fold, and so the model's `dateKey`, is this native
    /// parse list, the one list Martin chose (D-20261102-JDL: native list,
    /// configured file-name format included, a deliberate OG deviation).
    pub fn journal_format(&self) -> JournalFormat {
        JournalFormat::new(
            self.journal_file_format.as_deref(),
            self.journal_title_format.as_deref(),
        )
    }

    /// The graph's `:file/name-format`, as `Config::parse` reads it.
    pub fn name_format(&self) -> FileNameFormat {
        match self.file_name_format.as_deref() {
            Some("triple-lowbar") => FileNameFormat::TripleLowbar,
            None | Some("legacy") => FileNameFormat::Legacy,
            Some(other) => panic!("{}: unknown fileNameFormat {other:?}", self.name),
        }
    }

    /// The graph's `config.edn` entries (none when the fixture configures
    /// nothing).
    fn config_entries(&self) -> Vec<String> {
        let mut config = Vec::new();
        if let Some(f) = &self.favorites_page {
            config.push(format!(":tine/favorites-page {f:?}"));
        }
        if let Some(f) = &self.journal_title_format {
            config.push(format!(":journal/page-title-format {f:?}"));
        }
        if let Some(f) = &self.journal_file_format {
            config.push(format!(":journal/file-name-format {f:?}"));
        }
        if let Some(f) = &self.file_name_format {
            config.push(format!(":file/name-format :{f}"));
        }
        config.extend(self.extra_config.iter().cloned());
        config
    }

    /// The graph's config as the store parses its `config.edn`.
    pub fn config(&self) -> Config {
        Config::parse(&format!("{{{}}}\n", self.config_entries().join("\n ")))
    }

    /// Write the graph under `root` (pages/, journals/, and a config only
    /// when the fixture configures something).
    pub fn write(&self, root: &Path) {
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        let config = self.config_entries();
        if !config.is_empty() {
            fs::create_dir_all(root.join("logseq")).unwrap();
            fs::write(
                root.join("logseq/config.edn"),
                format!("{{{}}}\n", config.join("\n ")),
            )
            .unwrap();
        }
        for (rel, text) in &self.files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }

    /// Whether the graph written under `root` has every fixture file as its
    /// own directory entry: see [`listed_apart`].
    pub fn listed_apart(&self, root: &Path) -> bool {
        listed_apart(root, self.files.iter().map(|(rel, _)| rel.as_str()))
    }

    /// Page-name arguments of `{{query …}}` macros in the graph's text, the
    /// strings rule 6.2 pushes through the rename's spelling map.
    pub fn stored_query_args(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        for text in self.files.values() {
            for macro_text in query_macros(text) {
                out.extend(query_page_args(&macro_text));
            }
        }
        out.into_iter().collect()
    }
}

/// The argument text of each `{{query …}}` macro in `text`, in order.
pub fn query_macros(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{query ") {
        let after = &rest[start + "{{query ".len()..];
        match after.find("}}") {
            Some(end) => {
                out.push(after[..end].to_owned());
                rest = &after[end..];
            }
            None => break,
        }
    }
    out
}

/// Page-name arguments in a simple-DSL query text: `[[x]]`, `(page "x")`,
/// `(page x)`, `(page-ref x)`. A plain string `"x"` is search text, not a page
/// name, and is not an argument (rule 6.2).
pub fn query_page_args(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = q;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        match after.find("]]") {
            Some(end) => {
                out.push(after[..end].to_owned());
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    for head in ["(page ", "(page-ref "] {
        let mut rest = q;
        while let Some(start) = rest.find(head) {
            let after = rest[start + head.len()..].trim_start();
            let arg = if let Some(quoted) = after.strip_prefix('"') {
                quoted.split('"').next().unwrap_or("")
            } else {
                after.split(')').next().unwrap_or("").trim()
            };
            if !arg.is_empty() && !arg.starts_with("[[") {
                out.push(arg.to_owned());
            }
            rest = &rest[start + head.len()..];
        }
    }
    out
}

/// Tine's text for a bridge query: the simple DSL for block queries, TQL for
/// page queries (the simple DSL has no page-anchored name test). `None` for a
/// leaf the printer does not express (e.g. `name` at block anchor).
pub fn tine_query_text(anchor: &str, q: &Value) -> Option<(String, String)> {
    fn og(q: &Value) -> Option<String> {
        if let Some(s) = q.get("refs").and_then(Value::as_str) {
            return Some(format!("[[{s}]]"));
        }
        if let Some(l) = q.get("page") {
            let s = l.get("name")?.as_str()?;
            return Some(format!("(page \"{s}\")"));
        }
        if let Some(a) = q.get("and").and_then(Value::as_array) {
            return Some(format!("(and {} {})", og(&a[0])?, og(&a[1])?));
        }
        if let Some(a) = q.get("or").and_then(Value::as_array) {
            return Some(format!("(or {} {})", og(&a[0])?, og(&a[1])?));
        }
        if let Some(n) = q.get("not") {
            return Some(format!("(not {})", og(n)?));
        }
        None
    }
    fn tql(q: &Value) -> Option<String> {
        if let Some(s) = q.get("refs").and_then(Value::as_str) {
            return Some(format!("[[{s}]]"));
        }
        if let Some(s) = q.get("name").and_then(Value::as_str) {
            return Some(format!("name = '{s}'"));
        }
        if let Some(a) = q.get("and").and_then(Value::as_array) {
            return Some(format!("({} and {})", tql(&a[0])?, tql(&a[1])?));
        }
        if let Some(a) = q.get("or").and_then(Value::as_array) {
            return Some(format!("({} or {})", tql(&a[0])?, tql(&a[1])?));
        }
        if let Some(n) = q.get("not") {
            return Some(format!("not ({})", tql(n)?));
        }
        None
    }
    match anchor {
        "block" => og(q).map(|t| (t, "og".to_owned())),
        _ => tql(q).map(|t| (format!("@page and {t}"), "tql".to_owned())),
    }
}

/// The abstract shape of a bridge query: its syntax with names erased.
pub fn query_shape(q: &Value) -> String {
    if q.get("refs").is_some() {
        return "refs".into();
    }
    if q.get("name").is_some() {
        return "name".into();
    }
    if let Some(l) = q.get("page") {
        return format!("page({})", query_shape(l));
    }
    if let Some(a) = q.get("and").and_then(Value::as_array) {
        return format!("and({},{})", query_shape(&a[0]), query_shape(&a[1]));
    }
    if let Some(a) = q.get("or").and_then(Value::as_array) {
        return format!("or({},{})", query_shape(&a[0]), query_shape(&a[1]));
    }
    if let Some(n) = q.get("not") {
        return format!("not({})", query_shape(n));
    }
    "?".into()
}

pub fn query_args(q: &Value, out: &mut Vec<String>) {
    for leaf in ["refs", "name"] {
        if let Some(s) = q.get(leaf).and_then(Value::as_str) {
            out.push(s.to_owned());
        }
    }
    if let Some(l) = q.get("page") {
        query_args(l, out);
    }
    for op in ["and", "or"] {
        if let Some(a) = q.get(op).and_then(Value::as_array) {
            for x in a {
                query_args(x, out);
            }
        }
    }
    if let Some(n) = q.get("not") {
        query_args(n, out);
    }
}

fn refs_args(q: &Value, out: &mut Vec<String>) {
    if let Some(s) = q.get("refs").and_then(Value::as_str) {
        out.push(s.to_owned());
    }
    if let Some(l) = q.get("page") {
        refs_args(l, out);
    }
    for op in ["and", "or"] {
        if let Some(a) = q.get(op).and_then(Value::as_array) {
            for x in a {
                refs_args(x, out);
            }
        }
    }
    if let Some(n) = q.get("not") {
        refs_args(n, out);
    }
}

/// A sentinel Favorites name for graphs without `:tine/favorites-page`
/// (Tine then excludes no Favorites page; the model always excludes one key).
pub const NO_FAVORITES: &str = "\u{1}no favorites page configured";

/// Whether a fixture file is a sync tool's conflict copy.
pub fn is_sync_copy(rel: &str) -> bool {
    Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| tine_core::model::sync_conflict_base(stem).is_some())
}

/// The bridge's parser/configuration input, independent of inventory owners.
pub fn model_input(spec: &GraphSpec, aliases: AliasFn, map_strings: &[String]) -> Value {
    let journal = spec.journal_format();
    let config = spec.config();
    let mut input = json!({
        // A sync-conflict copy is not graph text in Tine (no page, no
        // references; `sync_conflict_base`), and the model has no such file:
        // it is left out of the model's graph (census Q2 fixtures).
        "files": spec.files.iter().filter(|(rel, _)| !is_sync_copy(rel)).map(|(rel, text)| file_json(rel, text, aliases, spec.name_format(), &config)).collect::<Vec<_>>(),
        "navigate": spec.navigate,
        "favoritesPage": spec.favorites_page.clone().unwrap_or_else(|| NO_FAVORITES.to_owned()),
        "queries": spec.queries.iter().map(|q| json!({"anchor": q.anchor, "q": q.q})).collect::<Vec<_>>(),
        "renames": spec.renames.iter().map(|(o, n)| json!({"old": o, "new": n})).collect::<Vec<_>>(),
        // Round-9 bridge: each rename is also asked as a merge (the model's
        // decided merge, M-Q5..7); the rename harness compares whichever of
        // the two Tine performed. Older bridges ignore the field.
        "merges": spec.renames.iter().map(|(o, n)| json!({"old": o, "new": n})).collect::<Vec<_>>(),
        "mapStrings": map_strings,
    });
    let mut extra = spec.navigate.clone();
    for q in &spec.queries {
        query_args(&q.q, &mut extra);
    }
    for (o, n) in &spec.renames {
        extra.push(o.clone());
        extra.push(n.clone());
    }
    extra.extend(map_strings.iter().cloned());
    if let Some(f) = &spec.favorites_page {
        extra.push(f.clone());
    }
    let keyed = spellings(&input, &extra);
    input["keys"] = json!(keyed
        .iter()
        .map(|s| (s.clone(), fold(&journal, s)))
        .collect::<BTreeMap<_, _>>());
    input["dateKeys"] = json!(keyed
        .iter()
        .filter(|s| journal.parse(s.trim()).is_some())
        .map(|s| fold(&journal, s))
        .collect::<BTreeSet<_>>());
    // Round 10 (S15): the model's `urlHasOrigin` is Tine's URL-origin test
    // (OG `url?`), given per spelling.
    input["urls"] = json!(keyed
        .iter()
        .filter(|s| tine_core::refs::url_has_origin(s))
        .collect::<Vec<_>>());
    input
}

/// Run the Lean bridge on one input.
pub fn run_bridge(bridge: &Path, input: &Value) -> Result<Value, String> {
    let out = Command::new(bridge)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())?;
            child.wait_with_output()
        })
        .map_err(|e| format!("cm-bridge did not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cm-bridge failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cm-bridge output: {e}"))
}

/// Round 13 keeps literal claims separate from the configured date target.
/// All claimant inputs are parser/config answers; no chosen owner is supplied.
pub fn model_input_r13(spec: &GraphSpec) -> Value {
    let mut input = model_input(spec, alias_spellings, &[]);
    let journal = spec.journal_format();
    let config = spec.config();
    input["favorites"] = json!(config.favorites);
    input["queries"] = json!(spec
        .queries
        .iter()
        .map(|q| {
            let dialect = if q.dialect == "tql" {
                QueryDialect::Tql
            } else {
                QueryDialect::Og
            };
            let (parsed, _) = parse_query_text(&q.tine, dialect, JournalDate::from_days(0));
            json!({"anchor": q.anchor, "q": q.q,
            "showHeaders": parsed.anchor == tine_core::query::ir::Anchor::Block,
            "removeBlockChildren": parsed.removes_block_children()})
        })
        .collect::<Vec<_>>());
    for file in input["files"].as_array_mut().unwrap() {
        let rel = file["path"].as_str().unwrap().to_owned();
        let raw = Path::new(&rel).file_stem().unwrap().to_str().unwrap();
        file["rawStem"] = json!(raw);
    }
    let mut extra = spec.navigate.clone();
    extra.extend(config.favorites.iter().cloned());
    for q in &spec.queries {
        query_args(&q.q, &mut extra);
    }
    for (old, new) in &spec.renames {
        extra.extend([old.clone(), new.clone()]);
    }
    for file in input["files"].as_array().unwrap() {
        for b in file["blocks"].as_array().unwrap() {
            for q in b["stored"].as_array().unwrap() {
                query_args(q, &mut extra);
            }
        }
    }
    let before = spellings(&input, &extra);
    // Finite closure of every candidate substitution, including alternate
    // real members. Key/date/codec tables are functions on spellings only.
    let destinations: BTreeSet<String> = spec
        .renames
        .iter()
        .map(|(_, new)| new.clone())
        .chain(input["files"].as_array().unwrap().iter().map(|f| {
            f["title"]
                .as_str()
                .filter(|_| f["dir"] == "pages")
                .unwrap_or_else(|| f["stem"].as_str().unwrap())
                .to_owned()
        }))
        .collect();
    let mut all = before.clone();
    let mut output_paths = Vec::new();
    for (old, _) in &spec.renames {
        for new in &destinations {
            let map = std::collections::HashMap::from([(page_key(old), new.clone())]);
            for name in &before {
                if let Some(mapped) = tine_core::refs::renamed_page_name(name, &map) {
                    all.insert(mapped.into_owned());
                }
            }
            for file in input["files"].as_array().unwrap() {
                let name = file["title"]
                    .as_str()
                    .filter(|_| file["dir"] == "pages")
                    .unwrap_or_else(|| file["stem"].as_str().unwrap());
                let mapped = tine_core::refs::renamed_page_name(name, &map)
                    .map(|s| s.into_owned())
                    .unwrap_or_else(|| name.to_owned());
                let rel = file["path"].as_str().unwrap();
                let ext = Path::new(rel).extension().unwrap().to_str().unwrap();
                output_paths.push(json!({"source": rel, "name": mapped,
                    "path": format!("pages/{}.{ext}", encode_page_name(&mapped, spec.name_format()))}));
            }
        }
    }
    // Namespace parents of output names and canonical date titles are keyed too.
    let more: Vec<_> = all
        .iter()
        .flat_map(|s| {
            let mut names = tine_core::refs::namespace_refs(s)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if let Some(day) = journal.parse(s) {
                names.extend(journal.spellings(day));
                names.push(journal.title(day));
            }
            names
        })
        .collect();
    all.extend(more);
    input["literalKeys"] = json!(all
        .iter()
        .map(|s| (s, page_key(s)))
        .collect::<BTreeMap<_, _>>());
    input["keys"] = input["literalKeys"].clone();
    input["canonicalDates"] = json!(all
        .iter()
        .filter_map(|s| journal
            .parse(s)
            .map(|d| (page_key(s), page_key(&journal.title(d)))))
        .collect::<BTreeMap<_, _>>());
    input["claimantConfig"] = json!({
        "kindPref": "page", "order": "unsigned-utf8-relative-path",
        "fileNameFormat": spec.file_name_format.as_deref().unwrap_or("legacy"),
        "journalFileFormat": journal.file_format(), "journalTitleFormat": journal.title_format(),
        "stems": input["files"].as_array().unwrap().iter().map(|f| json!({
            "path": f["path"], "raw": f["rawStem"], "decoded": f["stem"]
        })).collect::<Vec<_>>()
    });
    input["outputPaths"] = json!(output_paths);
    input["urls"] = json!(all
        .iter()
        .filter(|s| tine_core::refs::url_has_origin(s))
        .collect::<Vec<_>>());
    for file in input["files"].as_array_mut().unwrap() {
        native::decorate(file, &config);
    }
    input
}

/// One difference: a stable class (no names; used to deduplicate random
/// graphs) and the concrete detail.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diff {
    pub class: String,
    pub detail: String,
    /// Structured witnesses used only by r13 attribution; historical rows stay unchanged.
    pub keys: BTreeSet<String>,
    pub blocks: Vec<BlockId>,
}

impl Diff {
    pub fn new(class: impl Into<String>, detail: impl Into<String>) -> Diff {
        Diff {
            class: class.into(),
            detail: detail.into(),
            keys: BTreeSet::new(),
            blocks: Vec::new(),
        }
    }
    pub fn at_key(mut self, name: &str) -> Self {
        self.keys.insert(page_key(name));
        self
    }
    pub fn at_block(mut self, block: &BlockId) -> Self {
        self.blocks.push(block.clone());
        self
    }
    pub fn is_harness(&self) -> bool {
        self.class.starts_with("harness")
    }
}

pub type Page = (String, String);
pub type BlockId = (String, usize);

fn page_pair(v: &Value) -> Page {
    (
        v[0].as_str().unwrap().to_owned(),
        v[1].as_str().unwrap().to_owned(),
    )
}

fn block_pair(v: &Value) -> BlockId {
    (
        v[0].as_str().unwrap().to_owned(),
        v[1].as_u64().unwrap() as usize,
    )
}

/// One file of the input, as the comparisons read it.
pub struct FileInfo {
    pub path: String,
    pub journal: bool,
    pub name: String,
    pub aliases: Vec<String>,
    pub pre: bool,
    /// Harness-only roles of the initial positions; the bridge ignores this
    /// metadata and evaluates every supplied block normally.
    pub pre_block_kinds: Vec<String>,
    /// Per block: level and occurrences (kind, spelling).
    pub blocks: Vec<(usize, Vec<(String, String)>)>,
}

/// The input and the model's answers, indexed for the comparisons.
pub struct Ctx<'a> {
    pub view: &'a WholeGraph,
    pub journal: JournalFormat,
    pub input: &'a Value,
    pub model: &'a Value,
    pub files: BTreeMap<String, FileInfo>,
    pub pages: BTreeSet<Page>,
    pub conflicts: BTreeSet<String>,
    /// Owner file path -> its page.
    pub page_of_path: BTreeMap<String, Page>,
    pub owner_of_page: BTreeMap<Page, String>,
    /// Alias components over all keys in play, from the model's edges.
    pub class: BTreeMap<String, BTreeSet<String>>,
    pub date_keys: BTreeSet<String>,
    pub favorites_key: String,
    /// Tine runtime block id -> (path, outline index).
    pub tine_blocks: BTreeMap<String, BlockId>,
    /// Model references: block -> pages it references.
    pub refs: BTreeMap<BlockId, BTreeSet<Page>>,
}

impl<'a> Ctx<'a> {
    pub fn new(
        view: &'a WholeGraph,
        journal: JournalFormat,
        input: &'a Value,
        model: &'a Value,
    ) -> Ctx<'a> {
        let mut files = BTreeMap::new();
        for f in input["files"].as_array().unwrap() {
            let path = f["path"].as_str().unwrap().to_owned();
            let name = f["title"]
                .as_str()
                .filter(|_| f["dir"] == "pages")
                .unwrap_or_else(|| f["stem"].as_str().unwrap())
                .to_owned();
            let blocks = f["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| {
                    (
                        b["level"].as_u64().unwrap() as usize,
                        b["occ"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|o| {
                                (
                                    o[0].as_str().unwrap().to_owned(),
                                    o[1].as_str().unwrap().to_owned(),
                                )
                            })
                            .collect(),
                    )
                })
                .collect();
            files.insert(
                path.clone(),
                FileInfo {
                    path,
                    journal: f["dir"] == "journals",
                    name,
                    aliases: strings(&f["aliases"]),
                    pre: f["preBlock"].as_bool().unwrap_or(false),
                    pre_block_kinds: strings(&f["preBlockKinds"]),
                    blocks,
                },
            );
        }
        let pages: BTreeSet<Page> = model["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(page_pair)
            .collect();
        let conflicts = model["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let mut page_of_path = BTreeMap::new();
        let mut owner_of_page = BTreeMap::new();
        for o in model["owners"].as_array().unwrap() {
            if let Some(path) = o["path"].as_str() {
                let page = page_pair(&o["page"]);
                page_of_path.insert(path.to_owned(), page.clone());
                owner_of_page.insert(page, path.to_owned());
            }
        }
        // Union-find over the model's alias edges.
        let mut class: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for e in model["aliasEdges"].as_array().unwrap() {
            let a = e[0].as_str().unwrap().to_owned();
            let b = e[1].as_str().unwrap().to_owned();
            let mut merged: BTreeSet<String> = class
                .get(&a)
                .cloned()
                .unwrap_or_else(|| BTreeSet::from([a.clone()]));
            merged.extend(
                class
                    .get(&b)
                    .cloned()
                    .unwrap_or_else(|| BTreeSet::from([b.clone()])),
            );
            for k in merged.clone() {
                class.insert(k, merged.clone());
            }
        }
        let date_keys = input["dateKeys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let fav = input["favoritesPage"].as_str().unwrap();
        let favorites_key = input["keys"][fav]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| fav.to_owned());
        let refs = model["references"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    block_pair(&r["block"]),
                    r["pages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(page_pair)
                        .collect(),
                )
            })
            .collect();
        Ctx {
            view,
            journal,
            input,
            model,
            files,
            pages,
            conflicts,
            page_of_path,
            owner_of_page,
            class,
            date_keys,
            favorites_key,
            tine_blocks: tine_block_ids(view),
            refs,
        }
    }

    pub fn key(&self, s: &str) -> String {
        if self.model["round"] == "r13" {
            page_key(s)
        } else {
            fold(&self.journal, s)
        }
    }

    pub fn class_of(&self, k: &str) -> BTreeSet<String> {
        self.class
            .get(k)
            .cloned()
            .unwrap_or_else(|| BTreeSet::from([k.to_owned()]))
    }

    fn claimed_keys(&self) -> BTreeSet<String> {
        self.files.values().map(|f| self.key(&f.name)).collect()
    }

    fn occ(&self, b: &BlockId) -> &[(String, String)] {
        self.files
            .get(&b.0)
            .and_then(|f| f.blocks.get(b.1))
            .map(|x| x.1.as_slice())
            .unwrap_or(&[])
    }

    /// How a block's occurrences reach key `t`: direct, a namespace prefix,
    /// through the alias class, or not at all; plus the occurrence kinds used.
    fn via(&self, occ: &[(String, String)], t: &str) -> String {
        let cls = self.class_of(t);
        let mut ways = BTreeSet::new();
        let mut kinds = BTreeSet::new();
        for (kind, s) in occ {
            // `occClosure` (round 10): a non-namespace spelling (`./x`, a
            // URL) references only itself.
            for u in tine_core::refs::namespace_refs(s) {
                let ku = self.key(u);
                let ns = u != s.as_str();
                let way = if ku == t {
                    if ns {
                        "ns-prefix"
                    } else {
                        "direct"
                    }
                } else if cls.contains(&ku) {
                    if ns {
                        "alias+ns-prefix"
                    } else {
                        "alias"
                    }
                } else {
                    continue;
                };
                ways.insert(way);
                kinds.insert(kind.as_str());
            }
        }
        if ways.is_empty() {
            "none".into()
        } else {
            format!(
                "{} by {}",
                ways.into_iter().collect::<Vec<_>>().join("|"),
                kinds.into_iter().collect::<Vec<_>>().join("|")
            )
        }
    }

    fn page_feature(&self, p: &Page) -> String {
        let real = if self.owner_of_page.contains_key(p) {
            "real"
        } else {
            "ref-only"
        };
        format!("{} {}", p.0, real)
    }

    /// Source features of a block relative to target page `t`.
    fn source_feature(&self, b: &BlockId, t: &Page) -> String {
        let Some(f) = self.files.get(&b.0) else {
            return "src=unknown-file".into();
        };
        let pos = if f.pre && b.1 == 0 {
            "pre-block"
        } else if f.blocks.get(b.1).is_some_and(|x| x.0 > 1) {
            "child"
        } else {
            "root"
        };
        let src = match self.page_of_path.get(&b.0) {
            None => "non-owner file".to_owned(),
            Some(s) if s == t => "own page".to_owned(),
            Some(s) if self.class_of(&t.1).contains(&s.1) => format!("{} in target's class", s.0),
            Some(s) if s.1 == self.favorites_key => "favorites page".to_owned(),
            Some(s) => s.0.clone(),
        };
        format!("{pos} of {src}")
    }
}

/// Tine runtime block ids of every parsed file, as (path, outline index):
/// the header and prose pre-blocks precede the outline in pre-order, exactly
/// as in `file_json`. Their synthetic identities are the existing reference
/// rows' identities; prose-only pages have no phantom header position.
fn tine_block_ids(view: &WholeGraph) -> BTreeMap<String, BlockId> {
    fn walk(blocks: &[DocBlock], path: &str, i: &mut usize, out: &mut BTreeMap<String, BlockId>) {
        for b in blocks {
            out.insert(b.uuid.clone(), (path.to_owned(), *i));
            *i += 1;
            walk(&b.children, path, i, out);
        }
    }
    let mut out = BTreeMap::new();
    for page in view.corpus(None, true).pages {
        let path = page.id.to_string();
        let mut i = 0;
        if let Some(pre) = &page.document.pre_block {
            use tine_core::page_properties::{pre_block_projections, PreBlockKind};
            for (kind, _) in pre_block_projections(pre, file_format(&path) == Format::Org) {
                let prefix = match kind {
                    PreBlockKind::Properties => "page-property",
                    PreBlockKind::Preamble => "page-preamble",
                };
                out.entry(format!("{prefix}:{:?}:{}", page.kind, page_key(&page.name)))
                    .or_insert((path.clone(), i));
                i += 1;
            }
        }
        walk(&page.document.roots, &path, &mut i, &mut out);
    }
    out
}

/// Tine's page for a resolution, as the model's (kind, key).
fn tine_page(
    view: &WholeGraph,
    journal: &JournalFormat,
    r: &Resolved,
    name: &str,
    journal_kind: bool,
) -> Page {
    let entry_of = |id: &PageId| {
        let inv = view.inventory(InventoryScope::File(id));
        let e = inv.0.first().expect("a claimant has an inventory entry");
        (
            if e.is_journal { "journal" } else { "page" }.to_owned(),
            fold(journal, &e.name),
        )
    };
    match r {
        Resolved::Existing { id, .. } => entry_of(id),
        Resolved::Alias { owners } => entry_of(&owners[0]),
        Resolved::Absent { .. } => (
            if journal_kind { "journal" } else { "page" }.to_owned(),
            fold(journal, name),
        ),
    }
}

/// What a link `[[name]]` opens: the one answer (`WholeGraph::link_target`),
/// then the page `get_page` opens for that destination (`resolve`).
fn navigate(view: &WholeGraph, journal: &JournalFormat, name: &str) -> Page {
    let link = view.link_target(name);
    tine_page(
        view,
        journal,
        &view.resolve(&link.name, link.is_journal),
        &link.name,
        link.is_journal,
    )
}

/// The wire form of a resolution (`src-tauri` `ResolvedWire`, the
/// frontend's `ResolvedPage`).
fn resolved_wire(r: &Resolved) -> Value {
    match r {
        Resolved::Existing { id, others } => json!({
            "kind": "existing",
            "id": id.as_str(),
            "others": others.iter().map(|o| o.as_str()).collect::<Vec<_>>(),
        }),
        Resolved::Alias { owners } => json!({
            "kind": "alias",
            "owners": owners.iter().map(|o| o.as_str()).collect::<Vec<_>>(),
        }),
        Resolved::Absent { id } => json!({"kind": "absent", "id": id.as_str()}),
    }
}

/// Harness gap H3 closed for fixtures: what the frontend's link decision
/// needs to run in vitest (`src/cmConformanceNavigate.test.ts`), with the
/// answers it is compared to.
/// - `journalTitleFormat` / `journalFileFormat`: what GraphMeta hands
///   `setJournalFormats`;
/// - `inventory`: `page_inventory`'s entries, built as `src-tauri`'s
///   `entry_wire` builds them (with `link`);
/// - `spellings`: per spelling the model answers for, the native parse's day,
///   the model's page and the backend's own answer (`navigate`);
/// - `open`: per name the click path can route to (every spelling and every
///   inventory name), the page `get_page(name, kind)` opens for each kind
///   (`WholeGraph::resolve`, first alias owner, as `Store::page_named`);
/// - `counts`, `pages`, `backlinks`, `keyOf` (batch 5): All Pages' counts
///   read, the model's pages with their Linked References size, each page's
///   panel answer, and the model key of every inventory name.
pub fn frontend_case(
    ctx: &Ctx,
    journal_title_format: Option<&str>,
    journal_file_format: Option<&str>,
) -> Value {
    let view = ctx.view;
    let journal = &ctx.journal;
    let inventory = view.inventory(InventoryScope::All);
    let entries: Vec<Value> = inventory
        .0
        .iter()
        .map(|e| {
            let mut wire = json!({
                "key": page_key(&e.name),
                "name": e.name,
                "is_journal": e.is_journal,
                "day": e.day.map(|d| d.0),
                "target": resolved_wire(&e.target),
            });
            if let Some(link) = &e.link {
                wire["link"] = json!({
                    "name": link.name,
                    "kind": if link.is_journal { "journal" } else { "page" },
                });
            }
            if e.namespace_parent {
                wire["namespace_parent"] = json!(true);
            }
            if let Some(parent) = &e.parent {
                wire["parent"] = json!({ "name": parent.name, "kind": if parent.is_journal { "journal" } else { "page" } });
            }
            if e.empty_alias {
                wire["empty_alias"] = json!(true);
            }
            if matches!(e.target, Resolved::Alias { .. }) {
                wire["namespace_excluded"] = json!(true);
            }
            // As `inventory_wire::entry_wire` sets it (round 10, S15).
            if e.name.contains('/') && !tine_core::refs::is_namespace_name(&e.name) {
                wire["not_namespace"] = json!(true);
            }
            wire
        })
        .collect();
    let mut names: BTreeSet<String> = inventory
        .0
        .iter()
        .flat_map(|e| std::iter::once(e.name.clone()).chain(e.link.iter().map(|l| l.name.clone())))
        .collect();
    let mut spellings = BTreeMap::new();
    for (s, page) in ctx.model["navigate"].as_object().unwrap() {
        names.insert(s.clone());
        names.insert(view.link_target(s).name);
        let day = journal
            .parse(s.trim())
            .map(|d| json!([d.year, d.month, d.day]));
        let backend = navigate(view, journal, s);
        spellings.insert(
            s.clone(),
            json!({"day": day, "model": page, "backend": [backend.0, backend.1]}),
        );
    }
    let open: BTreeMap<String, Value> = names
        .iter()
        .map(|n| {
            let j = tine_page(view, journal, &view.resolve(n, true), n, true);
            let p = tine_page(view, journal, &view.resolve(n, false), n, false);
            (
                n.clone(),
                json!({"journal": [j.0, j.1], "page": [p.0, p.1]}),
            )
        })
        .collect();
    // CM step 2 batch 5 (counts and the page set Tine shows; read by
    // `src/cmConformanceCounts.test.ts`, which mounts All Pages and the Linked
    // References panel on these answers).
    let (files, references) = view.page_backlink_counts();
    let counts = json!([files, references]);
    // The views' parent of each model page (`Ref10.nsParent`, round 10).
    let parents: BTreeMap<Page, Value> = ctx.model["pageParents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            let page = &row["page"];
            let page = (
                page[0].as_str().unwrap().to_owned(),
                page[1].as_str().unwrap().to_owned(),
            );
            (page, row["parent"].clone())
        })
        .collect();
    let mut pages = Vec::new();
    let mut backlinks = BTreeMap::new();
    for p in &ctx.pages {
        if ctx.conflicts.contains(&p.1) {
            continue;
        }
        let name = tine_name(ctx, p);
        let groups: Vec<Value> = match view.backlinks(&name) {
            Ok(groups) => groups.iter().map(ref_group_wire).collect(),
            Err(e) => vec![json!({"harness": format!("backlinks({name:?}): {e:?}")})],
        };
        backlinks.insert(name.clone(), json!(groups));
        pages.push(json!({
            "kind": p.0,
            "key": p.1,
            "name": name,
            "owner": ctx.owner_of_page.get(p),
            "linkedRefs": model_panel(ctx, p).len(),
            "parent": parents.get(p).cloned().unwrap_or(Value::Null),
        }));
    }
    let key_of: BTreeMap<String, String> = inventory
        .0
        .iter()
        .map(|e| (e.name.clone(), fold(journal, &e.name)))
        .collect();
    json!({
        "treeNodes": ctx.model["treeNodes"],
        "treeParent": ctx.model["treeParent"],
        "tree": ctx.model["tree"],
        "namespaceNodes": ctx.model["namespaceNodes"],
        "namespaceTree": ctx.model["namespaceTree"],
        "journalTitleFormat": journal_title_format,
        "journalFileFormat": journal_file_format,
        "conflicts": ctx.conflicts,
        "inventory": entries,
        "spellings": spellings,
        "open": open,
        "counts": counts,
        "pages": pages,
        "backlinks": backlinks,
        "keyOf": key_of,
    })
}

/// A Linked References group as `get_backlinks` puts it on the wire, with
/// only the fields the panel's count and its filter chips read.
fn ref_group_wire(g: &RefGroup) -> Value {
    json!({
        "page": g.page,
        "kind": match g.kind {
            tine_core::model::PageKind::Journal => "journal",
            tine_core::model::PageKind::Page => "page",
        },
        "blocks": g.blocks.iter().map(|b| json!({
            "id": b.id,
            "raw": b.raw,
            "children": [],
            "collapsed": false,
            "tags": [],
        })).collect::<Vec<_>>(),
    })
}

/// Rule 1 and 3 (the manager's first slice): pages, conflicts, owners and
/// navigation.
pub fn compare_names(ctx: &Ctx) -> Vec<Diff> {
    let mut diffs = Vec::new();
    let view = ctx.view;
    let journal = &ctx.journal;
    let inventory = view.inventory(InventoryScope::All);
    let mut tine_pages = BTreeSet::new();
    let mut tine_conflicts = BTreeSet::new();
    let mut tine_owner = BTreeMap::new();
    for entry in &inventory.0 {
        let kind = if entry.is_journal { "journal" } else { "page" }.to_owned();
        let key = fold(journal, &entry.name);
        match &entry.target {
            Resolved::Existing { id, others } => {
                if !others.is_empty() {
                    tine_conflicts.insert(key.clone());
                }
                tine_owner.insert((kind.clone(), key.clone()), id.to_string());
                tine_pages.insert((kind, key));
            }
            Resolved::Alias { .. } => {}
            Resolved::Absent { .. } => {
                tine_pages.insert((kind, key));
            }
        }
    }
    let claimed = ctx.claimed_keys();
    let origin = |k: &str| {
        let mut o = Vec::new();
        if claimed.contains(k) {
            o.push("claimed");
        }
        let mut occ_direct = false;
        let mut occ_prefix = false;
        for f in ctx.files.values() {
            for (_, occ) in &f.blocks {
                for (_, s) in occ {
                    for u in tine_core::refs::namespace_refs(s) {
                        if ctx.key(u) == k {
                            if u == s.as_str() {
                                occ_direct = true
                            } else {
                                occ_prefix = true
                            }
                        }
                    }
                }
            }
        }
        if occ_direct {
            o.push("occurrence");
        }
        if occ_prefix {
            o.push("occurrence-prefix");
        }
        if ctx.files.values().any(|f| {
            tine_core::refs::namespace_refs(&f.name).any(|u| u != f.name && ctx.key(u) == k)
        }) {
            o.push("file-name-prefix");
        }
        if ctx.class.contains_key(k) {
            o.push("alias-edge");
        }
        if ctx.date_keys.contains(k) {
            o.push("date");
        }
        o.join(",")
    };
    for p in ctx.pages.difference(&tine_pages) {
        diffs.push(Diff::new(
            format!("pages: model-only {} ({})", p.0, origin(&p.1)),
            format!("pages: model has {p:?}, Tine does not"),
        ));
    }
    for p in tine_pages.difference(&ctx.pages) {
        diffs.push(Diff::new(
            format!("pages: Tine-only {} ({})", p.0, origin(&p.1)),
            format!("pages: Tine has {p:?}, model does not"),
        ));
    }
    if ctx.conflicts != tine_conflicts {
        let kinds = |k: &String| {
            let mut ks: Vec<&str> = ctx
                .files
                .values()
                .filter(|f| ctx.key(&f.name) == *k)
                .map(|f| if f.journal { "journal" } else { "page" })
                .collect();
            ks.sort();
            ks.join("+")
        };
        for k in ctx.conflicts.difference(&tine_conflicts) {
            diffs.push(Diff::new(
                format!("conflicts: model-only ({})", kinds(k)),
                format!("conflicts: model {k:?} conflicted, Tine not"),
            ));
        }
        for k in tine_conflicts.difference(&ctx.conflicts) {
            diffs.push(Diff::new(
                format!("conflicts: Tine-only ({})", kinds(k)),
                format!("conflicts: Tine {k:?} conflicted, model not"),
            ));
        }
    }
    for (page, model_path) in &ctx.owner_of_page {
        if ctx.conflicts.contains(&page.1) {
            continue;
        }
        match tine_owner.get(page) {
            Some(path) if path == model_path => {}
            other => diffs.push(Diff::new(
                "owners: differ",
                format!("owner of {page:?}: model {model_path}, Tine {other:?}"),
            )),
        }
    }
    for (s, page) in ctx.model["navigate"].as_object().unwrap() {
        let model_page = page_pair(page);
        if ctx.conflicts.contains(&model_page.1) {
            continue;
        }
        let tine = navigate(view, journal, s);
        if tine != model_page {
            diffs.push(Diff::new(
                format!(
                    "navigate: model {} vs Tine {} ({})",
                    model_page.0,
                    tine.0,
                    origin(&ctx.key(s))
                ),
                format!("navigate [[{s}]]: model {model_page:?}, Tine {tine:?}"),
            ));
        }
    }
    diffs
}

/// The model's Linked References of `p`: blocks referencing p, minus p's own
/// blocks and the Favorites page's (Link exclusions, §6.2).
fn model_panel(ctx: &Ctx, p: &Page) -> BTreeSet<BlockId> {
    ctx.refs
        .iter()
        .filter(|(b, pages)| {
            pages.contains(p)
                && ctx
                    .page_of_path
                    .get(&b.0)
                    .is_none_or(|s| s != p && s.1 != ctx.favorites_key)
        })
        .map(|(b, _)| b.clone())
        .collect()
}

/// The name Tine's name-keyed APIs take for model page p.
fn tine_name(ctx: &Ctx, p: &Page) -> String {
    match ctx.owner_of_page.get(p) {
        Some(path) => ctx
            .view
            .inventory(InventoryScope::File(&PageId::from(path.as_str())))
            .0
            .first()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| p.1.clone()),
        None => p.1.clone(),
    }
}

fn group_blocks(
    ctx: &Ctx,
    groups: &[RefGroup],
    diffs: &mut Vec<Diff>,
    what: &str,
) -> BTreeSet<BlockId> {
    let mut out = BTreeSet::new();
    for g in groups {
        for b in &g.blocks {
            match ctx.tine_blocks.get(&b.id) {
                Some(id) => {
                    out.insert(id.clone());
                }
                None => diffs.push(Diff::new(
                    "harness: unmapped Tine block id",
                    format!(
                        "{what}: Tine row {} ({:?}) has no outline position",
                        b.id, b.raw
                    ),
                )),
            }
        }
    }
    out
}

/// Rule 2 and 5 (Q9 panel): each model page's Linked References against
/// `WholeGraph::backlinks`.
pub fn compare_backlinks(ctx: &Ctx) -> Vec<Diff> {
    let mut diffs = Vec::new();
    for p in &ctx.pages {
        let model = model_panel(ctx, p);
        let name = tine_name(ctx, p);
        let tine = match ctx.view.backlinks(&name) {
            Ok(groups) => group_blocks(ctx, &groups, &mut diffs, "backlinks"),
            Err(e) => {
                diffs.push(Diff::new(
                    "harness: backlinks error",
                    format!("backlinks({name:?}): {e:?}"),
                ));
                continue;
            }
        };
        for b in model.difference(&tine) {
            diffs.push(Diff::new(
                format!(
                    "linked refs: model-only, target {}, {}, source {}",
                    ctx.page_feature(p),
                    ctx.via(ctx.occ(b), &p.1),
                    ctx.source_feature(b, p)
                ),
                format!("Linked References of {p:?}: model has block {b:?}, Tine does not"),
            ));
        }
        for b in tine.difference(&model) {
            diffs.push(Diff::new(
                format!(
                    "linked refs: Tine-only, target {}, {}, source {}",
                    ctx.page_feature(p),
                    ctx.via(ctx.occ(b), &p.1),
                    ctx.source_feature(b, p)
                ),
                format!("Linked References of {p:?}: Tine has block {b:?}, model does not"),
            ));
        }
    }
    diffs
}

/// Q9 counts: `page_backlink_counts` per owner file against the size of the
/// model's Linked References (rule 2: counts see the whole group). Tine's
/// count is, by construction, the size of its own unfiltered Linked
/// References panel of that page (D-20261102-CM4 CM-COUNT); a difference is
/// explained against that panel, so a count that follows a panel difference
/// is named as such and a count that leaves its own panel is a defect.
pub fn compare_counts(ctx: &Ctx) -> Vec<Diff> {
    let mut diffs = Vec::new();
    let counts: BTreeMap<String, usize> = ctx.view.page_backlink_counts().0.into_iter().collect();
    // Every file Tine holds (also built from tine-graph-features, so only
    // public API).
    let held: BTreeSet<String> = ctx
        .view
        .page_modified()
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    for (p, path) in &ctx.owner_of_page {
        let model = model_panel(ctx, p).len();
        let Some(&tine) = counts.get(path) else {
            let twin = held.contains(path);
            diffs.push(if twin {
                Diff::new(
                    "counts: the model's owner is a Tine non-owner twin (O1 order; All pages shows a dash, A13)",
                    format!("counts of {p:?} ({path}): model {model}, Tine none (non-owner twin)"),
                )
            } else {
                Diff::new(
                    "counts: no Tine count for an owner file",
                    format!("counts of {p:?} ({path}): model {model}, Tine none"),
                )
            });
            continue;
        };
        if tine == model {
            continue;
        }
        let name = tine_name(ctx, p);
        let panel = ctx
            .view
            .backlinks(&name)
            .map(|groups| groups.iter().map(|g| g.blocks.len()).sum::<usize>());
        let reason = match panel {
            Ok(panel) if panel == tine => {
                "as Tine's Linked References, which differ from the model's"
            }
            Ok(_) => "not the size of Tine's own Linked References",
            Err(_) => "Tine's Linked References failed",
        };
        diffs.push(Diff::new(
            format!("counts: {reason}, target {}", ctx.page_feature(p)),
            format!("backlink count of {p:?} ({path}): model {model}, Tine {tine} ({reason})"),
        ));
    }
    diffs
}

/// Rule 2 (Q7): each model page's alias class against
/// `inventory(AliasGroup)`.
pub fn compare_classes(ctx: &Ctx) -> Vec<Diff> {
    let mut diffs = Vec::new();
    let mut seen = BTreeSet::new();
    for p in &ctx.pages {
        if !seen.insert(p.1.clone()) {
            continue;
        }
        let model: BTreeSet<String> = ctx.model["classes"][&p.1]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
            .unwrap_or_else(|| BTreeSet::from([p.1.clone()]));
        let name = tine_name(ctx, p);
        let mut tine: BTreeSet<String> = ctx
            .view
            .inventory(InventoryScope::AliasGroup(&name))
            .0
            .iter()
            .map(|e| ctx.key(&e.name))
            .collect();
        tine.insert(p.1.clone());
        if model != tine {
            let journal_declarer = ctx
                .files
                .values()
                .any(|f| f.journal && !f.aliases.is_empty() && model.contains(&ctx.key(&f.name)));
            let shape = if model.is_superset(&tine) {
                "model larger"
            } else if tine.is_superset(&model) {
                "Tine larger"
            } else {
                "both differ"
            };
            diffs.push(Diff::new(
                format!(
                    "alias group: {shape}, target {}{}",
                    ctx.page_feature(p),
                    if journal_declarer {
                        ", journal declares"
                    } else {
                        ""
                    }
                ),
                format!("alias group of {p:?}: model {model:?}, Tine {tine:?}"),
            ));
        }
    }
    diffs
}

/// Q9 link table: model links (S, T) against Tine's `LinkGraph` pairs, both
/// as page keys (the table is keyed by key, not kind).
pub fn compare_links(ctx: &Ctx, tine: &BTreeSet<(String, String)>) -> Vec<Diff> {
    let mut diffs = Vec::new();
    let model: BTreeSet<(String, String)> = ctx.model["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (page_pair(&l[0]).1, page_pair(&l[1]).1))
        .collect();
    let kind_of = |k: &str| {
        ctx.pages
            .iter()
            .find(|p| p.1 == k)
            .map(|p| ctx.page_feature(p))
            .unwrap_or_else(|| "not a model page".into())
    };
    let via = |s: &str, t: &str| {
        let mut ways = BTreeSet::new();
        for (path, page) in &ctx.page_of_path {
            if page.1 == s {
                if let Some(f) = ctx.files.get(path) {
                    for (_, occ) in &f.blocks {
                        let w = ctx.via(occ, t);
                        if w != "none" {
                            ways.insert(w);
                        }
                    }
                }
            }
        }
        if ways.is_empty() {
            "none".to_owned()
        } else {
            ways.into_iter().collect::<Vec<_>>().join("; ")
        }
    };
    for (s, t) in model.difference(tine) {
        diffs.push(Diff::new(
            format!(
                "links: model-only, {} -> {}, {}{}",
                kind_of(s),
                kind_of(t),
                via(s, t),
                if ctx.class_of(t).contains(s) {
                    ", same alias group"
                } else {
                    ""
                }
            ),
            format!("link {s:?} -> {t:?}: model has it, Tine does not"),
        ));
    }
    for (s, t) in tine.difference(&model) {
        diffs.push(Diff::new(
            format!(
                "links: Tine-only, {} -> {}, {}{}",
                kind_of(s),
                kind_of(t),
                via(s, t),
                if ctx.class_of(t).contains(s) {
                    ", same alias group"
                } else {
                    ""
                }
            ),
            format!("link {s:?} -> {t:?}: Tine has it, model does not"),
        ));
    }
    diffs
}

/// Rule 7 (Q10): each query's model denotation against `query_ir` of Tine's
/// text, as sets of nodes.
pub fn compare_queries(ctx: &Ctx, queries: &[QuerySpec]) -> Vec<Diff> {
    let mut diffs = Vec::new();
    let model_all = ctx.model["queries"].as_array().unwrap();
    for (i, q) in queries.iter().enumerate() {
        // The class names the anchor and whether a negation is involved, not
        // the whole shape: the causes are per leaf (code-map observations 7
        // and 13), and shapes would multiply the classes.
        let raw_shape = query_shape(&q.q);
        let shape = format!(
            "@{}{}",
            q.anchor,
            if raw_shape.contains("not(") {
                " with not"
            } else {
                ""
            }
        );
        let mut model_blocks = BTreeSet::new();
        let mut model_pages = BTreeSet::new();
        for n in model_all[i].as_array().unwrap() {
            if let Some(b) = n.get("block") {
                model_blocks.insert(block_pair(b));
            } else {
                model_pages.insert(page_pair(&n["page"]));
            }
        }
        let dialect = if q.dialect == "tql" {
            QueryDialect::Tql
        } else {
            QueryDialect::Og
        };
        let (query, view) = parse_query_text(&q.tine, dialect, JournalDate::today());
        if query.is_invalid() {
            diffs.push(Diff::new(
                format!("harness: Tine rejects query text ({shape})"),
                format!("query {:?} ({raw_shape}): {:?}", q.tine, query.diagnostics),
            ));
            continue;
        }
        let result = match ctx.view.query_ir(
            IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::none(),
            },
            &Cancel::default(),
        ) {
            Ok(Some(IrAnswer::Result(result))) => result,
            other => {
                diffs.push(Diff::new(
                    "harness: query did not run",
                    format!("query {:?}: {:?}", q.tine, other.map(|a| a.is_some())),
                ));
                continue;
            }
        };
        let mut tine_blocks = BTreeSet::new();
        let mut tine_pages = BTreeSet::new();
        match &result.rows {
            QueryRows::Block { groups } => {
                tine_blocks = group_blocks(ctx, groups, &mut diffs, "query");
            }
            QueryRows::Page { pages } => {
                for row in pages {
                    let kind = match row.kind {
                        tine_core::model::PageKind::Journal => "journal",
                        tine_core::model::PageKind::Page => "page",
                    };
                    tine_pages.insert((kind.to_owned(), ctx.key(&row.name)));
                }
            }
        }
        let mut args = Vec::new();
        refs_args(&q.q, &mut args);
        let arg_keys: BTreeSet<String> = args.iter().map(|a| ctx.key(a)).collect();
        let block_feature = |b: &BlockId, in_model: &BTreeSet<BlockId>| {
            let Some(f) = ctx.files.get(&b.0) else {
                return "unknown file".to_owned();
            };
            let mut feats = Vec::new();
            if f.pre && b.1 == 0 {
                feats.push("pre-block".to_owned());
            }
            if f.pre_block_kinds
                .get(b.1)
                .is_some_and(|kind| kind == "preamble")
            {
                feats.push("preamble".to_owned());
            }
            if ctx.page_of_path.get(&b.0).is_none() {
                feats.push("non-owner file".to_owned());
            }
            // Nearest ancestor by level.
            let level = f.blocks[b.1].0;
            let mut ancestors = Vec::new();
            let mut want = level;
            for j in (0..b.1).rev() {
                if f.pre && j == 0 {
                    break;
                }
                if f.blocks[j].0 < want {
                    ancestors.push(j);
                    want = f.blocks[j].0;
                }
            }
            if ancestors
                .first()
                .is_some_and(|&j| in_model.contains(&(b.0.clone(), j)))
            {
                feats.push("parent also matches".to_owned());
            }
            let ways: BTreeSet<String> = arg_keys
                .iter()
                .map(|k| ctx.via(&f.blocks[b.1].1, k))
                .filter(|w| w != "none")
                .collect();
            if !ways.is_empty() {
                feats.push(format!(
                    "own {}",
                    ways.into_iter().collect::<Vec<_>>().join("; ")
                ));
            } else if ancestors.iter().any(|&j| {
                arg_keys
                    .iter()
                    .any(|k| ctx.via(&f.blocks[j].1, k) != "none")
            }) {
                feats.push("via ancestor".to_owned());
            } else if ctx
                .page_of_path
                .get(&b.0)
                .is_some_and(|p| arg_keys.iter().any(|k| ctx.class_of(k).contains(&p.1)))
            {
                feats.push(
                    if ctx
                        .page_of_path
                        .get(&b.0)
                        .is_some_and(|p| arg_keys.contains(&p.1))
                    {
                        "via owner page key"
                    } else {
                        "via owner page's alias class"
                    }
                    .to_owned(),
                );
            }
            feats.join(", ")
        };
        for b in model_blocks.difference(&tine_blocks) {
            diffs.push(
                Diff::new(
                    format!(
                        "query {shape}: model-only block ({})",
                        block_feature(b, &model_blocks)
                    ),
                    format!("query {:?}: model has block {b:?}, Tine does not", q.tine),
                )
                .at_block(b),
            );
        }
        for b in tine_blocks.difference(&model_blocks) {
            diffs.push(
                Diff::new(
                    format!(
                        "query {shape}: Tine-only block ({})",
                        block_feature(b, &tine_blocks)
                    ),
                    format!("query {:?}: Tine has block {b:?}, model does not", q.tine),
                )
                .at_block(b),
            );
        }
        for p in model_pages.difference(&tine_pages) {
            diffs.push(
                Diff::new(
                    format!("query {shape}: model-only page ({})", ctx.page_feature(p)),
                    format!("query {:?}: model has page {p:?}, Tine does not", q.tine),
                )
                .at_key(&p.1),
            );
        }
        for p in tine_pages.difference(&model_pages) {
            diffs.push(
                Diff::new(
                    format!("query {shape}: Tine-only page ({})", ctx.page_feature(p)),
                    format!("query {:?}: Tine has page {p:?}, model does not", q.tine),
                )
                .at_key(&p.1),
            );
        }
    }
    diffs
}

/// Census Q3 item 2: query reference-day facts against the supplied native
/// date parser. Direct refs include namespace prefixes (eval.rs Rel::DirectRefs),
/// so the test oracle includes those too. The model has no `journal` leaf.
pub fn compare_reference_days(ctx: &Ctx) -> Vec<Diff> {
    let mut diffs = Vec::new();
    let text = "any(direct_refs, journal = true)";
    let (query, qview) = parse_query_text(text, QueryDialect::Tql, JournalDate::today());
    if query.is_invalid() {
        diffs.push(Diff::new(
            "harness: Tine rejects query text (reference day)",
            format!("query {text:?}: {:?}", query.diagnostics),
        ));
        return diffs;
    }
    let tine = match ctx.view.query_ir(
        IrRequest::Run {
            query: &query,
            view: &qview,
            context: &ExecutionContext::none(),
        },
        &Cancel::default(),
    ) {
        Ok(Some(IrAnswer::Result(result))) => match &result.rows {
            QueryRows::Block { groups } => group_blocks(ctx, groups, &mut diffs, "reference day"),
            QueryRows::Page { .. } => BTreeSet::new(),
        },
        other => {
            diffs.push(Diff::new(
                "harness: query did not run",
                format!("query {text:?}: {:?}", other.map(|a| a.is_some())),
            ));
            return diffs;
        }
    };
    let mut native: BTreeMap<BlockId, Vec<String>> = BTreeMap::new();
    for (path, f) in &ctx.files {
        for (i, (_, occ)) in f.blocks.iter().enumerate() {
            let dated: Vec<String> = occ
                .iter()
                .flat_map(|(_, s)| {
                    std::iter::once(s.as_str()).chain(tine_core::refs::namespace_refs(s))
                })
                .filter(|s| ctx.journal.parse(s.trim()).is_some())
                .map(str::to_owned)
                .collect();
            if !dated.is_empty() {
                native.insert((path.clone(), i), dated);
            }
        }
    }
    for (b, spellings) in &native {
        if !tine.contains(b) {
            diffs.push(Diff::new(
                "date answerers: query `journal` fact misses a native date spelling",
                format!("query {text:?}: native list reads {spellings:?} in block {b:?} as a day, the query does not"),
            ));
        }
    }
    for b in tine.iter().filter(|b| !native.contains_key(*b)) {
        diffs.push(Diff::new(
            "date answerers: query `journal` fact on a spelling the native list rejects",
            format!("query {text:?}: block {b:?} matches, the native list reads no day in it"),
        ));
    }
    diffs
}

// ------------------------------------------------- census Q2: file -> name

/// The answer to "what is this file's page name": the store's inventory
/// entry for the file (`effective_page_name` family in
/// `model/page_identity.rs`: `title::` else the decoded stem; a journal's
/// formatted day). `None` for a file that names no page (a sync-conflict
/// copy).
pub fn answered_name(view: &WholeGraph, rel: &str) -> Option<String> {
    view.inventory(InventoryScope::File(&PageId::from(rel)))
        .0
        .first()
        .map(|e| e.name.clone())
}

/// Org `[[file:…]]` links in `text`: (link value, label).
pub fn org_file_links(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[file:") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];
        let (value, label) = match inner.split_once("][") {
            Some((v, l)) => (v, l),
            None => (inner, ""),
        };
        out.push((value.to_owned(), label.to_owned()));
        rest = &after[end..];
    }
    out
}

/// The graph-relative path an Org file link in `from` points to.
pub fn file_link_path(from: &str, value: &str) -> String {
    let target = value.strip_prefix("file:").unwrap_or(value);
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for p in target.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// Census Q2, the parts every harness can run: the answer against the OG
/// oracle (`ogNames`: checks Tine's codec and title parser, which the model
/// takes as given), and the Org file-link decode
/// (`block_regions::reference_target_name` with filename candidates, the
/// policy the frontend's `linkTargetName` and `nearestLink` call through
/// wasm) against the linked file's answered name.
pub fn compare_file_names(view: &WholeGraph, spec: &GraphSpec) -> Vec<Diff> {
    let mut diffs = Vec::new();
    for (rel, og) in &spec.og_names {
        if !spec.files.contains_key(rel) {
            diffs.push(Diff::new(
                "harness: ogNames names a file the fixture lacks",
                format!("harness: ogNames {rel}"),
            ));
            continue;
        }
        let tine = answered_name(view, rel);
        if tine.as_deref() != Some(og.as_str()) {
            diffs.push(Diff::new(
                "file name: Tine's name differs from OG's",
                format!("{rel}: OG names it {og:?}, Tine {tine:?}"),
            ));
        }
    }
    for (rel, text) in &spec.files {
        if file_format(rel) != Format::Org {
            continue;
        }
        for (value, label) in org_file_links(text) {
            let target = file_link_path(rel, &value);
            let Some(answer) = spec
                .files
                .contains_key(&target)
                .then(|| answered_name(view, &target))
                .flatten()
            else {
                continue;
            };
            let decoded = tine_core::block_regions::reference_target_name(
                "file",
                &value,
                &label,
                true,
                true,
                spec.name_format() == FileNameFormat::Legacy,
            );
            if decoded.as_deref() != Some(answer.as_str()) {
                diffs.push(Diff::new(
                    "file name: an Org file link's decoded target differs from the linked file's name",
                    format!(
                        "{rel}: [[{value}]] decodes to {decoded:?} (filename candidates), \
                         {target} is named {answer:?}"
                    ),
                ));
            }
        }
    }
    diffs
}

/// What the frontend name checks (`src/cmConformanceFileNames.test.ts`)
/// need, per fixture with a page header or an Org file link: per ordinary
/// page file its format, the `pre_block` a `PageDto` carries, the store's
/// title (`page_title_from_preamble`) and answered name; per Org block with
/// a file link its raw text and the linked file's answered name.
pub fn names_case(view: &WholeGraph, spec: &GraphSpec) -> Option<Value> {
    let corpus = view.corpus(None, false);
    let mut files = Vec::new();
    let mut links = Vec::new();
    for (rel, text) in &spec.files {
        let format = file_format(rel);
        let fmt = if format == Format::Org { "org" } else { "md" };
        let page = corpus.pages.iter().find(|p| p.id.as_str() == rel);
        if rel.starts_with("pages/") {
            if let Some(page) = page {
                if page.document.pre_block.is_some() {
                    files.push(json!({
                        "path": rel,
                        "format": fmt,
                        "preBlock": page.document.pre_block,
                        "title": page_title_from_preamble(text, format),
                        "name": answered_name(view, rel),
                    }));
                }
            }
        }
        if format != Format::Org {
            continue;
        }
        let parsed = parse_file(rel, text);
        let mut raws = Vec::new();
        fn collect(blocks: &[DocBlock], out: &mut Vec<String>) {
            for b in blocks {
                out.push(b.raw().to_owned());
                collect(&b.children, out);
            }
        }
        collect(&parsed.roots, &mut raws);
        for raw in raws {
            for (value, _) in org_file_links(&raw) {
                let target = file_link_path(rel, &value);
                if let Some(answer) = spec
                    .files
                    .contains_key(&target)
                    .then(|| answered_name(view, &target))
                    .flatten()
                {
                    links.push(json!({"path": rel, "raw": raw, "target": target, "name": answer}));
                }
            }
        }
    }
    let file_name_format = match spec.name_format() {
        FileNameFormat::Legacy => "legacy",
        FileNameFormat::TripleLowbar => "triple-lowbar",
    };
    (!files.is_empty() || !links.is_empty())
        .then(|| json!({"fileNameFormat": file_name_format, "files": files, "links": links}))
}

/// The bridge's own harness reports.
pub fn harness_reports(model: &Value) -> Vec<Diff> {
    let mut diffs = Vec::new();
    for field in ["missingKeys", "nonLocal"] {
        for s in model[field].as_array().unwrap() {
            diffs.push(Diff::new(
                format!("harness: {field}"),
                format!("harness: {field} {s}"),
            ));
        }
    }
    diffs
}

/// Whether the graph has a duplicate claim (two files, or a page and a
/// journal, claiming one key: the model's `conflicts`). Code-map observations
/// 2 and 3 make nearly every answer of such a graph differ, so its random
/// differences are classed coarsely under one prefix; the fixtures keep their
/// details.
pub fn has_duplicate_claim(ctx: &Ctx) -> bool {
    !ctx.conflicts.is_empty() || ctx.files.keys().any(|p| !ctx.page_of_path.contains_key(p))
}

/// The coarse class of a difference in a graph with a duplicate claim: its
/// comparison and direction only.
pub fn coarse(diffs: Vec<Diff>) -> Vec<Diff> {
    diffs
        .into_iter()
        .map(|d| {
            if d.is_harness() {
                return d;
            }
            let cut = d
                .class
                .find(" (")
                .into_iter()
                .chain(d.class.find(", "))
                .chain(d.class.find(" of "))
                .min()
                .unwrap_or(d.class.len());
            Diff::new(format!("[duplicate claim] {}", &d.class[..cut]), d.detail)
        })
        .collect()
}

/// Every comparison a public `WholeGraph` answers.
pub fn compare_meaning(ctx: &Ctx, queries: &[QuerySpec]) -> Vec<Diff> {
    let mut diffs = harness_reports(ctx.model);
    diffs.extend(compare_names(ctx));
    diffs.extend(compare_backlinks(ctx));
    diffs.extend(compare_counts(ctx));
    diffs.extend(compare_classes(ctx));
    diffs.extend(compare_queries(ctx, queries));
    diffs
}

// ---------------------------------------------------------------- generator

/// Deterministic splitmix64.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

/// The colliding alphabet: case variants, a namespace and its parent, a date
/// shape, and plain names.
pub const NAMES: &[&str] = &["A", "a", "B", "n", "n/y", "N/y", "2026-10-26", "x"];
/// Rename destinations: the alphabet plus fresh names and namespaces.
pub const NEW_NAMES: &[&str] = &[
    "A",
    "a",
    "B",
    "n",
    "n/y",
    "N/y",
    "2026-10-26",
    "x",
    "Z",
    "n/z",
    "Z/w",
];

fn random_block(rng: &mut Rng) -> String {
    let x = *rng.pick(NAMES);
    let y = *rng.pick(NAMES);
    let mut text = match rng.below(8) {
        0 => format!("see [[{x}]]"),
        1 => format!("#{x} tagged"),
        2 => format!("TODO task [[{x}]]"),
        3 => "[#A] priority".to_owned(),
        4 => "plain words".to_owned(),
        5 => format!("[[{x}]] and [[{y}]]"),
        6 => format!("{x} mentioned"),
        _ => format!("note\n  rel:: [[{x}]]"),
    };
    if rng.chance(25) {
        let z = *rng.pick(NAMES);
        text.push_str(&format!("\n  - child [[{z}]]"));
    }
    text
}

fn random_leaf(rng: &mut Rng, block: bool) -> Value {
    let s = *rng.pick(NAMES);
    if block {
        if rng.chance(75) {
            json!({"refs": s})
        } else {
            json!({"page": {"name": s}})
        }
    } else {
        json!({"name": s})
    }
}

fn random_query(rng: &mut Rng, block: bool) -> Value {
    match rng.below(5) {
        0 | 1 => random_leaf(rng, block),
        2 => json!({"and": [random_leaf(rng, block), {"not": random_leaf(rng, block)}]}),
        3 => json!({"or": [random_leaf(rng, block), random_leaf(rng, block)]}),
        _ => json!({"not": random_leaf(rng, block)}),
    }
}

/// A random graph of at most five files over the colliding alphabet, with a
/// block query and sometimes a page query.
pub fn random_graph(seed: u64) -> GraphSpec {
    let mut rng = Rng::new(seed);
    let mut files = BTreeMap::new();
    for _ in 0..1 + rng.below(5) {
        let (path, journal) = if rng.chance(15) {
            (
                format!("journals/{}.md", rng.pick(&["2026_10_26", "2026_10_27"])),
                true,
            )
        } else {
            (
                format!(
                    "pages/{}.md",
                    encode_page_name(rng.pick(NAMES), FileNameFormat::Legacy)
                ),
                false,
            )
        };
        if files.contains_key(&path) {
            continue;
        }
        let text = if rng.chance(10) {
            String::new()
        } else {
            let mut header = Vec::new();
            if !journal && rng.chance(20) {
                header.push(format!("title:: {}", rng.pick(NAMES)));
            }
            if rng.chance(35) {
                let a = *rng.pick(NAMES);
                if rng.chance(30) {
                    header.push(format!("alias:: {a}, {}", rng.pick(NAMES)));
                } else {
                    header.push(format!("alias:: {a}"));
                }
            }
            let blocks: Vec<String> = (0..rng.below(4)).map(|_| random_block(&mut rng)).collect();
            let mut text = header.join("\n");
            if !header.is_empty() {
                text.push('\n');
                if !blocks.is_empty() {
                    text.push('\n');
                }
            }
            for b in blocks {
                text.push_str(&format!("- {b}\n"));
            }
            text
        };
        files.insert(path, text);
    }
    let mut queries = Vec::new();
    for block in [true, false] {
        if !block && !rng.chance(50) {
            continue;
        }
        let q = random_query(&mut rng, block);
        let anchor = if block { "block" } else { "page" };
        if let Some((tine, dialect)) = tine_query_text(anchor, &q) {
            queries.push(QuerySpec {
                anchor: anchor.to_owned(),
                q,
                tine,
                dialect,
            });
        }
    }
    GraphSpec {
        name: format!("random-{seed}"),
        files,
        queries,
        ..Default::default()
    }
}

/// A random rename for `spec`: mostly of a page file's own name.
pub fn random_rename(seed: u64, spec: &GraphSpec) -> (String, String) {
    let mut rng = Rng::new(seed ^ 0x5EED_0F_4E4A_3E);
    let own: Vec<String> = spec
        .files
        .iter()
        .filter(|(p, _)| p.starts_with("pages/"))
        .map(|(p, text)| {
            page_title_from_preamble(text, Format::Md).unwrap_or_else(|| {
                decode_page_name(
                    Path::new(p).file_stem().unwrap().to_str().unwrap(),
                    FileNameFormat::Legacy,
                )
            })
        })
        .collect();
    loop {
        let old = if !own.is_empty() && rng.chance(80) {
            rng.pick(&own).clone()
        } else {
            (*rng.pick(NAMES)).to_owned()
        };
        let new = (*rng.pick(NEW_NAMES)).to_owned();
        if old != new {
            return (old, new);
        }
    }
}

// --------------------------------------------------- expected differences

/// One harness's results: per fixture its difference details, per random
/// class its smallest example.
#[derive(Default)]
pub struct Section {
    pub fixtures: BTreeMap<String, Vec<String>>,
    pub random: BTreeMap<String, (usize, Value)>,
    pub random_graphs: usize,
    pub harness_skipped: Vec<String>,
    /// Fixture difference lines this volume cannot pose, as (fixture, line
    /// prefix; empty for the whole fixture). Never written to the file.
    pub volume_skipped: BTreeSet<(String, String)>,
}

impl Section {
    /// This volume folds two of the fixture's file names into one
    /// ([`listed_apart`]): its lines starting with `prefix` are not compared
    /// here, and are kept as recorded by a case-sensitive volume.
    pub fn skip_on_volume(&mut self, fixture: &str, prefix: &str) {
        eprintln!(
            "SKIP on this volume: fixture {fixture} {prefix:?}: two of its file names name one file here"
        );
        self.volume_skipped
            .insert((fixture.to_owned(), prefix.to_owned()));
    }

    /// Whether `line` of `fixture` is one this volume cannot pose.
    pub fn volume_skips(&self, fixture: &str, line: &str) -> bool {
        self.volume_skipped
            .iter()
            .any(|(name, prefix)| name == fixture && line.starts_with(prefix.as_str()))
    }

    /// Record a random graph's differences, keeping the smallest example per class.
    pub fn add_random(&mut self, spec: &GraphSpec, diffs: &[Diff]) {
        self.random_graphs += 1;
        for d in diffs {
            let size = spec.size();
            let replace = self.random.get(&d.class).is_none_or(|(old, _)| size < *old);
            if replace {
                self.random.insert(
                    d.class.clone(),
                    (size, json!({"graph": spec.to_json(), "detail": d.detail})),
                );
            }
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "fixtures": self.fixtures,
            "random": self.random.iter().map(|(k, (_, v))| (k.clone(), v.clone())).collect::<BTreeMap<_, _>>(),
            "randomGraphs": self.random_graphs,
            "harnessSkipped": self.harness_skipped,
        })
    }
}

/// Compare `section` with the expected-differences file, or rewrite that
/// part of the file when `CM_REFRESH` is set. Panics when the set of
/// difference classes changed.
pub fn check_expected(manifest_dir: &str, part: &str, section: &Section, refresh_hint: &str) {
    let path = fixture_dir(manifest_dir).join("expected-differences.json");
    let mut file: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    let mut now = section.to_json();
    if std::env::var_os("CM_REFRESH").is_some() {
        // A folding volume cannot pose the skipped lines: keep them as recorded.
        for (name, lines) in file[part]["fixtures"].as_object().into_iter().flatten() {
            let kept: Vec<String> = strings(lines)
                .into_iter()
                .filter(|line| section.volume_skips(name, line))
                .collect();
            if kept.is_empty() {
                continue;
            }
            let mut merged: BTreeSet<String> =
                strings(&now["fixtures"][name]).into_iter().collect();
            merged.extend(kept);
            now["fixtures"][name] = json!(merged);
        }
        file[part] = now;
        fs::write(&path, serde_json::to_string_pretty(&file).unwrap() + "\n").unwrap();
        eprintln!("cm conformance: refreshed {} [{part}]", path.display());
        return;
    }
    let was = &file[part];
    let mut problems = Vec::new();
    let fixtures_was: BTreeMap<String, BTreeSet<String>> = was["fixtures"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), strings(v).into_iter().collect()))
                .collect()
        })
        .unwrap_or_default();
    let fixtures_now: BTreeMap<String, BTreeSet<String>> = section
        .fixtures
        .iter()
        .map(|(k, v)| (k.clone(), v.iter().cloned().collect()))
        .collect();
    let names: BTreeSet<&String> = fixtures_was.keys().chain(fixtures_now.keys()).collect();
    let empty = BTreeSet::new();
    for name in names {
        let a = fixtures_was.get(name).unwrap_or(&empty);
        let b = fixtures_now.get(name).unwrap_or(&empty);
        for d in b.difference(a).filter(|d| !section.volume_skips(name, d)) {
            problems.push(format!("NEW in fixture {name}: {d}"));
        }
        for d in a.difference(b).filter(|d| !section.volume_skips(name, d)) {
            problems.push(format!("GONE from fixture {name}: {d}"));
        }
    }
    let classes_was: BTreeSet<String> = was["random"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let classes_now: BTreeSet<String> = section.random.keys().cloned().collect();
    for c in classes_now.difference(&classes_was) {
        problems.push(format!(
            "NEW random class: {c}\n    e.g. {}",
            section.random[c].1
        ));
    }
    for c in classes_was.difference(&classes_now) {
        problems.push(format!("GONE random class: {c}"));
    }
    let skipped_was: BTreeSet<String> = strings(&was["harnessSkipped"]).into_iter().collect();
    let skipped_now: BTreeSet<String> = section.harness_skipped.iter().cloned().collect();
    if skipped_was != skipped_now {
        problems.push(format!(
            "harness-skipped graphs changed: was {skipped_was:?}, now {skipped_now:?}"
        ));
    }
    assert!(
        problems.is_empty(),
        "conceptual model conformance [{part}]: the differences changed.\n{}\n\n\
         If the change is intended (a fix, or a new difference you have recorded in the \
         divergence notes), refresh {} with:\n  {refresh_hint}",
        problems.join("\n"),
        path.display()
    );
}
