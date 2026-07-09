use std::env;
use std::fs::{self, File};
use std::hint::black_box;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tine_core::{BlockDto, Graph, PageKind, RefGroup};

const DEFAULT_SCALES: &[usize] = &[10_000, 50_000, 100_000, 200_000];
const BLOCKS_PER_FILE: usize = 50;
const COLD_RUNS: usize = 3;
const CACHE_BUILD_RUNS: usize = 3;
const WARM_SCAN_RUNS: usize = 5;
const MEMO_HIT_RUNS: usize = 9;
const EDIT_CYCLES: usize = 12;

const PRIMARY_QUERY: &str = "(task TODO)";
const COMPOUND_QUERY: &str = "(and (task TODO DOING) #SomeTag)";
const EDIT_PAGE_NAME: &str = "Bench Page 000000";
const EDIT_SENTINEL: &str = "bench-edit-target";

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
    println!("Sheets Phase 0 query benchmark");
    println!("primary_query={PRIMARY_QUERY}");
    println!("compound_query={COMPOUND_QUERY}");
    println!("note=bare TODO is not accepted by the current simple query parser; (task TODO) is the accepted task predicate");
    println!(
        "runs=cold:{COLD_RUNS} cache_build:{CACHE_BUILD_RUNS} warm_scan:{WARM_SCAN_RUNS} memo:{MEMO_HIT_RUNS} edit_cycles:{EDIT_CYCLES}"
    );
    println!();

    let mut rows = Vec::new();
    for scale in scales {
        let root = graph_path(scale);
        let generated = generate_graph(&root, scale)?;
        println!(
            "generated scale={} path={} files={} pages={} journals={} blocks={}",
            scale,
            root.display(),
            generated.files,
            generated.pages,
            generated.journals,
            generated.blocks
        );
        let row = bench_scale(scale, &root, generated)?;
        println!(
            "result scale={} cold_total_ms={:.3} cache_build_ms={:.3} warm_scan_ms={:.3} memo_hit_us={:.3} save_page_ms={:.3}/{:.3} edit_rescan_ms={:.3}/{:.3} compound_rescan_ms={:.3}/{:.3} primary_results={} compound_results={}",
            row.scale,
            row.cold_total_ms,
            row.cache_build_ms,
            row.warm_scan_ms,
            row.memo_hit_us,
            row.save_page_ms.median,
            row.save_page_ms.p95,
            row.primary_rescan_ms.median,
            row.primary_rescan_ms.p95,
            row.compound_rescan_ms.median,
            row.compound_rescan_ms.p95,
            row.primary_results,
            row.compound_results
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
    PathBuf::from(format!("/tmp/sheets-bench-graph-{scale}"))
}

#[derive(Clone, Copy)]
struct GeneratedGraph {
    blocks: usize,
    files: usize,
    pages: usize,
    journals: usize,
}

fn generate_graph(root: &Path, target_blocks: usize) -> io::Result<GeneratedGraph> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    fs::create_dir_all(root.join("pages"))?;
    fs::create_dir_all(root.join("journals"))?;

    let file_count = target_blocks.div_ceil(BLOCKS_PER_FILE).max(1);
    let journal_count = if file_count > 1 {
        file_count.saturating_sub(1).min(5)
    } else {
        0
    };
    let page_count = file_count - journal_count;
    let page_names: Vec<String> = (0..page_count).map(page_name).collect();

    let mut rng = Lcg::new(0x5eed_cafe_d00d_f00d ^ target_blocks as u64);
    let mut written_blocks = 0usize;

    for file_idx in 0..file_count {
        let remaining = target_blocks - written_blocks;
        let blocks_in_file = remaining.min(BLOCKS_PER_FILE);
        let is_page = file_idx < page_count;
        let path = if is_page {
            root.join("pages")
                .join(format!("{}.md", page_names[file_idx]))
        } else {
            let journal_idx = file_idx - page_count;
            root.join("journals")
                .join(format!("2026_01_{:02}.md", journal_idx + 1))
        };
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        if file_idx % 7 == 0 {
            writeln!(writer, "benchmark:: sheets-phase0")?;
            writeln!(writer, "tags:: Bench, SomeTag")?;
            writeln!(writer)?;
        }

        for local_idx in 0..blocks_in_file {
            let global_idx = written_blocks + local_idx;
            let depth = depth_for(local_idx);
            let raw = block_raw(global_idx, local_idx, file_idx, &page_names, &mut rng);
            write_block(&mut writer, depth, &raw)?;
        }
        written_blocks += blocks_in_file;
    }

    Ok(GeneratedGraph {
        blocks: written_blocks,
        files: file_count,
        pages: page_count,
        journals: journal_count,
    })
}

