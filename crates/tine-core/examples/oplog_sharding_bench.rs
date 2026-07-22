//! Deterministic sparse-oplog layout benchmark.
//!
//! The public process is only an orchestrator. Each selected layout runs in a
//! fresh child so its peak RSS is not inherited from another layout. Synthetic
//! encoded objects are written below `/tmp`, then read back for hot-working-set
//! and sequential one-object-at-a-time Loro scan/import measurements.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{Duration, Instant};

use loro::{
    Container, ExportMode, LoroDoc, LoroMap, LoroTree, LoroValue, TreeID, UpdateOptions,
    ValueOrContainer, VersionVector,
};

type BenchResult<T> = Result<T, Box<dyn Error>>;

const DEFAULT_PAGES: usize = 10_000;
const DEFAULT_BLOCKS: usize = 1_000_000;
const WORKING_SET_PAGES: usize = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    GraphWide,
    CatalogOwner,
    HomeOwner,
    HomeFanout,
}

impl Layout {
    const ALL: [Self; 4] = [
        Self::GraphWide,
        Self::CatalogOwner,
        Self::HomeOwner,
        Self::HomeFanout,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::GraphWide => "graph-wide",
            Self::CatalogOwner => "catalog-owner",
            Self::HomeOwner => "home-owner",
            Self::HomeFanout => "home-fanout",
        }
    }

    fn parse(value: &str) -> BenchResult<Vec<Self>> {
        Ok(match value {
            "all" => Self::ALL.to_vec(),
            "graph-wide" => vec![Self::GraphWide],
            "catalog-owner" => vec![Self::CatalogOwner],
            "home-owner" => vec![Self::HomeOwner],
            "home-fanout" => vec![Self::HomeFanout],
            _ => return Err(format!("unknown layout {value:?}").into()),
        })
    }
}

#[derive(Debug)]
struct Config {
    pages: usize,
    blocks: usize,
    layouts: Vec<Layout>,
    rename_referrers: usize,
    child: bool,
}

#[derive(Default)]
struct BuildMetrics {
    mutate: Duration,
    encode: Duration,
    write: Duration,
    encoded_bytes: u64,
    objects: usize,
}

#[derive(Default)]
struct SequentialImportMetrics {
    read: Duration,
    import: Duration,
    objects: usize,
}

struct RunMetrics {
    build: BuildMetrics,
    cold_open: Duration,
    cold_open_rss_kib: Option<u64>,
    edit: TimedUpdate,
    cross_page: TimedUpdate,
    working_set: Duration,
    working_set_pages: usize,
    working_set_rss_kib: Option<u64>,
    sequential_import: SequentialImportMetrics,
    cold_materialization: Option<MaterializationMetrics>,
    working_materialization: Option<MaterializationMetrics>,
    rename: Option<RenameMetrics>,
    convergence: &'static str,
}

struct TimedUpdate {
    wall: Duration,
    bytes: usize,
}

struct MaterializationMetrics {
    fanout: usize,
    unique_home_shards: usize,
    blocks: usize,
    current_rss_kib: Option<u64>,
    peak_rss_kib: Option<u64>,
}

struct RenameMetrics {
    update: TimedUpdate,
    referrer_shards: usize,
    affected_documents: usize,
}

struct TempDirGuard {
    path: PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("failed to clean {}: {error}", self.path.display());
            }
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("oplog_sharding_bench: {error}");
        process::exit(1);
    }
}

fn run() -> BenchResult<()> {
    let config = parse_args()?;
    if !config.child {
        println!(
            "oplog_sharding_bench pages={} blocks={} working_set_pages={} isolation=one-child-per-layout",
            config.pages,
            config.blocks,
            config.pages.min(WORKING_SET_PAGES),
        );
        let executable = env::current_exe()?;
        for layout in config.layouts {
            remove_stale_temp_dirs()?;
            let mut child = Command::new(&executable)
                .args([
                    "--child",
                    "--layout",
                    layout.name(),
                    "--pages",
                    &config.pages.to_string(),
                    "--blocks",
                    &config.blocks.to_string(),
                    "--rename-referrers",
                    &config.rename_referrers.to_string(),
                ])
                .spawn()?;
            let child_pid = child.id();
            let status = child.wait()?;
            if !status.success() {
                let child_root = temp_root(child_pid, layout, config.pages, config.blocks);
                if let Err(error) = fs::remove_dir_all(&child_root) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        eprintln!("parent failed to clean {}: {error}", child_root.display());
                    }
                }
                return Err(format!("{} child exited with {status}", layout.name()).into());
            }
        }
        return Ok(());
    }

    let layout = *config
        .layouts
        .first()
        .ok_or("child requires exactly one layout")?;
    if config.layouts.len() != 1 {
        return Err("child requires exactly one layout".into());
    }
    run_child(layout, config.pages, config.blocks, config.rename_referrers)
}

