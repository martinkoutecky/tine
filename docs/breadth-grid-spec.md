# Breadth: Grid, Supertag Tables & Splits — implementation spec

Status: **design-complete, not started.** This is the decided plan for Tine's
first deliberately *beyond-OG* feature. Rationale and alternatives live in the
brainstorm notes (don't re-derive — read these if a decision seems arbitrary):

- `subagent-tasks/notes/brainstorm-treesheets.md` — data model + verb set
- `subagent-tasks/notes/brainstorm-treesheets-flow.md` — interaction flow (edges, modality, zoom)
- `subagent-tasks/notes/brainstorm-supertags.md` — Tana / Logseq-DB supertags, the markdown-emulation surface
- `subagent-tasks/notes/brainstorm-obsidian-splits.md` — split view / vertical-tabs
- `subagent-tasks/notes/brainstorm-tine-seams.md` — codebase seams (file:line)
- `subagent-tasks/notes/brainstorm-split-libraries.md` — library survey (build-bespoke verdict)
- `subagent-tasks/notes/brainstorm-obsidian-bases.md` — Obsidian Bases study (prerequisite for v2 formulas / `tine.fields::` schema)

Audience: a fresh implementing session. Read §1–§4 + §10–§13 before writing code.

---

## 1. Goal

Add **breadth** — a second spatial dimension — to Tine's 1-D outline. The whole
feature is **one recursive layout engine** that renders a node's children in a
chosen *geometry*. The user-visible payoffs are all configurations of that one
engine:

- a **TreeSheets-style grid** (free-form, recursive cells);
- a **supertag-style table** (all `#tag` instances as an editable, property-keyed
  table — the most-requested feature, scratched on markdown);
- a **board** (kanban: children grouped by a property into columns);
- later, **split view** (panes are the same engine with a different leaf kind).

**Hard invariant: everything round-trips to plain Logseq markdown.** Geometry is
metadata; content stays a tree. OG (the file version) must render any grid as an
ordinary nested outline with harmless properties. No sidecar files, no
coordinates. This is the whole point — Tine occupies the niche neither OG-file
(read-only query tables) nor Logseq-DB (SQLite, breaks markdown) can.

**Positioning vs Obsidian Bases** (the closest living competitor — study:
`notes/brainstorm-obsidian-bases.md`): ***Bases is a database of your files;
Tine's grid is a database of your bullets.*** Bases rows are whole notes — by
founder-confirmed architecture it reads only cached frontmatter, never file
bodies — so its community's #1 ask (task/block rows) is impossible there and gets
worked around with one-file-per-task plugins; board/kanban was still not native
14 months after launch. Tine's row *is* a block, columns come free from native
facets (zero YAML authoring), and the v1 showcase is exactly what Bases can't do:
a task kanban whose card-move flips the marker. Concede honestly: Bases wins at
file-level reference libraries (books/covers/galleries) and formula maturity.

---

## 2. The engine (one primitive, four parameters)

Tine's live store is a **geometry-agnostic normalized tree** (`src/store.ts:41-82`):
a flat `byId` id→Node map + ordered `children: string[]`. Components read blocks
purely by id, so the store imposes no geometry; today `src/components/Block.tsx`
is the *only* code that imposes the vertical outline. The engine is a new view
component over that same store.

Model it as **a recursive, resizable, 2-D layout of content slots**, parameterized:

| parameter | values | v1? | meaning |
|---|---|---|---|
| `rowSource` | `children` \| `query` | both | rows are a node's direct children, or a query's results (graph-wide) |
| `columnModel` | `positional` \| `field-keyed` | both | columns by sibling order, or by *field* (see below) |
| `leafKind` | `block` \| `pane` | `block` only | a cell hosts a block editor, or a navigable pane (split view) |
| `trackModel` | `content` \| `manual` | `content` default | track sizes auto-fit content, or are dragged |

**Fields — do NOT assume users write `property::` (critical correction).** Most
file-version Logseq users (Martin included) almost never author explicit
`key:: value` properties, and OG/Tine give them no pleasant insert/display UI. So
a **field** is any extractor `(block) → value`, and columns/grouping key on
fields, of which an explicit property is only *one kind*. The others are the
block's **native Logseq attributes**, all of which Tine already parses into its
**facets** layer (`src/render/facets.ts`, `deriveFacets` :111-125):

- **task state** — the TODO/DOING/DONE/… marker (`src/markers.ts:13-25`; facet at `facets.ts:111-122`)
- **priority** — `[#A/B/C]` (`facets.ts:115-120`)
- **scheduled / deadline** — dates (`readSchedule` `store.ts:1260-1276`; facet `facets.ts:164-176`)
- **tags** — `#tag` / `[[link]]` (lsdoc AST `src/render/ast.ts:60-65`)
- **page / created / updated** — block metadata
- **explicit `property::`** — one field kind, opt-in (`isPropertyLine` `block.ts:31-38`)

Write-back is per field kind: a **status** cell reuses `cycleMarker`
(`src/editor/marker.ts:39-55`); a **scheduled** cell reuses the planning
normalizer (`src/editor/planning.ts:22-65`); a **property** cell rewrites the
`key:: value` line. This is what lets the table/board work on how people
*actually* structure data (tasks, tags, dates) without requiring properties.

Plus two cross-cutting mechanisms shared by all configs: the **logical grid
matrix** (handles spans; see §3) and the **edge/selection interaction model** (§4).

**v1 builds the engine with `leafKind: block` only**, but designs the seams
(`leafKind`, `trackModel`) so split view (§10 v3) drops in without a rewrite.
Configs that ship in v1:

- `children` + `positional` → **TreeSheets grid** (free-form data)
- `children`/`query` + `field-keyed` → **record / supertag / task table** (columns = fields)
- either + `group-by: <field>` → **board / kanban** — e.g. **`query(tasks)` grouped
  by task state** = a kanban of scattered TODOs with **zero properties** (the
  **showcase config** — headline demo and user-requested, but *not* the build
  priority; it falls out of the architecture. Card-move = flip the marker via
  `cycleMarker`)

**Engine honesty — where the unification is real, and where it isn't.** The truly
shared core is: the **Field extractors**, the **selection/seam model**, **track
sizing**, and the **logical matrix** (grid/table faces only). The **board is NOT a
matrix** — it's grouped stacks of unequal length, with no row concept, no
seams-as-cells, no spreadsheet keyboard; it shares Fields, group-by, and
pointer-drag, and should be built as its own renderer over those shared modules,
not forced through the matrix. **Split view** (v3) shares the seam gesture and
track sizing, but its hard parts (per-pane routing, tabs, focus chrome) live
*outside* the engine. Treat "one engine" as a design lens, not an implementation
directive: implement shared modules only where the sharing is real — an
over-abstracted framework would make each of the four faces harder than building
it concretely.

**Build bespoke.** Per the library survey: depend on nothing new. CSS Grid
(`grid-template-*` with `fr` tracks + a drag handle) is the track-layout
substrate; generalize Tine's existing ~12-line pointer resizers (sidebar, PDF
pane in `src/App.tsx`; `installPaneTracker` in `src/ui.ts`). All split/dock/grid
libraries own their own layout tree and would fragment this engine — reference
only (study react-mosaic's split-tree model, dockview's drag-to-edge, TreeSheets
for the recursive grid).

