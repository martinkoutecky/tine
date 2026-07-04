# 0018. In-app lsdoc↔mldoc diff panel bundles mldoc and runs it in a worker

- **Status:** Accepted
- **Date:** 2026-07-04

## Context

lsdoc (Tine's parser) is a from-scratch reimplementation of Logseq's `mldoc`, kept
byte-exact by a differential oracle. Finding the remaining divergences needs real
graphs. `lsdoc/tools/graph-check.mjs` already does this on the command line, but it
needs Node, npm, and a Rust toolchain — out of reach for the average Tine user we
want to crowd-source divergences from.

The forces:

- **Reference parser availability.** The only faithful oracle is mldoc itself.
  mldoc's npm build is a self-contained js_of_ocaml bundle (~413 KB) that runs in a
  browser realm (proven in jsdom, and it is what Logseq's own renderer uses). So the
  comparison can run entirely inside Tine's WebView — no subprocess, no second app.
- **Bundle weight vs. reach.** Shipping mldoc bloats the download for everyone, but a
  separate "lab" build adds friction ("download this other version") that loses most
  would-be helpers.
- **mldoc can hang / be quadratic** on adversarial input (that is literally the perf
  story), and it **leaks global state across parses in one realm**. graph-check
  handles both by spawning a fresh, timeout-guarded subprocess per file.
- **Licensing.** Tine, lsdoc, and mldoc are all AGPL-3.0, so bundling mldoc is clean
  (mldoc's npm `package.json` misreports "ISC"; the source is AGPL).

## Decision

We will ship an in-app **"Help improve Tine"** settings panel that runs a faithful
port of `graph-check.mjs` against the user's own graph, entirely locally:

- **mldoc is bundled but lazy-loaded** (dynamic `import()` of a `?url` asset), so it
  costs nothing at startup and only the panel's use pays for it. It ships in the
  normal release — no separate build.
- **mldoc runs in a Web Worker; lsdoc runs on the main thread.** The worker is the
  browser analog of graph-check's per-file subprocess: it gets mldoc off the UI
  thread and lets the orchestrator `terminate()` a wedged parse (timeout kill) and
  get a fresh, uncontaminated mldoc realm by respawning. lsdoc is stateless and O(n)
  by construction, so it needs neither isolation nor a timeout.
- **The comparison pipeline is transcribed, not reinvented** from graph-check (scrub
  tiers + verify-after-scrub, minimizer, projection key), reusing the harness's own
  `normalize`/`refs`/`compare` libs verbatim so an in-app diff and the CI gate agree
  on what a divergence is.
- Divergence snippets are shown/copied **only after anonymization is re-verified to
  still reproduce the divergence** — the privacy guarantee for sharing from a private
  graph.

## Consequences

- Easier: any Tine user can hunt divergences from a menu; one download, one click.
  The panel doubles as an lsdoc-vs-mldoc speed comparison on real graphs.
- The one-parser rule (ADR-style invariant) still holds: lsdoc remains Tine's sole
  block parser; mldoc is present **only** as a diagnostic oracle, never in the render
  path.
- Cost: +~413 KB in the shipped assets (lazy, so no startup cost); an AGPL bundle we
  must re-vendor when we bump the pinned mldoc.
- Committed follow-on: the panel needs a whole-file `parse_document_json` export in
  `lsdoc-wasm` (Tine's current wasm only parses per-block). Until it lands the lsdoc
  side is stubbed and the panel runs mldoc-only speed timing; divergence scanning
  lights up when the export ships (lsdoc 0.3.4).
- Risk: mldoc loading via dynamic `import()` inside a Vite **module** worker under
  WebKitGTK's CSP is validated by build + jsdom + Chromium, but the exact
  Tauri-WebKitGTK combination is only confirmable in the running app (the wiring
  smoke test).
