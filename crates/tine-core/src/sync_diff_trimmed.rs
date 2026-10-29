//! The windowed diff the git seen baseline uses (ADR 0079): [`diff_blocks_trimmed`]
//! and the per-body id memo it shares with the caller. Split from `sync_diff.rs`
//! along this seam; it reuses the parent's alignment helpers.

use super::*;

/// Persisted ids, read once per distinct block body. A block's `id::` comes
/// from the lsdoc parse of its body alone (`parse_projection(raw, is_org)`,
/// then the header facets), so equal bodies have equal ids; the memo spares
/// the projection of every block that already appeared, for one caller's
/// short-lived use (never kept across operations).
#[derive(Default)]
pub struct PersistedIds {
    /// Markdown bodies, then Org bodies.
    by_body: [HashMap<String, Option<String>>; 2],
}

impl PersistedIds {
    pub fn new() -> Self {
        Self::default()
    }
    pub(super) fn of(&mut self, block: &DocBlock) -> Option<String> {
        let memo = &mut self.by_body[usize::from(block.is_org)];
        if let Some(id) = memo.get(block.raw.as_str()) {
            return id.clone();
        }
        let id = persisted_id(block);
        memo.insert(block.raw.clone(), id.clone());
        id
    }
    /// Exactly [`anchor_fingerprint`], through the memo.
    pub(super) fn fingerprint(&mut self, block: &DocBlock) -> u64 {
        if let Some(id) = self.of(block) {
            return fnv(0xcbf2_9ce4_8422_2325 ^ 1, id.as_bytes());
        }
        let mut hash = fnv(0xcbf2_9ce4_8422_2325 ^ 2, block.raw.as_bytes());
        for child in &block.children {
            hash = fnv(hash, &self.fingerprint(child).to_le_bytes());
        }
        hash
    }
}

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// [`diff_blocks`] of two lists whose first `head` and last `tail` roots the
/// caller knows are content-equal on both sides, computed only where it can
/// differ from pairing those ends in place. `rows` cover `mine[head..len -
/// tail]` against `theirs[head..len - tail]` (with the returned, possibly
/// smaller, `head` and `tail`); every root outside is paired in place.
pub struct TrimmedDiff {
    pub head: usize,
    pub tail: usize,
    pub rows: Vec<DiffRow>,
}

