# Tine — full feature list

The complete catalogue. For the short pitch and install instructions, see the
[README](../README.md).

Tine **matches Logseq ("OG") by default** and round-trips the same Markdown/`.org`
files. **⊕ marks things Tine adds on top of Logseq core** (no plugins).

## Outliner & editing

- Click-to-edit blocks; the caret lands exactly where you clicked, including
  rendered bold/link/heading markup.
- **Headings stay heading-sized while you edit them** (Logseq parity) — clicking
  into a single-line `#`/`##`/`###…` heading keeps the text at its heading size and
  weight (the `#` markers show inline at the same size), so it doesn't shrink to body
  text on focus and jump back when you leave. Multi-line heading blocks edit at body
  size — only the heading's own line is enlarged — matching Logseq.
- `Enter` / `Tab` / `Shift+Tab` / `Backspace` / arrows with correct Logseq
  semantics and caret preservation — no reflow on indent/outdent; arrow nav
  respects *visual* wrapped rows.
- Collapse/expand, zoom into a block (with breadcrumb), drag-to-reorder, move
  up/down (`Alt+Shift+↑/↓`).
- Multi-block selection → move / indent / cut / copy; the viewport follows the
  active end as you extend past the top/bottom edge.
- Multi-line blocks, syntax-highlighted code blocks, Markdown tables.
- **One-click copy** on hover for fenced code blocks, inline `` `code` ``, and
  links — copies the raw source (not the rendered text) to the clipboard.
- Paste an indented outline → a real block tree; paste a clipboard image → a graph
  asset.
- **Paste a URL over selected text → a link** (Logseq parity) — select some text,
  paste a URL, and the selection is wrapped rather than replaced: `[text](url)` on
  a Markdown page, `[[url][text]]` on an Org page. (Skipped inside code, and when
  the selection is itself a URL — then a normal replace happens.)
- Inline formatting (`Mod+B/I`, strike, `==highlight==`, link) via a floating
  selection toolbar, plus Emacs-style word/line kill motions.
- **Select then wrap** (always on, Logseq parity) — with text selected, typing a
  wrap character surrounds it: `[` twice → `[[selection]]` (opens the page search
  seeded with the words — Enter links or creates), `(` twice → `((selection))`
  (block search), and emphasis marks `*` `~` `=` `_` (plus Org `/` `+` `^`) so a
  second press gives `**bold**`, `~~strike~~`, `==highlight==`.
- ⊕ **Optional auto-pairing** (Settings → Appearance) — for the *empty-caret* case:
  typing `(` `[` `{` `"` `` ` `` inserts the matching closer with the caret between,
  types through a closer, and `Backspace` on an empty pair clears both. **Off by
  default** (turn it on if you like it); page-ref `[[…]]` always auto-closes.
- ⊕ **Typographic replacements** (Settings → Appearance) — show `->` → `→`,
  `-->` → `⟶`, `--` → `–` (en), `---` → `—` (em) and friends as real glyphs,
  either *while reading* (your Markdown stays ASCII, only the rendered view
  changes — like `\Delta` → Δ) or *while typing* (rewrites the source itself). A
  Tine touch, not Logseq; default *while reading*, or turn it off.
- **In-block lists & checklists** — a `+`/`*`/ordered list *inside one bullet's
  content* renders as a styled list (distinct from outline bullets), with tickable
  `[ ]`/`[x]` checkboxes that are *not* TODO/agenda tasks. Uses `+` (OG's in-content
  marker) so it round-trips to OG and Logseq mobile.
- **Callouts / admonitions** — both Obsidian-style `> [!note] …` and org
  `#+BEGIN_NOTE … #+END_NOTE` render as colored callouts (`QUOTE` stays a plain
  blockquote).
