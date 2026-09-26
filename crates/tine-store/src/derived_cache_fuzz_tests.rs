//! Differential fuzz for published derived-result memos. Each Store edit
//! publishes a new generation. Compare its carried memos against a memo-free
//! capture of the same parsed pages, and verify an older view stays unchanged.

use crate::model::ReadSnapshot;
use crate::store::{
    PageId, SaveBase, SaveOutcome, Store, RESULT_BRIDGE_MAX_BYTES, RESULT_BRIDGE_MAX_ROWS,
};
use std::sync::Arc;
use tine_core::{BlockDto, RefGroup};

// --- deterministic PRNG (xorshift64) so a failure reproduces from its seed ----
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PAGES: &[&str] = &["P0", "P1", "P2", "P3", "P4", "P5", "P6"];
const TAGS: &[&str] = &["t0", "t1", "t2"];
// Queries the parser accepts; chosen to exercise every predicate class that a
// scoped scheme has to reason about (task/priority/sort/page-ref/page-tags/
// scheduled/content).
const QUERIES: &[&str] = &[
    "(task TODO)",
    "(task TODO DOING)",
    "(and (task TODO) (priority A))",
    "(priority A)",
    "(scheduled)",
    "(page-tags t0)",
    "(page-tags t1)",
    "[[P0]]",
    "[[P3]]",
    "(and (task TODO) (sort-by priority asc))",
    "(content shipped)",
];

fn b(raw: impl Into<String>) -> BlockDto {
    BlockDto {
        id: String::new(),
        raw: raw.into(),
        collapsed: false,
        children: vec![],
        breadcrumb: vec![],
        ..Default::default()
    }
}

// One random block body, drawing from every shape the queries/backlinks key off.
fn gen_block(r: &mut Rng) -> BlockDto {
    let p = PAGES[r.below(PAGES.len())];
    let alt = format!("Alt{}", r.below(PAGES.len())); // an alias target
    let tag = TAGS[r.below(TAGS.len())];
    let dd = 1 + r.below(28);
    match r.below(9) {
        0 => b(format!("TODO work on [[{p}]]")),
        1 => b(format!("DONE [[{p}]] shipped")),
        2 => b(format!("TODO [#A] urgent [[{p}]]")),
        3 => b(format!(
            "DOING [#B] thing\nSCHEDULED: <2026-06-{dd:02} Mon>"
        )),
        4 => b(format!("#{tag} idea about [[{p}]]")),
        5 => b(format!("plain prose mentioning {p} without a link")),
        6 => b(format!("note linking [[{alt}]] (an alias)")),
        7 => b(format!("LATER [#C] revisit [[{p}]] and #{tag}")),
        _ => b("just some text, shipped nothing".to_string()),
    }
}

