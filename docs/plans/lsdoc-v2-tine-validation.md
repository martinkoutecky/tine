# lsdoc v2 validation in Tine

## Goal

Validate that Tine works correctly with the new `lsdoc` v2 implementation before
shipping it. The validation must exercise the parser artifact Tine actually uses,
not only `lsdoc`'s standalone Rust harness.

The three integration risks this plan is meant to catch are:

- **pin skew**: Rust backend and renderer WASM are built from different `lsdoc`
  versions;
- **packaging skew**: dev mode works, but the vendored/bundled WASM used by the
  app is stale or loaded differently;
- **block-boundary skew**: whole-document parity passes, but Tine's per-block
  parse path changes behavior when it re-bullets de-bulleted block bodies.

Relevant background:

- ADR 0006: `../adr/0006-in-browser-wasm-parsing.md`
- ADR 0015: `../adr/0015-lsdoc-wire-contract.md`
- ADR 0018: `../adr/0018-in-app-lsdoc-mldoc-diff-panel.md`
- ADR 0007: `../adr/0007-data-safety-invariants.md`

## Preferred pinning route

Prefer testing an `lsdoc` release-candidate tag, for example
`v0.4.3-rc1`, rather than a local path dependency. That tests the same shape as
the shipping path and keeps Tine's existing pin guards meaningful.

Update both dependencies to the same tag:

- `crates/tine-core/Cargo.toml`
- `crates/lsdoc-wasm/Cargo.toml`

Do not update only one of them. The backend indexes through `tine-core`; the
renderer parses through the vendored WASM wrapper.

Local `path = "/aux/koutecky/logseq/lsdoc"` testing is acceptable for a quick
developer smoke test, but it is weaker. The current scripts
`scripts/check-wasm-pin.mjs` and `scripts/build-wasm.mjs` expect a Cargo
`tag = "..."` pin. If using a path or rev temporarily, update those scripts in
the same test branch or treat the result as non-release-equivalent.

Also do not rely on a root Cargo `[patch]` alone. `crates/lsdoc-wasm` is built as
its own WASM package, so the dependency source used by the renderer must be
checked explicitly.

## Build and automated gates

From the Tine repo root:

```bash
rtk npm run build:wasm
rtk npm run build
rtk cargo test --workspace
rtk npm test
rtk npm run test:render
```

`npm run build` matters because it runs the pin guard. A passing `cargo test` or
renderer test alone does not prove that the vendored WASM bytes, the frontend
stamp, and the Rust dependency pin agree.

Expected result:

- all commands pass;
- the generated `src/render/wasm/*` files correspond to the same `lsdoc` pin as
  both Cargo manifests;
- no parser trap is hidden by the renderer fallback path.

## In-app differential scan

Run Tine on a disposable copy of a real graph:

```bash
rtk npm run app
```

Then open **Settings -> Help improve Tine** and scan the graph. This is the best
app-level parser parity check because it compares the Tine-shipped WASM parser
against bundled `mldoc` in the same application surface users run.

Expected result: zero divergences.

If there is any divergence:

- use the panel's minimized/anonymized reproducer;
- verify that the anonymized snippet still reproduces;
- add the case to the `lsdoc` harness corpus;
- fix `lsdoc` before release.

Do not classify a divergence as harmless merely because the UI still renders
something plausible.

## Block-boundary gate

Whole-document parity is necessary but not sufficient. Tine's block renderer uses
`crates/lsdoc-wasm/src/lib.rs`, where `parse_block_json` re-adds a Markdown or Org
bullet marker before calling `lsdoc`. That path can diverge even when a
whole-file scan passes.

The strongest missing gate is to export real Tine block raw bodies from shared
graphs into `block-raws.json` and run the `lsdoc` blockgate against them. This
should include Markdown and Org graphs, plus blocks with:

- nested bullets and indentation-sensitive continuations;
- properties and drawers;
- block refs, page refs, tags, and aliases;
- tables;
- timestamps, priorities, markers, and logbook content;
- macros, images, file links, and inline HTML/hiccup.

If no export command exists, add a temporary/debug command for this validation.
This is high leverage because it exercises the exact Tine-to-`lsdoc` boundary,
not just `lsdoc`'s own whole-document parser.

## Data-safety smoke test

Use a copied graph, preferably one under git. Never do the first validation run on
the only copy of a real graph.

After opening the graph, navigating, scanning with Help improve Tine, and doing
one deliberate small edit, inspect disk changes:

```bash
rtk git diff --stat
rtk git diff
```

For pure navigation and scanning, there should be no note-file diffs. For the
deliberate edit, the diff should be localized and expected. Any unexpected write
is a blocker, even if parser parity passes.

Also watch for:

- parser-error banners;
- console errors from `initParser`, `parseBlock`, or WASM loading;
- quarantined parser traps that fall back to plain paragraph rendering;
- stale-runtime warnings from `lsdoc_tag()`.

## Packaged-app gate

Repeat the critical checks in the built app, not only in Vite dev mode:

```bash
rtk npm run build:wasm
rtk npm run build
rtk npm run tauri -- build
```

Open the packaged app on the same disposable graph and repeat:

- Help improve Tine graph scan;
- short navigation/render smoke test;
- one deliberate edit followed by disk diff inspection.

This catches vendoring and packaging differences that dev mode can hide.

## Release blockers

Block the `lsdoc` bump if any of these occur:

- Rust and WASM pins do not match;
- vendored WASM was not rebuilt from the intended `lsdoc` revision;
- Help improve Tine reports a reproducible divergence;
- `parse_block_json` block-body cases diverge from the expected output;
- the renderer traps and falls back to plain paragraph output;
- opening or scanning a graph causes unexpected file writes;
- packaged app behavior differs from dev behavior.

The intended release shape is: same `lsdoc` tag in both Cargo manifests, fresh
vendored WASM bytes stamped with that tag, green automated gates, zero in-app
mldoc divergences on real graphs, and no unexpected disk diffs on copied graphs.
