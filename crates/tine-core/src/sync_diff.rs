//! Structural, block-level 2-way diff between a page (the "winner", kept in the
//! graph) and a sync-tool conflict copy (Syncthing/Dropbox) of it. Feeds the
//! conflict-merge UI (see `docs/plans/sync-conflict-merge.md`).
//!
//! This is NOT a text-blob diff: it aligns the two BLOCK TREES so the UI can
//! offer per-block keep-mine / keep-theirs / keep-both, and so the resolve step
//! can rebuild a merged tree by re-deriving the SAME alignment and applying the
//! user's per-row decisions (the diff and the apply are symmetric — both are a
//! pure function of the two parsed docs, so a row id means the same thing to
//! both). See `Graph::resolve_sync_conflict`.
//!
//! Matching (per the plan's 3-level scheme), applied to each SIBLING list:
//!   L1  same persisted `id::` (strongest anchor — a conflict copy shares the
//!       winner's ids) OR content-equal subtree → an anchor.
//!   L2  in the gaps between anchors, pair by first-line similarity
//!       (normalized Levenshtein > 0.8) → a *modified* hunk.
//!   L3  whatever is left: present only in the winner → *added*; present only in
//!       the conflict → *removed*. Never silently dropped.
//! Anchored/paired rows with both sides present recurse into their children.
//!
//! Complexity: LCS uses Hirschberg reconstruction (O(k²) time, O(k) memory).
//! Similarity pairing is exact for ordinary gaps and falls back to a linear,
//! no-data-loss alignment when a gap would require excessive pair comparisons.

use crate::doc::DocBlock;
use serde::{Deserialize, Serialize};

/// One side of a diff row — enough for the UI to render (full `raw`, the UI
/// emphasizes its first line) and for a human to judge the hunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockView {
    /// Persisted `id::` if the block has one, else empty (display/debug only).
    pub uuid: String,
    /// The block's full dedented body (`raw`); may be multi-line.
    pub text: String,
    /// Number of direct children (so the UI can show "+3 sub-blocks").
    pub child_count: usize,
}

impl BlockView {
    fn of(b: &DocBlock) -> Self {
        BlockView {
            uuid: b.property("id").unwrap_or_default(),
            text: b.raw.clone(),
            child_count: b.children.len(),
        }
    }
}

/// How a row differs between winner (`mine`) and conflict (`theirs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowKind {
    /// Content-equal subtree — present and identical on both sides.
    Unchanged,
    /// Matched (by id or similarity) but the block content differs.
    Modified,
    /// Present only in the winner.
    Added,
    /// Present only in the conflict copy.
    Removed,
}

/// How a row relates to the 3-way BASE (the common ancestor a VCS merge wrote
/// into the file's diff3 `|||||||` region). Only present on 3-way diffs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Diff3Verdict {
    /// Only the winner diverged from the base → keeping mine preserves the change.
    MineOnly,
    /// Only the conflict copy diverged from the base → keeping theirs preserves it.
    TheirsOnly,
    /// Both sides diverged from the base — a true conflict, no safe suggestion.
    BothChanged,
}

/// Where a proposed merged body came from. Both are confirmation-gated and both
/// are re-derived at apply time; the distinction is provenance, which the UI
/// shows because the two carry different guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergedSource {
    /// Composed here from two disjoint edits of the base — a stronger claim,
    /// so it always wins when both sources can supply a body.
    Computed,
    /// Lifted from the merge tool's own `####### SUGGESTED CONFLICT RESOLUTION`
    /// region (Fossil), which Tine reconstructs into a whole page and aligns
    /// like any other document. Tine vouches for nothing about its content
    /// beyond the same validity gate — hence "artifact", not "merge".
    Artifact,
}

/// A merged body offered for a `BothChanged` row. Display only: the resolve
/// re-derives the text from the same three inputs and re-runs the same gates,
/// so a client can never make Tine write a body it did not itself compute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedProposal {
    pub text: String,
    pub source: MergedSource,
}

/// One aligned position in the block trees. `id` is a stable path ("2.1" = 2nd
/// child of the 3rd row) that the resolve step reproduces exactly, so the UI's
/// per-row decisions map back onto the same blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffRow {
    pub id: String,
    pub kind: RowKind,
    pub mine: Option<BlockView>,
    pub theirs: Option<BlockView>,
    /// Aligned children — only for `Modified`/`Unchanged` rows (both sides
    /// present). `Added`/`Removed` subtrees are atomic (one decision for the
    /// whole subtree), so they carry no child rows.
    pub children: Vec<DiffRow>,
    /// 3-way classification against the base (None on 2-way diffs and on rows
    /// where the base gives no signal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Diff3Verdict>,
    /// The pre-selected decision the base justifies: `"mine"`, `"theirs"`, or
    /// `"merged"` (see [`DiffRow::merged`]).
    /// Never auto-applied — the UI only pre-selects it for the user to confirm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// A merged body proposed for a `BothChanged` row: composed from two
    /// disjoint edits of one unambiguous base, or lifted from a merge tool's
    /// own suggestion region. Present only on 3-way diffs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<MergedProposal>,
}

/// The full diff of a conflict copy against its winner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictDiff {
    /// Revision of the exact winner bytes this alignment was computed from.
    /// The merge command requires it, so row decisions can never be applied to
    /// a later, differently aligned winner.
    pub base_rev: String,
    /// Revision of the exact conflict-copy bytes used for this alignment.
    pub conflict_rev: String,
    pub rows: Vec<DiffRow>,
    /// Winner's page-property pre-block, if any.
    pub mine_pre: Option<String>,
    /// Conflict copy's page-property pre-block, if any.
    pub theirs_pre: Option<String>,
    /// Whether the pre-blocks differ (so the UI can flag a property divergence).
    pub pre_differs: bool,
    /// True when the two block trees are identical (only the pre-block, or
    /// nothing, differs) — lets the UI say "no block changes".
    pub blocks_identical: bool,
    /// True when the rows carry 3-way verdicts computed against a real base
    /// (so the UI can explain where its pre-selections come from).
    /// Omitted from the wire when false, so a 2-way answer keeps the v0.6.5
    /// shape byte for byte.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub three_way: bool,
}

/// Diff `theirs` (the conflict copy's blocks) against `mine` (the winner's).
pub fn diff_blocks(mine: &[DocBlock], theirs: &[DocBlock]) -> Vec<DiffRow> {
    nodes_to_rows(&align_nodes(mine, theirs, ""))
}

/// Build the full page-level diff, including the pre-block comparison.
pub fn diff_docs(mine: &crate::doc::Document, theirs: &crate::doc::Document) -> SyncConflictDiff {
    let rows = diff_blocks(&mine.roots, &theirs.roots);
    let blocks_identical = rows.iter().all(|r| r.kind == RowKind::Unchanged);
    let mine_pre = normalize_pre(mine.pre_block.as_deref());
    let theirs_pre = normalize_pre(theirs.pre_block.as_deref());
    SyncConflictDiff {
        base_rev: String::new(),
        conflict_rev: String::new(),
        pre_differs: mine_pre != theirs_pre,
        mine_pre,
        theirs_pre,
        rows,
        blocks_identical,
        three_way: false,
    }
}

/// Build the full 3-way page-level diff: the SAME mine/theirs alignment as
/// [`diff_docs`] (so row ids stay compatible with [`merge_blocks`]), with each
/// row additionally classified against `base` — the common ancestor both sides
/// descend from (in og: the diff3 base a VCS merge left in the file itself). Non-conflicting rows carry a `suggestion`
/// (`"mine"`/`"theirs"`); rows both sides changed carry none. Suggestions are
/// advice for the UI to pre-select, never something to auto-apply.
pub fn diff3_docs(
    base: &crate::doc::Document,
    mine: &crate::doc::Document,
    theirs: &crate::doc::Document,
) -> SyncConflictDiff {
    diff3_docs_with_artifact(base, mine, theirs, None)
}

/// [`diff3_docs`] with a fourth, non-side document: the resolution a merge tool
/// already proposed for this file (Fossil's `#######` region, reconstructed by
/// `concord_queue::parse_vcs_marker_sides`). Rows where the computed merge
/// declines may then offer the artifact's own body instead.
///
/// The artifact is aligned with the SAME machinery as the base, so an artifact
/// block is located exactly the way a base block is; it never widens which rows
/// may carry a proposal (still `BothChanged` under a real base) and never
/// changes a verdict.
pub fn diff3_docs_with_artifact(
    base: &crate::doc::Document,
    mine: &crate::doc::Document,
    theirs: &crate::doc::Document,
    artifact: Option<&crate::doc::Document>,
) -> SyncConflictDiff {
    let nodes = align_nodes(&mine.roots, &theirs.roots, "");
    let mut rows = nodes_to_rows(&nodes);
    let mut base_of_mine = HashMap::new();
    collect_base_pairs(&base.roots, &mine.roots, &mut base_of_mine);
    let mut base_of_theirs = HashMap::new();
    collect_base_pairs(&base.roots, &theirs.roots, &mut base_of_theirs);
    let mut art_of_mine = HashMap::new();
    let mut art_of_theirs = HashMap::new();
    if let Some(artifact) = artifact {
        collect_base_pairs(&artifact.roots, &mine.roots, &mut art_of_mine);
        collect_base_pairs(&artifact.roots, &theirs.roots, &mut art_of_theirs);
    }
    let artifacts = artifact.map(|_| (&art_of_mine, &art_of_theirs));
    annotate_rows(&mut rows, &nodes, &base_of_mine, &base_of_theirs, artifacts);
    let blocks_identical = rows.iter().all(|r| r.kind == RowKind::Unchanged);
    let mine_pre = normalize_pre(mine.pre_block.as_deref());
    let theirs_pre = normalize_pre(theirs.pre_block.as_deref());
    SyncConflictDiff {
        base_rev: String::new(),
        conflict_rev: String::new(),
        pre_differs: mine_pre != theirs_pre,
        mine_pre,
        theirs_pre,
        rows,
        blocks_identical,
        three_way: true,
    }
}

fn parse_text(content: &str, org: bool) -> crate::doc::Document {
    if org {
        crate::org::parse_org(content)
    } else {
        crate::doc::parse(content)
    }
}

use std::collections::HashMap;

type BasePairs<'a> = HashMap<*const DocBlock, &'a DocBlock>;