fn page_name(i: usize) -> String {
    format!("Bench Page {i:06}")
}

fn depth_for(local_idx: usize) -> usize {
    match local_idx % 10 {
        0 | 4 | 7 => 0,
        1 | 3 | 5 | 8 => 1,
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
    match local_idx % 10 {
        0 | 5 => task_raw(global_idx, local_idx, file_idx, rng),
        1..=3 => ref_raw(global_idx, page_names, rng),
        4 => property_raw(global_idx, rng),
        _ => prose_raw(rng, 5, 40),
    }
}

fn task_raw(global_idx: usize, local_idx: usize, file_idx: usize, rng: &mut Lcg) -> String {
    let edit_target = file_idx == 0 && local_idx == 0;
    let marker = if edit_target {
        "TODO"
    } else {
        match (global_idx / 5) % 4 {
            0 => "TODO",
            1 => "DOING",
            2 => "DONE",
            _ => "LATER",
        }
    };

    let mut raw = String::new();
    raw.push_str(marker);
    raw.push(' ');
    if global_idx % 15 == 0 {
        raw.push_str("[#A] ");
    } else if global_idx % 35 == 0 {
        raw.push_str("[#B] ");
    }
    if edit_target {
        raw.push_str(EDIT_SENTINEL);
        raw.push(' ');
    }
    raw.push_str(&prose_raw(rng, 5, 14));
    if local_idx == 0 {
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

#[derive(Clone, Copy)]
struct DistributionMs {
    median: f64,
    p95: f64,
}

struct BenchRow {
    scale: usize,
    files: usize,
    pages: usize,
    journals: usize,
    cold_total_ms: f64,
    cache_build_ms: f64,
    warm_scan_ms: f64,
    memo_hit_us: f64,
    save_page_ms: DistributionMs,
    primary_rescan_ms: DistributionMs,
    compound_rescan_ms: DistributionMs,
    primary_results: usize,
    compound_results: usize,
}

fn bench_scale(scale: usize, root: &Path, generated: GeneratedGraph) -> io::Result<BenchRow> {
    let mut cold_total = Vec::with_capacity(COLD_RUNS);
    for _ in 0..COLD_RUNS {
        let graph = Graph::open(root);
        let started = Instant::now();
        let groups = graph.run_query(PRIMARY_QUERY);
        cold_total.push(started.elapsed());
        let primary_results = result_count(groups.as_ref());
        assert_nonzero(primary_results, PRIMARY_QUERY);
        black_box(primary_results);
    }

    let mut cache_build = Vec::with_capacity(CACHE_BUILD_RUNS);
    for _ in 0..CACHE_BUILD_RUNS {
        let graph = Graph::open(root);
        let started = Instant::now();
        let page_count = graph.with_pages(|pages| pages.len());
        cache_build.push(started.elapsed());
        assert_eq!(page_count, generated.files);
        black_box(page_count);
    }

    let warm_graph = Graph::open(root);
    warm_graph.with_pages(|pages| black_box(pages.len()));
    let mut warm_scan = Vec::with_capacity(WARM_SCAN_RUNS);
    for i in 0..WARM_SCAN_RUNS {
        let query = primary_query_variant(i);
        let started = Instant::now();
        let groups = warm_graph.run_query(&query);
        warm_scan.push(started.elapsed());
        assert_nonzero(result_count(groups.as_ref()), &query);
        black_box(groups.len());
    }

    let memo_graph = Graph::open(root);
    let seeded = memo_graph.run_query(PRIMARY_QUERY);
    assert_nonzero(result_count(seeded.as_ref()), PRIMARY_QUERY);
    let mut memo_hits = Vec::with_capacity(MEMO_HIT_RUNS);
    for _ in 0..MEMO_HIT_RUNS {
        let started = Instant::now();
        let groups = memo_graph.run_query(PRIMARY_QUERY);
        memo_hits.push(started.elapsed());
        black_box(groups.len());
    }

    let primary_edit = run_edit_cycles(root, PRIMARY_QUERY)?;
    let compound_edit = run_edit_cycles(root, COMPOUND_QUERY)?;

    Ok(BenchRow {
        scale,
        files: generated.files,
        pages: generated.pages,
        journals: generated.journals,
        cold_total_ms: ms(median(&cold_total)),
        cache_build_ms: ms(median(&cache_build)),
        warm_scan_ms: ms(median(&warm_scan)),
        memo_hit_us: us(median(&memo_hits)),
        save_page_ms: dist_ms(&primary_edit.save_durations),
        primary_rescan_ms: dist_ms(&primary_edit.query_durations),
        compound_rescan_ms: dist_ms(&compound_edit.query_durations),
        primary_results: primary_edit.last_result_count,
        compound_results: compound_edit.last_result_count,
    })
}

fn primary_query_variant(i: usize) -> String {
    if i == 0 {
        PRIMARY_QUERY.to_string()
    } else {
        format!("(task TODO{})", " ".repeat(i))
    }
}

struct EditCycleResult {
    save_durations: Vec<Duration>,
    query_durations: Vec<Duration>,
    last_result_count: usize,
}

fn run_edit_cycles(root: &Path, query: &str) -> io::Result<EditCycleResult> {
    let graph = Graph::open(root);
    let initial = graph.run_query(query);
    assert_nonzero(result_count(initial.as_ref()), query);

    let mut save_durations = Vec::with_capacity(EDIT_CYCLES);
    let mut query_durations = Vec::with_capacity(EDIT_CYCLES);
    let mut last_result_count = result_count(initial.as_ref());

    for _ in 0..EDIT_CYCLES {
        let mut page = graph
            .load_named(EDIT_PAGE_NAME, PageKind::Page)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, EDIT_PAGE_NAME))?;
        let base_rev = page.rev.clone();
        if !flip_edit_marker(&mut page.blocks) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "edit target block not found",
            ));
        }

        let started = Instant::now();
        let new_rev = graph.save_page(&page, base_rev.as_deref())?;
        save_durations.push(started.elapsed());
        black_box(new_rev.len());

        let started = Instant::now();
        let groups = graph.run_query(query);
        query_durations.push(started.elapsed());
        last_result_count = result_count(groups.as_ref());
        assert_nonzero(last_result_count, query);
        black_box(last_result_count);
    }

    Ok(EditCycleResult {
        save_durations,
        query_durations,
        last_result_count,
    })
}