fn parse_args() -> BenchResult<Config> {
    let mut pages = DEFAULT_PAGES;
    let mut blocks = DEFAULT_BLOCKS;
    let mut layouts = Layout::ALL.to_vec();
    let mut rename_referrers = 32;
    let mut child = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pages" => pages = parse_count("pages", args.next())?,
            "--blocks" => blocks = parse_count("blocks", args.next())?,
            "--layout" => layouts = Layout::parse(&args.next().ok_or("--layout needs a value")?)?,
            "--rename-referrers" => {
                rename_referrers = parse_count("rename-referrers", args.next())?
            }
            "--child" => child = true,
            "--help" | "-h" => {
                println!("usage: oplog_sharding_bench [--pages N] [--blocks N] [--rename-referrers N] [--layout all|graph-wide|catalog-owner|home-owner|home-fanout]");
                process::exit(0);
            }
            _ => return Err(format!("unknown argument {arg:?}").into()),
        }
    }
    if pages < 2 || blocks < 2 {
        return Err("pages and blocks must both be at least 2".into());
    }
    Ok(Config {
        pages,
        blocks,
        layouts,
        rename_referrers,
        child,
    })
}

fn parse_count(name: &str, value: Option<String>) -> BenchResult<usize> {
    let raw = value.ok_or_else(|| format!("--{name} needs a value"))?;
    let parsed = raw.replace('_', "").parse::<usize>()?;
    if parsed == 0 {
        return Err(format!("--{name} must be positive").into());
    }
    Ok(parsed)
}

fn run_child(
    layout: Layout,
    pages: usize,
    blocks: usize,
    rename_referrers: usize,
) -> BenchResult<()> {
    let root = temp_root(process::id(), layout, pages, blocks);
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;
    let _cleanup = TempDirGuard { path: root.clone() };

    let result = match layout {
        Layout::GraphWide => bench_graph_wide(&root, pages, blocks),
        Layout::CatalogOwner => bench_catalog_owner(&root, pages, blocks, rename_referrers),
        Layout::HomeOwner => bench_home_owner(&root, pages, blocks, rename_referrers, false),
        Layout::HomeFanout => bench_home_owner(&root, pages, blocks, rename_referrers, true),
    };

    match result {
        Ok(metrics) => {
            let (current_rss, peak_rss) = rss_kib();
            println!(
                "result layout={} pages={} blocks={} objects={} build_ms={:.3} encode_ms={:.3} object_write_ms={:.3} encoded_bytes={} cold_open_ms={:.3} cold_open_rss_kib={} cold_open_peak_rss_kib={} cold_fanout={} cold_unique_home_shards={} cold_materialized_blocks={} one_page_edit_ms={:.3} one_page_update_bytes={} cross_page_ms={:.3} cross_page_update_bytes={} rename_referrer_shards={} rename_affected_documents={} rename_ms={} rename_update_bytes={} working_set_pages={} working_set_ms={:.3} working_set_rss_kib={} working_set_peak_rss_kib={} working_unique_home_shards={} working_materialized_blocks={} sequential_read_ms={:.3} sequential_one_doc_import_ms={:.3} current_rss_kib={} peak_rss_kib={} convergence={}",
                layout.name(), pages, blocks, metrics.build.objects,
                ms(metrics.build.mutate), ms(metrics.build.encode), ms(metrics.build.write), metrics.build.encoded_bytes,
                ms(metrics.cold_open), display_rss(metrics.cold_open_rss_kib),
                display_optional_materialization(&metrics.cold_materialization, |value| value.peak_rss_kib),
                display_optional_usize(&metrics.cold_materialization, |value| value.fanout),
                display_optional_usize(&metrics.cold_materialization, |value| value.unique_home_shards),
                display_optional_usize(&metrics.cold_materialization, |value| value.blocks),
                ms(metrics.edit.wall), metrics.edit.bytes,
                ms(metrics.cross_page.wall), metrics.cross_page.bytes,
                metrics.rename.as_ref().map_or_else(|| "unsupported".to_string(), |value| value.referrer_shards.to_string()),
                metrics.rename.as_ref().map_or_else(|| "unsupported".to_string(), |value| value.affected_documents.to_string()),
                metrics.rename.as_ref().map_or_else(|| "unsupported".to_string(), |value| format!("{:.3}", ms(value.update.wall))),
                metrics.rename.as_ref().map_or_else(|| "unsupported".to_string(), |value| value.update.bytes.to_string()),
                metrics.working_set_pages, ms(metrics.working_set), display_rss(metrics.working_set_rss_kib),
                display_optional_materialization(&metrics.working_materialization, |value| value.peak_rss_kib),
                display_optional_usize(&metrics.working_materialization, |value| value.unique_home_shards),
                display_optional_usize(&metrics.working_materialization, |value| value.blocks),
                ms(metrics.sequential_import.read), ms(metrics.sequential_import.import),
                display_rss(current_rss), display_rss(peak_rss), metrics.convergence
            );
            Ok(())
        }
        Err(error) => {
            let (_, peak_rss) = rss_kib();
            eprintln!(
                "failed layout={} pages={} blocks={} peak_rss_kib={}: {error}",
                layout.name(),
                pages,
                blocks,
                display_rss(peak_rss)
            );
            Err(error)
        }
    }
}