/// Map each of `side`'s blocks (by identity) to its aligned base block, using
/// the same alignment machinery as the diff. Content-equal subtrees pair their
/// descendants positionally (identical shape); aligned modified pairs recurse.
fn collect_base_pairs<'a>(base: &'a [DocBlock], side: &'a [DocBlock], out: &mut BasePairs<'a>) {
    fn walk<'a>(nodes: &[Node<'a>], out: &mut BasePairs<'a>) {
        for node in nodes {
            if let Node::Both {
                mine: base_block,
                theirs: side_block,
                modified,
                children,
                ..
            } = node
            {
                out.insert(*side_block as *const DocBlock, base_block);
                if *modified {
                    walk(children, out);
                } else {
                    pair_equal_subtrees(base_block, side_block, out);
                }
            }
        }
    }
    fn pair_equal_subtrees<'a>(base: &'a DocBlock, side: &'a DocBlock, out: &mut BasePairs<'a>) {
        for (b, s) in base.children.iter().zip(side.children.iter()) {
            out.insert(s as *const DocBlock, b);
            pair_equal_subtrees(b, s, out);
        }
    }
    walk(&align_nodes(base, side, ""), out);
}

/// Classify each aligned row against the base. `rows` and `nodes` have the same
/// shape by construction (both come from the same `align_nodes` output).
fn annotate_rows(
    rows: &mut [DiffRow],
    nodes: &[Node],
    base_of_mine: &BasePairs,
    base_of_theirs: &BasePairs,
    artifacts: Option<(&BasePairs, &BasePairs)>,
) {
    for (row, node) in rows.iter_mut().zip(nodes.iter()) {
        match node {
            Node::Both {
                mine,
                theirs,
                modified,
                children,
                ..
            } => {
                if !*modified {
                    continue;
                }
                // The row decision picks this block's BODY (children have their
                // own rows), so classify the body only.
                if mine.raw != theirs.raw {
                    let base_mine = base_of_mine.get(&(*mine as *const DocBlock)).copied();
                    let base_theirs = base_of_theirs.get(&(*theirs as *const DocBlock)).copied();
                    let mine_changed = base_mine.is_none_or(|b| b.raw != mine.raw);
                    let theirs_changed = base_theirs.is_none_or(|b| b.raw != theirs.raw);
                    let (verdict, suggestion) = match (mine_changed, theirs_changed) {
                        (true, false) => (Diff3Verdict::MineOnly, Some("mine")),
                        (false, true) => (Diff3Verdict::TheirsOnly, Some("theirs")),
                        (true, true) => (Diff3Verdict::BothChanged, None),
                        // Both sides equal their base yet differ from each
                        // other: the two maps paired this row with DIFFERENT
                        // base blocks (duplicate content). An ambiguous base is
                        // no base — a conflict, and never a merge proposal.
                        (false, false) => (Diff3Verdict::BothChanged, None),
                    };
                    row.verdict = Some(verdict);
                    row.suggestion = suggestion.map(str::to_string);
                    if let (true, true) = (mine_changed, theirs_changed) {
                        // PRECEDENCE: a composition of two disjoint edits is a
                        // stronger claim than a third party's guess, so the
                        // artifact is consulted only where the computation
                        // declines. Same order in the apply path.
                        if let Some(text) = merged_body(base_mine, base_theirs, mine, theirs) {
                            row.suggestion = Some("merged".to_string());
                            row.merged = Some(MergedProposal {
                                text,
                                source: MergedSource::Computed,
                            });
                        } else if let Some(text) = artifact_body_for(artifacts, mine, theirs) {
                            row.suggestion = Some("merged".to_string());
                            row.merged = Some(MergedProposal {
                                text,
                                source: MergedSource::Artifact,
                            });
                        }
                    }
                }
                annotate_rows(
                    &mut row.children,
                    children,
                    base_of_mine,
                    base_of_theirs,
                    artifacts,
                );
            }
            // Added row (winner-only). Absent from base → mine added it (keep).
            // Present and unchanged → theirs deleted it (suggest the deletion).
            // Present but edited by mine while theirs deleted it → conflict.
            Node::Mine { block, .. } => {
                let (verdict, suggestion) = match base_of_mine.get(&(*block as *const DocBlock)) {
                    None => (Diff3Verdict::MineOnly, Some("mine")),
                    Some(b) if *b == *block => (Diff3Verdict::TheirsOnly, Some("theirs")),
                    Some(_) => (Diff3Verdict::BothChanged, None),
                };
                row.verdict = Some(verdict);
                row.suggestion = suggestion.map(str::to_string);
            }
            // Removed row (conflict-only). Absent from base → theirs added it
            // (suggest pulling it in). Present and unchanged → mine deleted it
            // (suggest skipping). Present but edited by theirs → conflict.
            Node::Theirs { block, .. } => {
                let (verdict, suggestion) = match base_of_theirs.get(&(*block as *const DocBlock)) {
                    None => (Diff3Verdict::TheirsOnly, Some("theirs")),
                    Some(b) if *b == *block => (Diff3Verdict::MineOnly, Some("mine")),
                    Some(_) => (Diff3Verdict::BothChanged, None),
                };
                row.verdict = Some(verdict);
                row.suggestion = suggestion.map(str::to_string);
            }
        }
    }
}

/// The merged body for one aligned `BothChanged` pair, or `None` when no
/// proposal may be offered. Both diff and apply go through here, so the
/// suggestion the user confirmed and the text finally written are one
/// computation over the same three inputs.
///
/// Requires ONE unambiguous base: both sides must have a base block and the two
/// base bodies must agree (the two maps can pair duplicate content with
/// different blocks; a merge against an ambiguous base is never offered).
fn merged_body(
    base_mine: Option<&DocBlock>,
    base_theirs: Option<&DocBlock>,
    mine: &DocBlock,
    theirs: &DocBlock,
) -> Option<String> {
    let (base_mine, base_theirs) = (base_mine?, base_theirs?);
    if base_mine.raw != base_theirs.raw {
        return None;
    }
    let text = crate::text_merge::merge_disjoint(&base_mine.raw, &mine.raw, &theirs.raw)?;
    merged_body_is_valid(&text, mine.is_org).then_some(text)
}

/// [`artifact_body`] for a row, given the pair of artifact maps (or `None` when
/// no artifact document reached this diff/merge at all). Kept next to the maps
/// so the diff and the apply do the same two lookups.
fn artifact_body_for(
    artifacts: Option<(&BasePairs, &BasePairs)>,
    mine: &DocBlock,
    theirs: &DocBlock,
) -> Option<String> {
    let (art_of_mine, art_of_theirs) = artifacts?;
    artifact_body(
        art_of_mine.get(&(mine as *const DocBlock)).copied(),
        art_of_theirs.get(&(theirs as *const DocBlock)).copied(),
        mine,
        theirs,
    )
}

/// The merge tool's own proposed body for one aligned `BothChanged` pair, or
/// `None` when it may not be offered. Mirrors [`merged_body`]: both the diff and
/// the apply go through here, so the text the user confirmed and the text
/// finally written are one lookup over the same inputs.
///
/// Requires ONE unambiguous artifact block — both sides must pair with one and
/// the two bodies must agree (the two alignments can pair duplicate content with
/// different blocks) — a body that differs from BOTH sides (a proposal equal to
/// a side is just one of the three choices the user already has, so offering it
/// as a fourth would be noise), and the same structural/org validity gate a
/// computed body passes.
fn artifact_body(
    art_mine: Option<&DocBlock>,
    art_theirs: Option<&DocBlock>,
    mine: &DocBlock,
    theirs: &DocBlock,
) -> Option<String> {
    let (art_mine, art_theirs) = (art_mine?, art_theirs?);
    if art_mine.raw != art_theirs.raw {
        return None;
    }
    let text = &art_mine.raw;
    if *text == mine.raw || *text == theirs.raw {
        return None;
    }
    merged_body_is_valid(text, mine.is_org).then(|| text.clone())
}

/// Whether a merged body still IS one block: serialized the way the page
/// serializer writes it and re-parsed, it must come back as exactly one root
/// block with no children and a byte-identical `raw`.
///
/// This is the structural gate a character-level merge needs. Composing two
/// disjoint edits can produce text that no longer round-trips as one block — a
/// line that now starts a new bullet, an unbalanced `:LOGBOOK:` drawer, an org
/// body that breaks its headline. Such a body is never offered and never
/// applied; the org firewall at write time remains the final authority.
fn merged_body_is_valid(merged: &str, is_org: bool) -> bool {
    // Complementary disjoint deletions can compose to an empty body. Emptying
    // the block is a deletion the user must choose deliberately — it is never
    // offered (or applied) as a "merge" of the two sides.
    if merged.trim().is_empty() {
        return false;
    }
    let mut block = DocBlock::new(merged.to_string());
    block.is_org = is_org;
    let doc = crate::doc::Document {
        pre_block: None,
        roots: vec![block],
    };
    let serialized = if is_org {
        crate::org::serialize_org(&doc)
    } else {
        crate::doc::serialize(&doc)
    };
    if is_org && !crate::org::org_round_trips(&serialized) {
        return false;
    }
    let reparsed = parse_text(&serialized, is_org);
    reparsed.pre_block.is_none()
        && reparsed.roots.len() == 1
        && reparsed.roots[0].children.is_empty()
        && reparsed.roots[0].raw == merged
}

fn normalize_pre(pre: Option<&str>) -> Option<String> {
    pre.map(|s| s.to_string()).filter(|s| !s.trim().is_empty())
}

/// The block's persisted `id::`, if present and non-empty.
fn persisted_id(b: &DocBlock) -> Option<String> {
    b.property("id").filter(|s| !s.is_empty())
}

/// Anchor equality for the LCS: same non-empty persisted id, or a content-equal
/// subtree. (Blocks without ids anchor only when their whole subtree matches.)
fn anchor_eq(a: &DocBlock, b: &DocBlock) -> bool {
    match (persisted_id(a), persisted_id(b)) {
        (Some(ia), Some(ib)) => ia == ib,
        _ => a == b, // DocBlock PartialEq = content-equal (raw + children), ignores uuid
    }
}

/// Cap on the similarity key. A block's "first line" is its ENTIRE body for a
/// single-line block, and the pairwise Levenshtein below is O(len²) — measured
/// at 5.4 s per 64 KB pair (2-way) before this cap. Pairing is a heuristic;
/// 512 chars decide "same edited line vs different line" just as well.
const SIMILARITY_KEY_MAX_CHARS: usize = 512;

/// First visible line of a block (property lines stripped), lowercased,
/// trimmed, and capped at [`SIMILARITY_KEY_MAX_CHARS`] — the key the L2
/// similarity pairing compares.
fn first_line_key(b: &DocBlock) -> String {
    b.visible_text()
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(SIMILARITY_KEY_MAX_CHARS)
        .collect::<String>()
        .to_lowercase()
}

