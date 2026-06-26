<p align="center">
  <img src="docs/logo.svg" alt="Tine" height="84">
</p>

<p align="center">
  <b>A fast, local, Logseq-compatible outliner.</b><br>
  Reads and writes the <i>same</i> markdown graph as Logseq — swap between the two on the same files.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white" alt="Tauri 2">
  <img src="https://img.shields.io/badge/SolidJS-1.9-2C4F7C?logo=solid&logoColor=white" alt="SolidJS">
  <img src="https://img.shields.io/badge/Rust-2021-000000?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/platform-Linux%20(WebKitGTK)-555" alt="Linux">
  <img src="https://img.shields.io/badge/license-AGPL--3.0-blue" alt="AGPL-3.0">
</p>

<p align="center">
  <img src="docs/img/hero.png" alt="Tine — journals view" width="820">
</p>

---

## What is Tine?

Tine is a desktop outliner built to look and feel like [Logseq](https://logseq.com) while being
much faster. It operates directly on the standard Logseq graph layout —
`journals/`, `pages/`, `assets/`, and `logseq/config.edn` — so you can point it at the graph you
already use and keep editing in either app (one at a time). Files are written back in
Logseq-compatible markdown, so there's **no import/export step and no lock-in**.

**Why build it?** Logseq's UI is Electron + DataScript with heavy re-rendering, and it gets
sluggish on large graphs. Tine is a ground-up rewrite: a small native shell (Tauri/WebKitGTK), a
pure-Rust core for parsing and indexing, and a fine-grained reactive frontend (SolidJS) that never
diffs a virtual DOM. The editor keeps the live block tree in the frontend, so keystrokes never
round-trip to Rust, and whole-graph reads (search, backlinks, queries) hit an in-memory cache
instead of re-parsing.

> **Status:** a usable daily-driver for outlining, linking, tasks, journals, search, queries, and
> PDF annotation. Linux is the primary, best-tested platform; macOS and Windows builds are produced
> too but are newer. Not yet 1.0 — see [Roadmap](#roadmap--non-goals).

---

## Install

Grab a prebuilt installer from the **[Releases](https://github.com/martinkoutecky/tine/releases)**
page. The builds aren't code-signed yet, so your OS may warn the first time — here's how to get past
it:

- **Linux** — the **AppImage** runs on any distro with no install: `chmod +x Tine_*.AppImage`, then
  run it. Or use the **`.deb`** (Debian/Ubuntu) or **`.rpm`** (Fedora/openSUSE).
- **macOS** — open the **`.dmg`**; on first launch macOS says *"unidentified developer"*, so
  **right-click the app → Open** (just once) and it opens normally after that.
- **Windows** — run the **`.exe`**; if SmartScreen appears, click **More info → Run anyway**.

(Want to hack on Tine instead? Build from source — see [Build & run](#build--run).)

---

## What Tine adds on top of Logseq

These are the things that started as *"I wish Logseq did this"* — Tine's reasons to exist beyond
raw speed. (Where a comparison is made, it's against current Logseq desktop core, no plugins.)

- **⚡ Native speed.** Pure-Rust core + SolidJS fine-grained reactivity (no virtual-DOM diffing) +
  a tiny Tauri/WebKitGTK runtime instead of Electron. Typing stays in the frontend tree; reads hit
  an in-memory index.
- **🗂️ Built-in tabs.** Middle-click any bullet, page title, query result, or switcher row to open
  it in a background tab; pin (persisted), drag-reorder, `Mod+W` to close. Logseq core has no tabs
  (only a right sidebar or a whole separate window).
- **⏯️ Browser-style back/forward**, bound to **`Alt+Left` / `Alt+Right`** by default (per-tab
  history; works mid-edit). *(Logseq has back/forward too, but defaults to `Ctrl/Cmd+[` and `]`.)*
- **🎯 Focus mode + dim-inactive-blocks** (`t f` / `t b`): hide the chrome and fade everything but
  the block you're working on, with Logseq-style layered `Esc` (editing → block-select → exit
  focus). No Logseq-core equivalent.
- **⚡ Global quick-capture** — bind `tine --capture` to a desktop hotkey and a small always-on-top
  box pops from *any* app, with the full editor (autocomplete, slash commands, the date picker,
  nested blocks), and files a bullet to today's journal. Logseq's quick-add only works when the app
  is already focused.
- **🔁 Carry unfinished tasks forward** to today (presets for the last 7 / 30 / 365 days or a
  configurable N), optionally keeping ancestor context.
- **🛟 A real data-safety story** (see below) — conflict detection instead of silent overwrites,
  launch snapshots with one-click restore, and delete-to-trash. Built to live safely on a graph
  you also edit from Logseq mobile over Syncthing.

<p align="center">
  <img src="docs/img/quick-capture.png" alt="Global quick-capture mini-window" width="32%">
  <img src="docs/img/focus-dim.png" alt="Focus mode with inactive blocks dimmed" width="32%">
  <img src="docs/img/tabs.png" alt="Built-in tabs" width="32%">
</p>

---

## Features

**Outliner**
- Click-to-edit blocks; the click lands the caret exactly where you clicked. `Enter` / `Tab` /
  `Shift+Tab` / `Backspace` / arrows with correct Logseq semantics and caret preservation (no
  reflow on indent/outdent); arrow nav respects *visual* wrapped rows.
- Collapse/expand, zoom into a block (with breadcrumb), drag-to-reorder, move up/down
  (`Alt+Shift+↑/↓`), multi-block selection → move / indent / cut / copy.
- Multi-line blocks, syntax-highlighted code blocks, markdown tables; paste an indented outline →
  real block tree; paste clipboard images → graph assets.
- **Media** — paste/import images, **video, and audio** (`/upload`); stored as `![](../assets/…)`
  with a `name_yyyymmdd_hhmmss` filename (keeps the origin + a sortable insert time). Video/audio
  play inline where the codec is supported, else fall back to a click-to-open chip that launches the
  OS default player. **Orphaned-media cleanup** (Settings → Backups): scan for `assets/` files no
  block references and move them to the recoverable trash — deleting a block never deletes its media,
  so this is how unused files get reclaimed.
- Inline formatting (`Mod+B/I`, strike, ==highlight==, link) with a floating selection toolbar, plus
  Emacs-style word/line kill motions.
- **Callouts/admonitions** — both Obsidian-style `> [!note] …` and org `#+BEGIN_NOTE … #+END_NOTE`
  blocks render as colored callouts (`QUOTE` stays a plain blockquote) — and a live **`/calc`
  block** that evaluates arithmetic as you type (`+ - * / ^ %`, parentheses, `name = expr`
  variables across lines, a running result).

**Linking, references & queries**
- `[[page]]`, `#tags`, `#[[multi word]]`, `((block ref))`, `{{embed}}` — all clickable, with
  autocomplete on `[[`, `#`, `((`, and `/`. The `((` popup full-text-searches blocks and inserts a
  *durable* reference (writes a stable `id::` first).
- Linked & unlinked references on every page (live/editable), with co-reference filtering and hover
  previews.
- `{{query}}` engine (inline or whole-block): boolean and/or/not, `(task …)`, `(priority …)`,
  `(property …)`, `(page-property …)`, `(page-tags …)`, `(scheduled)`, `(deadline)`, `(journal)`,
  `(namespace …)`, `(between START END)` with a field selector, `(sort-by …)`. Results render as a
  list or a sortable **table**; an interactive **visual query builder** (chip/clause bar) builds them
  without writing the DSL.
- A scoped compatibility path for Logseq's **advanced (Datalog) queries**: recognized clauses
  (`task`, `between`, `property`, `page-property`, `priority`, page-refs, boolean `or/and/not`, and
  `:today`/`:current-page`-style inputs) map onto the same engine, and any unsupported part is
  flagged in the result rather than silently dropped or wrongly answered.

**Tasks, journals & dates**
- `TODO/DOING/DONE/NOW/LATER/WAITING/CANCELED`, two configurable workflows, priorities, cycle with
  `Mod+Enter`.
- **In-block markdown lists & checklists** — a `+`/`*`/ordered list *inside one bullet's content*
  renders as a styled list (distinct from outline bullets), with tickable `[ ]`/`[x]` checkboxes that
  are *not* TODO/agenda tasks. Uses `+` (OG's in-content marker), so a checklist round-trips to OG and
  Logseq mobile. (`-` stays the outline bullet, exactly like OG.)
- `SCHEDULED:` / `DEADLINE:` via a calendar **date picker** (`/scheduled`, `/deadline`), including
  **recurring tasks** (`+1w` / `.+1w` / `++1w`) where completing a repeater advances the date.
- Multi-day **journal feed** (one continuous editable list); today's journal created lazily on
  first edit; move blocks across days; an **agenda** of *open* scheduled/deadline items (DONE and
  CANCELED are hidden, like OG) in a configurable look-back/-ahead window; journal **templates**; a
  calendar with content markers whose **first day
  of week** follows your `config.edn :start-of-week` (any of the seven days, settable in-app).

**PDF annotation**
- Open PDFs in a resizable, zoomable pane (instant zoom, HiDPI, per-page virtualization); in-PDF
  `Ctrl+F` find with a page jump box.
- Select text → colored **highlights**, or drag a rectangle (area mode / `Ctrl`-drag) to clip an
  **area (image) highlight** — both stored Logseq-compatibly (`assets/<key>.edn` + `hls__` pages,
  area crops as PNG assets). Each highlight becomes a clean bullet you can nest notes under; writes
  **merge with disk** so an externally-added highlight or your top-level notes are never dropped, and
  recoloring a highlight updates its note-page badge to match.

**Search & navigation**
- `Ctrl+K` quick switcher: page titles + full-text content hits (visible text only — no false hits
  on hidden properties/uuids), with block breadcrumbs and middle-click → background tab.
- Command palette (`Mod+Shift+P`), favorites, recent pages, a collapsible **namespace tree** in the
  sidebar, and read-only **"aka" alias chips** on pages reachable by another name.
- **Page rename** (double-click a title) rewrites every `[[ref]]`/`#tag` across the graph in one
  transaction (see data-safety, below).

**Works with your existing setup**
- **Edit safely alongside Logseq mobile over Syncthing.** A filesystem watcher — **inotify by
  default** (zero idle wakeups), with a polling fallback for filesystems where inotify misses edits,
  switchable in Settings — reconciles changes synced in from other devices, and Tine **never
  silently overwrites a file that changed on disk — it surfaces a conflict** instead. Saves preserve
  each file's exact formatting (tabs vs spaces, comments, compact EDN) and skip byte-identical
  rewrites, so they don't create sync diff churn.
- **Page rename is transactional** — the page move and every `[[ref]]`/`#tag` rewrite across the
  graph commit all-or-nothing, re-checking each file just before writing and rolling back on
  conflict, so a rename can't half-apply.
- **Custom journal date formats** — reads `:journal/file-name-format` and `:journal/page-title-format`
  and recognizes/creates journal files in your format (e.g. `dd-MM-yyyy`, `yyyy-MM-dd`, `yyyyMMdd`),
  falling back to the defaults so old/foreign files still resolve. The display-title format is
  pickable in Settings → *Journals & tasks* → **Journal date format** (file names are left as-is).
- **Duplicate-day reconcile** — if two files ever resolve to the same day (e.g. a `2026_06_26.org`
  plus a title-named `Friday, 26-06-2026.org` left over from a date-format change), Tine keeps
  **both** rather than silently dropping one, and Settings → *Backups* → **Duplicate journal days**
  lets you reach each file individually: **Open** it (editable, saves back to itself), **Merge** a
  stray into the canonical day, **Rename** it to a normal page, or **Trash** the redundant one.
- **Org-mode graphs** — opens, renders, and edits `.org` pages and journals (headlines as blocks,
  org inline syntax `*bold*` `/italic/` `_underline_` `~code~` `[[target][desc]]`, TODO markers,
  `#+BEGIN_SRC`/`QUOTE` blocks). Mixed `.md` + `.org` graphs work; the **File format** setting
  (Settings → *Journals & tasks*, `:preferred-format`) chooses what new pages/journals use. A `.org` file is only ever rewritten
  when Tine can reproduce it **byte-for-byte** — anything it can't round-trip loads **read-only**, so
  it can never corrupt an org graph.
- **Launch snapshots** (configurable keep-count) with a restore UI that takes a safety snapshot
  first; page delete moves to a recoverable **trash**; `atomic_write` + fsync.
- Open/switch graphs from the app (native folder picker) or via `TINE_GRAPH`.

**Customization & output**
- **Fully remappable keyboard shortcuts** — in the Settings modal or via `config.edn :shortcuts`.
- Light/dark themes, accent color, custom CSS, wide mode (`t w`) and document mode (`t d`).
- One-click **static HTML export** (`public:: true` pages); **"copy/export as"** for a block subtree
  or page as Markdown; a slash menu for headings, code, calculator, quote, callouts, divider, embed,
  query (raw or visual builder), template, asset upload, and dates.

<p align="center">
  <img src="docs/img/pdf.png" alt="PDF text + area (image) highlighting with a notes page" width="49%">
  <img src="docs/img/settings.png" alt="Remappable shortcuts" width="49%">
</p>

<p align="center">
  <img src="docs/img/dim.png" alt="Dim inactive blocks — spotlight the one you're working on" width="32%">
  <img src="docs/img/carry.png" alt="Carry unfinished tasks forward to today" width="32%">
  <img src="docs/img/query.png" alt="Query results + the visual query builder chip bar" width="32%">
</p>

---

## Built with

| Layer | Tech | Notes |
|------|------|-------|
| Shell | [Tauri 2](https://tauri.app) (Rust) | OS webview (WebKitGTK on Linux) — tiny runtime vs Electron |
| Frontend | [SolidJS](https://solidjs.com) + TypeScript + [Vite](https://vitejs.dev) | fine-grained reactivity, no virtual-DOM churn |
| Core | `crates/tine-core` (pure Rust) | parse/serialize, model, indexing, queries, refs, dates, PDF/EDN, HTML publish |
| Rendering | [pdf.js](https://mozilla.github.io/pdf.js/), [KaTeX](https://katex.org), highlight.js | PDF, math, code |

The Rust core is GUI-free and unit-tested in isolation; the Tauri layer is a thin set of ~41 IPC
commands over it. The frontend owns the live editing tree (normalized store) and pushes debounced,
format-preserving saves; whole-graph reads hit an in-memory page cache (`RwLock<Arc<Graph>>` — read
commands clone the Arc and release the lock immediately) keyed by a graph generation counter.

## Project layout

```
crates/tine-core/    Rust core: parse/serialize, model, config, dates, refs, query, pdf, edn, publish
src-tauri/           Tauri app — IPC commands + windows (main + quick-capture)
src/                 SolidJS frontend (components, store, render pipeline, keybindings)
scripts/             env.sh (toolchain paths), screenshot generators
docs/                Logo, images, feature notes
samples/             Demo graph used by tests/screenshots
```

## Build & run

```bash
source scripts/env.sh        # toolchain env (CARGO_HOME/RUSTUP_HOME, lib paths)
npm install                  # first time

# Build the release binary (NOT plain `cargo build` — that produces a dev-mode
# binary that can't connect to the bundled frontend):
npx tauri build --no-bundle

# Run it against your graph:
TINE_GRAPH=/path/to/your/graph ./target/release/tine
```

- Point `TINE_GRAPH` at the same `journals/` + `pages/` + `logseq/config.edn` tree you use with
  Logseq. **Run one app at a time** on a given graph.
- **GPU compositing (smooth scrolling) is on by default.** On the rare GPU/compositor combo where
  WebKitGTK's DMABUF renderer aborts (the window fails to appear, or you see
  `EGL_BAD_PARAMETER` on the console), set `TINE_GPU=0` to fall back to software rendering — slower,
  but it always starts. If Tine detects it's painting on the CPU it now says so with a banner.
- Prefer the **raw binary** over an AppImage on Linux — an AppImage's bundled graphics libraries can
  clash with the host GPU and silently drop you to (slow) software rendering. The `.deb`/`.rpm`
  packages use your system's drivers and don't have this problem.

**Global quick-capture:** bind your desktop environment's keyboard settings to run
`tine --capture` (a second launch is routed to the running instance via single-instance).

### Develop

```bash
npm run dev                  # frontend only, in a browser, against an in-memory mock backend
npm run app                  # full Tauri dev window  (alias for: tauri dev)
node scripts/screenshot.mjs  # regenerate screenshots from the mock backend
```

## Testing

```bash
source scripts/env.sh
cargo test -p tine-core      # Rust: parse/serialize round-trip, model, queries, search cache
npm test                     # Frontend: Vitest (editor ops, outline, autocomplete, markers, …)
```

Round-trip parsing is validated against a real Logseq graph (0 structural diffs beyond accepted
canonicalization); `tine-check` is a privacy-safe profiler that proves byte-faithful serialization
without reading note content.

## Roadmap & non-goals

**Planned / under evaluation:** graph view, configurable typographic auto-replace, and **broader
coverage of advanced Datalog queries** (a scoped subset works today — see above).

**Out of scope (by design):** whiteboards, flashcards, the plugin system, built-in git, and a
native mobile app — Tine coexists with Logseq mobile over your own sync instead of replacing it.

## Acknowledgements

Tine is an independent reimplementation, not a fork — the codebase is original Rust + SolidJS and
contains no Logseq source. It does target Logseq's on-disk format and adapts parts of Logseq's
outliner CSS (variables and bullet/indent rules), so it is a derivative work for licensing purposes
and is released under the same license.

[Logseq](https://github.com/logseq/logseq) is © its authors, licensed AGPL-3.0. Tine is **not
affiliated with or endorsed by Logseq.** Thanks to the Logseq project for the format and the design
it pioneered.

## License

[GNU AGPL-3.0-only](LICENSE).

Copyright (C) 2026 Martin Koutecký.

This program is free software: you can redistribute it and/or modify it under the terms of the GNU
Affero General Public License as published by the Free Software Foundation, version 3. It is
distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied
warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the [LICENSE](LICENSE) for
details.

---

<sub>Built ground-up as a faster, file-compatible alternative to Logseq. Not affiliated with Logseq.</sub>
