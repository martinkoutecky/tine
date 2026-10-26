# s3.5 executable storage model

This private, unwired module transcribes `storage-s35.qnt` s3.5, SHA-256
`7ebed8779c70012c6d02a2a82417d1461db3d059819187576931134c4ea7e95c`.
The scenario source is `scenarios-s35.inc` s3.5, SHA-256
`9200a28404f60177fa3dadda6e3b932f8a866c6bb7a2110854fcd7e181abe6b5`.
s3.4 adds durable external name claimants (`Sys.claimed`, `extClaim`), the
absent-create admission guard and `name-slot-create-over-claim` tag (MNS).
s3.5 restores a cancelling rename's source only when held (MRH).
s3.3 adds `opApply` (host-only-writer P2): one host operation applying caller
bytes to a page set, each page's expected bytes checked against the host's
current bytes (`opCur`), with an optional move whose gates order the writes as
a rename's do. Its mutants MEX/MAC/MAN/MAM drop the stale-base check, the clean
check, the drafts and the move gates.
s3.2 adds a rename's in-run write order (`order.rs`, STEP3-DESIGN "rename
ordering = option 2"): a full rename installs gates over the pages it changes,
a gated page's save does not start, a page's witness is its `dirSync(ok)` at the
operation's version, a Discard of an unwitnessed destination reverts the source
while it still holds the deletion, and `down` clears the gates. The ghost checks
`orderHolds` (no save started with an unretired obligation) and
`renameResolves` (a renamed page keeps a file at one of its two paths) beside
the guarantee; a power cut or launch clears both obligations.
It performs no I/O and has no clock, async tasks or production consumers.
Replay establishes transcription evidence, not backend or native I/O conformance.

The page-host byte-label lockstep substitutes an opaque pure reference rewrite
and preserves the moving source label. Production rename also calls the existing
deterministic title rebinder; those changed destination bytes are outside this
equality certificate by design (SPEC-s3 §2 / STEP2-DESIGN §3). A separate host
semantic test exercises title rebinding together with durable draft custody,
publication and crash recovery. Host conformance calls binding C3's test seam before the inner host can
record absent creates. Physical `.org` twins witness claimants independently.
Disabled `opApply` steps call admission and check the matching Changed,
Unsaved or Busy refusal and unchanged state. Native single-action refusal
probes run directly; other speculative forks and power remain ModelFs-only.
Correspondence is refinement: a creating save stopped at Check by a physical
twin takes the enabled `saveFail` successor, comparing state and ghost as usual
and keeping the dirty text. Scenario publication suffixes are reported as the
alternative branch; random traces continue, calling obsolete save-suffix steps
and checking their Disabled stutter until another save starts. An oracle-enabled
`opApply` refused with Changed may stutter only when a non-deletion effect planned
over ABSENT finds a physical canonical file or claimant. State, readable/durable
files and versions must stay unchanged. Unrelated failures/refusals still fail
conformance; positive controls and fault probes exercise both restrictions.
The production binding's Unsaved set must include every oracle-dirty input
page, and each additional member must have oracle window input pending/sent
or a queued up-channel message. Oracle-enabled queued-input refusals are checked
stutters. After admission stutters, obsolete trace save/receive requests are
actually invoked and must refuse without changing state, files or versions.
Rename candidate refs use the existing R5/A-W1 effective-set refinement:
dirty/busy held refs whose rewrite is unchanged are omitted from the model
effect set. The original host operation must match that enabled model successor;
changed dirty refs and dirty endpoints still refuse.
ModelFs discovers save-time twins from physical files, just like admission.

There is one s3 model; no separate s2 mode or s2-only fixture remains.
On two paths without the new actions, its rules are s2.1. Every read goes through
`table`; `observe` is enabled when that read changes the page, even if the bytes
equal the last read. `opRename`, `opDelete` and `flushDel` include the literal
draft/promise, trash, `opRead`, D4 and no-clobber rules and `trashed` guarantee.
`load` holds one previously unheld path with its read snapshot and a version;
operations require their changed pages and source to be held before capture.

`Config.pages` selects any positive count of contiguous page IDs. Vec implements
total page maps; BTreeSet implements sets; BTreeMap implements operation rewrite
maps; Vec preserves request order. Texts remain opaque equality labels 1/2/3,
with ABSENT (-1), NONE (-2), UNKNOWN (-3). Checked i64 increments panic on
representation exhaustion rather than wrapping or adding a transition guard.
VMAX/EMAX/QMAX remain diagnostics. Only base/R1/weak/all race profiles are
represented, all with crash and power enabled. Fault selection and serialization
remain `cfg(test)` only. Actions return a cloned successor or None when disabled,
and request subactions preserve the model's commit boundaries.

The existing `scripts/s2/` and `tests/fixtures/s2/` paths are retained to avoid
unrelated path/reference changes; their contents now describe s3. All generated
models, logs, ITF and larger corpora stay under ignored `scratch/s3/`.
Generators never write into the supplied model directory or proof lanes.
Quint calls take `$TINE_AGENTS/og/.tlc.lock`; `TINE_AGENTS` defaults to the
repository's sibling `tine-agents` directory.

Regenerate the 194 scenarios, four profile oracles, 38 model-mutant oracles and
three-path random traces from the repository root:

