# The block-text model

`lean/BlockText.lean` is a Lean 4 model of how Tine reads and writes the text
of a page. The Rust crate `crates/tine-text` follows it: `splice::apply` is
the model's `checkedWrite`, and each rule there names the Lean definition it
follows. The model is over bytes; the Rust interface takes `str`, so it
rejects a range off a character boundary, an out-of-range splice and a batch
that is not separate, and beyond that refuses exactly what the model refuses.
The tests in `crates/tine-text/tests/` check the two against each other on
generated pages and on the repository's corpus.

## What it covers

- **The bytes of a page.** A page is lexed into lines ending in LF, CRLF, a
  bare CR, or the end of the file (`lex`). An *outline* says where the
  preamble ends, and for each slot (block) its byte width, level, the width
  removed from each row and whether its header is bare. The parser supplies
  the outline and answers one question about a header line (`headReads`,
  below); the model does not parse Markdown or Org.
- **The content map.** A slot's content is a stream of cells: a text byte, or
  a newline that stands for a physical line ending plus the next row's
  removed layout (`Cell`, `Slot.gap`). A content offset maps to a byte gap of
  the file.
- **The write rule.** A batch of splices in content coordinates, all taken in
  one snapshot, is written back as bytes (`write`, `checkedWrite`):
  - a written newline takes the break style of the row it is in, and the
    slot's continuation layout;
  - a bare header (`-`, `-<tab>Title`, `-# Heading`: the marker with no
    space before the text) earns one space exactly when the slot's first
    sorted splice starts at content offset 0, the new first line is not
    empty, it differs from the old first line, and the parser does not read
    the bare layout followed by the new first line as a header of the same
    kind and level whose layout ends where the given layout ends
    (`headReads`; `-#**x` is no header, and in `* é` the header takes the
    space). An edit that changes the first line without a splice at offset 0
    (`-# H` with `x` put at offset 1) earns nothing, since no splice owns the
    gap before the kept text; whether that page reads back is the caller's
    check;
  - a retained bare CR is never followed by an LF (that would fuse the two
    into one CRLF); one CR is added before it instead;
  - exactly two inputs are refused: a replacement containing a CR, and a
    batch that leaves the last row of a slice empty where the slice does not
    end in a line break (`EmptySliceEnd`).

## What it proves

- `R0`, `R1`: reading keeps every byte; writing no edit gives the file back.
- `R3`, `R3_patchStart`, `R3_separatorPatch`: a write keeps every byte it does
  not replace, in order, and each replacement starts at its splice's mapped
  gap; the earned separator is part of the first replacement.
- `changedHead_separatorCheck`: for a changed, nonempty first line of a bare
  header edited at offset 0, the written header reads exactly when the bare
  layout or the layout with one space does; the writer asks only about the
  bare one.
- `checkedWrite_exact`, `checkedWrite_refuses`: the checked write accepts a
  batch exactly when its input is valid, and then returns `write`'s bytes.
- `checkedWrite_commute`: the order of a batch's splices changes neither the
  bytes nor the refusal.
- `repairedPage_joins`, `repairedPage_byteJoins`: no slot's output fuses with
  the bytes before it (a CR then an LF across a slot boundary).
- `readBack`, `readBack_checked`: if the parser places the written slots
  where the write planned them, re-reading gives each slot its level and
  its content with the splices applied.
- A set of kernel-checked examples (`decide +kernel`) on concrete pages. The
  Rust tests carry each one as a fixture.

## What it leaves to tests with the real parser

The model takes the outline as given, so it cannot say whether the parser
(lsdoc) will place the written slots as planned. That is the read-back
hypothesis of `readBack`, and only tests with the real parser check it. The
property tests re-read written pages with lsdoc, for edits that add no header
text, and name every kind of input where the parser places slots otherwise.
Such inputs include an edited fence, drawer or list line, a header lsdoc
finds inside a line, Org's trailing empty lines, and a bare header's first
line changed by an edit that starts after offset 0. Edits that add header
text are not covered at all.

## Building

The model builds with the toolchain in `lean/lean-toolchain`:

```
cd docs/block-text-model/lean && lake build
```

No CI job builds it.