fn bench_graph_wide(root: &Path, pages: usize, blocks: usize) -> BenchResult<RunMetrics> {
    let path = root.join("graph.loro");
    let mut build = BuildMetrics::default();
    let started = Instant::now();
    let doc = new_doc(1)?;
    let tree = doc.get_tree("graph");
    tree.enable_fractional_index(0);
    for page in 0..pages {
        let page_node = tree.create(None)?;
        let meta = tree.get_meta(page_node)?;
        meta.insert("node_type", "page")?;
        meta.insert("page_id", page_id(page))?;
        meta.insert("path", page_path(page))?;
        for block in blocks_for_page(page, pages, blocks) {
            let node = tree.create(page_node)?;
            populate_block(&tree, node, block, page)?;
        }
    }
    build.mutate += started.elapsed();
    write_doc(&path, &doc, &mut build)?;
    drop(doc);

    let cold_started = Instant::now();
    let cold_doc = read_doc(&path, 100)?;
    let cold_open = cold_started.elapsed();
    let cold_open_rss_kib = rss_kib().0;
    drop(cold_doc);
    let edit = graph_edit(&path)?;
    let cross_page = graph_move(&path)?;
    let working_started = Instant::now();
    let working_doc = read_doc(&path, 101)?;
    let working_set = working_started.elapsed();
    let working_set_rss_kib = rss_kib().0;
    drop(working_doc);
    let sequential_import = sequential_import_paths(std::slice::from_ref(&path))?;

    Ok(RunMetrics {
        build,
        cold_open,
        cold_open_rss_kib,
        edit,
        cross_page,
        working_set,
        working_set_pages: pages.min(WORKING_SET_PAGES),
        working_set_rss_kib,
        sequential_import,
        cold_materialization: None,
        working_materialization: None,
        rename: None,
        convergence: "single-doc",
    })
}

fn bench_catalog_owner(
    root: &Path,
    pages: usize,
    blocks: usize,
    rename_referrers: usize,
) -> BenchResult<RunMetrics> {
    let catalog_path = root.join("catalog.loro");
    let page_dir = root.join("pages");
    fs::create_dir_all(&page_dir)?;
    let mut build = BuildMetrics::default();

    let started = Instant::now();
    let catalog = new_doc(1)?;
    let paths = catalog.get_map("paths");
    let owners = catalog.get_map("owners");
    for page in 0..pages {
        paths.insert(&page_id(page), page_path(page))?;
    }
    for block in 0..blocks {
        owners.insert(&block_id(block), page_id(block % pages))?;
    }
    build.mutate += started.elapsed();
    write_doc(&catalog_path, &catalog, &mut build)?;
    drop(catalog);

    for page in 0..pages {
        let started = Instant::now();
        let doc = new_doc(peer_for_page(page))?;
        let tree = doc.get_tree("blocks");
        tree.enable_fractional_index(0);
        for block in blocks_for_page(page, pages, blocks) {
            let node = tree.create(None)?;
            populate_block(&tree, node, block, page)?;
        }
        build.mutate += started.elapsed();
        write_doc(&page_doc_path(&page_dir, page), &doc, &mut build)?;
    }

    let cold_started = Instant::now();
    let cold_docs = [
        read_doc(&catalog_path, 200)?,
        read_doc(&page_doc_path(&page_dir, 0), 201)?,
    ];
    let cold_open = cold_started.elapsed();
    let cold_open_rss_kib = rss_kib().0;
    drop(cold_docs);
    let edit = catalog_edit(&page_dir)?;
    let cross_page = catalog_move(&catalog_path, &page_dir)?;
    let rename = affected_only_rename(&catalog_path, &page_dir, pages, rename_referrers)?;
    let working_pages = pages.min(WORKING_SET_PAGES);
    let working_started = Instant::now();
    let mut loaded = Vec::with_capacity(working_pages + 1);
    loaded.push(read_doc(&catalog_path, 201)?);
    for page in 0..working_pages {
        loaded.push(read_doc(
            &page_doc_path(&page_dir, page),
            peer_for_page(page) + 20_000,
        )?);
    }
    let working_set = working_started.elapsed();
    let working_set_rss_kib = rss_kib().0;
    drop(loaded);
    let sequential_import = sequential_import_tree(&catalog_path, &page_dir, pages)?;

    Ok(RunMetrics {
        build,
        cold_open,
        cold_open_rss_kib,
        edit,
        cross_page,
        working_set,
        working_set_pages: working_pages,
        working_set_rss_kib,
        sequential_import,
        cold_materialization: None,
        working_materialization: None,
        rename: Some(rename),
        convergence: "owner-lww-only",
    })
}

