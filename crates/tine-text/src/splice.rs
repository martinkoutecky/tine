//! The splice write: replace named content ranges of slots, every other file
//! byte untouched (R3).

use crate::{Break, Owner, PageText};
use std::ops::Range;

/// Replace `range` of slot `slot`'s content with `text`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Splice<'s> {
    pub slot: usize,
    pub range: Range<usize>,
    pub text: &'s str,
}

/// Why [`apply`] wrote nothing; each names the index of the splice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpliceError {
    /// No such slot, or the range is reversed, past the content's end or not
    /// on a character boundary.
    OutOfRange(usize),
    /// The splice's file range overlaps an earlier splice's.
    Overlap(usize),
    /// The replacement has a CR: content has none, a CR would be a break the
    /// content cannot show.
    CarriageReturn(usize),
}

/// Candidate bytes of the page with every splice applied, all ranges taken in
/// the one snapshot `page`. Each content range is mapped to file bytes and
/// replaced; every other byte of the file is kept, in order (R3). A `"\n"` in
/// a replacement is written as the break of the line the splice starts in (LF
/// when that line has none) followed by the slot's continuation indent.
/// Splices are taken in file order, an insertion before a replacement that
/// starts at the same byte; insertions at one point keep the given order.
/// Whether the result reads back as the same blocks is the caller's check.
///
/// Cost: one sort of the splices, O(log lines of its slot) per splice, and
/// one copy of the output.
pub fn apply(page: &PageText<'_>, splices: &[Splice<'_>]) -> Result<String, SpliceError> {
    let text = page.text();
    let mut writes: Vec<(Range<usize>, usize, &Splice<'_>, Break)> =
        Vec::with_capacity(splices.len());
    for (i, splice) in splices.iter().enumerate() {
        if splice.slot >= page.slots().len() || splice.range.start > splice.range.end {
            return Err(SpliceError::OutOfRange(i));
        }
        if splice.text.contains('\r') {
            return Err(SpliceError::CarriageReturn(i));
        }
        let map = page.content_map(Owner::Slot(splice.slot));
        if splice.range.end > map.len() {
            return Err(SpliceError::OutOfRange(i));
        }
        let range = map.to_file(splice.range.start)..map.to_file(splice.range.end);
        if !text.is_char_boundary(range.start) || !text.is_char_boundary(range.end) {
            return Err(SpliceError::OutOfRange(i));
        }
        writes.push((range, i, splice, map.break_at(splice.range.start)));
    }
    writes.sort_by_key(|(range, i, ..)| (range.start, range.end, *i));
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (range, i, splice, brk) in writes {
        if range.start < at {
            return Err(SpliceError::Overlap(i));
        }
        out.push_str(&text[at..range.start]);
        let slot = &page.slots()[splice.slot];
        let lead = &text[slot.start..slot.start + slot.lead];
        for (n, segment) in splice.text.split('\n').enumerate() {
            if n > 0 {
                out.push_str(match brk.as_str() {
                    "" => "\n",
                    brk => brk,
                });
                // The continuation indent: the header's leading layout, then
                // spaces up to the slot's dedent (`"  "` under `- a`).
                out.push_str(lead);
                out.extend(std::iter::repeat_n(' ', slot.dedent - slot.lead));
            }
            out.push_str(segment);
        }
        at = range.end;
    }
    out.push_str(&text[at..]);
    Ok(out)
}

/// Whether physical rows (each a line's bytes before its break, and the
/// break) read back as exactly these rows: no row holds a CR or LF, a row
/// ending at the end of the file (or of a slot cut mid-line) is nonempty and
/// last, and a row ending in a bare CR is not followed by an empty LF row
/// (the two would read as one CRLF). This condition is necessary and
/// sufficient. O(rows + bytes).
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
