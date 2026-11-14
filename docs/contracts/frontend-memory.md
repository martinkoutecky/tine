# Frontend text lifetimes (L10-01–04 / L7-01)

Backend facets belong to loaded document nodes. Removing or replacing an owner
releases its seed; identical text shared by loaded nodes retains one entry until
the final owner leaves. A loaded page of any size keeps every shipped facet.

Read applicability and own-publication delivery follow
[`storage-contract.md`](../storage-contract.md). An applicable unchanged-revision
read keeps the loaded nodes without comparing the page body.

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
Inactive accounting follows answer completion and keyed consumer acquisition or
release. Eviction uses last-access order, regardless of completion or disposal
order; maintenance never walks active answers on another answer's arrival or
release. Precise publications visit only their named block answers.
Dependency keys with reactive consumers are released at the end of the turn on
disposal; immediate reacquisition during recomputation preserves their signal.
Untracked key
reads have a bounded 512-entry history and singleton keys have unique revision tokens, so eviction
can cause a conservative reread but never a stale cache hit.
An export preview owns its warmed reference answers until it closes, including
nested references; output completeness does not depend on inactive-cache capacity.

These are retention charges, not device heap budgets. No persisted format or
storage write changes.

Keyed page-inventory answers, selected block memberships, active editor
identities, sheet owners, seen baselines and authored identity claims notify
only their changed keys. Their dependency registries follow reactive consumers
and retain at most 512 imperative keys. Inventory deltas visit their named
buckets; ordered whole-inventory views materialize only when requested.
Reference panels retain independent fold snapshots and persist individual
choices without copying other groups; bulk changes and explicit reloads still
visit the replaced choices. Idle query schedulers do not subscribe to input or
pending gates; due work retains the quiet and publication checks.

## Journal feed days (L10-03, development only)

See also the [reference rendering contract](reference-rendering.md) for the
disposable content lifetime of Linked and Unlinked References. Their membership,
occurrence choices and measured geometry outlive expensive mounted children;
they retain no detached components or second source-text cache.

The feed keeps day names and measured-height placeholders, rather than retaining
every appended document. Once more than 12 days are loaded, measured offscreen
days retire their nodes, facet owners and undo snapshots. Near-viewport days,
editing pages, drafts, unsaved/saving/conflicted pages, transfers, graph freezes,
selection and routed/deep-link/sidebar targets stay loaded even above that bound.
Every mounted feed surface must have measured the day and be away from it before
it can retire. Retirement candidates follow per-name slot geometry and loaded
membership; observer frames do not scan unrelated near or retired placeholders.
Candidates keep slot registration order and pass every existing safety pin.
A day near any viewport reloads through the normal owned page read
and admission gate. Retiring and reloading preserve the reader's viewport and
measured space; no offscreen source text cache is introduced.
Measured heights survive feed surface disposal/reopening, are pruned on the
next feed surface update, and clear with the graph binding. No document body
is retained for this geometry.

Find/Replace searches loaded days only. When Journals is visible, Find shows
“N older days not searched — search all journals” for retired days. Its action
opens the existing graph-wide search tab with the current query (D-20261022-JN).
Find never reads retired bodies from disk. Find also leaves reference panels
out of its count, which would otherwise grow as their groups mount on scroll; it
names them, and its action opens the panel's own search with the query (GH #935;
[reference rendering contract](reference-rendering.md)).
