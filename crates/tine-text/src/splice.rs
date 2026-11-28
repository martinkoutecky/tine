use crate::{line_terminator, physical_lines, strip_layout_ws, PageText, SlotText};
use std::ops::Range;

/// A replacement in the slot's original LF-content coordinates.
pub struct Splice<'a> {
    pub slot: usize,
    pub range: Range<usize>,
    pub replacement: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpliceError {
    Slot { edit: usize },
    Opaque { edit: usize },
    Range { edit: usize },
    Overlap { edit: usize },
}

struct Prepared<'p, 'r> {
    edit: usize,
    slot: SlotText<'p>,
    native: Range<usize>,
    replacement: &'r str,
    eol: &'static str,
    separator: bool,
}

/// Replace mapped byte ranges, keeping all intervening bytes in order. The
/// input edits may be unsorted. Adjacent ranges are allowed, and insertions
/// at the same position retain input order before any replacement there.
/// Cost is one output copy, two map lookups per edit, and a sort of the edits.
pub fn apply(page: &PageText<'_>, splices: &[Splice<'_>]) -> Result<Vec<u8>, SpliceError> {
    let source = page.source();
    let mut edits = Vec::with_capacity(splices.len());
    for (edit, splice) in splices.iter().enumerate() {
        let slot = page
            .slots
            .get(splice.slot)
            .map(|slot| page.slot_text(slot))
            .ok_or(SpliceError::Slot { edit })?;
        if slot.is_opaque() {
            return Err(SpliceError::Opaque { edit });
        }
        let invalid = SpliceError::Range { edit };
        let line = slot.line_at_content(splice.range.start).ok_or(invalid)?;
        let start = line.text_start + splice.range.start - line.content_at;
        let end = slot.file_offset(splice.range.end).ok_or(invalid)?;
        if start > end || !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return Err(invalid);
        }
        if start == end && splice.replacement.is_empty() {
            continue;
        }
        let eol = if &source[line.line_break()] == "\r\n" {
            "\r\n"
        } else {
            "\n"
        };
        edits.push(Prepared {
            edit,
            slot,
            native: start..end,
            replacement: splice.replacement,
            eol,
            separator: false,
        });
    }
    edits.sort_unstable_by_key(|edit| (edit.native.start, edit.native.end, edit.edit));
    let mut last = 0;
    for edit in &edits {
        if edit.native.start < last {
            return Err(SpliceError::Overlap { edit: edit.edit });
        }
        last = edit.native.end;
    }
    for group in edits.chunk_by_mut(|a, b| std::ptr::eq(a.slot.slot, b.slot.slot)) {
        let slot = group[0].slot;
        let first = slot.lines[0];
        if slot.slot.missing_separator
            && first.text().is_empty()
            && group[0].native.start == first.text_start
        {
            group[0].separator = gains_title(
                source,
                group,
                first.text_start,
                slot.lines.last().unwrap().text_end,
            );
        }
    }
    let mut out = Vec::with_capacity(source.len());
    let mut cursor = 0;
    for edit in edits {
        out.extend_from_slice(&source.as_bytes()[cursor..edit.native.start]);
        if edit.separator {
            out.push(b' ');
        }
        write_replacement(&mut out, &edit);
        cursor = edit.native.end;
    }
    out.extend_from_slice(&source.as_bytes()[cursor..]);
    Ok(out)
}

fn gains_title(source: &str, group: &[Prepared<'_, '_>], mut at: usize, end: usize) -> bool {
    for edit in group {
        if at < edit.native.start {
            return !source[at..].starts_with(['\r', '\n']);
        }
        if !edit.replacement.is_empty() {
            return !edit.replacement.starts_with(['\r', '\n']);
        }
        at = edit.native.end;
    }
    at < end && !source[at..].starts_with(['\r', '\n'])
}

fn write_replacement(out: &mut Vec<u8>, edit: &Prepared<'_, '_>) {
    let slot = edit.slot;
    let indent = slot.slot.content_indent;
    let layout = &slot.source[slot.lines[0].layout()];
    let lead = layout.len() - strip_layout_ws(layout, indent).len();
    for line in physical_lines(edit.replacement) {
        let eol = line_terminator(line);
        out.extend_from_slice(&line.as_bytes()[..line.len() - eol.len()]);
        if !eol.is_empty() {
            out.extend_from_slice(edit.eol.as_bytes());
            out.extend_from_slice(&layout.as_bytes()[..lead]);
            out.resize(out.len() + indent - lead, b' ');
        }
    }
}