/// Normalized similarity of two capped strings in [0,1] (1 = identical).
///
/// When the length gap alone puts the pair under [`SIMILARITY_THRESHOLD`] the
/// exact distance is skipped and the (correct) upper bound is returned —
/// callers only test `>= SIMILARITY_THRESHOLD`, so a below-threshold value
/// never needs to be exact.
fn similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (la, lb) = (a.chars().count(), b.chars().count());
    let max = la.max(lb);
    if max == 0 {
        return 1.0;
    }
    // levenshtein(a, b) >= |la − lb|.
    let bound = 1.0 - (la.abs_diff(lb) as f32 / max as f32);
    if bound < SIMILARITY_THRESHOLD {
        return bound;
    }
    let d = levenshtein(a, b);
    1.0 - (d as f32 / max as f32)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

const SIMILARITY_THRESHOLD: f32 = 0.8;
const MAX_GAP_SIMILARITY_COMPARISONS: usize = 250_000;
/// Work budget for the exact (Hirschberg) LCS, in `n·m` products. At the old
/// 1e6 budget an all-conflicted 1000×1000 pair spent ~0.95 s in the exact
/// branch while 1001×1001 took 14 ms in the patience fallback — an inverted
/// cliff. 250k keeps the exact branch's worst case around a quarter second.
const MAX_LCS_COMPARISONS: usize = 250_000;

/// One aligned position in the two trees — the SINGLE source of alignment truth
/// that both the diff rows ([`nodes_to_rows`]) and the merged output
/// ([`nodes_to_merged`]) derive from. Because both walk the same nodes in the
/// same order, a row `id` addresses the same block in the diff the UI shows and
/// in the merge the resolve applies (the diff and the apply stay symmetric).
enum Node<'a> {
    /// Present on both sides (matched by id, content, or similarity). `modified`
    /// is false iff the subtrees are content-equal.
    Both {
        id: String,
        mine: &'a DocBlock,
        theirs: &'a DocBlock,
        modified: bool,
        children: Vec<Node<'a>>,
    },
    /// Winner-only subtree (an Added block).
    Mine { id: String, block: &'a DocBlock },
    /// Conflict-only subtree (a Removed block).
    Theirs { id: String, block: &'a DocBlock },
}

/// Align two sibling lists into ordered [`Node`]s. `prefix` is the parent's path
/// id ("" at the root, "2.0." otherwise) so child ids are stable and identical on
/// the diff and merge walks.
fn align_nodes<'a>(mine: &'a [DocBlock], theirs: &'a [DocBlock], prefix: &str) -> Vec<Node<'a>> {
    // --- L1: LCS over the sibling sequences using anchor equality. ---
    let matched = lcs_pairs(mine, theirs);
    let mut out: Vec<Node> = Vec::new();
    let mut mi = 0usize;
    let mut ti = 0usize;
    let mut counter = 0usize;
    for (i, j) in matched.iter().copied() {
        gap_nodes(mine, theirs, mi, i, ti, j, prefix, &mut out, &mut counter);
        let id = row_id(prefix, counter);
        counter += 1;
        let a = &mine[i];
        let b = &theirs[j];
        if a == b {
            out.push(Node::Both {
                id,
                mine: a,
                theirs: b,
                modified: false,
                children: Vec::new(),
            });
        } else {
            let children = align_nodes(&a.children, &b.children, &format!("{id}."));
            out.push(Node::Both {
                id,
                mine: a,
                theirs: b,
                modified: true,
                children,
            });
        }
        mi = i + 1;
        ti = j + 1;
    }
    gap_nodes(
        mine,
        theirs,
        mi,
        mine.len(),
        ti,
        theirs.len(),
        prefix,
        &mut out,
        &mut counter,
    );
    out
}

/// Align an unmatched gap: pair similar first-lines as a modified `Both`, then
/// leftover winner blocks as `Mine` (Added) and conflict blocks as `Theirs`
/// (Removed), preserving order.
#[allow(clippy::too_many_arguments)]
fn gap_nodes<'a>(
    mine: &'a [DocBlock],
    theirs: &'a [DocBlock],
    m_from: usize,
    m_to: usize,
    t_from: usize,
    t_to: usize,
    prefix: &str,
    out: &mut Vec<Node<'a>>,
    counter: &mut usize,
) {
    if (m_to - m_from).saturating_mul(t_to - t_from) > MAX_GAP_SIMILARITY_COMPARISONS {
        // A conflict with thousands of unrelated flat siblings must not spend
        // minutes doing pairwise Levenshtein. Emit both sides explicitly; the
        // safe default still keeps mine and the recoverable conflict copy keeps
        // theirs. Exact anchors were already removed by L1.
        for block in &mine[m_from..m_to] {
            let id = row_id(prefix, *counter);
            *counter += 1;
            out.push(Node::Mine { id, block });
        }
        for block in &theirs[t_from..t_to] {
            let id = row_id(prefix, *counter);
            *counter += 1;
            out.push(Node::Theirs { id, block });
        }
        return;
    }
    let mut used_theirs = vec![false; t_to.saturating_sub(t_from)];
    let their_keys: Vec<String> = (t_from..t_to).map(|j| first_line_key(&theirs[j])).collect();
    // Walk both gaps interleaved by winner position; a pure per-winner greedy
    // could emit a Removed out of order, so flush skipped conflict blocks first.
    let mut tj = t_from; // conflict cursor
    for i in m_from..m_to {
        let key = first_line_key(&mine[i]);
        let mut best: Option<(usize, f32)> = None;
        for j in tj..t_to {
            if used_theirs[j - t_from] {
                continue;
            }
            let s = similarity(&key, &their_keys[j - t_from]);
            if s >= SIMILARITY_THRESHOLD && best.map_or(true, |(_, bs)| s > bs) {
                best = Some((j, s));
            }
        }
        if let Some((j, _)) = best {
            for k in tj..j {
                if !used_theirs[k - t_from] {
                    let id = row_id(prefix, *counter);
                    *counter += 1;
                    out.push(Node::Theirs {
                        id,
                        block: &theirs[k],
                    });
                    used_theirs[k - t_from] = true;
                }
            }
            let id = row_id(prefix, *counter);
            *counter += 1;
            let children = align_nodes(&mine[i].children, &theirs[j].children, &format!("{id}."));
            out.push(Node::Both {
                id,
                mine: &mine[i],
                theirs: &theirs[j],
                modified: true,
                children,
            });
            used_theirs[j - t_from] = true;
            tj = j + 1;
        } else {
            let id = row_id(prefix, *counter);
            *counter += 1;
            out.push(Node::Mine {
                id,
                block: &mine[i],
            });
        }
    }
    for j in t_from..t_to {
        if !used_theirs[j - t_from] {
            let id = row_id(prefix, *counter);
            *counter += 1;
            out.push(Node::Theirs {
                id,
                block: &theirs[j],
            });
        }
    }
}

/// Project the aligned nodes into the diff rows the UI renders.
fn nodes_to_rows(nodes: &[Node]) -> Vec<DiffRow> {
    nodes
        .iter()
        .map(|n| match n {
            Node::Both {
                id,
                mine,
                theirs,
                modified,
                children,
            } => DiffRow {
                id: id.clone(),
                kind: if *modified {
                    RowKind::Modified
                } else {
                    RowKind::Unchanged
                },
                mine: Some(BlockView::of(mine)),
                theirs: Some(BlockView::of(theirs)),
                children: nodes_to_rows(children),
                verdict: None,
                suggestion: None,
                merged: None,
            },
            Node::Mine { id, block } => DiffRow {
                id: id.clone(),
                kind: RowKind::Added,
                mine: Some(BlockView::of(block)),
                theirs: None,
                children: Vec::new(),
                verdict: None,
                suggestion: None,
                merged: None,
            },
            Node::Theirs { id, block } => DiffRow {
                id: id.clone(),
                kind: RowKind::Removed,
                mine: None,
                theirs: Some(BlockView::of(block)),
                children: Vec::new(),
                verdict: None,
                suggestion: None,
                merged: None,
            },
        })
        .collect()
}

// --- merge (the resolve side; symmetric with the diff via the same nodes) -----

/// A user's per-row choice in the merge UI. Any row the UI didn't send defaults
/// to `Mine` (keep the winner) — the safe default, since the conflict copy is
/// trashed-recoverable so nothing is lost.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Decision {
    Mine,
    Theirs,
    Both,
    /// Take the proposed merged body — [`merged_body`] composed from both edits
    /// of the base, or failing that the merge tool's own [`artifact_body`].
    /// Only ever valid on an aligned `Modified` row of a 3-way resolve; the text
    /// is re-derived here, never carried in the decision.
    Merged,
}

fn decision_for(decisions: &std::collections::HashMap<String, String>, id: &str) -> Decision {
    match decisions.get(id).map(String::as_str) {
        Some("theirs") => Decision::Theirs,
        Some("both") => Decision::Both,
        Some("merged") => Decision::Merged,
        _ => Decision::Mine,
    }
}

/// A `"merged"` decision the resolve could not re-derive. The WHOLE resolve
/// refuses: a merged row never silently falls back to one side, because the
/// user confirmed a specific body and no other outcome is what they approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRefused {
    /// Path id of the offending row (same ids the diff published).
    pub row: String,
    pub reason: &'static str,
}

impl std::fmt::Display for MergeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot apply the merged body for row {}: {}",
            self.row, self.reason
        )
    }
}

impl std::error::Error for MergeRefused {}

/// No base reached the resolve (2-way diff, e.g. a sync-tool conflict copy,
/// which carries no common ancestor),
/// so no merged body was ever offered for any row.
const NO_BASE: &str = "no common ancestor is available for this resolve";
/// The decision names a row that carries no mergeable pair (an Added/Removed
/// subtree, or an unchanged row).
const NOT_A_MODIFIED_ROW: &str = "this row is not an aligned modified block";
/// Re-derivation declined on BOTH sources: ambiguous base or artifact,
/// overlapping edits, a bounded-diff give-up, or a body that would not re-parse
/// as this one block.
const NOT_MERGEABLE: &str = "the two edits no longer merge cleanly";

/// Rebuild a merged sibling list from the two trees and the user's per-row
/// decisions. Re-derives the SAME alignment the diff used, so a decision id maps
/// onto the same block. Per-kind semantics:
///   - Unchanged  → the block, as-is.
///   - Modified   → mine/theirs body with recursively-merged children; `both`
///                  keeps both whole subtrees (the conflict copy's `id::`s
///                  stripped so they don't collide with the winner's).
///   - Added      → kept unless explicitly dropped (`theirs`).
///   - Removed    → pulled in only on `theirs`/`both`.
/// `merged` is the fourth Modified outcome and needs a base — see
/// [`merge_blocks3`].
pub fn merge_blocks(
    mine: &[DocBlock],
    theirs: &[DocBlock],
    decisions: &std::collections::HashMap<String, String>,
) -> Result<Vec<DocBlock>, MergeRefused> {
    merge_blocks3(None, mine, theirs, None, decisions)
}

