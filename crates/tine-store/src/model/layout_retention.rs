//! Per-block physical retention: a Markdown save rewrites only what it changed,
//! and an Org save keeps each reused line's own terminator ([`restore_org`]).
//!
//! The DTO carries each block's semantic text, not the file's layout: base
//! indentation offsets, depth jumps, continuation indentation, whitespace-only
//! lines. Rebuilding the whole page from the DTO would rewrite that layout in
//! blocks the save never touched. Instead, every DTO block that is an old block
//! (by runtime id, when the caller knows the old blocks' ids) or, failing
//! identity, has an old block's raw text (by LCS, then by equal text for moved
//! blocks; parent scopes when lineage is ambiguous) reuses that block's physical
//! lines; only new or changed
//! blocks, and the page-property preamble when it changed, are rendered; a
//! changed block still keeps the bytes of the continuation lines its edit did
//! not touch. A block known to stay with its original parent retains its
//! offset from that parent, so it carries the parent's prefix change; a known
//! block joining another parent takes its new siblings' prefix (or one indent
//! step below its parent). A block matched by text alone keeps its old prefix
//! wherever the parse rule allows it. When runtime lineage is ambiguous, text
//! matching pairs parent subtrees first so repeated children cannot borrow
//! physical lines from a different subtree.
//!
//! Cost: borrows the caller's single old-source parse; one output parse (two,
//! plus a full serialization, when the
//! DTO itself does not round-trip) and an LCS over the changed middle of the
//! pre-order block sequence (common prefix and suffix are trimmed first),
//! capped at 4,000,000 table cells (about 16 MB); a larger middle skips the
//! LCS and matches blocks by equal text only. Ambiguous lineage adds linear
//! subtree classification inside the matcher, without another parse.

use std::collections::{HashMap, VecDeque};

use tine_core::doc::{self, DocBlock, Document, SerializeOpts};
use tine_core::BlockId;

use super::line_endings;

/// Serialize a Markdown `doc` reusing `source`'s physical lines for every block
/// whose raw text is unchanged (re-indented as a unit only when its new position
/// requires it). Every reused line keeps its own terminator (`\n`, `\r\n` or a
/// lone `\r`); a rendered line takes the file's convention
/// (`line_endings::convention`). Returns `None` when `doc` is empty (no
/// pre-block, no roots), `source`'s layout cannot be mapped to blocks, or the
/// result re-parses neither to `doc` nor as below; the caller then serializes
/// the whole page. Declines are silent (no log or counter).
///
/// `Some` output re-parses to exactly `doc` (pre-block plus each block's raw
/// text and children), with one exception: when `doc` itself cannot round-trip
/// (e.g. blocks after an unterminated code fence re-parse as fence text), `Some`
/// means the output re-parses equal to what the whole-page serialization
/// re-parses to, so it has the meaning the fallback would have written, which
/// is also not `doc`. Returns the parsed output with the bytes for equivalence
/// checks. Pure. Cost: borrows the old document; one output parse (two plus a full
/// serialization when the first check fails) and an LCS over the changed middle
/// of the pre-order block sequence, O(a·b) with a `u32` table, skipped above
/// 4,000,000 cells (about 16 MB).
pub(super) fn serialize(
    doc: &Document,
    source: &str,
    old: &Document,
    opts: &SerializeOpts,
) -> Option<(String, Document)> {
    if doc.roots.is_empty() && doc.pre_block.is_none() {
        return None;
    }
    let (lines, ends) = line_endings::split(source);
    let olds = map_old_blocks(old, &lines)?;
    let mut news = Vec::new();
    flatten(&doc.roots, &mut news);
    let Matches { keep, hint, known } = match_blocks(&olds, &news);

    let region = olds.first().map_or(lines.len(), |o| o.start);
    let mut out: Vec<(String, Option<usize>)> = Vec::new();
    emit_preamble(old, doc, &lines, region, opts, &mut out);
    let mut emitter = Emitter {
        lines: &lines,
        olds: &olds,
        keep,
        hint,
        known,
        unit: &opts.indent,
        out,
    };
    let mut index = 0;
    emitter.place(&doc.roots, None, &mut index);
    let out = emitter.out;
    let convention = line_endings::convention(Some(source));
    let end = |(_, origin): &(String, Option<usize>)| {
        origin
            .map(|i| ends[i])
            .filter(|end| !end.is_empty())
            .unwrap_or(convention)
    };
    let mut result = String::with_capacity(source.len() + 64);
    for (k, line) in out.iter().enumerate() {
        result.push_str(&line.0);
        if k + 1 < out.len() {
            result.push_str(end(line));
        }
    }
    if let Some(last) = out.last() {
        if source.ends_with(['\n', '\r']) || last.0.is_empty() {
            result.push_str(end(last));
        }
    }
    // Safety net: the reused lines must re-parse to exactly the DTO, or a
    // neighbour's layout changed its meaning.
    let reparsed = super::parse_doc(std::path::Path::new("new.md"), &result);
    if reparsed.pre_block == doc.pre_block && reparsed.roots == doc.roots {
        return Some((result, reparsed));
    }
    // A DTO that cannot round-trip at all (e.g. blocks after an unterminated
    // fence) keeps this layout only when it means what a full rebuild means.
    (reparsed
        == super::parse_doc(
            std::path::Path::new("new.md"),
            &doc::serialize_with(doc, opts),
        ))
    .then_some((result, reparsed))
}

