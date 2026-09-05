//! Gate 1's walk side (SPEC §8.1, §8's v16 evidence correction).
//!
//! For every `{{query …}}` / `{{tine-query …}}` in the given graphs, run the
//! walk under each of the five §8.1 counterfactual modes and print one row per
//! (query, mode, result identity):
//!
//! ```text
//! <graph>\t<page>\t<query-hash>\t<mode>\t<anchor>\t<identity>
//! ```
//!
//! **Identities, not counts.** Wave B's harness compared how MANY blocks each
//! side returned, so equal counts over different members passed; SPEC §8 and
//! CLOSURE §2 reject that. Identity is anchor-specific (N18): page names for
//! `@page`, blocks for `@block`.
//!
//! A block's cross-engine identity is `<page> › <first content line>`, with
//! `#n` appended to the n-th repeat after sorting. It is NOT the uuid §8.1
//! names, because OG mints a random `:block/uuid` for every block that carries
//! no `id::` property, so a uuid is not an identity the two engines share — it
//! is the identity `case.cljs` already uses to pin OG's measured §8.3 results,
//! which is what makes the two halves of the gate speak one language. Whatever
//! it is spelled as, it satisfies the operative rule: equal counts with
//! different members fail.
//!
//! A query with no results prints one row with the identity `-`, so the join
//! can tell "ran and matched nothing" from "never ran".
//!
//! The query TEXT goes to the file named by `--queries <path>`, keyed by the
//! same hash, so the OG side can run the identical string. That file is the
//! ONLY place a query's bytes appear; stdout carries hashes, so the output of a
//! private graph can be pasted into a receipt.
//!
//! Usage:
//! ```text
//! cargo run -q -p tine-core --example query-gate1-dump -- \
//!     --queries /tmp/gate1-queries.tsv [--query-list <file>] <graph-dir>…
//! ```
//!
//! `--query-list <file>` replaces the macro scan with a fixed list of queries,
//! one per line (`#` comments and blank lines skipped), run against every named
//! graph. It exists for SPEC §8.3's `case.cljs` twin: those queries have to run
//! against `fixture-case-graph` unchanged, and writing them into the graph as
//! `{{query …}}` macros would add blocks — and, for `[[book]]`, a page
//! REFERENCE — that change the very answers the fixture pins.
//!
//! A graph directory that does not exist is reported as `# skipped <label>` on
//! stdout rather than failing, so the same command line works with and without
//! the anonymized graph.

use std::io::Write;
use std::path::PathBuf;

use tine_core::query::atom::CompareMode;
use tine_core::query::ir::Anchor;
use tine_core::query::{parse_query_text, run_query_bounded_in_mode, QueryDialect};
use tine_core::{Graph, JournalDate};

#[path = "support/query_corpus.rs"]
mod query_corpus;

use query_corpus::{fnv1a, graph_files, macro_args, materialize_single_file};

/// The separator between a block's page and its first line. A glyph, not a
/// punctuation character, because a block's content routinely holds `|`, `:`
/// and `#`.
const TRAIL: &str = " › ";

