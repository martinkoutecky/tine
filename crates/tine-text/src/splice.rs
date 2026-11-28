//! The splice write (R3): replace named content ranges of slots, every other
//! file byte untouched.

use crate::{Owner, PageText};
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

/// The page with every splice applied. Each content range is mapped to file
/// bytes and replaced; a `"\n"` in a replacement is written as the break of
/// the line the splice starts in (LF when that line has none) followed by the
/// slot's continuation indent. Splices at one point apply in the given order.
/// O(page bytes + spliced slots' lines).
pub fn apply(page: &PageText<'_>, splices: &[Splice<'_>]) -> Result<String, SpliceError> {
    let text = page.text();
    let mut writes: Vec<(Range<usize>, usize, &str, String)> = Vec::with_capacity(splices.len());
    let mut map = None;
    for (i, splice) in splices.iter().enumerate() {
        if splice.slot >= page.slots().len() || splice.range.start > splice.range.end {
            return Err(SpliceError::OutOfRange(i));
        }
        if splice.text.contains('\r') {
            return Err(SpliceError::CarriageReturn(i));
        }
        if map.as_ref().map(|(slot, _)| *slot) != Some(splice.slot) {
            map = Some((splice.slot, page.content_map(splice.slot)));
        }
        let map = &map.as_ref().expect("set above").1;
        if splice.range.end > map.len() {
            return Err(SpliceError::OutOfRange(i));
        }
        let range = map.to_file(splice.range.start)..map.to_file(splice.range.end);
        if !text.is_char_boundary(range.start) || !text.is_char_boundary(range.end) {
            return Err(SpliceError::OutOfRange(i));
        }
        let brk = match map.pieces()[map.piece_at(splice.range.start)]
            .1
            .kind
            .as_str()
        {
            "" => "\n",
            brk => brk,
        };
        let newline = format!("{brk}{}", continuation_indent(page, splice.slot));
        writes.push((range, i, splice.text, newline));
    }
    writes.sort_by_key(|(range, i, ..)| (range.start, *i));
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (range, i, replacement, newline) in writes {
        if range.start < at {
            return Err(SpliceError::Overlap(i));
        }
        out.push_str(&text[at..range.start]);
        for (n, segment) in replacement.split('\n').enumerate() {
            if n > 0 {
                out.push_str(&newline);
            }
            out.push_str(segment);
        }
        at = range.end;
    }
    out.push_str(&text[at..]);
    Ok(out)
}

/// The indent a new line of slot `n` is written with: the header's leading
/// layout, then spaces up to the slot's dedent (`"  "` under `- a`).
fn continuation_indent(page: &PageText<'_>, n: usize) -> String {
    let dedent = page.slots()[n].dedent();
    let first = page
        .pieces(Owner::Slot(n))
        .next()
        .expect("a slot has a piece");
    let lead: String = page.text()[first.layout]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t' | '\x0c'))
        .take(dedent)
        .collect();
    let pad = dedent - lead.len();
    lead + &" ".repeat(pad)
}
