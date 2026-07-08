# Architecture Decision Records

Short, append-only notes on the **load-bearing** decisions behind Tine — the ones
where knowing *why* matters later, so we (and contributors, and the AI that does
much of the implementation) don't silently undo a deliberate choice.

This is **not** a log of every decision. Record one when a choice is architectural
and would be expensive or risky to reverse — a framework, a data-flow boundary, a
format commitment, a safety invariant. Skip the rest.

## Format

One file per decision, `NNNN-short-title.md`, using
[Michael Nygard's template](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
(see [`template.md`](template.md)): **Context → Decision → Consequences**, plus a
**Status**. Keep it to a screen. Don't edit a decided record to change its meaning;
if a decision is reversed, add a *new* ADR that supersedes it and flip the old one's
status to `Superseded by NNNN`.

## When to write one

When a load-bearing decision is actually made (not when it's merely floated). If
you're the AI implementing such a change, write the ADR in the same chunk of work —
see the project `CLAUDE.md`.

## Index

| # | Decision | Status |
|---|----------|--------|
| [0001](0001-tauri-webkitgtk-over-electron.md) | Tauri + the OS webview (WebKitGTK) instead of Electron | Accepted |
| [0002](0002-solidjs-frontend-owns-the-edit-tree.md) | SolidJS, and the frontend owns the live edit tree | Accepted |
| [0003](0003-pure-rust-core-thin-ipc.md) | A pure-Rust core (`tine-core`) behind a thin IPC layer | Accepted |
| [0004](0004-operate-on-the-logseq-format-og-parity.md) | Operate on the Logseq on-disk format; match Logseq by default | Accepted |
| [0005](0005-lsdoc-separate-parser-crate.md) | `lsdoc`: a separate, public mldoc reimplementation | Accepted |
| [0006](0006-in-browser-wasm-parsing.md) | Parse in the browser via `lsdoc` compiled to WASM (vendored) | Accepted |
| [0007](0007-data-safety-invariants.md) | Data-safety invariants: never silently overwrite | Accepted |
| [0008](0008-lazy-block-body-rendering.md) | Lazy block-body rendering (render-once-keep), not windowing | Accepted |
| [0009](0009-one-block-facet-source-on-the-dto.md) | One source for block-header facets: the lsdoc parse, shipped on the DTO | Accepted |
| [0010](0010-lsdoc-canonical-html-skeleton.md) | lsdoc owns the canonical HTML skeleton; the export consumes it, the frontend conforms | Accepted |
| [0011](0011-in-app-self-update.md) | In-app self-update on Windows/Linux via the Tauri v2 updater (macOS manual) | Accepted |
| [0012](0012-save-watch-edit-coherency-protocol.md) | The save/watch/edit coherency protocol has one shape — don't add a second | Accepted |
| [0013](0013-block-identity-vs-rendered-instance.md) | One block, many rendered instances: focus/caret ownership rules | Accepted |
| [0014](0014-cache-gen-invalidation-contract.md) | Derived caches self-invalidate via `cache_gen` — no hand-rolled invalidation | Accepted |
| [0015](0015-lsdoc-wire-contract.md) | The lsdoc wire contract: what Tine may assume about lsdoc's output | Accepted |
| [0016](0016-mock-backend-intentionally-lossy.md) | The mock backend is an intentionally lossy dev double | Accepted |
| [0017](0017-snapshot-scope.md) | Launch snapshots cover text + config + PDF highlight sidecars, not asset bytes | Accepted |
| [0018](0018-in-app-lsdoc-mldoc-diff-panel.md) | In-app lsdoc↔mldoc diff panel bundles mldoc lazily and runs it in a worker | Accepted |
| [0019](0019-raw-html-sanitizer.md) | Raw HTML renders live via a shared sanitizer allowlist (DOMPurify + ammonia), mirrored and contract-tested | Accepted |
| [0020](0020-sync-conflict-merge.md) | Sync-conflict copies: detect + block-tree merge from one shared alignment; resolve only through the safe save path | Accepted |
| [0021](0021-pdf-export.md) | PDF export reuses the HTML render + the webview's own print engine (hidden iframe + `window.print()`), no new deps | Accepted |
| [0022](0022-logbook-clock-drawer-format.md) | Logbook CLOCK drawer parsing and writes live in one Rust module shared with wasm | Accepted |
| [0023](0023-sheets-render-substrate.md) | Sheets lays out on CSS Grid max-content tracks (Phase-0 spike GO; table-auto rejected) | Accepted |
| [0024](0024-sheets-header-row.md) | Sheets positional-grid header row is explicit opt-in (`tine.header:: true`), never auto-detected | Accepted |
| [0025](0025-sheets-mode-boundaries.md) | Sheets mode boundaries: click selects, double-click edits, Esc ladder, flow-out not wrap | Accepted |
| [0026](0026-sheets-field-schema.md) | Sheets field schema: `tine.fields::` scalar grammar, per-tag + per-view homes, declared-first columns, typed cells | Accepted |
| [0027](0027-sheets-tags-writeback.md) | Tags write-back is span-guided and delta-shaped (first line only); tag boards use the Notion multi-group model | Accepted |
| [0028](0028-sheets-formula-dsl.md) | Sheets formulas: Bases-model typed expression DSL, `tine.formula.<name>::` lines, errors as values, derived never stored | Accepted |
| [0029](0029-sigkill-webkit-children-on-exit.md) | SIGKILL WebKitGTK's helper processes at quit (Linux) to prevent the GL-driver exit-teardown coredump (was master's 0023; renumbered in the sheets merge) | Accepted |
| [0030](0030-query-view-unification.md) | Query view unification: query DSL owns membership, `tine.view::` owns presentation | Accepted |
| [0031](0031-recursive-cell-form.md) | Recursive sheet cell form: compact and hosted grids with structural clipboard paste | Accepted |
| [0032](0032-pane-split-tree-architecture.md) | Split view: pane-router factory + focused-pane shims, pane split-tree, single journals feed pane | Accepted |
| [0033](0033-pane-select-nav-semantics.md) | Pane-select nav semantics: overlap-constrained stepping, pane-edge segments (split one pane vs the root), focus-follows-selection, 2-rung Esc ladder | Accepted |
