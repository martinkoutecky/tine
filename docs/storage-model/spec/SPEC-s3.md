# og storage — specification s3.3: page rename, delete and one host operation (2026-10-11; s3.1 adds `load`, §2; s3.2 the rename order, §3a; s3.3 `opApply`, §3b)

> **Publication note (2026-10-14).** This is the working specification as the project used it, published
> with three kinds of edits only: this note, removed host names, and private directory prefixes stripped
> from file references. Names such as `s3-lean7.md`, `s3lean6`, `logs/…`, `review/…`, snapshot or patch
> files refer to internal working records (prover/verifier notes, lane directories, run logs) that are not
> published; [`../README.md`](../README.md) summarises what they established. The published models and
> scenarios are in [`../quint/`](../quint/) and the Lean proofs in [`../lean/`](../lean/). Identify a model
> version by its SHA-256 prefix; the README maps every prefix to its published file.
>
> **File names in this text.** Here `storage-s3.qnt` / `scenarios-s3.inc` mean **s3.2** (`1e6ec36c` /
> `92a75904`), published as `quint/storage-s32.qnt` / `scenarios-s32.inc`. In `quint/`, `storage-s3.qnt` /
> `scenarios-s3.inc` are the original **s3** (`baaaeab4` / `b446ab25`), `storage-s31.qnt` is s3.1
> (`614f82a8`) and `storage-s33.qnt` is s3.3. The sentence below that the implementation conforms to s3.2 "until the host operation
> lands" is as of 2026-10-11; the host operation landed on 2026-10-12, and the conformance tests shipped in
> 0.8.0-beta.1 replay s3.3.
>
> **Later versions (added 2026-10-14).** The model has moved on twice since this text was last revised.
> **s3.4** (`quint/storage-s34.qnt`, `ddcc4332`) adds name-slot admission to `opApply`; **s3.5**
> (`quint/storage-s35.qnt`, `7ebed877`, scenarios `quint/scenarios-s35.inc`, `9200a284`) is the current model and
> differs from s3.4 only in the held-source guard of the cancelling Discard (the paragraph "Landed as s3.5"
> in §6 below). The model files and their scenarios are the specification; this text explains s3.3 and the
> history through the s3.5 guard. [`../README.md`](../README.md) has the lineage and the proof status; the
> complete Lean proof covers s3.5.

`storage-s33.qnt` (s3.3, scenarios `scenarios-s33.inc`) **is** the specification; this file explains it (sha256
prefixes in §6). `storage-s3.qnt` stays s3.2, the model the step-3b implementation conforms to, until the host
operation lands (s3.3 is s3.2 plus additions only, §3b). s3 = s2.1 plus page
rename with reference rewrite and page delete. s2.1 (`storage-s2x.qnt`) is s2 (`SPEC-s2.md`, model
`1c319429`) with one fix (§1a). Everything else in SPEC-s2 holds unchanged. Previous versions: `storage-s3.2026-10-09-pre-review.qnt` (before review 1),
`storage-s3.2026-10-09-pre-lean1.qnt` (before the Lean round 1 fix, §8).

Requested by the CLI session (`s2-requests-from-cli-and-refusal-rule.md` §3). Martin, 2026-10-09:
"model it … so the model behaves exactly the same outside those added operations". Decisions D4–D6 are the
defaults Martin accepted the same day (decision journal, 2026-10-09). Review 1: `s3-review.md` (codex Sol/high, "right with fixes"); §7 lists what it changed.

## 1. Conservative extension (the claim, and how it is checked)

On any run that uses none of the new actions (`opRename`, `opDelete`, `flushDel`), s3 behaves exactly like
**s2.1**: same states (ignoring the new fields, which stay empty), same enabled steps, same guarantee verdicts.
Relative to s2 itself it is not exact: s2.1's `observe` fix (§1a) adds runs (every s2 run is still an s3 run).

Changes to s2.1's text, each the identity on such runs (new state `trash`, `trashStable`; new ghost `opRead`,
`removed`, `delDurable`; all start empty and only the new actions put anything in them):

