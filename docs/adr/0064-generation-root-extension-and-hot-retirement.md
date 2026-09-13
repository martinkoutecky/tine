# 0064. Generation root extension and hot-history retirement

- **Status:** Accepted
- **Date:** 2026-09-11

## Context

The first clean checkpoint implementation reopened and retained work in
proportion to accepted device lifetime H. Its payload embedded the covered
manifest/object roster, ordinary namespace open decoded covered originals, and
floor selection enumerated accepted sequences through the age-eligible bound E
once per changed document. Cold publication was additive, so the same covered
bytes also remained in the hot namespace indefinitely.

## Decision

One current checkpoint format replaces those representations. Each publication
extends its predecessor's sealed accepted roots with only C+1..=new C, extends
a covered-object membership root, and records a `(document, acceptance
sequence)` latest-change index. A changed document asks that index for one
predecessor candidate instead of walking 1..=E. The payload carries roots and
bounded live state, never an H-sized roster or object union.

The marker-selected generation is qualified before namespace content
validation. Covered hot names are recognized by point membership and skipped;
only the live tail and explicit hot pins are decoded. Accepted rows remain
point-addressable on disk, and ordinary restore does not materialize them into
the engine's resident maps. Full sequence enumeration remains an explicit
historical consumer, not generation-open work.

Cold relocation visits exactly the newly covered delta. Candidate bytes and
roots are validated, cold originals are published, and `current` is durably
replaced before any hot unlink. The hot-retention set is the union of current
document/projection heads and bounded current-action roots. Retirement verifies
the exact cold manifest and objects before deletion, is idempotent, and resumes
from covered hot names after interruption.

## Consequences

Healthy publication and open work are functions of delta/live/action state,
not accepted lifetime. The observable counters ratchet zero covered-sequence
enumeration and delta-only relocation visits; covered namespace decodes are
ratcheted by the real `namespace_{manifest,object}_decodes` instrumentation and
made unwritable by the `is_covered` early `continue`. A counter is only kept
here when some code path can actually raise it — a field nothing increments
reads zero whether or not the property holds.

The accepted index and cold archive remain immutable recovery authority. A
missing or invalid disposable generation still takes the sequence-zero full
audit and replay. Marker-last ordering means a crash can leave both hot and cold
copies, but never authorizes deletion before an exact cold copy and the new
generation are durable.

Landing 4b extends that same marker and object pool with four typed identity
domains: block/home claims, Logseq UUID provenance, portable paths, and page
names. Each domain has a complete point root and a current-claim root. A cut
path-copies only changed leaves in the background publisher; the actor hands
off typed delta records and never clones or encodes the complete maps.
Current-root enumeration is confined to generation installation and rebuilds
only live claims and current path/head memos. Release-only evidence remains in
the complete roots and ordinary admission reads it by exact key. Page-name
acquisitions persist their declared frontier so reclaim admission does not
recover a covered manifest merely to classify the release.

The four adapters preserve their existing precedence: speculative local
overlay, accepted recent overlay/current claims, then sealed generation point.
An absent authenticated leaf is distinct from a missing or unreadable value.
The sealed values remain evidence for the established semantic admission
algorithms; they do not become a second ownership authority. Current path
memos bind the exact catalog row and current projection-head batch, allowing a
cut to discard released paths from the resident head map while retaining their
release facts on disk.

Landing 4c extends the same qualified generation with the physical SQLite
anchor and a bounded conflict-history seed. SQLite remains a disposable
derivation: a fresh file installs the generation at covered sequence C with an
empty document overlay and no accepted or materialization rows, then applies
only C+1 through the current terminal sequence. The logical frontier continues
to report total documents, blocks, retained bytes and acceptance sequence;
separate physical overlay counts describe only post-C rows. Integrity checks
resident accepted rows against `terminal − C`, never the lifetime sequence.

The generation marker is authoritative across the two stores. A committed
SQLite tail with the old marker is reusable. Once a newer marker is durable,
the same rows are covered and a later open rebuilds the disposable database
from the new anchor plus its uncovered tail. Candidate publication, generation
publication and handle replacement need not form one filesystem transaction:
every crash prefix selects a complete generation first and treats absent,
behind, divergent or differently anchored SQLite as rebuild input, never as a
veto on graph open.

Conflict evaluation restores causal tips, the non-linearity watermark and
still-unresolved pair endpoints from checkpoint state. Settled touches do not
replay through accepted history on healthy open. A covered batch offered to the
SQLite duplicate branch is a caller-bug tripwire, not a delivery or corruption
boundary: all production appliers prove the database equals the event's exact
prior frontier before calling.
