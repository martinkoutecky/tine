# Conceptual model and conformance

This directory holds Tine's conceptual model, model K (D-20261116-CM536, with
the header-key fix A-20261116-HK): what pages, journals, names, aliases and
references mean in a Logseq graph, and what renaming or merging a page by name
does to the graph's page files. Tine's rename and merge are written against it.

- [`SPEC.md`](SPEC.md): the rules. Part I is the core: reading a graph (§2),
  the name request O0–O7 (§3) and the text properties P1–P4 checked on the
  actual result (§4). Part II is the views (Linked References, queries, links,
  the namespace tree). Part III is two chapters: Settings (O8) and Day files.
- [`GUARANTEES.md`](GUARANTEES.md): which Lean theorem proves which rule, and
  the interpretation obligations the proofs leave to the implementation.
- [`CITATIONS.md`](CITATIONS.md): where each rule came from.
- [`lean/`](lean/): the Lean 4 sources (`lean/src`, 151 files) and Lake files.

The core imports neither the views nor the chapters; the chapters import the
core. The model reads one snapshot of the graph and needs no rename history.

## What the theorems promise

The graph is a list of files (path, directory, format, text). A request is
"rename A to B", possibly answered with confirm or cancel. The decision
function `decide` returns one of three outcomes: done, the single question
"merge into the page that holds B?", or a named refusal
(`decision_total`, `decision_deterministic`). A caller that cannot ask gets
no merge, even if it supplies the answer in advance
(`decision_unattended_no_question`, `done_unattended_no_merge`).

Whenever `decide` returns done, its candidate graph satisfies:

- **O4 safety** (`done_o4`, `done_name_safety`): no touched name lands on a
  name another page claims, no touched name is contested, no new pair of files
  claims one name, every new path is free (also on a case-folding file
  system), and a title never names two pages.
- **P1, only names change** (`done_p1`): every file outside a merge pair is
  its input bytes with only the rewritten name spans replaced, each on one
  line; a file with no rewritten site is byte-identical.
- **P2, reads as intended** (`done_reading`, `done_pair_observations`): every
  output file reads exactly as the expected observations computed from the
  input alone. In a merged survivor the header is a permutation of the
  expected entries.
- **P3, stable** (`done_stable`): saving each changed file with no edit
  returns the same file, path and bytes.
- **P4, nothing lost in a merge** (`expected_merge_conserves`, `done_p4`):
  every block and content-bearing header entry of both pages occurs exactly
  once in the survivor, blocks in O6's order and at their depths.
- **Reports** (`done_exception_locations`): every site left unchanged under
  O7 is reported at its own input location.
- **Twins and days** (`done_stationary_twins_opening`, `done_all_day_files`,
  `fileless_no_creation`): a name that several files claimed before still
  opens the same file, every day file keeps its day and path, and a source
  without a file never creates one.
- **Settings** (`done_settings_atomic`): the settings step returns either the
  complete intended settings record or the complete old one with the status
  "pages done, settings not updated"; it cannot change the page outcome
  (`decide_pages_projection`).
- **Day files** (`ensure_reuses`, `ensure_created_exact`, `ensure_idempotent`):
  opening a day reuses any file that claims it; otherwise it adds one empty
  file at an unoccupied path in the journals directory and keeps every other
  file byte for byte.

This is a conditional acceptance guarantee: `decide` runs the checks itself, so
any candidate it returns as done has these properties. It does not say that a
writer always produces a candidate that passes.

## What the proofs assume and do not discharge

Every theorem is stated for arbitrary parameters: the reader, codecs and save
(`Observations` in `lean/src/CutTypes.lean`), the writer (`Engine` in
`lean/src/CutPublish.lean`) and the name key and date parser. The promises
carry over to Tine only if Tine's real components are what these parameters
stand for. GUARANTEES.md lists them as undischarged interpretation
obligations (D-20261106-UA):

1. **Parser (P).** The reader reports every header, alias declaration, block,
   depth and typed name site with its true byte span, and reparsing a fragment
   with the same reader keeps its occurrences. The name key and date parser
   are Logseq's configured ones; no namespace law is assumed of the key.
2. **Codecs and paths (N).** Whether a site can spell a name, key spelling,
   wrappers, file-name encoding, directory and extension, and the file
   system's case folding are the real ones.
