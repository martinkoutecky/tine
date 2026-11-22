//! Page sources shared by folder publications and quick shares.

use crate::build_graph::MemoryFile;
use crate::publication_source::{visit_document_calls, visit_macro_calls};
use crate::publish_query::refusal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::Arc;
use tine_core::config::{macro_definition_spans, top_level_entry, Config};
use tine_core::edn::{self, Edn};
use tine_core::{Corpus, CorpusPage};
use tine_store::WholeGraph;

const CONFIG_PATH: &str = "logseq/config.edn";

const READING_KEYS: &[&str] = &[
    ":pages-directory",
    ":journals-directory",
    ":journal/page-title-format",
    ":date-formatter",
    ":journal/file-name-format",
    ":preferred-format",
    ":file/name-format",
    ":block-hidden-properties",
    ":property/separated-by-commas",
    ":property-pages/enabled?",
    ":property-pages/excludelist",
    ":ignored-page-references-keywords",
    ":start-of-week",
];

#[derive(Serialize, Deserialize)]
pub(crate) struct SourceFile<'a> {
    #[serde(borrow)]
    path: Cow<'a, str>,
    #[serde(borrow)]
    text: Cow<'a, str>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Snapshot<'a> {
    schema: u32,
    #[serde(borrow)]
    name: Cow<'a, str>,
    #[serde(borrow)]
    exported_at: Cow<'a, str>,
    #[serde(borrow)]
    files: Vec<SourceFile<'a>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) query_home: Option<crate::query_wire::PublicationQuery>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ShareDocument<'a> {
    pub(crate) quick_share: u32,
    #[serde(borrow)]
    pub(crate) snapshot: Snapshot<'a>,
}

#[derive(Default)]
struct BoundedShareJson(Vec<u8>);

impl io::Write for BoundedShareJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > crate::quick_share::MAX_DOCUMENT_BYTES - self.0.len() {
            return Err(refusal("the shared pages exceed the 32 MiB share limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct HomePage {
    pub name: String,
    path: String,
}

impl HomePage {
    pub fn new(graph: &WholeGraph, name: &str) -> Self {
        let config = graph.config();
        let name = tine_core::doc::normalize_line_endings(name).replace('\n', " ");
        let stem = tine_core::model::encode_page_name(&name, config.file_name_format);
        let stem = if stem.len() + ".md".len() <= 255 {
            stem
        } else {
            format!("export-{:x}", Sha256::digest(name.as_bytes()))
        };
        let dir = Path::new(&config.pages_dir)
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .expect("a UTF-8 configured path")
            })
            .collect::<Vec<_>>()
            .join("/");
        Self {
            name,
            path: format!("{dir}/{stem}.md"),
        }
    }

    pub fn taken(&self, graph: &WholeGraph, pages: &[CorpusPage]) -> bool {
        graph.publication_home_name_taken(&self.name)
            || pages.iter().any(|page| page.id.as_str() == self.path)
    }

    pub fn source(&self, pages: &[CorpusPage], query: Option<&str>) -> MemoryFile {
        let mut text = format!("title:: {}\n\n", self.name);
        if let Some(query) = query {
            text.push_str("- ");
            text.push_str(query);
            text.push('\n');
        }
        for page in pages {
            text.push_str(&format!("- [[{}]]\n", page.name));
        }
        (self.path.clone(), Arc::new(text.into_bytes()))
    }
}

fn reduced_config(source: &str, home: &str, names: &HashSet<String>) -> String {
    let mut out = String::from("{\n");
    for entry in READING_KEYS
        .iter()
        .filter_map(|key| top_level_entry(source, key))
    {
        out.push_str(entry);
        out.push('\n');
    }
    let macros = macro_definition_spans(source, names);
    if !macros.is_empty() {
        out.push_str(":macros {\n");
        for entry in macros {
            out.push_str(entry);
            out.push('\n');
        }
        out.push_str("}\n");
    }
    if !home.is_empty() {
        out.push_str(&format!(
            ":default-home {{:page {}}}\n",
            edn::to_string(&Edn::Str(home.into()))
        ));
    }
    out.push_str("}\n");
    out
}

fn page_macros(corpus: &Corpus, config: &Config) -> HashSet<String> {
    let mut names = HashSet::new();
    if !config.macros.is_empty() {
        for page in &corpus.pages {
            let _: Result<(), std::convert::Infallible> = visit_document_calls(
                &page.document,
                |_, _, _| Ok(()),
                |name, args, org| {
                    let mut budget = (1_000, 256 * 1024);
                    follow_macro(config, &mut names, org, name, args, 0, &mut budget);
                    Ok(())
                },
            );
        }
    }
    names
}

fn follow_macro(
    config: &Config,
    names: &mut HashSet<String>,
    org: bool,
    name: &str,
    args: &[String],
    depth: usize,
    budget: &mut (usize, usize),
) {
    if !tine_core::render::builtin_macro(name).is_empty() {
        return;
    }
    let Some(template) = config.macros.get(name) else {
        return;
    };
    names.insert(name.to_owned());
    if depth == 12 || budget.0 == 0 {
        return;
    }
    budget.0 -= 1;
    let Some(expanded) = expand_macro_template(template, args, budget.1) else {
        return;
    };
    budget.1 -= expanded.encode_utf16().count();
    let _: Result<(), std::convert::Infallible> =
        visit_macro_calls(&expanded, org, false, |name, args| {
            follow_macro(config, names, org, name, args, depth + 1, budget);
            Ok(())
        });
}

fn expand_macro_template(mut template: &str, args: &[String], mut budget: usize) -> Option<String> {
    let mut result = String::new();
    while let Some(at) = template.find('$') {
        let after = &template[at + 1..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        let value = after[..digits]
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| args.get(i).map(String::as_str))
            .unwrap_or(&template[at..at + 1 + digits]);
        budget = budget
            .checked_sub(template[..at].encode_utf16().count() + value.encode_utf16().count())?;
        result.push_str(&template[..at]);
        result.push_str(value);
        template = &after[digits..];
    }
    budget.checked_sub(template.encode_utf16().count())?;
    result.push_str(template);
    Some(result)
}

/// Writes exactly the files admitted by the browser's in-memory graph loader.
pub(crate) fn snapshot(
    selecting_view: &WholeGraph,
    corpus: &Corpus,
    files: Vec<MemoryFile>,
    name: &str,
    exported_at: &str,
    home: &str,
    generated: Option<MemoryFile>,
    quick_share: bool,
) -> io::Result<Vec<u8>> {
    snapshot_with_query(
        selecting_view,
        corpus,
        files,
        name,
        exported_at,
        home,
        generated,
        quick_share,
        None,
    )
}

pub(crate) fn snapshot_with_query(
    selecting_view: &WholeGraph,
    corpus: &Corpus,
    mut files: Vec<MemoryFile>,
    name: &str,
    exported_at: &str,
    home: &str,
    generated: Option<MemoryFile>,
    quick_share: bool,
    query_home: Option<crate::query_wire::PublicationQuery>,
) -> io::Result<Vec<u8>> {
    let config = selecting_view.config();
    let names = page_macros(corpus, config);
    let reduced = reduced_config(config.source().unwrap_or(""), home, &names);
    let generated_path = generated.as_ref().map(|(path, _)| path.clone());
    files.extend(generated);
    files.push((CONFIG_PATH.into(), Arc::new(reduced.into_bytes())));
    if quick_share {
        crate::share_admission::admit(
            name,
            exported_at,
            files.iter().map(|(path, text)| (path, text.as_slice())),
        )
        .map_err(|error| refusal(error.to_string()))?;
    }
    let (closed, _) = WholeGraph::from_files(files.clone())?;
    if carried(config) != carried(closed.config()) {
        return Err(refusal(
            "the reduced config did not preserve the graph's reading settings",
        ));
    }
    let actual = closed.corpus(None, true);
    let expected_pages = corpus.pages.len() + usize::from(generated_path.is_some());
    let matches_home = generated_path.as_ref().is_none_or(|path| {
        actual
            .pages
            .iter()
            .any(|page| page.id.as_str() == path && page.name == home)
    });
    let identities: HashSet<_> = actual
        .pages
        .iter()
        .map(|page| (page.id.as_str(), page.name.as_str(), page.kind))
        .collect();
    if !matches_home
        || actual.pages.len() != expected_pages
        || files.len() != expected_pages + 1
        || corpus
            .pages
            .iter()
            .any(|page| !identities.contains(&(page.id.as_str(), page.name.as_str(), page.kind)))
    {
        return Err(refusal(
            "published sources did not read back as the same pages; nothing was published",
        ));
    }
    drop(actual);
    drop(closed);
    let files = files
        .iter()
        .map(|(path, bytes)| {
            Ok(SourceFile {
                path: path.as_str().into(),
                text: std::str::from_utf8(bytes).map_err(io::Error::other)?.into(),
            })
        })
        .collect::<io::Result<_>>()?;
    let snapshot = Snapshot {
        schema: 2,
        name: name.into(),
        exported_at: exported_at.into(),
        files,
        query_home,
    };
    if !quick_share {
        return serde_json::to_vec(&snapshot).map_err(io::Error::other);
    }
    let mut json = BoundedShareJson::default();
    serde_json::to_writer(
        &mut json,
        &ShareDocument {
            quick_share: crate::quick_share::GENERATION,
            snapshot,
        },
    )
    .map_err(io::Error::other)?;
    Ok(json.0)
}

impl Snapshot<'_> {
    pub(crate) fn query_membership(self) -> io::Result<Vec<String>> {
        self.membership(false)
    }

    pub(crate) fn share_summary(mut self) -> io::Result<(String, String, Vec<String>)> {
        crate::share_admission::check_sizes(
            &self.name,
            &self.exported_at,
            self.files
                .iter()
                .map(|file| (file.path.len(), file.text.len())),
        )
        .map_err(|error| refusal(error.to_string()))?;
        Ok((
            std::mem::take(&mut self.name).into_owned(),
            std::mem::take(&mut self.exported_at).into_owned(),
            self.membership(true)?,
        ))
    }

    fn membership(self, include_single_home: bool) -> io::Result<Vec<String>> {
        if self.schema != 2 {
            return Err(refusal("unknown publication snapshot schema"));
        }
        let config_count = self
            .files
            .iter()
            .filter(|file| file.path == CONFIG_PATH)
            .count();
        let file_count = self.files.len();
        let (config, mut pages) = tine_store::publish::publication_page_names(
            self.files
                .into_iter()
                .map(|file| (file.path.into_owned(), file.text.into_owned().into_bytes()))
                .collect(),
        )?;
        let home = config.default_home.as_deref();
        if config_count != 1
            || pages.len() + 1 != file_count
            || !pages.iter().any(|(_, name)| Some(name.as_str()) == home)
        {
            return Err(refusal("invalid publication membership"));
        }
        let include_home = include_single_home && pages.len() == 1;
        pages.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Ok(pages
            .into_iter()
            .filter_map(|(_, name)| (include_home || Some(name.as_str()) != home).then_some(name))
            .collect())
    }
}

