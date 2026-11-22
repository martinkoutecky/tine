//! Page sources shared by folder publications and quick shares.

use crate::build_graph::MemoryFile;
use crate::publish_query::refusal;
use crate::render::visit_macro_calls;
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
        let stem = tine_core::model::encode_page_name(name, config.file_name_format);
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
            name: name.to_owned(),
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

fn page_macros(
    corpus: &Corpus,
    configured: bool,
    share_home_blocks: Option<usize>,
) -> io::Result<HashSet<String>> {
    let mut names = HashSet::new();
    let mut queries = 0;
    let check_limits = share_home_blocks.is_some();
    let mut blocks = share_home_blocks.unwrap_or_default();
    if !configured && !check_limits {
        return Ok(names);
    }
    let mut call = |name: &str| {
        queries += usize::from(tine_core::query::macro_text::is_query_macro_name(name));
        if configured {
            names.insert(name.to_owned());
        }
    };
    for page in &corpus.pages {
        let org = page.document.org;
        if let Some(pre) = page
            .document
            .pre_block
            .as_deref()
            .filter(|pre| pre.contains("{{"))
        {
            let regions = tine_core::block_regions::parse_document(pre, org);
            for (_, value) in tine_core::page_header::page_property_lines_in(&regions, org) {
                visit_macro_calls(&value, org, true, &mut call);
            }
            if let Some(opening) = tine_core::page_properties::preamble_range_in(pre, org, &regions)
            {
                visit_macro_calls(&pre[opening], org, false, &mut call);
            }
        }
        let mut pending: Vec<_> = page.document.roots.iter().map(|block| (block, 1)).collect();
        while let Some((block, depth)) = pending.pop() {
            if check_limits && depth > crate::quick_share::MAX_BLOCK_DEPTH {
                return Err(refusal("the shared pages nest too deeply to share"));
            }
            blocks += 1;
            if block.raw().contains("{{") {
                visit_macro_calls(block.raw(), org, false, &mut call);
                for (_, value) in block.projection().properties() {
                    visit_macro_calls(value, org, true, &mut call);
                }
            }
            pending.extend(block.children.iter().map(|child| (child, depth + 1)));
        }
    }
    if check_limits
        && (queries > crate::quick_share::MAX_QUERIES || blocks > crate::quick_share::MAX_BLOCKS)
    {
        return Err(refusal("the shared pages hold too many blocks or queries"));
    }
    Ok(names)
}

/// Writes exactly the files admitted by the browser's in-memory graph loader.
pub(crate) fn snapshot(
    selecting_view: &WholeGraph,
    corpus: &Corpus,
    mut files: Vec<MemoryFile>,
    name: &str,
    exported_at: &str,
    home: &str,
    generated: Option<MemoryFile>,
    quick_share: bool,
) -> io::Result<Vec<u8>> {
    let config = selecting_view.config();
    let share_home_blocks =
        quick_share.then(|| generated.as_ref().map_or(0, |_| corpus.pages.len()));
    let names = page_macros(corpus, !config.macros.is_empty(), share_home_blocks)?;
    let reduced = reduced_config(config.source().unwrap_or(""), home, &names);
    let generated_path = generated.as_ref().map(|(path, _)| path.clone());
    files.extend(generated);
    files.push((CONFIG_PATH.into(), Arc::new(reduced.into_bytes())));
    if quick_share
        && files
            .iter()
            .any(|(path, text)| path.len().max(text.len()) > crate::quick_share::MAX_STRING_BYTES)
    {
        return Err(refusal(
            "a shared source file exceeds the 1 MiB string limit",
        ));
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
    pub(crate) fn query_membership(&self) -> io::Result<Vec<String>> {
        self.membership(false)
    }

    pub(crate) fn share_summary(&self) -> io::Result<(String, String, Vec<String>)> {
        use crate::quick_share::{MAX_STRING_BYTES, VIEWER_MAX_PAGES};
        if self.files.len() > VIEWER_MAX_PAGES + 1
            || self.name.len() > MAX_STRING_BYTES
            || self.exported_at.len() > MAX_STRING_BYTES
            || self.files.iter().any(|file| {
                file.text.len() > MAX_STRING_BYTES || file.path.len() > MAX_STRING_BYTES
            })
        {
            return Err(refusal("share sources exceed the viewer limits"));
        }
        Ok((
            self.name.to_string(),
            self.exported_at.to_string(),
            self.membership(true)?,
        ))
    }

    fn membership(&self, include_single_home: bool) -> io::Result<Vec<String>> {
        if self.schema != 2 {
            return Err(refusal("unknown publication snapshot schema"));
        }
        let config_count = self
            .files
            .iter()
            .filter(|file| file.path == CONFIG_PATH)
            .count();
        let file_count = self.files.len();
        let (graph, _) = WholeGraph::from_files(
            self.files
                .iter()
                .map(|file| (file.path.to_string(), file.text.as_bytes().to_vec()))
                .collect(),
        )?;
        let home = graph.config().default_home.as_deref();
        let mut pages = graph.corpus(None, true).pages;
        if config_count != 1
            || pages.len() + 1 != file_count
            || !pages.iter().any(|page| Some(page.name.as_str()) == home)
        {
            return Err(refusal("invalid publication membership"));
        }
        let include_home = include_single_home && pages.len() == 1;
        pages.sort_unstable_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Ok(pages
            .into_iter()
            .filter_map(|page| {
                (include_home || Some(page.name.as_str()) != home).then_some(page.name)
            })
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
    fn macros_in_page_opening_and_property_values_are_calls() {
        let source = "description:: {{header}}\n\nOpening {{opening}}\n\n- `{{literal}}`\n  example:: {{property}}\n";
        let (graph, _) =
            WholeGraph::from_files(vec![("pages/Source.md".into(), source.as_bytes().to_vec())])
                .unwrap();
        let names = page_macros(&graph.corpus(None, true), true, None).unwrap();
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
