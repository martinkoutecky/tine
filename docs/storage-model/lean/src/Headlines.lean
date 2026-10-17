/-
Headline theorems of the Tine storage-model proofs (s2 to s3.5), with their exact statements and an axiom audit.

Building this module (`lake build`) prints each statement and its axioms, then walks the full dependency
cone of every headline (checked types AND proof terms, not the cached axiom list) and fails the build if
anything other than the three standard axioms `propext`, `Classical.choice`, `Quot.sound` occurs, or if any
dependency is `unsafe`. See ../README.md for what each theorem means and which model version it is about.

s3.5, rounds 10 and 11: the three target predicates hold in every initialized reachable state,
`Storage35.all_targets_reachable35_11` (guarantee, orderHolds and renameResolves together).
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
import Order9
import Reduction9
import Trash9
import Bounds34_9
import Streams9
import Nonvacuity9
import OrderInstall9
import Discard9
import PartialGuarantee9
import Witness35
import Witness11
import Resolution11
import Verifier9
import Verifier10
import Verifier11
open Lean Elab Command

-- s2 (model 1c319429) and its non-vacuity mutants: the whole guarantee, every page count, texts 1..T, every race profile.
#check @Storage.guarantee_positiveTexts
#check @Storage.noLoss_positiveTexts
#check @Storage.MC_nonvacuous
#check @Storage.G_mutant_nonvacuous
#check @VerifierA.reachable_A_counterexamples

-- s3 (model baaaeab4): the whole guarantee; old-only runs are exactly s2.1 runs; deleted bytes reach the trash.
#check @Storage3.guarantee_positiveTexts6
#check @Storage3.initialized_old_runs_exact
#check @Storage3.infinite3_projects
#check @Storage3.infinite21_lifts
#check @Storage3.trashed_reachable

-- s3.1 (model 614f82a8): the whole guarantee; old-only runs are s2.1 runs; load is reachable and useful.
#check @Storage31.guarantee_positiveTexts7
#check @Storage31.uniform_guarantee7
#check @Storage31.no_new_actions_are_s21_runs7
#check @Storage31.infinite_old_runs_exact7
#check @Storage31.infinite_core_lifts7
#check @Storage31.load_delete_enabled7

-- s3.2 (model 1e6ec36c), round 8, PARTIAL (superseded by s3.5): the s3.1 guarantee on runs without a full rename.
#check @Storage32.noFull_refines31_8
#check @Storage32.noFull_guarantee8

-- s3.3 (model 353162e9), round 8, PARTIAL (superseded by s3.5): conservativity, order/resolution without a move, one-step opApply lemmas.
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

-- s3.4 (model ddcc4332), round 9, PARTIAL for the whole guarantee and renameResolves; orderHolds, name-slot, trash, bounds and conservativity are unrestricted.
#check @Storage34.no_extClaim_runs_exact_s33_9
#check @Storage34.no_opApply_extClaim_runs_exact_s32_9
#check @Storage34.no_new_actions_are_s21_runs9
#check @Storage34.noClaim_stream_projects33_9
#check @Storage34.oldOnly_infinite_projects21_9
#check @Storage34.name_slot_history9
#check @Storage34.apply_no_claimed_create9
#check @Storage34.apply_guarantee9
#check @Storage34.projects_reachable33_9
#check @Storage34.guarantee_universal_iff_s33_9
#check @Storage34.order_universal_iff_s33_9
#check @Storage34.resolution_universal_iff_s33_9
#check @Storage32.noRestore_guarantee9
#check @Storage34.noApplyRestore_guarantee34_9
#check @Storage34.noMove_order_resolves34_9
#check @Storage34.trashed_reachable34_9
#check @Storage34.bounds_reachable34_9
#check @Storage32.discard_bad9
#check @Storage32.backed_gated9
#check @Storage32.order_flush9
#check @Storage32.backed_discharge9
#check @Storage32.backed_discarded9
#check @Storage32.complete_discharge9
#check @Storage32.complete_discarded9
#check @Storage32.makeObl_backed9
#check @Witness9.full_move_reachable9
#check @Witness9.full_move_completed9
#check @Witness9.full_move_guarantee9
#check @Storage32.orderHolds_reachable32_9
#check @Storage33.orderHolds_reachable33_9
#check @Storage34.orderHolds_reachable34_9