---

## 3. Data model & round-trip encoding (decide property names BEFORE coding)

Geometry lives in **`property::` lines on the parent + the outline structure
itself.** OG renders these as harmless properties (Tine already hides selected
props, `Block.tsx:92-98`). The on-disk format expresses only a tree
(`crates/tine-core/src/doc.rs:434-453`); we never exceed that.

### 3.1 Config property names (DECIDED — OG-safe, from the OG-source study)
See `subagent-tasks/notes/brainstorm-og-property-safety.md`. All grid config lives
in **`tine.`-namespaced properties** (mirrors OG's own `logseq.table.*`
convention; keeps the auto-created property-pages tidy and signals "Tine-managed").
Verified collision-free against OG's reserved registry
(`deps/graph-parser/src/logseq/graph_parser/property.cljs`); **avoid** the
read-only `query-table` / `query-properties` / `query-sort-*` family and
`logseq.table.*` / `logseq.color`.

- **`tine.view::`** = `list` (default, == today's outline) | `table` | `grid` | `board` — the face selector.
- **`tine.group-by::`** = a **field id** (`state` | `priority` | `scheduled` |
  `tag` | `prop:<key>`) — the board's grouping axis (NOT only a property key).
- **`tine.span::`** = `RxC` (e.g. `2x3`) on a merged cell's anchor (v2).
- **`tine.col-aggregates::`** / **`tine.col-widths::`** = token-free `key=value;…`
  lists. **Keys are field ids for field-keyed tables** (stable under column
  insert/reorder, e.g. `prop:qty=sum;state=count`); **index-keyed only for
  positional grids** (e.g. `0=120;1=200`) — and any column insert/delete rewrites
  this config **in the same undo unit** (§3.8). **Config-write hygiene** (Bases'
  git-churn mistake, inverted): `tine.col-widths::` is written only on an explicit
  user resize — never as a side effect of rendering or sorting.
- **`tine.header::`** = optional, marks a header row.
- **Where config lives:** on the block that *owns the view* — the parent block for
  `rowSource: children`; **the `{{query}}` block itself** for `rowSource: query`.
  So a query block with `tine.view:: board` renders as a normal query list in OG
  and as a kanban in Tine — same block, no new file, graceful degradation.

**Normative value rule (critical safety finding):** OG parses every property
*value* as inline markdown, so config values must be **plain scalars** — never
contain `[[ ]]`, `(( ))`, `{{ }}`, or `#`, or OG turns them into page/block refs,
tags, or macro calls. This rules out any JSON-ish blob value; use scalars and
`=`/`;`-delimited lists only. (This applies to the `tine.*` *config* values; cell
*content* is real markdown and may contain refs.) Each `tine.*` key auto-creates
one harmless property-page in OG — expected; the namespace keeps them grouped.

Throughout the rest of this doc, bare `view::` / `group-by::` / `span::` refer to
these `tine.`-prefixed names.

**Org coverage (decided Jul 6 2026 — Sheets is format-agnostic, not md-only).**
Both carriers of grid state exist in org: geometry = headline nesting (org's
block tree), config = the block's `:PROPERTIES:` drawer (`:tine.view: grid`
instead of `tine.view:: grid`). Verified against the parsers: lsdoc's org
drawer keys reject only `:`/space/newline (`block_common.rs` `drawer_property`,
matching mldoc `drawer.ml`), so dotted `tine.*` keys are legal; `tine-core`
reads org drawer properties through the same one-recognizer path as md
(`doc.rs:99`); facets derive format-aware (`facetsOf(raw, format)`), so the
engine reads fields identically on both. Empty cell = empty bullet in md
(canonical form: bare `-`, NO trailing space) / bare `**` headline in org (both
corpus-validated). The write side inherits org's existing gate: pages that fail
the org round-trip self-check are read-only in Tine, so grid *editing* is
gated exactly like block editing — no new rule. Every phase's round-trip
fixtures must include an org variant. Emacs org-mode users get bonus graceful
degradation: property drawers fold away by default. Sample pages for both
formats: `tine-test/pages/Sheets demo.md`, `org-graph/pages/Sheets demo.org`.