fn bench_home_owner(
    root: &Path,
    pages: usize,
    blocks: usize,
    rename_referrers: usize,
    dispersed: bool,
) -> BenchResult<RunMetrics> {
    ownership_convergence_probe()?;
    let catalog_path = root.join("catalog.loro");
    let page_dir = root.join("pages");
    fs::create_dir_all(&page_dir)?;
    let mut build = BuildMetrics::default();

    let started = Instant::now();
    let catalog = new_doc(1)?;
    let paths = catalog.get_map("paths");
    for page in 0..pages {
        paths.insert(&page_id(page), page_path(page))?;
    }
    build.mutate += started.elapsed();
    write_doc(&catalog_path, &catalog, &mut build)?;

    for page in 0..pages {
        let started = Instant::now();
        let doc = new_doc(peer_for_page(page))?;
        let members = doc.get_map("members");
        let entities = doc.get_tree("entities");
        entities.enable_fractional_index(0);
        for block in blocks_for_page(page, pages, blocks) {
            let node = entities.create(None)?;
            populate_block(&entities, node, block, page)?;
            entities
                .get_meta(node)?
                .insert("owner", page_id(current_page(block, pages, dispersed)))?;
        }
        for block in current_blocks_for_page(page, pages, blocks, dispersed) {
            let order = block / pages;
            let home = block % pages;
            members.insert(&block_id(block), membership_value(order, home))?;
        }
        build.mutate += started.elapsed();
        write_doc(&page_doc_path(&page_dir, page), &doc, &mut build)?;
    }

    let cold_started = Instant::now();
    let cold_materialization = materialize_home_pages(&catalog_path, &page_dir, &[0])?;
    let cold_open = cold_started.elapsed();
    let cold_open_rss_kib = cold_materialization.current_rss_kib;
    let edit = home_edit(&page_dir)?;
    let cross_page = home_move(&page_dir)?;
    let rename = affected_only_rename(&catalog_path, &page_dir, pages, rename_referrers)?;
    let working_pages = pages.min(WORKING_SET_PAGES);
    let working_started = Instant::now();
    let requested_pages: Vec<usize> = (0..working_pages).collect();
    let working_materialization =
        materialize_home_pages(&catalog_path, &page_dir, &requested_pages)?;
    let working_set = working_started.elapsed();
    let working_set_rss_kib = working_materialization.current_rss_kib;
    let sequential_import = sequential_import_tree(&catalog_path, &page_dir, pages)?;

    Ok(RunMetrics {
        build,
        cold_open,
        cold_open_rss_kib,
        edit,
        cross_page,
        working_set,
        working_set_pages: working_pages,
        working_set_rss_kib,
        sequential_import,
        cold_materialization: Some(cold_materialization),
        working_materialization: Some(working_materialization),
        rename: Some(rename),
        convergence: "move-move+move-edit+move-delete+delete-edit-pass",
    })
}

fn graph_edit(path: &Path) -> BenchResult<TimedUpdate> {
    let doc = read_doc(path, 401)?;
    let tree = doc.get_tree("graph");
    let page = tree.roots().first().copied().ok_or("missing page")?;
    let block = tree
        .children(page)
        .and_then(|v| v.first().copied())
        .ok_or("missing block")?;
    timed_update(&[&doc], || {
        update_raw(&tree, block, "edited synthetic block")
    })
}

fn graph_move(path: &Path) -> BenchResult<TimedUpdate> {
    let doc = read_doc(path, 402)?;
    let tree = doc.get_tree("graph");
    let roots = tree.roots();
    let source = roots.first().copied().ok_or("missing source")?;
    let destination = roots.get(1).copied().ok_or("missing destination")?;
    let block = tree
        .children(source)
        .and_then(|v| v.first().copied())
        .ok_or("missing block")?;
    timed_update(&[&doc], || Ok(tree.mov(block, destination)?))
}

fn catalog_edit(page_dir: &Path) -> BenchResult<TimedUpdate> {
    let doc = read_doc(&page_doc_path(page_dir, 0), 501)?;
    let tree = doc.get_tree("blocks");
    let block = find_block(&tree, &block_id(0))?;
    timed_update(&[&doc], || {
        update_raw(&tree, block, "edited synthetic block")
    })
}

fn catalog_move(catalog_path: &Path, page_dir: &Path) -> BenchResult<TimedUpdate> {
    let catalog = read_doc(catalog_path, 502)?;
    let source = read_doc(&page_doc_path(page_dir, 0), 503)?;
    let destination = read_doc(&page_doc_path(page_dir, 1), 504)?;
    let source_tree = source.get_tree("blocks");
    let destination_tree = destination.get_tree("blocks");
    let block = find_block(&source_tree, &block_id(0))?;
    timed_update(&[&catalog, &source, &destination], || {
        source_tree.delete(block)?;
        let new_block = destination_tree.create(None)?;
        populate_block(&destination_tree, new_block, 0, 1)?;
        catalog.get_map("owners").insert(&block_id(0), page_id(1))?;
        Ok(())
    })
}

fn home_edit(page_dir: &Path) -> BenchResult<TimedUpdate> {
    let doc = read_doc(&page_doc_path(page_dir, 0), 601)?;
    let tree = doc.get_tree("entities");
    let block = find_block(&tree, &block_id(0))?;
    timed_update(&[&doc], || {
        update_raw(&tree, block, "edited synthetic block")
    })
}

fn home_move(page_dir: &Path) -> BenchResult<TimedUpdate> {
    let source = read_doc(&page_doc_path(page_dir, 0), 602)?;
    let destination = read_doc(&page_doc_path(page_dir, 1), 603)?;
    let entities = source.get_tree("entities");
    let block = find_block(&entities, &block_id(0))?;
    timed_update(&[&source, &destination], || {
        source.get_map("members").delete(&block_id(0))?;
        destination
            .get_map("members")
            .insert(&block_id(0), membership_value(0, 0))?;
        entities.get_meta(block)?.insert("owner", page_id(1))?;
        Ok(())
    })
}