-- s3.5 (model 7ebed877), round 10: the whole guarantee and orderHolds for every reachable state; conservativity; local lemmas; witnesses.
#check @Storage35.noApply_guarantee35_10
#check @Storage35.guarantee_reachable35_10
#check @Storage35.inv_reachable35_10
#check @Storage35.apply_inv35_10
#check @Storage35.orderHolds_reachable35_10
#check @Storage35.name_slot_history35_10
#check @Storage35.trashed_reachable35_10
#check @Storage35.aux_reachable10
#check @Storage35.compatible_runs_exact34_10
#check @Storage35.compatible_stream_exact34_10
#check @Storage35.compatible_reachable34_10
#check @Storage35.old_only_are_s21_runs35_10
#check @Storage35.renameResolves_noDeleteStart35_10
#check @Storage35.pairSafety_resolves10
#check @Storage35.pairSafety_flushDelete10
#check @Storage35.fileRename_resolves_iff10
#check @Storage31.silent_inv10
#check @Storage35Base.factor_restore10
#check @Storage31.batch_inv10
#check @Storage33.apply_eq_batch10
#check @Witness35.move_reachable10
#check @Witness35.restoration_reachable10
#check @Witness35.apply_move_reachable10
#check @Witness35.outcomes10
#check @Witness35.all_guarantees10
#check @Witness35.overlapping_joint_inv10
#check @Witness35.overlapping_orders10
#check @Witness35.candidate_not_inductive10

-- s3.5 (model 7ebed877), round 11: renameResolves for every reachable state, and the joint theorem; supporting invariants.
#check @Storage35.gateOwnership_reachable35_11
#check @Storage35.inputTexts_reachable35_11
#check @Storage35.pairPublished_reachable35_11
#check @Storage35.livePairs_reachable35_11
#check @Storage35.pairSafety_discard11
#check @Storage35.pairSafety_fileRename11
#check @Storage35.pairSafety_dirSync11
#check @Storage35.pairSafety_opRename11
#check @Storage35.pairSafety_apply11
#check @Storage35.pairSafety_execute11
#check @Storage35.pairSafety_reachable35_11
#check @Storage35.renameResolves_reachable35_11
#check @Storage35.all_targets_reachable35_11
#check @Witness35.completed_targets11
#check @Witness35.overlapping_not_owned11
#check @Witness35.overlapping_unreachable11

-- Independent verifiers (rounds 9, 10, 11): literal statement identities and non-vacuity witnesses.
#check @Verifier9.ordering
#check @Verifier9.literal_restricted_guarantee
#check @Verifier10.literal_identity
#check @Verifier10.literal_order
#check @Verifier10.literal_resolution
#check @Verifier10.uniformGuarantee
#check @Verifier11.literal_guarantee
#check @Verifier11.literal_order
#check @Verifier11.literal_resolution
#check @Verifier11.uniform_targets
#check @Verifier11.every_enabled_step
#check @Verifier11.completed_move_nonvacuous
#check @Verifier11.completed_apply_nonvacuous
#check @Verifier11.discard_retains_pair
#check @Verifier11.literal_rejects_missing
#check @Verifier11.literal_rejects_missing_down

#print axioms Storage.guarantee_positiveTexts
#print axioms Storage3.guarantee_positiveTexts6
#print axioms Storage31.guarantee_positiveTexts7
#print axioms Storage34.orderHolds_reachable34_9
#print axioms Storage35.guarantee_reachable35_10
#print axioms Storage35.orderHolds_reachable35_10
#print axioms Storage35.renameResolves_reachable35_11
#print axioms Storage35.all_targets_reachable35_11