| Change | Why it is the identity without the new actions |
|---|---|
| `PAGES` may have more than two paths; paths 0 and 1 start as in s2, the others with no file | with `PAGES = {0,1}` the initial state is s2's |
| `power`, `wOp` (`wOpTo` for any two distinct pages), launch's version numbering, mutant MDO and two TLC witnesses are written for any number of paths | the same transitions on two paths (`power(k0,k1)` = `powerK` of the kept set) |
| the save's file step (`rename`) moves the removed bytes to the trash, and records them in `removed`, when the save writes "no file" | only `flushDel` starts such a save |
| `dirSync` makes the trash durable with the path; `power` reverts it with the path and forgets removals it undid | the trash and `removed` are empty |
| G also accepts replacing bytes a page operation computed its text from (`opRead`) | `opRead` is empty |
| `guarantee` gains the conjunct `trashed` (§3) | `removed` is empty, so it is true |

Evidence so far: `cons.py` runs all 88 s2 scenarios on s2.1 and on s3 restricted to two paths, under every
profile × mutant (FULL=1), and fails on any difference in outcome. That compares scenario outcomes only; the
claim about states and enabled steps is the target of the Lean conservativity theorem (round s3lean1).

## 1a. The s2.1 fix (found while writing s3)

s2's `observe` was enabled only when the bytes read differ from the page's last read (`obs`). Tine's own save
does not update `obs`, so after a save of new bytes, another program writing back exactly the bytes read
before was never applied: the model skipped runs a real watcher has. s2.1 enables `observe` whenever the read
changes the page. The new guard is implied by the old one, so s2.1 has every s2 run and more. The whole s2 Lean
proof rebuilds unchanged against it (`obsx/`: 54 modules, 540 theorems, standard axioms, no `sorry`),
and the 88 scenarios pass. Implementation rule: apply every read through the table; never skip a read because
it equals the last read.

## 2. The operations

A page operation is requested by a client (the window or the CLI) and applied by the backend **in one step**,
under the lock of every page it changes; it waits while a save holds one. **s3.1:** it reads only pages the
backend holds. The backend first **loads** each page it does not hold, as a separate step per page (`load`: one
read of its path; the page becomes held and clean, like an open with no window). s3 read an unheld page from
disk inside the operation's own step, which an implementation cannot do atomically against another program
writing the path between its read and its durable draft install (step-2 lane counterexamples,
`LANE-2a-M3-BLOCKER.md`, `A1-CHECK.md`). An external write after the
load is an ordinary write to a held page: the operation keeps the loaded base, and the page's later save meets
the guard. Mutant MUL restores s3's read-inside-the-step (a comparison, not a guarantee breach). Its new texts are **ordinary edits**, saved later
by ordinary guarded saves: no operation record, no multi-file transaction, no undo.

**Rename** `src → dst`, rewriting references in the referring pages `refs`:
- The page's text moves to `dst` as in s2's move; `src`'s new text is "no file" (its save moves the file to the
  trash); each referring page gets its rewritten text.
- **Every page the operation changes is drafted when it is applied** (`dst`'s text, each rewrite, and `src`'s
  deletion), and acknowledged (promised). A crash at any later point therefore
  loses nothing and leaves nothing half-done: launch restores every page of the operation, and their saves
  complete it.
- **All of an operation's drafts are written as one file** (one versioned record per page; launch takes each
  page's newest record, whether from that file or a later per-page draft). A crash or a failed write leaves
  all of them or none; a failed write refuses the operation. This is what makes the model's single step
  faithful (review 1, finding 3). It records the pages' new states, not the operation.
- `dst` must be free when the operation is applied: no file and no text.
- If `src` has no file, only the references are rewritten (re-running a rename whose page has already moved,
  e.g. to repair a reference another program wrote back).
- Out of scope for s3: merge into an existing page (`merge_into`), and moving namespace descendants (several
  `src → dst` pairs in one operation). The page's own title rewrite is a deterministic edit of the moved
  text; the model moves the text unchanged.

**Delete** `p`: the page's new text is "no file"; its save (`flushDel`, then the usual guard check, file step
and directory sync) moves the file to the graph trash in one rename, made durable by the directory sync
(of both directories). The deletion is drafted and acknowledged when applied, like every page an operation
changes: a crash before its save completes it at launch.

**D4 (clean first).** Every page an operation would change must be clean: the client saves first, and until
then the operation is refused. A refusal changes nothing, so it is not a step. This includes referring pages
(as Beta does: `WouldBlock`). A page in conflict is never clean, so it blocks the operation until resolved.

**D5 (interruption).** No intent record. With every changed page drafted in one file, an applied operation
always completes after a crash; one lost before it was applied has no effect.

