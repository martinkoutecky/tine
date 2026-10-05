# 0072. Window large outline shells with measured re-entry geometry

- **Status:** Accepted
- **Date:** 2026-10-05
- **Supersedes:** the defer-windowing decision in [0008](0008-lazy-block-body-rendering.md)

## Context

GH #623 reports slow first and repeat opens of pages with hundreds to thousands
of blocks and long paragraphs. Body laziness from ADR 0008 removes almost all
parsing on entry, but mounting every reactive block shell and tearing it down
still scales with the whole outline. Production Chromium on the QF1 baseline
opened the 2,000-block benchmark in a median 703 ms with only 12 body parses.
The routed measurement including cold entry and history was slower still.

A cheaper shell that retains all block components cannot bound cleanup after
reading the whole page. Windowing shells while keeping raw-text DOM offscreen
reduced the mixed page to 238 ms, but long paragraphs still took 2.6 seconds:
their invisible text layout remained page-sized. Estimating every word reduced
that cost but still missed the budget for long paragraphs. Bounded text samples
remove that work without adding a content parser or a second outline model.

## Decision

`BlockList` is the ordinary sibling-list renderer in pages (including journals
and split panes), sidebar pages and recursive block children. Lists above 80
expanded blocks use groups with a target weight of 24 rendered descendants;
smaller outlines retain direct rendering. This includes large trees whose
individual sibling lists are short. Single-child chains keep their existing
cleanup depth through the shared For props. One observer per large list mounts groups near the viewport and replaces distant groups with
spacers. These constants are implementation choices, not compatibility rules.

Initial heights estimate literal text wrapping from at most 160 characters per
line; they never recognize Logseq syntax. Actual group heights replace estimates
on first visit. On exit, the last measured height remains; returning to an
unchanged, same-width group uses that height. A bounded, graph-scoped cache also
preserves geometry across navigation. Width, font or document changes invalidate the
affected geometry. Geometry uses the same occurrence-local collapse context as
the rendered blocks, so folding a secondary view does not change its source or
leave expanded spacers behind. There is no `content-visibility` CSS or detached component
cache. First visits can refine estimates, like ADR 0008's first body render;
seen content does not revert to an estimate during a scroll round-trip.

The document remains the navigation, selection and persistence authority.
`outlineViewport` lets editor intent, router block jumps, selection scrolling and
Find synchronously reveal a model id within a view before using its DOM. Windows
register descendants, so revealing a nested id mounts its ancestor windows too.
The current editor pins its group. Pointer capture warms a newly scrolled drop
destination before the shared drag hit test. Browser printing temporarily mounts
the full outline and bodies; static export continues to read the document.

## Consequences

Live shell creation and cleanup depend on the viewport and active editor rather
than total sibling count. DTO ingestion, cheap geometry and id registration still
cost O(page blocks); this decision does not change native page reads or searches.
Editing, caret, selection, drag, collapse/zoom, block jumps, model Find, history,
references/embeds, print/export and seen scrollbar extent have regression evidence
at their observing layers. Linux measurements do not certify Windows timing.

Unit cost: no persisted record, file, edit kind or transport change; delta is
0 bytes and 0 files per edit on both 1-block and 60-block pages. The only cache
is bounded, disposable in-memory geometry.