fn affected_only_rename(
    catalog_path: &Path,
    page_dir: &Path,
    pages: usize,
    requested_referrers: usize,
) -> BenchResult<RenameMetrics> {
    let catalog = read_doc(catalog_path, 610)?;
    let mut selected = BTreeSet::new();
    let wanted = requested_referrers.min(pages.saturating_sub(1));
    let mut sequence = 0usize;
    while selected.len() < wanted {
        let candidate = (sequence.wrapping_mul(7_919) + 1) % pages;
        if candidate != 0 {
            selected.insert(candidate);
        }
        sequence += 1;
    }
    let referrers: Vec<usize> = selected.into_iter().collect();
    let mut documents = Vec::with_capacity(referrers.len());
    for page in &referrers {
        documents.push(read_doc(
            &page_doc_path(page_dir, *page),
            peer_for_page(*page) + 40_000,
        )?);
    }
    let mut update_docs = Vec::with_capacity(documents.len() + 1);
    update_docs.push(&catalog);
    update_docs.extend(documents.iter());
    let update = timed_update(&update_docs, || {
        let new_path = "pages/Renamed Page 00000000.md";
        catalog.get_map("paths").insert(&page_id(0), new_path)?;
        for document in &documents {
            document
                .get_map("synthetic_rename_referrers")
                .insert(&page_id(0), new_path)?;
        }
        Ok(())
    })?;
    Ok(RenameMetrics {
        update,
        referrer_shards: referrers.len(),
        affected_documents: referrers.len() + 1,
    })
}

#[derive(Clone)]
struct MembershipClaim {
    block: String,
    current_page: usize,
}

fn materialize_home_pages(
    catalog_path: &Path,
    page_dir: &Path,
    requested_pages: &[usize],
) -> BenchResult<MaterializationMetrics> {
    let catalog = read_doc(catalog_path, 620)?;
    let paths = catalog.get_map("paths");
    let mut claims_by_home: BTreeMap<usize, Vec<MembershipClaim>> = BTreeMap::new();
    let mut fanout = 0usize;

    for current_page in requested_pages {
        let expected_path = page_path(*current_page);
        if map_string(&paths, &page_id(*current_page))? != expected_path {
            return Err(format!("catalog path mismatch for page {current_page}").into());
        }
        let current = read_doc(
            &page_doc_path(page_dir, *current_page),
            peer_for_page(*current_page) + 50_000,
        )?;
        let members = current.get_map("members");
        let keys: Vec<String> = members.keys().map(|key| key.to_string()).collect();
        let mut homes = BTreeSet::new();
        for block in keys {
            let value = map_string(&members, &block)?;
            let (_, home_id) = value
                .split_once(':')
                .ok_or_else(|| format!("malformed membership for {block}"))?;
            let home = parse_page_id(home_id)?;
            homes.insert(home);
            claims_by_home
                .entry(home)
                .or_default()
                .push(MembershipClaim {
                    block,
                    current_page: *current_page,
                });
        }
        fanout = fanout.max(homes.len());
        drop(current);
    }
    drop(catalog);

    let unique_home_shards = claims_by_home.len();
    let mut materialized = Vec::new();
    for (home, claims) in claims_by_home {
        let home_doc = read_doc(&page_doc_path(page_dir, home), peer_for_page(home) + 60_000)?;
        let entities = home_doc.get_tree("entities");
        for claim in claims {
            let entity = find_block(&entities, &claim.block)?;
            let meta = entities.get_meta(entity)?;
            let owner = map_string(&meta, "owner")?;
            if owner != "tombstone" {
                parse_page_id(&owner)?;
            }
            if owner == page_id(claim.current_page) {
                materialized.push(map_text(&meta, "raw")?);
            } else if owner != "tombstone" {
                // A losing membership is intentionally invisible. The owner register
                // remains the sole authority, so no content is extracted for it.
            }
        }
        drop(home_doc);
    }
    let blocks = materialized.len();
    std::hint::black_box(&materialized);
    let (current_rss_kib, peak_rss_kib) = rss_kib();
    Ok(MaterializationMetrics {
        fanout,
        unique_home_shards,
        blocks,
        current_rss_kib,
        peak_rss_kib,
    })
}

fn timed_update<F>(docs: &[&LoroDoc], mutation: F) -> BenchResult<TimedUpdate>
where
    F: FnOnce() -> BenchResult<()>,
{
    let frontiers: Vec<VersionVector> = docs.iter().map(|doc| doc.oplog_vv()).collect();
    let started = Instant::now();
    mutation()?;
    let mut bytes = 0;
    for (doc, frontier) in docs.iter().zip(frontiers.iter()) {
        bytes += doc.export(ExportMode::updates(frontier))?.len();
    }
    Ok(TimedUpdate {
        wall: started.elapsed(),
        bytes,
    })
}