/// Give an Org save's LF-separated `serialized` text (`org::serialize_org_detect`
/// of `doc`) the line terminators of `source` (K01a). `old` is `source` parsed:
/// the Org firewall has proved `serialized`'s rule reproduces it, so `source`'s
/// lines are `old`'s pre-block lines, then each block's lines in pre-order, then
/// the trailing newline run.
///
/// Rule: a source whose terminators are all one kind gives every line that
/// kind. Otherwise each new line takes the terminator of the old line it comes
/// from, and a line from no old line takes [`line_endings::convention`]. Blocks
/// pair as Markdown's do ([`match_blocks`]: unchanged ones wherever they moved,
/// changed ones with the old block in their place), the pre-block with the old
/// pre-block; within a pair, lines pair by [`pair_lines`]. The trailing newline
/// run aligns from the end. Pure. Cost: O(lines) for a uniform or LF-only
/// source; otherwise O(lines) plus capped LCS over the changed middles.
pub(super) fn restore_org(
    serialized: String,
    doc: &Document,
    source: &str,
    old: &Document,
) -> String {
    if !source.contains('\r') {
        return serialized;
    }
    let (lines, ends) = line_endings::pieces(source);
    if let Some(&end) = ends.first().filter(|&&e| ends.iter().all(|&x| x == e)) {
        return serialized.replace('\n', end);
    }
    let pre_len = |d: &Document| {
        d.pre_block
            .as_ref()
            .map_or(0, |p| emitted_lines(p).split('\n').count())
    };
    let mut flat = Vec::new();
    flatten(&old.roots, &mut flat);
    let mut start = pre_len(old);
    let olds: Vec<OldBlock> = flat
        .iter()
        .map(|block| {
            let len = block.raw().split('\n').count();
            start += len;
            OldBlock {
                block,
                start: start - len,
                len,
                bulleted: true,
                parent: None,
                id: &block.uuid,
            }
        })
        .collect();
    let old_content = start;
    let mut news = Vec::new();
    flatten(&doc.roots, &mut news);
    let Matches { keep, hint, .. } = match_blocks(&olds, &news);

    let new: Vec<&str> = serialized.split('\n').collect();
    let mut origin: Vec<Option<usize>> = vec![None; new.len()];
    let new_pre = pre_len(doc);
    let mut place = |at: usize, start: usize, a: &str, b: &[&str]| {
        let a: Vec<&str> = a.split('\n').collect();
        for (k, i) in pair_lines(&a, b).into_iter().enumerate() {
            if let Some(slot) = origin.get_mut(at + k) {
                *slot = i.map(|i| start + i);
            }
        }
    };
    if let (Some(a), Some(b)) = (&old.pre_block, &doc.pre_block) {
        let b = emitted_lines(b);
        place(0, 0, &emitted_lines(a), &b.split('\n').collect::<Vec<_>>());
    }
    let mut at = new_pre;
    for (j, block) in news.iter().enumerate() {
        let text = emitted_lines(block.raw());
        let b: Vec<&str> = text.split('\n').collect();
        if let Some(o) = keep[j].or(hint[j]).map(|o| &olds[o]) {
            place(at, o.start, o.block.raw(), &b);
        }
        at += b.len();
    }
    // The trailing newline run aligns with the old run from the end. It starts
    // where `serialized`'s content ends, not at `at`: the serializer folds a
    // last block's own trailing newlines (a draft's `b\n`) into that run.
    let content = serialized.trim_end_matches('\n');
    let tail = if content.is_empty() {
        0
    } else {
        content.split('\n').count()
    };
    for (t, slot) in origin.iter_mut().enumerate().skip(tail) {
        *slot = (lines.len() + t)
            .checked_sub(new.len())
            .filter(|&i| i >= old_content);
    }
    let convention = line_endings::convention(Some(source));
    let mut out = String::with_capacity(serialized.len() + new.len());
    for (k, line) in new.iter().enumerate() {
        out.push_str(line);
        if k + 1 < new.len() {
            out.push_str(
                origin[k]
                    .and_then(|i| ends.get(i).copied())
                    .unwrap_or(convention),
            );
        }
    }
    out
}

