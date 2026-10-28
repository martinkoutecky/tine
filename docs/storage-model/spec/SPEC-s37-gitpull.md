# og storage — model s3.7-gitpull: the git pull of ADR 0078 §4 (2026-10-28)

`quint/storage-s37-gitpull.qnt` (scenarios `quint/scenarios-s37-gitpull.inc`) is s3.6 plus a git layer and
the four steps of ADR 0078 §4 ("Applying a pull") as separate actions. The other s3.6 actions stay in the
same next-state relation, so typing, window delivery, external writers, crashes, power cuts and graph
switches can fall between any two pull steps. This note says what is modelled, what is abstracted, how the
ADR text was read, and what the checks found. The model and its scenarios are the specification.

**Status: a proposal.** The unmutated model is the ADR **plus the proposed repairs P1, P1b, P2, P3 and
P4**. Each literal ADR rule (or, for P1b, today's code) is a named mutant, and each one breaks a checked
property with a concrete trace (§4). None of the repairs is decided; they need the manager's review
and, where they change the ADR, Martin's ruling.

## 1. What is modelled

- **Git layer** (`Sys.git`): `HEAD` as one of H (0), the fetched U (1) or any other tip (2); the trees of H
  and U over `PATHS` = the three s3.6 pages plus path 3, one non-page file (an asset); the index; whether
  `index.lock` exists and whether a live Tine holds it (the file survives a crash, its holder does not);
  the checked-out branch; the last commit's tree; the non-page file's bytes and its durable bytes; the
  index's durable copy.
- **The pull's steps** as actions: fetch (`gitFetch`, five upstream trees including one rename, one
  create/delete mix and one ineligible tree), the sync cycle's decision (`gitCycle`: up to date / diverged /
  resume / autocommit first), the pre-pull autocommit, (1) pre-check, (2) page `HostOp`s through s3.6's
  `opApply` with `Expected::Bytes(H blob)` (`plan`), `Input::Refuse` (opApply admits only clean, held
  pages) and one move per op, (3) the non-page file's checked atomic write in four steps (temp written,
  pre-rename check, rename, directory sync), (4) index.lock, ref compare-and-swap and index install.
  `gitHeld` ends a cycle at any busy or refused answer; an `Uncertain` directory sync of a pulled page or a
  failed non-page write ends it as `pull-incomplete`.
- **The environment's git**: the user staging a blob (`git add`), the user's plain `git commit` (records
  the index), removing a stale `index.lock` after a crash (Tine never does, ADR §1), another program
  moving the branch or checking out another branch, and another program writing the non-page file (R1:
  also between its check and its rename).
