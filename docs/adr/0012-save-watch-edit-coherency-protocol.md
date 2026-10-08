# 0012. The save/watch/edit coherency protocol has one shape — don't add a second

Date: 2026-07-02

## Status

Accepted

## Context

The graph is concurrently edited by Tine, OG Logseq, mobile sync, and Syncthing.
Keeping the frontend tree, the Rust cache, the disk, and the file watcher
coherent is the single most regression-prone protocol in the project's history
(`3d4a6ff`, `130f5b5`, `b23c1d6`, `9a14a37`, …): every past bug in this area came
from a path obeying only *part* of the protocol. Until now the contract lived in
scattered comments across four files; the 2026-07 audit named that the #1
architectural risk.

## Decision

The protocol, and who owns each piece:

- **`src/persistence.ts` is the only writer.** It owns `dirty`, per-page
  `baseRev`, deletion tombstones, the graph token, the debounce, and a per-page
  *serial* save chain. Every frontend mutation reaches disk via
  `markDirty` → `persistence.ts` → `backend.savePage`, never a bespoke write.
- **`reloadDisposition` (`src/store.ts`) is the only reload decision point.**
  Any code reacting to a watcher/external-change event consults it; it protects
  in-flight saves and dirty pages from being clobbered by a reload.
- **Rust `Graph::save_page` → `commit_write` owns disk truth:** per-path lock,
  baseline conflict check + last-moment recheck, atomic temp+fsync+rename, and
  the self-write marker (`note_self_write` records the rev *before* the rename
  lands; the watcher consumes it to recognize Tine's own writes).
- **New mutation paths** (import, migration, capture, rename-cascades, sidecar
  writes, …) must go through these primitives — `save_page`/`commit_write`/
  `atomic_write` on the Rust side, the persistence chain on the frontend side —
  or explicitly document why not.
- **Graph transitions are quiescent and leased.** Switch, restore, and close first
  block interaction and commit the active editor, then flush. Every graph-scoped
  IPC carries the generation of the window→graph binding; Rust rejects a stale
  generation before resolving the graph, so queued old-window work cannot land in
  a newly bound graph.

## Consequences

- A feature that writes or reloads "its own way" is a bug even if it works —
  it re-opens the historical failure classes (stale overwrite, false conflict,
  reload-clobbers-edit).
- The protocol is testable piecemeal (round-trip, conflict, self-write tests
  exist); a cross-layer state-transition test matrix is the desired next gate.
