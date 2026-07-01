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