fn carried(config: &Config) -> impl PartialEq + std::fmt::Debug + '_ {
    // Exhaustive on purpose. A new Config field needs a disclosure decision.
    let Config {
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
    } = config;
    (
        (
            journals_dir,
            pages_dir,
            start_of_week,
            block_hidden_properties,
            separated_by_commas,
            ignored_page_references_keywords,
            property_pages_enabled,
            property_pages_excludelist,
        ),
        (
            journal_file_name_format,
            journal_page_title_format,
            preferred_format,
            file_name_format,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_keeps_reachable_macros_and_drops_shadowed_builtins() {
        let (graph, _) = WholeGraph::from_files(
            [
                ("pages/Markdown.md", "- {{query (task TODO)}} {{embed [[Markdown]]}} {{outer}}"),
                ("pages/Org.org", "* {{tine-query @page}} {{org-outer}}"),
                ("logseq/config.edn", r#"{:macros {"query" "Private Query" "embed" "Private Embed" "tine-query" "Private TQL" "outer" "{{inner}}" "inner" "nested Markdown" "org-outer" "{{org-inner}}" "org-inner" "nested Org"}}"#),
            ].map(|(path, text)| (path.to_owned(), text.as_bytes().to_vec())).to_vec(),
        ).unwrap();
        let corpus = graph.corpus(None, true);
        let (_, _, files) = graph
            .subgraph(
                corpus
                    .pages
                    .iter()
                    .map(|page| (page.id.as_str().to_owned(), Arc::clone(&page.document)))
                    .collect(),
                true,
            )
            .unwrap();
        let bytes = snapshot(
            &graph, &corpus, files, "Markdown", "", "Markdown", None, false,
        )
        .unwrap();
        let written: Snapshot = serde_json::from_slice(&bytes).unwrap();
        let config = written
            .files
            .iter()
            .find(|file| file.path == CONFIG_PATH)
            .unwrap();
        let macros = Config::parse(&config.text).macros;
        assert_eq!(macros.len(), 4, "{macros:?}");
        assert_eq!(macros["inner"], "nested Markdown");
        assert_eq!(macros["org-inner"], "nested Org");
        assert!(macros.contains_key("outer") && macros.contains_key("org-outer"));
    }

    #[test]
    fn generated_home_normalizes_line_breaks_and_preserves_the_display_name() {
        let (graph, _) = WholeGraph::from_files::<Vec<u8>>(vec![]).unwrap();
        let corpus = graph.corpus(None, true);
        for display in ["One\nTwo", "One\r\nTwo", "One\rTwo"] {
            let home = HomePage::new(&graph, display);
            let bytes = snapshot(
                &graph,
                &corpus,
                vec![],
                display,
                "",
                &home.name,
                Some(home.source(&[], None)),
                false,
            )
            .unwrap();
            let written: Snapshot = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(written.name, display);
            assert!(written
                .files
                .iter()
                .any(|file| file.text.starts_with("title:: One Two\n")));
        }
    }

    #[test]
    fn writer_refuses_excess_source_blocks_and_both_query_forms() {
        let (graph, _) = WholeGraph::from_files::<Vec<u8>>(vec![]).unwrap();
        use crate::share_admission::{MAX_BLOCKS, MAX_QUERIES};
        for (text, reason) in [
            ("- x\n".repeat(MAX_BLOCKS + 1), "too many blocks"),
            (
                "- {{query (task TODO)}}\n".repeat(MAX_QUERIES + 1),
                "too many queries",
            ),
            (
                "- #+BEGIN_QUERY\n  {:query (task TODO)}\n  #+END_QUERY\n".repeat(MAX_QUERIES + 1),
                "too many queries",
            ),
        ] {
            let corpus = Corpus {
                pages: vec![CorpusPage {
                    id: "pages/Public.md".into(),
                    name: "Public".into(),
                    kind: tine_core::model::PageKind::Page,
                    document: Arc::new(tine_core::doc::parse(&text)),
                }],
            };
            let files = vec![("pages/Public.md".into(), Arc::new(text.into_bytes()))];
            let error =
                snapshot(&graph, &corpus, files, "Public", "", "Public", None, true).unwrap_err();
            assert!(error.to_string().contains(reason), "{error}");
        }
    }

    #[test]
    #[ignore = "run alone for native preamble counts and wall time"]
    fn prior_membership_cost_at_1200_pages() {
        use tine_store::cost_counters;
        let body = "- unrelated paragraph with [[Ref]]\n".repeat(60);
        let mut files = (0..1200)
            .map(|i| SourceFile {
                path: format!("pages/file-{i:04}.md").into(),
                text: format!("title:: Named {i:04}\n\n{body}").into(),
            })
            .collect::<Vec<_>>();
        files.push(SourceFile {
            path: CONFIG_PATH.into(),
            text: "{:default-home {:page \"Named 0000\"}}".into(),
        });
        let bytes = serde_json::to_vec(&Snapshot {
            schema: 2,
            name: "Membership".into(),
            exported_at: "".into(),
            files,
            query_home: None,
        })
        .unwrap();
        for _ in 0..3 {
            cost_counters::reset();
            let started = std::time::Instant::now();
            let pages = serde_json::from_slice::<Snapshot>(&bytes)
                .unwrap()
                .query_membership()
                .unwrap();
            let elapsed = started.elapsed();
            let counts = cost_counters::snapshot();
            eprintln!("membership paths=1200 members={} preambles={} full_reads={} parses={} corpora={} elapsed={elapsed:?}", pages.len(), counts.preamble_reads, counts.full_reads, counts.parses, counts.corpus);
            assert_eq!((pages.len(), pages[0].as_str()), (1199, "Named 0001"));
            assert_eq!(
                (
                    counts.preamble_reads,
                    counts.full_reads,
                    counts.parses,
                    counts.corpus
                ),
                (1200, 1, 0, 0)
            );
        }
    }

    #[test]
    fn macros_in_page_opening_and_property_values_are_calls() {
        let source = "description:: {{header}}\n\nOpening {{opening}}\n\n- `{{literal}}`\n  example:: {{property}}\n";
        let (graph, _) =
            WholeGraph::from_files(vec![("pages/Source.md".into(), source.as_bytes().to_vec())])
                .unwrap();
        let config =
            Config::parse(r#"{:macros {"header" "" "opening" "" "property" "" "literal" ""}}"#);
        let names = page_macros(&graph.corpus(None, true), &config);
        assert_eq!(
            names,
            HashSet::from(["header".into(), "opening".into(), "property".into()])
        );
    }

    #[test]
    fn reduced_config_preserves_every_carried_value_and_classifies_every_field() {
        let used = HashSet::from(["greeting".to_owned(), "ns/other".to_owned()]);
        for source in [
            r#"; header :ignored-page-references-keywords #{:fake}
            {:pages-directory , ; directory comment
             "notes", :journals-directory "daily", :start-of-week 0
             :block-hidden-properties #{:secret ; set comment
             :other}, :property/separated-by-commas #{:x}
             :ignored-page-references-keywords #{:literal}
             :property-pages/enabled? false :property-pages/excludelist #{:exclude}
             :journal/file-name-format "yyyy-MM-dd" :journal/page-title-format false
             :date-formatter "yyyy/MM/dd" :preferred-format :org :file/name-format :triple-lowbar
             :favorites ["Private Favorite"] :default-home {:page "Private Home"}
             :macros {:greeting "shadowed" "greeting" ; keep odd spacing
             "Hello $1" :ns/other "other" :unused "Unused Secret" :bad 7 :ignored "after bad"}}"#,
            r#"{:date-formatter "dd.MM.yyyy" :start-of-week -1 :preferred-format "ORG" :favorites ["torn"#,
        ] {
            let reduced = reduced_config(source, "Public Home", &used);
            let expected = Config::parse(source);
            let actual = Config::parse(&reduced);
            assert_eq!(actual.default_home.as_deref(), Some("Public Home"));
            assert_eq!(
                actual.macros,
                expected
                    .macros
                    .iter()
                    .filter(|(key, _)| used.contains(*key))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            );
            assert!(actual.favorites.is_empty());
            assert_eq!(carried(&actual), carried(&expected));
            for key in READING_KEYS {
                assert_eq!(
                    top_level_entry(&reduced, key),
                    top_level_entry(source, key),
                    "{key}"
                );
            }
            assert!(!reduced.contains("Unused Secret") && !reduced.contains("Private Favorite"));
        }
    }

    #[test]
    fn source_admission_refuses_an_unreviewed_file_even_when_home_reads_back() {
        let (selecting, _) =
            WholeGraph::from_files(vec![("pages/Reviewed.md".into(), b"- body\n".to_vec())])
                .unwrap();
        let corpus = selecting.corpus(None, true);
        let home = HomePage::new(&selecting, "Home");
        let reviewed = ("pages/Reviewed.md".into(), Arc::new(b"- body\n".to_vec()));
        let injected = (
            "pages/Other.md".into(),
            Arc::new(b"- unreviewed\n".to_vec()),
        );
        let make = |files| {
            snapshot(
                &selecting,
                &corpus,
                files,
                "Home",
                "2026-11-22T00:00:00Z",
                &home.name,
                Some(home.source(&corpus.pages, None)),
                false,
            )
        };
        assert!(make(vec![reviewed.clone()]).is_ok());
        assert!(make(vec![reviewed, injected]).is_err());
    }
}