3. **Save (S).** The `save` callback is Tine's complete unchanged-save path.
4. **Writer (W).** No theorem says a writer exists that produces an accepted
   candidate for every request that should succeed.
5. **Host (H).** The host publishes exactly the checked bytes through the
   guarded, crash-safe save path, delivers the located reports, and writes
   the settings file all or nothing. The model performs no I/O.
6. **Views (G).** Query evaluation is given a truthful query form and truthful
   predicate results on the actual graph.

The finite fixtures (`CutFixture*`, `CutExamples`) show the assumptions can all
hold at once, using a small generated reader and an identity save. They are
not runs of Tine's parser or save and do not prove that those are faithful.
Faithfulness is checked by the conformance tests below, not proved.

## What is not modelled

- **Storage.** I/O, locks, revisions, concurrency, crashes and atomic
  publication of several files are the host's, specified by the
  [storage model](../storage-model/README.md). No Lean file in either model
  imports the other.
- **Changes between decision and publication.** The model decides on one
  snapshot. If a file it read changes before the host publishes, nothing in
  the model speaks about the result.
- **Physical file merge** (merging two files directly rather than by name).
  Tine runs the same merge writer and the same checks there, but no theorem
  covers that entry point.
- **Persisted block ids** (`id::`, `((block references))`). The model's block
  identity is the file path and outline position; it says nothing about two
  blocks carrying one persisted id after a merge.
- **Ordinary edits** of a title or `alias::` line, and **outside moves** of a
  file: the reader accepts the new snapshot, with no promise that references
  follow the moved content.
- **Interface helpers** (the alias dialog, the day move, the contested-name
  helper) are sequences of requests and edits with no atomicity across steps.
- **Bytes the parser does not report** inside a merged file: blank lines,
  indentation, bullets, line endings, header syntax and header-entry order.
- **The settings file's bytes** are outside P1–P4.

## Building the proofs

Lean 4 `v4.31.0` (pinned in `lean/lean-toolchain`) and the local
[listed-disk vocabulary](../model-base/lean/README.md).

```bash
cd docs/conceptual-model/lean
lake build
python3 audit.py
```

`lake build` compiles the conceptual model and the shared listed-disk module. During the build, `CutHeadlines.lean`
walks the whole dependency cone of the listed theorems and fails on any unsafe
constant or any axiom other than `propext`, `Classical.choice` and
`Quot.sound` (so a `sorry` anywhere in the cone fails the build). It needs
about 3 GB of memory at peak (the Lake file runs Lean with `-j2`).
`audit.py` then scans the sources for `sorry`, `admit`, `native_decide`,
`axiom` and similar escapes, checks that every named theorem is a root of that
cone, that the core imports no view or chapter, and prints the axioms of every
named theorem in both packages, accepting only the same three.

`CutPatch` relates a successful page operation to a finite patch of changed listed entries and their
origins. Replaying the patch yields the exact candidate image and preserves every path outside the
effects. `ListedLayout` is the premise that connects an observed listing to K's paths and UTF-8 bytes.
The listed image covers the modeled page files. Durable publication, physical alias resolution and
concurrent edits belong to the storage composition.

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

The witness runner counts every row. `source_preamble`, `bare_bullet_header`
and `header_children_leftover` are explicitly listed toy-reader differences,
with separate transaction controls using Tine-readable observations.
`key_spelling` uses an identity save: its original text is refused with P3,
and an ordinary first block makes its two key sites stable and admissible.
`org_leftover_reference` loses a reference only under the toy reader; Tine
keeps it in the quote, while a native drawer control refuses real loss.
`folded_occupant` probes its fixture's filesystem: a folded-path occupant
refuses on a folding filesystem and permits the distinct path otherwise.
No unlisted skip or zero-row run passes.

```bash
cargo test -p tine-graph-features --test integration cm_witness
cargo test -p tine-core cm_check
CM_REQUIRE_BRIDGE=1 CM_STATIC_BRIDGE=/path/to/cm-bridge-k \
  cargo test -p tine-store --lib conceptual_model_conformance
```

The focused gate must run all three, reject zero matched tests and unaccounted
rows, and run frontend consumers on the generated cases.

The models, proofs and application are written by AI agents. The maintainer
decides product rules without reading the code or proofs. Keep backups.