**D6 (trash).** A delete moves the bytes it removes to the trash, including bytes another program wrote in
the R1 window between the guard check and the file step.

## 3. Guarantee additions

s2's A, B, B′, C, G and `accepted` apply unchanged to every step, including the new ones. In particular:
- **A** covers every page an operation changes (the moved text, every rewrite, and every deletion): each is
  promised when applied (as s2's move receiver).
- **B**: the operation may replace `src`, `dst` and the clean referring pages (`mine`), so B does not check
  them; D4 is pinned by its own tag instead (below). B still checks any other page, and a referring page that
  is not clean (mutant MRN2 is caught by B and by the D4 tag).
- **G** gains one exemption: a save may replace bytes that a page operation read and computed the page's new
  text from (`src` and referrers: their read bytes; `dst`: only "no file"). Without it, every CLI rename of a
  page never shown in the window would be flagged.
- **New transition tags:** `D4-unclean-page-changed` (an operation changed a page that was not clean) and
  `rename-clobbered-target` (a rename's `dst` existed). Neither relies on G or B.
- **New state invariant `trashed` (D):** bytes a Tine delete removed from a path are at that path or in the
  trash, across power cuts. A delete the power cut undid (not yet durable) no longer counts as removed: its
  bytes are back at the path if they were ever durable there (otherwise they were another program's unflushed
  write, the accepted race).

**What s3 does not prove** (implementation obligations, §4): that each rewrite is the right rewrite of the
bytes read (the model's `rt` is arbitrary), and that every referring page is found (its `refs` is arbitrary).
s3 proves custody: nothing acknowledged is lost, nothing unseen is overwritten except by an operation's own
computed text, deleted bytes reach the trash, and the target is never clobbered.

Known, accepted effects:
- The backend cannot see input the window has not sent yet, so D4 cannot wait for it. Input the window typed
  on the older version of any page an operation changed, sent afterwards, becomes a stale conflict (s2 §3)
  whose newer text supersedes the operation's text there: a referring page keeps an old reference (re-running
  the rename rewrites it); a deleted or renamed-away page comes back with the user's text. The user sees the
  conflict. Because the operation's text was acknowledged, B accepts this replacement (Lean round 1, §8).
- R1, as for every save (SPEC-s2): another program creating `dst` between its save's guard check and file
  step is replaced. Candidate s2.2 (all saves that create a file): publish with no-replace where the platform
  has it (`renameat2(RENAME_NOREPLACE)`, `link`), keeping the current path elsewhere (Android FUSE refuses
  no-replace rename). Not decided.

## 3a. s3.2: a rename's in-run order (Martin 2026-10-10, option 2)

**Guarantee (new, beside A–G, which it does not widen).** While Tine runs, a host rename never trashes `src`
before `dst` and the rewritten referrers have published durably: the order is `dst`, then the referrers, then
the `src` deletion. A page's witness is its own save's `Published` at the operation's version or later (its
directory sync completed), never a file step, an observation or a version advance. Model: in-memory `gates`,
installed by a full `opRename`, cleared by `down` (launch starts with none); `flush` and `flushDel` do not start
a gated page's save. D4 gains "no chaining": an operation over any page of a running order (witnessed pages
included) waits until that order ends (`touches`). A successful Discard of the unwitnessed destination cancels
the order: the source, if its buffer still holds the operation's deletion, takes its disk bytes; newer input
typed into the source stays. A discarded referrer leaves the
order.

Checked by two invariants beside `guarantee`:
- `orderHolds`: ghost obligations `g.obl`, recorded by a full `opRename` (or an `opApply` move) independently of
  the gates, checked at every save start. Publication retires the dependencies it witnesses (at the required
  version or later); a successful Discard retires every obligation involving the discarded page, and for an
  unwitnessed destination cancels the whole order; `down` clears them;
- `renameResolves`: for each pair still in `g.renamed`, a file exists at its source or its destination. A full
  rename (or move) records its pair when its source has a file; a later external write or operation on either
  path retires the pair; `down` clears the set, so it is checked between crashes and power cuts only.

**Across a crash, power cut or graph switch:** A (no loss) and completion at relaunch only, with no order.

