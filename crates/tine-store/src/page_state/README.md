# s2 executable storage model

This private, unwired module transcribes the frozen Quint model
`storage-s2.qnt`, SHA-256
`1c3194293856a11630536b74a8221c76c2a8725e77bc366b9172641681a08cef`.
It performs no I/O and has no clock, async tasks, or production consumers.
It does not establish conformance of the existing storage implementation.

Every primitive action returns a cloned successor or `None` when disabled.
The explicit `Action` argument replaces Quint's nondeterministic choice in
`step`. Backend request subactions preserve their original commit boundary.
The state includes all system and ghost fields; guarantee predicates preserve
the model's history through the ghost sets and promises.

The finite model has two pages and equality-only text labels 1, 2, 3, plus
ABSENT (-1), NONE (-2), UNKNOWN (-3). Arrays represent total page maps;
BTreeSet represents mathematical sets; Vec preserves request order.
Versions and external-write counts use checked i64 increments, with a panic
on representation exhaustion rather than wraparound. There is no artificial
VMAX/EMAX/QMAX transition bound. Only the four specified race profiles are
represented; all allow crash and power cut. Mutant selection and serialization
exist only under `cfg(test)`. The module is private pending the later backend
integration decision.

From the repository root, regenerate with the model directory as an argument:

```sh
rtk proxy python3 -B scripts/s2/generate.py /path/to/og/merged --quint /path/to/og/model/tools/node_modules/.bin/quint
rtk proxy bash -c 'source scripts/env.sh; export CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target S2_TRACE_FIXTURE=$PWD/scratch/s2/traces.json; rtk cargo test -p tine-store page_state -- --nocapture'
```

The generator verifies the model hash, parses and expands the original 88
scenarios (preserving intermediate assertions and conditionals), and invokes
Quint for four profiles and all 19 sweep mutants. The test compares all 2,024
outcomes, distinguishing pass, disabled action, and failed assertion. Each
mutant's own scenario must fail, and its resulting guarantee must be false
(MDE additionally executes its power-loss suffix).

For random traces the generator instruments only the next-state driver to
record the selected primitive and its arguments. Repeated branches weight the
original choices toward protocol progress; all original choices remain, and
the complete generated corpus must cover every next-state action. The rules remain unchanged.
It invokes Quint with per-profile seeds 20261008 + profile index × 7919,
64 traces per profile, 40 steps, and the
full guarantee. All 256 traces stay under ignored `scratch/s2/`; eight per
profile are committed, chosen deterministically to retain rare actions.
Replay compares every Sys/Ghost field after init and
every transition, including inactive records, set contents and queue order.
The ordinary crate test always replays the committed sample; setting
`S2_TRACE_FIXTURE` additionally replays the larger generated corpus.
Generators write nothing to the supplied model directory or proof lanes.

Step 1b adds `witnesses.json`: targeted counterexamples to branch-avoidance
invariants over scratch copies of the same frozen model. Every scheduled
transition is an original model action. Some traces use its declared test-only
faults, including MSM, to exercise false guarantee clauses. Normal traces also
carry Quint-evaluated guards for all finite action choices, so replay checks
disabled actions as well as successful transitions. Fault traces compare full
states and the model's guarantee predicates without assuming they remain true.
The three diagnostic bounds require crossing 1,000; their longer traces use
lossless state deltas with interned paths/actions. Replay reconstructs the
entire expected Sys/Ghost before comparing every state. These bounds remain
diagnostics, never transition guards.

Regenerate the witnesses and supporting reachability checks:

```sh
rtk proxy python3 -B scripts/s2/witnesses.py /path/to/og/merged
rtk proxy python3 -B scripts/s2/equivalence.py /path/to/og/merged
```

`--reuse` on the witness generator accepts only identical scratch model text
with an existing invariant-violation log and ITF; it claims no fresh run.
`equivalence.py --only FACT,...` checks selected structural facts; `--mutant MIS`
can check the refused/accepted move fact under the frozen MIS fault. Samples
support the short inductive arguments in `scripts/s2/equivalents.json`; they
are not exhaustive proofs. The ledger explicitly limits domain equivalence to
the frozen model's action domains.

The hand-mutation sweep is CI-runnable. It uses the Rust syntax tree to drop
each operand of each &&/|| chain, including nested groups. It asserts that the
originally missed draft-equals-disk discard disjunct belongs to the set. Install
the parser in a local environment, then run from the repository root:

```sh
rtk proxy uv --cache-dir scratch/s2/uv-cache venv scratch/s2/python
rtk proxy uv --cache-dir scratch/s2/uv-cache pip install --python scratch/s2/python/bin/python tree-sitter==0.26.0 tree-sitter-rust==0.24.2
rtk proxy bash -c 'source scripts/env.sh; export CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target; rtk proxy scratch/s2/python/bin/python -B scripts/s2/hand_mutations.py --run --allow-equivalents scripts/s2/equivalents.json'
```

`--only H002,H146` is a quick check of one reviewed equivalent and the planted
mutant. Any unexplained survivor or compile failure exits nonzero. The ledger
is tied to the complete source hash and exact dropped expression, so changing
the implementation requires reviewing equivalences again. Each outcome and
build/test log is written under the requested scratch output directory.

Cargo-mutants is optional locally, not required in CI:

```sh
rtk proxy bash -c 'source scripts/env.sh; export CARGO_INCREMENTAL=0 LANG=C.UTF-8 CARGO_TARGET_DIR=$PWD/target; rtk proxy python3 -B scripts/s2/cargo_mutations.py'
```

Both sweeps prepare isolated scratch crates with byte-identical copies of the
production module, its tests and fixtures, and run only the page_state library
tests. This avoids repeatedly linking unrelated storage tests. All targets
remain inside this worktree. The real-package `cargo test -p tine-store` gate
still runs the complete crate. Sweeps never mutate the production file or the
frozen model directory; their scratch files are restored on normal completion.
`mutation_report.py` merges the saved fail-before and correction censuses and
rejects any survivor lacking an explicit argument before generating the ledger.

**A model change lands here and in the fixtures in the same commit.**
Update the pinned hash deliberately when transcribing that change. Replay is
evidence of transcription fidelity, not a proof of semantic equivalence or of
native I/O conformance. Model obligations about real reads, request identity,
draft durability, and orderly shutdown remain for later integration steps.
