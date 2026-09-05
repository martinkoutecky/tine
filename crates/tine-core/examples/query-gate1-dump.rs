//! Gate 1's walk side (SPEC §8.1, dossier B6/D15).
//!
//! For every `{{query …}}` / `{{tine-query …}}` in the given graphs, run the
//! walk under each of the five §8.1 counterfactual modes and print one row per
//! (query, mode):
//!
//! ```text
//! <graph>\t<page>\t<query-hash>\t<mode>\t<total>
//! ```
//!
//! and, to the file named by `--queries <path>`, the query TEXT keyed by the
//! same hash, so the OG side can run the identical string. The text file is the
//! ONLY place a query's bytes appear; stdout carries hashes and counts, so the
//! output of a private graph can be pasted into a receipt.
//!
//! Usage:
//! ```text
//! cargo run -q -p tine-core --example query-gate1-dump -- \
//!     --queries /tmp/gate1-queries.tsv [--query-list <file>] <graph-dir>…
//! ```
//!
//! `--query-list <file>` replaces the macro scan with a fixed list of queries,
//! one per line (`#` comments and blank lines skipped), run against every named
//! graph. It exists for SPEC §8.3's `case.cljs` twin: those twelve queries have
//! to run against `fixture-case-graph` unchanged, and writing them into the
//! graph as `{{query …}}` macros would add blocks — and, for `[[book]]`, a
//! page REFERENCE — that change the very answers the fixture pins.
//!
//! A graph directory that does not exist is reported as `# skipped <label>` on
//! stdout rather than failing, so the same command line works with and without
//! the anonymized graph.

use std::io::Write;
use std::path::PathBuf;

use tine_core::query::atom::CompareMode;
use tine_core::query::run_query_bounded_in_mode;
use tine_core::Graph;

#[path = "support/query_corpus.rs"]
mod query_corpus;

use query_corpus::{fnv1a, graph_files, macro_args, materialize_single_file};

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
                for mode in CompareMode::all() {
                    let bounded =
                        run_query_bounded_in_mode(&graph, &source, mode, usize::MAX, usize::MAX);
                    let total: usize = bounded.groups.iter().map(|group| group.blocks.len()).sum();
                    rows.push(format!(
                        "{label}\t{page}\t{hash}\t{}\t{total}",
                        mode.label()
                    ));
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
