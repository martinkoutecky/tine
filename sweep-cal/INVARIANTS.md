# What Tine must be — the invariant list

**Version: 1.2 (FROZEN), 2026-09-01.** (v1.1→v1.2: I-22 sharpened — Martin:
sanitization lives at the point of CONSUMPTION; our own export sanitization
never protects import, because a shared graph's producer is arbitrary code.) (v1.0→v1.1, same day: Part 1 items
2/4 and I-7 scoped to *current content* — the md projection does not carry
history, and losing oplog history on exit is accepted; Martin. Verdicts citing
v1.0 differ only there: v1.0 I-7 flags about history retention are non-findings.) Draft 1 → Martin's review (ranking
decided, direction added, two families extended) → draft 2 → Sol/xhigh
adversarial review (38 findings; all folded or explicitly rejected in
`sol-review-disposition.md`) → this freeze. Sweep verdicts cite
`invariants: v1.0`. Changes only by a dated edit here plus a version bump;
verdicts made under an older version keep their citation.

**Purpose:** this is the checklist a reader applies to **every** file in the
sweep manifest. Its quality is the ceiling on the sweep's quality: a reader who
does not have an invariant in hand cannot flag its violation, no matter how
capable the reader.

**Why the sweep exists.** Under a case-insensitive substring search, 269 of the
manifest's 385 files contain neither "managed" nor "authority". `src/carry.ts`
is one of them — and Carry mutates a managed graph. Every audit that searched
outward from Managed Storage was structurally unable to reach it. Reading every
file against a written list is the only method without that blind spot.

**What a verdict means — read this first.** Every verdict in this sweep is a
claim about **the file in front of the reader, and nothing more**. `PASS` means
"no local violation found, and here is what I checked" — it never certifies a
global property. Global questions (uniqueness, reachability, who else computes
this) are answered at **collation**, from the facts all readers recorded, not by
any single reader. `CANNOT-DETERMINE` is a first-class honest answer.

---

## Part 1 — What Tine is (Martin's section; item 8 added by Martin 2026-09-01)

1. Tine is a **fast, local-first outliner over the user's real Logseq graph** —
   plain Markdown and Org files on disk that other tools, and the user, can read.
2. **The files are the product; the user's ownership is never hostage to Tine.**
   Managed Storage may hold its own internal durable authority (the accepted
   journal/manifest — `model.rs`'s `ReconstructibleManagedProjection` names
   exactly this), but the Markdown/Org projection must always carry the graph's
   complete **current content** once the store is quiescent. It does not carry
   history: the oplog's edit history is Tine-private and losing it on exit is
   accepted. (A future git bridge might project some history into a repo — a
   bonus if it happens, never an obligation.)
3. **Speed is why Tine exists.** A correct outliner that feels slow has failed at
   the thing it was built for.
4. The user must be able to **leave at any time** and lose nothing *current*:
   deleting all of Tine's private state after quiescence loses no current user
   content — it may lose edit history and undo, which is the accepted price —
   and Tine can rebuild its private state from the files.
5. **Logseq's behaviour is the default.** Divergence is allowed, but it must be a
   decision someone made and can name — never an accident.
6. Tine is maintained by **one person working through agents, part-time**.
   Legibility is a first-class product requirement, not tidiness: what cannot be
   read cannot be kept correct.
7. Managed Storage is becoming the **main** backend. Direct Files is not retired.
   Nobody will adopt MS unless it is at least as fast and at least as trustworthy.
8. Tine is becoming a **collaboration surface** (the epics on the GH project) —
   AI agents first: MCP, a CLI, agents reading and changing the graph through the
   same semantic operations the UI uses; then people: subgraph sharing, E2EE
   serverless sync, and possibly ~1 s-latency collaborative editing later.
   Consequence for today's code: nothing may assume the UI is the only writer,
   this process the only instance, or this machine the only home of the graph.

## Part 2 — The ranking, when these conflict (DECIDED — Martin, 2026-09-01)

1. **Never lose or corrupt the user's data.**
2. **Be fast.**
3. **Keep the Markdown projection faithful.**
4. **Match Logseq's behaviour.**
5. Everything else.

