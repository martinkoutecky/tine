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
    let theirs = parse("- totally rewritten line\n  id:: aaaaaaaa-0000-0000-0000-0000000000ab\n");
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
    let suggestions: Vec<Option<String>> = table(&d.rows).into_iter().map(|(_, _, s)| s).collect();
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
        let error =
            merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &dec).unwrap_err();
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
    let merged = merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &dec).unwrap();
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
    let page =
        |body: &str, kid: &str| parse(&format!("- {body}\n{ID}\t- the child line {kid}\n{KID}"));
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