Known, accepted effects (add to §3's list):
- A crash during a rename whose destination write is also failing can leave the page with no file until the
  write succeeds. Its bytes are in the trash and in the drafts. This is user-visible: the Guide has a line.
- The order is not durable: after a relaunch, the source deletion may publish before the destination.
- Across a graph switch, completion only: the switch stops the host, which ends the gates.
- A delete holds the page read lock for up to 10 s (the R1 bounded wait) before it answers Uncertain.
- Window close (J6): the window implementation narrows close; the model keeps the broader close so MCL stays
  checked. Window observation is not modelled.

## 3b. s3.3: one host operation, `opApply` (Martin 2026-10-11: the page host is the only page writer)

Every feature that changes pages (merge, trash, a Guide page, highlights, a sync-conflict fold, and so on) does
it through the page host as one operation (design: the internal host-only-writer design, §3.1).
In the model this is `opApply(eff, plan, dst, src)`:
- `eff` lists the pages it changes and their new texts, where ABSENT means a deletion;
- `plan` gives the bytes the feature planned on; admission checks the pages hold them. How the feature computes
  `eff` from them is abstract (`eff` is arbitrary, like `opRename`'s `rt`);
- `(dst, src)` is an optional move designation: `src`'s effect is a deletion and `dst`'s is a text, and the move
  installs the rename order. That `dst`'s new text holds `src`'s text (whole for a rename, merged for a merge)
  is the feature's obligation (§4), not checked by the action. With `dst` holding a file it is a merge; with no
  file there, a rename with no rewritten reference.

**Admission** is `opRename`'s, plus the plan:
- every page it changes is held (loaded first) and clean (D4);
- no page is in a running order (no chaining);
- every page holds the bytes the feature planned on. A stale page is refused (`Changed`; the feature reads
  again and replans). A refusal changes nothing, so it is not a step.

**Effect:** the new texts are drafted in one step and acknowledged, as `opRename`'s, so an interrupted operation
completes at launch. A move installs `opRename`'s order with `dst` as destination and `src` as source: the other
pages after `dst`, and `src`'s deletion last. A Discard of the unwitnessed receiver cancels the move. An
operation with no move has no order: none of its new texts holds text it removes from another page.

G gains nothing new: a page's text was computed from the planned bytes, which `opRead` records (as for `opRename`'s
reads). New transition tag: `op-stale-base` (an operation replaced bytes other than those its feature planned on).

s3.3 is s3.2 plus additions only: the action, its disjunct in `step`, and new mutant names. Every s3.2 action is
textually unchanged, so runs without `opApply` are s3.2 runs by construction. `opRename` and `opDelete` stay as
they are; a rename with no rewritten reference and a delete are also `opApply` instances (scenarios
`ownApplyRename`, `ownApplyDelete`). Conformance maps the host's rename operation to `opRename` and every other
host operation to `opApply`.

**Not checked in the model:** an operation with more than one move (the model has 3 paths). The implementation
generalises the order (receivers never wait; a page that is neither waits while any receiver is open; a source
waits while any non-source page is open) and pins it with its own unit test.

## 4. Implementation obligations (og; for conformance tests)

1. The operation's drafts are one file, written before the edits are applied; its failure refuses the
   operation (§2).
2. Each rewrite is computed under the page locks from the bytes the operation reads (the buffer if held,
   else the file), and changes only references to the renamed page.
3. Referring pages come from the current index plus every page the backend holds; a reference the index
   has not seen yet may be missed, and re-running the rename rewrites it.
4. Delete's file step is a no-replace move into the graph trash; the directory sync covers the page's
   directory and the trash directory; a name collision in the trash gets a fresh name, never a replace.
5. Every read is applied through the table (s2.1).
6. This replaces Beta's `tine-store` multi-file transaction (in-memory undo, "not crash atomic") for rename.
7. (s3.3) Every `HostOp`'s effects are computed from the bytes it names as its plan, and a feature that moves
   text from one page into another declares the move, with the receiver's new text holding the source's text.
   An op with more than one move follows the generalised order (§3b) under its own unit test.

## 5. Mutants (each caught by its scenario)

| Mutant | Fault | Caught by |
|---|---|---|
| MRN1 | `dst` not drafted | A, after the old file is deleted and the app crashes |
| MRN2 | a referring page that is not clean is rewritten | B and `D4-unclean-page-changed` |
| MRN3 | an existing `dst` is overwritten (one the window has shown, so G cannot object) | `rename-clobbered-target` |
| MRN4 | rewrites not drafted | A, after a crash |
| MRN5 | `src`'s deletion neither drafted nor acknowledged | a scenario assertion: after a crash before any save, launch does not restore the deletion, so the old file stays beside the new one (completion is not part of the guarantee) |
| MTS | the directory sync does not make the trash durable | `trashed`, after a power cut |
| MDT | delete removes the file instead of moving it to the trash | `trashed` |
| MDD | a deletion neither drafted nor acknowledged | B (the Lean round 1 counterexample: unsent window input, delete, stale submit) |
| MDF | (s3.2) the source deletion ignores its gate | `orderHolds` |
| MRF | (s3.2) a referrer ignores its gate | `orderHolds` |
| MRD | (s3.2) the source's wait omits the referrers | `orderHolds` |
| MPR | (s3.2) the witness is taken at the file step, not at `Published` | `orderHolds` |
| MCV | (s3.2) any save end (Uncertain, failure) counts as the witness | `orderHolds` |
| MEX | (s3.3) `opApply` does not check the planned bytes | `op-stale-base` |
| MAC | (s3.3) `opApply` changes a page that is not clean | B and `D4-unclean-page-changed` |
| MAN | (s3.3) `opApply`'s new texts are not drafted | A, after a crash |
| MAM | (s3.3) a move installs no order | `orderHolds` |

## 6. Verification status (s3.3: model `353162e99b1e8760`, scenarios `033fc5b7aac419a7`; s3.2: model `1e6ec36c29ec8fd1`, scenarios `92a75904b2da9b43`)

**s3.3 (2026-10-11), `storage-s33.qnt` `353162e9`, `scenarios-s33.inc` `033fc5b7`.** Written by the manager.
- Scenarios: 177 (s3.2's 159 plus 18 for `opApply`: merge, its order, the merge crash in both orders, the
  Discard of the survivor, no chaining, refusals for a dirty page, an existing create target and a stale plan,
  and trash and rename as instances) pass in all four profiles (`logs/sweep-s33.out`).
- Mutants: all 36 in `SWEEP-MUTS` caught (s3.2's 32 plus MEX, MAC, MAN, MAM).
- Conservativity vs s3.2: by construction (additions only; `diff` shows one changed line, the mutant list).
- Random simulation, invariant `guarantee and orderHolds and renameResolves`, 4 profiles × 4,000 runs × 60 steps:
  no violation (`logs/sim-s33.out`). Non-vacuity: witnesses for a running merge and for a no-move create are
  reached within 300 runs.
- **Lean round 8**: a partial result, shared with s3.2 below. Notes: `s3-lean8.md`.
  Verification by codex Sol/high (`s3-lean8-verify.md`): a fresh clean build of 235
  modules and 1,362 named theorems, an exhaustive axiom audit (standard axioms only), and replay of 7,080
  outcomes plus 8,624 complete-state observations. Proved for s3.3:
  - **Exact conservativity.** A run without `opApply` equals the s3.2 run (`no_opApply_runs_exact_s32_8`).
    Old-only runs equal s2.1 runs, finite and infinite (`no_new_actions_are_s21_runs8`).
  - **No-move runs.** If no full rename is performed and every `opApply` is undesignated, then `orderHolds` and
    `renameResolves` hold (`noMove_order_resolves8`). This is vacuous about moves.
  - **One-step `opApply` lemmas** (admission, effects drafted, no new bad tag, promise, new custody), and
    **one-step preservation** of `guarantee` (`apply_guarantee8`, which has `guarantee x` as a premise).
  - **Open:** the guarantee, `orderHolds` and `renameResolves` over all reachable s3.3 states.

**s3.2 (2026-10-10, applied 2026-10-11), `storage-s3.qnt` `1e6ec36c`, `scenarios-s3.inc` `92a75904`.** Patch by
the step-3b lane (`storage-s3-s32.patch`, minus its `run.py` default; `run.py` now
takes the invariant from `INV`). Gates rerun by the manager:
- Scenarios: 159 pass in all four profiles; all 32 mutants caught, including MDF, MRF, MRD, MPR, MCV
  (`logs/sweep-s32.out`).
- Conservativity vs s2.1 (`cons.py`): 23 cases × 88 scenarios, 0 differences (`logs/cons-s32.out`).
- Random simulation, invariant `guarantee and orderHolds and renameResolves`, 4 profiles × 2,000 runs × 60
  steps: no violation (`logs/sim-s32.out`).
- **Lean round 8** (partial, verified; see s3.3 above). Replay: 5,724 outcomes, equal to Quint.
  - **Runs without a full rename:** the whole s3.1 guarantee holds (`noFull_guarantee8`, via
    `noFull_refines31_8`). That theorem derives round-7 reachability; it does not assume it.
  - **Open:** the guarantee, `orderHolds` and `renameResolves` with full renames. These need preservation
    through the gated flush and the two-page Discard.
  - **The prover's "InactivePages refuted" claim was false.** The probe trace is alive throughout, so the
    invariant holds.
  - **Model defect (low, verified).** The two-page Discard's source-restoration guard
    (`src != p && buf == ABSENT`) also fires on a retired, unheld source (NOPAGE). It rewrites that source's
    unheld buffer from disk and bumps its version. No disk, draft or trash effect follows, and all three
    targets hold on the trace. The intent (§3a) is to restore a *held* source whose buffer holds the
    operation's deletion.
    - Fix: add `held` to the guard.
    - Landed as **s3.5** (2026-10-14, `storage-s35.qnt`, model receipt not published): the guard also requires `held`; mutant MRH
      restores the old guard. 194 scenarios × 4 profiles, 38/38 mutants; s3.4 vs s3.5 over all 191 old scenarios, 152
      cases, 0 differences. Production already matched s3.5 (`page_host/order.rs` filters on the held-page map; this is
      a source reading; the conformance run against s3.5 on 2026-10-15, og `490465a70`, confirms it). Lean round 9 proved
      the literal s3.4; rounds 10 and 11 prove s3.5.


**s3.1 (2026-10-09), model `614f82a8`, scenarios `4b4e2720`.** Lane `s3x-quint` (notes
`s3x-quint.md`; base/none and base/MUL rerun by the manager, same result):
- Scenarios: 143 (the 120 below, 20 repaired with explicit `load` steps, all assertions unchanged, plus 23 new:
  six unheld-page refusals with six successful controls, delete/load/write guarding, rename + external target
  + power for all eight keep sets) pass in all four profiles.
- Mutants: all 27 caught by their named witnesses in base and R1; in weak and all, MTS is not caught (0 failures: those profiles permit a power cut to revert the trash, so what MTS breaks is allowed there), so 26/27 there. MUL fails exactly the six
  `ownUnheld*` refusals (QNT511) and no guarantee or outcome scenario.
- Conservativity vs s2.1: 80 pairs × 88 scenarios, 0 differences. Random simulation: 4 × 5,000 × 60, no
  violation.
- Lean (lane `s3lean7`, notes `s3-lean7.md`; **verified** by codex Sol/high,
  `s3-lean7-verify.md`, fresh rebuild + exhaustive axiom audit): the model's literal
  `guarantee` holds at every reachable state, for every P ≥ 1, texts 1..T with T ≥ 2, all four race flags
  arbitrary, mutant none; axioms only `propext`, `Classical.choice`, `Quot.sound`. Old-only s3.1 runs (no
  load/opRename/opDelete/flushDel) equal s2.1 runs, finite and infinite. `LoadWitness7`: load is reachable and
  enables delete (non-vacuity). Safety only (no liveness); abstract model, not the implementation. Two low
  documentation notes in the verifier's report (MRN5 sweep wording, CPU-pinning instructions); no proof defect.

The lines below are the s3 (`baaaeab4`) record.

- Scenarios: 120 (88 s2 + 32 s3) pass in all four profiles. Refusals use `.fail()`, checked to fail when the
  step is enabled.
- Mutants: all 27 in `SWEEP-MUTS` caught (s2's 19 + MRN1–5, MTS, MDT, MDD).
- Conservativity (`cons.py` vs s2.1): 0 differences on the final model. The FULL run (every profile × mutant,
  0 differences, `logs/cons-s3-full.out`) ran on the version before the Lean round 1 fix, which changed only
  the new operations.
- Random simulation: 4 × 5,000 runs × 60 steps, no violation. Weak evidence: 4 × 10,000 runs on the previous
  version missed the counterexample below.
- **Lean round 1** (`s3-lean1.md`, lane `s3lean1`, on the pre-fix model): executable transcription
  for every P, replay 3,510 outcomes = Quint; **conservativity proved** (whole runs, finite and infinite, both
  directions, same enabledness and guarantee verdict, every P, texts, profiles, any initial disk; mutant none);
  **`trashed` proved** for every reachable state; the **whole guarantee refuted** by two 7-step runs (§8),
  reproduced by me in Quint. 579 theorems, standard axioms only. Verifier (`s3-lean1-verify.md`, Sol/high): restatement faithful, no prover defect; both runs replayed in all four profiles with all 64 prefix states matching Lean; clean rebuild, axiom audit and replay passed.
- **Lean round 2** (`s3-lean2.md`, lane `s3lean2`, on the fixed model): transcription updated,
  replay 3,720 outcomes = Quint (27/27 mutants), the two former counterexample runs proved good at every
  prefix. Proved over all reachable s3 states: conservativity, `trashed`, C, B′(1), B′(2), `accepted`, the D4
  and no-clobber tags; and that the whole guarantee is equivalent to A plus no B tag plus no G tag. **A, B and
  G are open** (no counterexample), each reduced to obligations on named old actions. 738 theorems, standard
  axioms only. Verifier (`s3-lean2-verify.md`): no prover defect, partial results accepted.
- **Lean round 3** (`s3-lean3.md`): B's `launch` case closed (`B_launch3`); A, G and B on four old
  actions still open. Scope finding: the claim cannot hold for T = 1, because the initial disk holds text 2,
  which is then never recorded as seen, so G fails with no operation. This is a domain artifact, not a design
  defect, and it is fixed by claiming T ≥ 2 (every initial byte is a text). Verifier (`s3-lean3-verify.md`): no prover defect.
- **Lean round 4** (`s3-lean4.md`): B's `observe`, `check` and `switchFin` cases closed. B is now
  reduced to its stale-submit case of `deliverUp`; A and G are unchanged (open). No counterexample.
  Verifier (`s3-lean4-verify.md`): accepted; one low-severity wording overclaim (the byte-domain theorem
  does not cover the trash fields).
- **Lean round 5** (`s3-lean5.md`): **B closed** over every reachable state (T ≥ 2: `B_positiveTexts5`).
  The whole guarantee is reduced exactly (iff) to two obligations: A custody across crash, switchFin, power
  and extWriteD; and G authority at the save's file step. No counterexample. Verifier (`s3-lean5-verify.md`): no prover
  defect; B proved; 984 theorems audited.
- **Lean round 6** (`s3-lean6.md`, lane `s3lean6`): **the whole s3 guarantee is proved**,
  `Storage3.guarantee_positiveTexts6`: for every positive P, texts `[1..T]` with T ≥ 2, every race profile,
  mutant none, every reachable state, `guarantee = true`. Kernel-checked, standard axioms only, no residual
  hypothesis; transcription unchanged since round 2. Verifier (`s3-lean6-verify.md`, adversarial):
  ACCEPT, no findings (fresh rebuild, full axiom audit, 3,720 replays, 29,000 field/set comparisons).
- **Status: s3 is complete to the s2 standard.** Remaining trust: Lean model = Quint model is tested by
  replay, not proved; implementation conformance (SPEC-s3 §4). Not run: TLC on s3 (superseded by the proof),
  a second design review.

## 7. Changes from review 1

Accepted and fixed: `src`'s deletion is drafted (crash before any save no longer leaves an unrepairable
duplicate, finding 2); all of an operation's drafts in one file (finding 3); rewrite correctness and
referrer discovery stated as obligations, not claimed (finding 4); conservativity claimed against s2.1, with
`cons.py` hardened (finding 5); trash durability modelled with an invariant (finding 6); direct no-clobber and
D4 tags (guarantee section). Kept as accepted: the R1 race on creating `dst` (finding 1), the same race every
save has, with a no-replace publication noted as candidate s2.2.

## 8. Changes from Lean round 1

The Lean prover refuted the whole guarantee with two seven-step runs in every profile: the window has unsent
input on a page whose backend copy is clean; a delete (or a rename away) of that page is applied (D4 passes:
the backend cannot see unsent input); the window then sends its text on the old version, the stale submit
replaces the operation's "no file", and B flags it, because the deletion was not acknowledged (delete: not
drafted; rename: drafted but not promised). Rewritten referrers already behaved correctly: they are promised.
Fix: every page an operation changes is drafted and acknowledged, deletions included. The stale input
becomes a conflict over acknowledged text, which B accepts and A covers. The two runs are scenarios
`ownStaleAfterDelete` and `ownStaleAfterRename`; mutant MDD restores the delete fault.
Cost: one more draft record per delete.