**The rank-1 / rank-3 boundary, so two triagers agree:** it is **rank 1 (data
loss)** when a round-trip changes what the user wrote — content, block structure,
property values, or page/block identity — as *parsed*, i.e. Logseq or Tine would
show or resolve something different afterwards. It is **rank 3 (fidelity)** when
the bytes differ but parse to the same content: whitespace and formatting
normalization, property ordering, escaping style. Dropped metadata is rank 1 if
anything consumes it, rank 3 only if it is provably decorative. When in doubt,
triage at rank 1 and argue down.

Martin's rationale for 2 above 3: interop *means* maintaining the md projection;
in daily use he has hit speed problems repeatedly and interop problems never, and
the projection is the part guarded by an oracle harness and byte-level tests —
which is what makes this ranking safe. Consequence for triage: where fidelity and
speed genuinely conflict, speed wins by default and fidelity must argue its way
back in. Rank 1 trumps everything, always.

---

## Part 3 — The invariants

Each has a **Rule**, a **Specimen** (something real it would have caught — an
invariant with no specimen is speculation and is marked as such), and an **Ask**:
the exact question the reader answers for the file in front of them.

The reader reports facts against the rule. **The reader never decides whether
something is a bug** — that triage is the manager's. When one site violates
several invariants, report it under the **lowest-numbered** applicable invariant
and mention the others in that entry; do not repeat it.

### A. The user's data

**I-1 · No unaudited write path.**
*Rule:* every write into the user's graph goes through an audited protocol. The
canonical one is temp file → fsync → atomic rename → base-revision guard → lock;
append-only journals with fsync discipline, SQLite transactions, and no-clobber
directory publication are audited equivalents **when the code names the protocol
it is following**. The violation is a write path that follows no named protocol.
*Specimen:* the standing golden rule; the data-safety programme rests on it.
*Ask:* does this file write, rename, truncate or delete anything under the
user's graph or Tine's private durable state? For each site: which named
protocol carries it, or none?

**I-2 · The process may die between any two lines, and the graph survives.**
*Rule:* every multi-step durable transition — activation, rebuild, a move
transaction, conflict staging, format rewrite — has a defined answer to "what if
we crash between step k and k+1", and that answer is proven somewhere, not
intended.
*Specimen:* crash-mid-activation has been unproven on the clean runtime since the
08-15 switch; the four `public_activation_cut_*` proofs are contract-excluded.
Activation is what every adopting user runs exactly once, unattended, against
their real graph.
*Ask:* does this file perform a durable transition with more than one step? List
the steps. For each gap, what does recovery see? If the answer lives outside
this file (a recovery module, a test), say `CANNOT-DETERMINE` and name the step
— that is the expected honest answer, and collation matches it against the
recovery side.

**I-3 · One user intent is one operation to storage.**
*Rule:* a user action that changes more than one page reaches storage as **one
semantic request** whose scope storage can see — whatever its declared outcome
semantics (atomic, per-item, best-effort) are. The violation is **inference**:
several ordinary page saves sequenced by the frontend, leaving storage to guess
they belong together. The operation must exist **below the UI**.
*Specimen:* `src/carry.ts` — `markDirty` + `flushPage` per day, destination
first. Valid Direct Files choreography, invisible as a move to the managed
actor. This is the invariant the sweep was commissioned to find.
*Also why:* agents are coming (Part 1, item 8). MCP and the CLI must be able to
say "carry these tasks" as one call.
*Ask:* does anything here change state belonging to more than one page? Name the
user action it serves and the single operation that carries it. If the grouping
exists only as frontend choreography, say so — that is the finding. If you
cannot name the user action from this file, `CANNOT-DETERMINE` and name the
entry points you'd need.

**I-4 · What lands on disk stays Logseq-readable.**
*Rule:* files Tine writes are readable by Logseq, and round-trip without silent
change to content, block structure, property values, or identity (the Part 2
rank-1 boundary).
*Specimen:* the product premise; `title::` page identity, NFC/NFD twins, the
3-byte empty bullet, the duplicate-parser heading bug — all real, all found late.
*Ask:* does this file decide the **bytes** of anything in the user's graph? What
does it emit that Logseq must understand? If where-it-is-proven is not visible
from this file, record what you found and `CANNOT-DETERMINE` the proof.

