use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::hint::black_box;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tine_core::{PageKind, RefGroup};
use tine_store::model::Graph;

const DEFAULT_SCALES: &[usize] = &[2_000, 10_000, 20_000];
const BLOCKS_PER_PAGE: usize = 5;
const JOURNAL_COUNT: usize = 30;
const FIND_ENTRY_LOOKUPS: usize = 500;
const FIND_ENTRY_RUNS: usize = 3;
const COLD_RUNS: usize = 3;
const CACHE_BUILD_RUNS: usize = 3;
const SWITCHER_RUNS: usize = FIND_ENTRY_LOOKUPS;
const WARM_QUERY_RUNS: usize = 5;
const PUBLISH_RUNS: usize = 2;

const PRIMARY_QUERY: &str = "(task TODO)";
const COMPOUND_QUERY: &str = "(and (task TODO DOING) #SomeTag)";

const WORDS: &[&str] = &[
    "alpha", "archive", "board", "budget", "cache", "canvas", "cluster", "column", "context",
    "delta", "design", "draft", "entry", "event", "field", "filter", "focus", "graph", "grid",
    "index", "journal", "layout", "link", "marker", "memo", "metric", "note", "outline", "page",
    "panel", "phase", "priority", "project", "query", "record", "ref", "review", "row", "scan",
    "sheet", "signal", "source", "status", "sync", "tag", "task", "thread", "value", "view",
    "workflow",
];

fn main() -> io::Result<()> {
    let scales = parse_scales()?;
    println!("Graph scale benchmark");
    println!("primary_query={PRIMARY_QUERY}");
    println!("compound_query={COMPOUND_QUERY}");
    println!(
        "runs=cold:{COLD_RUNS} cache_build:{CACHE_BUILD_RUNS} find_entry:{FIND_ENTRY_RUNS} switcher:{SWITCHER_RUNS} warm_query:{WARM_QUERY_RUNS} publish:{PUBLISH_RUNS}"
    );
    println!();

    let mut rows = Vec::new();
    for scale in scales {
        let root = graph_path(scale);
        let generated = generate_graph(&root, scale)?;
        println!(
            "generated scale={} path={} pages={} nested={} journals={} blocks={}",
            scale,
            root.display(),
            generated.pages,
            generated.nested,
            generated.journals,
            generated.blocks
        );
        let row = bench_scale(scale, &root, generated)?;
        println!(
            "result scale={} cold_open_ms={:.3} cache_build_ms={:.3} find_entry_total_ms={:.3} find_entry_us={:.3} switcher_ms={:.3} warm_query_ms={:.3} publish_ms={:.3} publish_pages={}",
            row.scale,
            row.cold_open_ms,
            row.cache_build_ms,
            row.find_entry_total_ms,
            row.find_entry_per_us,
            row.switcher_ms,
            row.warm_query_ms,
            row.publish_ms,
            row.publish_pages
        );
        println!();
        rows.push(row);
    }

    print_table(&rows);
    Ok(())
}

fn parse_scales() -> io::Result<Vec<usize>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return Ok(DEFAULT_SCALES.to_vec());
    }
    let mut scales = Vec::with_capacity(args.len());
    for arg in args {
        let cleaned = arg.replace('_', "");
        let scale = cleaned.parse::<usize>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid scale argument: {arg}"),
            )
        })?;
        if scale == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "scale must be greater than zero",
            ));
        }
        scales.push(scale);
    }
    Ok(scales)
}

fn graph_path(scale: usize) -> PathBuf {
    PathBuf::from(format!("/tmp/graph-scale-bench-{scale}"))
}

#[derive(Clone)]
struct GeneratedGraph {
    blocks: usize,
    pages: usize,
    nested: usize,
    journals: usize,
    page_names: Vec<String>,
}

