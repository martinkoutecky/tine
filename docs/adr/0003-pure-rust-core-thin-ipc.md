# 0003. A pure-Rust core (`tine-core`) behind a thin IPC layer

- **Status:** Accepted
- **Date:** 2026-06-29

## Context

Parsing, serialization, the document model, indexing, queries, ref extraction,
dates, EDN/config, PDF-annotation storage, and HTML publishing are the substance of
the app and must be correct, fast, and testable without a GUI. They could live in
the frontend (TypeScript, easy to call, but slow and hard to test in isolation) or
in a Rust core behind Tauri IPC.

## Decision

We will put all of that in a **GUI-free pure-Rust crate, `crates/tine-core`**,
unit-tested in isolation, and expose it through a **thin set of Tauri IPC commands**
(~41) that do little beyond marshalling. The Tauri layer holds the in-memory page
cache (`RwLock<Arc<Graph>>` — read commands clone the `Arc` and release the lock
immediately), keyed by a graph generation counter.

## Consequences

- **Easier:** the hard logic is testable with plain `cargo test` and a real-graph
  round-trip harness (`tine-check`, a privacy-safe profiler that proves byte-faithful
  serialization without reading note content); the IPC surface stays small.
- **Harder:** anything the frontend needs from the core crosses an async IPC
  boundary, which shaped later decisions — notably that *rendering* a block from the
  core was too slow/async for live editing, leading to in-browser WASM parsing
  ([0006](0006-in-browser-wasm-parsing.md)).
- **Committed to:** the core stays GUI-free and the IPC layer stays thin; business
  logic does not leak into `src-tauri` command handlers or the frontend.
