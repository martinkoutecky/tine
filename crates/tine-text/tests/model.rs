//! The block-text model's write rule (`docs/block-text-model/lean/BlockText.lean`),
//! transcribed definition by definition onto this crate's reader: a slot is
//! its rows, its content a stream of cells, and a write the planned rows of
//! every edited slot. It works on cells, as the model does; `apply` works on
//! bytes. The property tests compare the two, so each name here is the
//! model's.

use std::ops::Range;
use tine_text::{takes_separator, Break, Owner, PageText, Splice};

/// `Cell`: a content byte, or a line separator with the break before it and
/// the next row's layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Text(u8),
    Newline(Break, Vec<u8>),
}

impl Cell {
    fn raw(&self) -> Vec<u8> {
        match self {
            Cell::Text(b) => vec![*b],
            Cell::Newline(e, indent) => [e.as_str().as_bytes(), indent].concat(),
        }
    }
}

/// `Piece`: one physical row of a slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub layout: Vec<u8>,
    pub text: Vec<u8>,
    pub end: Break,
}

impl Row {
    pub fn bytes(&self) -> Vec<u8> {
        [&self.layout[..], &self.text, self.end.as_str().as_bytes()].concat()
    }
    /// `Piece.line`: the body and the break.
    pub fn line(&self) -> (Vec<u8>, Break) {
        ([&self.layout[..], &self.text].concat(), self.end)
    }
}

/// An owner's content rows, as the reader gives them.
pub fn rows(page: &PageText<'_>, owner: Owner) -> Vec<Row> {
    let text = page.text().as_bytes();
    page.pieces(owner)
        .map(|p| Row {
            layout: text[p.layout].to_vec(),
            text: text[p.text].to_vec(),
            end: p.kind,
        })
        .collect()
}

/// An owner's trailer rows.
pub fn trailer(page: &PageText<'_>, owner: Owner) -> Vec<Row> {
    let text = page.text().as_bytes();
    page.trailer(owner)
        .map(|p| Row {
            layout: Vec::new(),
            text: text[p.text].to_vec(),
            end: p.kind,
        })
        .collect()
}

/// `rowCells`.
fn row_cells(rows: &[Row]) -> Vec<Cell> {
    let mut cells = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            cells.push(Cell::Newline(rows[i - 1].end, row.layout.clone()));
        }
        cells.extend(row.text.iter().map(|b| Cell::Text(*b)));
    }
    cells
}

/// `finalEnding`; the end of a slice (`Continues`) is the model's `eof`.
fn final_ending(rows: &[Row]) -> Break {
    rows.last().map_or(Break::Eof, |r| r.end)
}

/// `followingEnding`.
fn following_ending(fallback: Break, cells: &[Cell]) -> Break {
    cells
        .iter()
        .find_map(|c| match c {
            Cell::Newline(e, _) => Some(*e),
            Cell::Text(_) => None,
        })
        .unwrap_or(fallback)
}

/// `insertedEnding`.
fn inserted_ending(e: Break) -> Break {
    match e {
        Break::CrLf => Break::CrLf,
        _ => Break::Lf,
    }
}

/// `encode`.
fn encode(rows: &[Row], continuation: &[u8], edit: &Splice<'_>) -> Vec<Cell> {
    let source = row_cells(rows);
    let style = following_ending(
        final_ending(rows),
        &source[edit.range.start.min(source.len())..],
    );
    edit.text
        .bytes()
        .map(|b| {
            if b == b'\n' {
                Cell::Newline(inserted_ending(style), continuation.to_vec())
            } else {
                Cell::Text(b)
            }
        })
        .collect()
}

/// `ordered`: a stable sort by `keyLE`.
fn ordered<'a, 's>(edits: &[&'a Splice<'s>]) -> Vec<&'a Splice<'s>> {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    edits
}