fn flip_edit_marker(blocks: &mut [BlockDto]) -> bool {
    for block in blocks {
        if block.raw.contains(EDIT_SENTINEL) {
            if block.raw.starts_with("TODO ") {
                block.raw.replace_range(0..4, "DONE");
                return true;
            }
            if block.raw.starts_with("DONE ") {
                block.raw.replace_range(0..4, "TODO");
                return true;
            }
            return false;
        }
        if flip_edit_marker(&mut block.children) {
            return true;
        }
    }
    false
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

fn p95(durations: &[Duration]) -> Duration {
    assert!(!durations.is_empty());
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[idx]
}

fn duration_from_nanos(nanos: u128) -> Duration {
    Duration::from_nanos(nanos.min(u64::MAX as u128) as u64)
}

fn dist_ms(durations: &[Duration]) -> DistributionMs {
    DistributionMs {
        median: ms(median(durations)),
        p95: ms(p95(durations)),
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn us(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn print_table(rows: &[BenchRow]) {
    println!("| scale | files | pages | journals | cold total ms | cache-build ms | warm scan ms | memo-hit us | save_page med/p95 ms | edit re-scan med/p95 ms | compound re-scan med/p95 ms | results primary/compound |");
    println!(
        "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    );
    for row in rows {
        println!(
            "| {} | {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3}/{:.3} | {:.3}/{:.3} | {:.3}/{:.3} | {}/{} |",
            row.scale,
            row.files,
            row.pages,
            row.journals,
            row.cold_total_ms,
            row.cache_build_ms,
            row.warm_scan_ms,
            row.memo_hit_us,
            row.save_page_ms.median,
            row.save_page_ms.p95,
            row.primary_rescan_ms.median,
            row.primary_rescan_ms.p95,
            row.compound_rescan_ms.median,
            row.compound_rescan_ms.p95,
            row.primary_results,
            row.compound_results
        );
    }
}
