//! The splice write: replace named content ranges of slots, every other file
//! byte untouched (R3), or refuse the whole edit.
//!
//! The rule is the block-text model's `checkedWrite`
//! (`docs/block-text-model/lean/BlockText.lean`); each step below is one
//! function and names the model's definition it implements.

use crate::{Break, ContentMap, Owner, PageText};
use std::ops::Range;

/// Replace `range` of slot `slot`'s content with `text`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Splice<'s> {
    pub slot: usize,
    pub range: Range<usize>,
    pub text: &'s str,
}

/// Why [`apply`] wrote nothing. `OutOfRange` and `NotSeparate` reject a
/// batch outside the write's contract; `CarriageReturn` and `EmptySliceEnd`
/// are the write rule's two refusals. Each names the index of a splice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpliceError {
    /// No such slot, or the range is reversed, past the content's end or not
    /// on a character boundary.
    OutOfRange(usize),
    /// The splice and another of its slot are not separate: their ranges
    /// overlap, one is an insertion strictly inside the other, or both are
    /// insertions at one gap.
    NotSeparate(usize),
    /// The replacement has a CR: content has none, a CR would be a break the
    /// content cannot show.
    CarriageReturn(usize),
    /// The edited slot's last line would be empty where its slice ends
    /// without a break (the end of the file, or a slot cut mid-line), with no
    /// layout to carry the line: the line would not be read back.
    EmptySliceEnd(usize),
}

/// Whether lsdoc's outline header rule takes byte `b` after a marker (`-`,
/// a `*` run) as the end of the marker: space, tab or form feed (lsdoc
/// `mldoc_heading_boundary`, `src/block_common.rs`; the end of the line also
/// ends it). The predicate the model's `separator` is given.
pub fn takes_separator(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | 0x0c)
}

/// One splice, checked and mapped to file bytes.
struct Planned<'s> {
    index: usize,
    slot: usize,
    range: Range<usize>,
    file: Range<usize>,
    text: &'s str,
    /// The break its `"\n"`s are written as.
    style: &'static str,
}

/// The page with every splice applied, all ranges taken in the one snapshot
/// `page`, or the reason the edit is refused; refusals are decided from the
/// input before any byte is written. Each content range is mapped to file
/// bytes and replaced; every other byte of the file is kept, in order (R3).
/// A `"\n"` in a replacement is written as [`break_style`] followed by the
/// slot's continuation layout ([`PageText::continuation`]); a bare header
/// may gain one space ([`separator`]); a kept bare CR is never followed by an
/// LF ([`repair_cr`]). The order of `splices` changes neither the bytes nor
/// the refusal. Whether the result reads back as the same blocks depends on
/// the parser placing them there; that is the caller's check.
///
/// Cost: one sort of the splices, O(log lines of its slot) per splice and per
/// edited slot, and one copy of the output.
pub fn apply(page: &PageText<'_>, splices: &[Splice<'_>]) -> Result<String, SpliceError> {
    let plan = ordered(page, splices)?;
    let text = page.text();
    let mut out = String::with_capacity(text.len() + 2 * plan.len());
    let mut groups = Vec::new();
    for group in plan.chunk_by(|a, b| a.slot == b.slot) {
        let map = page.content_map(Owner::Slot(group[0].slot));
        if empty_slice_end(page, &map, group) {
            return Err(SpliceError::EmptySliceEnd(group[0].index));
        }
        groups.push((group, map));
    }
    let mut at = 0;
    for (group, map) in groups {
        let n = group[0].slot;
        let continuation = page.continuation(n);
        let space = separator(page, &map, group);
        for (k, splice) in group.iter().enumerate() {
            push_kept(&mut out, &text[at..splice.file.start]);
            if k == 0 && space {
                out.push(' ');
            }
            push_replacement(&mut out, splice, &continuation);
            at = splice.file.end;
        }
    }
    push_kept(&mut out, &text[at..]);
    Ok(out)
}

/// The splices in the model's order (`ordered`, by `keyLE`: slot, then
/// start, then stop, so an insertion comes before a replacement that starts
/// at its gap), each checked against the content and mapped to file bytes,
/// and pairwise `separate` within a slot. A replacement with a CR is refused
/// (`crFreeCheck`).
fn ordered<'s>(
    page: &PageText<'_>,
    splices: &[Splice<'s>],
) -> Result<Vec<Planned<'s>>, SpliceError> {
    let text = page.text();
    let mut plan = Vec::with_capacity(splices.len());
    for (index, splice) in splices.iter().enumerate() {
        let range = splice.range.clone();
        if splice.slot >= page.slots().len() || range.start > range.end {
            return Err(SpliceError::OutOfRange(index));
        }
        if splice.text.contains('\r') {
            return Err(SpliceError::CarriageReturn(index));
        }
        let map = page.content_map(Owner::Slot(splice.slot));
        if range.end > map.len() {
            return Err(SpliceError::OutOfRange(index));
        }
        let file = map.to_file(range.start)..map.to_file(range.end);
        if !text.is_char_boundary(file.start) || !text.is_char_boundary(file.end) {
            return Err(SpliceError::OutOfRange(index));
        }
        plan.push(Planned {
            index,
            slot: splice.slot,
            style: break_style(&map, range.start),
            range,
            file,
            text: splice.text,
        });
    }
    plan.sort_by_key(|p| (p.slot, p.range.start, p.range.end));
    if let Some(bad) = not_separate(&plan) {
        return Err(SpliceError::NotSeparate(bad));
    }
    Ok(plan)
}

