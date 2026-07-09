# Sheets Phase 6 — the supertag/database layer (v2 core)

Status: **planned, not started** (Martin approved direction Jul 7 2026; nits
from his v1 testing are deliberately parked — see "Parked" below). This doc is
the self-contained plan for the next big implementation phase; the build
regime is unchanged (branch `sheets`, codex-first with tight specs,
orchestrator verifies + fixes, §3.8 atomicity, round-trip gates md+org,
screenshots + real-app e2e per chunk).

## Why this phase

Spec §3.3 is honest that v1 is "a very good editable query table," not full
supertags: zero-schema means stray `key::`s add junk columns, column order
churns, cells are untyped text, add-row seeds nothing, and tag boards
(multi-valued group-by) don't exist. This phase closes exactly that gap —
the most-requested "database on markdown" story — while staying inside the
engine we already have. Formulas (Bases DSL, §6 Tier 2) are deliberately NOT
here: separate expression engine, separate phase (7).

## Design decisions to settle FIRST (each → ADR or spec edit)

1. **`tine.fields::` grammar** (§3.3 sketch, make normative): scalar-safe
   `name=type;name=type` list; types = `text | number | date | datetime |
   checkbox | list | state | priority | enum:a,b,c | ref`. Enum values
   comma-separated inside the token (no markdown-special chars — the §3.1
   value rule applies; validate on write). Defaults ride as `name=type:def`?
   NO — keep v1 of the schema default-free except enum's implicit first value;
   defaults can come later without breaking the grammar.
2. **Where the schema lives:** on the TAG PAGE's pre-block for tag tables
   (per-tag, the spec's deliberate improvement over Obsidian's vault-global
   types.json) AND allowed on any view-owning block for children/query tables
   (same key, same grammar — one parser, two homes). Tag-table resolution
   order: view block's own `tine.fields::` wins over the tag page's.
3. **Declared vs observed columns:** declared fields first (schema order),
   observed strays after (first-seen), visually distinguished (stray header
   in italic + a "+ declare" affordance in its header menu). Strays are never
   hidden (data must stay visible — honesty rule).
4. **Tags write-back** (the risky one — currently read-only): adding a tag
   appends ` #tag` at the END of the block's first line; removing deletes
   that inline token (lsdoc-span-guided — find the Tag inline's span on the
   first line and cut it, with surrounding-whitespace normalization). NEVER
   touch tags inside code/refs (spans already exclude them). This unlocks
   multi-valued group-by. Decide + ADR before building 6c.
5. **Multi-valued group-by semantics** (§3.4 decided: Notion model): card
   appears in EVERY group it belongs to; dragging INTO a column adds that
   value, dragging OUT... Notion's drag between columns = remove source value
   + add target value. Keyboard `Ctrl+←/→` = same move semantics. `(none)`
   column = blocks with zero tags; dragging into `(none)` removes all
   group-by-field values? NO — too destructive; `(none)` is a valid drop
   target only FROM a single-valued card (then it's a plain remove). ADR.
6. **Add-row targets:** children table → new child row seeded with declared
   keys (empty values → placeholder inputs); tag/query table → new block on
   today's journal seeded `#tag ` + declared property keys with empty values
   pre-filled as `key:: ` lines? NO property lines with empty values (OG
   renders them; junk). Seed = tag + open editor + the table shows the new
   row with empty typed cells inviting clicks. Keep it minimal; revisit
   defaults later.

## Sub-phases (each = one codex dispatch + verify + commit + push)

### 6a — schema core: grammar, column ordering, typed rendering
- `tine.fields::` parse/serialize beside the other config grammar owners
  (`src/sheet/config.ts`), `SheetConfig.fields`.
- SheetTable consumes it: declared-first stable column order; observed strays
  after (italic header); `prop:` columns get their declared type.
- Typed RENDER (read side): checkbox cells render a real checkbox (readonly
  this sub-phase), number cells right-aligned, date cells as date badges,
  enum as chip, list as chips, ref as page-ref chip.
- Tag-page tables read the tag page's schema; view-block override.
- Tests: grammar (valid/malformed/scalar-safety), ordering incl. strays,
  md+org round-trip fixtures with `tine.fields::`.

### 6b — typed editing + add-row seeding
- Cell editors per type: checkbox click-toggles (`writeField` prop write
  `true`/`false`... decide the stored vocabulary: `true/false` like OG
  checkbox properties), enum → dropdown of declared values (+ "clear"),
  number → inline input with numeric validation (reject non-numbers, keep
  focus), date/datetime → existing DatePicker, list → chip editor (comma
  list), ref → autocomplete popup reusing the `[[` machinery if cheaply
  reusable, else plain text v1 (note it).
- Add-row: children table button (new row block, editor opens on title cell);
  tag table's existing journal add-row gains schema awareness (still just
  `#tag ` seed + editor; declared columns show empty typed cells for it).
