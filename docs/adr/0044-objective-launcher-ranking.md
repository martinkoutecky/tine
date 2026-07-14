# 0044. Objective search classes and bounded launcher frecency

- **Status:** Accepted
- **Date:** 2026-07-14

## Context

Ctrl+K currently exposes an opaque numeric page score. GitHub #143 asks search
to learn from choices, but applying one adaptive order to persistent searches,
queries, and launchers would make saved results device-history-dependent and
hard to explain. An unconstrained history also creates privacy, performance,
and unbounded-state risks.

## Decision

The Rust query engine reports discrete objective match classes: exact title or
alias, title or alias prefix, title or alias substring/phrase, fuzzy title or
alias, and block-body evidence. The class and deterministic tie breakers define
all persistent search and query results. Aliases participate in the same class
as titles, while the hit retains which alias supplied the evidence.

Ctrl+K may apply query-conditioned frecency only inside one objective class. It
records intentional activation, never display, hover, or keyboard focus. One
accidental activation has no visible effect. History is graph-scoped,
device-local, bounded, decayed, stored outside Markdown, and never included in
diagnostic exports. Orphaned identities are pruned; users can disable learning
and reset the current graph's ranking.

The backend returns a bounded objective top-K. The frontend reranks that bounded
set, after matching, and writes only when a result is opened. Candidate keys are
precomputed/cached; no history write or whole-graph adaptive work occurs per
keystroke. Favorites populate empty-query or dedicated views and do not outrank
stronger nonempty-query evidence.

## Consequences

Search membership and broad relevance remain stable and explainable. Ctrl+K can
become faster for repeated personal navigation without contaminating durable
queries or other devices. The history format is an implementation detail rather
than graph data or a plugin ranking API.