### 3.2 Positional grid (`children` + `positional`)
Two outline levels carry the 2-D geometry:
- the grid node's **children = rows**;
- each row's **children = its cells** (columns, by sibling order).
- **Empty cell = an empty child bullet.** Trailing holes cost nothing (short
  row); an interior hole is a real empty bullet, auto-created when typed into.
  **Never auto-prune empty cells** (consistent with the carry-spacer rule).
- No coordinates anywhere — geometry is the tree shape. Round-trips to a plain
  2-deep nested outline.

### 3.3 Record / field table (`field-keyed`)
- **Columns = fields** (§2 Fields), not just properties. A column can be the task
  **state**, **priority**, **scheduled/deadline**, **tags**, **page/metadata**, or
  an explicit **`property::`** — read from the facets layer, not invented.
  Zero-schema in v1: the column set is the union of fields present across the rows
  (generalize the query-table's column logic `Macro.tsx:178-182` from
  props-only to fields; extend its `Row` to carry facets, `Macro.tsx:166-175`).
- Rows = the node's children (`rowSource: children`) **or** query results
  (`rowSource: query`) — e.g. `query(task)` for a task table, or `tag = Person`
  for the supertag table.
- **Empty cell = absent field value.** Editing writes back per field kind
  (§2): status → `cycleMarker`, scheduled → planning normalizer, property →
  `key:: value` line. Nothing empty is stored; properties stay opt-in.
- Cell edits go through the existing multi-surface machinery (the cell is another
  surface onto that block — `SurfaceContext`, `Block.tsx:190`).
- **The grid is the property UI OG lacks:** for people who *do* want properties, a
  property-keyed cell is a pleasant `key:: value` editor — but nothing requires it.
- **Property write-back placement matters:** OG expects `key:: value` lines
  immediately after the block's first line (before body text). Inserting a
  property from a cell must respect that ordering — same class of care as the
  SCHEDULED/DEADLINE repositioning issue (see `tine-scheduled-deadline-reposition`
  memory). Don't append to the end of `raw`.

