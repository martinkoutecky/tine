# 0004. Operate on the Logseq on-disk format; match Logseq by default

- **Status:** Accepted
- **Date:** 2026-06-29

## Context

Tine is meant to be usable *on the graph you already keep in Logseq*, switching
between the two apps and (over Syncthing) Logseq mobile. The alternative — Tine's
own database/format with import/export — would be simpler internally (we'd control
the schema) but would create lock-in, a sync step, and drift from Logseq's evolving
behavior.

## Decision

We will operate **directly on the standard Logseq graph layout** (`journals/`,
`pages/`, `assets/`, `logseq/config.edn`), reading and writing Logseq-compatible
Markdown (and `.org`), with **no import/export step**. And we will **match Logseq
("OG") behavior by default** — semantics, file format, edge cases, defaults,
keyboard behavior — deviating only on an explicit, documented decision. When unsure,
we determine what OG actually does before implementing.

## Consequences

- **Easier:** no lock-in, no migration; users can adopt and abandon Tine non-
  destructively; "is this right?" usually has a concrete oracle (what OG does).
- **Harder:** we inherit Logseq's format quirks and must reproduce them faithfully
  (property pre-blocks, `id::`/`collapsed::` handling, namespace filename encoding,
  journal date formats, in-content `+`/`*` lists vs outline bullets). Byte-faithful
  round-tripping is a hard constraint, not a nicety — it's what makes coexistence
  safe (see [0007](0007-data-safety-invariants.md)).
- **Committed to:** "match OG by default" is a standing project rule (see
  `CLAUDE.md`); divergences are opt-in, surfaced in the UI ("Differs from Logseq"),
  and few.
