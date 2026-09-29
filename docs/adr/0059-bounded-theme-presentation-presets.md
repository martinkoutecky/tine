# 0059. Theme presentation remains a bounded host-owned contract

- **Status:** Accepted
- **Date:** 2026-08-28

## Context

Theme API 0.1 safely recolors Tine, but color alone cannot represent editorial
themes such as notnote. Their identity also comes from reading typography, a
distinct journal header, and small host-computed context such as a Today task
summary. Allowing arbitrary CSS, fonts, markup, or queries would make themes
depend on Tine's private DOM and widen an inert package into executable or
content-reading extension code.

## Decision

- Theme API 0.2 retains the 0.1 color-token contract and adds one optional
  `presentation` object. Its fields and values are closed enums. Unknown fields
  and values are rejected by both the runtime parser and `theme:check`.
- The initial host-owned choices are `editorial-serif` content typography,
  `editorial` journal headers, and a `compact` Today task summary. Packages select
  a treatment; Tine owns its CSS selectors, local system-font stack, dimensions,
  responsive layout, accessibility, text, and computation.
- The editorial journal header centers and enlarges Today, keeps older journal
  dates left-aligned, and omits the calendar glyph. Other page and navigation
  behavior is unchanged.
- The compact summary reads only the already-loaded Today page and uses Tine's
  canonical block facets. It counts open task markers on that page; its
  in-progress count is the subset marked `DOING`, `NOW`, `STARTED`, or
  `IN-PROGRESS`. It performs no IPC, graph scan, or second parse.
- A future graph-wide definition, such as notnote's overdue and scheduled task
  aggregation, requires a dedicated bounded indexed count API. A theme must
  never run a query or receive graph content itself.
- API 0.1 packages remain installable and selectable. They cannot declare
  presentation fields. `logseq/custom.css` still loads last.

## Consequences

Themes can now change the reading character of Tine and add a small semantic
host slot without gaining code or DOM authority. The initial task count is
deliberately page-local and cheap; expanding its semantics is a core Tine feature,
not a manifest escape hatch. Presentation names are stable public vocabulary,
while their implementation can evolve with Tine's UI.
