# 0027 — Tags write-back and multi-valued group-by (tag boards)

Status: accepted (Jul 7 2026, Phase 6c prerequisite; settled autonomously per
the Sheets build mandate — listed under "Decisions Martin should review")

## Context

Sheets v1 reads a block's tags (via lsdoc facets) but never writes them, so
boards can't group by tags — the flagship "reading list by topic" use case.
Tags are inline markup, not a property line: writing them back means editing
the block's first line, which risks corrupting code spans, refs, or URLs if
done with regexes. Spec §3.4 already picked the Notion model for multi-valued
group-by.

## Decision

**Write-back is span-guided, delta-shaped.** `writeField(id, "tags", …)`
gains add/remove of ONE tag per call (the API takes a delta, never the full
list — no reconstruct-the-line writes):

- **Add**: append ` #tag` at the end of the block's *first line* (multi-word
  page names → `#[[multi word]]`). Adding a tag the block already has is a
  no-op.
- **Remove**: locate the tag's `Tag` inline via lsdoc's inline spans on the
  first line and cut exactly that byte range, then normalize the surrounding
  whitespace (never leave doubled spaces). lsdoc spans already exclude
  lookalikes inside code spans/refs/URLs — a `#tag` in a code span is
  invisible to remove, by construction. If the tag exists only *below* the
  first line, remove refuses (returns false) rather than editing body text.
- One parser: the ONLY tag recognizer is lsdoc (facets + spans). No regex
  scanning for `#` anywhere in the write path.

**Multi-valued group-by (Notion model), for boards with
`tine.group-by:: tags`:**

- Columns = observed tag values in first-seen row order, plus `(none)` for
  blocks with zero tags. A card renders in EVERY column whose value it
  carries.
- Moving a card between columns (pointer drag or `Ctrl+←/→`) = remove the
  source column's value + add the target column's value, as ONE undo unit
  (§3.8).
- `(none)` as a drop target is only valid FROM a single-valued card (it is
  then a plain remove). Dropping a multi-valued card on `(none)` is refused —
  silently stripping several tags is too destructive for one gesture.
- Selection identity is (column, row): the other renderings of the same
  card are distinct grid coordinates and render unselected.

## Consequences

- Tag boards become possible with honest markdown round-trip: the file diff
  of a card move is exactly one `#old` removed and one ` #new` appended on
  one line.
- First-line-only editing is a real limitation (a body-line tag can't be
  removed from the board), traded for zero risk to block bodies; the cell
  falls back to opening the block for manual editing.
- The delta API keeps every gesture a single minimal edit — no
  full-line rewrites to drift formatting.
- Org: same inline `#tag` form Tine already parses; the write path is
  format-agnostic because it goes through spans, but 6c must verify against
  the org corpus before enabling.
