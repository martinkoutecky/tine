# logseq-claude

A fast, near-identical clone of [Logseq](https://logseq.com) that operates on the
same standard markdown graph (`journals/` + `pages/` + `logseq/config.edn`), so you
can swap between OG Logseq and this app on the same files (one at a time).

**Why:** OG Logseq's UI is slow (Electron + DataScript + heavy re-rendering). This is a
ground-up rewrite focused on speed and visual/functional parity.

## Stack

- **Shell:** [Tauri 2](https://tauri.app) (Rust) — OS webview, tiny runtime vs Electron.
- **Frontend:** SolidJS + TypeScript + Vite — fine-grained reactivity, no virtual-DOM churn.
- **Core:** `crates/logseq-core` — pure Rust: parsing, Logseq-compatible serialization,
  the graph model, indexing, queries. No GUI deps, fully unit-testable.

## Layout

```
crates/logseq-core/   Rust core (parse/serialize/model) — testable standalone
src-tauri/            Tauri app: IPC commands + window (added in M0)
src/                  SolidJS frontend
scripts/env.sh        Sets CARGO_HOME/RUSTUP_HOME to the persistent toolchain mount
```

## Development

```bash
source scripts/env.sh          # Rust toolchain lives on the persistent /aux mount
cargo test -p logseq-core      # core unit + round-trip tests

# Round-trip any real graph (reports structural bugs vs acceptable canonicalization):
cargo run -p logseq-core --example roundtrip_dir -- /path/to/graph
```

## Running it on a real graph

```bash
source scripts/env.sh
npm install                       # first time
cargo build --release -p logseq-claude    # or: npm run app  (tauri dev)
LOGSEQ_CLAUDE_GRAPH=/path/to/your/graph ./target/release/logseq-claude
```

Point it at the same `journals/`+`pages/`+`logseq/config.edn` tree you use with OG
Logseq (one app open at a time). Files are written back in Logseq-compatible form.

## Status

Working: outliner editing (click-to-edit, Enter/Tab/Shift-Tab/Backspace/arrows, caret
preserved), inline rendering (bold/italic/code/strike/highlight, headings, `[[links]]`,
`#tags`, `((block refs))`, `{{embed}}`, KaTeX math), `{{query}}` (boolean/task/property
subset), properties, tasks, Linked References, full-text search, Ctrl-K quick switcher,
`[[`/`#`/`/` autocomplete, multi-day journals feed, configurable shortcuts
(config.edn `:shortcuts`), light/dark themes, and PDF viewing + text highlighting
(stored OG-compatibly as `assets/<key>.edn` + `hls__` pages).

Tests: 33 frontend (Vitest) + 34 Rust. Round-trip validated against the real OG
shui-graph (0 structural bugs).

Not yet: block-move shortcuts remapping for in-editor keys, area (image) PDF highlights,
right sidebar, graph view. Whiteboards/flashcards intentionally out of scope.
