# Composition at admission

The conceptual model supplies an exact patch on listed files. Storage keeps its finite slots, integer labels and its existing single-pair admission. This package proves the admission prefix of the composition of the two.

The codec enumerates the finite listed image and patch support. One slot names one literal path, and one nonnegative integer names one byte string. A host slot key checks that the literal paths on this support identify distinct physical slots. The codec keeps file absence distinct from a missing draft, an unknown base, and an out-of-vocabulary label.

The adapter builds the storage effect and plan maps from the same patch. It accepts a nonempty footprint and at most one cross-path binding. A designated pair must retire a present source and have a destination effect with present after-bytes. Same-path bindings require no storage pair. The actual storage guard remains responsible for admission. Codec construction scans only the finite image and patch data supplied to the proof. It performs no storage I/O and adds no operation to Tine.

The bridge relates storage to K at effect slots. Storage admission establishes that the encoded before-values match held buffers and their clean bases. Agreement with the current disk is a separate premise. On admission each decoded draft agrees with the candidate at that path and carries the version assigned to its new unsaved promise. Outside the footprint, pages and drafts are unchanged. Admission leaves both visible and stable disk unchanged at every slot.

Build with `lake build` in `docs/model-composition/lean` using the pinned Lean toolchain. `CompositionCodec` proves the path and byte round trips. `CompositionAdmission` proves `admit_decision`. `CompositionWitnesses` checks the designated rename with a referrer rewrite and the reachable case where a clean source buffer lags an external deletion.

## Does an admitted request finish? (Quint, bounded)

`quint/finish.inc` extends the storage scenarios with a request identity, a driver and a terminal observer for one admitted rename with a rewritten referrer; `quint/check.cjs` generates the models, searches every reachable state to a stated depth for pending loops, and checks two mutants (a starving driver, a premature success). This is bounded evidence, not a proof. It found two runs where a request stays pending although the app runs, I/O works and other writers have stopped: a stale window edit that replaces the admitted referrer draft, and a crash between a discard and the removal of its drafts. Both are open questions for the next stages of the composition.