fn generate_graph(root: &Path, page_count: usize) -> io::Result<GeneratedGraph> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    fs::create_dir_all(root.join("pages"))?;
    fs::create_dir_all(root.join("journals"))?;
    fs::create_dir_all(root.join("logseq"))?;
    fs::write(
        root.join("logseq").join("config.edn"),
        "{:publishing/all-pages-public? true}\n",
    )?;

    let page_names: Vec<String> = (0..page_count).map(page_name).collect();
    let mut rng = Lcg::new(0x5eed_cafe_d00d_f00d ^ page_count as u64);
    let mut blocks = 0usize;
    let mut nested = 0usize;

    for (idx, name) in page_names.iter().enumerate() {
        let is_nested = is_nested_page(idx);
        if is_nested {
            nested += 1;
        }
        let path = page_path(root, name, idx, is_nested);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        if idx % 7 == 0 {
            writeln!(writer, "benchmark:: graph-scale")?;
            writeln!(writer, "tags:: Bench, SomeTag")?;
            writeln!(writer)?;
        }

        for local_idx in 0..BLOCKS_PER_PAGE {
            let global_idx = idx * BLOCKS_PER_PAGE + local_idx;
            let raw = block_raw(global_idx, local_idx, idx, &page_names, &mut rng);
            write_block(&mut writer, depth_for(local_idx), &raw)?;
            blocks += 1;
        }
    }

    for journal_idx in 0..JOURNAL_COUNT {
        let path = root
            .join("journals")
            .join(format!("2026_01_{:02}.md", journal_idx + 1));
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writeln!(writer, "benchmark:: graph-scale")?;
        if journal_idx % 3 == 0 {
            writeln!(writer, "tags:: SomeTag")?;
        }
        writeln!(writer)?;
        for local_idx in 0..BLOCKS_PER_PAGE {
            let global_idx =
                page_count * BLOCKS_PER_PAGE + journal_idx * BLOCKS_PER_PAGE + local_idx;
            let raw = block_raw(
                global_idx,
                local_idx,
                page_count + journal_idx,
                &page_names,
                &mut rng,
            );
            write_block(&mut writer, depth_for(local_idx), &raw)?;
            blocks += 1;
        }
    }

    write_dashboard(root, &page_names)?;
    blocks += 60;

    Ok(GeneratedGraph {
        blocks,
        pages: page_count + 1,
        nested,
        journals: JOURNAL_COUNT,
        page_names,
    })
}

fn page_name(i: usize) -> String {
    format!("Page {i:06}")
}

fn is_nested_page(i: usize) -> bool {
    i % 20 < 3
}

fn page_path(root: &Path, name: &str, idx: usize, nested: bool) -> PathBuf {
    if nested {
        root.join("pages")
            .join(format!("ns-{}", idx % 16))
            .join(format!("{name}.md"))
    } else {
        root.join("pages").join(format!("{name}.md"))
    }
}

fn write_dashboard(root: &Path, page_names: &[String]) -> io::Result<()> {
    let file = File::create(root.join("pages").join("Dashboard.md"))?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "benchmark:: graph-scale")?;
    writeln!(writer, "tags:: Bench, SomeTag")?;
    writeln!(writer)?;
    for _ in 0..20 {
        write_block(&mut writer, 0, &format!("{{{{query {PRIMARY_QUERY}}}}}"))?;
    }
    for _ in 0..20 {
        write_block(&mut writer, 0, &format!("{{{{query {COMPOUND_QUERY}}}}}"))?;
    }
    for i in 0..20 {
        let name = &page_names[(i * 37) % page_names.len()];
        write_block(&mut writer, 0, &format!("{{{{embed [[{name}]]}}}}"))?;
    }
    Ok(())
}

fn depth_for(local_idx: usize) -> usize {
    match local_idx {
        0 | 3 => 0,
        1 | 4 => 1,
        _ => 2,
    }
}

fn block_raw(
    global_idx: usize,
    local_idx: usize,
    file_idx: usize,
    page_names: &[String],
    rng: &mut Lcg,
) -> String {
    match local_idx {
        0 | 3 => task_raw(global_idx, local_idx, file_idx, rng),
        1 | 2 => ref_raw(global_idx, page_names, rng),
        _ => property_raw(global_idx, rng),
    }
}

fn task_raw(global_idx: usize, local_idx: usize, file_idx: usize, rng: &mut Lcg) -> String {
    let marker = match (global_idx / 5) % 4 {
        0 => "TODO",
        1 => "DOING",
        2 => "DONE",
        _ => "LATER",
    };

    let mut raw = String::new();
    raw.push_str(marker);
    raw.push(' ');
    if global_idx % 15 == 0 {
        raw.push_str("[#A] ");
    } else if global_idx % 35 == 0 {
        raw.push_str("[#B] ");
    }
    raw.push_str(&prose_raw(rng, 5, 14));
    if local_idx == 0 || file_idx % 6 == 0 {
        raw.push_str(" #SomeTag");
    } else if global_idx % 3 == 0 {
        raw.push_str(" #task-tag");
    }
    if global_idx % 4 == 0 {
        raw.push('\n');
        raw.push_str(&format!(
            "SCHEDULED: <2026-{:02}-{:02} Mon>",
            (global_idx % 12) + 1,
            (global_idx % 28) + 1
        ));
    }
    if global_idx % 17 == 0 {
        raw.push('\n');
        raw.push_str(&format!("owner:: team-{}", global_idx % 9));
    }
    raw
}