fn ownership_convergence_probe() -> BenchResult<()> {
    let base = probe_base()?;

    let move_a = probe_move_batch(&base, 701, 0, 1)?;
    let move_b = probe_move_batch(&base, 702, 0, 2)?;
    let move_move = assert_probe_batch_orders(&base, &move_a, &move_b, "move/move")?;
    if move_move.owner != page_id(1) && move_move.owner != page_id(2) {
        return Err("move/move did not select a destination owner".into());
    }
    let claims = probe_claim_pages(&merge_probe_batches(&base, &[&move_a, &move_b])?)?;
    if !claims.contains(&page_id(1)) || !claims.contains(&page_id(2)) {
        return Err("move/move did not retain both destination membership claims".into());
    }
    if move_move.visible.len() != 1 || move_move.visible[0].0 != move_move.owner {
        return Err("move/move did not hide losing membership claims".into());
    }

    let moved = probe_move_batch(&base, 711, 0, 1)?;
    let edited = probe_edit_batch(&base, 712, "concurrent edit")?;
    let move_edit = assert_probe_batch_orders(&base, &moved, &edited, "move/edit")?;
    if move_edit.owner != page_id(1)
        || move_edit.recoverable_raw != "concurrent edit"
        || move_edit.visible != vec![(page_id(1), "concurrent edit".to_string())]
    {
        return Err("move/edit did not retain the edit at the winning destination".into());
    }

    let moved = probe_move_batch(&base, 721, 0, 1)?;
    let deleted = probe_delete_batch(&base, 722, 0)?;
    let move_delete = assert_probe_batch_orders(&base, &moved, &deleted, "move/delete")?;
    if move_delete.owner != "tombstone" || !move_delete.visible.is_empty() {
        return Err("move/delete tombstone winner remained visible".into());
    }

    let deleted = probe_delete_batch(&base, 731, 0)?;
    let edited = probe_edit_batch(&base, 732, "recoverable concurrent edit")?;
    let delete_edit = assert_probe_batch_orders(&base, &deleted, &edited, "delete/edit")?;
    if delete_edit.owner != "tombstone"
        || delete_edit.recoverable_raw != "recoverable concurrent edit"
        || !delete_edit.visible.is_empty()
    {
        return Err("delete/edit did not retain hidden recoverable content".into());
    }
    Ok(())
}

struct ProbeBatch {
    objects: Vec<(usize, Vec<u8>)>,
}

#[derive(Debug, Eq, PartialEq)]
struct ProbeCanonical {
    owner: String,
    recoverable_raw: String,
    visible: Vec<(String, String)>,
}

fn probe_base() -> BenchResult<Vec<Vec<u8>>> {
    let home = new_doc(680)?;
    let entities = home.get_tree("entities");
    let entity = entities.create(None)?;
    populate_block(&entities, entity, 0, 0)?;
    entities.get_meta(entity)?.insert("owner", page_id(0))?;
    let mut base = vec![home.export(ExportMode::all_updates())?];
    for page in 0..3 {
        let document = new_doc(681 + page as u64)?;
        if page == 0 {
            document
                .get_map("members")
                .insert(&block_id(0), membership_value(0, 0))?;
        }
        base.push(document.export(ExportMode::all_updates())?);
    }
    Ok(base)
}

fn probe_replica(base: &[Vec<u8>], peer: u64) -> BenchResult<Vec<LoroDoc>> {
    base.iter()
        .map(|bytes| {
            let document = LoroDoc::new();
            document.import(bytes)?;
            document.set_peer_id(peer)?;
            Ok(document)
        })
        .collect()
}

fn make_probe_batch<F>(
    base: &[Vec<u8>],
    peer: u64,
    touched: &[usize],
    mutation: F,
) -> BenchResult<ProbeBatch>
where
    F: FnOnce(&[LoroDoc]) -> BenchResult<()>,
{
    let documents = probe_replica(base, peer)?;
    let frontiers: Vec<VersionVector> = documents.iter().map(LoroDoc::oplog_vv).collect();
    mutation(&documents)?;
    let mut objects = Vec::with_capacity(touched.len());
    for index in touched {
        objects.push((
            *index,
            documents[*index].export(ExportMode::updates(&frontiers[*index]))?,
        ));
    }
    Ok(ProbeBatch { objects })
}

fn probe_move_batch(
    base: &[Vec<u8>],
    peer: u64,
    source: usize,
    destination: usize,
) -> BenchResult<ProbeBatch> {
    make_probe_batch(base, peer, &[source + 1, destination + 1, 0], |docs| {
        docs[source + 1].get_map("members").delete(&block_id(0))?;
        docs[destination + 1]
            .get_map("members")
            .insert(&block_id(0), membership_value(0, 0))?;
        set_probe_owner(&docs[0], &page_id(destination))
    })
}

fn probe_delete_batch(base: &[Vec<u8>], peer: u64, source: usize) -> BenchResult<ProbeBatch> {
    make_probe_batch(base, peer, &[source + 1, 0], |docs| {
        docs[source + 1].get_map("members").delete(&block_id(0))?;
        set_probe_owner(&docs[0], "tombstone")
    })
}

fn probe_edit_batch(base: &[Vec<u8>], peer: u64, raw: &str) -> BenchResult<ProbeBatch> {
    make_probe_batch(base, peer, &[0], |docs| {
        let entities = docs[0].get_tree("entities");
        let entity = find_block(&entities, &block_id(0))?;
        update_raw(&entities, entity, raw)
    })
}