```sh
rtk proxy python3 -B scripts/s2/generate.py /path/to/og/merged --scenario-source /path/to/og/merged/scenarios-s35.inc --quint /path/to/og/model/tools/node_modules/.bin/quint
rtk proxy python3 -B scripts/s2/witnesses.py /path/to/og/merged --quint /path/to/og/model/tools/node_modules/.bin/quint
rtk proxy bash -c 'source scripts/env.sh; export CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target S3_TRACE_FIXTURE=$PWD/scratch/s3/traces.json; rtk cargo test -p tine-store'
```

The generator checks both pinned source hashes, expands intermediate assertions,
conditionals and refusal checks, and compares 8,148 scenario outcomes. It
distinguishes pass, disabled action (QNT507/QNT513), failed assertion (QNT508)
and test returned false (QNT511). All 38 sweep mutants fail their own scenario.
MRH fails the retired-source outcome assertion while the three invariants hold.
MRN5 violates the restored-deletion completion assertion rather than the
checked invariants (guarantee, orderHolds, renameResolves);
MDE additionally executes the power-loss suffix to demonstrate A.

Random traces use 64 runs per profile, 40 steps, seeds 20261009 plus profile index
times 7919. Repeated original driver choices weight protocol progress; helper
actions encode the three-path power set and rewrite map in integer arguments
without changing any model rule. The complete corpus must cover every step
primitive, including `extClaim` and the new operations. Eight traces per profile are selected
deterministically to retain rare actions and committed using lossless state
deltas and interned paths/actions. Normal tests replay that sample; setting
`S3_TRACE_FIXTURE` additionally replays all 256 generated traces. Every Sys/Ghost
field is compared after init and every transition, including inactive padding,
set contents and queue order.

Targeted witnesses are short Quint counterexamples to branch-avoidance invariants
over scratch copies. They compare full states, normal enabled/disabled choices
and Quint's guarantee predicates even when faults make them false. A final
instrumentation-only capture stutter exports the last state's guard/predicate
oracles; it is checked to preserve Sys/Ghost and excluded from replay.
Parameterized witnesses instantiate one, two and five paths, including sparse
launch version numbering. Three longer witnesses cross the diagnostic 1,000
bounds; they use the same lossless delta encoding, never extra transition guards.

Regenerate supporting reachability samples:

```sh
rtk proxy python3 -B scripts/s2/equivalence.py /path/to/og/merged --samples 4096 --steps 80 --output scratch/s3/equivalence-core
rtk proxy python3 -B scripts/s2/equivalence.py /path/to/og/merged --only saveBaseSeenOrRead --samples 4096 --steps 80 --output scratch/s3/equivalence-g
```

Samples support explicit constructor/transition arguments; they are not exhaustive
proofs. s2's writtenSeen, typedSeen and absentClean facts are not asserted for s3.

Run the Rust port sweeps (an isolated scratch crate uses byte-identical source,
tests and fixtures, avoiding unrelated integration-test linking):

```sh
rtk proxy uv --cache-dir scratch/s3/uv-cache venv scratch/s3/python
rtk proxy uv --cache-dir scratch/s3/uv-cache pip install --python scratch/s3/python/bin/python tree-sitter==0.26.0 tree-sitter-rust==0.24.2
rtk proxy bash -c 'source scripts/env.sh; export CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target; rtk proxy scratch/s3/python/bin/python -B scripts/s2/hand_mutations.py --run --output scratch/s3/hand-final --allow-equivalents scripts/s2/equivalents.json'
rtk proxy bash -c 'source scripts/env.sh; export CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target; rtk proxy python3 -B scripts/s2/cargo_mutations.py --output scratch/s3/cargo-final'
rtk proxy python3 -B scripts/s2/mutation_report.py
```

The hand sweep drops each operand of every &&/|| chain, preserving nested groups
and including all new guards/rules. Each hand output directory owns its Cargo
target directory, so independent scratch crates cannot reuse another sweep's
test executable. Healthy Quint traces also replay through the host-facing oracle
wrapper, including its independent R1 and weak profile selection.
Cargo-mutants is optional locally; the task
receipt records its completed sweep. `--skip-witnesses` supports the baseline
census before witness regeneration; final verification includes the witnesses.
The exact-source equivalence ledger records each surviving mutation and its
argument, limited to the model's finite action domains. A changed source hash,
unexplained survivor, missing census or hand compile failure fails the checker.
The real-package crate gate remains mandatory.

**A model change lands here and in the fixtures in the same commit.**
Update both pinned source hashes deliberately. Rust representation choices,
literal model observations and mutation findings are recorded in the task's
TRANSCRIPTION.md and RECEIPT.md. Unit cost: none (no persisted record).

The lane receipt is `RECEIPT-s35-correspondence.md` at the worktree root.
The task4 manager ruling permits queued-input Unsaved under the runtime predicate
above. The receipt distinguishes completed checks from in-progress gates and
records each refinement/refusal class and its fail-before evidence.
The `planCreate` read-only scenario helper is a stutter when disk is absent and
its name is unclaimed, disabled otherwise; it is outside the random step driver.
All builds use `scripts/env.sh`; set `TINE_AGENTS` explicitly in linked worktrees.