**I-5 · The user's content stays on the machine and out of the logs.**
*Rule:* graph content, page titles and file paths do not cross the **machine
boundary** unencrypted (network, telemetry, crash reports, public artifacts) and
do not enter any **always-on log or diagnostic record**. Tine's own local IPC
(frontend ↔ backend commands) is inside the boundary. User-initiated export,
clipboard, and opening external tools are the user exercising ownership — fine.
Opt-in debug logs are the labelled exception.
*Direction note:* E2EE serverless sync (Part 1, item 8) sharpens this: content
may eventually leave, but only end-to-end encrypted; a relay sees ciphertext and
minimal metadata.
*Specimen:* none live — pinned **before** the diagnostics work (I-9) starts
adding recorded detail. Speculative by design.
*Ask:* does this file write to a log/recorder, build a network payload, or emit
an artifact that leaves the machine or persists as diagnostics? Could any field
carry user content? Name the field.

### B. Storage authority and format

**I-6 · Storage authority is selected in one place, then flows as a value.**
*Rule:* the decision "which authority governs this graph" is made once, at one
site, from configuration/state. Everything downstream **dispatches on the
authority value it was handed** — a backend adapter matching on an
already-bound authority is correct layering, not a violation. The violation is a
second site *deriving* the decision (from config, path shape, heuristics), and
any route on which a managed graph can reach Direct persistence.
*Specimen:* four independent cross-page-move forks in `store.ts` (audit UI-3),
plus Carry outside all four. A missed site degrades silently to Direct writing.
*Ask:* does this file branch on storage mode/backend/authority, by any name?
Quote the branch. Does it dispatch on a value it received, or decide afresh?

**I-7 · One current format — and private state the user can always walk away from.**
*Rule:* before 0.7, production code has exactly one Managed Storage format: no
dual readers, no legacy decoders, no in-place migration. Unrecognized private
state is preserved as a backup and rebuilt from the files. And the Part 1
promise: private durable state holds no **current user content** that, at
quiescence, exists nowhere in the user's files. Edit history, undo state and
provenance are legitimately private-only and sacrificeable (Martin, 2026-09-01).
*Specimen:* `LegacyLazyGenesisPageCapsuleV4`, live in `decode()` (audit M-09).
*Ask:* does this file read or write a private Tine format? Does it accept more
than one shape of it — name every version/generation/legacy branch. Does it put
**current** user content into private state? If so, what guarantees that content
also reaches the files, and where? (History/undo/provenance in private state is
fine — do not flag it.)

**I-8 · Storage refusals name the in-scope failure they defend against.**
*Rule:* scope: refusals in **storage, sync, and persistence lifecycle** paths —
not argument validation, plugin permission checks, resource limits, or
programmer-invariant guards. Wherever such code refuses an operation, fails
closed, or re-verifies state it already established, it names a concrete
in-scope scenario: crash/power loss, torn write, disk error, sync-service
delivery, external-editor race, honest concurrent instance, honest multi-device
divergence, malformed imported content.
*Specimen:* the Android UID/ownership check that made sync refuse to start,
defending only against an out-of-scope local attacker. A check with no in-scope
scenario is a future availability bug, not hardening.
*Ask:* does this file refuse/abort/fail-closed on a storage, sync, or
persistence path? For each site: what scenario does the code (or its comment/
refusal table reference) name? Record "names none" as the fact it is — whether
recovery should replace refusal is triage, not your call.

### C. Failure and recovery

**I-9 · A durable failure says which failure it was.**
*Rule:* a failure the user can see, or that survives a restart, carries enough
fixed, privacy-safe vocabulary to identify its family — in the always-on record,
without a debug relaunch.
*Specimen:* live. `trusted_local.preparation.finalize` names the phase that
failed, not what failed; Martin hit it and the shipped binary retained nothing
diagnosable. Also: the clean-open path flattens typed errors into `String` at
32 `map_err(display)` sites inside `open_clean_runtime_resources_with_progress`
and `RuntimeActor::from_clean_resources` (P-04a census; `sync_runtime.rs` has
86 such calls overall).
*Ask:* does this file produce, convert, or forward an error that can reach the
user or outlive the process? Does the type survive, or become a string?

**I-10 · Every state the user gets stuck in has a way out.**
*Rule:* any persistent state that blocks work — a retained conflict, an
unsaveable draft, a refusal capsule — offers a terminal action: resolve, retry,
or deliberately retire. Repeated identical failures deduplicate.
*Specimen:* live. A Concord live-save conflict survived restart, reported the
two versions identical, and offered no way to retire itself.
*Ask:* does this file create state that outlives the operation and can block the
user? What retires it? If the retire path lives elsewhere, `CANNOT-DETERMINE`
and name the state — collation matches creators against retirers.

