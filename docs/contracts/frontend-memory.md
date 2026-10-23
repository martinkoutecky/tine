# Frontend text lifetimes (L10-01, L10-02, L10-04 / L7-01)

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

These are retention charges, not device heap budgets. No persisted format,
storage write, or journal retirement behavior changes. L10-03 remains separate.
