//! Page sources shared by folder publications and quick shares.

use crate::build_graph::MemoryFile;
use crate::publication_source::{visit_document_calls, visit_macro_calls};
use crate::publish_query::refusal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::sync::Arc;
use tine_core::config::Config;
use tine_core::edn::{self, Edn};
use tine_core::model::PageKind;
use tine_core::{Corpus, CorpusPage};
use tine_store::WholeGraph;

const CONFIG_PATH: &str = "logseq/config.edn";
pub(crate) const CONFIG_OMITTED: &str =
    "Publication uses default reading settings because config.edn could not be read as an EDN map.";

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
    pub fn new(graph: &WholeGraph, pages: &[CorpusPage], name: &str) -> Result<Self, String> {
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
        let path = format!("{dir}/{stem}.md");
        if graph.publication_home_name_taken(&name)
            || pages.iter().any(|page| page.id.as_str() == path)
        {
            return Err(name);
        }
        Ok(Self { name, path })
    }

    pub fn source(&self, pages: &[CorpusPage], query: Option<&str>) -> io::Result<MemoryFile> {
        let mut text = format!("title:: {}\n\n", self.name);
        if let Some(query) = query {
            text.push_str("- ");
            text.push_str(query);
            text.push('\n');
        } else {
            for page in pages {
                let link = tine_core::refs::markdown_page_link(&page.name).ok_or_else(|| {
                    refusal("a published page title could not be written as a link")
                })?;
                writeln!(text, "- {link}").unwrap();
            }
        }
        Ok((self.path.clone(), Arc::new(text.into_bytes())))
    }
}