### D. Whether the code tells the truth

**I-11 · The code does not lie about itself.**
*Rule:* comments, module docs and contract references describe what the code
does now. A claim of the form "X is not wired to Y", "deliberately inert", "only
used by Z", "retired" is **enforced** — by a test, or by compile-time structure
(visibility, types, exhaustive matching) — or it is deleted.
*Specimen:* contract §3.2 cites a type that does not exist
(`PendingLocalMutation`) and calls the **live** coordinator "retired legacy"
(both verified); the storage layer's "nothing here is wired" header survived
months after becoming false and mistrained every agent that read it.
*Ask:* what does this file **claim** about itself? For each claim: confirmed
true from this file, contradicted (say where), or unverifiable-from-here —
record the claim text either way; collation checks recurring claims. Do not
assume a claim is true because it is written down.

**I-12 · One question has one canonical answer.**
*Rule:* each domain question — a page's backlinks, a block's visible text, a
page's identity, whether a save may proceed — has **one canonical definition**;
everything else that computes it derives from, delegates to, or is tested
against that definition. Deliberate second computations (an oracle, a
frontend projection of a backend truth, a validator) are fine **when declared
as such**. The violation is independent reimplementation that can drift
silently.
*Specimen:* ~75 `application_*` functions re-implementing Direct-path logic
over a second type (count grew since August; exact number depends on the
counting rule — the growth is the point); two block-facet renderers that
diverge; multiple raw `SYS_renameat2` users across four files where exactly one
carries the Android errno classification.
*Ask:* what domain questions does this file compute an answer to? Name each in
plain words — that record is the deliverable; duplicates are found by collating
everyone's records, never by you guessing globally.

### E. What things cost

**I-13 · No unrequested whole-graph work on a path the user waits on.**
*Rule:* nothing on a keystroke, caret, render, scroll or save path does work
proportional to the whole graph **unless the user asked a whole-graph question**
(All Pages, global search, export, full open are output-proportional and fine).
*Specimen:* `referencedPageNames` — an O(graph) scan keyed to `dataRev`, rerun
after every typing lull, in a file already containing the memo pattern it
should have used. (Its neighbours `pageIconBatch`/`pageExistsBatch` cache per
graph state — the contrast is the point.)
*Ask:* does this file do work proportional to page/block/file count? What
triggers it, is a user waiting, and did the user's request imply that scope?

**I-14 · Cost tracks the graph, not its lifetime — or names its bound.**
*Rule:* operations may touch bounded recent history (an oplog tail, an undo
stack, unacked sync ops), but any cost that grows with **lifetime** edit count
must have a stated compaction/rebaselining bound that returns it to
graph-proportional. Unbounded lifetime growth is the violation.
*Specimen:* on a fixed 20-page graph, save+drain 18.4 → 23.3 → 32.0 ms and
crash reopen 128 → 663 → 1467 ms at 50/400/800 accepted batches — measured, MS
only; no fresh-corpus benchmark can see it, which is why a reader must.
*Ask:* does this file read, scan, replay or retain anything that accumulates
with edits? What bounds it, per this file? If the bound (compaction) lives
elsewhere, record the accumulation and `CANNOT-DETERMINE` the bound.