- Column header menu: declare a stray (adds to `tine.fields::` with type
  text), change a declared type, remove from schema (data untouched).
  All schema writes = single property write on the schema's home block.
- Tests: each type's write-back raw assertions; stray→declared flow;
  add-row; §3.8 single-undo per gesture.

### 6c — tags write-back + multi-valued group-by (tag boards)
- `writeField(id, "tags", ...)`: add/remove ONE tag value (API takes the
  delta, not the full list): append ` #tag` to first line end; remove via the
  lsdoc Tag-inline span on the first line (one parser — spans, not regex).
  Multi-word tags → `#[[multi word]]`. Regression tests incl. tags inside
  code spans/refs staying untouched, org variant (org tags: same inline
  `#tag` form Tine already parses — verify against the org corpus first).
- Board `tine.group-by:: tags` (or `tag`? keep field id `tags`): columns =
  observed tag values (first-seen) + `(none)`; a card renders in EVERY
  matching column; drag/Ctrl+arrow between columns = remove-source+add-target
  (ADR from decision 5); same-card-in-two-columns selection semantics: a
  selected card is (col,row) — the OTHER copies render unselected (they're
  distinct grid coordinates, no change needed, but test it).
- The showcase config for the release notes: a reading-list board grouped by
  topic tags.
- Tests: grouping duplication, moves add/remove correctly (one undo), `(none)`
  rules, round-trip.

### 6d — riders: pipe-table ↔ grid conversion + CSV file-drop
- Context-menu "Convert to grid" on a rendered `md-table` block: parse the
  pipe table (the EXISTING md-table parser owns that grammar — read from the
  lsdoc AST, do not re-parse text) → children grid (header row → `tine.header::
  true`); inverse "Convert to pipe table" for grids ≤ some sane size whose
  cells are single-line (else refuse with a toast). One undo unit each;
  round-trip test both directions.
- CSV/TSV file-drop onto a page (extend `src/filedrop.ts`): creates a grid
  block via the existing `sheet/tsv.ts` parser (one grammar owner).
- Update FEATURES.md + README one-liners; onboarding demo gains a schema'd
  table example; regenerate website/demo; new screenshots per SCREENSHOTS.md.

## Verification gates (unchanged regime, every sub-phase)
`npm test` both configs + `npx tsc` + `cargo test -p tine-core`; md+org
byte-exact round-trip fixtures for every new writer; screenshot self-verify;
extend `scripts/e2e-sheets.mjs` (6b: checkbox+enum write; 6c: tag-board move)
and keep ALL PASS; progress doc updated; commit+push per sub-phase.

## Parked (explicitly NOT this phase)
- **Martin's v1 UX nits** — he has a list; capture it into `docs/BACKLOG.md`
  (or a NITS section here) when he dumps it, then batch-fix in a dedicated
  polish pass. Don't interleave them into 6a–6d.
- Formulas (Phase 7, Bases DSL per §6). Merged cells `span::`. Split view,
  canvas face, whiteboards importer (v3). Pointer cell/card drag-reorder.
  In-column card ordering. Facet indices (measured fine through 200k blocks).