/// `firstTextByte`.
fn first_text_byte(cells: &[Cell]) -> Option<u8> {
    match cells.first() {
        Some(Cell::Text(b)) => Some(*b),
        _ => None,
    }
}

/// `separator`, with the parser's predicate.
fn separator(bare: bool, rows: &[Row], stream: &[Cell]) -> Vec<u8> {
    match first_text_byte(stream) {
        Some(b) if bare && first_text_byte(&row_cells(rows)) != Some(b) && !takes_separator(b) => {
            vec![b' ']
        }
        _ => Vec::new(),
    }
}

/// `repairEnding`.
fn repair_ending(after_cr: bool, e: Break) -> Break {
    if after_cr && e == Break::Lf {
        Break::CrLf
    } else {
        e
    }
}

/// `Cell.afterCR`.
fn after_cr(cell: &Cell) -> bool {
    matches!(cell, Cell::Newline(Break::Cr, indent) if indent.is_empty())
}

/// `repairCells`.
fn repair_cells(mut prior: bool, stream: &[Cell]) -> Vec<Cell> {
    stream
        .iter()
        .map(|cell| {
            let repaired = match cell {
                Cell::Text(b) => Cell::Text(*b),
                Cell::Newline(e, indent) => Cell::Newline(repair_ending(prior, *e), indent.clone()),
            };
            prior = after_cr(cell);
            repaired
        })
        .collect()
}

/// `exitCR`.
fn exit_cr(prior: bool, stream: &[Cell]) -> bool {
    stream.last().map_or(prior, after_cr)
}

/// `assembleRows`.
fn assemble_rows(layout: Vec<u8>, ending: Break, stream: &[Cell]) -> Vec<Row> {
    let mut rows = vec![Row {
        layout,
        text: Vec::new(),
        end: ending,
    }];
    for cell in stream {
        match cell {
            Cell::Text(b) => rows.last_mut().unwrap().text.push(*b),
            Cell::Newline(e, indent) => {
                rows.last_mut().unwrap().end = *e;
                rows.push(Row {
                    layout: indent.clone(),
                    text: Vec::new(),
                    end: ending,
                });
            }
        }
    }
    rows
}

/// One slot's `plannedSlot`, with the byte patch of each sorted splice
/// (`bytePatches` of `framedChunks`, the separator folded into the first as
/// `R3_separatorPatch` states).
pub struct SlotPlan {
    pub rows: Vec<Row>,
    /// The splices in the model's order, each with its file range and the
    /// bytes written in place of it.
    pub patches: Vec<(Range<usize>, Vec<u8>)>,
    pub separator: bool,
}