**I-15 · Count the expensive primitives.**
*Rule:* an operation invokes each expensive primitive — fsync, syncfs,
full-file rewrite, whole-tree scan, re-parse, re-render, IPC round-trip — a
number of times justifiable from its purpose. Recovery checks and durability
barriers may legitimately run without a change; the violation is *unjustified
repetition*. (Martin's addition: "is work getting duplicated?")
*Specimen:* the save path executed ~65 write barriers against a design budget
of 3; managed activation went 60 s → ~3 s, the difference being almost entirely
work that never needed to happen. Both were "working correctly" throughout —
only counting finds this class.
*Ask:* which expensive primitives does this file invoke, and how many times per
call of its own entry points — as visible **in this file**? Note loops and
per-item flushes. Callers' multiplicity is collation's job, not yours.

### F. Boundaries

**I-16 · A platform choice covers all five shipped targets.**
*Rule:* Tine ships Linux, Windows, macOS, iOS and Android. A platform
**decision** — the set of `cfg` branches that together select how one operation
is provided — must account for all five, across however many arms it is split.
A fallback arm must mean "genuinely not a Tine platform", never "one we
forgot"; partitioned arms (unix/windows, apple/android) are fine when their
union covers five.
*Specimen:* `tine-storage`'s `rename_noreplace` had no iOS arm through v0.10.0 —
every no-clobber publication failed on iOS, page creation included. It did not
fail to compile; it silently selected the unsupported fallback.
*Ask:* does this file make platform decisions? For each: which targets does the
union of its arms cover, and what falls to the fallback?

**I-17 · The plugin API exposes semantic verbs, never native surface.**
*Rule:* every capability is something the host performs on the guest's behalf.
Never `invoke`, raw filesystem or path handles, process, sockets, shell, OS
services, or a pass-through letting a guest name a native command.
*Specimen:* Apple review guideline 4.7.2 forbids plug-ins that extend or expose
native platform APIs *without prior permission from Apple*; Tine's recorded
product decision (ADR 0052) is to never depend on such permission, so one such
capability forfeits plugins on iOS under our policy, and a desktop-only escape
hatch does not help.
*Ask:* does this file define, forward or validate a plugin capability? For
each: a verb the host performs, or a handle the guest holds?

**I-18 · State that is meant to travel, travels; state that is not, says so.**
*Rule:* persisted state divides into **graph-portable** (sync payloads, oplog,
anything another device/process/day must reinterpret) and **device-local**
(window geometry, local paths, trust grants). Graph-portable state contains no
run-local pointers, no machine-local paths, no sole-writer assumptions.
Device-local state is fine — but it lives in a place or type that says so, and
never rides along inside graph-portable state.
*Specimen:* MS-09: persisted identity carrying state a later run rejected
(`projection_baselines`) forced a full store rebuild on the next open.
Deliberate counter-example: `settings.rs` persists canonical local paths as a
device-local trust grant — correct, because it is device-local by design.
*Ask:* does this file persist anything? Classify each piece portable /
device-local. For portable pieces, name every field that could not survive
another process, day, or machine.

### G. Added after the adversarial review (Sol, 2026-09-01)

**I-19 · One graph, two backends, one behaviour.**
*Rule:* where code branches on storage authority, both arms deliver the same
user-visible outcome; any intentional difference is named at the branch.
*Specimen:* the managed arm of the frontend's forks is exercised by 15 of 371
test files; `application_*` twins have already drifted.
*Ask:* for every authority branch in this file (see I-6): do the arms produce
the same outcome? Name observable differences you can see locally.

**I-20 · A late result cannot land on the wrong state.**
*Rule:* an async result completes into shared state only after proving it still
applies — a binding/generation/revision check against what it was computed for.
*Specimen:* the `bindingGeneration` / `expectedGraphBinding` machinery exists
precisely because stale completions once landed on the wrong graph; a new async
path that skips it reintroduces the class.
*Ask:* does this file complete async work into shared state? For each site:
what staleness check guards the landing, or none?

**I-21 · Everything acquired is released.**
*Rule:* watchers, workers, blob URLs, file handles, subscriptions, timers and
locks have a named owner and a release path on every exit, including error and
graph-switch.
*Specimen:* partially speculative — resource lifecycle is a standing release-
audit area and blob-URL/watcher ownership has needed care repeatedly, but no
single confirmed leak is on record here.
*Ask:* what does this file acquire? For each: where is the release, and does it
run on the error and switch paths?

**I-22 · Content from outside is hostile until proven bland.**
*Rule:* Markdown/Org from disk, pasted/imported content, plugin input and web
content are untrusted at the boundary: sanitized before rendering as HTML,
bounded in size/nesting before recursive processing, path-checked before
touching the filesystem. **The check lives at the point of consumption**
(render / interpret / dereference), never only at production: provenance is not
a waiver, because any graph may have been written or modified by other software
— sync, import, sharing, another editor. "Tine sanitized it when writing"
protects other consumers of our output; it protects us not at all.
*Specimen:* GH #16 — raw HTML rendering required DOMPurify (the export-side
sanitization added there protects downstream consumers, not import); lsdoc v2's
fail-safe-on-untranscribed-input design exists for exactly this.
*Ask:* does this file accept content that originated outside Tine? Where does
it get sanitized/bounded/path-checked — in this file, or provably upstream **on
the consumption path** (a render gateway it must pass through — sanitized-at-
write-time does not count), or nowhere you can see (`CANNOT-DETERMINE`, name
the input)?