/// `text` as the Org serializer emits it before a following line: every
/// terminator one `\n`, a trailing lone `\r` merged with the joining `\n`.
/// Borrows when `text` has no `\r`. O(text).
fn emitted_lines(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('\r') {
        return text.into();
    }
    let mut lf = doc::normalize_line_endings(&format!("{text}\n")).into_owned();
    lf.pop();
    lf.into()
}

/// mldoc's layout whitespace (tine-core `strip_layout_ws`): what lsdoc skips
/// before a dash bullet and in a block's indentation.
const LAYOUT_WS: [char; 3] = [' ', '\t', '\x0c'];

fn is_unbulleted_heading_line(line: &str) -> bool {
    #[cfg(feature = "test-faults")]
    crate::cost_counters::parse();
    tine_core::doc::is_unbulleted_heading_line(line)
}

/// An old block's pre-order position and its physical lines.
struct OldBlock<'a> {
    block: &'a DocBlock,
    start: usize,
    len: usize,
    /// Written as `- …`; false for an unbulleted ATX heading (lsdoc), whose
    /// lines are its raw text verbatim.
    bulleted: bool,
    /// Markdown parent's pre-order index, used to carry a rebased prefix only
    /// into that parent's original children. Org only pairs line terminators.
    parent: Option<usize>,
    /// Its runtime id, empty when the caller does not know it.
    id: &'a BlockId,
}

/// Locate every old block's physical lines. Blocks own the tail of the body,
/// in pre-order, one line per raw line; the preamble owns the rest. A block's
/// first line is `- raw` (any dash form lsdoc accepts) or, for an unbulleted
/// heading, the raw line itself. `None` when that layout does not hold.
fn map_old_blocks<'a>(old: &'a Document, lines: &[&str]) -> Option<Vec<OldBlock<'a>>> {
    let mut flat = Vec::new();
    flatten(&old.roots, &mut flat);
    let total: usize = flat.iter().map(|b| b.raw().split('\n').count()).sum();
    let mut start = lines.len().checked_sub(total)?;
    let pre_len = old
        .pre_block
        .as_ref()
        .map_or(0, |pre| pre.split('\n').count());
    if pre_len > start
        || old.pre_block.as_deref()
            != (pre_len > 0)
                .then(|| lines[..pre_len].join("\n"))
                .as_deref()
        || lines[pre_len..start]
            .iter()
            .any(|line| !line.trim().is_empty())
    {
        return None;
    }
    let mut olds = Vec::with_capacity(flat.len());
    let mut parents: Vec<(usize, usize)> = Vec::new();
    for block in flat {
        while parents.last().is_some_and(|(_, left)| *left == 0) {
            parents.pop();
        }
        let parent = parents.last_mut().map(|(index, left)| {
            *left -= 1;
            *index
        });
        let raw = block.raw();
        let len = raw.split('\n').count();
        let head = lines[start].trim_start_matches(LAYOUT_WS);
        let first = raw.split('\n').next().unwrap_or("");
        let dash = head
            .strip_prefix('-')
            .map(|rest| rest.strip_prefix(' ').unwrap_or(rest));
        let bulleted = lines[start] != first;
        if bulleted && dash != Some(first) {
            return None;
        }
        parents.push((olds.len(), block.children.len()));
        olds.push(OldBlock {
            block,
            start,
            len,
            bulleted,
            parent,
            id: &block.uuid,
        });
        start += len;
    }
    Some(olds)
}

fn flatten<'a>(blocks: &'a [DocBlock], out: &mut Vec<&'a DocBlock>) {
    for block in blocks {
        out.push(block);
        flatten(&block.children, out);
    }
}

fn subtree_len(block: &DocBlock) -> usize {
    1 + block.children.iter().map(subtree_len).sum::<usize>()
}

