# Split view — implementation spec (Round 5)

Status: ACTIVE (Jul 8 2026). Parent design: `docs/breadth-grid-spec.md` §10 v3
(`leafKind: pane`). Grounding: `subagent-tasks/notes/split-view-architecture.md`
(file:line map of every touched seam — read it before implementing),
`brainstorm-obsidian-splits.md` (the 80/20 + Obsidian's unbound-keys mistake),
`brainstorm-split-libraries.md` (build bespoke; pointer events only, NEVER
HTML5 DnD).

The ask (Martin, Jul 8 2026): split screen **with the grid nav model** (à la
TreeSheets) — panes are peers, seams are selectable, **typing at an edge/seam
opens a new split**. This is the Sheets select→edit ladder with panes as the
top rung, not just a two-pane toggle.

## 1. Model

- **Pane split-tree** (react-mosaic-shaped, data only — no library):
  `LayoutNode = { kind: "split", dir: "row" | "col", ratio: number,
  children: [LayoutNode, LayoutNode] } | { kind: "pane", paneId: string }`.
  Binary tree; arbitrary nesting falls out. `ratio` ∈ (0.1, 0.9) = first
  child's share. Root starts as a single pane leaf.
- **Each pane = a full router instance.** `createPaneRouter()` factory owns
  what `src/router.ts` module state owns today: `tabs`, `activeId`,
  `closedTabs`, `scrollByRoute`, back/forward. Zoom stays a route field, so
  zoom is per-pane for free.
- **Focused pane** = generalized `activePane` (`ui.ts:238-254`):
  `installPaneTracker` matches `closest("[data-pane-id]")` on capture-phase
  pointerdown/focusin. The existing router exports (`openPage`, `route()`,
  `goBack`, …) remain as **shims delegating to the focused pane's router** —
  correct for all pointer + keyboard paths because the tracker fires before
  click handlers (study §1b). Non-interactive callers (file watcher, carry,
  ContextMenu delete-fallback, graph switch, session restore) take explicit
  pane handles / iterate panes — they may NOT use the shims.
- **Feed pane vs page panes (v1 restriction, ADR 0032):** the journals feed
  (`doc.feed`) stays a singleton owned by AT MOST ONE pane at a time — the
  **feed pane**. Other panes are **page panes**: single-page routes loaded via
  the satellite path (`ensurePageLoaded`, as RightSidebar does — never
  `loadSingle`/`loadFeed`, so they don't clobber the feed or `endEdit` other
  panes). Opening journals in a second pane focuses the pane already showing
  them (dedup), it does not create a second feed. Full feed decoupling = v2
  if ever wanted.
- **Editing singleton unchanged:** one caret app-wide (`editorController.ts`).
  Pane surfaces mint `SurfaceContext = "pane:{paneId}"`; the feed pane keeps
  `"main"`. Revisit the `ref:`-based "primary surface" rule (`Block.tsx:268-276`)
  so pane surfaces count as primary.
- Desktop-only: on mobile the layout is forced to a single pane (tab strip is
  already hidden there; the hardware-back bridge binds to the focused pane).
- PDF pane + right sidebar stay docked chrome OUTSIDE the pane tree (v1).
  They are not pane leaves; "PDF as a pane" is deferred.

## 2. Rendering + chrome

- `App.tsx`: the single `<main class="main-content">` becomes a recursive
  `<PaneTree>` renderer over the layout tree — splits = nested flex
  containers with `flex-grow` per ratio; leaves = `<Pane>` =
  `[data-pane-id]` wrapper → per-pane `TabBar` strip → per-pane scroller →
  `PageView`.
- `PageView` takes its route from `PaneContext` (Solid context providing the
  pane's router), not the global `route()`. The feed pane renders exactly what
  PageView renders today; page panes render the `PageSection` chrome (title,
  blocks, linked refs) for one page.
- Per-pane scroller replaces the hardcoded `mainScroller()`
  (`router.ts:170-173`); scroll restoration and InPageFind become pane-scoped.
  Lenis smooth-scroll binds per scroller (or is v1-disabled inside split
  layouts if binding N instances misbehaves — measure, don't guess).
- `TabBar` is parametrized by its pane's router (no module-global reads).
  Only the TOP strip region in the topbar remains `data-tauri-drag-region`;
  pane strips do not drag the window. Single-pane layout must look ~identical
  to today.
- Seam between panes = the shared edge: a thin hover-highlight resizer
  (pointer drag writes `ratio`, same ~12-line pattern as the three existing
  resizers) that is ALSO a selectable nav target (§3).
- The file-watcher subscription hoists OUT of PageView into one app-level
  subscriber that iterates panes (delete-fallback: every pane showing the
  deleted page falls back, not just the focused one). Working-set eviction
  pins every pane's visible pages (`store.ts:352-363`).

## 3. The grid nav model (what makes this THE feature)

Pane-select mode = the top rung of the existing select→edit ladder:

- **Enter the rung:** `Esc` from the outline's top level (block-selection at
  page root) enters pane-select mode; the focused pane shows a selection ring.
  `Enter`/`→`-equivalent descends back into the pane's content.
- **Mode affordance (Jul 8, from Martin's field report):** because the same
  `Esc` both enters and exits the rung, "press Esc repeatedly" leaves the user
  on an unknown side of the toggle with arrows silently dead. While the mode is
  active a **hint pill** (bottom-center: arrows/Enter/type/Esc legend, target-
  aware) is shown and the targeted pane is **tinted** (the bare 2px ring reads
  as the focused-pane indicator). The mode is also exposed as a **"Pane select
  mode" command** in the palette.
- **Arrows** move pane selection spatially (left/right/up/down across the
  tree, geometric nearest-neighbor like the sheet's 2-D stepping). **Seam
  stepping default ON** (as in Sheets): arrows also land on seams between
  panes and on the window edges (virtual outer seams).
- **Typing on a seam/edge materializes a split there** — the pane splits at
  that seam, the new pane opens with the QuickSwitcher prefilled with what
  was typed; picking a page lands it in the new pane; `Esc` cancels and
  unsplits. This is the sheet seam-insert gesture at pane scale.
- **Enter on a seam** = same materialization with an empty QuickSwitcher.
- **Default-bound keys** (Obsidian's unbound-by-default mistake is the
  anti-pattern): `Ctrl/Cmd+1..9` focus pane N (spatial reading order);
  focus-direction keys (`Ctrl+Alt+←/→/↑/↓`); "move current tab to next pane"
  (`Ctrl+Alt+Shift+→` etc.); "split right"/"split down"/"close pane"
  commands in the palette with bindings. Exact chords: pick ones free in
  `keybindings.ts`, record in the ADR.
- **`Ctrl/Cmd+click` on a `[[link]]`/ref = open in other pane**, creating the
  split (right) if none exists — the IDE move. Middle-click stays "new tab in
  this pane".
- Block multi-selection never crosses a pane boundary (drag is scoped to the
  origin pane; pane switch clears block selection).
- Closing a pane's last tab closes the pane (collapsing its split); the last
  remaining pane can never be closed (same invariant as the last tab today).

## 3a. Open-target rules (Martin's Jul 8 usability questions — rulings)

- **No pane is ever born empty.** Every creation gesture carries its own
  content: the explicit "split right/down" command **duplicates the current
  tab** (page + zoom + scroll position) into the new pane — the
  VSCode/Obsidian convention, and two live surfaces of one page already work
  (sidebar precedent). You split to *keep something visible*, then navigate
  one side away. Type-on-seam/Enter-on-seam creates an embryo pane whose
  content is chosen in the prefilled QuickSwitcher (Esc cancels and unsplits,
  so no empty pane can result). Ctrl+click carries the clicked target; tab
  drag carries the dragged tab.
- **The switcher (Ctrl+K) acts on the focused pane.** One app-modal palette,
  centered, as today — not per-pane. Enter opens in the focused pane
  (respecting the tab-reuse setting, exactly today's semantics); the existing
  new-tab modifier keeps meaning "new tab in the focused pane";
  **Alt+Enter = open in the OTHER pane**, creating a split (right) if none
  exists — quick-open's "open to the side". Corollary requirement: a
  **visible focused-pane indicator** whenever more than one pane exists
  (subtle accent on the pane's tab strip or a 1px ring), so "where will this
  open" is always legible at a glance. Single-pane layout shows no indicator
  (zero visual change from today).
- **A page moves between splits by dragging its TAB** (the page's handle —
  S4): drop on another pane's strip = move it there at the drop position;
  drop on a seam or pane edge = split there and move (with a half-pane
  highlight preview while hovering). Keyboard equivalent ("move current tab
  to next pane") is default-bound per §3. Moving a pane's last tab out
  collapses that pane. In-content drags stay pane-local in v1: block drag
  keeps its reorder meaning and never becomes "open here" across a boundary
  (mis-drop risk), link drag is not a gesture.

## 4. Persistence

Extend `PersistedSession` (`router.ts:503-516`) → same `tine-session.json`,
same atomic-rename backend commands: `layout` = the split tree with each leaf
carrying `{tabs, activeIndex, scrolls}` (today's fields, nested); plus
`focusedPaneId`. `parseSession` keeps back-compat: an old flat session parses
into a single-leaf layout; a new file read by an old build must not crash it
(keep the old top-level fields mirrored from the focused pane — cheap and
safe). Layout is per-machine state: session file, never config.edn.

## 5. Phasing (each phase = gated codex batch(es), committed + pushed)

- **S1 — router extraction (pure refactor, zero behavior change).**
  `createPaneRouter()` factory + pane registry (`src/panes.ts`: layout tree
  signal, focusedPaneId, PaneContext) with root = one leaf; shims in place;
  PageView reads route via context; TabBar parametrized; pane tracker
  generalized to `[data-pane-id]`. Gates: full suites + e2e-sheets 50/50 +
  session round-trip (old file loads, new file loads in old shape). VERIFY
  FIRST in the real app: capture-phase pointerdown precedes `auxclick`
  handlers in WebKitGTK (study §1b caveat) — a 5-line probe, before building
  on the shim assumption.
- **S2 — two panes work.** PaneTree renderer + seam resizer; page-pane
  satellite loading; per-pane scroll/strip; split/close/open-in-other-pane
  commands (split duplicates the current tab, §3a) + Ctrl+click;
  focused-pane indicator; watcher hoist; eviction pins; session layout schema.
  Gates: suites + new render tests + a real-app two-pane e2e probe
  (edit in A while B shows the same page; navigate B, A's editor keeps the
  caret; session survives restart).
- **S3 — the nav model.** Pane-select rung + seam stepping + type-on-seam
  materialization + QuickSwitcher prefill + switcher Alt+Enter open-in-other-
  pane (§3a) + all default bindings + selection scoping. Gates: suites + e2e keyboard walk (Esc ladder up, arrow to seam,
  type, pick page, Esc unsplit).
- **S4 — tab drag + polish.** TabBar reorder converted HTML5 DnD → pointer
  `beginDrag` pattern; drag-tab-to-seam = split, drag-tab-to-other-strip =
  move; pane-scoped InPageFind; docs (README/FEATURES/CHANGELOG/screenshots),
  demo-page note, BACKLOG sweep.

Deferred past Round 5: N-feed decoupling (two journals panes), PDF/sidebar as
pane leaves, stacked/ephemeral tabs, named layouts, per-pane zoom.

## 6. Non-negotiables (restate in every codex spec)

Pointer events only, never HTML5 DnD (WebKitGTK). No layout libraries. One
caret app-wide. Durable state via the Rust backend, never bare localStorage.
OG round-trip is untouched (layout is workspace state, no file-format
change). Don't touch `docs/plans/theme-gallery.md`, `reddit-backup/`,
`docs/plans/lsdoc-v2-tine-validation.md`. Test corpora: tine-test /
org-graph / kitchen-sink only. Agents never run git ops. Two vitest configs.
Build with `--features custom-protocol`.
