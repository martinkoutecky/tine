//! Character-level three-way merge over one block body, for the Direct Files
//! conflict resolver. [`merge_disjoint`] produces a merged body that is only
//! ever PRE-SELECTED for the user to confirm, never auto-applied, which is why
//! it may use the relaxed collision rule of [`hunks_collide_relaxed`].

/// Inputs larger than this (per text, in bytes) get no merge proposal
/// without diffing; the bounded search below stays cheap.
const MAX_MERGE_BYTES: usize = 256 * 1024;

/// Upper bound on the Myers edit distance explored per side before giving up
/// conservatively. The backtrack trace retains one band per distance step —
/// (d+1)^2 cells total — so this bound is also the memory bound: 1024 keeps
/// the worst-case trace near 8 MB, where 8192 with full-width rows reached
/// ~1 GiB for a pair of 4 KiB full replacements (audit 4). Any real pair of
/// disjoint-region edits of one outline block sits far below this distance;
/// larger rewrites get no merge proposal.
const MAX_EDIT_DISTANCE: usize = 1024;

/// One replaced region of the ancestor: bytes `base_start..base_end` become
/// `replacement`. Pure insertions have `base_start == base_end`.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Hunk {
    base_start: usize,
    base_end: usize,
    replacement: String,
}

/// Three-way merge of two edits of `base` whose hunks are disjoint under the
/// RELAXED rule (see [`hunks_collide_relaxed`]), for the Direct Files conflict
/// resolver: the composition of both edits, or `None` when no merge may be
/// offered at all — a nonempty-range overlap, two pure insertions at one
/// position, oversized inputs, a bounded-diff give-up, or a side equal to
/// `base` (a one-sided change needs no merge; the caller suggests that side).
///
/// The result is a PROPOSAL. It is recomputed from the same three texts at
/// apply time and gated there again; nothing here is authority to write.
pub fn merge_disjoint(base: &str, mine: &str, theirs: &str) -> Option<String> {
    if base == mine || base == theirs {
        return None;
    }
    if base.len() > MAX_MERGE_BYTES
        || mine.len() > MAX_MERGE_BYTES
        || theirs.len() > MAX_MERGE_BYTES
    {
        return None;
    }
    let base_chars: Vec<char> = base.chars().collect();
    let my_hunks = diff_hunks(&base_chars, &mine.chars().collect::<Vec<_>>())?;
    let their_hunks = diff_hunks(&base_chars, &theirs.chars().collect::<Vec<_>>())?;
    if hunks_collide_relaxed(&my_hunks, &their_hunks) {
        return None;
    }
    Some(compose(&base_chars, &my_hunks, &their_hunks))
}

/// Minimal character diff of `base -> target` as replaced base regions, via
/// bounded Myers. Returns `None` if the bounded search gives up.
fn diff_hunks(base: &[char], target: &[char]) -> Option<Vec<Hunk>> {
    let common = lcs_matches(base, target)?;
    // Walk the match list; every gap between consecutive matches is a hunk.
    let mut hunks = Vec::new();
    let mut base_pos = 0usize;
    let mut target_pos = 0usize;
    for (base_match, target_match) in common
        .iter()
        .copied()
        .chain(std::iter::once((base.len(), target.len())))
    {
        if base_match != base_pos || target_match != target_pos {
            hunks.push(Hunk {
                base_start: base_pos,
                base_end: base_match,
                replacement: target[target_pos..target_match].iter().collect(),
            });
        }
        base_pos = base_match + 1;
        target_pos = target_match + 1;
    }
    Some(hunks)
}