/// Exact subtree classes shared by old and new trees, and sibling groups
/// (group zero is the roots, group i+1 is block i's children). Interning
/// (raw, child classes) compares structure without projections, hash collisions or
/// repeatedly walking descendants. O(bytes + blocks) time and O(blocks) space.
fn text_tree<'a>(
    blocks: &[&'a DocBlock],
    classes: &mut HashMap<(&'a str, Vec<usize>), usize>,
) -> (Vec<usize>, Vec<Vec<usize>>) {
    let mut tags = vec![0; blocks.len()];
    let mut ends = vec![0; blocks.len()];
    for i in (0..blocks.len()).rev() {
        let mut next = i + 1;
        let children = blocks[i]
            .children
            .iter()
            .map(|_| {
                let tag = tags[next];
                next = ends[next];
                tag
            })
            .collect();
        ends[i] = next;
        let fresh = classes.len();
        tags[i] = *classes.entry((blocks[i].raw(), children)).or_insert(fresh);
    }
    let mut parents: Vec<usize> = Vec::new();
    let mut groups = vec![Vec::new(); blocks.len() + 1];
    for i in 0..blocks.len() {
        while parents.last().is_some_and(|&p| ends[p] <= i) {
            parents.pop();
        }
        groups[parents.last().map_or(0, |p| p + 1)].push(i);
        parents.push(i);
    }
    (tags, groups)
}

/// For each new block: the old block whose lines it reuses (equal raw); for
/// unmatched ones an old block whose indentation and terminators it should
/// prefer (a changed block); and whether that old block is its identity.
struct Matches {
    keep: Vec<Option<usize>>,
    hint: Vec<Option<usize>>,
    known: Vec<bool>,
}

/// Which old block each new block is. Identity first: a new block whose
/// runtime id names exactly one old block is that block (the save path gives
/// the old blocks the ids the saved document uses for them, where it can tell:
/// `model::page_parse::identify_old_blocks`). Text only for the rest, among
/// the old blocks no identity kept: equal raw by an LCS over the changed middle, unpaired blocks by
/// position inside each gap between matches (a changed block), then equal raw
/// anywhere (a moved block). Where old identities are ambiguous, pair whole
/// subtrees before equal parent text, then match children within the paired
/// parents. A text match is never identity, so it keeps its old lines and
/// prefix but does not carry a parent's
/// re-indentation ([`Emitter::preferred_prefix`]). O(old + new bytes and blocks)
/// plus the capped LCS.
fn match_blocks(olds: &[OldBlock], news: &[&DocBlock]) -> Matches {
    #[cfg(test)]
    BLOCK_MATCHES.with(|n| n.set(n.get() + 1));
    let (n, m) = (olds.len(), news.len());
    let (mut keep, mut hint) = (vec![None; m], vec![None; m]);
    let (mut used, mut known) = (vec![false; n], vec![false; m]);
    // `None`: an id two old blocks carry names neither.
    let mut by_id: HashMap<&BlockId, Option<usize>> = HashMap::new();
    for (o, old) in olds
        .iter()
        .enumerate()
        .filter(|(_, old)| !old.id.is_empty())
    {
        by_id
            .entry(old.id)
            .and_modify(|o| *o = None)
            .or_insert(Some(o));
    }
    for (j, new) in news.iter().enumerate() {
        if let Some(Some(o)) = by_id.get_mut(&new.uuid).map(Option::take) {
            known[j] = true;
            // A changed block's old lines stay free for a block that kept
            // their text (text matching below), as they always were.
            if olds[o].block.raw() == new.raw() {
                (keep[j], used[o]) = (Some(o), true);
            } else {
                hint[j] = Some(o);
            }
        }
    }
    let mut matched = Matches { keep, hint, known };
    if matched.known.iter().all(|known| *known) {
        return matched;
    }
    let mut budget = 4_000_000;
    if olds.iter().any(|o| o.id.is_empty()) && matched.known.iter().any(|known| !known) {
        let mut classes = HashMap::new();
        let (old_tags, mut old_groups) = text_tree(
            &olds.iter().map(|o| o.block).collect::<Vec<_>>(),
            &mut classes,
        );
        let (new_tags, mut new_groups) = text_tree(news, &mut classes);
        drop(classes);
        // Unclaimed old roots may move under another parent. Descendants
        // belong to their paired parent's sibling group, never a global pool.
        let mut roots: HashMap<usize, VecDeque<usize>> = HashMap::new();
        for &o in &old_groups[0] {
            roots.entry(old_tags[o]).or_default().push_back(o);
        }
        let mut groups = vec![(
            std::mem::take(&mut old_groups[0]),
            std::mem::take(&mut new_groups[0]),
        )];
        while let Some((old_group, new_group)) = groups.pop() {
            let mut exact: HashMap<usize, VecDeque<usize>> = HashMap::new();
            for &o in &old_group {
                exact.entry(old_tags[o]).or_default().push_back(o);
            }
            for &j in &new_group {
                if !matched.known[j] {
                    let take = |pool: &mut HashMap<usize, VecDeque<usize>>| {
                        let queue = pool.get_mut(&new_tags[j])?;
                        while queue.front().is_some_and(|&o| used[o]) {
                            queue.pop_front();
                        }
                        queue.pop_front()
                    };
                    if let Some(o) = take(&mut exact).or_else(|| take(&mut roots)) {
                        matched.keep[j] = Some(o);
                        used[o] = true;
                    }
                }
            }
            match_text(
                olds,
                news,
                &old_group,
                &new_group,
                &mut matched,
                &mut used,
                &mut budget,
            );
            for j in new_group.into_iter().rev() {
                if !new_groups[j + 1].is_empty() {
                    let old_group = matched.keep[j]
                        .or(matched.hint[j])
                        .map(|o| std::mem::take(&mut old_groups[o + 1]))
                        .unwrap_or_default();
                    groups.push((old_group, std::mem::take(&mut new_groups[j + 1])));
                }
            }
        }
    } else {
        match_text(
            olds,
            news,
            &(0..n).collect::<Vec<_>>(),
            &(0..m).collect::<Vec<_>>(),
            &mut matched,
            &mut used,
            &mut budget,
        );
    }
    matched
}

/// The text fallback over one scope: the whole flat page when identity is
/// unambiguous, otherwise a paired parent's direct children. All scopes share
/// one LCS cell budget, so partitioning never multiplies the cost cap.
fn match_text(
    olds: &[OldBlock],
    news: &[&DocBlock],
    old_group: &[usize],
    new_group: &[usize],
    matched: &mut Matches,
    used: &mut [bool],
    budget: &mut usize,
) {
    let Matches { keep, hint, known } = matched;
    let b: Vec<usize> = new_group
        .iter()
        .copied()
        .filter(|&j| !known[j] && keep[j].is_none())
        .collect();
    if b.is_empty() {
        return;
    }
    let a: Vec<usize> = old_group.iter().copied().filter(|&o| !used[o]).collect();
    let eq = |i: usize, j: usize| olds[a[i]].block.raw() == news[b[j]].raw();
    let (n2, m2) = (a.len(), b.len());
    let mut head = 0;
    while head < n2.min(m2) && eq(head, head) {
        keep[b[head]] = Some(a[head]);
        head += 1;
    }
    let mut tail = 0;
    while tail < (n2 - head).min(m2 - head) && eq(n2 - 1 - tail, m2 - 1 - tail) {
        keep[b[m2 - 1 - tail]] = Some(a[n2 - 1 - tail]);
        tail += 1;
    }
    // Quadratic LCS only over the changed middle; a huge middle keeps the
    // unchanged prefix/suffix and re-renders the rest.
    let (mid_a, mid_b) = (n2 - tail - head, m2 - tail - head);
    if mid_a * mid_b <= *budget {
        *budget -= mid_a * mid_b;
        for (i, j) in lcs(mid_a, mid_b, |i, j| eq(head + i, head + j)) {
            keep[b[head + j]] = Some(a[head + i]);
        }
    }
    new_group
        .iter()
        .filter_map(|&j| keep[j])
        .for_each(|o| used[o] = true);
    // Changed blocks: pair unmatched old and new blocks positionally inside
    // each gap between matched anchors.
    let positions: HashMap<usize, usize> =
        old_group.iter().enumerate().map(|(i, &o)| (o, i)).collect();
    let mut next_old = 0;
    for &j in new_group {
        if let Some(o) = keep[j].or(hint[j]) {
            if let Some(i) = positions.get(&o) {
                next_old = i + 1;
            }
        } else if next_old < old_group.len() && !used[old_group[next_old]] {
            hint[j] = Some(old_group[next_old]);
            next_old += 1;
        }
    }
    // Moved blocks: unmatched new blocks with the text of an unmatched old block.
    let mut free: HashMap<&str, VecDeque<usize>> = HashMap::new();
    old_group
        .iter()
        .copied()
        .filter(|&o| !used[o])
        .for_each(|o| {
            free.entry(olds[o].block.raw()).or_default().push_back(o);
        });
    for &j in new_group {
        if keep[j].is_none() && !known[j] {
            keep[j] = free.get_mut(news[j].raw()).and_then(VecDeque::pop_front);
            if let Some(o) = keep[j] {
                used[o] = true;
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    /// [`match_blocks`] calls on this thread, for the uniform-ending fast path test.
    static BLOCK_MATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The index pairs `(i, j)` of a longest common subsequence of two sequences
/// of lengths `n` and `m`, where `eq(i, j)` compares their items, in order.
/// O(n·m) time and table; callers cap the product.
fn lcs(n: usize, m: usize, eq: impl Fn(usize, usize) -> bool) -> Vec<(usize, usize)> {
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if eq(i, j) {
                dp[(i + 1) * w + j + 1] + 1
            } else {
                dp[(i + 1) * w + j].max(dp[i * w + j + 1])
            };
        }
    }
    let (mut pairs, mut i, mut j) = (Vec::new(), 0, 0);
    while i < n && j < m {
        if eq(i, j) {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// For each line of `b`, the line of `a` it comes from: equal lines pair by
/// LCS (after the common head and tail; a middle over 4,096 line pairs pairs
/// them greedily in order, [`greedy_anchors`]), and inside each gap between equal pairs the unpaired lines pair by
/// position (an edit in place); a new line past its gap's old lines is `None`.
fn pair_lines(a: &[&str], b: &[&str]) -> Vec<Option<usize>> {
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - head;
    let tail = (a.iter().rev().zip(b.iter().rev()))
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut out: Vec<Option<usize>> = (0..head).map(Some).collect();
    let anchors = match am.len() * bm.len() <= 4_096 {
        true => lcs(am.len(), bm.len(), |i, j| am[i] == bm[j]),
        false => greedy_anchors(am, bm),
    };
    let (mut pi, mut pj) = (0, 0);
    for (i, j) in anchors.into_iter().chain([(am.len(), bm.len())]) {
        out.extend((pj..j).map(|k| (pi + k - pj < i).then_some(head + pi + k - pj)));
        out.extend((j < bm.len()).then_some(Some(head + i)));
        (pi, pj) = (i + 1, j + 1);
    }
    out.extend((a.len() - tail..a.len()).map(Some));
    out
}

/// An increasing run of equal-line pairs, O(lines): each new line takes the
/// first unused equal old line after the previous pair. The over-cap stand-in
/// for [`lcs`], so untouched lines of a long block still keep their origin.
fn greedy_anchors(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let mut at: std::collections::HashMap<&str, std::collections::VecDeque<usize>> =
        std::collections::HashMap::new();
    for (i, line) in a.iter().enumerate() {
        at.entry(line).or_default().push_back(i);
    }
    let (mut pairs, mut next) = (Vec::new(), 0);
    for (j, line) in b.iter().enumerate() {
        let Some(queue) = at.get_mut(line) else {
            continue;
        };
        while queue.front().is_some_and(|&i| i < next) {
            queue.pop_front();
        }
        if let Some(i) = queue.pop_front() {
            pairs.push((i, j));
            next = i + 1;
        }
    }
    pairs
}

/// Emit the preamble and, once, the separator before the first root: the old
/// blank lines after the old preamble when there are any, else one blank line
/// when the file's style uses it.
fn emit_preamble(
    old: &Document,
    doc: &Document,
    lines: &[&str],
    region: usize,
    opts: &SerializeOpts,
    out: &mut Vec<(String, Option<usize>)>,
) {
    let old_pre_len = old
        .pre_block
        .as_ref()
        .map_or(0, |pre| pre.split('\n').count());
    let reused = |range: std::ops::Range<usize>| range.map(|i| (lines[i].to_string(), Some(i)));
    let gap = reused(old_pre_len..region);
    if old.pre_block == doc.pre_block {
        out.extend(reused(0..old_pre_len));
    } else if let Some(pre) = &doc.pre_block {
        out.extend(pre.split('\n').map(|line| (line.to_string(), None)));
    }
    if doc.pre_block.is_none() {
        // Without a preamble the old region is blank lines only; keep them
        // unless a removed preamble owned them.
        if old.pre_block.is_none() {
            out.extend(gap);
        }
    } else if old.pre_block.is_some() && region > old_pre_len {
        out.extend(gap);
    } else if !doc.roots.is_empty() && opts.blank_after_props {
        out.push((String::new(), None));
    }
}

struct Emitter<'a> {
    lines: &'a [&'a str],
    olds: &'a [OldBlock<'a>],
    keep: Vec<Option<usize>>,
    hint: Vec<Option<usize>>,
    known: Vec<bool>,
    unit: &'a str,
    /// Emitted lines, each with the old line it reuses (for its terminator).
    out: Vec<(String, Option<usize>)>,
}

impl Emitter<'_> {
    fn old_prefix(&self, old: usize) -> &str {
        let line = self.lines[self.olds[old].start];
        &line[..line.len() - line.trim_start_matches(LAYOUT_WS).len()]
    }

    /// The old block new block `new` is (its identity), if known.
    fn identity(&self, new: usize) -> Option<usize> {
        self.known[new]
            .then(|| self.keep[new].or(self.hint[new]))
            .flatten()
    }

    /// The prefix new block `new` would like from its old block `old`, under
    /// `parent` (its chosen prefix and identity). A block known to be `old`
    /// under the old parent keeps its offset from that parent's prefix; known
    /// at the root, it keeps its own; known elsewhere, it has no preference.
    /// A block matched by text alone keeps `old`'s prefix, whatever moved.
    fn preferred_prefix(
        &self,
        new: usize,
        old: usize,
        parent: Option<(&str, Option<usize>)>,
    ) -> Option<String> {
        let own = self.old_prefix(old);
        if !self.known[new] {
            return Some(own.to_string());
        }
        match (parent, self.olds[old].parent) {
            (None, None) => Some(own.to_string()),
            (Some((prefix, Some(was))), Some(old_parent)) if was == old_parent => {
                let from = self.old_prefix(was);
                Some(if prefix == from {
                    own.to_string()
                } else {
                    format!("{prefix}{}", &own[from.len()..])
                })
            }
            _ => None,
        }
    }

    /// Emit one sibling list. `parent` is the parent's chosen prefix; `index`
    /// is the pre-order index of the first sibling and advances past the list.
    fn place(
        &mut self,
        blocks: &[DocBlock],
        parent: Option<(&str, Option<usize>)>,
        index: &mut usize,
    ) {
        let mut starts = Vec::with_capacity(blocks.len() + 1);
        starts.push(*index);
        for block in blocks {
            starts.push(starts[starts.len() - 1] + subtree_len(block));
        }
        let mut prev: Option<String> = None;
        for (j, block) in blocks.iter().enumerate() {
            let i = starts[j];
            // The outline parser nests by column: a block must be deeper than
            // its parent and no deeper than its previous sibling.
            let valid = |p: &str| {
                parent.is_none_or(|(pp, _)| p.len() > pp.len())
                    && prev.as_ref().is_none_or(|ps| p.len() <= ps.len())
            };
            let own = self.keep[i]
                .or(self.hint[i])
                .and_then(|o| self.preferred_prefix(i, o, parent));
            let next = (j + 1 < blocks.len())
                .then(|| self.keep[starts[j + 1]])
                .flatten()
                .and_then(|o| self.preferred_prefix(starts[j + 1], o, parent));
            let prefix = own
                .filter(|p| valid(p))
                .or_else(|| prev.clone())
                .or_else(|| next.filter(|p| valid(p)))
                .unwrap_or_else(|| {
                    parent.map_or(String::new(), |(pp, _)| format!("{pp}{}", self.unit))
                });
            match self.keep[i] {
                // An unbulleted heading's indentation is part of its raw text,
                // so it can only be reused where it stood.
                Some(o) if self.olds[o].bulleted || self.old_prefix(o) == prefix => {
                    self.reuse(o, &prefix)
                }
                Some(o) => self.render(block.raw(), &prefix, Some(o)),
                None => self.render(block.raw(), &prefix, self.hint[i]),
            }
            *index = i + 1;
            self.place(&block.children, Some((&prefix, self.identity(i))), index);
            prev = Some(prefix);
        }
        *index = starts[blocks.len()];
    }

    /// Copy an old block's lines, moving them from its old prefix to `prefix`.
    /// Continuation lines keep their indentation relative to the header, so the
    /// parser strips them to the same raw text.
    fn reuse(&mut self, old: usize, prefix: &str) {
        let from = self.old_prefix(old).len();
        let OldBlock { start, len, .. } = self.olds[old];
        let same = self.old_prefix(old) == prefix;
        for (i, line) in self.lines.iter().enumerate().skip(start).take(len) {
            let text = if same || line.is_empty() {
                line.to_string()
            } else {
                let lead = line.len() - line.trim_start_matches(LAYOUT_WS).len();
                format!("{prefix}{}", &line[lead.min(from)..])
            };
            self.out.push((text, Some(i)));
        }
    }

    /// Render a new or changed block. A changed block's lines take the
    /// terminators of the old block it replaces (`hint`), line by line.
    fn render(&mut self, raw: &str, prefix: &str, hint: Option<usize>) {
        let origin = |j: usize| {
            hint.map(|o| &self.olds[o])
                .filter(|old| j < old.len)
                .map(|old| old.start + j)
        };
        let mut lines = raw.split('\n');
        let first = lines.next().unwrap_or("");
        // A changed unbulleted heading stays unbulleted while lsdoc still
        // reads its first line, at this column, as one (OG writes a leading
        // heading so: og@6e7afa8 file/core.cljs `transform-content`).
        if hint.is_some_and(|o| !self.olds[o].bulleted)
            && first
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.starts_with([' ', '\t']))
            && is_unbulleted_heading_line(first)
        {
            let emitted: Vec<_> = raw
                .split('\n')
                .enumerate()
                .map(|(j, line)| (line.to_string(), origin(j)))
                .collect();
            self.out.extend(emitted);
            return;
        }
        let first = if first.is_empty() {
            format!("{prefix}-")
        } else {
            format!("{prefix}- {first}")
        };
        let untouched = self.untouched_lines(raw, prefix, hint);
        let mut emitted = vec![(first, origin(0))];
        for (j, line) in lines.enumerate() {
            let j = j + 1;
            emitted.push(match untouched.get(&j) {
                Some(&i) => (self.lines[i].to_string(), Some(i)),
                None if line.is_empty() => (String::new(), origin(j)),
                None => (format!("{prefix}  {line}"), origin(j)),
            });
        }
        self.out.extend(emitted);
    }

    /// The physical continuation lines of a changed block that the edit did
    /// not touch, keyed by their line in `raw`: a raw line in the common head or
    /// tail of the old and new raw lines whose old bytes still dedent to it
    /// under the rendered bullet. Rendering would rewrite such a line from its
    /// raw text and lose layout the DTO cannot carry: a whitespace-only line
    /// (raw empty) or a continuation indented less than the bullet's content
    /// column. Only a block that keeps its old prefix reuses them (a re-based
    /// block rewrites its lines anyway). O(block lines).
    fn untouched_lines(
        &self,
        raw: &str,
        prefix: &str,
        hint: Option<usize>,
    ) -> HashMap<usize, usize> {
        let Some(old) = hint.filter(|&o| self.old_prefix(o) == prefix) else {
            return HashMap::new();
        };
        let OldBlock { block, start, .. } = self.olds[old];
        let was = block.raw();
        let (a, b): (Vec<_>, Vec<_>) = (was.split('\n').collect(), raw.split('\n').collect());
        let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let tail = a[head.min(a.len())..]
            .iter()
            .rev()
            .zip(b[head.min(b.len())..].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        // `render` writes `{prefix}- …`, so continuations dedent by this much.
        let indent = prefix.len() + 2;
        (1..b.len())
            .filter_map(|j| {
                let k = if j < head {
                    j
                } else if j >= b.len() - tail {
                    a.len() - (b.len() - j)
                } else {
                    return None;
                };
                let line = self.lines[start + k];
                (doc::continuation_raw(line, indent) == b[j]).then_some((j, start + k))
            })
            .collect()
    }
}

#[cfg(test)]
mod org_terminator_tests {
    use super::restore_org;
    use tine_core::org::{parse_org, serialize_org_detect};

    /// Save `edit` of `source`'s model the way the store does.
    fn save(source: &str, edit: impl FnOnce(&mut tine_core::doc::Document)) -> String {
        let old = parse_org(source);
        let mut doc = old.clone();
        edit(&mut doc);
        restore_org(serialize_org_detect(&doc, Some(source)), &doc, source, &old)
    }

    /// A source with one kind of terminator is restored without block or
    /// line matching (a 2,000-block reversal would otherwise run the LCS).
    #[test]
    fn a_uniform_ending_save_does_no_matching() {
        let matches = || super::BLOCK_MATCHES.with(std::cell::Cell::get);
        let reversed = |d: &mut tine_core::doc::Document| d.roots.reverse();
        let lines: Vec<String> = (0..2_000).map(|i| format!("* b{i}")).collect();
        let before = matches();
        let saved = save(&(lines.join("\r\n") + "\r\n"), reversed);
        assert_eq!(matches(), before, "uniform CRLF matched blocks");
        assert!(saved.starts_with("* b1999\r\n* b1998\r\n"));
        assert_eq!(saved.matches("\r\n").count(), saved.matches('\n').count());
        assert_eq!(save("* a\r* b\r", reversed), "* b\r* a\r");
        assert_eq!(matches(), before, "uniform lone CR matched blocks");
        assert_eq!(save("* a\r\n* b\n", reversed), "* b\n* a\r\n");
        assert_eq!(matches(), before + 1, "a mixed source pairs blocks");
    }

    #[test]
    fn each_line_keeps_its_own_terminator() {
        assert_eq!(
            save("* a\n", |d| d.roots.push(d.roots[0].clone())),
            "* a\n* a\n"
        );
        assert_eq!(save("* a\r\n", |_| {}), "* a\r\n");
        assert_eq!(save("* a\r* b\r", |_| {}), "* a\r* b\r");
        assert_eq!(
            save("* a\r* b", |d| d.roots[0].set_raw("a!".to_string())),
            "* a!\r* b"
        );
        let added = |d: &mut tine_core::doc::Document| {
            let mut n = d.roots[0].clone();
            n.set_raw("n".to_string());
            d.roots.insert(1, n);
        };
        assert_eq!(save("* a\r\n* b\r\n", added), "* a\r\n* n\r\n* b\r\n");
        assert_eq!(
            save("* a\n* b\r", added),
            "* a\n* n\r* b\r",
            "convention: lone CR"
        );
        assert_eq!(
            save("* a\n* b\r", |d| d.roots[0].set_raw("x".to_string())),
            "* x\n* b\r",
            "an edited line keeps its own terminator"
        );
        assert_eq!(save("* a\r\n*\r\n\r\n* b", |_| {}), "* a\r\n*\r\n\r\n* b");
        assert_eq!(save("* a\r\r\n* b\n", |_| {}), "* a\r\r\n* b\n");
    }

    #[test]
    fn a_moved_block_keeps_its_terminators() {
        assert_eq!(
            save("* a\r\n* b\n* c\r\n", |d| d.roots.swap(0, 1)),
            "* b\n* a\r\n* c\r\n"
        );
        assert_eq!(
            save("#+title: T\r\n\n* a\r\nbody\n* b\n\r\n", |d| d
                .roots
                .swap(0, 1)),
            "#+title: T\r\n\n* b\n* a\r\nbody\n\r\n"
        );
    }
}