**Beyond v1 — the supertag ladder (what Tana/Logseq-DB users will still miss, and
how to close it).** Be honest about v1: zero-schema means any stray `key::` on one
block adds a junk column to the whole table, and the column *order* can churn as
rows change. v1 is "a very good editable query table," not full supertags. The
ladder up:
1. **v2 — `tine.fields::` schema on the tag page**: a declared field list with
   light type hints, scalar-safe per §3.1 (e.g.
   `birthday=date;qty=number;done=checkbox;status=enum:todo,doing,done;owner=ref`).
   Buys: **stable column set + order** (declared fields first, observed strays
   after), **typed cell inputs** (date cells reuse the planning-style picker;
   enum → dropdown; checkbox), and **add-row seeding** (new instance pre-filled
   with the declared keys / defaults) — Tana's tag-template feel. Type tokens
   mirror Obsidian's six property types (`text|number|date|datetime|checkbox|list`)
   plus Tine-native `state|priority` and `enum|ref`; the schema stays
   **per-tag-page** (deliberately better than Obsidian's vault-global
   `types.json`); the UI distinguishes declared vs inferred columns; add-row also
   stamps filter-implied fields (Bases' New-button behavior).
2. **v3 — relations**: ref-valued fields (`owner:: [[Alice]]`) + **lookup columns**
   (pull `[[Alice]]`'s `phone::` — already in §10 deferred). The top forum wish.
3. **Skip**: inheritance/extends (low value on markdown).
Prerequisite before designing `tine.fields::` syntax or v2 formulas: the
**Obsidian Bases study** (`notes/brainstorm-obsidian-bases.md`) — Bases is the
closest living "database on markdown" and its formula/config DSL is battle-tested.

### 3.4 Board / kanban (`group-by: <field>`)
- A board is a non-destructive **view**: group the rows by a **field** value
  (§2 — task state, priority, a tag, a property, …) and render groups as columns,
  cards = rows. Tree unchanged → round-trips trivially.
- **Showcase: `rowSource: query(tasks)` + `group-by: state`** → a kanban of TODOs
  scattered across the graph, columns = TODO / DOING / DONE. **Needs no
  properties.** Columns come from the group-by field's value set (for `state`, the
  marker enum — configurable subset). Column headers show **card counts**.
- **Keyboard card-move:** with a card selected, `Ctrl+←/→` moves it to the
  adjacent column — the same group-by field write as a drag (for `state`, a
  marker flip). Trivial given `cycleMarker`; ship it with the board.
- **Card-move semantics = write the group-by field on that one block**, a *local*
  edit (no reparent, no cross-file move — so it works on scattered query rows):
  dragging a card to another column rewrites its **state** (via `cycleMarker` /
  set-marker), or its property value, etc. Resolves §13.3.
- To make a grouping *real structure* (children source only), use **Hierarchify**
  (§5); for `rowSource: query` the grouping stays a view (can't relocate scattered
  blocks), which is exactly right for a task kanban.

**Grouping cardinality (a block can hold many fields — which column?).** A board
groups on **one** field; the block's *other* fields just ride along on the **card
face** as chips (priority, date, tags) — so multi-field blocks are an asset, not a
confusion. The only real question is whether the *grouping field itself* is
multi-valued:
- **v1: single-valued group-by only.** Offer state / priority / single-value
  property / date-bucket as grouping axes — all single-valued by nature (a task is
  TODO xor DOING xor DONE), so the ambiguity cannot arise and the flagship is fully
  covered. Blocks with no value → a **`(none)`** column.
- **Multi-valued group-by (e.g. by `tags`) is v2** (§10): adopt the Notion model —
  the card appears in **every** group it belongs to, and dragging in/out
  adds/removes that value on the block. (Airtable's alternative is to forbid
  multi-value grouping entirely; we defer rather than forbid.)

### 3.5 Merged cells — v2, but reserve the encoding now
- Spans are the one bit of 2-D geometry the tree can't carry → `span:: RxC`
  (rows×cols) on the **anchor** cell bullet; covered slots are simply **absent**
  (anchor-only). The renderer builds a **logical grid matrix** from
  rows×cells×spans and emits native HTML `rowspan`/`colspan`.
- **Build the logical-matrix pass in v1** even though every span is `1×1`, so v2
  merges drop in with no renderer rewrite, and so the matrix is available for 2-D
  keyboard navigation. Sort/insert-through-a-merge follows spreadsheet-standard
  rules (block, or auto-unmerge) — v2.

### 3.6 Column config
Per-column settings use the decided properties (§3.1): `tine.col-aggregates::`
(e.g. `0=sum;2=avg`), `tine.col-widths::` (e.g. `0=120;1=200`), and the board's
`tine.group-by::`. All values are scalar / `=;`-delimited per the §3.1 value rule
(no markdown-special chars). Derived values (aggregates, future formula results)
are **never written into the tree** — recompute on load to avoid staleness from
external edits.

### 3.7 Round-trip acceptance invariant
Any grid must serialize to a nested outline that (a) OG renders without error as
plain bullets + properties, and (b) re-parses in Tine to the identical structure.
This is the gate for every phase (§12).

### 3.8 Mutation safety, atomicity & undo (normative — data safety is Tine's main worry)
Grid mutations introduce new multi-block write paths on Martin's REAL graph; treat
them with the same severity as the rename-transaction problem (`DEFERRED.md`
rename/B, `high`).

- **Every grid mutation is ONE atomic undo unit**: TSV/CSV paste, fill, range
  ops, column insert/delete (*including* the §3.1 config rewrite), Hierarchify /
  Flatten, card-move. No mutation may leave the store in a state that one `undo`
  cannot fully revert.
- **Hierarchify ships only together with Flatten**, tested as an inverse pair
  (Hierarchify → Flatten = identity, modulo the property written back).
- **Multi-file structural write-backs are v1-EXCLUDED.** Column add/rename/delete
  on a `rowSource: query` table would rewrite every instance block across many
  files — the exact transactionless multi-file class DEFERRED.md flags. Until Tine
  has graph-write transactions, column-*structure* ops are enabled only when all
  rows live in one page (children source). Single-**cell** edits on query rows
  remain allowed (single-block writes on the existing per-page save path with its
  conflict baseline / `baseRev`).
- **A failed multi-block op rolls back the in-memory store** to its pre-op state —
  no half-applied grids, ever.

---

## 4. Interaction model

**A grid is a block whose *edit-interior* is 2-D.** Tine already has the two modes
this needs — block-**selection** (multi-select, super-↑/↓ move, indent) and
**edit** (`Enter` drops the caret in). The grid does **not** add a new modality;
it (1) extends selection mode from 1-D to 2-D, and (2) adds the **seam** as a new
selectable target. The select→edit ladder is reused, with one 2-D rung inserted:

```
outline selection  --Enter/→-->  grid cell-selection (2-D)  --Enter-->  text-edit
       (1-D)         <--Esc/←--          + seams            <--Esc--      (caret)
```

### 4.1 Modes (scoped to inside a grid)
- **Selection mode:** arrows move the cell selection in 2-D (and step onto seams,
  see 4.2); printable keys start editing; `Ctrl+arrow` **moves content** (the 2-D
  generalization of Tine's existing super-↑/↓ block reorder); `Enter` toggles into
  edit; `Esc`/`←`-past-left ascends/exits.
- **Edit mode:** arrows = text caret; `Esc` exits to selection.
- A grid is **opaque to outline-level selection** (select / super-move / indent it
  as one unit). `Enter` or `→` enters it; entering lands on the first cell in
  selection mode (not editing).

### 4.2 The edge / seam (the create primitive)
- A seam is a first-class selection target: a cell-selection with one dimension
  collapsed to zero (model after TreeSheets' `Thin()`). Reachable by **keyboard
  arrow-stepping, default ON** (cell → edge → next cell), and by pointer hover/click
  within a few px of a ruling line. (Default-ON matches current TreeSheets behavior
  — Martin verified empirically Jul 3 2026; an earlier code-read calling it opt-in
  was wrong or stale. Keep a setting to disable if the 2×-stops cost annoys.)
- **Type on a seam → insert a row/column there and enter edit** (materialize-then-act).
- **Backspace/Delete on a seam → delete the adjacent row/col** (before/after).
- **Seam meaning forks by config:** *positional* → insert a child bullet at that
  position; *field-keyed record table* → add a field/column (a new `property::` for
  property columns; virtual fields like state aren't user-inserted this way);
  *board* → columns are the group-by field's values, so "add column" = introduce a
  new value (e.g. a new status) rather than a free column. Row-insert is uniform (a
  new child / instance / card).

### 4.3 Keyboard map (FULL spreadsheet keyboard — v1, not deferred)
The keyboard *is* the feature (the reason TreeSheets makes complex sheets easy to
build); ship the complete model in v1.
- **Navigate:** `arrows` move selection 2-D (select mode) / caret (edit mode);
  `Tab`/`Shift+Tab` next/prev cell (overtype-ready); `Enter`/`→` descend,
  `Esc`/`←` ascend; **flow-out at grid top/bottom borders into the outline (NOT
  wrap)** — the one deliberate divergence from TreeSheets.
- **Edit:** type to edit; `Enter` toggle/commit; `F2` edit-at-end; `Alt+Enter`
  commit + first cell of new row; `Ctrl+Enter` commit + next cell right.
- **Range-select:** `Shift+arrow` extends a 2-D cell rectangle; full-row /
  full-column extend bindings; select-all-in-grid; ranges cross hierarchy
  boundaries by auto-selecting the whole child (TreeSheets `Merge` rule).
- **Move content:** `Ctrl+arrow` reorders the selected cell(s)/row/col, 2-D (the
  generalization of Tine's existing super-↑/↓ block reorder).
- **Seams:** type on a seam inserts a row/col + enters edit; `Backspace`/`Delete`
  on a seam deletes the adjacent row/col (before/after); keyboard seam-stepping
  default ON.
- **Fill:** `Ctrl+D` / `Ctrl+R` fill-down / fill-right across a range.
- **Clipboard interop:** copy/cut a range → **TSV** (pastes into real
  spreadsheets); paste **TSV/CSV** into a range fills/creates cells; paste
  **indented text** builds nested structure. Operates on the logical matrix; the
  write-back differs by `columnModel` (positional → cell bullets; property-keyed →
  `property::` values).

### 4.4 Cells: no persistent bullet
Drop the bullet dot inside cells. Affordances via: right-click the cell body →
context menu (switch view, color, …); a **hover-reveal handle** (drag grip +
expand chevron, Notion/Craft pattern) for drag + menu; keyboard for the rest.
Nested content inside a cell renders as indented lines (tree-line connectors are
Tier C), no bullets, hover-handles per item.

### 4.5 Recursion
A cell **is a node**; how its children display (list vs sub-grid) is the cell's
own `view::` mode. So "right-click a cell → show children as grid" gives a
table-within-table, recursively — for free. "Zoom into a cell" reuses the existing
routable block-zoom verb (`zoomInto`, `src/ui.ts:619-628`, `router.ts:206-216`).

### 4.6 Drag = pointer events, never HTML5 DnD
WebKitGTK makes native HTML5 DnD unreliable (`Block.tsx:107`; `TabBar.tsx`
workaround). **All new layout drag — seam drag, cell reorder, future tab-to-split
— must be pointer-event based**, generalizing `beginDrag` (`Block.tsx:122`).

---

## 5. Hierarchify / Flatten (v1 core — the bridge between faces)

The three faces are views over one tree; Hierarchify/Flatten are the verbs that
**commit a grouping into the tree (or undo it)** — this is what makes the faces
feel like one fluid tool, and lets the user slide between grouping keys live.

- **Hierarchify by K:** group children that share the same value of **field K**,
  insert a new parent bullet per value (labelled by the value), reparent the
  children under it. Implementation = create-parent + `moveBlock` (mutations exist
  in `store.ts`). **v1 scope: field-keyed children tables only** (group by a
  field). The positional-grid variant (group rows by the values in a chosen
  column, TreeSheets-style) is **v2**.
- **Flatten:** remove a grouping level — pull grandchildren up to children,
  optionally writing the parent's label back as a property. The inverse.
- **Constraint:** structural Hierarchify is only valid for `rowSource: children`
  (local reparent within a page). For `rowSource: query` (scattered instances
  across files), **group-by stays view-only** — never relocate blocks across files.
- Mental model: *face = ephemeral lens (preview a board); Hierarchify = bake it in.*
- `Transpose` and `Hierarchy-Swap` (pivot = Flatten + Hierarchify by another key)
  are v2.

---

## 6. Arithmetic — v1 = column aggregates; v2 = formula columns

- **v1 (Tier 1): per-column footer aggregate** — adopt Bases' built-in summary
  set and naming (numbers: Sum / Average / Median / Min / Max / Range / Stddev;
  dates: Earliest / Latest / Range; any type: Empty / Filled / Unique; checkbox:
  Checked / Unchecked), chosen from a per-column dropdown. Non-modal, always live,
  **derived (never stored)**; only the chosen aggregate function is saved in the
  column config (§3.6). This covers the common "just give me a total/count" need.
  Custom `values`-expression summaries are reserved for v2.
- **v2 (Tier 2): formula / computed columns** — **model: the Obsidian Bases
  formula DSL** (study done — `notes/brainstorm-obsidian-bases.md` §3, function
  inventory included). A JS-flavored typed expression language: method chaining
  (`price.toFixed(2)`), `if()`/`isEmpty()` guards, **duration-string date math**
  (`due < now() - '7d'`, `start + "2w"` — durations stay scalar-safe strings),
  and a small typed stdlib (the 20% that matters: `if`; string
  `contains/lower/trim/replace`; number `round/toFixed`; date
  `now/today/format/relative`; list `length/join/contains`). **Named formulas
  become pseudo-fields** (`formula:<name>` field kind), usable as columns, in
  filters, and as group-by axes — one expression engine, three consumers. Values
  derived, never written back. **Encoding:** one formula per property line —
  `tine.formula.<name>:: expr` — never packed into `=;` lists (expressions contain
  `=`/`;`); two scalar-safety escapes: serialize nested opens as `( (` (avoid OG's
  `((` block-ref parse) and escape `#` inside string literals. Ship with a
  **validating formula editor** (field-name autocomplete + live syntax check) —
  the UX reason Bases' DSL is learnable.
- **Rejected (do not build):** TreeSheets' operator-cell "Run" language (modal,
  no markdown home) and A1-style `=SUM(B2:B5)` references (no stable coordinates in
  a ragged recursive grid; bad round-trip).

---

## 7. Cell styling

Reuse Tine's existing **block highlight colors** for per-cell background/text
color (round-trips as the same `background-color::`-style property OG renders).
Bold/italic/borders/drawstyles are deferred (Tier C).

---

## 8. Rendering & performance

- **Start from the query-table renderer** (`src/components/Macro.tsx:332-377`) —
  it already renders matched blocks as a sortable table. The net-new work over it
  is **live-editable cells**, the 2-D interaction model, and **generalizing columns
  from properties to fields** (facets — task state/priority/scheduled/tags, not
  just `property::`; extend `cols()` `:178-182` and the `Row` type `:166-175`).
- **Projection vs editor:** Tine separates the cheap `blockView` projection
  (`src/render/block.ts`) from the live `Editor`. **Only the focused cell mounts a
  live `Editor`; every other cell renders the cheap projection.** This is already
  how multi-surface editing works and is the main perf lever.
- **PHASE 0 — a throwaway perf spike BEFORE any Phase-1 code.** The TreeSheets
  *feel* (fluid auto-resize-to-content per keystroke) is the premise of this
  feature, and TreeSheets achieves it with an immediate-mode C++ canvas — DOM +
  WebKitGTK may not deliver it. Falsify the risk first: a disposable 50×10 grid +
  one nested sub-grid + a live editor, on the uni machine's WebKitGTK; measure
  keystroke→paint and resize-reflow latency; set a jank budget (≈16 ms typical /
  50 ms worst) and choose the auto-fit strategy (native CSS table auto-layout vs
  measured fixed tracks) from data. Go/no-go gate. (Context: the earlier
  wheel-scroll trouble turned out to be largely **MX Master 3 high-resolution
  scrolling** — hardware-specific; a Logi Trackball behaves normally — so
  WebKitGTK may be less scary than feared. Verify anyway.)
- **No virtualization today** (the feed renders every loaded block; `Page.tsx`,
  `Block.tsx:321`). A large grid materializes every cell. v1 mitigation: focused-cell-only
  live editors + `content-visibility` on offscreen rows; **log/cap on very large
  grids** rather than silently degrade. Known risk — note it, don't pretend.
  (Field validation: even Obsidian's resourced team drew "Bases is super slow and
  stuttery" complaints on large tables — the cap/log mitigation is not paranoia.)
- Layout via CSS Grid `fr` tracks; resizers generalize the existing pointer resizers.

### 8.1 The query layer (layers A–C) — grounded, not assumed
Four separable perf layers; §8 above is layer D (DOM render) only. The
grid/board's "database of your bullets" promise is a **layer A–C** promise.
Grounded facts (code study, Jul 3 2026):

- **A — parse/load: lazy + paced warm.** `Graph::open()` parses nothing
  (`crates/tine-core/src/model.rs:373-400`); the full parsed-page cache builds on
  first graph-wide call (`with_pages`, `model.rs:1322-1347`) or via a background
  warm paced 2 ms/24 pages (`warm_cache`, `model.rs:1351-1411`). Tine avoids OG's
  boot tax by construction; the first query after startup pays the cache build.
- **B — evaluation: Rust full scan over the parsed cache, memoized.**
  `run_query` walks every block (`query.rs:210-284`); memoized by query-string +
  `cache_gen` (`model.rs:266-271`). The DSL already covers task markers, tags,
  page refs, property filters, and/or/not, sort — **the kanban/table row source
  already ships**; new work is renderer + write-back, not a query engine.
  Backlinks use the identical scan→memoize→invalidate architecture
  (`query.rs:107-138`) and are used daily without complaint.
- **C — liveness: reactive but crude.** Query resources key on `dataRev()`
  (bumped per save) → any edit invalidates **all** memos → next render = full
  re-scan. Cost model: **one full graph scan per (edit, visible query view)**,
  debounced by save.
- **No indices exist** — caches are memoized *results*, not acceleration
  structures. **Upgrade path (v2, only if measured hot):** facet indices
  (marker/tag/property → blocks) in the Rust `Graph`, built during warm,
  maintained at the save path; the scoped-invalidation logic that already detects
  which queries an edit affects (`query.rs:286-300`) is the reuse point;
  frontend mutations all flow through ~10 store functions ending in
  `setDoc`+`markDirty` — a clean choke point. The existing DSL maps directly to
  index lookups; no new query language.
- **Phase-0 gate (layer B):** on the REAL graph, time `run_query("TODO")` cold
  (incl. cache build) and warm, and the edit-while-board-visible re-scan path.
  Budget: warm scan comfortably under a frame (~10-30 ms) at current graph size;
  if not, indices move from v2 to v1.
- **Positioning, mechanically:** Tine can do what Bases can't because it already
  pays — in Rust, lazily, paced — the parsed-graph cache Obsidian core refuses to
  pay in JS/mobile, and that OG paid in ClojureScript at boot (their slowness).
  The moat is a sunk cost, but keep it honest: it's a *measured* claim per the
  Phase-0 gate, not a vibe.

---

## 9. Codebase seams to build on

| seam | location | use |
|---|---|---|
| geometry-agnostic store | `src/store.ts:41-82` | the engine reads children by id; no model change |
| only vertical-geometry code | `src/components/Block.tsx:317-324` | add a `view::` render branch here |
| query → property table | `src/components/Macro.tsx:332-377`, cols() `:178-182`, rows `:166-175` | copy-from base; generalize cols() from props-only to **fields**, extend Row to carry facets |
| multi-surface live editing | `SurfaceContext`, `Block.tsx:190` | a cell is another surface onto a block; cell edit = block edit |
| block zoom as route | `src/ui.ts:619-628`, `src/router.ts:206-216` | "zoom into a cell" — already built |
| **facets = the field extractors** | `src/render/facets.ts:111-125` (`deriveFacets`) | marker/priority/scheduled/props per block — the `Field` substrate already exists |
| task markers | `src/markers.ts:13-25`; `src/render/ast.ts:129` | the status field's value set |
| **marker write-back** | `src/editor/marker.ts:39-55` (`cycleMarker`), `repeat.ts:81-91` (smart) | card-move on a task board = flip the marker |
| scheduled/deadline | `readSchedule` `src/store.ts:1260-1276`; `src/editor/planning.ts:22-65` | date field extract + write-back |
| tags/links | lsdoc AST `src/render/ast.ts:60-65` | tag field |
| property parsing | `isPropertyLine` `src/render/block.ts:31-38`; `pageProperties` `:44-81` | the property field kind (opt-in) |
| pointer resizers | `src/App.tsx` content-row, `src/ui.ts:167` | generalize for grid tracks |
| pointer reorder drag | `src/components/Block.tsx:122` (`beginDrag`) | basis for seam/cell drag |
| **query engine (full-scan, shipped)** | `crates/tine-core/src/query.rs:210-284` (`run_query`/`run_pred`); memo `model.rs:266-271`, `:1675` | the kanban/table row source already exists — DSL covers markers/tags/props/bools |
| lazy load + paced warm | `model.rs:373-400` (`Graph::open`), `:1322-1347` (`with_pages`), `:1351-1411` (`warm_cache`) | layer-A cost model; first query pays the cache build |
| scoped query invalidation | `query.rs:286-300` | the reuse point for incremental facet indices (v2-if-measured) |
| store mutation choke point | `store.ts` mutators (~10 fns, all via `setDoc`+`markDirty`) | clean hook for any frontend-side index maintenance |
| serialize (tree-only) | `crates/tine-core/src/doc.rs:434-453` | the round-trip boundary |
| build flag | `--features custom-protocol` | release build (see `tine-release-build` memory) |

---

## 10. Scope: v1 / v2 / deferred

**v1 (this spec's target):**
- the engine, `leafKind: block`, with `rowSource`/`columnModel`/`trackModel` seams;
- positional grid (`children`+`positional`); **field-keyed** table (`rowSource`
  children **and** query), columns from **facets** (task state / priority /
  scheduled / tags / page / properties — NOT properties-only), editable cells that
  write back per field kind; supertag table + tag-page table via an **opt-in
  toggle** (NOT the default tag-page view — OG-parity) + add-row;
- **the task kanban** (`query(tasks)` + `group-by: state`, columns TODO/DOING/DONE,
  card-move = `cycleMarker`, keyboard card-move `Ctrl+←/→`, column card counts) —
  the showcase, needs zero properties;
- **discoverability**: `/table` · `/grid` · `/board` slash commands; a grid+kanban
  demo page in the onboarding welcome-graph templates
  (`crates/tine-core/src/templates/`);
- the edge/selection interaction model + the **full §4.3 spreadsheet keyboard**
  (navigation, 2-D range-select, content-move, fill, TSV/indented-text clipboard);
- automatic resize-to-content;
- Hierarchify / Flatten; board via **single-valued** group-by (§3.4), `(none)`
  column, rich **card faces** (non-grouping fields as chips);
- column aggregates (Tier 1);
- recursion (cell's own `view::`); zoom-into-cell (reused);
- cell highlight colors;
- the logical-matrix render pass (all spans `1×1`).

**v2:** merged cells (`span::`); **multi-valued group-by (tag boards)** — Notion-style
card-in-each-column, drag adds/removes the value (§3.4); **`tine.fields::` tag-page
schema** (stable columns, typed inputs, add-row defaults — §3.3 ladder); formula
columns (Tier 2 — Bases-DSL model per §6; study DONE); a **`this` context
variable** in query/filter expressions ("rows linking to *this* page" /
"children of *this* block" — what makes an embedded view feel live, stolen from
Bases); **pipe-table ↔ grid conversion commands**
(Tine already renders `md-table`; existing notes become upgradeable); **CSV
file-drop → grid** (extends TSV paste + `src/filedrop.ts`); positional Hierarchify
(group by column values); Transpose; Hierarchy-Swap pivot; shrink-not-hide analog relative-size (grids first, with a config flag to
extend to the outline — keep size a render property *on the block*, not grid-only,
so the outline version isn't precluded); zoom "deeper levels still drawn smaller";
F9 wrap-in-parent; per-column wrap width.

**v3 / deferred:** **split view** = `leafKind: pane` (each cell hosts a navigable
PageView + tab chrome; needs per-pane `RouteContext` instead of the global
singleton `route()`; the seam-materialize gesture = "split here"; a
focus-pane-N jump Cmd/Ctrl+1–9; drag-tab-to-seam via pointer events); **lookup /
VLOOKUP columns** (the relations/"don't duplicate" wish); drawstyles (grid/bubble/
tree-line); tag-color + go-to-matching-text; filter/spotlight overlay;
images-in-cells; **canvas face** (`trackModel: free`) — the whiteboard-adjacent
job that IS engine-shaped. Decomposition of the "whiteboards" requests (Jul 5
2026): (a) *freehand sketching* stays out of the engine forever — served by the
excalidraw asset pattern (`docs/excalidraw-assets-spec.md`); (b) *spatial
arrangement of existing blocks* = the mainstream job (evidence: Obsidian shipped
Canvas — cards + embeds + edges, deliberately no ink — as core and left
excalidraw to a plugin) and is a fourth face: children rendered as free-floating
cards, geometry as scalar `tine.x:: / tine.y:: / tine.w:: / tine.h::` props
(same §3.1 encoding discipline), edges = block refs with an optional label,
rendered as absolutely-positioned DOM divs inside a pan/zoom container (no
`<canvas>` element — cards reuse the block renderer). Round-trips as a flat
bullet list OG shows harmlessly — unlike OG's own whiteboards, which live in
non-markdown `whiteboards/*.edn` sidecars. Engine-honesty note (§2 discipline):
the canvas face shares Fields, card rendering, and the encoding rules, but NOT
the track/matrix/seam machinery — the `free` track model bypasses that pass
entirely; don't contort the matrix to host it. **Rider: one-way lossy importer
from OG `whiteboards/*.edn`** — OG stores each tldraw shape as a block whose
property IS the shape (`shape->block`/`block->shape`,
`handler/whiteboard.cljs:29`), so `logseq-portal` shapes (`blockType` P/B +
`pageId` + point/size) map 1:1 to cards (`[[page]]` / `((uuid))` content +
`tine.x/y/w/h`), text shapes → text cards, bound arrows → edges; ink and
geometry shapes are dropped with an explicit per-shape-type count report
(import, never sync — the `.edn` file is left untouched). Parse the EDN with
the existing `edn.rs` reader (one-parser rule).

---

## 11. Sequencing (phases)

0. **Perf spike (throwaway — §8 + §8.1).** Two measurements, one gate: (D) render
   — 50×10 grid + nested + live editor on WebKitGTK, jank budget, auto-fit
   strategy; (B) query — `run_query("TODO")` cold/warm on the real graph + the
   edit-while-board-visible re-scan. Go/no-go + "do indices move to v1?" decision.
   Nothing ships.
1. **Encoding + read-only positional grid.** Property names are already decided
   (§3.1). Render a `tine.view:: grid` node's children as a positional table (reuse
   the query-table CSS + the logical-matrix pass). Round-trip test (§12). No editing yet.
2. **Editable cells + the modality + the full keyboard.** Live-edit a cell
   (write-back via multi-surface), the selection-vs-edit modes, the edge/seam
   insert + delete, and the **complete §4.3 keyboard** (2-D navigation,
   range-select, `Ctrl+arrow` content-move, fill, TSV/indented-text clipboard).
   Pointer-event drag only. This is the largest phase — the keyboard is the feature.
3. **Field-keyed table + query rowSource + the task kanban.** Generalize columns
   from properties to **fields** (facets); `rowSource: query`; the **task kanban**
   (`query(tasks)` + `group-by: state`, card-move = `cycleMarker`) — the showcase,
   zero properties; and the supertag table (opt-in tag-page toggle, add-row). Cells
   write back per field kind. All mutations under the §3.8 atomicity rules.
4. **Hierarchify / Flatten + board + aggregates.** Group-by (view) → board; commit
   via Hierarchify; Flatten; per-column footer aggregates.
5. **Recursion + colors + polish.** Cell-level `view::` (table-in-table), zoom-into-cell,
   cell highlight colors, hover-handle affordances, empty-cell ergonomics.

Each phase is independently shippable and independently round-trip-tested.

---

## 12. Definition of done / verification (per phase)

- **Build:** `--features custom-protocol`; source `scripts/env.sh` (see
  `tine-release-build`).
- **Self-verify visually** with the headless screenshot harness *before* handing to
  Martin — build, screenshot the relevant grid state against the mock backend, look
  at the image, iterate (project CLAUDE.md rule; `docs/SCREENSHOTS.md`).
- **Round-trip test (the acceptance gate):** write a grid in Tine → confirm the
  on-disk markdown is a plain nested outline + harmless properties → open it in OG
  (file version) and confirm it renders without error → reopen in Tine and confirm
  the structure is identical. Grow the shared test graph at `~/research/tine-test`.
- **Docs:** update `README.md` feature list/roadmap and regenerate any touched
  screenshots (`docs/SCREENSHOTS.md`) in the same chunk of work.
- **Discoverability & showcase:** TreeSheets' own confessed failure was "not
  obvious why you'd want this" — so each shipped face gets: its slash-command
  entry point, a spot on the onboarding demo page, **many README screenshots**
  (one per face: grid, table, board, recursion), and — once the feature is real —
  a short video (Martin records). Screenshots per the SCREENSHOTS.md workflow.
- **Undo audit per phase:** every mutation introduced in the phase is verified to
  be a single atomic undo unit (§3.8) before the phase is called done.
- Commit in meaningful chunks; push to GitLab `origin/master` per `tine-push-policy`.

---

## 13. Open decisions (genuinely undecided — settle in Phase 1)

1. ~~Config property names~~ — **RESOLVED (OG-source study, §3.1):** dotted
   `tine.`-namespaced keys (`tine.view`, `tine.group-by`, `tine.span`,
   `tine.col-aggregates`, `tine.col-widths`, `tine.header`); values must be plain
   scalars / `=;`-delimited, never markdown-special (`[[]] (()) {{}} #`). No longer open.
2. **Mode-boundary transitions** — where the caret/selection lands entering/exiting
   a grid; click *into* a cell (edit) vs *onto* the grid (select); exact flow-out
   behavior at borders. The part most likely to feel wrong if rushed (§4.1).
3. ~~Board move semantics~~ — **RESOLVED (§3.4):** dragging a card = **write the
   group-by field on that one block** (state → `cycleMarker`, property → set value),
   a local single-block edit that works on scattered query rows. Structural reparent
   only applies when committing a grouping via Hierarchify (children source).
4. **Optional header row** for the positional grid (treat first row as a header?).
5. ~~Keyboard scope~~ — **RESOLVED (Martin, Jun 29): the FULL §4.3 spreadsheet
   keyboard ships in v1. The keyboard *is* the feature — do not defer any of it
   (range-select, content-move, fill, TSV clipboard all in v1).** No longer open.

---

_Spec authored from the 2026-06-28/29 brainstorm. The vision absorbed four user
requests into one engine: TreeSheets-style breadth, supertag/database tables, a
kanban board, and (deferred) VSCode-style splits — all round-tripping to plain
Logseq markdown._