/// Longest common subsequence as `(base_index, target_index)` match pairs,
/// via Myers O(ND) with a bounded distance. `None` when the bound is hit.
fn lcs_matches(base: &[char], target: &[char]) -> Option<Vec<(usize, usize)>> {
    if base.is_empty() && target.is_empty() {
        // With both sides empty the bound is 0, so `v` holds a single cell and
        // step 0 indexes one past it. Every other shape gives bound >= 1.
        return Some(Vec::new());
    }
    let n = base.len();
    let m = target.len();
    let max = n + m;
    let bound = max.min(MAX_EDIT_DISTANCE);
    let offset = bound;
    let mut v = vec![0usize; 2 * bound + 1];
    // Per-step band of `v`: step d's backtrack touches only k ∈ [-d, d], so
    // retain exactly `v[offset-d ..= offset+d]` (2d+1 cells) instead of the
    // full 2*bound+1 row — the full-width clone made the trace Θ(bound²·width)
    // ≈ 1 GiB for two 4 KiB replacements (audit 4).
    let mut trace: Vec<Vec<usize>> = Vec::new();
    let mut found = None;
    'outer: for d in 0..=bound {
        trace.push(v[offset - d..=offset + d].to_vec());
        let mut k = -(d as isize);
        while k <= d as isize {
            let index = (k + offset as isize) as usize;
            let mut x = if k == -(d as isize) || (k != d as isize && v[index - 1] < v[index + 1]) {
                v[index + 1]
            } else {
                v[index - 1] + 1
            };
            let mut y = (x as isize - k) as usize;
            while x < n && y < m && base[x] == target[y] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'outer;
            }
            k += 2;
        }
    }
    let d_final = found?;
    // Backtrack through the trace to recover the matched positions.
    let mut matches = Vec::new();
    let mut x = n;
    let mut y = m;
    for d in (0..=d_final).rev() {
        // Rows are the per-step band `v[offset-d ..= offset+d]`, so diagonal
        // k lives at row index k + d.
        let v = &trace[d];
        let k = x as isize - y as isize;
        let index = (k + d as isize) as usize;
        let (prev_k, from_down) = if d == 0 {
            (0, false)
        } else if k == -(d as isize) || (k != d as isize && v[index - 1] < v[index + 1]) {
            (k + 1, true)
        } else {
            (k - 1, false)
        };
        let prev_index = (prev_k + d as isize) as usize;
        let prev_x = if d == 0 { 0 } else { v[prev_index] };
        let prev_y = (prev_x as isize - prev_k) as usize;
        // The snake: diagonal matches walked after the edit step.
        let snake_start_x = if d == 0 {
            0
        } else if from_down {
            prev_x
        } else {
            prev_x + 1
        };
        let snake_start_y = if d == 0 {
            0
        } else if from_down {
            prev_y + 1
        } else {
            prev_y
        };
        while x > snake_start_x && y > snake_start_y {
            x -= 1;
            y -= 1;
            matches.push((x, y));
        }
        if d > 0 {
            x = prev_x;
            y = prev_y;
        }
    }
    matches.reverse();
    Some(matches)
}

/// True when any hunk of one side overlaps a hunk of the other in a NONEMPTY
/// base interval, or both are pure insertions at the same position (their
/// relative order is unknowable).
///
/// Touching delete/insert pairs compose deterministically (see [`compose`]'s
/// `(start, end)` order) and so merge here: the result is a suggestion the
/// user confirms, never a silent apply.
fn hunks_collide_relaxed(mine: &[Hunk], theirs: &[Hunk]) -> bool {
    // A pure insertion strictly inside the other side's replaced range: the
    // insertion's anchor no longer exists in the merged text, and `compose`
    // (which walks base ranges in order) would see interleaved ranges.
    fn inserts_strictly_inside(ins: &Hunk, other: &Hunk) -> bool {
        ins.base_start == ins.base_end
            && other.base_start < ins.base_start
            && ins.base_start < other.base_end
    }
    for a in mine {
        for b in theirs {
            let overlaps = a.base_start.max(b.base_start) < a.base_end.min(b.base_end);
            let same_point_insertions = a.base_start == a.base_end
                && b.base_start == b.base_end
                && a.base_start == b.base_start;
            if overlaps
                || same_point_insertions
                || inserts_strictly_inside(a, b)
                || inserts_strictly_inside(b, a)
            {
                return true;
            }
        }
    }
    false
}

