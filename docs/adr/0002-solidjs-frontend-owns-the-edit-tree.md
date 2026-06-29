# 0002. SolidJS, and the frontend owns the live edit tree

- **Status:** Accepted
- **Date:** 2026-06-29

## Context

An outliner is a latency-critical text editor: every keystroke, indent, and caret
move must feel instant. Two questions drove the frontend design. (1) What renders
the UI — a virtual-DOM framework (React-style, diff-on-every-change) or fine-grained
reactivity (compile-time-tracked signals, no diff)? (2) Where does the authoritative
editing state live — in the Rust backend (round-trip each keystroke over IPC) or in
the frontend?

## Decision

We will use **SolidJS** (fine-grained reactivity, no virtual-DOM diffing) for the
frontend, and the **frontend owns the live block tree** as a normalized store.
Keystrokes mutate the in-frontend tree directly and never round-trip to Rust; the
backend receives **debounced, format-preserving saves**, and serves whole-graph
reads (search, backlinks, queries) from its in-memory index.

## Consequences

- **Easier:** typing latency is bounded by local DOM work, not IPC; no vDOM churn on
  large pages; per-keystroke work stays block-local.
- **Harder:** there are now **two representations of a page** — the frontend store
  and the backend cache — and keeping them coherent is a recurring source of
  data-safety bugs (dirty-state races, watcher reloads clobbering unsaved edits,
  graph-switch windows). The data-safety invariants in
  [0007](0007-data-safety-invariants.md) exist largely to manage this boundary.
- **Committed to:** saves are debounced and must be format-preserving and conflict-
  checked; the frontend, not Rust, is the source of truth *while editing*, so flush
  ordering matters before any backend operation (rename, highlight write, graph
  switch).