-- Walk checked types and values of the headline constants; fail on a nonstandard axiom or unsafe constant.
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [
    ``Storage.guarantee_positiveTexts,
    ``Storage.noLoss_positiveTexts,
    ``Storage.MC_nonvacuous,
    ``Storage.G_mutant_nonvacuous,
    ``VerifierA.reachable_A_counterexamples,
    ``Storage3.guarantee_positiveTexts6,
    ``Storage3.initialized_old_runs_exact,
    ``Storage3.infinite3_projects,
    ``Storage3.infinite21_lifts,
    ``Storage3.trashed_reachable,
    ``Storage31.guarantee_positiveTexts7,
    ``Storage31.uniform_guarantee7,
    ``Storage31.no_new_actions_are_s21_runs7,
    ``Storage31.infinite_old_runs_exact7,
    ``Storage31.infinite_core_lifts7,
    ``Storage31.load_delete_enabled7,
    ``Storage32.noFull_refines31_8,
    ``Storage32.noFull_guarantee8,
    ``Storage33.no_opApply_runs_exact_s32_8,
    ``Storage33.prior_stream_exact8,
    ``Storage33.no_new_actions_are_s21_runs8,
    ``Storage33.infinite_old_runs_exact8,
    ``Storage33.infinite_core_lifts8,
    ``Storage33.noMove_order_resolves8,
    ``Storage33.apply_admission8,
    ``Storage33.apply_effects_drafted8,
    ``Storage33.apply_bad8,
    ``Storage33.apply_guarantee8,
    ``Storage33.apply_promise8,
    ``Storage33.apply_new_custody8,
    ``Verifier8.delete_nonvacuous,
    ``Verifier8.refs_nonvacuous,
    ``Verifier8.apply_nonvacuous,
    ``Storage34.no_extClaim_runs_exact_s33_9,
    ``Storage34.no_opApply_extClaim_runs_exact_s32_9,
    ``Storage34.no_new_actions_are_s21_runs9,
    ``Storage34.noClaim_stream_projects33_9,
    ``Storage34.oldOnly_infinite_projects21_9,
    ``Storage34.name_slot_history9,
    ``Storage34.apply_no_claimed_create9,
    ``Storage34.apply_guarantee9,
    ``Storage34.projects_reachable33_9,
    ``Storage34.guarantee_universal_iff_s33_9,
    ``Storage34.order_universal_iff_s33_9,
    ``Storage34.resolution_universal_iff_s33_9,
    ``Storage32.noRestore_guarantee9,
    ``Storage34.noApplyRestore_guarantee34_9,
    ``Storage34.noMove_order_resolves34_9,
    ``Storage34.trashed_reachable34_9,
    ``Storage34.bounds_reachable34_9,
    ``Storage32.discard_bad9,
    ``Storage32.backed_gated9,
    ``Storage32.order_flush9,
    ``Storage32.backed_discharge9,
    ``Storage32.backed_discarded9,
    ``Storage32.complete_discharge9,
    ``Storage32.complete_discarded9,
    ``Storage32.makeObl_backed9,
    ``Witness9.full_move_reachable9,
    ``Witness9.full_move_completed9,
    ``Witness9.full_move_guarantee9,
    ``Storage32.orderHolds_reachable32_9,
    ``Storage33.orderHolds_reachable33_9,
    ``Storage34.orderHolds_reachable34_9,
    ``Storage35.noApply_guarantee35_10,
    ``Storage35.guarantee_reachable35_10,
    ``Storage35.inv_reachable35_10,
    ``Storage35.apply_inv35_10,
    ``Storage35.orderHolds_reachable35_10,
    ``Storage35.name_slot_history35_10,
    ``Storage35.trashed_reachable35_10,
    ``Storage35.aux_reachable10,
    ``Storage35.compatible_runs_exact34_10,
    ``Storage35.compatible_stream_exact34_10,
    ``Storage35.compatible_reachable34_10,
    ``Storage35.old_only_are_s21_runs35_10,
    ``Storage35.renameResolves_noDeleteStart35_10,
    ``Storage35.pairSafety_resolves10,
    ``Storage35.pairSafety_flushDelete10,
    ``Storage35.fileRename_resolves_iff10,
    ``Storage31.silent_inv10,
    ``Storage35Base.factor_restore10,
    ``Storage31.batch_inv10,
    ``Storage33.apply_eq_batch10,
    ``Witness35.move_reachable10,
    ``Witness35.restoration_reachable10,
    ``Witness35.apply_move_reachable10,
    ``Witness35.outcomes10,
    ``Witness35.all_guarantees10,
    ``Witness35.overlapping_joint_inv10,
    ``Witness35.overlapping_orders10,
    ``Witness35.candidate_not_inductive10,
    ``Storage35.gateOwnership_reachable35_11,
    ``Storage35.inputTexts_reachable35_11,
    ``Storage35.pairPublished_reachable35_11,
    ``Storage35.livePairs_reachable35_11,
    ``Storage35.pairSafety_discard11,
    ``Storage35.pairSafety_fileRename11,
    ``Storage35.pairSafety_dirSync11,
    ``Storage35.pairSafety_opRename11,
    ``Storage35.pairSafety_apply11,
    ``Storage35.pairSafety_execute11,
    ``Storage35.pairSafety_reachable35_11,
    ``Storage35.renameResolves_reachable35_11,
    ``Storage35.all_targets_reachable35_11,
    ``Witness35.completed_targets11,
    ``Witness35.overlapping_not_owned11,
    ``Witness35.overlapping_unreachable11,
    ``Verifier9.ordering,
    ``Verifier9.literal_restricted_guarantee,
    ``Verifier10.literal_identity,
    ``Verifier10.literal_order,
    ``Verifier10.literal_resolution,
    ``Verifier10.uniformGuarantee,
    ``Verifier11.literal_guarantee,
    ``Verifier11.literal_order,
    ``Verifier11.literal_resolution,
    ``Verifier11.uniform_targets,
    ``Verifier11.every_enabled_step,
    ``Verifier11.completed_move_nonvacuous,
    ``Verifier11.completed_apply_nonvacuous,
    ``Verifier11.discard_retains_pair,
    ``Verifier11.literal_rejects_missing,
    ``Verifier11.literal_rejects_missing_down]
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
