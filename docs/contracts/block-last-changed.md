# Bullet last changed (GH #831)

Prior decisions: follow D-20261107-MC3 §9 and ADR 0079 first-parent attribution.

Hovering a block bullet adds last-change information to its existing gesture
tooltip. Desktop bullets reuse the clock badge's tooltip surface so asynchronously
loaded dates update visibly; there is no competing native-title popup. A committed
block shows its newest body-changing commit's timestamp,
formatted in local time using `Date.toLocaleString()` (the Settings convention).
A changed or never-committed body says **Changed, not yet committed**. A graph
outside git says **Unknown**. An older unresolved lineage says **Unknown —
history limit reached**. No creation time or graph property is stored.
Fallback reference rows whose current source is unavailable also say **Unknown**;
loaded live references and embeds use their source page's current snapshot.

`get_page_last_changed(path, current_text)` takes a graph-relative path and the
existing unsaved-inclusive page snapshot. It returns tagged facts keyed by
structural block paths into that snapshot. Fold properties and trailing spaces
normalize as in seen-state. First-parent merges belong to the merge commit.
Malformed/non-UTF-8 history is unknown; I/O errors remain command failures.

The read runs off the UI thread and reuses the seen owner's bounded Walk,
History and structural aligner. The graph binding owns one page/HEAD cache,
replaced when either changes and dropped with the binding. Cache hits check
HEAD but read no historical objects; changed working text only realigns the
current snapshot. First reads walk at most 500 commits, touching the page or
not. No timestamp work happens on render/page open. Touch has no new gesture.

Unit cost: zero persisted bytes, files and transport bytes per edit on both
one-block and sixty-block pages. The cache is memory only.
