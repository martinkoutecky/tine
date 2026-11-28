//! The splice write: replace named content ranges of slots, every other file
//! byte untouched (R3), or refuse the whole edit.
//!
//! The rule is the block-text model's `checkedWrite`
//! (`docs/block-text-model/lean/BlockText.lean`); each step below is one
//! function and names the model's definition it implements. The model is
//! over bytes; this interface takes `str`, so it rejects a range off a
//! character boundary, an out-of-range splice and a batch that is not
//! separate, and beyond that refuses exactly what the model refuses.

use crate::{Break, ContentMap, Owner, PageText};
use std::ops::Range;

/// Replace `range` of slot `slot`'s content with `text`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Splice<'s> {
    pub slot: usize,
    pub range: Range<usize>,
    pub text: &'s str,
}

/// Why [`apply`] wrote nothing. `OutOfRange`, `NotCharBoundary` and
/// `NotSeparate` reject a batch outside the write's contract;
/// `CarriageReturn` and `EmptySliceEnd` are the write rule's two refusals.
/// The refusal is the first of these causes, in this order, that the batch
/// has, and it names the caller's index of the first splice with that cause
/// in the model's order (slot, then range, then replacement), so neither
/// depends on the order of the batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpliceError {
    /// No such slot, or the range is reversed or past the content's end.
    OutOfRange(usize),
    /// An end of the range splits a character of the content.
    NotCharBoundary(usize),
    /// The splice and an earlier one of its slot are not separate: their
    /// ranges overlap, one is an insertion strictly inside the other, or both
    /// are insertions at one gap.
    NotSeparate(usize),
    /// The replacement has a CR: content has none, a CR would be a break the
    /// content cannot show.
    CarriageReturn(usize),
    /// The edited slot's last line would be empty where its slice ends
    /// without a break (the end of the file, or a slot cut mid-line), with no
    /// layout to carry the line: the line would not be read back.
    EmptySliceEnd(usize),
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

/// One edited slot's splices and what the write adds around them.
struct Edited<'p, 's> {
    group: &'p [Planned<'s>],
    continuation: String,
    space: bool,
}

/// The page with every splice applied, all ranges taken in the one snapshot
/// `page`, or the reason the edit is refused ([`SpliceError`]); refusals are
/// decided from the input before any byte is written. Each content range is
/// mapped to file bytes and replaced; every other byte of the file is kept,
/// in order (R3). A `"\n"` in a replacement is written as [`break_style`]
/// followed by the slot's continuation layout ([`PageText::continuation`]); a
/// bare header may gain one space ([`separator`]); a kept bare CR is never
/// followed by an LF ([`repair_cr`]). The order of `splices` changes neither
/// the bytes nor the refusal. Whether the result reads back as the same
/// blocks depends on the parser placing them there; that is the caller's
/// check.
///
/// Cost: one sort of the splices, O(log lines of its slot) per splice and per
/// edited slot, one parse of one line per edited bare slot whose first line
/// changed, and one copy of the output into a buffer of its exact length.
pub fn apply(page: &PageText<'_>, splices: &[Splice<'_>]) -> Result<String, SpliceError> {
    let plan = ordered(page, splices)?;
    let groups: Vec<_> = plan.chunk_by(|a, b| a.slot == b.slot).collect();
    for group in &groups {
        let map = page.content_map(Owner::Slot(group[0].slot));
        if empty_slice_end(page, &map, group) {
            return Err(SpliceError::EmptySliceEnd(group[0].index));
        }
    }
    let edited: Vec<Edited<'_, '_>> = groups
        .into_iter()
        .map(|group| {
            let n = group[0].slot;
            Edited {
                group,
                continuation: page.continuation(n),
                space: separator(page, &page.content_map(Owner::Slot(n)), group),
            }
        })
        .collect();
    let mut length = Length::default();
    write(page, &edited, &mut length);
    let mut out = String::with_capacity(length.len);
    write(page, &edited, &mut out);
    Ok(out)
}

/// Where [`write`] puts bytes: the output, or a count of its length that
/// knows whether the bytes so far end in a CR.
trait Sink {
    fn push(&mut self, s: &str);
    fn ends_in_cr(&self) -> bool;
}