/// 3-way form of [`merge_blocks`]: with `base` present, a row may also decide
/// `"merged"`, and the merged body is RE-DERIVED here from `base`/`mine`/
/// `theirs` (or, failing that, from `artifact`) — the decision map carries only
/// the plain string. `base` and `artifact` must be the same documents the diff
/// was computed against; the caller's staleness guards are what make that
/// true. For a marker file both are
/// re-derived from the guarded bytes of the one file, so "same inputs" is
/// structural.
///
/// With `base == None` the behavior is exactly the old 2-way merge, except that
/// a `"merged"` decision — which no 2-way diff can have offered — refuses. An
/// `artifact` without a `base` can therefore never be reached, matching the
/// diff, where a proposal needs a `BothChanged` verdict.
pub fn merge_blocks3(
    base: Option<&[DocBlock]>,
    mine: &[DocBlock],
    theirs: &[DocBlock],
    artifact: Option<&[DocBlock]>,
    decisions: &std::collections::HashMap<String, String>,
) -> Result<Vec<DocBlock>, MergeRefused> {
    let mut base_of_mine = BasePairs::new();
    let mut base_of_theirs = BasePairs::new();
    if let Some(base) = base {
        collect_base_pairs(base, mine, &mut base_of_mine);
        collect_base_pairs(base, theirs, &mut base_of_theirs);
    }
    let mut art_of_mine = BasePairs::new();
    let mut art_of_theirs = BasePairs::new();
    if let Some(artifact) = artifact {
        collect_base_pairs(artifact, mine, &mut art_of_mine);
        collect_base_pairs(artifact, theirs, &mut art_of_theirs);
    }
    let bases = base.map(|_| (&base_of_mine, &base_of_theirs));
    let artifacts = artifact.map(|_| (&art_of_mine, &art_of_theirs));
    nodes_to_merged(&align_nodes(mine, theirs, ""), decisions, bases, artifacts)
}

fn nodes_to_merged(
    nodes: &[Node],
    decisions: &std::collections::HashMap<String, String>,
    bases: Option<(&BasePairs, &BasePairs)>,
    artifacts: Option<(&BasePairs, &BasePairs)>,
) -> Result<Vec<DocBlock>, MergeRefused> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::Both {
                id,
                mine,
                theirs,
                modified,
                children,
            } => {
                if !*modified {
                    if decision_for(decisions, id) == Decision::Merged {
                        return Err(refused(id, NOT_A_MODIFIED_ROW));
                    }
                    out.push((*mine).clone()); // content-equal — keep as-is
                    continue;
                }
                match decision_for(decisions, id) {
                    Decision::Mine => out.push(rebuild(
                        mine,
                        nodes_to_merged(children, decisions, bases, artifacts)?,
                    )),
                    Decision::Theirs => out.push(rebuild(
                        theirs,
                        nodes_to_merged(children, decisions, bases, artifacts)?,
                    )),
                    Decision::Both => {
                        out.push((*mine).clone());
                        // Fresh block — must not duplicate the winner's id:: on disk.
                        out.push(strip_ids(theirs));
                    }
                    Decision::Merged => {
                        let Some((base_of_mine, base_of_theirs)) = bases else {
                            return Err(refused(id, NO_BASE));
                        };
                        // SAME precedence as the diff — computed first, the
                        // merge tool's artifact only where it declines — so the
                        // user is written the body they were shown.
                        let text = merged_body(
                            base_of_mine.get(&(*mine as *const DocBlock)).copied(),
                            base_of_theirs.get(&(*theirs as *const DocBlock)).copied(),
                            mine,
                            theirs,
                        )
                        .or_else(|| artifact_body_for(artifacts, mine, theirs))
                        .ok_or_else(|| refused(id, NOT_MERGEABLE))?;
                        // Same shape as keep-mine/keep-theirs: one body, the
                        // children's own decisions.
                        out.push(rebuild_with_raw(
                            mine,
                            text,
                            nodes_to_merged(children, decisions, bases, artifacts)?,
                        ));
                    }
                }
            }
            Node::Mine { id, block } => {
                // Added (winner-only): kept unless the user drops it.
                match decision_for(decisions, id) {
                    Decision::Merged => return Err(refused(id, NOT_A_MODIFIED_ROW)),
                    Decision::Theirs => {}
                    _ => out.push((*block).clone()),
                }
            }
            Node::Theirs { id, block } => {
                // Removed (conflict-only): pulled in on keep-theirs / keep-both. Its
                // id is unique to the conflict (a shared id would have anchored it as
                // a Both), so it's kept as-is.
                match decision_for(decisions, id) {
                    Decision::Merged => return Err(refused(id, NOT_A_MODIFIED_ROW)),
                    Decision::Theirs | Decision::Both => out.push((*block).clone()),
                    _ => {}
                }
            }
        }
    }
    Ok(out)
}

fn refused(row: &str, reason: &'static str) -> MergeRefused {
    MergeRefused {
        row: row.to_string(),
        reason,
    }
}

/// A block with `side`'s own body but the given (already-merged) children.
fn rebuild(side: &DocBlock, children: Vec<DocBlock>) -> DocBlock {
    rebuild_with_raw(side, side.raw.clone(), children)
}

/// [`rebuild`] with an explicit body — the merged-decision shape. The format
/// still comes from `side`, since a merge never crosses page formats.
fn rebuild_with_raw(side: &DocBlock, raw: String, children: Vec<DocBlock>) -> DocBlock {
    let mut b = DocBlock::new(raw);
    b.is_org = side.is_org;
    b.children = children;
    b
}

