# Tine's storage model: specification, Quint models and Lean proofs

This directory holds the formal model of how Tine saves your pages, the evidence that the model keeps its
guarantee, and the scripts to check that evidence yourself. It covers the part of Tine that owns page buffers
and every file write: saves, drafts, crash recovery, external edits, page rename and delete, and one general
host operation. It does not cover the UI, search, or how fast text is saved.

- [`spec/`](spec/): the prose specifications. `SPEC-s2.md` is the base design; `SPEC-s3.md` extends it with
  rename, delete, `load`, the rename write order and the host operation. Each starts with a short publication
  note.
- [`quint/`](quint/): the [Quint](https://quint-lang.org/) models, one file per version, the scenario files,
  and the runners (`sweep.py`, `cons.py`, `run.py`). **The Quint model is the specification**; the prose
  explains it, and where they differ, the model wins.
- [`lean/`](lean/): a Lake project with the [Lean 4](https://lean-lang.org/) transcription of the models and
  the proofs. `src/Headlines.lean` states every headline theorem quoted below.
- [`replay/`](replay/): the Quint-versus-Lean outcome replay, the evidence that the Lean copy behaves like
  the Quint model.

## The guarantee in plain language

The model has a window (the editor, which sends typed text with the version it was typed on) and a backend
(which holds every page buffer and does all file I/O). A save is a guarded publish: write a temp file and
fsync it, check that the file still holds exactly the bytes the buffer is based on, rename, fsync the
directory. A page that is not safely saved is *at risk* and has a draft in app data. Launch restores every
draft before anything else touches the page. Against this, the model allows at any moment: app crashes,
window reloads, power cuts (which may undo anything not yet synced), graph switches, and other programs
writing, creating or deleting files, durably or not.

Under the assumptions below, the guarantee (`guarantee` in the models) says:

- **A, no loss.** The newest text Tine acknowledged for a page (by a published save, a durable draft, an
  applied move, or a page operation) survives any crash, power cut or graph switch: it, or a newer text Tine
  wrote, is at the page's path or in the draft store.
- **B, unsaved is never replaced.** A background step never replaces or drops a buffer holding text that is
  neither acknowledged nor equal to what is on disk.
- **B′, what the window sent is taken.** The window's text is replaced only once the backend has it.
- **C, no blind overwrite.** A Tine write replaces only the bytes its guard check found.
- **G, no overwrite of unseen changes.** A Tine save never replaces file bytes the user was never shown,
  unless Tine wrote them itself, across crashes too.
- From s3 on: **deleted bytes reach the trash** (`trashed`): bytes a Tine delete removed are at the path or
  in the graph's trash, across power cuts; and every page a rename or delete changes is drafted and covered
  by A, B and G like an edit. From s3.2 on the models also check **`orderHolds`** (while Tine runs, a
  rename's old page is removed only after the new page and every rewritten link are safely on disk) and
  **`renameResolves`** (a renamed page keeps a file at one of its two paths).

**Accepted, not losses** (SPEC-s2 §5, SPEC-s3 §3):
- R1: another program writing a file between Tine's guard check and its rename. Its text is overwritten. No
  portable OS mechanism closes this window.
- R3: a weak filesystem that reverts a save it acknowledged, on a power cut.
- An external write after Tine's published save (it wins).
- Text Tine never acknowledged: unsent input lost by a window reload, and text the backend accepted but had
  not yet saved or drafted when the whole app crashed. Also three narrow cases of text the user saw but Tine
  never acknowledged as durable, reverted by a power cut (SPEC-s2 §5 lists them).
- A duplicate left by an interrupted move: the moved text may end up in both places, never in neither.
- A refused discard still drops the unsent input it was asked to drop.

**Assumptions** (SPEC-s2 §4.7–4.8): the filesystem's rename is atomic, and after a power cut a file holds
either its old or its new bytes, never a torn mix; the draft store is durable (atomic, checksummed, synced);
page texts are opaque values compared by equality. The claim is made for ordinary local filesystems (the
"strong" profiles); other stores (Android/iOS document providers, network drives, stores without atomic
replace) run the same code on a best-effort basis.

**Race profiles.** Every check runs in four profiles: `base`, `R1` (adds the race above), `weak` (adds R3),
and `all` (both). In the Lean theorems the R1 and weak switches, and whether crashes and power cuts occur,
are four independent arbitrary booleans.

**What is not modelled:** progress (how soon text is saved; the implementation's obligation is about one
second), torn or in-place writes, document-provider semantics, finding every page that refers to a renamed
page and computing the right rewrite (the model takes both as arbitrary inputs), and anything about the UI.

## Version lineage

| Version | Date | File (SHA-256 prefix) | Scenarios (prefix) | What changed |
|---|---|---|---|---|
| s2 | 2026-10-08 | `storage-s2.qnt` (`1c319429`) | `scenarios-s2.inc` (`f6efe526`), 88 | Base design: buffers, guarded saves, drafts, moves, launch recovery. |
| s2.1 | 2026-10-09 | `storage-s2x.qnt` (`b0b9322f`) | `scenarios-s2.inc`, 88 | One fix: a file read is applied even when it equals the last read (SPEC-s3 §1a). |
| s3 | 2026-10-09 | `storage-s3.qnt` (`baaaeab4`) | `scenarios-s3.inc` (`b446ab25`), 120 | s2.1 plus page rename (with reference rewrite) and page delete. |
| s3.1 | 2026-10-09 | `storage-s31.qnt` (`614f82a8`) | `scenarios-s31.inc` (`4b4e2720`), 143 | `load`: operations act only on pages the backend holds. |
| s3.2 | 2026-10-10 | `storage-s32.qnt` (`1e6ec36c`) | `scenarios-s32.inc` (`92a75904`), 159 | A rename's write order while Tine runs (gates; `orderHolds`, `renameResolves`). |
| s3.3 | 2026-10-11 | `storage-s33.qnt` (`353162e9`) | `scenarios-s33.inc` (`033fc5b7`), 177 | `opApply`: one general host operation that later features reuse. |
| s3.4 | 2026-10-13 | `storage-s34.qnt` (`ddcc4332`) | `scenarios-s34.inc` (`b322d41b`), 191 | Name-slot admission: a new page cannot take a name another file already claims. |

Every s3.x model behaves exactly like s2.1 on runs that use none of the page operations (checked by
`cons.py` for each version, and proved in Lean for s3, s3.1 and s3.3). s3.3 adds to s3.2 and s3.4 adds to
s3.3 without changing existing actions; s3.1 and s3.2 also tighten the s3 operations (held pages, the
rename order), so they are not extensions of s3 in that sense. Full SHA-256 sums
are in [`quint/SHA256SUMS`](quint/SHA256SUMS). Note that inside `SPEC-s3.md`, `storage-s3.qnt` names the
s3.2 model (its publication note explains the mapping).

## Status: what is checked and what is proved

"Scenarios" are hand-written runs, each with assertions, executed in all four profiles. "Mutants" are
deliberately broken variants of the model; each must be caught by its own scenario failing an assertion.
Every Lean result below was rebuilt and audited by an independent verifier (a separate model run with its
own fresh build); the verdicts are summarised, with their limits.

| Version | Model checking (Quint) | Lean proof | Shipped code |
|---|---|---|---|
| s2 | 88 scenarios pass in all 4 profiles; 19/19 mutants caught. Also, superseded by the Lean proof: inductive invariants checked with Apalache at 2 pages × 13 texts plus a handwritten cutoff argument, both independently verified. Bounded TLC runs were supplementary and are not part of the claim. | **Complete.** The whole guarantee holds in every reachable state, for any number of pages, texts 1..T, every profile (`Storage.guarantee_positiveTexts`, `Storage.noLoss_positiveTexts`). Verifier: accepted, 0 findings. Replay: 2,024 outcomes equal. Non-vacuity: six mutants reach kernel-checked real losses. | Superseded by s3.x. |
| s2.1 | 88 scenarios pass in all 4 profiles. | **Rebuild, not separately verified.** The s2 proof rebuilds unchanged against the s2.1 transcription (recorded in SPEC-s3 §1a, and rebuilt again for this publication; see "s2.1" below). No separate verifier report. | Superseded by s3.x. |
| s3 | 120 scenarios pass in all 4 profiles; 27/27 mutants caught; on old-only runs identical to s2.1 (0 differences); random simulation 4 profiles × 5,000 runs × 60 steps, no violation. | **Complete** for texts T ≥ 2 (the initial disk holds two texts; a modelling artifact): `Storage3.guarantee_positiveTexts6`. Conservativity: old-only runs are exactly s2.1 runs, finite and infinite. Deleted bytes reach the trash: `Storage3.trashed_reachable`. Verifier: accepted, no findings. Replay: 3,720 outcomes equal. | Superseded. |
| s3.1 | 143 scenarios pass in all 4 profiles; 27 mutants caught (in `weak`/`all` MTS is not caught because those profiles allow the trash revert it causes; the sweep checks it in `base`); conservativity vs s2.1 80 cases × 88 scenarios, 0 differences; simulation 4 × 5,000 × 60, no violation. | **Complete** (the latest complete proof): `Storage31.guarantee_positiveTexts7`, any P ≥ 1, T ≥ 2, every profile; old-only runs equal s2.1 runs (`no_new_actions_are_s21_runs7`, infinite versions too); `load` is reachable and enables a delete (non-vacuity). Verifier: accepted; two low documentation notes, no proof defect. Replay: 4,433 outcomes plus 572 for the heldness mutant, and 804 complete-state observations, all equal. | Superseded. |
| s3.2 | 159 scenarios pass in all 4 profiles; 32/32 mutants; conservativity vs s2.1 23 cases × 88, 0 differences; simulation 4 × 2,000 × 60 with `guarantee and orderHolds and renameResolves`, no violation. | **Partial.** Proved: the s3.1 guarantee holds on every run without a full rename (`Storage32.noFull_guarantee8`, which derives, not assumes, s3.1 reachability). **Open:** the guarantee, `orderHolds` and `renameResolves` on runs with full renames. Verifier: partial theorems accepted. Replay: 5,724 outcomes equal. Known model defect (low, verified): the two-page Discard's restore step also fires on an already retired, unheld source page and rewrites its unheld buffer; no disk, draft or trash effect and all targets hold on the trace; fix (add `held` to the guard) is pending with the next Lean round. | Its order is implemented (0.8.0-beta.1). |
| s3.3 | 177 scenarios pass in all 4 profiles; 36/36 mutants; additions only relative to s3.2; simulation 4 × 4,000 × 60 with all three invariants, no violation. | **Partial.** Proved: runs without `opApply` are exactly s3.2 runs, and old-only runs exactly s2.1 runs (`no_opApply_runs_exact_s32_8`, `no_new_actions_are_s21_runs8`); `orderHolds` and `renameResolves` on runs with no full rename and no designated move (`noMove_order_resolves8`, vacuous about moves); one-step `opApply` lemmas, including that one admitted step preserves the guarantee **given** it held before (`apply_guarantee8`). **Open:** the guarantee, `orderHolds` and `renameResolves` over all reachable s3.3 states. Replay: 7,080 outcomes and 8,624 complete-state observations equal. | **The model the shipped code is tested against** (0.8.0-beta.1 and current Beta): the save engine's conformance tests replay all 177 scenarios in each profile, in lockstep. |
| s3.4 | 191 scenarios pass in all 4 profiles; 37/37 mutants; conservativity vs s3.3 over all 177 old scenarios, every mutant in every profile (148 cases), 0 differences; simulation 4 × 4,000 × 60, no violation. These come from the modelling run's receipt (scenarios, mutants and conservativity were rerun for this publication); no independent review. | **None.** (A Lean round for s3.4 is in progress and not published.) | The name-slot check is implemented and covered by API tests; replaying s3.4 in the conformance tests is an open follow-up. |

**Exact scope of the complete proofs.** "Every reachable state" means every state reachable from the
model's initial state by any sequence of enabled actions: any number of pages P ≥ 1, texts 1..T (T ≥ 2 from
s3 on), no bound on history length, versions, queues or external writes, every profile, with the unmutated
model. These are safety theorems (nothing bad ever happens); there is no liveness or progress claim. The
kernel checks them with only Lean's three standard axioms (`propext`, `Classical.choice`, `Quot.sound`).

## The trust gaps

The chain is: prose spec → Quint model (checked) → Lean model (proved, complete up to s3.1) → Rust code
(tested against the model, not proved). Two links are evidence rather than proof:

1. **Lean copy = Quint model** is checked by replay, not proved. The Lean transcription is executable; for
   each version, every scenario is run in both under every profile and mutant and the outcomes must agree
   (`replay/`), and for s3 to s3.3 complete states along scenario traces were also compared field by field. Reviewers checked the
   transcription line by line. There is no proof that Quint's interpreter and the Lean definitions agree.
2. **Code = model** is checked by conformance tests, not proved. The save engine is an explicit state
   machine; its tests replay every model scenario in lockstep in each profile, plus random walks, on a
   simulated filesystem that drops unsynced data at a power cut, compare the code's state with the model's
   after every step, and run deliberately broken variants that must be caught.

The model also assumes the filesystem properties listed above, and takes reference discovery and rewriting
as given.

## How to check it yourself

### Quint (model checking)

Requires Node.js (tested with 20) and Python 3. Quint is pinned to 0.33.0.

```sh
cd docs/storage-model/quint
npm ci                                                     # installs Quint 0.33.0 into ./node_modules
QNT=storage-s33.qnt SCEN=scenarios-s33.inc python3 sweep.py # all scenarios x 4 profiles, every mutant
python3 cons.py storage-s2x.qnt storage-s33.qnt scenarios-s2.inc --two-pages   # s3.3 vs s2.1 on old runs
python3 cons.py storage-s33.qnt storage-s34.qnt scenarios-s33.inc --full       # s3.4 vs s3.3, every mutant/profile
QNT=storage-s33.qnt SCEN=scenarios-s33.inc INV='guarantee and orderHolds and renameResolves' \
  python3 run.py base none 4000 60                         # random simulation (s3.2+: three invariants)
```

Use the model/scenario pairs from the lineage table (s2.1 uses `scenarios-s2.inc`). For s2 to s3.1 the
simulation invariant is `guarantee` (the default). Generated variants and logs go to `quint/.out/`
(override with `OUT=`); the model files are never modified. A full sweep took between about 3 and 12
minutes per version here (one core per Quint process).

### Lean (the proofs)

Requires the Lean toolchain `leanprover/lean4:v4.31.0` (pinned in `lean/lean-toolchain`; with
[elan](https://github.com/leanprover/elan) installed, `lake` picks it up). No other dependencies.

```sh
cd docs/storage-model/lean
lake build          # builds all 149 modules; Headlines.lean prints the headline statements and fails the
                    # build if their dependency cone uses any axiom beyond propext/Classical.choice/Quot.sound
python3 audit.py    # prints the axioms of every named theorem (1,342) and checks them; scans for sorry etc.
```

A clean build took between 1.5 and 7 minutes on four cores here (depending on load) and needs about 1.7 GB of memory. `src/Headlines.lean` lists each headline with its
statement; the module names in the table below locate the proofs.

| Version | Headline theorems | Proof entry points |
|---|---|---|
| s2 | `Storage.guarantee_positiveTexts`, `Storage.noLoss_positiveTexts` | `Storage.lean` (transcription), `Guarantee.lean`, `FinalA.lean`; non-vacuity `Nonvacuity*.lean` |
| s3 | `Storage3.guarantee_positiveTexts6`, `Storage3.initialized_old_runs_exact`, `Storage3.trashed_reachable` | `Storage21.lean`, `Storage3.lean`, `GClosure6.lean`, `Conservativity3.lean`, `Trash3.lean` |
| s3.1 | `Storage31.guarantee_positiveTexts7`, `Storage31.no_new_actions_are_s21_runs7` | `Storage31.lean`, `Guarantee31.lean`, `Conservativity31.lean`, `LoadWitness7.lean`, `Scope7.lean` |
| s3.2 (partial) | `Storage32.noFull_guarantee8` | `Storage32.lean`, `Partial8.lean` |
| s3.3 (partial) | `Storage33.no_opApply_runs_exact_s32_8`, `Storage33.noMove_order_resolves8`, `Storage33.apply_guarantee8` (one step, assumes the guarantee before) | `Storage33.lean`, `Conservativity8.lean`, `Partial8.lean`, `Apply8.lean` |

`NonvacuityA.lean`, `Verifier7.lean` and `Verifier8.lean` were written by the independent verifiers
(reachable counterexamples for the mutants, and witnesses that the restricted theorems are not vacuous).
The file names carry the proof round that produced them; Lean namespaces carry the model version
(`Storage` = s2, `Storage21` = s2.1, `Storage3` = s3, and so on).

**s2.1.** The published s2 transcription (`Storage.lean`) is s2. The s2.1 rebuild changes only the
`observe` guard; to repeat it, replace in `src/Storage.lean`

```lean
  | .observe p => x.s.alive && (x.s.pages p).held && !locked x.s p && x.s.disk p != (x.s.pages p).obs
```

with

```lean
  | .observe p => x.s.alive && (x.s.pages p).held && !locked x.s p &&
      table c (x.s.pages p) (x.s.disk p) (x.g.vc + 1) != x.s.pages p
```

and run `lake build`. (s3 and later use their own s2.1 core, `Storage21.lean`, which already has this
guard.)

### Replay (Lean copy versus Quint model)

After `lake build` and `npm ci`:

```sh
cd docs/storage-model/replay
python3 compare.py s33     # or s2, s3, s31, s32
```

`replay*.py` translate the scenario files into Lean programs (written to `replay/.out/`) and `compare.py`
runs both sides under every profile and mutant and requires identical outcomes. The complete-state
comparisons quoted above are not included in this package.

## Provenance

The models, scenarios and Lean sources here are byte-identical to the files the cited checks and
verifications ran on (the SHA-256 sums match those recorded in the specs and verifier reports). The Lean
package is the union of the complete s2, s3 and s3.1 proofs and the partial round-8 proofs (the round-8
proof tree contains the earlier rounds unchanged), restricted to the modules the headline theorems depend
on, plus the three verifier files. Generated replay states and intermediate proof attempts were left out.
Comments in the model files occasionally name internal working records (review notes such as
`subagent-tasks/notes/s3-review.md`, or `og/merged/storage-s3.qnt` for the previous version); those records
are not published, and the files are left unedited so that their hashes still match the evidence.

The process that produced this (Quint first, then Lean proofs, each round checked by an independent
verifier) is described in the announcement discussion. Decisions referenced as "Martin, 2026-10-xx" are the
project owner's recorded design decisions.