/// One line of identity text, safe to put in a TSV cell: tabs, newlines and
/// runs of spaces collapse to one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The sorted identities of one result set, `#n` disambiguating repeats.
///
/// `@page` reports the pages that answered; `@block`, every matched block. The
/// walk answers a `@page`-anchored query through the legacy block-group adapter
/// (`run_query_bounded_*` rebases the filter — the same semantics OG's
/// `:blocks? false` wrapper has), so its answering pages are its groups.
fn identities(anchor: Anchor, groups: &tine_core::query::BoundedGroups) -> Vec<String> {
    let mut out: Vec<String> = match anchor {
        Anchor::Page => groups
            .groups
            .iter()
            .map(|group| tine_core::refs::page_key(&group.page))
            .collect(),
        Anchor::Block => groups
            .groups
            .iter()
            .flat_map(|group| {
                let page = tine_core::refs::page_key(&group.page);
                group.blocks.iter().map(move |block| {
                    let first = block.raw.lines().next().unwrap_or_default();
                    format!("{page}{TRAIL}{}", one_line(first))
                })
            })
            .collect(),
    };
    out.sort();
    out.dedup_by(|later, earlier| {
        // Two blocks with the same page and first line are one identity plus an
        // occurrence number. Both engines sort first and then number, so the
        // n-th repeat gets the same name on both sides.
        if later != earlier {
            return false;
        }
        let n = earlier
            .rsplit_once('#')
            .and_then(|(_, n)| n.parse::<u32>().ok())
            .unwrap_or(0);
        *later = format!("{later}#{}", n + 1);
        false
    });
    out
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut queries_path: Option<PathBuf> = None;
    let mut query_list: Option<Vec<String>> = None;
    let mut roots: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--queries" => {
                queries_path = args.next().map(PathBuf::from);
            }
            "--query-list" => {
                let path = args.next().unwrap_or_default();
                let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
                    eprintln!("cannot read {path}: {error}");
                    std::process::exit(2);
                });
                query_list = Some(
                    text.lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty() && !line.starts_with('#'))
                        .map(str::to_string)
                        .collect(),
                );
            }
            other => roots.push(other.to_string()),
        }
    }
    if roots.is_empty() {
        eprintln!("usage: query-gate1-dump [--queries <path>] <graph-dir>…");
        std::process::exit(2);
    }
    let mut queries_out = queries_path.map(|path| {
        std::fs::File::create(&path).unwrap_or_else(|error| {
            eprintln!("cannot write {}: {error}", path.display());
            std::process::exit(2);
        })
    });

    for arg in roots {
        let root = PathBuf::from(&arg);
        let label = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(arg.as_str())
            .to_string();
        let (root, scratch) = materialize_single_file(&root);
        if !root.is_dir() {
            println!("# skipped {label} (absent)");
            continue;
        }
        let graph = Graph::open(&root);
        graph.warm_cache();
        let mut rows: Vec<String> = Vec::new();
        // A fixed list has no owning page; the page column is `-` so the join
        // key stays three columns wide either way.
        let scanned: Vec<(String, Vec<String>)> = match &query_list {
            Some(list) => vec![("-".to_string(), list.clone())],
            None => graph_files(&root)
                .into_iter()
                .filter_map(|file| {
                    let text = std::fs::read_to_string(&file).ok()?;
                    let page = file
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let mut sources = macro_args(&text, "query");
                    sources.extend(macro_args(&text, "tine-query"));
                    Some((page, sources))
                })
                .collect(),
        };
        for (page, sources) in scanned {
            for source in sources {
                let hash = format!("{:016x}", fnv1a(source.as_bytes()));
                if let Some(out) = queries_out.as_mut() {
                    // Tabs and newlines would break the join file; a query
                    // containing either is emitted with them escaped, and the
                    // OG side unescapes.
                    let escaped = source
                        .replace('\\', "\\\\")
                        .replace('\t', "\\t")
                        .replace('\n', "\\n");
                    let _ = writeln!(out, "{label}\t{page}\t{hash}\t{escaped}");
                }
                // The anchor is a property of the query text, not of the mode,
                // so it is parsed once and reported on every row: the OG side
                // reads it off `query-dsl/parse`'s own `:blocks?` flag and the
                // two must agree about what is being identified.
                let (parsed, _view) =
                    parse_query_text(&source, QueryDialect::Og, JournalDate::today());
                let anchor = parsed.anchor;
                for mode in CompareMode::all() {
                    let bounded =
                        run_query_bounded_in_mode(&graph, &source, mode, usize::MAX, usize::MAX);
                    let found = identities(anchor, &bounded);
                    let anchor_label = match anchor {
                        Anchor::Page => "@page",
                        Anchor::Block => "@block",
                    };
                    if found.is_empty() {
                        rows.push(format!(
                            "{label}\t{page}\t{hash}\t{}\t{anchor_label}\t-",
                            mode.label()
                        ));
                    }
                    for identity in found {
                        rows.push(format!(
                            "{label}\t{page}\t{hash}\t{}\t{anchor_label}\t{identity}",
                            mode.label()
                        ));
                    }
                }
            }
        }
        rows.sort();
        if rows.is_empty() {
            println!("# no queries in {label}");
        }
        for row in rows {
            println!("{row}");
        }
        if let Some(dir) = scratch {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
