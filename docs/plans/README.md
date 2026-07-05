# Implementation plans

Durable, grounded plans for the next Tine work items — written so they survive a
context reset (each is self-contained: goal, OG-parity behavior, current-state
grounding with file:function references, approach, steps, risks, and acceptance
tests). One file per item. When an item ships, delete its plan here and remove its
`docs/BACKLOG.md` row.

These are **plans, not commitments to the order** — priority still lives in
`docs/BACKLOG.md`. A plan existing here means "grounded and ready to execute", not
"in progress" (that's the backlog's *In flight* section).

## Current plans (the top of the P2 backlog)

| # | Plan | Backlog item |
|---|------|--------------|
| 1 | [rendered-copy-fidelity.md](rendered-copy-fidelity.md) | Rendered-copy fidelity — math + off-screen refs + provider macros |
| 2 | [plugin-css-var-shim.md](plugin-css-var-shim.md) | OG `--ls-*` CSS-variable alias shim (theme compat) |

_Shipped: Syncthing sync-conflict detection + block-level merge UI (ADR 0020,
Jul 5 2026)._