fn reduced_config(config: &Config, home: &str, names: &HashSet<String>) -> String {
    let mut out = String::from("{\n");
    for (key, value) in config.publication_reading_values() {
        writeln!(out, "{key} {}", edn::to_string(&value)).unwrap();
    }
    let mut macros: Vec<_> = names
        .iter()
        .filter(|name| config.macros.contains_key(*name))
        .collect();
    macros.sort_unstable();
    if !macros.is_empty() {
        out.push_str(":macros {\n");
        for name in macros {
            let key = edn::to_string(&Edn::Str(name.clone()));
            let value = edn::to_string(&Edn::Str(config.macros[name].clone()));
            writeln!(out, "{key} {value}").unwrap();
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
    if config.macros.is_empty() {
        return HashSet::new();
    }
    let mut reached: HashMap<String, HashSet<usize>> = HashMap::new();
    let mut pending = Vec::new();
    for page in &corpus.pages {
        visit_document_calls(&page.document, |name, args| {
            pending.push((name.to_owned(), args.to_vec()));
            while let Some((name, args)) = pending.pop() {
                let Some(template) = config.macros.get(&name) else {
                    continue;
                };
                if !tine_core::render::builtin_macro(&name).is_empty() {
                    continue;
                }
                let slots = reached.entry(name).or_insert_with(|| {
                    let mut slots = HashSet::new();
                    for org in [false, true] {
                        visit_macro_calls(template, org, false, |name, args| {
                            let slot = name
                                .strip_prefix('$')
                                .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_digit()))
                                .and_then(|digits| digits.parse::<usize>().ok())
                                .and_then(|index| index.checked_sub(1));
                            if let Some(slot) = slot {
                                slots.insert(slot);
                            } else {
                                pending.push((name.to_owned(), args.to_vec()));
                            }
                        });
                    }
                    slots
                });
                for arg in slots.iter().filter_map(|slot| args.get(*slot)) {
                    let call = format!("{{{{{arg}}}}}");
                    for org in [false, true] {
                        let mut visit = |name: &str, args: &[String]| {
                            pending.push((name.to_owned(), args.to_vec()))
                        };
                        visit_macro_calls(&call, org, false, &mut visit);
                        visit_macro_calls(arg, org, false, visit);
                    }
                }
            }
        });
    }
    reached.into_keys().collect()
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
    warnings: &mut Vec<String>,
) -> io::Result<Vec<u8>> {
    let source = selecting_view.config().source().unwrap_or("");
    let admitted = Config::parse_for_publication(source);
    let omitted = admitted.is_none();
    let config = admitted.unwrap_or_default();
    let names = page_macros(corpus, &config);
    let reduced = reduced_config(&config, home, &names);
    files.extend(generated.iter().cloned());
    files.push((CONFIG_PATH.into(), Arc::new(reduced.into_bytes())));
    if quick_share {
        crate::share_admission::check_sizes(
            name,
            exported_at,
            files.iter().map(|(path, text)| (path.len(), text.len())),
        )
        .map_err(|error| refusal(error.to_string()))?;
    }
    let (read_config, pages, home_document) = tine_store::publish::publication_page_identities(
        files.clone(),
        generated
            .as_ref()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    )?;
    if config.publication_reading_values() != read_config.publication_reading_values() {
        return Err(refusal(
            "the reduced config did not preserve the graph's reading settings",
        ));
    }
    let expected_pages = corpus.pages.len() + usize::from(generated.is_some());
    let identities: HashSet<_> = pages
        .iter()
        .map(|(path, name, kind)| (path.as_str(), name.as_str(), *kind))
        .collect();
    if generated
        .as_ref()
        .is_some_and(|(path, _)| !identities.contains(&(path.as_str(), home, PageKind::Page)))
        || pages.len() != expected_pages
        || files.len() != expected_pages + 1
        || corpus
            .pages
            .iter()
            .any(|page| !identities.contains(&(page.id.as_str(), page.name.as_str(), page.kind)))
    {
        return Err(refusal(if omitted {
            "config.edn could not be read as an EDN map and default settings change the published page identities. Fix config.edn and try again."
        } else {
            "published sources did not read back as the same pages; nothing was published"
        }));
    }
    if quick_share {
        let documents = corpus.pages.iter().map(|page| page.document.as_ref());
        crate::share_admission::check_blocks(documents.chain(home_document.as_ref()))
            .map_err(|error| refusal(error.to_string()))?;
    }
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
    if omitted {
        warnings.push(CONFIG_OMITTED.into());
    }
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
        let (config, mut pages, _) = tine_store::publish::publication_page_identities(
            self.files
                .into_iter()
                .map(|file| {
                    (
                        file.path.into_owned(),
                        Arc::new(file.text.into_owned().into_bytes()),
                    )
                })
                .collect(),
            None,
        )?;
        let home = config.default_home.as_deref();
        if config_count != 1
            || pages.len() + 1 != file_count
            || !pages.iter().any(|(_, name, _)| Some(name.as_str()) == home)
        {
            return Err(refusal("invalid publication membership"));
        }
        let include_home = include_single_home && pages.len() == 1;
        pages.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Ok(pages
            .into_iter()
            .filter_map(|(_, name, _)| {
                (include_home || Some(name.as_str()) != home).then_some(name)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_config_carries_only_effective_reading_values() {
        let source = r#"{:block-hidden-properties #{:secret ; Private Hidden
            "a private :String-Lookalike" :other}, :property/separated-by-commas #{:comma ; Private Comma
            :ns/other, [:Nested-Lookalike]}, :ignored-page-references-keywords #{:literal ; Private Ignored
            :other}, :property-pages/excludelist #{:exclude ; Private Exclude
            :other}, :property-pages/enabled? {:note "Private Page"}
            :journal/page-title-format false :date-formatter "dd.MM.yyyy"
            :preferred-format "ORG" :file/name-format :triple-lowbar
            :start-of-week 1}"#;
        let (graph, _) = WholeGraph::from_files(vec![
            ("pages/Source.md".into(), b"- body\n".to_vec()),
            (CONFIG_PATH.into(), source.as_bytes().to_vec()),
        ])
        .unwrap();
        let corpus = graph.corpus(None, true);
        let mut leaks = Vec::new();
        for quick_share in [false, true] {
            let bytes = crate::build_graph::with_build_graph_text(
                &graph,
                &corpus.pages,
                "did not build",
                |_, built, files| {
                    snapshot(
                        &graph,
                        &built,
                        files,
                        "Source",
                        "",
                        "Source",
                        None,
                        quick_share,
                        &mut Vec::new(),
                    )
                },
            )
            .unwrap();
            let written = if quick_share {
                serde_json::from_slice::<ShareDocument>(&bytes)
                    .unwrap()
                    .snapshot
            } else {
                serde_json::from_slice::<Snapshot>(&bytes).unwrap()
            };
            let config = &written
                .files
                .iter()
                .find(|file| file.path == CONFIG_PATH)
                .unwrap()
                .text;
            let actual = Config::parse(config);
            let expected = Config::parse(source);
            assert_eq!(
                actual.publication_reading_values(),
                expected.publication_reading_values()
            );
            assert_eq!(actual.block_hidden_properties, ["secret", "other"]);
            assert_eq!(actual.separated_by_commas, ["comma", "ns/other"]);
            assert_eq!(
                actual.ignored_page_references_keywords,
                ["literal", "other"]
            );
            assert_eq!(actual.property_pages_excludelist, ["exclude", "other"]);
            assert!(actual.property_pages_enabled);
            for private in [
                "Private Hidden",
                "Private Comma",
                "Private Ignored",
                "Private Exclude",
                "Private Page",
                "String-Lookalike",
                "Nested-Lookalike",
            ] {
                if config.contains(private) {
                    leaks.push((quick_share, private));
                }
            }
        }
        assert!(
            leaks.is_empty(),
            "private content in publications: {leaks:?}"
        );
    }

    #[test]
    fn publication_config_refuses_unreadable_sets_and_drops_edn_trivia() {
        let keys = [
            ":block-hidden-properties",
            ":property/separated-by-commas",
            ":ignored-page-references-keywords",
            ":property-pages/excludelist",
        ];
        let cases = [
            ("hashbang LF", "#{:public\n#! note :Private\n}", true),
            ("hashbang CRLF", "#{:public\r\n#! note :Private\r\n}", true),
            ("semicolon bare CR", "#{:public ; note\r :Private\n}", true),
            ("hashbang bare CR", "#{:public #! note\r :Private\n}", true),
            ("metadata", "#{:public ^ :Private []}", true),
            ("dispatch metadata", "#{:public #^ :Private []}", true),
            ("discarded metadata", "#{:public #_ ^ :Private []}", true),
            ("discarded instant", r#"#{:public #_ #inst "2020"}"#, false),
            ("discarded UUID", r#"#{:public #_ [#uuid "ABC"]}"#, false),
            ("discarded queue", "#{:public #_ #queue []}", false),
            ("discarded JS vector", "#{:public #_ [#js []]}", false),
            ("discarded JS map", "#{:public #_ #js {:p :Private}}", false),
            ("signed Unicode character", r"#{\u+041 :Private}", false),
            (
                "nested signed Unicode character",
                r"#{[\u+041] :Private}",
                false,
            ),
            (
                "discarded signed Unicode character",
                r"#{#_ \u+041 :Private}",
                false,
            ),
            ("auto-resolved keyword", "#{:public ::Private}", false),
            ("trailing colon", "#{:public :Private:}", false),
            ("empty namespace", "#{:public :/Private}", false),
            ("missing name", "#{:public :Private/}", false),
            ("double namespace", "#{:public :ns/Private/other}", false),
            ("colon in namespace", "#{:public :Private:/name}", false),
            ("digit after namespace", "#{:public :Private/1name}", false),
            ("duplicate member", "#{:public :Private :Private}", false),
            (
                "duplicate nested map key",
                "#{:public {:Private 1 :Private 2}}",
                false,
            ),
            (
                "duplicate nested set member",
                "#{:public #{:Private :Private}}",
                false,
            ),
        ];
        let mut failures = Vec::new();
        for (label, value, keep_public) in cases {
            for key in keys {
                let source =
                    format!("{{:start-of-week 1 {key} {value} :property-pages/enabled? false}}");
                let (graph, _) = WholeGraph::from_files(vec![
                    ("pages/Source.md".into(), b"- body\n".to_vec()),
                    (CONFIG_PATH.into(), source.into_bytes()),
                ])
                .unwrap();
                let corpus = graph.corpus(None, true);
                for quick_share in [false, true] {
                    let mut warnings = Vec::new();
                    let bytes = crate::build_graph::with_build_graph_text(
                        &graph,
                        &corpus.pages,
                        "did not build",
                        |_, built, files| {
                            snapshot(
                                &graph,
                                &built,
                                files,
                                "Source",
                                "",
                                "Source",
                                None,
                                quick_share,
                                &mut warnings,
                            )
                        },
                    )
                    .unwrap();
                    let written = if quick_share {
                        serde_json::from_slice::<ShareDocument>(&bytes)
                            .unwrap()
                            .snapshot
                    } else {
                        serde_json::from_slice::<Snapshot>(&bytes).unwrap()
                    };
                    let config = &written
                        .files
                        .iter()
                        .find(|file| file.path == CONFIG_PATH)
                        .unwrap()
                        .text;
                    let expected = if keep_public {
                        format!("{{:start-of-week 1 {key} #{{:public}} :property-pages/enabled? false}}")
                    } else {
                        "{}".into()
                    };
                    if config.contains("Private")
                        || warnings
                            != if keep_public {
                                Vec::new()
                            } else {
                                vec![CONFIG_OMITTED]
                            }
                        || Config::parse(config).publication_reading_values()
                            != Config::parse(&expected).publication_reading_values()
                    {
                        failures.push((label, key, quick_share, config.to_string()));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "incorrect reading values in publications: {failures:#?}"
        );
    }

    #[test]
    fn publication_config_reads_wrapped_roots_and_the_first_form() {
        for source in [
            "^:private {:start-of-week 1}",
            "#:graph{:_/start-of-week 1 :start-of-week 3}",
            "{:start-of-week 1} #unknown []",
        ] {
            let config = Config::parse_for_publication(source).unwrap();
            assert_eq!(
                reduced_config(&config, "", &HashSet::new()),
                "{\n:start-of-week 1\n}\n",
                "{source}"
            );
        }
    }

    #[test]
    fn generated_home_links_use_the_full_page_titles() {
        let titles = ["Ends]", "Middle]]Here", "A ]] [[ B", r"Slash\]]Here"];
        let (graph, _) = WholeGraph::from_files(
            titles
                .iter()
                .enumerate()
                .map(|(i, title)| {
                    (
                        format!("pages/{i}.md"),
                        format!("title:: {title}\n\n- body\n").into_bytes(),
                    )
                })
                .collect(),
        )
        .unwrap();
        let corpus = graph.corpus(None, true);
        let home = HomePage::new(&graph, &corpus.pages, "Home").unwrap();
        let (_, source) = home.source(&corpus.pages, None).unwrap();
        let document = tine_core::doc::parse(std::str::from_utf8(&source).unwrap());
        let targets = document
            .roots
            .iter()
            .map(|block| {
                block
                    .projection()
                    .reference_source()
                    .explicit
                    .iter()
                    .map(|reference| reference.name.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected = corpus
            .pages
            .iter()
            .map(|page| vec![page.name.clone()])
            .collect::<Vec<_>>();
        assert_eq!(expected.len(), titles.len());
        assert!(titles
            .iter()
            .all(|title| expected.contains(&vec![title.to_string()])));
        assert_eq!(targets, expected);
    }

    #[test]
    fn share_preparation_parses_each_selected_source_once() {
        use tine_store::cost_counters;

        let body = "- selected body with [[Ref]]\n".repeat(60);
        let (graph, _) = WholeGraph::from_files(
            (0..200)
                .map(|i| (format!("pages/Page {i:03}.md"), body.as_bytes().to_vec()))
                .collect(),
        )
        .unwrap();
        let selected = graph.corpus(None, true);
        let mut samples = Vec::new();
        for _ in 0..3 {
            cost_counters::reset();
            let started = std::time::Instant::now();
            crate::build_graph::with_build_graph_text(
                &graph,
                &selected.pages,
                "did not build",
                |built, corpus, files| {
                    let home = HomePage::new(built, &corpus.pages, "Home").unwrap();
                    snapshot(
                        &graph,
                        &corpus,
                        files,
                        "Home",
                        "",
                        "Home",
                        Some(home.source(&corpus.pages, None)?),
                        true,
                        &mut Vec::new(),
                    )
                },
            )
            .unwrap();
            let counts = cost_counters::snapshot();
            let sample = (
                counts.full_reads,
                counts.preamble_reads,
                counts.parses,
                counts.corpus,
            );
            eprintln!("share writer: 200 pages, 12000 selected blocks, full reads/preambles/parses/corpora={sample:?}, elapsed={:?}", started.elapsed());
            samples.push(sample);
        }
        assert_eq!(samples, [(201, 201, 201, 1); 3]);
    }

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
            &graph,
            &corpus,
            files,
            "Markdown",
            "",
            "Markdown",
            None,
            false,
            &mut Vec::new(),
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
            let home = HomePage::new(&graph, &[], display).unwrap();
            let bytes = snapshot(
                &graph,
                &corpus,
                vec![],
                display,
                "",
                &home.name,
                Some(home.source(&[], None).unwrap()),
                false,
                &mut Vec::new(),
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
    fn writer_checks_opened_blocks_and_admits_sources_with_live_queries() {
        let (graph, _) = WholeGraph::from_files::<Vec<u8>>(vec![]).unwrap();
        for (text, too_many_blocks) in [
            ("- x\n".repeat(crate::share_admission::MAX_BLOCKS + 1), true),
            ("- {{query (task TODO)}}\n".repeat(257), false),
            (
                "- #+BEGIN_QUERY\n  {:query (task TODO)}\n  #+END_QUERY\n".repeat(257),
                false,
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
            let result = snapshot(
                &graph,
                &corpus,
                files,
                "Public",
                "",
                "Public",
                None,
                true,
                &mut Vec::new(),
            );
            if too_many_blocks {
                let error = result.unwrap_err();
                assert!(error.to_string().contains("too many blocks"), "{error}");
            } else {
                result.unwrap();
            }
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
    fn macro_closure_uses_names_and_argument_positions_beyond_render_limits() {
        let mut source =
            String::from("{:preferred-format ; private reading gap\n :markdown :macros {");
        for n in 1..=13 {
            let next = if n == 13 {
                "leaf".to_owned()
            } else {
                format!("m{}", n + 1)
            };
            source.push_str(&format!(r#" "m{n}" "{{{{{next}}}}}""#));
        }
        source.push_str(
            r#" "leaf" ; private macro gap
          "visible" "choose" "{{$1 x}}" "inner" "{{deep}}" "deep" "also visible"
          "dual" "={{md-only}}= and {{common}}" "md-only" "Markdown" "common" "shared"
          "ignored" "private unused" "query" "private built-in"}}"#,
        );
        let config = Config::parse(&source);
        let (graph, _) = WholeGraph::from_files(vec![
            (
                "pages/Source.md".into(),
                b"- {{m1}} {{choose inner}} {{query (task TODO)}}".to_vec(),
            ),
            (
                "pages/Other.org".into(),
                b"* {{choose inner}} {{dual}}".to_vec(),
            ),
        ])
        .unwrap();
        let names = page_macros(&graph.corpus(None, true), &config);
        let reduced = reduced_config(&config, "Source", &names);
        assert!(!reduced.contains("private"), "{reduced}");
        let macros = Config::parse(&reduced).macros;
        for name in [
            "m13", "leaf", "choose", "inner", "deep", "md-only", "common",
        ] {
            assert!(macros.contains_key(name), "{name}: {reduced}");
        }
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
            r#"{:journal/page-title-format "yyyy-MM-dd" :date-formatter "Private Fallback"}"#,
        ] {
            let expected = Config::parse_for_publication(source).unwrap_or_default();
            let reduced = reduced_config(&expected, "Public Home", &used);
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
            assert_eq!(actual.pages_dir, expected.pages_dir);
            assert_eq!(actual.journals_dir, expected.journals_dir);
            assert_eq!(
                actual.journal_page_title_format,
                expected.journal_page_title_format
            );
            assert_eq!(
                actual.journal_file_name_format,
                expected.journal_file_name_format
            );
            assert_eq!(actual.preferred_format, expected.preferred_format);
            assert_eq!(actual.file_name_format, expected.file_name_format);
            assert_eq!(
                actual.block_hidden_properties,
                expected.block_hidden_properties
            );
            assert_eq!(actual.separated_by_commas, expected.separated_by_commas);
            assert_eq!(
                actual.property_pages_enabled,
                expected.property_pages_enabled
            );
            assert_eq!(
                actual.property_pages_excludelist,
                expected.property_pages_excludelist
            );
            assert_eq!(
                actual.ignored_page_references_keywords,
                expected.ignored_page_references_keywords
            );
            assert_eq!(actual.start_of_week, expected.start_of_week);
            assert!(!reduced.contains("comment"));
            assert!(!reduced.contains("Private Fallback"));
            assert!(!reduced.contains("Unused Secret") && !reduced.contains("Private Favorite"));
        }
    }

    #[test]
    fn source_admission_refuses_an_unreviewed_file_even_when_home_reads_back() {
        let (selecting, _) =
            WholeGraph::from_files(vec![("pages/Reviewed.md".into(), b"- body\n".to_vec())])
                .unwrap();
        let corpus = selecting.corpus(None, true);
        let home = HomePage::new(&selecting, &corpus.pages, "Home").unwrap();
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
                Some(home.source(&corpus.pages, None).unwrap()),
                false,
                &mut Vec::new(),
            )
        };
        assert!(make(vec![reviewed.clone()]).is_ok());
        assert!(make(vec![reviewed, injected]).is_err());
    }
}
