import Witness35
import Lean
namespace Verifier10
open Storage21
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
def configuration (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) : Config P :=
  {positive := pos, texts := Storage3.positiveTexts4 T,
   nonempty := by simp [Storage3.positiveTexts4]; omega,
   r1, weak, crashes, powers, mutant := "none"}

-- Independently spell out the Quint predicate using the entire s3.5 state.
def literalGuarantee (x : Storage35.State P) : Bool :=
  (x.s.alive || (ids P).all (fun p =>
    let pr := x.g.promise p
    !pr.on || (pr.saved && x.g.ext p > pr.ep) ||
      ([x.s.disk p,(x.s.drafts p).bytes]).any (fun b =>
        b == pr.bytes || (x.g.wrote p).any (fun w => w.1 == b && w.2 > pr.ver)))) &&
  x.g.bad.isEmpty &&
  (!x.s.alive || x.g.owed.all (fun o => x.s.up.any (fun u => (carries u).contains o))) &&
  (ids P).all (fun p => (x.g.removed p).all (fun b =>
    (x.s.trash p).contains b || x.s.disk p == b))
theorem literal_identity (x : Storage35.State P) :
    Storage35.guarantee x = literalGuarantee x := rfl
theorem literal_order (x : Storage35.State P) :
    Storage35.orderHolds x = x.g.orderOk := rfl
theorem literal_resolution (x : Storage35.State P) :
    Storage35.renameResolves x =
      x.g.renamed.all (fun r => x.s.disk r.1 != absent || x.s.disk r.2 != absent) := rfl
theorem uniformGuarantee (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) (x : Storage35.State P)
    (hr : Storage35.Reachable (configuration P T pos two r1 weak crashes powers)
      (Storage31.initDisk (configuration P T pos two r1 weak crashes powers)) x) :
    literalGuarantee x = true :=
  Storage35.guarantee_reachable35_10 _ T rfl two rfl x hr
theorem initial_exists (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) :
    Storage35.Reachable (configuration P T pos two r1 weak crashes powers)
      (Storage31.initDisk (configuration P T pos two r1 weak crashes powers))
      (Storage35.init (configuration P T pos two r1 weak crashes powers)) := .initial
theorem literal_trash (c : Config P) (disk : Fin P → Int) (x : Storage35.State P)
    (hm : c.mutant = "none") (hr : Storage35.Reachable c disk x) :
    ∀ p, ∀ b ∈ x.g.removed p, b ∈ x.s.trash p ∨ x.s.disk p = b := by
  have h := Storage35.trashed_reachable35_10 c disk x hm hr
  intro p b hb
  have hp := List.all_eq_true.mp h p (by simp [ids])
  simpa only [Bool.or_eq_true,List.contains_iff_mem,beq_iff_eq,
    Storage32.project,Storage35.project,Storage34.project] using
    List.all_eq_true.mp hp b hb

-- Every enabled action, without exclusions, advances the actual relation.
theorem every_enabled_step (c : Config P) (disk : Fin P → Int)
    (x : Storage35.State P) (a : Storage35.Action P)
    (hr : Storage35.Reachable c disk x) (en : Storage35.enabled c x a = true) :
    Storage35.Reachable c disk (Storage35.execute c x a) :=
  .advance hr a (by simp [Storage35.step,en])
theorem noDeleteStart_prior (a : Storage32.Action P) :
    Storage35.NoDeleteStart10 (.prior a) ↔ ∀ p, a ≠ .flushDel p := by
  cases a <;> simp [Storage35.NoDeleteStart10,Storage35Base.NoDeleteStart10]

-- Retained pairs, real operations and source-restoring Discard are reachable.
theorem restoration_nonvacuous :
    Storage35.Reachable Witness35.c (Storage31.initDisk Witness35.c) Witness35.restored ∧
    Witness35.restored.g.renamed = [(Witness35.p0,Witness35.p2)] ∧
    (Witness35.restored.s.pages Witness35.p0).held = true ∧
    (Witness35.restored.s.pages Witness35.p0).buf = 1 := by
  exact ⟨Witness35.restoration_reachable10, by decide, by decide, by decide⟩
theorem completed_apply_nonvacuous :
    Storage35.Reachable Witness35.c (Storage31.initDisk Witness35.c) Witness35.applied ∧
    Witness35.applied.g.renamed = [(Witness35.p0,Witness35.p2)] ∧
    Witness35.applied.s.disk Witness35.p0 = absent ∧
    Witness35.applied.s.disk Witness35.p2 = 1 := by
  exact ⟨Witness35.apply_move_reachable10, by decide, by decide, by decide⟩