---

## Part 4 — The two open channels

**W · "I cannot explain why this is here."**

The invariants above can only find what we already knew to look for. That is
precisely the failure the sweep exists to fix, one level up. So every reader
also records, freely: something that surprised them and resists explanation;
code apparently unreachable, or reachable only from tests; two things that look
like they should be the same and are not; a comment whose rationale no longer
matches the code; anything they would ask a colleague about.

A W entry needs no invariant and no theory — file, lines, one honest sentence.
`- none` is a legitimate answer **for a file**. But a whole *bundle* with zero W
entries is a signal to re-check the reader, not to celebrate — never invent
confusion to fill quota; the check is on the reading, not the count.

**P · "What here would make a seasoned engineer wince?"** (Martin's addition)

W is for what the reader *cannot explain*. P is for what the reader can explain
perfectly well **and a professional would still not do it this way**: a
hand-rolled version of a solved problem, error handling that swallows, a clever
thing where a boring thing was available, a structure that fights the language.
One sentence per wince; `- NOTHING` when honest. P entries are triaged as
**leads, never findings**.

## Part 5 — What each reader returns

Each verdict file begins with a header:

```
bundle: B0NN
commit: <git rev-parse HEAD of /aux/koutecky/logseq/tine-master>
invariants: v1.0
date: YYYY-MM-DD
```

Then one block per assigned file, in manifest order:

```
=== FILE: <repo-relative path> (<manifest line count> lines)
purpose: <one sentence; prefix `[tooling]` for examples/benches/bins/devtools.
          If you cannot state it: "CANNOT STATE:" + why — itself a finding.>
read: FULLY | FULLY (production; test region lines N–M not judged) | PARTIALLY (<what, why>)
I-1: <VERDICT>
...all 22, in order...
I-22: <VERDICT>
W:
- <file:lines — one sentence>   (or `- none`)
P:
- <file:lines — one sentence>   (or `- NOTHING`)
```

`<VERDICT>` is exactly one of (all are **claims about this file only**):
- `N/A` — the invariant's subject matter does not occur in this file.
- `PASS — <one sentence naming what you checked>` (sentence required; a bare
  PASS is box-ticking and fails validation).
- `VIOLATION — <file>:<line> — <one sentence>`. Multiple sites: the scalar line
  carries the worst/first site; add further sites as indented `  - file:line —
  sentence` continuation lines beneath it.
- `CANNOT-DETERMINE — <what you would need to know>`; add `<file>:<line>` when
  the uncertainty is site-specific, omit when it is an external proof/caller/
  product question.

Severity precedence when one invariant has mixed sites in one file:
VIOLATION > CANNOT-DETERMINE > PASS. Continuation lines carry the rest.

Inline `#[cfg(test)]` modules and `*.test.*` content inside an assigned file
are **out of scope for verdicts** (note their line range on the `read:` line) —
except under I-11, where a test enforcing a claim is exactly what you cite.

**The deliverable of this sweep is the grid — what has been read against what —
not the findings.** Findings age; coverage compounds. A guessed verdict poisons
the grid; `CANNOT-DETERMINE` never does.

## Part 6 — Scope

**In:** every file in `manifest.tsv` — 385 files, 346,193 raw lines (of which
~275k are production once inline test regions are excluded; the three largest
files carry large `#[cfg(test)]` regions that readers note and skip). The
manifest includes examples, benches, `src/bin` utilities and devtools; readers
tag these `[tooling]` in `purpose:` — they are swept for I-1/I-5-class risks
and W/P, and their cost findings are triaged accordingly.

**Out:** dedicated test files, generated code, vendored dependencies,
`target/`, `src-tauri/gen/`.

**The three monsters:** `model.rs`, `oplog/hot_engine.rs`, `sync_runtime.rs`
hold ~74k production lines (27%). Each is a solo bundle; each verdict block may
take hours of reading. That is the price of the first complete read.

**What the sweep is and is not:** it replaces the *sampled reading* that audits
have done until now — it is how we know what has actually been read. It does
not replace call-graph analysis, runtime measurement, security mechanism
review, or integration testing; I-13/14/15 findings from reading are candidates
until measured, and cross-file questions resolve at collation, not in any one
verdict.

**Never:** `~/research/brain`, and no content from `~/research/logseq-anonymized`
in any artifact this sweep produces.