- **The other autocommits** (§3: cadence, background/quit, structural brackets) as `gitCadence`, outside a
  pull (the run lock is the pull's phase).
- Power cuts keep or revert each of the four paths and the index (`powerK(keep)`, keep ⊆ {0,1,2,3,4}).

## 2. Abstracted

Fetch and transport (U and its blobs exist when `gitFetch` runs; one pull per run); the object store (a
blob is its bytes); git's rename detection (`gitFetch` names at most one move); file modes, symlinks,
submodules, ignored files (one `eligible` flag decides the pre-check); `logseq/config.edn`'s own writer;
the autocommit's internal steps (one atomic step); what a checkout writes; the push. Ref durability is
taken from libgit2 with `GIT_OPT_ENABLE_FSYNC_GITDIR`: `filebuf.c` fsyncs the file and then its parent
directory (`git_futils_fsync_parent`, libgit2 1.9.7 as vendored by `libgit2-sys 0.18.8`), so a ref move is
durable. Index writes of other git programs and of Tine's autocommit are taken as durable. The P2
barrier is one atomic step.

## 3. Reading of the ADR text, and the checked properties

| Target (brief) | Checked as | Reading |
| --- | --- | --- |
| (1) storage guarantee | `storageSafety` = s3.6 `guarantee ∧ orderHolds ∧ renameResolves`, unchanged | — |
| (2) ref never at U while a changed path lacks its U blob durably | `gitLanded`: with HEAD = U, every changed path's durable bytes are U's blob **or a later write** | The literal form is false after any legitimate edit made after U landed (`qGitTypedAfterLanding`, `qGitChangedBeforeRef`); such an edit is a local change on top of U, not a lost pull. Ghost sets `atU`/`dFresh`/`sFresh` track "U or later". |
| (3) a crash mid-pull resumes or diverges, never silently reverts | `gitLanded`, `gitAutocommitSafe` (no autocommit on top of U records H's blob of a path that was not written after U landed), plus the resume scenarios | "Resumes to U or ends diverged" is progress; it is checked by directed scenarios, not by an invariant. |
| (4) the index never makes a plain `git commit` record a reversion | `gitIndexSafe` (HEAD = U and no lock ⇒ no changed path's index entry is H's blob) and `gitPlainCommitSafe` | A staged blob other than H's and U's is kept (git_run's staged-kept rule); committing it is the user's intent. |
| (5) local edits during the apply are refused or kept | `gitNoOverwrite` (the pull writes only over H's bytes, and never over a write this process made after U landed) and `gitOtherChecked` | The cross-crash case is the residual below (`gitNoOverwriteStrict`, not claimed). |

Interpretation of step 1: on resume, the non-page file may already hold its U blob (the ADR says "still
holds its H blob (or is absent)", which a resume that already wrote it would fail).

Weak filesystems (R3): a power cut may revert anything fsynced, so `allSafetyWeak` claims (1), (5) and
`gitOtherChecked` only; the durability conjuncts of (2)/(3) and (4) are not claimed there.

## 4. Findings: literal ADR §4 against the properties

Each row is a mutant of the unmutated model (= the literal ADR rule) with its own scenario `m<NAME>`
(fails with QNT508) and a witness run `counterexample<NAME>` whose rendered trace is in the lane evidence
(`artifacts/gs-model/cex/`).

**P1 — index before ref (MGI).** ADR step 4 moves the ref, then the index. A crash between them leaves
`index.lock` (a crash never runs `IndexLock`'s drop) and HEAD = U over an index of H's blobs. The ADR's
own stale-lock notice tells the user to remove the lock; their next plain `git commit` records the reversion
of the pull, and Tine pushes it. Violates (4). Trace: pages and the asset land, lock, barrier, CAS (HEAD =
U), crash, user removes `index.lock`, `git commit` → `plain-commit-reverts`. **Repair:** install the index
(changed paths to U's blobs, staged-kept) first; `IndexLock::install` renames the lock over the index, so
re-take `index.lock` for the CAS. A crash in between leaves HEAD = H with an index at U: a plain commit then
records H plus U's changes (no reversion; the pull ends diverged and phase 2 merges cleanly), and otherwise
the next cycle resumes (`qGitCrashAfterIndexCommit`, `…Diverges`, `…Resume`).

**P1b — the installed index must be durable before the ref moves (MGD).** `IndexLock::install`
(`src-tauri/src/owner/git_run.rs`) fsyncs the lock file and renames it over the index, with no fsync of
`.git` after the rename; the ref move is durable (libgit2). A power cut after the pull can revert the
index to H's entries under HEAD = U: the same reversion as P1. Trace: full pull, power cut (index
reverts), plain commit → `plain-commit-reverts`. **Repair:** fsync the `.git` directory after the rename,
before the CAS. The same missing directory fsync affects every autocommit's index install (a reverted
index under the autocommit's tip makes the next plain commit undo that autocommit); that is outside the
pull and is listed as a follow-up.

**P2 — durability barrier before step 4 (MGB).** The resume rule skips "paths already at their U blob".
After an `Uncertain` directory sync (or a crash after the rename, before the directory sync) the path
holds U's bytes that were never proved durable. The resumed pull moves the ref; a power cut reverts the
path to H's bytes under HEAD = U, and the next autocommit records the reversion of the pull (pushed).
Violates (2)/(3). Trace: page op, flush, rename, `dirSync(Uncertain)` → `pull-incomplete`, crash, resume
(page skipped as at U), ref to U, power cut (page back to H), cadence autocommit → `autocommit-reverts`.
**Repair:** under `index.lock`, before the index and ref, fsync every changed path and its directory (a
deleted page: its directory and the trash), including skipped ones; a failed fsync ends the cycle
`pull-incomplete` (`qGitBarrierFails`).

**P3 — every autocommit applies the resume test (MGC).** The ADR applies the resume test only "before the
pre-pull autocommit". After `pull-held`/`pull-incomplete` the run lock is released, and a cadence,
background/quit or structural-bracket autocommit (§3) commits the half-applied tree on H. The next cycle
sees HEAD ≠ H and ends `diverged`, contradicting the refusal table's "the resume rule finishes it". No text
is lost (a three-way merge of that commit with U is clean), but phase 1 stops with a spurious divergence
after a mere disk error or busy page. Classified as a required neighbour, not a data-loss blocker.
**Repair:** every autocommit skips while HEAD = H, U is a fetched strict descendant, and every path that
differs from H holds U's blob or is this process's applied path (`gitPending`).

**P4 — remember what this process applied (MGA).** The resume test and the skip rule compare bytes only.
If the user edits a pulled page back to exactly H's bytes after U landed (an undo right after the pull is
the likely way), and the cycle then ends held or incomplete, the next cycle sees "H's bytes, not yet
applied" and writes U over the user's edit. Violates (5). **Repair:** an in-memory set of paths this
process's pull put U's blob into (`applied`); the resume test and the skip rule treat them as applied.

**Residual (not repaired; `gitNoOverwriteStrict` is not claimed).** The same edit back to exactly H's bytes
followed by a crash, power cut or graph switch before the ref moves: the in-memory set is gone and nothing
on disk distinguishes "edited back" from "not yet applied", so the pull re-applies U over it
(`qGitResidualEditBack`). The same holds when another program does it: a random run found Syncthing-like
writes of U's bytes and then H's bytes to a page before the pull's op, which then writes U over them (P4
cannot help: Tine did not write U there). The edited text equals H's blob, so it stays in git history.
Closing it needs a durable per-path progress record (e.g. moving each path's index entry as it lands, or a
pull journal in app data): a persisted-format decision.

**Checked and holding (no finding).** Window typing during the pull follows s3.6's version protocol (a
stale submit becomes a conflict, never saved over the pulled text unseen; `qGitWindowDuring`,
`qGitWindowDuringSave`); unsaved input refuses the op (`qGitHeldUnsavedRefused`); a local save during the
pages step refuses the op and the next cycle autocommits then diverges (`qGitLocalDuringPages`); an
external page write after the op is caught by the guard (`qGitExternalPage`); the non-page pre-rename
check (`qGitOtherChanged`; MGO removes it); a branch move or checkout during the pull makes the CAS refuse
(`qGitRefRace`, `qGitCheckoutRace`); a crash holding `index.lock` holds the cycle until the user removes
the lock (`qGitCrashAtLock`); a rename resumed across a crash (`qGitRenameResume`); a refused pre-check
writes nothing (`qGitRefusedPrecheck`).

## 5. Follow-ups and questions (not decided here)

- A changed path whose index entry is a staged blob other than H's and U's is kept staged while the working
  tree gets U; git's own fast-forward refuses in that case. Keep staged-kept, or refuse the pull by name?
- An external editor still holding H-derived text can save it after the pull lands; indistinguishable from a
  deliberate local edit (also true after any completed pull). Accepted as out of scope here.
- The CAS refusing after the P1 install leaves the index at U, which matches the working tree; no restore
  is modelled or proposed.
- Every autocommit's `IndexLock::install` lacks the `.git` directory fsync (P1b outside the pull).
- If git's rename detection misses a rename, the pull applies it as delete + add without the s3.2 order
  gate; the model only covers detected renames.

## 6. Reproduce

From `docs/storage-model/quint/` (Quint 0.33.0, `npm ci`):

```
QNT=storage-s37-gitpull.qnt SCEN=scenarios-s37-gitpull.inc python3 sweep.py      # scenarios x 4 profiles, mutants
python3 cons37.py                                                                 # s3.6 outcomes unchanged
QNT=storage-s37-gitpull.qnt SCEN=scenarios-s37-gitpull.inc STEP=gitFocusStep INV=allSafety \
  python3 run.py base none 5000 40                                                # random runs (weak/all: allSafetyWeak)
```

`gitFocusStep` drops the CLI rename/delete, generic `opApply`, switch and name claims from `step`.
Uniform random runs from `init` do not get deep into a pull: none of 5000 × 40 reached HEAD = U. The
scenario file therefore defines mid-pull starts (`startOpen`, `startPages`, `startRename`,
`startIncomplete`, `startAtRef`) and `gitDriveStep`, a relation narrowed to page 0, its window, the
non-page file and git (`INIT=startPages STEP=gitDriveStep`). Those runs find MGI, MGE and MGO and the
residual within seconds. They do not find MGD, MGB or MGA, and they do not reach HEAD = U in 20 steps. The
deep sequences are covered by the directed scenarios and the witnesses, not by random search.
