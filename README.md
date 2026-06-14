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

## Status

See `look-at-og-this-toasty-ripple.md` (plan) for milestones M0–M6. Currently: Rust core
parse/serialize round-trip validated against the real OG shui-graph (0 structural bugs).