end Verifier10
#check Storage35.noApply_guarantee35_10
#print axioms Storage35.noApply_guarantee35_10
#check Storage35.guarantee_reachable35_10
#print axioms Storage35.guarantee_reachable35_10
#check Storage35.inv_reachable35_10
#print axioms Storage35.inv_reachable35_10
#check Storage35.apply_inv35_10
#print axioms Storage35.apply_inv35_10
#check Storage35.orderHolds_reachable35_10
#print axioms Storage35.orderHolds_reachable35_10
#check Storage35.name_slot_history35_10
#print axioms Storage35.name_slot_history35_10
#check Storage35.trashed_reachable35_10
#print axioms Storage35.trashed_reachable35_10
#check Storage35.aux_reachable10
#print axioms Storage35.aux_reachable10
#check Storage35.compatible_runs_exact34_10
#print axioms Storage35.compatible_runs_exact34_10
#check Storage35.compatible_stream_exact34_10
#print axioms Storage35.compatible_stream_exact34_10
#check Storage35.compatible_reachable34_10
#print axioms Storage35.compatible_reachable34_10
#check Storage35.old_only_are_s21_runs35_10
#print axioms Storage35.old_only_are_s21_runs35_10
#check Storage35.renameResolves_noDeleteStart35_10
#print axioms Storage35.renameResolves_noDeleteStart35_10
#check Storage35.pairSafety_resolves10
#print axioms Storage35.pairSafety_resolves10
#check Storage35.pending_gated10
#print axioms Storage35.pending_gated10
#check Storage35.pairSafety_flushDelete10
#print axioms Storage35.pairSafety_flushDelete10
#check Storage35.fileRename_resolves_iff10
#print axioms Storage35.fileRename_resolves_iff10
#check Storage31.silent_inv10
#print axioms Storage31.silent_inv10
#check Storage35Base.factor_restore10
#print axioms Storage35Base.factor_restore10
#check Storage31.batch_inv10
#print axioms Storage31.batch_inv10
#check Storage33.apply_eq_batch10
#print axioms Storage33.apply_eq_batch10
#check Witness35.move_reachable10
#print axioms Witness35.move_reachable10
#check Witness35.restoration_reachable10
#print axioms Witness35.restoration_reachable10
#check Witness35.apply_move_reachable10
#print axioms Witness35.apply_move_reachable10
#check Witness35.outcomes10
#print axioms Witness35.outcomes10
#check Witness35.all_guarantees10
#print axioms Witness35.all_guarantees10
#check Witness35.overlapping_joint_inv10
#print axioms Witness35.overlapping_joint_inv10
#check Witness35.overlapping_orders10
#print axioms Witness35.overlapping_orders10
#check Witness35.candidate_not_inductive10
#print axioms Witness35.candidate_not_inductive10
#check Verifier10.literal_identity
#print axioms Verifier10.literal_identity
#check Verifier10.literal_order
#print axioms Verifier10.literal_order
#check Verifier10.literal_resolution
#print axioms Verifier10.literal_resolution
#check Verifier10.uniformGuarantee
#print axioms Verifier10.uniformGuarantee
#check Verifier10.initial_exists
#print axioms Verifier10.initial_exists
#check Verifier10.literal_trash
#print axioms Verifier10.literal_trash
#check Verifier10.every_enabled_step
#print axioms Verifier10.every_enabled_step
#check Verifier10.noDeleteStart_prior
#print axioms Verifier10.noDeleteStart_prior
#check Verifier10.restoration_nonvacuous
#print axioms Verifier10.restoration_nonvacuous
#check Verifier10.completed_apply_nonvacuous
#print axioms Verifier10.completed_apply_nonvacuous
open Lean Elab Command
run_cmd do
  let env ← getEnv
  let mut todo := [``Storage35.noApply_guarantee35_10,``Storage35.guarantee_reachable35_10,``Storage35.inv_reachable35_10,``Storage35.apply_inv35_10,``Storage35.orderHolds_reachable35_10,``Storage35.name_slot_history35_10,``Storage35.trashed_reachable35_10,``Storage35.aux_reachable10,``Storage35.compatible_runs_exact34_10,``Storage35.compatible_stream_exact34_10,``Storage35.compatible_reachable34_10,``Storage35.old_only_are_s21_runs35_10,``Storage35.renameResolves_noDeleteStart35_10,``Storage35.pairSafety_resolves10,``Storage35.pending_gated10,``Storage35.pairSafety_flushDelete10,``Storage35.fileRename_resolves_iff10,``Storage31.silent_inv10,``Storage35Base.factor_restore10,``Storage31.batch_inv10,``Storage33.apply_eq_batch10,``Witness35.move_reachable10,``Witness35.restoration_reachable10,``Witness35.apply_move_reachable10,``Witness35.outcomes10,``Witness35.all_guarantees10,``Witness35.overlapping_joint_inv10,``Witness35.overlapping_orders10,``Witness35.candidate_not_inductive10,``Verifier10.literal_identity,``Verifier10.literal_order,``Verifier10.literal_resolution,``Verifier10.uniformGuarantee,``Verifier10.initial_exists,``Verifier10.literal_trash,``Verifier10.every_enabled_step,``Verifier10.noDeleteStart_prior,``Verifier10.restoration_nonvacuous,``Verifier10.completed_apply_nonvacuous]
  let mut seen : NameSet := {}
  let mut axs : NameSet := {}
  while !todo.isEmpty do
    let n := todo.head!
    todo := todo.tail!
    if seen.contains n then continue
    seen := seen.insert n
    let some info := env.checked.get.find? n | throwError "Missing checked constant {n}"
    if info.isUnsafe then throwError "Unsafe dependency {n}"
    if let .axiomInfo _ := info then
      unless n == ``propext || n == ``Classical.choice || n == ``Quot.sound do
        throwError "Nonstandard axiom {n}"
      axs := axs.insert n
    todo := info.type.getUsedConstants.toList ++ todo
    if let some value := info.value? (allowOpaque := true) then
      todo := value.getUsedConstants.toList ++ todo
    if let .inductInfo i := info then todo := i.ctors ++ todo
  logInfo m!"INDEPENDENT10_CONE {seen.size}; AXIOMS {axs.toList}"
