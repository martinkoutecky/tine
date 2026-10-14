/-
Headline theorems of the Tine storage-model proofs, with their exact statements and an axiom audit.

Building this module (`lake build`) prints each statement and its axioms, then walks the full dependency
cone of every headline (checked types AND proof terms, not the cached axiom list) and fails the build if
anything other than the three standard axioms `propext`, `Classical.choice`, `Quot.sound` occurs, or if any
dependency is `unsafe`. See ../README.md for what each theorem means and which model version it is about.
-/
import Lean
import Guarantee
import FinalA
import NonvacuityA
import NonvacuityG
import GClosure6
import Conservativity3
import Trash3
import Guarantee31
import Conservativity31
import LoadWitness7
import Scope7
import Partial8
import Apply8
import Conservativity8
import Verifier7
import Verifier8
open Lean Elab Command

-- s2 (model 1c319429): the whole guarantee and A (no loss), every page count, texts 1..T, every race profile.
#check @Storage.guarantee_positiveTexts
#check @Storage.noLoss_positiveTexts
-- s2 non-vacuity: mutants reach real violations (C, G, A).
#check @Storage.MC_nonvacuous
#check @Storage.G_mutant_nonvacuous
#check @VerifierA.reachable_A_counterexamples

-- s3 (model baaaeab4): the whole guarantee; old-only runs are exactly s2.1 runs; deleted bytes reach the trash.
#check @Storage3.guarantee_positiveTexts6
#check @Storage3.initialized_old_runs_exact
#check @Storage3.infinite3_projects
#check @Storage3.infinite21_lifts
#check @Storage3.trashed_reachable

-- s3.1 (model 614f82a8): the whole guarantee; old-only runs are exactly s2.1 runs; load is reachable and useful.
#check @Storage31.guarantee_positiveTexts7
#check @Storage31.uniform_guarantee7
#check @Storage31.no_new_actions_are_s21_runs7
#check @Storage31.infinite_old_runs_exact7
#check @Storage31.infinite_core_lifts7
#check @Storage31.load_delete_enabled7

-- s3.2 (model 1e6ec36c), PARTIAL: the s3.1 guarantee on runs without a full rename.
#check @Storage32.noFull_refines31_8
#check @Storage32.noFull_guarantee8
-- s3.3 (model 353162e9), PARTIAL: exact conservativity, order/resolution on runs without a move,
-- and one-step opApply lemmas (apply_guarantee8 ASSUMES the guarantee before the step).
#check @Storage33.no_opApply_runs_exact_s32_8
#check @Storage33.prior_stream_exact8
#check @Storage33.no_new_actions_are_s21_runs8
#check @Storage33.infinite_old_runs_exact8
#check @Storage33.infinite_core_lifts8
#check @Storage33.noMove_order_resolves8
#check @Storage33.apply_admission8
#check @Storage33.apply_effects_drafted8
#check @Storage33.apply_bad8
#check @Storage33.apply_guarantee8
#check @Storage33.apply_promise8
#check @Storage33.apply_new_custody8
#check @Verifier8.delete_nonvacuous
#check @Verifier8.refs_nonvacuous
#check @Verifier8.apply_nonvacuous

#print axioms Storage.guarantee_positiveTexts
#print axioms Storage3.guarantee_positiveTexts6
#print axioms Storage31.guarantee_positiveTexts7
#print axioms Storage32.noFull_guarantee8

-- Walk checked types and values of the headline constants; fail on a nonstandard axiom or unsafe constant.
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [``Storage.guarantee_positiveTexts, ``Storage.noLoss_positiveTexts,
    ``Storage.MC_nonvacuous, ``Storage.G_mutant_nonvacuous, ``VerifierA.reachable_A_counterexamples,
    ``Storage3.guarantee_positiveTexts6, ``Storage3.initialized_old_runs_exact, ``Storage3.infinite3_projects,
    ``Storage3.infinite21_lifts, ``Storage3.trashed_reachable,
    ``Storage31.guarantee_positiveTexts7, ``Storage31.uniform_guarantee7,
    ``Storage31.no_new_actions_are_s21_runs7, ``Storage31.infinite_old_runs_exact7,
    ``Storage31.infinite_core_lifts7, ``Storage31.load_delete_enabled7,
    ``Storage32.noFull_refines31_8, ``Storage32.noFull_guarantee8,
    ``Storage33.no_opApply_runs_exact_s32_8, ``Storage33.prior_stream_exact8,
    ``Storage33.no_new_actions_are_s21_runs8, ``Storage33.infinite_old_runs_exact8,
    ``Storage33.infinite_core_lifts8, ``Storage33.noMove_order_resolves8,
    ``Storage33.apply_admission8, ``Storage33.apply_effects_drafted8, ``Storage33.apply_bad8,
    ``Storage33.apply_guarantee8, ``Storage33.apply_promise8, ``Storage33.apply_new_custody8,
    ``Verifier8.delete_nonvacuous, ``Verifier8.refs_nonvacuous, ``Verifier8.apply_nonvacuous]
  let mut seen : NameSet := {}
  let mut axs : NameSet := {}
  while !pending.isEmpty do
    let n := pending.head!
    pending := pending.tail!
    if seen.contains n then continue
    seen := seen.insert n
    let some info := env.checked.get.find? n | throwError "missing checked constant {n}"
    if info.isUnsafe then throwError "unsafe dependency {n}"
    if let .axiomInfo _ := info then
      axs := axs.insert n
      unless n == ``propext || n == ``Classical.choice || n == ``Quot.sound do
        throwError "nonstandard axiom {n}"
    pending := info.type.getUsedConstants.toList ++ pending
    if let some v := info.value? (allowOpaque := true) then
      pending := v.getUsedConstants.toList ++ pending
    if let .inductInfo i := info then pending := i.ctors ++ pending
  logInfo m!"HEADLINE CONE: {seen.size} constants; axioms {axs.toList}"