impl Sink for String {
    fn push(&mut self, s: &str) {
        self.push_str(s);
    }
    fn ends_in_cr(&self) -> bool {
        self.ends_with('\r')
    }
}

/// The length of the output and whether it ends in a CR so far. O(1) per
/// piece.
#[derive(Default)]
struct Length {
    len: usize,
    cr: bool,
}

impl Sink for Length {
    fn push(&mut self, s: &str) {
        if let Some(last) = s.as_bytes().last() {
            self.len += s.len();
            self.cr = *last == b'\r';
        }
    }
    fn ends_in_cr(&self) -> bool {
        self.cr
    }
}

/// The new file: each edited slot's splices in order, with the kept runs of
/// the snapshot between them.
fn write(page: &PageText<'_>, edited: &[Edited<'_, '_>], out: &mut impl Sink) {
    let text = page.text();
    let mut at = 0;
    for slot in edited {
        for (k, splice) in slot.group.iter().enumerate() {
            push_kept(out, &text[at..splice.file.start]);
            if k == 0 && slot.space {
                out.push(" ");
            }
            push_replacement(out, splice, &slot.continuation);
            at = splice.file.end;
        }
    }
    push_kept(out, &text[at..]);
}

/// The splices in the model's order (`ordered`, by `keyLE`: slot, then
/// start, then stop, so an insertion comes before a replacement that starts
/// at its gap; equal ranges, never separate, by replacement), each checked
/// against the content and mapped to file bytes. The batch is refused for
/// the first cause any splice has, in [`SpliceError`]'s order: out of range,
/// off a character boundary, not `separate` from an earlier splice of its
/// slot, a replacement with a CR (`crFreeCheck`); named at the first such
/// splice in this order.
fn ordered<'s>(
    page: &PageText<'_>,
    splices: &[Splice<'s>],
) -> Result<Vec<Planned<'s>>, SpliceError> {
    let text = page.text();
    let mut order: Vec<usize> = (0..splices.len()).collect();
    order.sort_by_key(|i| {
        let s = &splices[*i];
        (s.slot, s.range.start, s.range.end, s.text)
    });
    let (mut out_of_range, mut split, mut cr) = (None, None, None);
    let mut plan = Vec::with_capacity(splices.len());
    for index in order {
        let splice = &splices[index];
        let range = splice.range.clone();
        if splice.slot >= page.slots().len() || range.start > range.end {
            out_of_range.get_or_insert(index);
            continue;
        }
        let map = page.content_map(Owner::Slot(splice.slot));
        if range.end > map.len() {
            out_of_range.get_or_insert(index);
            continue;
        }
        let file = map.to_file(range.start)..map.to_file(range.end);
        if !text.is_char_boundary(file.start) || !text.is_char_boundary(file.end) {
            split.get_or_insert(index);
        }
        if splice.text.contains('\r') {
            cr.get_or_insert(index);
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
    if let Some(index) = out_of_range {
        return Err(SpliceError::OutOfRange(index));
    }
    if let Some(index) = split {
        return Err(SpliceError::NotCharBoundary(index));
    }
    if let Some(index) = not_separate(&plan) {
        return Err(SpliceError::NotSeparate(index));
    }
    if let Some(index) = cr {
        return Err(SpliceError::CarriageReturn(index));
    }
    Ok(plan)
}

/// The index of the first splice that is not `separate` from an earlier one
/// of its slot, in the sorted `plan`: ranges are half-open, an insertion may
/// touch another range's end or start, but not lie strictly inside it, and
/// two insertions may not share a gap. In sorted order this is: each splice
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

/// The text of the slot's first line after the whole edit, up to its first
/// `"\n"` (`firstRowText` of the edited stream): kept content and
/// replacements in order. O(that line + splices + log lines).
fn first_line(page: &PageText<'_>, map: &ContentMap<'_>, group: &[Planned<'_>]) -> String {
    let text = page.text();
    let mut line = String::new();
    // Kept content `at..stop`, up to its first separator; whether it has one.
    let kept = |line: &mut String, at: usize, stop: usize| {
        if at >= stop {
            return false;
        }
        let end = map.line_end(at);
        line.push_str(&text[map.to_file(at)..map.to_file(end.min(stop))]);
        end < stop
    };
    let mut at = 0;
    for splice in group {
        if kept(&mut line, at, splice.range.start) {
            return line;
        }
        if let Some((head, _)) = splice.text.split_once('\n') {
            line.push_str(head);
            return line;
        }
        line.push_str(splice.text);
        at = splice.range.end;
    }
    kept(&mut line, at, map.len());
    line
}

/// Whether bare slot `group[0].slot` earns one space after its marker
/// (`separator`): its first sorted splice starts at content offset 0, the
/// edited first line ([`first_line`]) is not empty and differs from the
/// snapshot's, and lsdoc does not read the header's layout followed by that
/// line as the same header ([`PageText::head_reads`]). The space is header
/// layout at the first content gap, where that first splice starts.
/// O(splices + first line + log lines), and the one-line parse only when the
/// first line changed.
fn separator(page: &PageText<'_>, map: &ContentMap<'_>, group: &[Planned<'_>]) -> bool {
    let n = group[0].slot;
    if !page.slots()[n].bare() || group[0].range.start != 0 {
        return false;
    }
    let text = page.text();
    let new = first_line(page, map, group);
    let old = &text[map.to_file(0)..map.to_file(map.line_end(0))];
    !new.is_empty() && new != old && !page.head_reads(n, &new)
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
fn repair_cr(out: &mut impl Sink) {
    if out.ends_in_cr() {
        out.push("\r");
    }
}

/// Copy a kept run of the snapshot. Its first byte follows a splice (or
/// starts the file); when that byte is a kept LF the splice exposed to a bare
/// CR, the CR [`repair_cr`] writes is the splice's last byte.
fn push_kept(out: &mut impl Sink, kept: &str) {
    if kept.starts_with('\n') {
        repair_cr(out);
    }
    out.push(kept);
}

/// Write a replacement: its bytes, each `"\n"` as the splice's break style
/// and the slot's continuation layout.
fn push_replacement(out: &mut impl Sink, splice: &Planned<'_>, continuation: &str) {
    for (n, segment) in splice.text.split('\n').enumerate() {
        if n > 0 {
            if splice.style == "\n" {
                repair_cr(out);
            }
            out.push(splice.style);
            out.push(continuation);
        }
        out.push(segment);
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
    use crate::{Format, HEAD_QUERIES, VISITS};

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

    /// The header question costs one parse of one line per edited bare slot
    /// whose first line changed, and none for any other slot: not for a
    /// header with a separator, a bare header edited after its first line or
    /// at a later offset of it, an edit that leaves the first line as it was,
    /// or one that empties it.
    #[test]
    fn a_bare_header_is_asked_about_once_when_its_first_line_changes() {
        let slots = 40;
        let mut text = String::new();
        for _ in 0..slots {
            text += "-\tT\n  u\n- a\n";
        }
        let page = PageText::read_text(&text, Format::Markdown).unwrap();
        assert_eq!(page.slots().len(), 2 * slots);
        let edit =
            |slot: usize, range: Range<usize>, text: &'static str| Splice { slot, range, text };
        // Bare slot 2k gets edit k % 6; every bullet slot 2k+1 gets `x` at 0.
        let mut splices = Vec::new();
        let mut changed = 0;
        for k in 0..slots {
            let bare = 2 * k;
            splices.push(match k % 6 {
                0 => {
                    changed += 1;
                    edit(bare, 0..0, "x")
                }
                1 => {
                    changed += 1;
                    edit(bare, 0..1, "")
                }
                2 => edit(bare, 3..4, "v"),
                3 => edit(bare, 1..2, "S"),
                4 => edit(bare, 0..1, "\t"),
                _ => edit(bare, 0..0, "\n"),
            });
            splices.push(edit(bare + 1, 0..0, "x"));
        }
        HEAD_QUERIES.with(|n| n.set(0));
        apply(&page, &splices).unwrap();
        assert_eq!(HEAD_QUERIES.with(|n| n.get()), changed);
    }
}
