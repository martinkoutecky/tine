# Frontend text lifetimes (L10-01–04 / L7-01)

Backend facets belong to loaded document nodes. Removing or replacing an owner
releases its seed; identical text shared by loaded nodes retains one entry until
the final owner leaves. A loaded page of any size keeps every shipped facet.

Historical parse results have an 8 MiB aggregate charge, an 8,000 entry cap, and
a 1 MiB single-entry admission limit. Charge includes UTF-16 source plus a
conservative estimate of the serialized AST and regions (eight bytes per JSON
character plus fixed overhead). Up to 8,000 hash-keyed weak references reuse ASTs still held by
current consumers without admitting oversized entries or retaining dead text.
Store reset clears strong and weak lookups eagerly. Hash collisions are verified
against the exact source; they may cause a cold parse, never a wrong answer.

Block resolutions coalesce in flight. Completed answers with reactive change-key
consumers remain available; inactive answers share an 8 MiB / 512 entry budget
with a 1 MiB single-entry admission limit. Charge covers source strings and DTO
structure conservatively. Invalidations remove obsolete completed answers at
publication while unrelated saves preserve them. Binding reset eagerly clears
answers and queued requests; late replies cannot repopulate the new binding.
Dependency keys with reactive consumers are released at the end of the turn on
disposal; immediate reacquisition during recomputation preserves their signal.
Untracked key
reads have a bounded 512-entry history and singleton keys have unique revision tokens, so eviction
can cause a conservative reread but never a stale cache hit.
An export preview owns its warmed reference answers until it closes, including
nested references; output completeness does not depend on inactive-cache capacity.

These are retention charges, not device heap budgets. No persisted format or
storage write changes.

## Journal feed days (L10-03, development only)

The feed keeps day names and measured-height placeholders, rather than retaining
every appended document. Once more than 12 days are loaded, measured offscreen
days retire their nodes, facet owners and undo snapshots. Near-viewport days,
editing pages, drafts, unsaved/saving/conflicted pages, transfers, graph freezes,
selection and routed/deep-link/sidebar targets stay loaded even above that bound.
Every mounted feed surface must have measured the day and be away from it before
it can retire. A day near any viewport reloads through the normal owned page read
and admission gate. Retiring and reloading preserve the reader's viewport and
measured space; no offscreen source text cache is introduced.
Measured heights survive feed surface disposal/reopening, are pruned on the
next feed surface update, and clear with the graph binding. No document body
is retained for this geometry.

Find/Replace searches loaded days only. When Journals is visible, Find shows
“N older days not searched — search all journals” for retired days. Its action
opens the existing graph-wide search tab with the current query (D-20261022-JN).
Find never reads retired bodies from disk.