// A random pre-block (page properties): sometimes a tag, sometimes an alias.
fn gen_pre(r: &mut Rng, page_idx: usize) -> Option<String> {
    let mut lines = Vec::new();
    if r.below(2) == 0 {
        lines.push(format!("tags:: {}", TAGS[r.below(TAGS.len())]));
    }
    if r.below(3) == 0 {
        lines.push(format!("alias:: Alt{page_idx}"));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

// Order-sensitive fingerprint (page order + within-page block order both matter,
// e.g. for sorted queries). uuid-free: compares block first-lines, since the two
// graphs assign generated uuids independently.
fn fingerprint(g: &ReadSnapshot) -> String {
    let fmt = |label: String, groups: Arc<Vec<RefGroup>>| {
        let body = groups
            .iter()
            .map(|grp| {
                let blocks = grp
                    .blocks
                    .iter()
                    .map(|bl| bl.raw.lines().next().unwrap_or("").to_string())
                    .collect::<Vec<_>>()
                    .join("|");
                format!("{}/{:?}[{}]", grp.page, grp.kind, blocks)
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!("{label}=> {body}")
    };
    let mut out = Vec::new();
    for p in PAGES {
        out.push(fmt(
            format!("bl:{p}"),
            g.backlinks_bounded(p, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES)
                .groups,
        ));
        out.push(fmt(
            format!("ul:{p}"),
            g.unlinked_refs_bounded(p, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES)
                .groups,
        ));
        out.push(fmt(
            format!("blAlt:{p}"),
            g.backlinks_bounded(
                &format!("Alt{}", &p[1..]),
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )
            .groups,
        ));
    }
    for q in QUERIES {
        out.push(fmt(
            format!("q:{q}"),
            g.run_query_bounded(q, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES)
                .groups,
        ));
    }
    out.join("\n")
}

fn mk(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("tine-dcfuzz-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    root
}

fn run_seed(seed: u64) {
    let mut r = Rng(seed);
    let root = mk(&format!("s{seed}"));
    // Initial fixed page set on disk.
    for (i, p) in PAGES.iter().enumerate() {
        let nblocks = 1 + r.below(3);
        let mut s = String::new();
        if let Some(pre) = gen_pre(&mut r, i) {
            s.push_str(&pre);
            s.push_str("\n\n");
        }
        for _ in 0..nblocks {
            s.push_str(&format!(
                "- {}\n",
                gen_block(&mut r).raw.replace('\n', "\n  ")
            ));
        }
        std::fs::write(root.join("pages").join(format!("{p}.md")), s).unwrap();
    }

    let store = Store::open(&root, Default::default()).unwrap().0;
    let mut previous = store.whole_graph().unwrap();
    let mut previous_fp = fingerprint(&previous.graph);

    for iter in 0..220 {
        // Apply one random CONTENT edit to a random page through the real save path.
        let pi = r.below(PAGES.len());
        let name = PAGES[pi];
        let id = PageId::from(format!("pages/{name}.md"));
        let read = store.page(&id).unwrap();
        let mut dto = read.doc;
        match r.below(4) {
            0 => {
                // Replace all blocks with a fresh random set.
                let n = 1 + r.below(3);
                dto.blocks = (0..n).map(|_| gen_block(&mut r)).collect();
            }
            1 if !dto.blocks.is_empty() => {
                // Mutate one block.
                let bi = r.below(dto.blocks.len());
                dto.blocks[bi] = gen_block(&mut r);
            }
            2 => dto.blocks.push(gen_block(&mut r)), // add a block
            _ => {
                // Toggle the page's pre-block (alias/tags) — the escalation path.
                dto.pre_block = gen_pre(&mut r, pi);
            }
        }
        match store.save(&id, SaveBase::Existing(read.rev), &dto) {
            SaveOutcome::Saved(_) => {}
            SaveOutcome::Unchanged(rev) => {
                // Keep every random case while making this step a real generation.
                dto.pre_block = Some(format!(
                    "{}\nfuzz-step:: {iter}",
                    dto.pre_block.unwrap_or_default()
                ));
                assert!(matches!(
                    store.save(&id, SaveBase::Existing(rev), &dto),
                    SaveOutcome::Saved(_)
                ));
            }
            other => panic!("seed {seed} iter {iter}: save failed: {other:?}"),
        }

        let view = store.whole_graph().unwrap();
        assert_ne!(
            view.rev(),
            previous.rev(),
            "seed {seed} iter {iter}: no publication"
        );
        assert_eq!(
            fingerprint(&previous.graph),
            previous_fp,
            "seed {seed} iter {iter}: old view changed"
        );
        // The same publication constructor, without a parent, gives this
        // generation an empty memo table for the differential oracle.
        let fresh = ReadSnapshot::capture(
            &store.graph,
            (*view.config.config).clone(),
            Arc::clone(&view.list),
            None,
            &[],
        );
        let live_fp = fingerprint(&view.graph);
        let fresh_fp = fingerprint(&fresh);
        if live_fp != fresh_fp {
            // Find the first diverging probe line for a readable failure.
            let diff = live_fp
                .lines()
                .zip(fresh_fp.lines())
                .find(|(a, b)| a != b)
                .map(|(a, b)| format!("\n  LIVE : {a}\n  FRESH: {b}"))
                .unwrap_or_default();
            panic!("seed {seed} iter {iter}: carried memos diverged from fresh after editing {name}{diff}");
        }
        previous = view;
        previous_fp = live_fp;
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn derived_cache_matches_fresh_under_random_edits() {
    for seed in [1u64, 2, 3, 4, 5, 0xC0FFEE, 0xDEADBEEF] {
        run_seed(seed);
    }
}
