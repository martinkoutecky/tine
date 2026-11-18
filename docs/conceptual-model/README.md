# Conceptual model and conformance

The confirmed model is K (D-20261116-CM536). Rename and merge compare their
prepared output with its input-derived expected observations before publishing
anything. Safe publication remains the [storage model's](../storage-model/README.md)
responsibility.

A rename onto one physical holder asks once to merge; a destination held only
as another page's alias is refused. The source's own alias permits a direct
rename. A source without a file never creates one. Days and task/priority sites
stay fixed. The same substitution covers namespace children, references,
aliases and stored query/macro names. Settings follow in a separate step.

The text promises are exact bytes outside rewritten name spans (P1), the
intended parser observations (P2), unchanged-save stability (P3), and merge
content conservation (P4). Merge header order and parser-unobserved layout are
writer conventions. Unspellable sites and conflicted referrers stay and are
reported at their original locations. A physical file merge uses the same
writer and checks, and rewrites no outside reference or setting.

The parser, name/site spelling readers and real save are trusted boundaries.
The finite model fixture uses a toy reader and an identity save callback; it
does not prove production parser fidelity or that every admitted candidate can
be written. A production check refusal writes nothing.

## Executable checks

- `tine-core/src/cm_check`: independent expected observations and comparison.
- `tine-store/src/transaction/model_check.rs`: P2/P3 on final staged bytes,
  before any guarded write.
- `tine-graph-features/tests/cm_witness.rs`: K's requests through the real
  offer, host and writers; expected paths and observations come from the model
  fixture. Refusals preserve file bytes. Deliberately wrong output rows are
  injected into the checker rather than treated as refused user requests.
- The shared fixture is `crates/tine-core/src/cm_check/witness-fixtures-r6.json`,
  a byte copy of K's generated header-key matrix: 35 name requests, one physical
  merge control, 17 negatives, and four negative-answer annotations.
- `cm_requests`, `cm_touched`, `cm_report`, `cm_settings`, `cm_merge_o6`,
  `cm_model_check`, and the witness gap tests cover decisions, whole-graph
  safety, byte preservation, located reports, Settings, and physical merge.
- `tine-store/src/cm_conformance_tests.rs`: 435 static graphs against K's
  exporter, selected only by `CM_STATIC_BRIDGE`. This path also produces the
  frontend cases used by navigation, counts, namespace and published tests.
  Retired operation bridges and their frozen difference sections are removed.

The witness runner counts every row. `source_preamble`, `bare_bullet_header`
and `header_children_leftover` are explicitly listed toy-reader differences,
with separate transaction controls using Tine-readable observations.
`key_spelling` uses an identity save: its original text is refused with P3,
and an ordinary first block makes its two key sites stable and admissible.
`org_leftover_reference` loses a reference only under the toy reader; Tine
keeps it in the quote, while a native drawer control refuses real loss.
`folded_occupant` probes its fixture's filesystem: a folded-path occupant
refuses on a folding filesystem and permits the distinct path otherwise.
No unlisted skip or zero-row run passes. The lane receipt records totals and
the clause-to-test map, including the separate title-with-new-inner-link P2
refusal in Markdown and Org rename and merge controls.

```bash
cargo test -p tine-graph-features --test integration cm_witness
cargo test -p tine-core cm_check
CM_REQUIRE_BRIDGE=1 CM_STATIC_BRIDGE=/path/to/cm-bridge-k \
  cargo test -p tine-store --lib conceptual_model_conformance
```

The focused gate must run all three, reject zero matched tests and unaccounted
rows, and run frontend consumers on the generated cases. No operation bridge
environment variable is needed.

## Model sources

The checked-in `SPEC.md`, `GUARANTEES.md` and `lean/` are the older model
snapshot awaiting the separate K documentation sync. Their candidate menus,
operation adapters and proof counts describe that snapshot, not the current
implementation contract. This conformance change does not modify or build Lean.

The models, proofs and application are written by AI agents. The maintainer
decides product rules without reading the code or proofs. Keep backups.