/// Deep-copy a block with every `id::` property line stripped — so keeping the
/// conflict's version alongside the winner's (keep-both) can't duplicate the
/// winner's `id::` on disk. The copy becomes a fresh, un-referenced block.
fn strip_ids(b: &DocBlock) -> DocBlock {
    let raw: String = b
        .raw
        .lines()
        .filter(|l| {
            crate::doc::parse_property_line(l).map_or(true, |(k, _)| !k.eq_ignore_ascii_case("id"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut nb = DocBlock::new(raw);
    nb.is_org = b.is_org;
    nb.children = b.children.iter().map(strip_ids).collect();
    nb
}

fn row_id(prefix: &str, n: usize) -> String {
    format!("{prefix}{n}")
}

/// LCS row lengths for Hirschberg reconstruction. Memory is O(right.len()).
fn lcs_lengths(left: &[DocBlock], right: &[DocBlock]) -> Vec<u32> {
    let mut prev = vec![0u32; right.len() + 1];
    let mut cur = vec![0u32; right.len() + 1];
    for a in left {
        for (j, b) in right.iter().enumerate() {
            cur[j + 1] = if anchor_eq(a, b) {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.fill(0);
    }
    prev
}

fn lcs_lengths_reversed(left: &[DocBlock], right: &[DocBlock]) -> Vec<u32> {
    let mut prev = vec![0u32; right.len() + 1];
    let mut cur = vec![0u32; right.len() + 1];
    for a in left.iter().rev() {
        for (j, b) in right.iter().rev().enumerate() {
            cur[j + 1] = if anchor_eq(a, b) {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.fill(0);
    }
    prev
}

fn hirschberg_pairs(
    mine: &[DocBlock],
    theirs: &[DocBlock],
    mine_offset: usize,
    theirs_offset: usize,
    out: &mut Vec<(usize, usize)>,
) {
    if mine.is_empty() || theirs.is_empty() {
        return;
    }
    if mine.len() == 1 {
        if let Some(j) = theirs.iter().position(|b| anchor_eq(&mine[0], b)) {
            out.push((mine_offset, theirs_offset + j));
        }
        return;
    }
    let mid = mine.len() / 2;
    let forward = lcs_lengths(&mine[..mid], theirs);
    let backward = lcs_lengths_reversed(&mine[mid..], theirs);
    let split = (0..=theirs.len())
        .max_by_key(|&j| forward[j] + backward[theirs.len() - j])
        .unwrap_or(0);
    hirschberg_pairs(
        &mine[..mid],
        &theirs[..split],
        mine_offset,
        theirs_offset,
        out,
    );
    hirschberg_pairs(
        &mine[mid..],
        &theirs[split..],
        mine_offset + mid,
        theirs_offset + split,
        out,
    );
}

fn anchor_fingerprint(block: &DocBlock) -> u64 {
    fn add(mut hash: u64, bytes: &[u8]) -> u64 {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
    if let Some(id) = persisted_id(block) {
        return add(0xcbf2_9ce4_8422_2325 ^ 1, id.as_bytes());
    }
    let mut hash = add(0xcbf2_9ce4_8422_2325 ^ 2, block.raw.as_bytes());
    for child in &block.children {
        hash = add(hash, &anchor_fingerprint(child).to_le_bytes());
    }
    hash
}

/// Fast large-list alignment: unique strong/content fingerprints become
/// candidates, then a patience/LIS pass keeps their longest ordered subset.
/// Hash collisions are verified with `anchor_eq`; duplicate keys are left to the
/// safe gap fallback rather than causing quadratic work.
fn patience_pairs(mine: &[DocBlock], theirs: &[DocBlock]) -> Vec<(usize, usize)> {
    use std::collections::HashMap;
    let mut theirs_keys: HashMap<u64, (usize, usize)> = HashMap::new();
    for (j, block) in theirs.iter().enumerate() {
        let entry = theirs_keys
            .entry(anchor_fingerprint(block))
            .or_insert((j, 0));
        entry.1 += 1;
    }
    let mut mine_counts: HashMap<u64, usize> = HashMap::new();
    for block in mine {
        *mine_counts.entry(anchor_fingerprint(block)).or_default() += 1;
    }
    let candidates: Vec<(usize, usize)> = mine
        .iter()
        .enumerate()
        .filter_map(|(i, block)| {
            let key = anchor_fingerprint(block);
            let &(j, theirs_count) = theirs_keys.get(&key)?;
            (mine_counts.get(&key) == Some(&1) && theirs_count == 1 && anchor_eq(block, &theirs[j]))
                .then_some((i, j))
        })
        .collect();
    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![usize::MAX; candidates.len()];
    for (idx, &(_, j)) in candidates.iter().enumerate() {
        let pos = tails.partition_point(|&tail| candidates[tail].1 < j);
        if pos > 0 {
            previous[idx] = tails[pos - 1];
        }
        if pos == tails.len() {
            tails.push(idx);
        } else {
            tails[pos] = idx;
        }
    }
    let Some(&last) = tails.last() else {
        return Vec::new();
    };
    let mut chain = Vec::with_capacity(tails.len());
    let mut cursor = last;
    loop {
        chain.push(candidates[cursor]);
        if previous[cursor] == usize::MAX {
            break;
        }
        cursor = previous[cursor];
    }
    chain.reverse();
    chain
}

/// Longest common subsequence of two sibling lists under [`anchor_eq`], returned
/// as sorted `(mine_idx, theirs_idx)` pairs. Hirschberg keeps peak memory linear
/// rather than allocating a `(n+1)*(m+1)` matrix (400 MiB at 10k×10k).
fn lcs_pairs(mine: &[DocBlock], theirs: &[DocBlock]) -> Vec<(usize, usize)> {
    if mine.len().saturating_mul(theirs.len()) > MAX_LCS_COMPARISONS {
        return patience_pairs(mine, theirs);
    }
    let mut out = Vec::new();
    hirschberg_pairs(mine, theirs, 0, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc;

    fn parse(s: &str) -> doc::Document {
        doc::parse(s)
    }

    fn kinds(rows: &[DiffRow]) -> Vec<(String, RowKind)> {
        let mut out = Vec::new();
        fn rec(rows: &[DiffRow], out: &mut Vec<(String, RowKind)>) {
            for r in rows {
                out.push((r.id.clone(), r.kind));
                rec(&r.children, out);
            }
        }
        rec(rows, &mut out);
        out
    }

    #[test]
    fn identical_docs_are_all_unchanged() {
        let a = parse("- one\n- two\n\t- child\n");
        let d = diff_docs(&a, &a);
        assert!(d.blocks_identical);
        assert!(d.rows.iter().all(|r| r.kind == RowKind::Unchanged));
        assert!(!d.pre_differs);
    }

    #[test]
    fn lcs_budget_stays_at_the_measured_ceiling() {
        // Audit 2026-08-24: at a 1e6 budget the exact branch spent ~0.95 s on
        // an all-conflicted 1000×1000 sibling pair while the patience fallback
        // handled 1001×1001 in 14 ms. Raising the budget re-opens that cliff.
        assert!(MAX_LCS_COMPARISONS <= 250_000);
    }

    #[test]
    fn alignment_agrees_on_both_sides_of_the_lcs_budget() {
        // 500×500 = 250_000 runs the exact branch, 501×501 the patience
        // fallback; on a pair whose shared anchors are unique both must find
        // the same alignment (shared rows unchanged, the rest added/removed).
        for n in [500usize, 501] {
            let mine: String = (0..n)
                .map(|i| {
                    if i % 2 == 0 {
                        format!("- shared anchor {i}\n")
                    } else {
                        format!("- winner only {i}\n")
                    }
                })
                .collect();
            let theirs: String = (0..n)
                .map(|i| {
                    if i % 2 == 0 {
                        format!("- shared anchor {i}\n")
                    } else {
                        format!("- conflict only {i}\n")
                    }
                })
                .collect();
            let d = diff_docs(&parse(&mine), &parse(&theirs));
            let k = kinds(&d.rows);
            let count = |kind: RowKind| k.iter().filter(|(_, x)| *x == kind).count();
            let shared = n.div_ceil(2);
            assert_eq!(count(RowKind::Unchanged), shared, "n = {n}");
            assert_eq!(count(RowKind::Added), n - shared, "n = {n}");
            assert_eq!(count(RowKind::Removed), n - shared, "n = {n}");
        }
    }

    #[test]
    fn similarity_pairing_is_bounded_on_giant_single_line_blocks() {
        // A single-line block's "first line" is its whole body. Before the
        // similarity key cap this pair cost ~92 s (audit 2026-08-24, 256 KB);
        // with the cap the equal capped prefixes still pair the rows.
        let filler = "x".repeat(256 * 1024);
        let mine = format!("- {filler} left\n");
        let theirs = format!("- {filler} right\n");
        let start = std::time::Instant::now();
        let d = diff_docs(&parse(&mine), &parse(&theirs));
        assert!(
            d.rows.iter().any(|r| r.kind == RowKind::Modified),
            "capped keys share a 512-char prefix and must still pair"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "similarity must not be quadratic in first-line length"
        );
    }

    #[test]
    fn added_and_removed_without_ids() {
        // winner has A, B; conflict has A, C.  B is added (winner-only), C removed.
        let mine = parse("- alpha\n- beta\n");
        let theirs = parse("- alpha\n- gamma\n");
        let d = diff_docs(&mine, &theirs);
        let k = kinds(&d.rows);
        // alpha unchanged; beta vs gamma are dissimilar → Added + Removed
        assert_eq!(k[0].1, RowKind::Unchanged);
        let has_added = k.iter().any(|(_, kind)| *kind == RowKind::Added);
        let has_removed = k.iter().any(|(_, kind)| *kind == RowKind::Removed);
        assert!(has_added && has_removed, "kinds: {k:?}");
        assert!(!d.blocks_identical);
    }

    #[test]
    fn large_unrelated_flat_conflict_uses_bounded_fallback_without_data_loss() {
        let mine: Vec<DocBlock> = (0..1100)
            .map(|i| DocBlock::new(format!("mine unique block {i}")))
            .collect();
        let theirs: Vec<DocBlock> = (0..1100)
            .map(|i| DocBlock::new(format!("theirs unrelated block {i}")))
            .collect();
        let rows = diff_blocks(&mine, &theirs);
        assert_eq!(rows.len(), 2200);
        assert_eq!(
            rows.iter().filter(|row| row.kind == RowKind::Added).count(),
            1100
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.kind == RowKind::Removed)
                .count(),
            1100
        );
        assert_eq!(
            merge_blocks(&mine, &theirs, &std::collections::HashMap::new()).unwrap(),
            mine
        );
    }

    #[test]
    fn modified_by_similar_first_line() {
        let mine = parse("- the quick brown fox jumps\n");
        let theirs = parse("- the quick brown fox leaps\n");
        let d = diff_docs(&mine, &theirs);
        assert_eq!(d.rows.len(), 1);
        assert_eq!(d.rows[0].kind, RowKind::Modified);
    }

    #[test]
    fn modified_matched_by_id_even_when_text_differs() {
        // Same id::, very different text → matched as Modified (id anchor), not
        // Added+Removed.
        let mine = parse("- hello world\n  id:: aaaaaaaa-0000-0000-0000-0000000000ab\n");
        let theirs =
            parse("- totally rewritten line\n  id:: aaaaaaaa-0000-0000-0000-0000000000ab\n");
        let d = diff_docs(&mine, &theirs);
        assert_eq!(d.rows.len(), 1);
        assert_eq!(d.rows[0].kind, RowKind::Modified);
    }

    #[test]
    fn child_change_recurses() {
        // A small edit in one child (one typo) → the child pairs as Modified via
        // similarity, and its parent is Modified because its subtree changed.
        let mine = parse("- parent\n\t- the first child\n\t- the second child line\n");
        let theirs = parse("- parent\n\t- the first child\n\t- the second child lyne\n");
        let d = diff_docs(&mine, &theirs);
        assert_eq!(d.rows.len(), 1);
        assert_eq!(d.rows[0].kind, RowKind::Modified);
        let ck = kinds(&d.rows[0].children);
        assert!(ck.iter().any(|(_, k)| *k == RowKind::Unchanged), "{ck:?}");
        assert!(ck.iter().any(|(_, k)| *k == RowKind::Modified), "{ck:?}");
    }

    #[test]
    fn large_child_edit_shows_add_remove_not_wrong_pairing() {
        // A change too big to be confidently the "same" block → add+remove, never a
        // misleading Modified pairing (the plan's data-safety default).
        let mine = parse("- parent\n\t- kid two\n");
        let theirs = parse("- parent\n\t- kid TWO totally rewritten and much longer now\n");
        let d = diff_docs(&mine, &theirs);
        let ck = kinds(&d.rows[0].children);
        assert!(ck.iter().any(|(_, k)| *k == RowKind::Added), "{ck:?}");
        assert!(ck.iter().any(|(_, k)| *k == RowKind::Removed), "{ck:?}");
        assert!(!ck.iter().any(|(_, k)| *k == RowKind::Modified), "{ck:?}");
    }

    #[test]
    fn reordered_blocks_keep_one_anchor() {
        // winner: A B C ; conflict: A C B  → LCS keeps A and one of B/C as anchors,
        // the other becomes an Added/Removed pair. No crash, order preserved.
        let mine = parse("- aaa\n- bbb\n- ccc\n");
        let theirs = parse("- aaa\n- ccc\n- bbb\n");
        let d = diff_docs(&mine, &theirs);
        assert_eq!(
            d.rows
                .iter()
                .filter(|r| r.kind == RowKind::Unchanged)
                .count()
                >= 2,
            true
        );
        assert!(!d.blocks_identical);
    }

    // --- merge -------------------------------------------------------------

    use std::collections::HashMap;

    fn raws(blocks: &[DocBlock]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| b.raw.lines().next().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn merge_default_keeps_winner() {
        // No decisions → winner wins: modified keeps mine's body, added kept,
        // removed dropped. Result equals the winner's blocks.
        let mine = parse("- alpha\n- the quick brown fox jumps\n- winner only\n");
        let theirs = parse("- alpha\n- the quick brown fox leaps\n- conflict only\n");
        let merged = merge_blocks(&mine.roots, &theirs.roots, &HashMap::new()).unwrap();
        assert_eq!(
            raws(&merged),
            vec!["alpha", "the quick brown fox jumps", "winner only"]
        );
    }

    #[test]
    fn merge_keep_theirs_on_modified() {
        let mine = parse("- the quick brown fox jumps\n");
        let theirs = parse("- the quick brown fox leaps\n");
        // The single modified root has id "0".
        let dec = HashMap::from([("0".to_string(), "theirs".to_string())]);
        let merged = merge_blocks(&mine.roots, &theirs.roots, &dec).unwrap();
        assert_eq!(raws(&merged), vec!["the quick brown fox leaps"]);
    }

    #[test]
    fn merge_pull_in_removed_block() {
        // Removed (conflict-only) block pulled in with keep-theirs.
        let mine = parse("- alpha\n");
        let theirs = parse("- alpha\n- conflict only line\n");
        let d = diff_docs(&mine, &theirs);
        // Find the Removed row's id.
        let removed_id = d
            .rows
            .iter()
            .find(|r| r.kind == RowKind::Removed)
            .map(|r| r.id.clone())
            .expect("a removed row");
        let dec = HashMap::from([(removed_id, "theirs".to_string())]);
        let merged = merge_blocks(&mine.roots, &theirs.roots, &dec).unwrap();
        assert_eq!(raws(&merged), vec!["alpha", "conflict only line"]);
    }

    #[test]
    fn merge_keep_both_strips_duplicate_id() {
        // Same id::, both kept → the conflict copy loses the id:: so it doesn't
        // duplicate the winner's on disk.
        let mine = parse("- winner text\n  id:: aaaaaaaa-0000-0000-0000-0000000000cd\n");
        let theirs = parse("- their text\n  id:: aaaaaaaa-0000-0000-0000-0000000000cd\n");
        let dec = HashMap::from([("0".to_string(), "both".to_string())]);
        let merged = merge_blocks(&mine.roots, &theirs.roots, &dec).unwrap();
        assert_eq!(merged.len(), 2);
        // Winner keeps its id::; the pulled-in copy does not.
        assert!(merged[0]
            .raw
            .contains("id:: aaaaaaaa-0000-0000-0000-0000000000cd"));
        assert!(
            !merged[1].raw.contains("id::"),
            "dup id leaked: {:?}",
            merged[1].raw
        );
        assert!(merged[1].raw.contains("their text"));
    }

    // --- 3-way (diff3 against a base) ---------------------------------------

    /// Flatten (kind, verdict, suggestion) over the whole row tree.
    fn table(rows: &[DiffRow]) -> Vec<(RowKind, Option<Diff3Verdict>, Option<String>)> {
        let mut out = Vec::new();
        fn rec(rows: &[DiffRow], out: &mut Vec<(RowKind, Option<Diff3Verdict>, Option<String>)>) {
            for r in rows {
                out.push((r.kind, r.verdict, r.suggestion.clone()));
                rec(&r.children, out);
            }
        }
        rec(rows, &mut out);
        out
    }

    #[test]
    fn diff3_identical_sides_have_no_verdicts() {
        let base = parse("- one\n- two\n");
        let d = diff3_docs(&base, &base, &base);
        assert!(d.three_way);
        assert!(d.blocks_identical);
        assert!(table(&d.rows)
            .iter()
            .all(|(_, v, s)| v.is_none() && s.is_none()));
    }

    #[test]
    fn diff3_mine_only_edit_suggests_mine() {
        let base = parse("- alpha\n- the quick brown fox jumps\n");
        let mine = parse("- alpha\n- the quick brown fox JUMPED\n");
        let theirs = base.clone();
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        assert!(
            t.iter().any(|(k, v, s)| *k == RowKind::Modified
                && *v == Some(Diff3Verdict::MineOnly)
                && s.as_deref() == Some("mine")),
            "{t:?}"
        );
    }

    #[test]
    fn diff3_theirs_only_edit_suggests_theirs() {
        let base = parse("- alpha\n- the quick brown fox jumps\n");
        let mine = base.clone();
        let theirs = parse("- alpha\n- the quick brown fox LEAPT\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        assert!(
            t.iter().any(|(k, v, s)| *k == RowKind::Modified
                && *v == Some(Diff3Verdict::TheirsOnly)
                && s.as_deref() == Some("theirs")),
            "{t:?}"
        );
    }

    #[test]
    fn diff3_both_edited_is_a_true_conflict_without_suggestion() {
        let base = parse("- the quick brown fox jumps\n");
        let mine = parse("- the quick brown fox jumped\n");
        let theirs = parse("- the quick brown fox leaped\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].1, Some(Diff3Verdict::BothChanged));
        assert_eq!(t[0].2, None);
    }

    #[test]
    fn diff3_independent_edits_to_different_blocks_suggest_each_side() {
        // The genuinely mergeable case: mine edited block one, theirs edited
        // block two — both rows get a confident suggestion.
        let base = parse("- first shared line here\n- second shared line here\n");
        let mine = parse("- first shared line herz\n- second shared line here\n");
        let theirs = parse("- first shared line here\n- second shared line herz\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let suggestions: Vec<Option<String>> =
            table(&d.rows).into_iter().map(|(_, _, s)| s).collect();
        assert_eq!(
            suggestions,
            vec![Some("mine".to_string()), Some("theirs".to_string())],
            "{:?}",
            table(&d.rows)
        );
    }

    #[test]
    fn diff3_addition_by_each_side_is_kept() {
        // mine added a block absent from base → Added row suggests keeping it.
        let base = parse("- alpha\n");
        let mine = parse("- alpha\n- winner addition\n");
        let theirs = parse("- alpha\n- copy addition\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        assert!(
            t.iter().any(|(k, v, s)| *k == RowKind::Added
                && *v == Some(Diff3Verdict::MineOnly)
                && s.as_deref() == Some("mine")),
            "{t:?}"
        );
        // theirs added one too → Removed row suggests pulling it in.
        assert!(
            t.iter().any(|(k, v, s)| *k == RowKind::Removed
                && *v == Some(Diff3Verdict::TheirsOnly)
                && s.as_deref() == Some("theirs")),
            "{t:?}"
        );
    }

    #[test]
    fn diff3_deletion_by_theirs_of_unchanged_block_suggests_the_deletion() {
        // theirs deleted a block mine left untouched → the Added row (winner-
        // only) is really a theirs-side deletion → suggest "theirs" (drop).
        let base = parse("- alpha\n- doomed block\n");
        let mine = base.clone();
        let theirs = parse("- alpha\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        assert!(
            t.iter().any(|(k, v, s)| *k == RowKind::Added
                && *v == Some(Diff3Verdict::TheirsOnly)
                && s.as_deref() == Some("theirs")),
            "{t:?}"
        );
    }

    #[test]
    fn diff3_deletion_by_mine_of_unchanged_block_suggests_skipping() {
        // mine deleted it, theirs still has it unchanged → Removed row suggests
        // "mine" (keep it deleted).
        let base = parse("- alpha\n- gone from winner\n");
        let mine = parse("- alpha\n");
        let theirs = base.clone();
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        assert!(
            t.iter().any(|(k, v, s)| *k == RowKind::Removed
                && *v == Some(Diff3Verdict::MineOnly)
                && s.as_deref() == Some("mine")),
            "{t:?}"
        );
    }

    #[test]
    fn diff3_delete_vs_edit_is_a_conflict() {
        // mine edited the block, theirs deleted it → both changed → no suggestion.
        let base = parse("- alpha\n- contested block text\n");
        let mine = parse("- alpha\n- contested block texz\n");
        let theirs = parse("- alpha\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        assert!(
            t.iter().any(|(k, v, s)| *k == RowKind::Added
                && *v == Some(Diff3Verdict::BothChanged)
                && s.is_none()),
            "{t:?}"
        );
    }

    #[test]
    fn diff3_move_by_theirs_suggests_enacting_the_move() {
        // theirs moved B after C; base == mine. The diff shows B as an Added
        // (old position) + Removed (new position) pair; 3-way suggests "theirs"
        // on BOTH rows — drop at the old spot, pull in at the new — which
        // together enact the move.
        let base = parse("- aaa\n- bbb\n- ccc\n");
        let mine = base.clone();
        let theirs = parse("- aaa\n- ccc\n- bbb\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let t = table(&d.rows);
        let added: Vec<_> = t.iter().filter(|(k, _, _)| *k == RowKind::Added).collect();
        let removed: Vec<_> = t
            .iter()
            .filter(|(k, _, _)| *k == RowKind::Removed)
            .collect();
        assert_eq!(added.len(), 1, "{t:?}");
        assert_eq!(removed.len(), 1, "{t:?}");
        assert_eq!(added[0].2.as_deref(), Some("theirs"), "{t:?}");
        assert_eq!(removed[0].2.as_deref(), Some("theirs"), "{t:?}");
    }

    #[test]
    fn diff3_nested_child_edit_classifies_the_child_not_the_parent() {
        let base = parse("- parent\n\t- the first child line\n\t- the second child line\n");
        let mine = base.clone();
        let theirs = parse("- parent\n\t- the first child line\n\t- the second child lyne\n");
        let d = diff3_docs(&base, &mine, &theirs);
        // Parent row: modified subtree but identical body → no row verdict.
        assert_eq!(d.rows.len(), 1);
        assert_eq!(d.rows[0].kind, RowKind::Modified);
        assert_eq!(d.rows[0].verdict, None);
        let ct = table(&d.rows[0].children);
        assert!(
            ct.iter().any(|(k, v, s)| *k == RowKind::Modified
                && *v == Some(Diff3Verdict::TheirsOnly)
                && s.as_deref() == Some("theirs")),
            "{ct:?}"
        );
    }

    #[test]
    fn two_way_diff_carries_no_suggestions() {
        // The 2-way path stays suggestion-free — the fallback when no base exists.
        let mine = parse("- the quick brown fox jumps\n");
        let theirs = parse("- the quick brown fox leaps\n");
        let d = diff_docs(&mine, &theirs);
        assert!(!d.three_way);
        assert!(table(&d.rows)
            .iter()
            .all(|(_, v, s)| v.is_none() && s.is_none()));
    }

    #[test]
    fn merge_drop_added_block() {
        let mine = parse("- alpha\n- winner only\n");
        let theirs = parse("- alpha\n");
        let d = diff_docs(&mine, &theirs);
        let added_id = d
            .rows
            .iter()
            .find(|r| r.kind == RowKind::Added)
            .map(|r| r.id.clone())
            .expect("an added row");
        let dec = HashMap::from([(added_id, "theirs".to_string())]); // drop it
        let merged = merge_blocks(&mine.roots, &theirs.roots, &dec).unwrap();
        assert_eq!(raws(&merged), vec!["alpha"]);
    }

    // --- the fourth outcome: a proposed merged body --------------------------

    const ID: &str = "  id:: aaaaaaaa-0000-0000-0000-0000000000ab\n";

    /// Flagship: mine deleted the trailing " 5", theirs appended " kk" right
    /// after it. The two hunks only TOUCH, so the relaxed rule composes them.
    #[test]
    fn diff3_disjoint_edits_offer_a_merged_body() {
        let base = parse(&format!("- Desktop 5\n{ID}"));
        let mine = parse(&format!("- Desktop\n{ID}"));
        let theirs = parse(&format!("- Desktop 5 kk\n{ID}"));
        let d = diff3_docs(&base, &mine, &theirs);
        assert_eq!(d.rows.len(), 1);
        let row = &d.rows[0];
        assert_eq!(row.kind, RowKind::Modified);
        assert_eq!(row.verdict, Some(Diff3Verdict::BothChanged));
        assert_eq!(row.suggestion.as_deref(), Some("merged"));
        let proposal = row.merged.as_ref().expect("a merged proposal");
        assert_eq!(proposal.source, MergedSource::Computed);
        assert!(
            proposal.text.starts_with("Desktop kk"),
            "{:?}",
            proposal.text
        );
        // The DTO the UI receives (mirrored in `src/types.ts`).
        let wire = serde_json::to_value(row).unwrap();
        assert_eq!(wire["suggestion"], "merged");
        assert_eq!(wire["merged"]["source"], "computed");
        assert_eq!(wire["merged"]["text"], proposal.text.as_str());
        // Absent, not null, on rows without a proposal (the field is skipped).
        let plain = serde_json::to_value(&diff_docs(&mine, &theirs).rows[0]).unwrap();
        assert!(plain.get("merged").is_none(), "{plain}");
    }

    /// Argument order must not change the proposal: `compose` orders hunks by
    /// their base range, not by which side produced them.
    #[test]
    fn diff3_merged_body_is_the_same_with_the_sides_swapped() {
        let base = parse(&format!("- Desktop 5\n{ID}"));
        let mine = parse(&format!("- Desktop\n{ID}"));
        let theirs = parse(&format!("- Desktop 5 kk\n{ID}"));
        let one = diff3_docs(&base, &mine, &theirs).rows[0]
            .merged
            .as_ref()
            .map(|m| m.text.clone());
        let two = diff3_docs(&base, &theirs, &mine).rows[0]
            .merged
            .as_ref()
            .map(|m| m.text.clone());
        assert_eq!(one, two);
        assert!(one.is_some());
    }

    #[test]
    fn diff3_overlapping_edits_offer_no_merged_body() {
        let base = parse(&format!("- the quick brown fox jumps\n{ID}"));
        let mine = parse(&format!("- the quick brown fox jumped\n{ID}"));
        let theirs = parse(&format!("- the quick brown fox leaped\n{ID}"));
        let d = diff3_docs(&base, &mine, &theirs);
        assert_eq!(d.rows[0].verdict, Some(Diff3Verdict::BothChanged));
        assert!(d.rows[0].merged.is_none());
        assert!(d.rows[0].suggestion.is_none());
    }

    #[test]
    fn two_way_diff_never_offers_a_merged_body() {
        let d = diff_docs(&parse("- Desktop\n"), &parse("- Desktop 5 kk\n"));
        fn rec(rows: &[DiffRow]) {
            for row in rows {
                assert!(row.merged.is_none());
                rec(&row.children);
            }
        }
        rec(&d.rows);
    }

    /// One-sided rows keep their plain suggestion: a merge is pointless when
    /// only one side moved, and `merge_disjoint` declines it outright.
    #[test]
    fn diff3_one_sided_rows_keep_their_side_suggestion() {
        let base = parse(&format!("- Desktop 5\n{ID}"));
        let mine = parse(&format!("- Desktop\n{ID}"));
        let d = diff3_docs(&base, &mine, &base);
        assert_eq!(d.rows[0].suggestion.as_deref(), Some("mine"));
        assert!(d.rows[0].merged.is_none());
    }

    /// Both sides equal a base block yet differ from each other: the two base
    /// maps paired the row with DIFFERENT base blocks (duplicate-ish content).
    /// An ambiguous base is no base — conflict, and never a merged proposal.
    #[test]
    fn diff3_inconsistent_base_maps_offer_no_merged_body() {
        let base = parse("- the shared sentence with alpha\n- the shared sentence with beta\n");
        let mine = parse("- the shared sentence with alpha\n");
        let theirs = parse("- the shared sentence with beta\n");
        let d = diff3_docs(&base, &mine, &theirs);
        let modified: Vec<&DiffRow> = d
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Modified)
            .collect();
        assert_eq!(modified.len(), 1, "{:?}", kinds(&d.rows));
        assert_eq!(modified[0].verdict, Some(Diff3Verdict::BothChanged));
        assert!(modified[0].merged.is_none());
        assert!(modified[0].suggestion.is_none());
    }

    /// The two base maps each supply a block, but not the SAME body.
    #[test]
    fn merged_body_refuses_two_disagreeing_base_bodies() {
        let base_mine = DocBlock::new("Desktop 5");
        let base_theirs = DocBlock::new("Desktop 6");
        let mine = DocBlock::new("Desktop");
        let theirs = DocBlock::new("Desktop 5 kk");
        assert!(merged_body(Some(&base_mine), Some(&base_theirs), &mine, &theirs).is_none());
        // Agreeing bases (and only then) produce the proposal.
        assert!(merged_body(Some(&base_mine), Some(&base_mine), &mine, &theirs).is_some());
        // A missing base on either side is no base at all.
        assert!(merged_body(None, Some(&base_mine), &mine, &theirs).is_none());
        assert!(merged_body(Some(&base_mine), None, &mine, &theirs).is_none());
    }

    // --- validity gate -------------------------------------------------------

    #[test]
    fn the_gate_accepts_an_ordinary_one_block_body() {
        assert!(merged_body_is_valid("Desktop kk", false));
        assert!(merged_body_is_valid(
            "Desktop kk\nid:: aaaaaaaa-0000-0000-0000-0000000000ab",
            false
        ));
        assert!(merged_body_is_valid("Desktop kk", true));
    }

    /// A merged body whose continuation line became a bullet would re-parse as
    /// a parent with a CHILD — a different tree than the row the user decided.
    #[test]
    fn the_gate_rejects_a_body_that_would_split_into_two_blocks() {
        assert!(!merged_body_is_valid("alpha\n- beta gamma", false));
        assert!(!merged_body_is_valid("alpha\n* beta gamma", true));
    }

    /// Empty merged text: documented behavior, whatever the serializer does
    /// with an empty block — the gate is the single authority either way.
    #[test]
    fn the_gate_refuses_an_empty_merged_body() {
        // Complementary disjoint deletions compose to "" (audit 2026-08-24,
        // A9); emptying the block is a decision, never a merge proposal.
        assert!(!merged_body_is_valid("", false));
        assert!(!merged_body_is_valid("", true));
        assert!(!merged_body_is_valid("  ", false));
    }

    #[test]
    fn complementary_deletions_offer_no_merged_body() {
        // mine deletes "a" ([0,1) of the base), theirs deletes "b" ([1,2)) —
        // touching, so the relaxed rule composes them, to an EMPTY body the
        // gate must refuse.
        let base = parse("- ab\n");
        let mine = parse("- b\n");
        let theirs = parse("- a\n");
        assert_eq!(
            crate::text_merge::merge_disjoint("ab", "b", "a").as_deref(),
            Some(""),
            "the merge itself composes, so the gate is what is under test"
        );
        assert!(merged_body(
            Some(&base.roots[0]),
            Some(&base.roots[0]),
            &mine.roots[0],
            &theirs.roots[0]
        )
        .is_none());
    }

    /// End to end: a char merge that produces a second bullet is never offered.
    #[test]
    fn diff3_offers_no_merged_body_whose_reparse_would_split() {
        // base body "alpha\nXbeta gamma": mine drops the X, theirs turns the
        // continuation line into a bullet after it. The hunks merely touch, so
        // `merge_disjoint` composes them — and the GATE is what refuses.
        let base = parse("- alpha\n  Xbeta gamma\n");
        let mine = parse("- alpha\n  beta gamma\n");
        let theirs = parse("- alpha\n  X- beta gamma\n");
        assert_eq!(base.roots.len(), 1);
        assert_eq!(theirs.roots.len(), 1);
        assert_eq!(
            crate::text_merge::merge_disjoint(
                &base.roots[0].raw,
                &mine.roots[0].raw,
                &theirs.roots[0].raw
            )
            .as_deref(),
            Some("alpha\n- beta gamma"),
            "the merge itself must succeed, so the gate is what is under test"
        );
        let d = diff3_docs(&base, &mine, &theirs);
        assert_eq!(d.rows.len(), 1);
        assert!(d.rows[0].merged.is_none());
        assert!(d.rows[0].suggestion.is_none());
    }

    /// The org equivalent: a merged body starting a new headline would re-parse
    /// as two root blocks, so it is refused before Apply ever sees it.
    #[test]
    fn diff3_offers_no_org_merged_body_that_breaks_the_outline() {
        let base = crate::org::parse_org("* alpha\nXbeta gamma\n");
        let mine = crate::org::parse_org("* alpha\nbeta gamma\n");
        let theirs = crate::org::parse_org("* alpha\nX* beta gamma\n");
        assert_eq!(base.roots.len(), 1);
        assert_eq!(theirs.roots.len(), 1);
        let d = diff3_docs(&base, &mine, &theirs);
        assert_eq!(d.rows.len(), 1);
        assert!(d.rows[0].merged.is_none());
    }

    // --- apply ---------------------------------------------------------------

    #[test]
    fn merge3_applies_the_merged_body_and_honors_child_decisions() {
        let base = parse(&format!(
            "- Desktop 5\n{ID}\t- the child line here\n\t\tid:: aaaaaaaa-0000-0000-0000-0000000000cd\n"
        ));
        let mine = parse(&format!(
            "- Desktop\n{ID}\t- the child line here\n\t\tid:: aaaaaaaa-0000-0000-0000-0000000000cd\n"
        ));
        let theirs = parse(&format!(
            "- Desktop 5 kk\n{ID}\t- the child line THERE\n\t\tid:: aaaaaaaa-0000-0000-0000-0000000000cd\n"
        ));
        let d = diff3_docs(&base, &mine, &theirs);
        assert_eq!(d.rows[0].suggestion.as_deref(), Some("merged"));
        let dec = HashMap::from([
            ("0".to_string(), "merged".to_string()),
            ("0.0".to_string(), "theirs".to_string()),
        ]);
        let merged = merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &dec)
            .expect("the merged decision applies");
        assert_eq!(merged.len(), 1);
        assert!(
            merged[0].raw.starts_with("Desktop kk"),
            "{:?}",
            merged[0].raw
        );
        assert_eq!(merged[0].children.len(), 1);
        assert!(
            merged[0].children[0].raw.contains("THERE"),
            "{:?}",
            merged[0].children[0].raw
        );
    }

    #[test]
    fn merge3_without_a_base_refuses_a_merged_decision() {
        let mine = parse(&format!("- Desktop\n{ID}"));
        let theirs = parse(&format!("- Desktop 5 kk\n{ID}"));
        let dec = HashMap::from([("0".to_string(), "merged".to_string())]);
        let error = merge_blocks(&mine.roots, &theirs.roots, &dec).expect_err("no base");
        assert_eq!(error.row, "0");
        assert_eq!(error.reason, NO_BASE);
    }

    /// A row that was never offered a merge (one-sided change) refuses rather
    /// than falling back to a side the user did not choose.
    #[test]
    fn merge3_refuses_a_merged_decision_on_a_row_with_no_offer() {
        let base = parse(&format!("- Desktop 5\n{ID}"));
        let mine = parse(&format!("- Desktop\n{ID}"));
        let dec = HashMap::from([("0".to_string(), "merged".to_string())]);
        let error = merge_blocks3(Some(&base.roots), &mine.roots, &base.roots, None, &dec)
            .expect_err("theirs never moved");
        assert_eq!(error.reason, NOT_MERGEABLE);
    }

    /// A forged decision on a row whose merged body would not survive the gate.
    #[test]
    fn merge3_refuses_a_forged_merged_decision_that_fails_the_gate() {
        let base = parse("- alpha\n  Xbeta gamma\n");
        let mine = parse("- alpha\n  beta gamma\n");
        let theirs = parse("- alpha\n  X- beta gamma\n");
        let dec = HashMap::from([("0".to_string(), "merged".to_string())]);
        let error = merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &dec)
            .expect_err("the gate refuses");
        assert_eq!(error.reason, NOT_MERGEABLE);
    }

    #[test]
    fn merge3_refuses_a_merged_decision_on_an_added_or_removed_row() {
        let base = parse("- alpha\n");
        let mine = parse("- alpha\n- winner only line\n");
        let theirs = parse("- alpha\n- conflict only line\n");
        let d = diff3_docs(&base, &mine, &theirs);
        for row in d.rows.iter().filter(|r| r.kind != RowKind::Unchanged) {
            let dec = HashMap::from([(row.id.clone(), "merged".to_string())]);
            let error = merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &dec)
                .unwrap_err();
            assert_eq!(error.reason, NOT_A_MODIFIED_ROW, "row {}", row.id);
        }
    }

    /// Other decisions are untouched by the new arm: the 2-way merge still
    /// answers with a plain tree.
    #[test]
    fn merge3_with_a_base_leaves_the_other_decisions_alone() {
        let base = parse("- alpha\n- the quick brown fox jumps\n");
        let mine = parse("- alpha\n- the quick brown fox jumped\n");
        let theirs = parse("- alpha\n- the quick brown fox leaped\n");
        let dec = HashMap::from([("1".to_string(), "theirs".to_string())]);
        let merged =
            merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &dec).unwrap();
        assert_eq!(raws(&merged), vec!["alpha", "the quick brown fox leaped"]);
    }

    // --- the second source: the merge tool's own suggested resolution --------

    /// The overlapping-edit case the computed merge refuses (see
    /// `diff3_overlapping_edits_offer_no_merged_body`) — the only place an
    /// artifact can ever show up.
    fn overlapping() -> (
        crate::doc::Document,
        crate::doc::Document,
        crate::doc::Document,
    ) {
        (
            parse(&format!("- the quick brown fox jumps\n{ID}")),
            parse(&format!("- the quick brown fox jumped\n{ID}")),
            parse(&format!("- the quick brown fox leaped\n{ID}")),
        )
    }

    #[test]
    fn diff3_offers_the_artifact_where_the_computed_merge_declines() {
        let (base, mine, theirs) = overlapping();
        let artifact = parse(&format!("- the quick brown fox leapt\n{ID}"));
        let d = diff3_docs_with_artifact(&base, &mine, &theirs, Some(&artifact));
        assert_eq!(d.rows.len(), 1);
        let row = &d.rows[0];
        // The verdict is untouched — the artifact fills a conflict, it never
        // reclassifies one.
        assert_eq!(row.verdict, Some(Diff3Verdict::BothChanged));
        assert_eq!(row.suggestion.as_deref(), Some("merged"));
        let proposal = row.merged.as_ref().expect("an artifact proposal");
        assert_eq!(proposal.source, MergedSource::Artifact);
        assert!(proposal.text.starts_with("the quick brown fox leapt"));
        // On the wire for `src/types.ts`.
        let wire = serde_json::to_value(row).unwrap();
        assert_eq!(wire["merged"]["source"], "artifact");
        assert_eq!(wire["merged"]["text"], proposal.text.as_str());
    }

    /// PRECEDENCE: a composition of two disjoint edits is a stronger claim than
    /// a third party's guess, so the artifact is never consulted when the
    /// computation succeeds.
    #[test]
    fn diff3_computed_wins_over_an_available_artifact() {
        let base = parse(&format!("- Desktop 5\n{ID}"));
        let mine = parse(&format!("- Desktop\n{ID}"));
        let theirs = parse(&format!("- Desktop 5 kk\n{ID}"));
        let artifact = parse(&format!("- something the tool made up\n{ID}"));
        let row = &diff3_docs_with_artifact(&base, &mine, &theirs, Some(&artifact)).rows[0];
        let proposal = row.merged.as_ref().expect("a proposal");
        assert_eq!(proposal.source, MergedSource::Computed);
        assert!(
            proposal.text.starts_with("Desktop kk"),
            "{:?}",
            proposal.text
        );
    }

    /// A proposal equal to a side is one of the three choices the user already
    /// has; offering it a fourth time is noise, not an outcome.
    #[test]
    fn an_artifact_equal_to_a_side_is_not_offered() {
        let (base, mine, theirs) = overlapping();
        for same_as in [&mine, &theirs] {
            let d = diff3_docs_with_artifact(&base, &mine, &theirs, Some(same_as));
            assert!(d.rows[0].merged.is_none(), "{:?}", d.rows[0].merged);
            assert!(d.rows[0].suggestion.is_none());
        }
    }

    /// The same structural gate a computed body passes: a body that would come
    /// back as TWO blocks is never offered, whatever produced it.
    #[test]
    fn an_artifact_that_would_split_into_two_blocks_is_not_offered() {
        let (base, mine, theirs) = overlapping();
        let mut forged = parse(&format!("- the quick brown fox leapt\n{ID}"));
        // A body that re-parses as a bullet of its own.
        forged.roots[0].raw = "leapt\n- and a second bullet".to_string();
        assert!(!merged_body_is_valid(&forged.roots[0].raw, false));
        let d = diff3_docs_with_artifact(&base, &mine, &theirs, Some(&forged));
        assert!(d.rows[0].merged.is_none());
    }

    /// Two artifact blocks that disagree mean the two alignments paired the row
    /// with DIFFERENT suggestion blocks — an ambiguous artifact is no artifact.
    #[test]
    fn a_disagreeing_or_missing_artifact_pair_is_not_offered() {
        let a = parse("- alpha\n");
        let b = parse("- beta\n");
        let mine = parse(&format!("- the quick brown fox jumped\n{ID}"));
        let theirs = parse(&format!("- the quick brown fox leaped\n{ID}"));
        let (m, t) = (&mine.roots[0], &theirs.roots[0]);
        assert_eq!(
            artifact_body(Some(&a.roots[0]), Some(&b.roots[0]), m, t),
            None
        );
        // A missing block on either side is likewise no artifact.
        assert_eq!(artifact_body(Some(&a.roots[0]), None, m, t), None);
        assert_eq!(artifact_body(None, Some(&a.roots[0]), m, t), None);
    }

    /// A proposal needs a `BothChanged` verdict, which needs a base. A merge
    /// tool that wrote a suggestion but no common-ancestor section therefore
    /// surfaces nothing — no special case, just the 2-way path.
    #[test]
    fn an_artifact_without_a_base_never_surfaces() {
        let mine = parse(&format!("- the quick brown fox jumped\n{ID}"));
        let theirs = parse(&format!("- the quick brown fox leaped\n{ID}"));
        let d = diff_docs(&mine, &theirs);
        assert!(d.rows[0].merged.is_none());
        // And the apply side refuses a forged decision for want of a base.
        let dec = HashMap::from([("0".to_string(), "merged".to_string())]);
        let error = merge_blocks(&mine.roots, &theirs.roots, &dec).expect_err("no base");
        assert_eq!(error.reason, NO_BASE);
    }

    #[test]
    fn applying_a_confirmed_artifact_writes_the_suggested_body() {
        let (base, mine, theirs) = overlapping();
        let artifact = parse(&format!("- the quick brown fox leapt\n{ID}"));
        let offered = diff3_docs_with_artifact(&base, &mine, &theirs, Some(&artifact)).rows[0]
            .merged
            .as_ref()
            .expect("an artifact proposal")
            .text
            .clone();
        let dec = HashMap::from([("0".to_string(), "merged".to_string())]);
        let merged = merge_blocks3(
            Some(&base.roots),
            &mine.roots,
            &theirs.roots,
            Some(&artifact.roots),
            &dec,
        )
        .unwrap();
        assert_eq!(merged.len(), 1);
        // Byte-for-byte the body the user was shown, id:: and all.
        assert_eq!(merged[0].raw, offered);
        assert_eq!(merged[0].raw, artifact.roots[0].raw);
    }

    /// Determinism the other way round: with a computed body available the
    /// apply writes THAT, never the artifact — the same precedence the diff
    /// used to pick what the user saw.
    #[test]
    fn applying_prefers_the_computed_body_over_the_artifact() {
        let base = parse(&format!("- Desktop 5\n{ID}"));
        let mine = parse(&format!("- Desktop\n{ID}"));
        let theirs = parse(&format!("- Desktop 5 kk\n{ID}"));
        let artifact = parse(&format!("- something the tool made up\n{ID}"));
        let dec = HashMap::from([("0".to_string(), "merged".to_string())]);
        let merged = merge_blocks3(
            Some(&base.roots),
            &mine.roots,
            &theirs.roots,
            Some(&artifact.roots),
            &dec,
        )
        .unwrap();
        assert!(
            merged[0].raw.starts_with("Desktop kk"),
            "{:?}",
            merged[0].raw
        );
    }

    /// A forged `"merged"` where NEITHER source can supply a body refuses the
    /// whole resolve — it never falls back to a side.
    #[test]
    fn a_forged_merged_decision_refuses_when_both_sources_decline() {
        let (base, mine, theirs) = overlapping();
        let dec = HashMap::from([("0".to_string(), "merged".to_string())]);
        // No artifact at all.
        let error = merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &dec)
            .expect_err("nothing to merge");
        assert_eq!(error.row, "0");
        assert_eq!(error.reason, NOT_MERGEABLE);
        // An artifact that duplicates a side is not a fourth outcome either.
        let error = merge_blocks3(
            Some(&base.roots),
            &mine.roots,
            &theirs.roots,
            Some(&mine.roots),
            &dec,
        )
        .expect_err("the artifact duplicates mine");
        assert_eq!(error.reason, NOT_MERGEABLE);
    }

    /// Children of an artifact-merged row keep their own decisions, exactly as
    /// for a computed one.
    #[test]
    fn an_artifact_merged_row_keeps_its_children_decisions() {
        const KID: &str = "\t\tid:: aaaaaaaa-0000-0000-0000-0000000000cd\n";
        let page = |body: &str, kid: &str| {
            parse(&format!("- {body}\n{ID}\t- the child line {kid}\n{KID}"))
        };
        let base = page("the quick brown fox jumps", "here");
        let mine = page("the quick brown fox jumped", "here");
        let theirs = page("the quick brown fox leaped", "THERE");
        let artifact = page("the quick brown fox leapt", "here");
        let d = diff3_docs_with_artifact(&base, &mine, &theirs, Some(&artifact));
        assert_eq!(
            d.rows[0].merged.as_ref().map(|m| m.source),
            Some(MergedSource::Artifact)
        );
        let dec = HashMap::from([
            ("0".to_string(), "merged".to_string()),
            ("0.0".to_string(), "theirs".to_string()),
        ]);
        let merged = merge_blocks3(
            Some(&base.roots),
            &mine.roots,
            &theirs.roots,
            Some(&artifact.roots),
            &dec,
        )
        .unwrap();
        assert!(merged[0].raw.starts_with("the quick brown fox leapt"));
        assert_eq!(raws(&merged[0].children), vec!["the child line THERE"]);
    }
}
