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
</p>

<p align="center">
  <img src="docs/img/hero.png" alt="Tine — journals view" width="820">
</p>

---

## What is Tine?

Tine is a desktop outliner built to look and feel like [Logseq](https://logseq.com) while being
much faster. It operates directly on the standard Logseq graph layout —
`journals/`, `pages/`, and `logseq/config.edn` — so you can point it at the graph you already
use and keep editing in either app (one at a time). Files are written back in
Logseq-compatible markdown, so there's no lock-in and no import/export step.

**Why build it?** Logseq's UI is built on Electron + DataScript with heavy re-rendering, and it
gets sluggish on large graphs. Tine is a ground-up rewrite: a small native shell, a pure-Rust
core for parsing and indexing, and a fine-grained reactive frontend that never diffs a virtual
DOM. The editor keeps the live tree in the frontend so keystrokes never round-trip to Rust.

> **Status:** usable daily-driver for outlining, linking, tasks, search, and PDF annotation.
> Not yet a 1.0 — see [Roadmap](#roadmap--non-goals).

## Features

**Outliner**
- Click-to-edit blocks; `Enter` / `Tab` / `Shift+Tab` / `Backspace` / arrows with correct
  Logseq semantics and caret preservation (no reflow on indent/outdent).
- Collapse/expand, zoom into a block, drag-to-reorder, multi-block selection + move/indent/cut.
- Multi-line blocks, code blocks (syntax-highlighted), markdown tables.

**Linking & references**
- `[[page]]` links, `#tags`, `((block refs))`, `{{embed}}` — all clickable, with `[[` / `#` / `/`
  autocomplete.
- Linked & unlinked references on every page.

**Tasks, properties, queries**
- `TODO/DOING/DONE/NOW/LATER/WAITING/CANCELED`, priorities, `SCHEDULED`/`DEADLINE`.
- Page & block properties.
- `{{query}}` — boolean / task / property / scheduled-deadline filters, rendered as a list or a
  sortable **table** (full datalog is out of scope).

**PDF annotation**
- Open PDFs in a resizable, zoomable pane (buttons + Ctrl/Cmd-scroll).
- Select text → colored highlights, stored Logseq-compatibly (`assets/<key>.edn` + `hls__` pages).
- A highlight becomes a clean bullet on the notes page — press `Enter`/`Tab` to add nested notes;
  the metadata stays hidden. The notes page refreshes as you highlight.

**Search & navigation**
- `Ctrl+K` quick switcher: page titles + full-text content hits.
- Tabs (middle-click to open, double-click to pin), favorites, recent pages, journals feed.

**Customization & output**
- Fully remappable keyboard shortcuts — in the Settings modal or via `config.edn :shortcuts`.
- Light / dark themes.
- One-click **static HTML export** of the whole graph.
- Slash menu: headings, code, quote, divider, embed, scheduled/deadline, asset upload, and more.

<p align="center">
  <img src="docs/img/pdf.png" alt="PDF highlighting with notes" width="49%">
  <img src="docs/img/settings.png" alt="Remappable shortcuts" width="49%">
</p>

## Built with

| Layer | Tech | Notes |
|------|------|-------|
| Shell | [Tauri 2](https://tauri.app) (Rust) | OS webview (WebKitGTK on Linux) — tiny runtime vs Electron |
| Frontend | [SolidJS](https://solidjs.com) + TypeScript + [Vite](https://vitejs.dev) | fine-grained reactivity, no virtual-DOM churn |
| Core | `crates/logseq-core` (pure Rust) | parsing, Logseq-compatible serialization, model, indexing, queries, PDF/EDN, HTML publish |
| Rendering | [pdf.js](https://mozilla.github.io/pdf.js/), [KaTeX](https://katex.org), highlight.js | PDF, math, code |

The Rust core is GUI-free and unit-tested in isolation; the Tauri layer is a thin set of IPC
commands over it. The frontend owns the live editing tree (normalized store) and pushes debounced
saves; whole-graph reads (search, backlinks, queries) hit an in-memory page cache rather than
re-reading the tree.

## Project layout

```
crates/logseq-core/   Rust core: parse/serialize, model, config, dates, refs, query, pdf, edn, publish
src-tauri/            Tauri app — IPC commands + window
src/                  SolidJS frontend (components, store, render pipeline, keybindings)
scripts/env.sh        Points CARGO_HOME/RUSTUP_HOME at the persistent toolchain
docs/                 Feature map, notes, screenshots
samples/              Demo graph used by tests/screenshots
```

## Build & run

```bash
source scripts/env.sh        # toolchain env (CARGO_HOME/RUSTUP_HOME, lib paths)
npm install                  # first time

# Build the release binary (NOT `cargo build` — that produces a dev-mode binary
# that can't connect to the bundled frontend):
npx tauri build --no-bundle

# Run it against your graph:
TINE_GPU=1 TINE_GRAPH=/path/to/your/graph ./target/release/tine
```

- Point `TINE_GRAPH` at the same `journals/` + `pages/` + `logseq/config.edn` tree you use with
  Logseq. Run one app at a time on a given graph.
- `TINE_GPU=1` enables GPU compositing (smooth scrolling). It's off by default because some
  GPU/compositor combos abort WebKitGTK's DMABUF renderer; leave it unset if the window fails to
  appear.
- Prefer the **raw binary** over an AppImage on Linux — a bundled Mesa can clash with the host GPU.

### Develop

```bash
npm run dev                  # frontend only, in a browser, against an in-memory mock backend
npm run app                  # full Tauri dev window  (alias for: tauri dev)
```

## Testing

```bash
source scripts/env.sh
cargo test -p logseq-core    # Rust: parse/serialize round-trip, model, queries, search cache
npm test                     # Frontend: Vitest (editor ops, outline, autocomplete, markers)
```

Round-trip parsing is validated against a real Logseq graph (0 structural diffs beyond accepted
canonicalization).

## Roadmap & non-goals

**Planned:** page rename with ref-update, namespaces/aliases UI, callouts, query sorting via the
DSL, graph view, area (image) PDF highlights.

**Out of scope (by design):** whiteboards, flashcards, the plugin system, full datalog queries,
sync/built-in git, and mobile.

## Acknowledgements

Tine is an independent reimplementation, not a fork — the codebase is original Rust + SolidJS and
contains no Logseq source. It does, however, target Logseq's on-disk format and adapts parts of
Logseq's outliner CSS (variables and bullet/indent rules), so it is a derivative work for
licensing purposes and is released under the same license.

[Logseq](https://github.com/logseq/logseq) is © its authors, licensed AGPL-3.0. Tine is **not
affiliated with or endorsed by Logseq.** Thanks to the Logseq project for the format and the
design it pioneered.

## License

[GNU AGPL-3.0-only](LICENSE).

Copyright (C) 2026 Martin Koutecký.

This program is free software: you can redistribute it and/or modify it under the terms of the GNU
Affero General Public License as published by the Free Software Foundation, version 3. It is
distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied
warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
[LICENSE](LICENSE) for details.

---

<sub>Built ground-up as a faster, file-compatible alternative to Logseq. Not affiliated with Logseq.</sub>