- **Raw inline/block HTML** — HTML the way Logseq renders it: `<ins>`, `<del>`,
  `<sup>`/`<sub>`, `<kbd>`, `<mark>`, `<abbr>`, `<a>`, a self-closed `<img/>`, small
  containers. It renders **live, sanitized** to a safe allowlist (both the app and
  the HTML export) — scripts, event handlers (`onerror=`) and `style` are stripped.
  Two Logseq-parity notes: a *bare* `<img>` is literal in Logseq too (only a
  self-closed `<img/>` is raw HTML), and the Markdown carets `^x^`/`~x~` are literal
  in Logseq (not sub/superscript) — use the tags or `$math$`. (Sanitizer allowlist
  is shared between the two surfaces and contract-tested; see ADR 0019.)
  - **Local-file images (opt-in).** In-graph and `https` images always work; to also
    let a raw-HTML `<img>` load an image from an absolute path *outside* the graph
    (e.g. an imported note's `<img src="/home/…/pic.png"/>`), turn on **Settings →
    Editing → "Load local-file images"**. Off by default — it's a permission (a
    synced/imported note isn't self-authored), read over a gated, image-only IPC. The
    HTML export never serves local files.
- ⊕ **`/calc` block** — evaluates arithmetic live as you type (`+ - * / ^ %`,
  parentheses, `name = expr` variables across lines, a running result).

## Media

- Paste/import **images, video, and audio** (`/upload`); stored as
  `![](../assets/…)`.
- **Configurable asset filenames** (Settings → Backups → *Asset names*): a
  `%`-token template — `%assetname %ext %yyyymmdd %hhmmss` (and granular `%yyyy %MM
  %dd %HH %mm %ss`) — defaulting to the plain original name, with a one-click
  *Date + name* preset.
- **Drag the corner grip to resize an image *or a video*** — stored as a width % in
  Logseq's `{:width …}` brace, so it round-trips.
- ⊕ **Audio ⤢ Expand** opens a wide overlay player — a **waveform scrubber** with
  ±5s / ±15s skip, play/pause, speed, and a time read-out.
- Click an image for a **lightbox** (Esc / click-away to close; right-click / Copy
  puts it on the clipboard).
- Video/audio play **inline** where the codec is supported, else fall back to a
  click-to-open chip that launches the OS default player (Tine scrubs its own render
  env and detaches the child so the player doesn't inherit a broken video context).
- **Orphaned-media cleanup** (Settings → Backups): scan for `assets/` files no block
  references and move them to the recoverable trash — deleting a block never deletes
  its media, so this is how unused files get reclaimed.
- **Diagrams in your own drawio / Excalidraw** — Tine bundles no editor. drawio and
  Excalidraw can save an *editable* SVG (the diagram lives inside the file), so one
  `foo.drawio.svg` / `foo.excalidraw.svg` is both the rendered preview *and* the
  editable source. `/drawio` creates a new editable `assets/…​.drawio.svg`, inserts
  it as an image, and opens it in drawio; hovering any matching image shows an **Edit
  in …** button. Switch back to Tine and the rendered image refreshes. It's a plain
  image reference, so the graph still renders in Logseq. Set the editor commands
  (drawio has an **Autodetect**) under **Settings → Files → Diagram editors**; empty
  uses the system default opener. A `{}` in the command is replaced by the file path.
  Desktop only. (For Excalidraw, export with *Embed scene* into `assets/` as
  `name.excalidraw.svg`; there's no in-app "new Excalidraw".)

## Linking, references & queries

- `[[page]]`, `#tags`, `#[[multi word]]`, `((block ref))` — including the labeled
  form `[text](((id)))` — and `{{embed}}`, all clickable, with autocomplete on
  `[[`, `#`, `((`, and `/`.
- **Page icons on inline references.** If a page has an `icon::` (emoji or
  character), it shows as a prefix on every inline `[[reference]]` and `#tag` to
  that page — like Logseq, and like Tine's own page title / namespace listing. (Emoji
  render as Twemoji SVG, since WebKitGTK paints color-emoji fonts blank.)
- The `((` popup full-text-searches blocks and inserts a **durable** reference
  (writes a stable `id::` first).
- The `[[`/`#` Enter default is configurable (Settings → *Journals & tasks* →
  **Link autocomplete default**): create-a-new-page (default, like Logseq) or
  link-the-first-match.
- **Linked & unlinked references** on every page (live/editable), with co-reference
  filtering and hover previews — and in the **right sidebar** page view too
  (shift-click a page to open it there).
- **Per-block reference count** badge → click to reveal the referencing blocks
  (grouped by page, each with its **ancestor breadcrumb**), or shift-click to open
  in the sidebar.
- **Right-click an inline `((ref))`** for a menu (open in sidebar / go to block /
  copy ref / copy embed); in the editor, **`Mod+C` with nothing selected copies a
  reference** to the current block.
- Inline block refs render as **link-styled text** (full-strength color + accent
  underline, like OG — not a grey chip).
- Copying a block **strips `id::`** from the clipboard text (like OG) so it never
  leaks into a paste — though the `id::` stays in the file so sidebar/tab/zoom spots
  persist a restart. Copy behavior is configurable (Settings → Journals & tasks),
  with two Tine defaults that differ from Logseq (one click to revert): *copy only
  the selected blocks* (vs Logseq's whole sub-tree) and *strip `collapsed::`*.
- **Macros:** `{{query}}`, `{{embed}}`,
  `{{video}}`/`{{youtube}}`/`{{vimeo}}`/`{{bilibili}}`, `{{tweet}}`/`{{twitter}}`,
  `{{img url [w h] [left|right|center]}}`, `{{namespace}}`, and **user-defined
  `:macros`** from `config.edn` — positional `$1..$N` substitution (unfilled
  placeholders stay literal, like Logseq); a template that expands to block-level
  Markdown (heading / list / multiple paragraphs) renders as **real nested blocks**,
  not a flattened line.
  `{{youtube-timestamp}}`, `{{cloze}}` (click-to-reveal) and `{{zotero-*}}` render in
  a degraded form (no on-page-player seek / SRS / Zotero connector — flagged inline).
- **`{{query}}` engine** (inline or whole-block): boolean `and`/`or`/`not`,
  `(task …)`, `(priority …)`, `(property …)`, `(page-property …)`, `(page-tags …)`,
  `(scheduled)`, `(deadline)`, `(journal)`, `(namespace …)`, `(between START END)`
  with a field selector, `(sort-by …)`. Results render as a list or a sortable
  **table**; ⊕ an interactive **visual query builder** (chip/clause bar) builds them
  without writing the DSL. The builder's **Sort** control offers one-click presets
  — *Newest / Oldest first*, *Priority*, *Page*, *Deadline*, *Scheduled* — plus a
  free-text field for any other property. *Newest first* orders results on one
  recency timeline: journal pages by the day they represent (stable, not their file
  mtime), other pages by file modified-time, so journal and page todos interleave
  chronologically. (`sort-by modified/priority/page/deadline/scheduled` extend
  Logseq's property-only `sort-by`.)
- **Summarize results** (beyond Logseq) — the builder's **∑ summarize** control
  computes, with no code, a **count** / **sum** / **average** of a property over the
  matched blocks, and/or a **group-by** (page or property) that breaks the results
  down into a per-group table. Sum/average parse the property numerically and report
  how many rows were skipped. Rides in the DSL as `(aggregate count|sum|avg …)` /
  `(group-by page|<prop>)`; the engine returns the full set and the math is computed
  client-side. (Logseq does this only via Datalog `:result-transform`.)
- A scoped compatibility path for Logseq's **advanced (Datalog) queries**:
  recognized clauses (`task`, `between` with a field selector, `property`,
  `page-property`, `priority`, `page`, `namespace`, `page-tags`, `scheduled`,
  `deadline`, `journal`, page-refs, boolean `or/and/not`, `:today`/`:current-page`-style
  inputs) map onto the same engine; any unsupported part is **flagged** in the result
  rather than silently dropped or wrongly answered.

## Sheets (2-D grids)

Tine's first deliberately beyond-Logseq feature: render a block's children as a
**recursive, editable, TreeSheets-style grid** — while everything round-trips to
plain Logseq markdown (and org). Geometry is the outline shape; config is a few
harmless `tine.*` properties (`tine.view:: grid`, `tine.header:: true`,
`tine.col-widths:: 0=140`, `tine.fields:: qty=number;done=checkbox`). Logseq
renders the same file as an ordinary nested outline — no sidecars, no
coordinates, no lock-in.

- **Positional grid** — a block with `tine.view:: grid` shows its children as
  rows and grandchildren as cells (sibling order = column). Ragged rows render
  holes; empty cells are just empty bullets. Optional header row
  (`tine.header:: true`).
- **Cells are real blocks** — click selects a cell; double-click, `Enter`, or
  `F2` edits it like any block (links, refs, tags all live). Nested children
  render inside the cell, and a cell can itself be a grid (recursion).
- **Full spreadsheet keyboard** — 2-D arrow navigation with a
  selection↔edit ladder (`Enter`/`Esc`), `Tab` cell-walk, type-to-overtype,
  `Shift+arrow` range select (plus row/column/grid selection),
  `Ctrl+arrow` content moves, `Ctrl+D`/`Ctrl+R` fill down/right.
- **Seams** — the boundary between rows/columns is a first-class selection
  target (TreeSheets-style): arrow onto a seam, then **type to insert a
  row/column right there**; `Backspace`/`Delete` on a seam removes the adjacent
  row/column. Drag a column ruling to resize (double-click resets).
- **Clipboard interop** — copy a range as TSV (+ an HTML table, so it pastes
  into real spreadsheets); paste TSV/CSV to fill and grow the grid; paste
  indented text to build nested structure. Dropping a `.csv` or `.tsv` file
  onto a page creates a new grid block.
- **Field tables** — `tine.view:: table` turns children or query results into an
  editable table whose columns come from facets: task state, priority,
  scheduled/deadline dates, tags, page, and block properties. Writable fields
  write back to the source block. Optional `tine.fields::` schemas pin column
  order and type cells as text, number, date/datetime, checkbox, list, ref, or
  enum; the header menu edits that schema in place.
- **Formula columns and filters** — add read-only computed columns with one
  property per expression:
  `tine.formula.effort:: points * 2`,
  `tine.formula.due-soon:: if(isEmpty(deadline), false, deadline < today() + "7d")`,
  and filter rows with `tine.filter:: status != "done" && formula.due-soon`.
  The DSL is a JS-flavored, typed expression subset: field names are bare
  identifiers, formulas can reference `formula.<name>`, and the stdlib is
  deliberately small (`if`, `isEmpty`; text `.contains/.lower/.trim/.replace/.length`;
  number `.round/.floor/.ceil/.abs/.toFixed(n)`; date `now()`, `today()`,
  `.format(fmt)`, `.year/.month/.day`, `.relative()`; list
  `.length/.join(sep)/.contains(x)`). Errors are values: a bad formula renders an
  ⚠ chip with the message on hover, and formula values are derived at render time
  and never stored back onto blocks.
- **Task kanban** — `tine.view:: board` groups task/query rows by state,
  priority, or a property; dragging a card or pressing `Ctrl+←/→` writes the
  grouping field back to the card.
- **Tag boards** — boards can group by tags too: a multi-tag card appears in
  each matching column, and moving it adds/removes the tag on that block.
- **Formula group-by and fail-open filters** — boards can group on computed axes
  with `tine.group-by:: formula.due-soon`; formula axes are read-only, so moving a
  card cannot rewrite a derived value. `tine.filter::` works on tables and boards
  with the same evaluator. Honesty rule: if the filter has a parse error, returns
  a non-boolean, or any row evaluates to an error, filtering is disabled for the
  whole view and Tine shows a **Filter disabled** chip instead of quietly hiding
  rows.
- **Validating formula editor** — right-click a table or board and choose **Add
  formula…** or **Edit filter…**. The popup live-validates the expression with a
  caret marker, offers field/formula/stdlib chips, and keeps Save disabled until
  the expression parses.
- **Hierarchify / Flatten** — commit a field grouping into the outline, or pull
  grouped children back up into a flat table.
- **Conversions** — convert a markdown pipe table block into a grid, or export
  a safe grid back to a pipe table. The conversion keeps markdown cell source
  where the parser exposes spans, refuses lossy grids, and makes the whole
  operation one undo step.
- **Aggregates** — per-column summaries (sum, average, min/max, dates, filled,
  unique, checked, and more) live in a quiet footer and store only the selected
  aggregate token.
- **Cell polish** — cells/cards render Logseq block highlight colors, cell
  menus can switch child rendering between outline/grid/table or zoom into the
  cell, and scheduled/deadline table cells use the same date picker as normal
  blocks.
- **Tag-page tables** — a tag page with tagged references can opt into a table
  view (`tine.tag-table:: true`) and add new tagged rows to today's journal.
- **Slash commands** — `/grid`, `/table`, and `/board` convert the current block
  into the corresponding sheet face.
- **Safety** — every grid gesture is a single undo step, and grid pages are
  byte-exact round-trip tested (md and org).

V1 limits: `page` columns are read-only; range operations are single-level
within the current grid; board cards can move between columns but not reorder
within a column; merged cells are still v2+.

## Split view

- ⊕ **Panes with independent tabs and history** — split the workspace into
  multiple note panes. Each pane has its own tab strip, active tab, back/forward
  stack, scroll position, and focused-pane indicator; the layout is saved in the
  session and restored on launch.
- **Default split bindings:** `Mod+Alt+\` splits right and duplicates the current
  tab; `Mod+Alt+Shift+\` splits down. The *Close pane* command is available from
  the command palette.
- **Pane focus bindings:** `Mod+1` … `Mod+9` focuses panes in spatial reading
  order; `Mod+Alt+Left/Right/Up/Down` focuses the nearest pane in that direction.
- **Move-tab bindings:** `Mod+Alt+Shift+Left/Right/Up/Down` moves the active tab
  to the nearest pane in that direction. Moving the last page tab out closes the
  emptied pane; the journals feed keeps its last tab.
- **Esc pane-select ladder:** once overlays/editing/block selection have peeled
  away, `Esc` enters pane-select mode. Arrow keys step through panes, seams, and
  outer edges; `Enter` on a pane returns to it; `Enter` on a seam/edge makes a
  **mirror split** (same content side by side, no dialog); **typing** on a
  seam/edge opens a split with the switcher pre-filled with what you typed —
  the fast "open/create that page in a new split" (commands are hidden there;
  only pages, page creation, and blocks).
- **Open to the side:** `Ctrl+click` a page link, tag, or block reference to open
  it in another pane, creating a right split when needed. In the `Ctrl+K` switcher,
  `Alt+Enter` opens the highlighted page/create/block result in the other pane.
- **Tab drag:** drag a tab within a strip to reorder it, onto another pane's strip
  to move it at that position, onto a pane body to append and activate it there,
  or onto a seam/pane edge to split that half and move the tab into the new pane.
  `Esc` cancels an in-progress tab drag.

## Tasks, journals & dates

- `TODO/DOING/DONE/NOW/LATER/WAITING/CANCELED`, two configurable workflows,
  priorities, cycle with `Mod+Enter`.
- **Time tracking / logbook** — marker transitions write OG-compatible
  `:LOGBOOK:` drawers: moving into `DOING`/`NOW` clocks in, moving back to
  `TODO`/`LATER` or into `DONE` clocks out. CLOCK rows use Logseq's exact local
  timestamp shape (`yyyy-MM-dd E HH:mm[:ss]`, English weekday abbreviations),
  seconds are on by default, and closed rows keep the two spaces after `=>`.
  `DONE`/`TODO`/`LATER` blocks show an elapsed-time badge with a Type / Start /
  End / Span tooltip; the drawer itself stays hidden by default, like Logseq.
- **Task checkbox** in front of every task block (Logseq parity): click it to mark
  the task `DONE`, click again to reopen it (to `TODO`, or `LATER` under the "now"
  workflow); a repeating task rolls its date forward instead of closing. `DONE`
  shows a checked box, `CANCELED`/`CANCELLED` show none. The marker word stays
  beside the box and still cycles on click. Checkboxes also show (read-only) on
  tasks surfaced in Linked References, query results, and embeds.
- `SCHEDULED:` / `DEADLINE:` via a calendar **date picker** (`/scheduled`,
  `/deadline`), with an optional **clock time** ("Add time" → `HH:mm`, written as
  `<2026-07-07 Tue 14:30>` like Logseq) and **recurring tasks** (`+1w` / `.+1w` /
  `++1w`) where completing a repeater advances the date. Re-picking the date keeps
  an existing time (and repeater). You can type a planning line *anywhere*
  in a block while editing; on exit it's moved to its canonical position (after the
  first line, before properties — OG's layout). A `SCHEDULED:`/`DEADLINE:` inside
  inline code or a code fence stays literal content (it's not a real timestamp), so
  it's never turned into a date badge or moved.
- ⊕ **Carry unfinished tasks forward** to today (presets for the last 7 / 30 / 365
  days or a configurable N), optionally keeping ancestor context.
- Multi-day **journal feed** (one continuous editable list); today's journal created
  lazily on first edit; move blocks across days.
- An **agenda** of *open* scheduled/deadline items (DONE and CANCELED hidden, like
  OG) in a configurable look-back/-ahead window; journal **templates**; a calendar
  with content markers whose **first day of week** follows your `config.edn
  :start-of-week`.

## PDF annotation

- Open PDFs in a resizable, zoomable pane (instant zoom, HiDPI, per-page
  virtualization); in-PDF `Ctrl+F` find with a page jump box.
- Select text → colored **highlights**, or drag a rectangle (area mode / `Ctrl`-drag)
  to clip an **area (image) highlight** — both stored Logseq-compatibly
  (`assets/<key>.edn` + `hls__` pages, area crops as PNG assets).
- Each highlight becomes a clean bullet you can nest notes under; writes **merge with
  disk**, so an externally-added highlight or your top-level notes are never dropped,
  and recoloring a highlight updates its note-page badge to match.

## Search & navigation

- `Ctrl+K` quick switcher: page titles + full-text content hits (visible text only —
  no false hits on hidden properties/uuids), with block breadcrumbs and middle-click
  → background tab.
- `Mod+F` in-page find on normal pages: a slim find bar with next/previous,
  `n / total` counts, and highlights for the current page. The match list is built
  from the block model, so collapsed or lazy-rendered branches are found and opened
  before the active match is highlighted.
- Command palette (`Mod+Shift+P`), favorites, recent pages, a collapsible
  **namespace tree** in the sidebar, the **`{{namespace X}}`** macro (a bold
  "Namespace" header + nested descendant tree), an automatic **"Hierarchy"** section
  (breadcrumb paths of descendant pages) on any namespaced page, and read-only
  **"aka" alias chips** on pages reachable by another name.
- ⊕ **Right-click page rows in the left sidebar** (favorites, recents, all pages,
  namespace tree) for the full page menu, including trash-backed Delete. Logseq core
  has page-title journal delete, but not sidebar-row delete.
- ⊕ **Built-in tabs.** Middle-click any bullet, page title, query result, or
  switcher row to open it in a background tab; pin (persisted), drag-reorder, `Mod+W`
  to close. Plain navigation to a route already open in another tab focuses that
  tab instead of duplicating it; Settings → Editor can turn this off. (Logseq core
  has no tabs.)
- ⊕ **Browser-style back/forward** (`Alt+Left` / `Alt+Right`, per-tab history, works
  mid-edit).
- ⊕ **Focus mode + dim-inactive-blocks** (`t f` / `t b`): hide the chrome and fade
  everything but the block you're working on, with Logseq-style layered `Esc`.
- ⊕ **Global quick-capture** — bind `tine --capture` to a desktop hotkey and a small
  always-on-top box pops from *any* app, with the full editor (autocomplete, slash
  commands, the date picker, nested blocks), filing a bullet to today's journal.
- **Page icons** — a page's `icon::` emoji shows next to its title and in the
  namespace tree. Emoji render as bundled **Twemoji SVG** images (not a font), so
  they show in every engine — including WebKitGTK, which paints color-emoji webfonts
  blank — and work offline.
- **Page rename** (double-click a title) rewrites every `[[ref]]`/`#tag` across the
  graph in one transaction.

## Works with your existing setup

- **Edit safely alongside Logseq mobile over Syncthing.** A filesystem watcher —
  **inotify by default** (zero idle wakeups), with a polling fallback for filesystems
  where inotify misses edits, switchable in Settings — reconciles changes synced in
  from other devices, and Tine **never silently overwrites a file that changed on
  disk — it surfaces a conflict** instead. Saves preserve each file's exact
  formatting (tabs vs spaces, comments, compact EDN) and skip byte-identical
  rewrites, so they don't create sync diff churn.
- **Page rename is transactional** — the page move and every `[[ref]]`/`#tag` rewrite
  commit all-or-nothing, re-checking each file just before writing and rolling back
  on conflict.
- **Custom journal date formats** — reads `:journal/file-name-format` and
  `:journal/page-title-format` and recognizes/creates journal files in your format
  (e.g. `dd-MM-yyyy`, `yyyy-MM-dd`, `yyyyMMdd`), falling back to the defaults so
  old/foreign files still resolve. The display-title format is pickable in Settings.
- **Duplicate-day reconcile** — if two files ever resolve to the same day, Tine keeps
  **both** rather than silently dropping one, and Settings → *Backups & recovery* → **Duplicate
  journal days** lets you **Open** / **Merge** / **Rename** / **Trash** each.
- **Sync-conflict merge** — Syncthing/Dropbox leave a `*.sync-conflict-*` (or
  `(conflicted copy)`) file when the same page was edited on two devices. Tine keeps
  these **out of your page list** (they're not real pages) and surfaces them under
  Settings → *Backups & recovery* → **Sync conflict copies**. **Review & merge** shows a
  **block-by-block diff** against the current page — matched by `id::`, then by
  content, then by first-line similarity — with a per-block **keep-current /
  keep-copy / keep-both** choice (and page-property merge); **Discard copy** trashes
  it. The merge writes through the normal save path (base-revision-guarded, atomic)
  and moves the copy to the recoverable **trash** — never auto-merged, never unlinked.
- **Pages in sub-folders are found** — like Logseq, Tine scans `pages/` (and
  `journals/`) **recursively**, so pages you've filed into real sub-directories
  (e.g. archiving `pages/client-a/…`) show up in the page list and are searchable
  and linkable. A nested page is keyed by its **file name** (`pages/client-a/foo.md`
  → page `foo`), matching Logseq; edits save back to the file in place. (Namespaces
  — `parent/child` — are still the flat `parent___child.md` filename form, not real
  folders, exactly as in Logseq.)
- **Org-mode graphs** — opens, renders, and edits `.org` pages and journals
  (headlines as blocks; org inline `*bold*` `/italic/` `_underline_` `~code~`
  `[[target][desc]]`; TODO markers; `#+BEGIN_SRC`/`QUOTE`). Mixed `.md` + `.org`
  graphs work; the **File format** setting (`:preferred-format`) chooses what new
  pages/journals use. A `.org` file is rewritten only when Tine can reproduce it
  **byte-for-byte** — anything it can't round-trip loads **read-only**, so it can
  never corrupt an org graph.
- **Launch snapshots** (configurable keep-count) with a restore UI that takes a
  safety snapshot first; page delete moves to a recoverable **trash**; `atomic_write`
  + fsync.
- **Switch graphs** right from the sidebar header — click the current graph name
  (under the "Tine" wordmark) for **Open graph…** (native folder picker) and
  **New graph…**. Also openable from the command line: `tine /path/to/graph` or
  the `TINE_GRAPH` env var. (No saved recent-graphs list yet — you pick the folder
  each time.)

## Mobile (Android)

- **Native Android app** (Tauri v2, arm64) that opens and edits your **real**
  Logseq graph — the same Markdown files, over your own sync. On first run you
  grant "All files access" and pick your graph folder (e.g. a Syncthing folder),
  then Tine reads and writes it directly, so it coexists with the Logseq mobile
  app on one graph. The file watcher runs in poll mode so external edits appear
  live.
- **Above-keyboard editing toolbar** — a strip that docks above the on-screen
  keyboard while editing, with the keyboard-only actions: outdent / indent, move
  block up / down, soft line break, TODO, **camera**, **voice memo**, date,
  `[[ ]]` / `(( ))`, the slash menu, and a pinned hide-keyboard button.
- **Camera & voice memo** — the camera button takes a photo (or picks an existing
  image) straight into the graph's `assets/` and inserts it; the mic button
  records a voice memo (`.m4a`) into `assets/` and inserts an audio player
  (permission-prompted on first use, with a pulsing stop button while recording).
- **Mobile-tuned UI** — a real app icon, an edge-to-edge layout that keeps
  controls clear of the status/navigation bars, a hardware **Back** button that
  navigates within Tine (exiting only at the root), and compact journal headers
  and settings for a phone.
- **Distribution** — sideloaded, release-signed APK attached to each GitHub
  release (built and signed in CI). Play Store / F-Droid are planned; iOS is
  being scoped.

## Customization & output

- **Help & shortcuts** — `?` toggles the OG-style bottom-right help popup;
  **Keyboard shortcuts** inside it opens the Settings modal on the shortcuts tab.
  `g s` opens that tab directly. Unlike OG's popup entry, Tine deliberately shows
  shortcuts in Settings rather than the right sidebar, so remapping and reference
  docs live in one place.
- **Fully remappable keyboard shortcuts** — in the Settings modal or via
  `config.edn :shortcuts`.
- **Help improve Tine** (Settings tab) — runs Tine's parser (lsdoc) against
  Logseq's own parser (mldoc) over your graph, locally, and reports any place they
  disagree plus a parse-speed comparison. Every divergence snippet is **anonymized**
  (words scrubbed, markup structure kept) and **re-verified to still reproduce the
  divergence** before it's shown, so it's safe to paste into a bug report. mldoc is
  loaded only on demand; nothing is uploaded.
- Light/dark themes, a built-in theme gallery (Default, Nord, Solarized, Gruvbox),
  accent color, custom CSS, wide mode (`t w`), document mode (`t d`). Gallery
  themes are app-level and device-local: Tine stores only the selected theme id in
  its backend settings, applies the theme as a managed `#tine-theme` CSS layer, and
  never writes to your graph. Tine also aliases common Logseq `--ls-*` theme
  variables, so both gallery themes and file-based themes in `logseq/custom.css`
  can recolor backgrounds, text, links, borders, and inline code. The cascade is
  built so your own `logseq/custom.css` loads last and takes priority. This is
  theme CSS compatibility, not plugin support.
- **Developer tools** — `Ctrl+Shift+J`, *Toggle developer tools* in the command
  palette, or right-click → *Inspect Element* opens the WebKit/WebView inspector for
  theme and CSS debugging; the shortcut toggles it closed. Available in release
  builds, not just debug. (`Ctrl+Shift+I` / `F12` are captured by WebKitGTK itself,
  so Tine's default is `Ctrl+Shift+J` — remappable under Settings.)
- ⊕ **Spell checking** in the editor (on by default, like Logseq) with red squiggles
  + right-click suggestions, using the system dictionaries. Unlike Logseq you can
  check **multiple languages at once** — Settings → Editor lists the dictionaries
  installed on your machine; tick the ones you want (no locale codes), and a word
  valid in any ticked language isn't flagged. None ticked follows your OS locale.
  Install more with your package manager (e.g. `hunspell-cs`) and hit Rescan.
- One-click **static HTML export** (`public:: true` pages) — like Logseq's published
  graphs, each page gets a **left sidebar** (Favorites / Journals / Pages) and a
  **fuzzy full-text search** box (block-level; results deep-link straight to the
  block). Driven by a small embedded index + a vendored Fuse.js, so the exported site
  works offline / off disk. Pages render from the **same parser the app uses** (lsdoc's
  canonical HTML), so the export matches what you see: syntax-highlighted code blocks
  (highlight.js), aligned tables, callouts, KaTeX math, lists and task checkboxes,
  **task markers + priorities + SCHEDULED/DEADLINE**, block properties, page/block
  links, and **sanitized raw HTML**. Dynamic content is resolved **at publish time** against your graph, too:
  `{{query …}}` runs and lists its results, `{{embed}}` inlines the target block/page,
  `{{namespace}}` lists child pages, and `{{video}}` embeds the player. (No interactive
  graph view yet.) See the live **[Feature showcase](../website/demo/)** demo page.
- **Export a page to PDF** — right-click a page title → **Export to PDF…** (or the
  **Export current page to PDF…** command). A small dialog offers **collapsed blocks:
  expand / keep folded**, **font size**, and **margins**; then Tine renders the *whole*
  page (not just the on-screen blocks — the editor virtualizes long pages) to a
  self-contained document with images inlined, using the **same parser + renderer as
  the app and the HTML export**, and opens your OS print dialog so you pick **Save as
  PDF**. The document always prints on a **light** background (printable, whatever your
  theme) with font ligatures off (so `->`/`--` stay literal), and drops the on-screen
  bullet rails. No extra dependency — it reuses the HTML export and the webview's own
  print engine. (Matches what the Logseq PDF-export community plugin did; OG has no
  native PDF export.)
- **Copy/export as** Markdown for a block subtree or a whole page, with a *Rendered*
  mode that flattens to what you see. Rendered copy keeps math delimiters (`$…$`,
  `$$…$$`) so pasted math is re-parseable, pre-warms off-screen `((block ref))`
  targets before copy, and resolves user macros plus provider macros with sensible
  text forms: `{{embed}}` inlines the target, `{{query}}` emits a capped result list
  with visible truncation, and media macros such as `{{video}}` copy the URL. A
  **Resolve refs fully** export toggle expands multi-line block refs; full
  math-typeset-to-plain-text remains on the backlog.
- A slash menu for headings, code, calculator, quote, callouts, divider, embed, query
  (raw or visual builder), template, asset upload, and dates.

<p align="center">
  <img src="img/pdf.png" alt="PDF text + area (image) highlighting with a notes page" width="49%">
  <img src="img/settings.png" alt="Remappable shortcuts" width="49%">
</p>

<p align="center">
  <img src="img/dim.png" alt="Dim inactive blocks — spotlight the one you're working on" width="32%">
  <img src="img/carry.png" alt="Carry unfinished tasks forward to today" width="32%">
  <img src="img/query.png" alt="Query results + the visual query builder chip bar" width="32%">
</p>
