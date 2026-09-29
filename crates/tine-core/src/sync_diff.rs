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
    /// Identity (sha256 hex) of the Concord-ledger base a sync-copy 3-way
    /// alignment and its `"merged"` proposals were computed from. The resolve
    /// requires it back, so a base that changed between diff and apply can
    /// never silently substitute a merged body the user was not shown. `None`
    /// on 2-way diffs and on marker diffs, whose base is reconstructed from the
    /// same file `base_rev` already pins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_base_rev: Option<String>,
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
        merge_base_rev: None,
    }
}

/// Build the full 3-way page-level diff: the SAME mine/theirs alignment as
/// [`diff_docs`] (so row ids stay compatible with [`merge_blocks`]), with each
/// row additionally classified against `base` — the common ancestor both sides
/// descend from (in og: the diff3 base a VCS merge left in the file itself, or the Concord
/// ledger's last-agreed text for a sync copy). Non-conflicting rows carry a `suggestion`
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
        merge_base_rev: None,
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
    let nodes = align_nodes(mine, theirs, "");
    let mut owned = std::collections::HashSet::new();
    owned_ids(&nodes, decisions, &mut owned);
    let ctx = MergeCtx {
        decisions,
        bases,
        artifacts,
        owned: &owned,
    };
    nodes_to_merged(&nodes, &ctx)
}

struct MergeCtx<'a> {
    decisions: &'a std::collections::HashMap<String, String>,
    bases: Option<(&'a BasePairs<'a>, &'a BasePairs<'a>)>,
    artifacts: Option<(&'a BasePairs<'a>, &'a BasePairs<'a>)>,
    /// Persisted ids the merged page keeps on a block in its own place (every
    /// output block except a conflict copy pulled in beside the winner).
    owned: &'a std::collections::HashSet<String>,
}

/// Collect the persisted ids [`nodes_to_merged`] writes on non-copy blocks, so
/// a conflict copy drops exactly the ids already held (I-11: a reorder aligns
/// one id as Added + Removed; keep-both must not write it twice). A copy whose
/// twin the user dropped keeps its id, so references still resolve.
fn owned_ids(
    nodes: &[Node],
    decisions: &std::collections::HashMap<String, String>,
    owned: &mut std::collections::HashSet<String>,
) {
    fn subtree(b: &DocBlock, owned: &mut std::collections::HashSet<String>) {
        owned.extend(persisted_id(b));
        for child in &b.children {
            subtree(child, owned);
        }
    }
    for n in nodes {
        match n {
            Node::Both {
                id,
                mine,
                theirs,
                modified,
                children,
            } => match (*modified, decision_for(decisions, id)) {
                (false, _) | (true, Decision::Both) => subtree(mine, owned),
                (true, Decision::Theirs) => {
                    owned.extend(persisted_id(theirs));
                    owned_ids(children, decisions, owned);
                }
                (true, _) => {
                    owned.extend(persisted_id(mine));
                    owned_ids(children, decisions, owned);
                }
            },
            Node::Mine { id, block } => {
                if decision_for(decisions, id) != Decision::Theirs {
                    subtree(block, owned);
                }
            }
            Node::Theirs { .. } => {}
        }
    }
}

fn nodes_to_merged(nodes: &[Node], ctx: &MergeCtx) -> Result<Vec<DocBlock>, MergeRefused> {
    let MergeCtx {
        decisions,
        bases,
        artifacts,
        owned,
    } = *ctx;
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
                    Decision::Mine => out.push(rebuild(mine, nodes_to_merged(children, ctx)?)),
                    Decision::Theirs => out.push(rebuild(theirs, nodes_to_merged(children, ctx)?)),
                    Decision::Both => {
                        out.push((*mine).clone());
                        // Fresh block — must not duplicate the winner's id:: on disk.
                        out.push(strip_held_ids(theirs, owned));
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
                            nodes_to_merged(children, ctx)?,
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
                // Removed (conflict-only): pulled in on keep-theirs / keep-both.
                // A reorder can align the same id as Added elsewhere, so an id the
                // merged page already holds is dropped from the copy (I-11).
                match decision_for(decisions, id) {
                    Decision::Merged => return Err(refused(id, NOT_A_MODIFIED_ROW)),
                    Decision::Theirs | Decision::Both => out.push(strip_held_ids(block, owned)),
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

/// Deep-copy a conflict-side block, dropping each persisted id the merged page
/// already holds (`owned`), so keeping the copy beside the winner can't write an
/// id twice. The id is removed where the block stores it: a Markdown `id::`
/// line, or the `:id:` entry of an Org `:PROPERTIES:` drawer (an emptied drawer
/// goes too). Other bytes, line endings included, are kept.
fn strip_held_ids(b: &DocBlock, owned: &std::collections::HashSet<String>) -> DocBlock {
    let raw = match persisted_id(b).filter(|id| owned.contains(id)) {
        Some(id) => without_id_line(&b.raw, b.is_org, &id),
        None => b.raw.clone(),
    };
    let mut nb = DocBlock::new(raw);
    nb.is_org = b.is_org;
    nb.children = b
        .children
        .iter()
        .map(|c| strip_held_ids(c, owned))
        .collect();
    nb
}

fn without_id_line(raw: &str, is_org: bool, id: &str) -> String {
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    fn body(l: &str) -> &str {
        l.trim_end_matches(['\r', '\n'])
    }
    let is_id = |l: &str| {
        if is_org {
            let t = body(l).trim();
            t.len() > 4 && t[..4].eq_ignore_ascii_case(":id:") && t[4..].trim() == id
        } else {
            crate::doc::parse_property_line(body(l))
                .is_some_and(|(k, v)| k.eq_ignore_ascii_case("id") && v.trim() == id)
        }
    };
    let Some(at) = lines.iter().position(|l| is_id(l)) else {
        return raw.to_owned();
    };
    let mut drop = vec![at];
    let marker = |i: usize, m: &str| {
        lines
            .get(i)
            .is_some_and(|l| body(l).trim().eq_ignore_ascii_case(m))
    };
    if is_org && at > 0 && marker(at - 1, ":PROPERTIES:") && marker(at + 1, ":END:") {
        drop = vec![at - 1, at, at + 1];
    }
    let last_dropped = *drop.last().unwrap() == lines.len() - 1;
    let mut out: String = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, l)| *l)
        .collect();
    // Dropping a final line without a newline must not leave one behind.
    if last_dropped && !raw.ends_with('\n') {
        let ending = if out.ends_with("\r\n") {
            2
        } else {
            usize::from(out.ends_with('\n'))
        };
        out.truncate(out.len() - ending);
    }
    out
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
#[path = "sync_diff_tests.rs"]
mod tests;