fn ref_raw(global_idx: usize, page_names: &[String], rng: &mut Lcg) -> String {
    let target = &page_names[(global_idx + rng.range(page_names.len())) % page_names.len()];
    let mut raw = prose_raw(rng, 5, 16);
    raw.push(' ');
    if global_idx % 2 == 0 {
        raw.push_str("#SomeTag");
    } else {
        raw.push_str(&format!("#topic-{}", global_idx % 23));
    }
    raw.push(' ');
    raw.push_str(&format!("[[{target}]]"));
    raw
}

fn property_raw(global_idx: usize, rng: &mut Lcg) -> String {
    format!(
        "{}\nmetric:: {}\nowner:: team-{}",
        prose_raw(rng, 5, 12),
        global_idx % 101,
        global_idx % 9
    )
}

fn prose_raw(rng: &mut Lcg, min_words: usize, max_words: usize) -> String {
    let len = min_words + rng.range(max_words - min_words + 1);
    let mut out = String::new();
    for i in 0..len {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(WORDS[rng.range(WORDS.len())]);
    }
    out
}

fn write_block(writer: &mut impl Write, depth: usize, raw: &str) -> io::Result<()> {
    let indent = "  ".repeat(depth);
    for (line_idx, line) in raw.lines().enumerate() {
        if line_idx == 0 {
            writeln!(writer, "{indent}- {line}")?;
        } else {
            writeln!(writer, "{indent}  {line}")?;
        }
    }
    Ok(())
}

struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn range(&mut self, upper: usize) -> usize {
        debug_assert!(upper > 0);
        (self.next() as usize) % upper
    }
}

struct BenchRow {
    scale: usize,
    pages: usize,
    nested: usize,
    journals: usize,
    cold_open_ms: f64,
    cache_build_ms: f64,
    find_entry_total_ms: f64,
    find_entry_per_us: f64,
    switcher_ms: f64,
    warm_query_ms: f64,
    publish_ms: f64,
    publish_pages: usize,
}

fn bench_scale(scale: usize, root: &Path, generated: GeneratedGraph) -> io::Result<BenchRow> {
    let mut cold_open = Vec::with_capacity(COLD_RUNS);
    for _ in 0..COLD_RUNS {
        let started = Instant::now();
        let graph = Graph::open(root);
        let page_count = graph.with_pages(|pages| pages.len());
        cold_open.push(started.elapsed());
        assert_eq!(page_count, generated.pages + generated.journals);
        black_box(page_count);
    }

    let mut cache_build = Vec::with_capacity(CACHE_BUILD_RUNS);
    for _ in 0..CACHE_BUILD_RUNS {
        let graph = Graph::open(root);
        let started = Instant::now();
        let page_count = graph.with_pages(|pages| pages.len());
        cache_build.push(started.elapsed());
        assert_eq!(page_count, generated.pages + generated.journals);
        black_box(page_count);
    }

    let lookup_names = lookup_sample(&generated.page_names, FIND_ENTRY_LOOKUPS);
    let (find_entry_total_ms, find_entry_per_us) = bench_find_entry(root, &lookup_names)?;
    let switcher_ms = bench_switcher(root)?;
    let warm_query_ms = bench_warm_query(root)?;
    let (publish_ms, publish_pages) = bench_publish(root)?;

    Ok(BenchRow {
        scale,
        pages: generated.pages,
        nested: generated.nested,
        journals: generated.journals,
        cold_open_ms: ms(median(&cold_open)),
        cache_build_ms: ms(median(&cache_build)),
        find_entry_total_ms,
        find_entry_per_us,
        switcher_ms,
        warm_query_ms,
        publish_ms,
        publish_pages,
    })
}

fn lookup_sample(names: &[String], requested: usize) -> Vec<String> {
    let limit = requested.min(names.len());
    let mut sample = Vec::with_capacity(limit);
    let mut seen = HashSet::with_capacity(limit);
    let mut i = 0usize;
    while sample.len() < limit {
        let idx = (i * 37) % names.len();
        if seen.insert(idx) {
            sample.push(names[idx].clone());
        }
        i += 1;
    }
    assert!(sample.iter().any(|name| {
        name.strip_prefix("Page ")
            .and_then(|s| s.parse::<usize>().ok())
            .is_some_and(is_nested_page)
    }));
    sample
}