fn merge_probe_batches(base: &[Vec<u8>], batches: &[&ProbeBatch]) -> BenchResult<Vec<LoroDoc>> {
    let documents = probe_replica(base, 799)?;
    for batch in batches {
        for (document, update) in &batch.objects {
            documents[*document].import(update)?;
        }
    }
    Ok(documents)
}

fn assert_probe_batch_orders(
    base: &[Vec<u8>],
    a: &ProbeBatch,
    b: &ProbeBatch,
    label: &str,
) -> BenchResult<ProbeCanonical> {
    let ab = canonical_probe_state(&merge_probe_batches(base, &[a, b])?)?;
    let ba = canonical_probe_state(&merge_probe_batches(base, &[b, a])?)?;
    if ab != ba {
        return Err(
            format!("{label} complete batch sets did not converge: {ab:?} != {ba:?}").into(),
        );
    }
    Ok(ab)
}

fn set_probe_owner(doc: &LoroDoc, owner: &str) -> BenchResult<()> {
    let tree = doc.get_tree("entities");
    let block = find_block(&tree, &block_id(0))?;
    tree.get_meta(block)?.insert("owner", owner)?;
    Ok(())
}

fn canonical_probe_state(documents: &[LoroDoc]) -> BenchResult<ProbeCanonical> {
    let entities = documents[0].get_tree("entities");
    let entity = find_block(&entities, &block_id(0))?;
    let meta = entities.get_meta(entity)?;
    let owner = map_string(&meta, "owner")?;
    let recoverable_raw = map_text(&meta, "raw")?;
    let mut visible = Vec::new();
    if owner != "tombstone" {
        for page in 0..3 {
            let members = documents[page + 1].get_map("members");
            if let Some(ValueOrContainer::Value(LoroValue::String(value))) =
                members.get(&block_id(0))
            {
                let (_, home) = value
                    .split_once(':')
                    .ok_or("probe membership is malformed")?;
                if home != page_id(0) {
                    return Err("probe membership references the wrong home shard".into());
                }
                if owner == page_id(page) {
                    visible.push((page_id(page), recoverable_raw.clone()));
                }
            }
        }
    }
    Ok(ProbeCanonical {
        owner,
        recoverable_raw,
        visible,
    })
}

fn probe_claim_pages(documents: &[LoroDoc]) -> BenchResult<Vec<String>> {
    let mut pages = Vec::new();
    for page in 0..3 {
        if documents[page + 1]
            .get_map("members")
            .get(&block_id(0))
            .is_some()
        {
            pages.push(page_id(page));
        }
    }
    Ok(pages)
}

fn populate_block(tree: &LoroTree, node: TreeID, block: usize, page: usize) -> BenchResult<()> {
    let meta = tree.get_meta(node)?;
    meta.insert("node_type", "block")?;
    meta.insert("block_id", block_id(block))?;
    let raw = meta.ensure_mergeable_text("raw")?;
    raw.insert(0, &block_raw(block, page))?;
    Ok(())
}

fn update_raw(tree: &LoroTree, node: TreeID, value: &str) -> BenchResult<()> {
    let meta = tree.get_meta(node)?;
    let raw = match meta.get("raw") {
        Some(ValueOrContainer::Container(Container::Text(text))) => text,
        _ => return Err("missing raw text".into()),
    };
    raw.update(value, UpdateOptions::default())?;
    Ok(())
}

fn find_block(tree: &LoroTree, wanted: &str) -> BenchResult<TreeID> {
    for node in tree.roots() {
        if map_string(&tree.get_meta(node)?, "block_id")? == wanted {
            return Ok(node);
        }
    }
    Err(format!("missing block {wanted}").into())
}

fn map_string(map: &LoroMap, key: &str) -> BenchResult<String> {
    match map.get(key) {
        Some(ValueOrContainer::Value(LoroValue::String(value))) => Ok((*value).clone()),
        _ => Err(format!("missing string field {key}").into()),
    }
}

fn map_text(map: &LoroMap, key: &str) -> BenchResult<String> {
    match map.get(key) {
        Some(ValueOrContainer::Container(Container::Text(text))) => Ok(text.to_string()),
        _ => Err(format!("missing text field {key}").into()),
    }
}

fn write_doc(path: &Path, doc: &LoroDoc, metrics: &mut BuildMetrics) -> BenchResult<()> {
    let started = Instant::now();
    let bytes = doc.export(ExportMode::all_updates())?;
    metrics.encode += started.elapsed();
    metrics.encoded_bytes += bytes.len() as u64;
    metrics.objects += 1;
    let started = Instant::now();
    fs::write(path, bytes)?;
    metrics.write += started.elapsed();
    Ok(())
}

fn read_doc(path: &Path, peer: u64) -> BenchResult<LoroDoc> {
    let bytes = fs::read(path)?;
    let doc = LoroDoc::new();
    doc.import(&bytes)?;
    doc.set_peer_id(peer)?;
    Ok(doc)
}

