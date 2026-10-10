# s3.2 executable storage model

This private, unwired module transcribes `storage-s3.qnt` s3.2, SHA-256
`1e6ec36c29ec8fd1b7a1ebdce099e82509c0ff59fa6c137cf6f0ef3d1e0d2bd4`.
The scenario source is `scenarios-s3.inc` s3.2, SHA-256
`92a75904b2da9b43a7bb9c80a0f2fc41b462cef5a60ec41f4d613b67d58ff1c3`.
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
publication and crash recovery. Scenario `.fail()` outcomes alone do not prove
backend refusals: direct host API tests independently cover those guards.

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

Regenerate the 159 scenarios, four profile oracles, 32 model-mutant oracles and
three-path random traces from the repository root:

```sh
rtk proxy python3 -B scripts/s2/generate.py /path/to/og/merged --scenario-source /path/to/og/merged/scenarios-s3.inc --quint /path/to/og/model/tools/node_modules/.bin/quint
rtk proxy python3 -B scripts/s2/witnesses.py /path/to/og/merged --quint /path/to/og/model/tools/node_modules/.bin/quint
rtk proxy bash -c 'source scripts/env.sh; export CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target S3_TRACE_FIXTURE=$PWD/scratch/s3/traces.json; rtk cargo test -p tine-store'
```

The generator checks both pinned source hashes, expands intermediate assertions,
conditionals and refusal checks, and compares 5,724 scenario outcomes. It
distinguishes pass, disabled action (QNT507/QNT513), failed assertion (QNT508)
and test returned false (QNT511). All 32 sweep mutants fail their own scenario.
MRN5 violates the restored-deletion completion assertion rather than the
checked invariants (guarantee, orderHolds, renameResolves);
MDE additionally executes the power-loss suffix to demonstrate A.

Random traces use 64 runs per profile, 40 steps, seeds 20261009 plus profile index
times 7919. Repeated original driver choices weight protocol progress; helper
actions encode the three-path power set and rewrite map in integer arguments
without changing any model rule. The complete corpus must cover every step
primitive, including the new operations. Eight traces per profile are selected
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
rtk proxy bash -c 'source scripts/env.sh; export CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target; rtk proxy scratch/s3/python/bin/python -B scripts/s2/hand_mutations.py --run --output scratch/s3/hand-final --allow-equivalents scripts/s2/equivalents.json'
rtk proxy bash -c 'source scripts/env.sh; export CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target; rtk proxy python3 -B scripts/s2/cargo_mutations.py --output scratch/s3/cargo-final'
rtk proxy python3 -B scripts/s2/mutation_report.py
```

The hand sweep drops each operand of every &&/|| chain, preserving nested groups
and including all new guards/rules. Cargo-mutants is optional locally; the task
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