fn bench_find_entry(root: &Path, names: &[String]) -> io::Result<(f64, f64)> {
    let mut totals = Vec::with_capacity(FIND_ENTRY_RUNS);
    let mut per_lookup = Vec::with_capacity(FIND_ENTRY_RUNS * names.len());
    for _ in 0..FIND_ENTRY_RUNS {
        let graph = Graph::open(root);
        let page_count = graph.with_pages(|pages| pages.len());
        black_box(page_count);
        let started = Instant::now();
        for name in names {
            let call_started = Instant::now();
            let page = graph
                .load_named(name, PageKind::Page)?
                .unwrap_or_else(|| panic!("generated page not found: {name}"));
            per_lookup.push(call_started.elapsed());
            black_box(page.blocks.len());
        }
        totals.push(started.elapsed());
    }
    Ok((ms(median(&totals)), us(median(&per_lookup))))
}

fn bench_switcher(root: &Path) -> io::Result<f64> {
    let graph = Graph::open(root);
    let page_count = graph.with_pages(|pages| pages.len());
    black_box(page_count);
    let mut durations = Vec::with_capacity(SWITCHER_RUNS);
    for _ in 0..SWITCHER_RUNS {
        let started = Instant::now();
        let results = tine_store::query::quick_switch(&graph, "pa", 12);
        durations.push(started.elapsed());
        assert!(!results.is_empty(), "quick_switch returned no results");
        black_box(results.len());
    }
    Ok(ms(median(&durations)))
}

fn bench_warm_query(root: &Path) -> io::Result<f64> {
    let graph = Graph::open(root);
    let page_count = graph.with_pages(|pages| pages.len());
    black_box(page_count);
    let mut durations = Vec::with_capacity(WARM_QUERY_RUNS);
    for i in 0..WARM_QUERY_RUNS {
        let query = primary_query_variant(i);
        let started = Instant::now();
        let groups = graph.run_query(&query);
        durations.push(started.elapsed());
        assert_nonzero(result_count(groups.as_ref()), &query);
        black_box(groups.len());
    }
    Ok(ms(median(&durations)))
}

fn bench_publish(root: &Path) -> io::Result<(f64, usize)> {
    let mut durations = Vec::with_capacity(PUBLISH_RUNS);
    let mut publish_pages = 0usize;
    for _ in 0..PUBLISH_RUNS {
        let graph = Graph::open(root);
        let page_count = graph.with_pages(|pages| pages.len());
        black_box(page_count);
        let started = Instant::now();
        let (out, count) = tine_store::publish::publish_graph(&graph)?;
        durations.push(started.elapsed());
        assert!(count > 0, "publish_graph returned no pages");
        publish_pages = count;
        black_box(out.len());
        black_box(count);
    }
    Ok((ms(median(&durations)), publish_pages))
}

fn primary_query_variant(i: usize) -> String {
    if i == 0 {
        PRIMARY_QUERY.to_string()
    } else {
        format!("(task TODO{})", " ".repeat(i))
    }
}

fn result_count(groups: &[RefGroup]) -> usize {
    groups.iter().map(|group| group.blocks.len()).sum()
}

fn assert_nonzero(count: usize, query: &str) {
    assert!(count > 0, "query returned no results: {query}");
}

fn median(durations: &[Duration]) -> Duration {
    assert!(!durations.is_empty());
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        duration_from_nanos((sorted[mid - 1].as_nanos() + sorted[mid].as_nanos()) / 2)
    }
}

fn duration_from_nanos(nanos: u128) -> Duration {
    Duration::from_nanos(nanos.min(u64::MAX as u128) as u64)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn us(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn print_table(rows: &[BenchRow]) {
    println!("| scale | pages | nested | journals | cold_open_ms | cache_build_ms | find_entry_K total_ms/per_us | switcher_ms | warm_query_ms | publish_ms | publish_pages |");
    println!("| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for row in rows {
        println!(
            "| {} | {} | {} | {} | {:.3} | {:.3} | {:.3}/{:.3} | {:.3} | {:.3} | {:.3} | {} |",
            row.scale,
            row.pages,
            row.nested,
            row.journals,
            row.cold_open_ms,
            row.cache_build_ms,
            row.find_entry_total_ms,
            row.find_entry_per_us,
            row.switcher_ms,
            row.warm_query_ms,
            row.publish_ms,
            row.publish_pages
        );
    }
}