fn sequential_import_paths(paths: &[PathBuf]) -> BenchResult<SequentialImportMetrics> {
    let mut metrics = SequentialImportMetrics::default();
    for path in paths {
        let started = Instant::now();
        let bytes = fs::read(path)?;
        metrics.read += started.elapsed();
        let started = Instant::now();
        let doc = LoroDoc::new();
        doc.import(&bytes)?;
        metrics.import += started.elapsed();
        metrics.objects += 1;
    }
    Ok(metrics)
}

fn sequential_import_tree(
    catalog: &Path,
    page_dir: &Path,
    pages: usize,
) -> BenchResult<SequentialImportMetrics> {
    let mut paths = Vec::with_capacity(pages + 1);
    paths.push(catalog.to_path_buf());
    for page in 0..pages {
        paths.push(page_doc_path(page_dir, page));
    }
    sequential_import_paths(&paths)
}

fn new_doc(peer: u64) -> BenchResult<LoroDoc> {
    let doc = LoroDoc::new();
    doc.set_peer_id(peer)?;
    Ok(doc)
}

fn blocks_for_page(page: usize, pages: usize, blocks: usize) -> impl Iterator<Item = usize> {
    (page..blocks).step_by(pages)
}

fn current_page(block: usize, pages: usize, dispersed: bool) -> usize {
    let home = block % pages;
    if dispersed {
        (home + (block / pages).wrapping_mul(37)) % pages
    } else {
        home
    }
}

fn current_blocks_for_page(
    page: usize,
    pages: usize,
    blocks: usize,
    dispersed: bool,
) -> Vec<usize> {
    if !dispersed {
        return blocks_for_page(page, pages, blocks).collect();
    }
    let layers = blocks.div_ceil(pages);
    let mut selected = Vec::with_capacity(layers);
    for order in 0..layers {
        let offset = order.wrapping_mul(37) % pages;
        let home = (page + pages - offset) % pages;
        let block = order * pages + home;
        if block < blocks {
            selected.push(block);
        }
    }
    selected
}

fn parse_page_id(value: &str) -> BenchResult<usize> {
    let raw = value
        .strip_prefix('p')
        .ok_or_else(|| format!("invalid page id {value:?}"))?;
    Ok(usize::from_str_radix(raw, 16)?)
}

fn page_doc_path(dir: &Path, page: usize) -> PathBuf {
    dir.join(format!("{page:08x}.loro"))
}

fn page_id(page: usize) -> String {
    format!("p{page:08x}")
}

fn block_id(block: usize) -> String {
    format!("b{block:016x}")
}

fn page_path(page: usize) -> String {
    format!("pages/Page {page:08}.md")
}

fn block_raw(block: usize, page: usize) -> String {
    format!(
        "TODO synthetic block {block:016x} on [[Page {:08}]] #bench",
        (page * 37 + 11) % 100_000
    )
}

fn membership_value(order: usize, home_page: usize) -> String {
    format!("{order:08x}:{}", page_id(home_page))
}

fn peer_for_page(page: usize) -> u64 {
    page as u64 + 10
}

fn temp_root(pid: u32, layout: Layout, pages: usize, blocks: usize) -> PathBuf {
    env::temp_dir().join(format!(
        "tine-oplog-sharding-{pid}-{}-{pages}-{blocks}",
        layout.name()
    ))
}

fn remove_stale_temp_dirs() -> BenchResult<()> {
    let prefix = "tine-oplog-sharding-";
    for entry in fs::read_dir(env::temp_dir())? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(rest) = name.strip_prefix(prefix) {
            let pid = rest.split('-').next().unwrap_or_default();
            #[cfg(target_os = "linux")]
            if fs::read_link(Path::new("/proc").join(pid).join("exe"))
                .ok()
                .and_then(|path| path.file_name().map(|name| name.to_owned()))
                .is_some_and(|name| name == "oplog_sharding_bench")
            {
                continue;
            }
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn display_rss(value: Option<u64>) -> String {
    value.map_or_else(|| "unsupported".to_string(), |rss| rss.to_string())
}

fn display_optional_materialization<F>(metrics: &Option<MaterializationMetrics>, field: F) -> String
where
    F: FnOnce(&MaterializationMetrics) -> Option<u64>,
{
    metrics
        .as_ref()
        .and_then(field)
        .map_or_else(|| "unsupported".to_string(), |value| value.to_string())
}

fn display_optional_usize<F>(metrics: &Option<MaterializationMetrics>, field: F) -> String
where
    F: FnOnce(&MaterializationMetrics) -> usize,
{
    metrics.as_ref().map_or_else(
        || "unsupported".to_string(),
        |value| field(value).to_string(),
    )
}

fn rss_kib() -> (Option<u64>, Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = fs::read_to_string("/proc/self/status") else {
            return (None, None);
        };
        let mut current = None;
        let mut peak = None;
        for line in status.lines() {
            if let Some(value) = line.strip_prefix("VmRSS:") {
                current = value.split_whitespace().next().and_then(|v| v.parse().ok());
            } else if let Some(value) = line.strip_prefix("VmHWM:") {
                peak = value.split_whitespace().next().and_then(|v| v.parse().ok());
            }
        }
        (current, peak)
    }
    #[cfg(not(target_os = "linux"))]
    {
        (None, None)
    }
}
