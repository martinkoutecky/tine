# 0031. Recursive Sheet Cell Form

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

Sheet cells can already render child outlines, and `SheetOutline` recurses back
through `SheetBlock`, so a cell can visually contain ordinary bullets and sheet
faces. The missing decision was the persisted shape: flattening a copied grid to
TSV destroys row/cell subtrees, while treating pasted outline markdown inside a
cell editor as sibling blocks can scatter `tine.view:: grid` property lines onto
row blocks.

## Decision

We will support two sheet-in-cell forms. The compact form keeps
`tine.view:: grid` on the cell itself, with its direct children as rows. The
hosted form keeps ordinary cell children, where one or more child host blocks
carry `tine.view:: grid` and own their rows. A cell can therefore contain plain
outline bullets and multiple hosted grids.

Sheet copy will keep TSV/text-html for external apps and also record an
in-memory structural payload keyed by the exact text/plain TSV fingerprint. When
the fingerprint matches on paste into a selected sheet cell, Tine parses the
structural outline and appends it as a hosted child grid. If the target cell is
still compact, Tine first moves its grid properties and row children into a first
host child, in the same undo unit. Pasting multiline text while editing a cell is
plain text insertion, never outline splitting.

## Consequences

Copy and paste can preserve nested sheet structure without introducing a second
clipboard grammar or weakening external spreadsheet interoperability. The model
also gives "add child bullet" a safe place to put plain outline children: compact
grids are auto-wrapped before ordinary bullets are appended.

Code that mutates cell children must now distinguish compact sheet cells from
hosted cells and reuse the wrapping helper rather than mixing bullets into grid
rows. Cross-process clipboard paste remains best effort because the structural
payload is intentionally in-memory; external apps still receive the TSV/html
flavors.
