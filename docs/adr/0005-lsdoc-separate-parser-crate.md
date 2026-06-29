# 0005. `lsdoc`: a separate, public mldoc reimplementation

- **Status:** Accepted
- **Date:** 2026-06-29

## Context

Logseq parses its Markdown/org with **mldoc** (an OCaml library). Tine needs the
*same* parse to render faithfully and to extract refs. Early on, parsing lived in
ad-hoc Tine code (an "optimistic scanner") that grew ~15 perf and correctness bugs —
the same architectural flaw Logseq's own renderer had, recreated in Rust. A parser
this central deserves to be a real, separately-tested component, not an organic
sprawl inside the app.

## Decision

We will build **`lsdoc`**, a standalone Rust reimplementation of mldoc, as its own
crate and **public repository** (AGPL), with a differential **oracle harness** that
diffs `lsdoc` against real mldoc (`mldoc@1.5.7` + Logseq's `block.cljs`) on a corpus.
Tine depends on it by pinned tag. `lsdoc`'s scanner is being rebuilt as a real
two-phase lexer (cmark-style block-stack + delimiter-stack), with the AST/oracle as
the rewrite's gate.

## Consequences

- **Easier:** one well-tested parser with a correctness oracle, reusable beyond Tine;
  parser work happens in its own repo/session without churning the app.
- **Harder:** a cross-repo dependency with a version pin to manage (and a WASM build
  to regenerate on bump — see [0006](0006-in-browser-wasm-parsing.md)); the
  public API surface (`parse`, `refs`, projections, `ast::Inline`) must stay stable.
- **Lesson recorded:** a parity gate (AST matches mldoc) validates *output*, not
  *architecture* — the optimistic scanner passed the gate while being structurally
  wrong. The rewrite keeps the gate but fixes the structure.
