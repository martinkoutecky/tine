# Tine's conceptual model: specification and Lean proofs

This directory holds the formal model of what a Logseq graph *means* to Tine: which file a page name opens, which
blocks reference a page, what an alias group is, what a query returns, and what renaming or merging a page does to
all of that. It is a pure model of meaning. How Tine writes files safely is the separate
[storage model](../storage-model/README.md).

- [`SPEC.md`](SPEC.md): the rules, in prose. Section 1 is written for Logseq users; the later sections are precise
  enough to implement against. The spec states the rules as they currently are; what changed, and when, is in git
  history.
- [`GUARANTEES.md`](GUARANTEES.md): what is proved, the hypotheses each headline theorem needs, where those may be
  too strong, and the exhaustive comparison with the previous model round.
- [`lean/`](lean/): a Lake project with the [Lean 4](https://lean-lang.org/) model and its proofs (77 modules,
  1,480 named theorems). The current reader and operations are `Date13.lean` with the `*14` modules
  (`DateOperations14`, `DateProofs14`, `Journal14`, `DateEffects14`, `DateSafety14`, `DateTwins14`), and
  `Alias14.lean` for alias fields and complete-text accounting. The `*13` and earlier modules are libraries the
  current ones build on; theorems there that carry extra hypotheses are special cases, not guarantees of the current
  model (GUARANTEES.md lists which).

## What the model guarantees

Rename and merge are a choice for the user. A request offers every candidate that passes one finite safety check:
rename literally, merge into any real page of the destination's alias group, or make an alias name its own page.
The dialog shows the whole group and a preview; a caller without a dialog (the CLI, plugins, agents) succeeds only
when exactly one candidate is safe, and is refused otherwise. For every offered candidate, Lean proves:

- no user text is lost, and text outside the edited spans is unchanged (given the writer obligations in SPEC.md §5.1).
  The one exception: merging two pages that both declare the same alias may drop the redundant declaration (SPEC.md §5.2);
- file paths stay unique and the operation creates no new claim collision;
- journal dates and task/priority markers never change;
- no page is captured as somebody else's namespace child;
- when two files claim one name, the operation never changes which file is active;
- every reference whose target changes appears in the preview;
- the after-state's aliases, references, Linked References, counts, query results, links and navigation are
  computed exactly (they may change; the guarantee is that every change is characterized, not that nothing
  changes);
- cancel changes nothing.

These hold for every candidate the model offers, under the named boundaries below and the hypotheses GUARANTEES.md
lists for each headline. One rule is still open: how a date page may be renamed inside Tine. The model keeps that
policy as an explicit parameter, and its rename theorems hold for every choice of it.
An exhaustive comparison with the previous model round classifies every request as unchanged or as one of a named
list of differences.

## Boundaries

- **The parser and writer.** The model takes the parser's answers (names, keys, dates, occurrences, outlines) as
  input, and its text guarantees assume a writer that does what SPEC.md §5.1 says. Whether Tine's code does is
  checked by the conformance tests (`crates/tine-store/src/cm_conformance*`), not proved.
- **Storage.** Publishing the result to disk (crash safety, locking, sync) is the storage model's job.
- **Status of the code.** Tine is being changed to follow this version of the model. Until that work is finished,
  some behaviour (notably rename/merge options, which file opens for a name two files claim, and date-named pages)
  still follows the previous model. `CHANGELOG.md` records what has shipped.

## How this was made

The model, the proofs and the code are written by AI agents. Each model round was checked by an independent
verifier agent (axiom audit, rebuilt counterexamples, a review of every hypothesis). Tine's maintainer decides the
product rules but does not read the code or the proofs. Keep backups of your graph.

## Checking it yourself

```bash
cd docs/conceptual-model/lean
LEAN_NUM_THREADS=2 lake build     # toolchain pinned in lean-toolchain
python3 audit.py                  # no sorry, every theorem uses only the standard axioms
```

`audit.py` imports every module, prints the axioms of every named theorem, and fails on anything beyond
`propext`, `Quot.sound` and `Classical.choice`. Comments in the model files sometimes cite internal working records
or decision identifiers; those records are not part of this repository.