/// The index of a splice that is not `separate` from an earlier one of its
/// slot, in the sorted `plan`: ranges are half-open, an insertion may touch
/// another range's end or start, but not lie strictly inside it, and two
/// insertions may not share a gap. In sorted order this is: each splice
/// starts at or after the stops of the earlier ones of its slot, and none
/// repeats the range before it. O(splices).
fn not_separate(plan: &[Planned<'_>]) -> Option<usize> {
    let mut stop = 0;
    for pair in plan.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.slot != b.slot {
            stop = 0;
            continue;
        }
        stop = stop.max(a.range.end);
        if b.range.start < stop || a.range == b.range {
            return Some(b.index);
        }
    }
    None
}

/// The break a replacement's `"\n"` is written as (`encode`, with
/// `followingEnding` and `insertedEnding`): taken from the snapshot line
/// holding content offset `start`, CRLF for a CRLF line and LF for every
/// other (LF, bare CR, end of the slice). O(log lines).
fn break_style(map: &ContentMap<'_>, start: usize) -> &'static str {
    match map.break_at(start) {
        Break::CrLf => "\r\n",
        _ => "\n",
    }
}

/// Content byte `at` of the slot `map` maps (`"\n"` for a line separator),
/// `None` past the end. O(log lines).
fn content_byte(page: &PageText<'_>, map: &ContentMap<'_>, at: usize) -> Option<u8> {
    (at < map.len()).then(|| match page.text().as_bytes()[map.to_file(at)] {
        b'\r' | b'\n' => b'\n',
        b => b,
    })
}

/// Whether bare slot `group[0].slot` earns one space after its marker
/// (`separator`): after the whole edit, the first line's first text byte
/// (`firstTextByte`) differs from the snapshot's, which may be absent, and is
/// not one the header rule takes as a separator ([`takes_separator`]). The
/// space is header layout at the first content gap; the first sorted splice
/// starts there whenever it is earned. O(splices + log lines).
fn separator(page: &PageText<'_>, map: &ContentMap<'_>, group: &[Planned<'_>]) -> bool {
    if !page.slots()[group[0].slot].bare() {
        return false;
    }
    let text_byte = |b: Option<u8>| b.filter(|b| *b != b'\n');
    // The edited content's first byte: the first splice's replacement when
    // it starts where the kept content so far ends, else the kept byte.
    let mut at = 0;
    let mut first = None;
    for splice in group {
        if splice.range.start > at {
            break;
        }
        if let Some(b) = splice.text.bytes().next() {
            first = Some(Some(b));
            break;
        }
        at = splice.range.end;
    }
    let new = text_byte(first.unwrap_or_else(|| content_byte(page, map, at)));
    let old = text_byte(content_byte(page, map, 0));
    new.is_some_and(|b| Some(b) != old && !takes_separator(b))
}

/// The refusal `EmptySliceEnd`: the slot's last line ends its slice without
/// a break, and after the edit that line has no byte at all
/// (`lastBodyEmpty`): the edited content's last byte is a `"\n"` with no
/// layout after it, or the content is empty and so is the header's layout.
/// O(splices + log lines).
fn empty_slice_end(page: &PageText<'_>, map: &ContentMap<'_>, group: &[Planned<'_>]) -> bool {
    let len = map.len();
    if !matches!(map.final_break(), Break::Eof | Break::Continues) {
        return false;
    }
    let text = page.text().as_bytes();
    let n = group[0].slot;
    // The edited content's last byte, walking back from the end: a
    // replacement's last byte where it ends at the kept content's start.
    let mut at = len;
    for splice in group.iter().rev() {
        if splice.range.end < at {
            break;
        }
        if let Some(b) = splice.text.bytes().last() {
            return b == b'\n' && page.continuation(n).is_empty();
        }
        at = splice.range.start;
    }
    match at.checked_sub(1) {
        // The kept content's last byte: a line separator's layout is the
        // next line's removed indent, empty when a break byte precedes it.
        Some(last) => {
            content_byte(page, map, last) == Some(b'\n')
                && matches!(text[map.to_file(last + 1) - 1], b'\r' | b'\n')
        }
        None => map.to_file(0) == page.slots()[n].start(),
    }
}

/// The model's `repairEnding`, in bytes: a kept bare CR is never followed by
/// an LF, which would read as one CRLF break. Before an LF break written
/// right after a bare CR, write one CR, so the LF reads as its own CRLF.
/// Content and layout bytes are never CR, so a CR at the end of the output
/// is always a bare-CR break.
fn repair_cr(out: &mut String) {
    if out.ends_with('\r') {
        out.push('\r');
    }
}

/// Copy a kept run of the snapshot. Its first byte follows a splice (or
/// starts the file); when that byte is a kept LF the splice exposed to a bare
/// CR, the CR [`repair_cr`] writes is the splice's last byte.
fn push_kept(out: &mut String, kept: &str) {
    if kept.starts_with('\n') {
        repair_cr(out);
    }
    out.push_str(kept);
}

/// Write a replacement: its bytes, each `"\n"` as the splice's break style
/// and the slot's continuation layout.
fn push_replacement(out: &mut String, splice: &Planned<'_>, continuation: &str) {
    for (n, segment) in splice.text.split('\n').enumerate() {
        if n > 0 {
            if splice.style == "\n" {
                repair_cr(out);
            }
            out.push_str(splice.style);
            out.push_str(continuation);
        }
        out.push_str(segment);
    }
}

/// Whether physical rows (each a line's bytes before its break, and the
/// break) read back as exactly these rows: no row holds a CR or LF, a row
/// ending at the end of the file (or of a slot cut mid-line) is nonempty and
/// last, and a row ending in a bare CR is not followed by an empty LF row
/// (the two would read as one CRLF). This condition is necessary and
/// sufficient (`lexicalCheck_iff`). O(rows + bytes).
pub fn check_planned_rows(rows: &[(&[u8], Break)]) -> bool {
    rows.iter().enumerate().all(|(i, (body, end))| {
        let last = i + 1 == rows.len();
        !body.iter().any(|b| matches!(b, b'\n' | b'\r'))
            && match end {
                Break::Eof | Break::Continues => !body.is_empty() && last,
                Break::Cr => rows.get(i + 1) != Some(&(&[][..], Break::Lf)),
                Break::Lf | Break::CrLf => true,
            }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Format, VISITS};

    fn visits(page: &PageText<'_>, splices: &[Splice<'_>]) -> (String, usize) {
        VISITS.with(|n| n.set(0));
        let out = apply(page, splices).unwrap();
        (out, VISITS.with(|n| n.get()))
    }

    /// Two Markdown slots of `lines` continuation lines each, the second
    /// indented by `indent`, so its continuation indent is that much longer.
    fn page_text(lines: usize, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut text = String::from("- a\n");
        text += &"  x\n".repeat(lines);
        text += &format!("{pad}- b\n");
        text += &format!("{pad}  x\n").repeat(lines);
        text
    }

    /// The work of `apply` per splice is a lookup in its slot's lines: it does
    /// not grow with the slot's indent, nor with how the splices alternate
    /// between slots, and no map is rebuilt.
    #[test]
    fn apply_costs_a_lookup_per_splice() {
        let lines: usize = 2_400;
        // Probes of one binary search over the slot's lines.
        let probes = (usize::BITS - lines.leading_zeros()) as usize + 1;
        for indent in [0, 40] {
            let text = page_text(lines, indent);
            let page = PageText::read_text(&text, Format::Markdown).unwrap();
            assert_eq!(page.slots().len(), 2);
            let splice = |slot: usize, line: usize| Splice {
                slot,
                range: 2 + 2 * line..3 + 2 * line,
                text: "y\nz",
            };
            let alternating: Vec<_> = (0..lines)
                .flat_map(|k| [splice(0, k), splice(1, k)])
                .collect();
            let grouped: Vec<_> = (0..2)
                .flat_map(|slot| (0..lines).map(move |k| splice(slot, k)))
                .collect();
            let (a, a_visits) = visits(&page, &alternating);
            let (b, b_visits) = visits(&page, &grouped);
            assert_eq!(a, b);
            assert_eq!(a_visits, b_visits, "indent {indent}");
            // Two lookups (start and end) and one for the break, each a
            // binary search of the slot's lines.
            assert!(
                a_visits <= alternating.len() * 3 * probes,
                "{a_visits} visits for {} splices",
                alternating.len()
            );
        }
        let visits_at = |indent| {
            let text = page_text(lines, indent);
            let page = PageText::read_text(&text, Format::Markdown).unwrap();
            let one = [Splice {
                slot: 1,
                range: 4..5,
                text: "y\nz",
            }];
            visits(&page, &one).1
        };
        assert_eq!(visits_at(0), visits_at(40));
    }
}