/// Apply both sides' disjoint hunks to the ancestor.
fn compose(base: &[char], mine: &[Hunk], theirs: &[Hunk]) -> String {
    let mut all: Vec<&Hunk> = mine.iter().chain(theirs.iter()).collect();
    all.sort_by_key(|hunk| (hunk.base_start, hunk.base_end));
    let mut result = String::new();
    let mut pos = 0usize;
    for hunk in all {
        result.extend(base[pos..hunk.base_start].iter());
        result.push_str(&hunk.replacement);
        pos = hunk.base_end;
    }
    result.extend(base[pos..].iter());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flagship case the relaxed rule exists for: a trailing deletion and
    /// an append that merely TOUCH. Both argument orders compose identically,
    /// because `compose` orders hunks by `(base_start, base_end)`.
    #[test]
    fn touching_delete_and_append_merge_in_either_order() {
        assert_eq!(
            merge_disjoint("Desktop 5", "Desktop", "Desktop 5 kk").as_deref(),
            Some("Desktop kk")
        );
        assert_eq!(
            merge_disjoint("Desktop 5", "Desktop 5 kk", "Desktop").as_deref(),
            Some("Desktop kk")
        );
    }

    #[test]
    fn insertion_touching_a_following_deletion_merges() {
        // base "ab": theirs inserts "X" at [0,0), mine deletes "a" at [0,1).
        assert_eq!(merge_disjoint("ab", "b", "Xab").as_deref(), Some("Xb"));
        assert_eq!(merge_disjoint("ab", "Xab", "b").as_deref(), Some("Xb"));
    }

    #[test]
    fn disjoint_region_edits_merge() {
        assert_eq!(
            merge_disjoint("alpha beta gamma", "ALPHA beta gamma", "alpha beta GAMMA").as_deref(),
            Some("ALPHA beta GAMMA")
        );
    }

    #[test]
    fn same_point_double_insertion_is_no_merge() {
        assert_eq!(
            merge_disjoint("one two", "one extra two", "one other two"),
            None
        );
    }

    #[test]
    fn nonempty_range_overlap_is_no_merge() {
        assert_eq!(
            merge_disjoint("alpha beta gamma", "ALPHA BETA gamma", "alpha BETA GAMMA"),
            None
        );
    }

    /// A side equal to the base is a one-sided change: the caller suggests that
    /// side outright, so no merge is offered (and never one for both sides).
    #[test]
    fn a_side_equal_to_base_is_no_merge() {
        assert_eq!(merge_disjoint("base", "base", "edited"), None);
        assert_eq!(merge_disjoint("base", "edited", "base"), None);
    }

    /// Identical edits produce identical hunks, which collide with themselves.
    #[test]
    fn identical_edits_are_no_merge() {
        assert_eq!(merge_disjoint("base", "same edit", "same edit"), None);
    }

    #[test]
    fn oversized_inputs_are_no_merge() {
        let big = "x".repeat(MAX_MERGE_BYTES + 1);
        assert_eq!(merge_disjoint(&big, "a", "b"), None);
    }

    /// Two full replacements exceed `MAX_EDIT_DISTANCE`; the bounded search
    /// gives up and no merge is offered.
    #[test]
    fn bounded_diff_give_up_is_no_merge() {
        let base = "a".repeat(MAX_EDIT_DISTANCE);
        let mine = "b".repeat(MAX_EDIT_DISTANCE);
        let theirs = "c".repeat(MAX_EDIT_DISTANCE);
        assert_eq!(merge_disjoint(&base, &mine, &theirs), None);
    }

    /// Repeated text: the diff anchors on the FIRST match, so an edit the human
    /// made to the second "ab" is attributed to the first. The composition is
    /// still deterministic and disjoint — documenting the accepted behavior,
    /// which the user confirms before anything is written.
    #[test]
    fn repeated_text_merges_at_the_diffs_anchor() {
        // Both sides append distinct suffixes to different "ab" occurrences.
        assert_eq!(
            merge_disjoint("ab ab", "Xab ab", "ab abY").as_deref(),
            Some("Xab abY")
        );
        // Deleting one of two identical words anchors on the first match.
        assert_eq!(
            merge_disjoint("ab ab", "ab", "ab ab!").as_deref(),
            Some("ab!")
        );
    }

    /// Hunk boundaries are char-indexed, so multi-byte characters on either
    /// side of a boundary survive the composition intact.
    #[test]
    fn multibyte_chars_across_a_hunk_boundary_merge() {
        assert_eq!(
            merge_disjoint("káva a čaj", "KÁVA a čaj", "káva a ČAJ").as_deref(),
            Some("KÁVA a ČAJ")
        );
        // Touching pair around a multi-byte char: delete "čaj", append after it.
        assert_eq!(
            merge_disjoint("káva a čaj", "káva a ", "káva a čaj ♥").as_deref(),
            Some("káva a  ♥")
        );
    }

    #[test]
    fn insertion_strictly_inside_the_others_deletion_is_no_merge() {
        // mine deletes "bcde" (hunk [1,5)); theirs inserts X between c and d
        // (pure insertion at 3, strictly inside). Must be None, not a panic
        // or an interleave.
        assert_eq!(merge_disjoint("abcdef", "af", "abcXdef"), None);
    }
    #[test]
    fn whole_replacement_by_one_side_still_collides() {
        assert_eq!(merge_disjoint("hello", "goodbye", "hello!"), None);
    }

    #[test]
    fn lcs_of_two_empty_inputs_is_an_empty_match_list() {
        // Latent: today's public entry points both return early on
        // `base == mine`/`base == theirs`, so neither can reach this. The
        // guard belongs to the module rather than to its callers, because a
        // future caller of `diff_hunks` would inherit the panic silently.
        assert_eq!(lcs_matches(&[], &[]), Some(Vec::new()));
    }
}