/// The whole-list [`diff_blocks`]`(mine, theirs)` pairing, computed on a window
/// when that is provably the same: every block of either side gets the same
/// partner, or none, as in the whole-list diff. Row kinds may differ only
/// where the whole-list diff's gap pass pairs two content-equal blocks (a
/// `Modified` row whose children pair in place); the window reports nothing
/// there. `None` means the whole list must be diffed. The git seen baseline
/// diffs one page snapshot against the next this way (ADR 0079).
///
/// A *cut* is a root whose anchor fingerprint occurs exactly once on each
/// side. Repeated bodies (empty bullets, `---`, a line copied) are allowed.
/// - Exact LCS (the whole list within [`MAX_LCS_COMPARISONS`]): every longest
///   common subsequence pairs the head up to its last cut and the tail from
///   its first cut in place, so only the region between them matters. When
///   the region's longest pairing is unique, it pairs its own head and tail
///   blocks in place and is the window's unique longest pairing too, which
///   the window's own LCS finds.
/// - Patience (larger lists): its anchors are exactly the cuts. When they
///   keep their order, the region between the last head cut and the first
///   tail cut is aligned with the same anchors and the same gap pass, and
///   the gaps holding only head (or only tail) blocks are equal lists, which
///   the gap pass pairs in place within its budget.
///
/// Anything else (a reorder of unique blocks, an ambiguous pairing near the
/// edit, a budget edge) gives `None`. The proof is in the seen backend
/// round-3 receipt; `trimmed_diff_pairs_exactly_like_the_whole_list` and
/// `trimmed_diff_accepts_repeated_bodies_on_real_shaped_pages` check it.
pub fn diff_blocks_trimmed(
    mine: &[DocBlock],
    theirs: &[DocBlock],
    head: usize,
    tail: usize,
    ids: &mut PersistedIds,
) -> Option<TrimmedDiff> {
    let (n, m) = (mine.len(), theirs.len());
    if head + tail > n.min(m) {
        return None;
    }
    if head == 0 && tail == 0 {
        return Some(TrimmedDiff {
            head,
            tail,
            rows: diff_blocks(mine, theirs),
        });
    }
    let fm: Vec<u64> = mine.iter().map(|b| ids.fingerprint(b)).collect();
    let ft: Vec<u64> = theirs.iter().map(|b| ids.fingerprint(b)).collect();
    let counts = |f: &[u64]| {
        let mut c = HashMap::<u64, usize>::with_capacity(f.len());
        for &x in f {
            *c.entry(x).or_default() += 1;
        }
        c
    };
    let (cm, ct) = (counts(&fm), counts(&ft));
    let cut = |f: u64| cm.get(&f) == Some(&1) && ct.get(&f) == Some(&1);
    // The region: after the head's last cut, before the tail's first cut.
    let start = (0..head).rev().find(|&k| cut(fm[k])).map_or(0, |k| k + 1);
    let end = (0..tail).find(|&b| cut(fm[n - tail + b])).unwrap_or(tail);
    let (rm, rt) = (start..n - tail + end, start..m - tail + end);
    if n.saturating_mul(m) <= MAX_LCS_COMPARISONS {
        let im: Vec<Option<String>> = mine[rm.clone()].iter().map(|b| ids.of(b)).collect();
        let it: Vec<Option<String>> = theirs[rt.clone()].iter().map(|b| ids.of(b)).collect();
        // Exactly anchor_eq (equal anchors have equal fingerprints).
        let eq = |i: usize, j: usize| {
            fm[start + i] == ft[start + j]
                && match (&im[i], &it[j]) {
                    (Some(a), Some(b)) => a == b,
                    _ => mine[start + i] == theirs[start + j],
                }
        };
        // The region's longest pairing has the length of the window's plus
        // its own head and tail (the common prefix and suffix lemma), so a
        // unique one pairs those in place and is the window's unique one.
        if !lcs_unique(rm.len(), rt.len(), &eq) {
            return None;
        }
        // The window's own diff_blocks, whose L1 is Hirschberg (the window is
        // within the budget too), with each anchor's id read through the memo.
        let (wm, wt) = (&mine[head..n - tail], &theirs[head..m - tail]);
        let (am, at) = (Anchor::memo(wm, ids), Anchor::memo(wt, ids));
        let mut matched = Vec::new();
        hirschberg_pairs(&am, &at, 0, 0, &mut matched);
        return Some(TrimmedDiff {
            head,
            tail,
            rows: nodes_to_rows(&align_matched(wm, wt, &matched, "")),
        });
    }
    // Patience: the gap pass pairs two equal lists in place (each block's
    // own first line is its first best match) when the gap fits the budget
    // and each pair's children take the exact LCS, which pairs equal lists
    // in place.
    let in_place = |blocks: &[DocBlock]| {
        blocks.len().saturating_mul(blocks.len()) <= MAX_GAP_SIMILARITY_COMPARISONS
            && blocks
                .iter()
                .all(|b| b.children.len().saturating_mul(b.children.len()) <= MAX_LCS_COMPARISONS)
    };
    let mut from = 0;
    for k in 0..start {
        if cut(fm[k]) {
            if !in_place(&mine[from..k]) {
                return None;
            }
            from = k + 1;
        }
    }
    if end < tail {
        let mut from = n - tail + end + 1;
        for k in from..n {
            if cut(fm[k]) {
                if !in_place(&mine[from..k]) {
                    return None;
                }
                from = k + 1;
            }
        }
        if !in_place(&mine[from..n]) {
            return None;
        }
    }
    let theirs_at: HashMap<u64, usize> = rt
        .clone()
        .filter(|&j| cut(ft[j]))
        .map(|j| (ft[j], j))
        .collect();
    let mut matched = Vec::new();
    for i in rm.clone() {
        if !cut(fm[i]) {
            continue;
        }
        // A cut's partner lies in the region: anything before or after it is
        // repeated on this side by the content-equal ends.
        let j = *theirs_at.get(&fm[i])?;
        if !anchor_eq(&mine[i], &theirs[j]) {
            continue; // a fingerprint collision: not a patience anchor
        }
        if matched.last().is_some_and(|&(_, l)| j <= l + start) {
            return None; // patience would pick a longest ordered subset
        }
        matched.push((i - start, j - start));
    }
    let rows = nodes_to_rows(&align_matched(&mine[rm], &theirs[rt], &matched, ""));
    Some(TrimmedDiff {
        head: start,
        tail: tail - end,
        rows,
    })
}

/// Whether exactly one set of pairs reaches the longest common subsequence of
/// `0..a` and `0..b` under `eq`. Counts are exact up to `u64::MAX`, which
/// stands for "very many" (two rows of memory).
fn lcs_unique(a: usize, b: usize, eq: &dyn Fn(usize, usize) -> bool) -> bool {
    const MANY: u64 = u64::MAX;
    let add = |x: u64, y: u64| x.checked_add(y).unwrap_or(MANY);
    let mut prev: Vec<(u32, u64)> = vec![(0, 1); b + 1];
    let mut cur = prev.clone();
    for i in 0..a {
        cur[0] = (0, 1);
        for j in 0..b {
            let (up, left, diag) = (prev[j + 1], cur[j], prev[j]);
            let matched = eq(i, j);
            let len = up.0.max(left.0).max(if matched { diag.0 + 1 } else { 0 });
            // Pairings of this length that skip (i, j) avoid row i or column
            // j (never both): inclusion-exclusion over the two, plus those
            // ending in (i, j).
            let mut count = 0u64;
            if up.0 == len {
                count = add(count, up.1);
            }
            if left.0 == len {
                count = add(count, left.1);
            }
            if diag.0 == len && count != MANY {
                // Those pairings are counted in both terms above.
                count = count.checked_sub(diag.1).unwrap_or(MANY);
            }
            if matched && diag.0 + 1 == len {
                count = add(count, diag.1);
            }
            cur[j + 1] = (len, count);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b].1 == 1
}
