//! Per-block physical retention: a Markdown save rewrites only what it changed.
//!
//! The DTO carries each block's semantic text, not the file's layout: base
//! indentation offsets, depth jumps, continuation indentation, whitespace-only
//! lines. Rebuilding the whole page from the DTO would rewrite that layout in
//! blocks the save never touched. Instead, every DTO block whose raw text equals
//! an old block's raw (matched by a pre-order LCS, then by equal text for moved
//! blocks) reuses that block's physical lines; only new or changed blocks, and
//! the page-property preamble when it changed, are rendered. A reused block
//! keeps its indentation unless the parse rule for its new position forbids it
//! (it must be deeper than its parent and no deeper than its previous sibling);
//! then its lines are re-based as a unit onto a valid prefix.
//!
//! Cost: two parses of the page (three, plus a full serialization, when the
//! DTO itself does not round-trip) and an LCS over the changed middle of the
//! pre-order block sequence (common prefix and suffix are trimmed first),
//! capped at 4,000,000 table cells (about 16 MB); a larger middle skips the
//! LCS and matches blocks by equal text only.

use std::collections::{HashMap, VecDeque};

use tine_core::doc::{self, DocBlock, Document, SerializeOpts};

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
/// is also not `doc`. Pure. Cost: two parses of the page (three plus a full
/// serialization when the first check fails) and an LCS over the changed middle
/// of the pre-order block sequence, O(a·b) with a `u32` table, skipped above
/// 4,000,000 cells (about 16 MB).
pub(super) fn serialize(doc: &Document, source: &str, opts: &SerializeOpts) -> Option<String> {
    if doc.roots.is_empty() && doc.pre_block.is_none() {
        return None;
    }
    let old = doc::parse(source);
    let (lines, ends) = line_endings::split(source);
    let olds = map_old_blocks(&old, &lines)?;
    let mut news = Vec::new();
    flatten(&doc.roots, &mut news);
    let (keep, hint) = match_blocks(&olds, &news);

    let region = olds.first().map_or(lines.len(), |o| o.start);
    let mut out: Vec<(String, Option<usize>)> = Vec::new();
    emit_preamble(&old, doc, &lines, region, opts, &mut out);
    let mut emitter = Emitter {
        lines: &lines,
        olds: &olds,
        keep,
        hint,
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
    let reparsed = doc::parse(&result);
    if reparsed.pre_block == doc.pre_block && reparsed.roots == doc.roots {
        return Some(result);
    }
    // A DTO that cannot round-trip at all (e.g. blocks after an unterminated
    // fence) keeps this layout only when it means what a full rebuild means.
    (reparsed == doc::parse(&doc::serialize_with(doc, opts))).then_some(result)
}

/// An old block's pre-order position and its physical lines.
struct OldBlock<'a> {
    raw: &'a str,
    start: usize,
    len: usize,
    /// Written as `- …`; false for an unbulleted ATX heading (lsdoc), whose
    /// lines are its raw text verbatim.
    bulleted: bool,
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
    for block in flat {
        let raw = block.raw();
        let len = raw.split('\n').count();
        let head = lines[start].trim_start_matches([' ', '\t']);
        let first = raw.split('\n').next().unwrap_or("");
        let dash = head
            .strip_prefix('-')
            .map(|rest| rest.strip_prefix(' ').unwrap_or(rest));
        let bulleted = lines[start] != first;
        if bulleted && dash != Some(first) {
            return None;
        }
        olds.push(OldBlock {
            raw,
            start,
            len,
            bulleted,
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

/// For each new block: the old block whose lines it reuses (equal raw), and
/// for unmatched ones an old block at the same place whose indentation it
/// should prefer (a changed block).
type Matches = (Vec<Option<usize>>, Vec<Option<usize>>);

fn match_blocks(olds: &[OldBlock], news: &[&DocBlock]) -> Matches {
    let (n, m) = (olds.len(), news.len());
    let mut keep = vec![None; m];
    let mut head = 0;
    while head < n.min(m) && olds[head].raw == news[head].raw() {
        keep[head] = Some(head);
        head += 1;
    }
    let mut tail = 0;
    while tail < (n - head).min(m - head) && olds[n - 1 - tail].raw == news[m - 1 - tail].raw() {
        keep[m - 1 - tail] = Some(n - 1 - tail);
        tail += 1;
    }
    let (a, b) = (&olds[head..n - tail], &news[head..m - tail]);
    // Quadratic LCS only over the changed middle; a huge middle keeps the
    // unchanged prefix/suffix and re-renders the rest.
    if !a.is_empty() && !b.is_empty() && a.len() * b.len() <= 4_000_000 {
        let w = b.len() + 1;
        let mut dp = vec![0u32; (a.len() + 1) * w];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                dp[i * w + j] = if a[i].raw == b[j].raw() {
                    dp[(i + 1) * w + j + 1] + 1
                } else {
                    dp[(i + 1) * w + j].max(dp[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            if a[i].raw == b[j].raw() {
                keep[head + j] = Some(head + i);
                i += 1;
                j += 1;
            } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    // Changed blocks: pair unmatched old and new blocks positionally inside
    // each gap between matched anchors.
    let mut hint = vec![None; m];
    let mut used = vec![false; n];
    keep.iter().flatten().for_each(|&o| used[o] = true);
    let (mut next_old, mut j) = (0, 0);
    while j < m {
        if let Some(o) = keep[j] {
            next_old = o + 1;
        } else if next_old < n && !used[next_old] {
            hint[j] = Some(next_old);
            next_old += 1;
        }
        j += 1;
    }
    // Moved blocks: unmatched new blocks with the text of an unmatched old block.
    let mut free: HashMap<&str, VecDeque<usize>> = HashMap::new();
    (0..n)
        .filter(|&o| !used[o])
        .for_each(|o| free.entry(olds[o].raw).or_default().push_back(o));
    for j in 0..m {
        if keep[j].is_none() {
            keep[j] = free.get_mut(news[j].raw()).and_then(VecDeque::pop_front);
        }
    }
    (keep, hint)
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
    unit: &'a str,
    /// Emitted lines, each with the old line it reuses (for its terminator).
    out: Vec<(String, Option<usize>)>,
}

impl Emitter<'_> {
    fn old_prefix(&self, old: usize) -> &str {
        let line = self.lines[self.olds[old].start];
        &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
    }

    /// Emit one sibling list. `parent` is the parent's chosen prefix; `index`
    /// is the pre-order index of the first sibling and advances past the list.
    fn place(&mut self, blocks: &[DocBlock], parent: Option<&str>, index: &mut usize) {
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
                parent.is_none_or(|pp| p.len() > pp.len())
                    && prev.as_ref().is_none_or(|ps| p.len() <= ps.len())
            };
            let own = self.keep[i]
                .or(self.hint[i])
                .map(|o| self.old_prefix(o).to_string());
            let next = (j + 1 < blocks.len())
                .then(|| self.keep[starts[j + 1]])
                .flatten()
                .map(|o| self.old_prefix(o).to_string());
            let prefix = own
                .filter(|p| valid(p))
                .or_else(|| prev.clone())
                .or_else(|| next.filter(|p| valid(p)))
                .unwrap_or_else(|| parent.map_or(String::new(), |pp| format!("{pp}{}", self.unit)));
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
            self.place(&block.children, Some(&prefix), index);
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
                let lead = line.len() - line.trim_start_matches([' ', '\t']).len();
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
            && first.strip_prefix(prefix).is_some_and(|rest| !rest.starts_with([' ', '\t']))
            && tine_core::doc::is_unbulleted_heading_line(first)
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
        let mut emitted = vec![(first, origin(0))];
        for (j, line) in lines.enumerate() {
            let line = if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}  {line}")
            };
            emitted.push((line, origin(j + 1)));
        }
        self.out.extend(emitted);
    }
}
