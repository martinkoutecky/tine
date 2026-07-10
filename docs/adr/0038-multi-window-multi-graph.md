# 0038. Multi-window, multi-graph: one process, per-window graph map

- **Status:** Proposed (Claude, design note for GH #70/#56/#55 — Martin to review)
- **Date:** 2026-07-10

## Context

Three linked feature requests ask Tine to behave like Logseq's multi-graph
workflow: **#55** — view/manage multiple graphs and quick-switch between them
(the OG top-left graph dropdown); **#70** — open a graph in its own window so two
graphs are editable side by side; **#56** — shift-click a graph in that list to
open it in a new window. The reporter's own dependency note is correct: #56 needs
#70, and #70 is the architectural work; #55 is separable and mostly UI.

What the code assumes today (all verified read-only, Jul 10):

- **One graph per process, hard-wired.** The backend holds a single graph:
  `AppState.graph: RwLock<Option<Arc<Graph>>>` (`src-tauri/src/state.rs`). Every
  command resolves it through `with_graph(state)` with no notion of *which* graph
  the caller wants. A switch replaces that one slot, re-warms the whole-graph
  caches (`warm_generation` guards against a stale warm reporting done), and
  re-points the single file-watcher (`src-tauri/src/graph.rs` `load_graph`).
- **Single-instance is active and load-bearing** (`src-tauri/src/lib.rs`, the
  `tauri_plugin_single_instance` init). A second `tine` launch does not spawn a
  process — the plugin routes its argv into the running instance (`--capture` pops
  the capture window; anything else surfaces main). This is the mechanism behind
  quick-capture's single-writer guarantee (one process owns the writes).
- **Only two static windows exist** (`src-tauri/tauri.conf.json`): `main` and the
  hidden `capture` mini-window. No code creates a window dynamically. "Split view"
  is in-webview panes (`src/panes.ts`, ADR 0032), not OS windows.
- **The frontend store is already a per-webview singleton** (module-global SolidJS
  store in `src/store.ts`). A second window therefore gets an independent store for
  free — the hard part is entirely backend-side.
- **No persisted graph list.** Only `tine.graphPath` (the one current path) is kept
  (localStorage, `src/graph.ts`). Switching itself is fully built:
  `loadGraphPath` does a complete workspace reset. `Sidebar.tsx` already carries a
  `GraphSwitcher` dropdown and a comment naming this cluster "R4a".

Two designs were genuinely on the table for #70:

- **Design 1 — multiple windows, one process, a per-window graph map.** The backend
  keys graphs by window; each command resolves *its* window's graph.
- **Design 2 — multiple OS processes (the literal "multiple .exe" ask).** Each
  process stays single-graph, so the backend barely changes — but single-instance
  must be dropped and re-implemented *keyed per graph root* (at most one process per
  graph, else two processes writing the same files = the data-corruption class we
  rank first), plus cross-process "who owns graph X, go focus its window"
  coordination whose absence is the whole reason single-instance exists.

## Decision

We will implement #70 as **Design 1: a single process hosting N top-level windows,
each bound to its own graph, with the backend holding a map of graphs keyed by
window** rather than the current single `Option<Arc<Graph>>` slot.

Concretely:

- `AppState.graph` becomes a per-window registry — conceptually
  `RwLock<HashMap<WindowKey, GraphSlot>>`, where `GraphSlot` carries the
  `Arc<Graph>` plus that graph's own `warm_done` / `warm_generation`. `with_graph`
  gains the calling window's key; commands learn their caller by taking a
  `tauri::WebviewWindow` (or `Window`) parameter and passing `window.label()`.
- The file-watcher becomes multi-root (one watcher covering every open graph's
  dirs, dispatching change events to the owning window) — not one watcher per
  window. Backups and the launch journal-migration continue to run per graph at the
  moment that graph is opened.
- Window creation is dynamic (`WebviewWindowBuilder`). The single-instance handler,
  on `tine <path>` for a graph not already open, **opens a new window bound to that
  graph** (not a new process); on a path already open it focuses that window. This
  keeps single-instance — and its one-writer-process guarantee — intact.
- **#55 ships first and independently, single-graph-at-a-time:** persist a
  known/recent-graphs list (one key in the existing `tine-settings.json`, appended
  on each successful `load_graph`) and render it in the existing `GraphSwitcher`
  dropdown. This needs none of the map refactor above.
- **#56 is the glue on top of #70:** shift-click a row in the #55 list calls the
  "open in new window" path instead of the in-place switch.
- Scope is **desktop only.** Android/iOS are single-activity; multi-window is a
  non-goal there and those targets keep the single-graph slot.

## Consequences

- The blast radius of #70 concentrates in `src-tauri/src/state.rs`,
  `commands.rs` (threading the window key into every `with_graph` call — pervasive
  but mechanical), `watcher.rs` (single-root → multi-root dispatch), and window
  lifecycle in `lib.rs`. The per-webview frontend store means minimal frontend
  churn: each window boots its own store as today.
- **Data safety is preserved by construction.** One process stays the sole writer;
  the per-file save path (temp + fsync + rename + base-rev + lock) is already safe
  for N distinct graphs in one process, and two windows can never point at the same
  graph writing concurrently the way two *processes* could. This is the decisive
  reason Design 1 beats Design 2.
- Design 2 was rejected: it looks cheaper (each process is unchanged) until the
  per-graph instance keying and the focus-an-existing-window IPC — which reconstruct
  exactly what single-instance already gives us — and it reopens the two-writer risk
  the moment the keying is wrong. Its only genuine win (fault isolation between
  graphs) does not outweigh that.
- New commitments: warm-cache generation, the `warm-cache-done` event, and any
  future whole-graph cache all become **per-window-key** rather than global; a
  reviewer must check no command still reads a process-global "current graph".
- Quick-capture needs a routing decision once multiple graphs are open: which
  graph a capture lands in (the last-focused window's graph is the likely answer).
  Recorded as follow-on, not solved here.
- Memory / footprint grows with the number of open graphs (each holds its warm
  caches); acceptable for a deliberate "open a second graph" action, and bounded by
  how many windows the user opens.
- Sequencing note: this is an L-sized desktop-QoL track that competes directly with
  the perf (X5) and lsdoc focus work on the roadmap. #55 (S) can slot in anytime;
  #70/#56 should wait behind the data-safety and perf tracks unless multi-graph
  demand outweighs the 10k-page-graph signal.