/// `plannedSlot` of slot `n` for `edits` (in bounds, pairwise separate),
/// after a slice that ends in a bare CR when `prior_cr`.
pub fn planned_slot(
    page: &PageText<'_>,
    n: usize,
    edits: &[&Splice<'_>],
    prior_cr: bool,
) -> SlotPlan {
    let old = rows(page, Owner::Slot(n));
    if edits.is_empty() {
        return SlotPlan {
            rows: old,
            patches: Vec::new(),
            separator: false,
        };
    }
    let continuation = page.continuation(n).into_bytes();
    let source = row_cells(&old);
    let map = page.content_map(Owner::Slot(n));
    // `partition`: kept runs and replacements, in order.
    let mut stream = Vec::new();
    let mut chunks = Vec::new();
    let mut cursor = 0;
    for edit in ordered(edits) {
        stream.extend_from_slice(&source[cursor..edit.range.start]);
        let put = encode(&old, &continuation, edit);
        let file = map.to_file(edit.range.start)..map.to_file(edit.range.end);
        chunks.push((stream.len(), put.len(), file));
        stream.extend(put);
        cursor = edit.range.end;
    }
    let terminal = stream.len();
    stream.extend_from_slice(&source[cursor..]);
    let space = separator(page.slots()[n].bare(), &old, &stream);
    let layout = [&old[0].layout[..], &space].concat();
    let adjacent = prior_cr && layout.is_empty();
    let repaired = repair_cells(adjacent, &stream);
    let end = repair_ending(exit_cr(adjacent, &stream), final_ending(&old));
    // Each patch: its repaired replacement, then the CR that repairs the
    // first cell of the kept run after it (the final break for the last).
    let mut patches = Vec::new();
    for (i, (at, len, file)) in chunks.iter().enumerate() {
        let mut bytes: Vec<u8> = repaired[*at..at + len].iter().flat_map(Cell::raw).collect();
        let next = at + len;
        let kept_next = chunks.get(i + 1).map_or(terminal, |c| c.0) > next || i + 1 == chunks.len();
        let changed = if next < stream.len() {
            repaired[next] != stream[next]
        } else {
            end != final_ending(&old)
        };
        if kept_next && changed {
            bytes.push(b'\r');
        }
        patches.push((file.clone(), bytes));
    }
    if !space.is_empty() {
        patches[0].1.insert(0, b' ');
    }
    SlotPlan {
        rows: assemble_rows(layout, end, &repaired),
        patches,
        separator: !space.is_empty(),
    }
}

/// `linesEndInCR` of rows and trailer.
fn ends_in_cr(rows: &[Row], trailer: &[Row]) -> bool {
    trailer
        .last()
        .or(rows.last())
        .is_some_and(|r| r.end == Break::Cr)
}

/// `lexicalCheck`: lexing the slice's bytes gives its rows back (the end of
/// a slice cut mid-line lexes as the end of the file).
pub fn lexical_check(rows: &[Row]) -> bool {
    let bytes: Vec<u8> = rows.iter().flat_map(Row::bytes).collect();
    let text = String::from_utf8_lossy(&bytes);
    let lexed: Vec<(Vec<u8>, Break)> = tine_text::lines(&text)
        .map(|(body, e)| (text.as_bytes()[body].to_vec(), e))
        .collect();
    let planned: Vec<(Vec<u8>, Break)> = rows
        .iter()
        .map(|r| {
            let (body, e) = r.line();
            (body, if e == Break::Continues { Break::Eof } else { e })
        })
        .collect();
    lexed == planned
}

/// The whole page as `planPage` writes it: every owner's planned rows and
/// trailer, and each edited slot's plan.
pub struct PagePlan {
    pub owners: Vec<Vec<Row>>,
    pub slots: Vec<SlotPlan>,
}

impl PagePlan {
    pub fn bytes(&self) -> Vec<u8> {
        self.owners.iter().flatten().flat_map(Row::bytes).collect()
    }
}

/// `planPage`, for a batch in bounds and pairwise separate.
pub fn plan_page(page: &PageText<'_>, splices: &[Splice<'_>]) -> PagePlan {
    let mut owners = vec![[rows(page, Owner::Preamble), trailer(page, Owner::Preamble)].concat()];
    let mut prior = ends_in_cr(&owners[0], &[]);
    let mut slots = Vec::new();
    for n in 0..page.slots().len() {
        let edits: Vec<&Splice<'_>> = splices.iter().filter(|s| s.slot == n).collect();
        let plan = planned_slot(page, n, &edits, prior);
        let tail = trailer(page, Owner::Slot(n));
        prior = ends_in_cr(&rows(page, Owner::Slot(n)), &tail);
        owners.push([plan.rows.clone(), tail].concat());
        slots.push(plan);
    }
    PagePlan { owners, slots }
}

/// `checkedWrite`: the planned bytes when every replacement is CR-free and
/// every slot's planned rows pass `lexicalCheck`; `None` otherwise.
pub fn checked_write(page: &PageText<'_>, splices: &[Splice<'_>]) -> Option<Vec<u8>> {
    let plan = plan_page(page, splices);
    let cr_free = splices.iter().all(|s| !s.text.contains('\r'));
    (cr_free && plan.owners[1..].iter().all(|rows| lexical_check(rows))).then(|| plan.bytes())
}
