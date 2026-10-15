import Witness11
import Lean
namespace Verifier11
open Storage21
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000

def configuration (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) : Config P :=
  {positive := pos, texts := Storage3.positiveTexts4 T,
   nonempty := by simp [Storage3.positiveTexts4]; omega,
   r1, weak, crashes, powers, mutant := "none"}

-- Independent spelling of the three literal Quint predicates.
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
def literalResolution (x : Storage35.State P) : Bool :=
  x.g.renamed.all (fun r => x.s.disk r.1 != absent || x.s.disk r.2 != absent)
theorem literal_guarantee (x : Storage35.State P) :
    Storage35.guarantee x = literalGuarantee x := rfl
theorem literal_order (x : Storage35.State P) : Storage35.orderHolds x = x.g.orderOk := rfl
theorem literal_resolution (x : Storage35.State P) :
    Storage35.renameResolves x = literalResolution x := rfl

theorem uniform_targets (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) (x : Storage35.State P)
    (hr : Storage35.Reachable (configuration P T pos two r1 weak crashes powers)
      (Storage31.initDisk (configuration P T pos two r1 weak crashes powers)) x) :
    literalGuarantee x = true ∧ x.g.orderOk = true ∧ literalResolution x = true :=
  Storage35.all_targets_reachable35_11 _ T rfl two rfl x hr

theorem initial_exists (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) :
    Storage35.Reachable (configuration P T pos two r1 weak crashes powers)
      (Storage31.initDisk (configuration P T pos two r1 weak crashes powers))
      (Storage35.init (configuration P T pos two r1 weak crashes powers)) := .initial

theorem every_enabled_step (c : Config P) (disk : Fin P → Int)
    (x : Storage35.State P) (a : Storage35.Action P)
    (hr : Storage35.Reachable c disk x) (en : Storage35.enabled c x a = true) :
    Storage35.Reachable c disk (Storage35.execute c x a) :=
  .advance hr a (by simp [Storage35.step,en])

-- Each auxiliary invariant is a conclusion, never an extra run premise.
theorem reachable_auxiliaries (c : Config P) (T : Nat)
    (texts : c.texts = Storage3.positiveTexts4 T) (mutant : c.mutant = "none")
    (disk : Fin P → Int) (x : Storage35.State P) (hr : Storage35.Reachable c disk x) :
    Storage32.GateOwnership11 x.s.gates ∧ InputTexts11 x.s.w x.s.up ∧
    Storage32.PairPublished11 (Storage35.project x) ∧ Storage35.LivePairs11 x :=
  ⟨Storage35.gateOwnership_reachable35_11 c disk x mutant hr,
   Storage35.inputTexts_reachable35_11 c disk x mutant
     (Storage35.positiveTexts_nondelete11 c T texts) hr,
   Storage35.pairPublished_reachable35_11 c disk x mutant hr,
   Storage35.livePairs_reachable35_11 c disk x mutant hr⟩

-- Source deletion really occurs with a retained pair in a legal run.
theorem completed_move_nonvacuous :
    Storage35.Reachable Witness35.c (Storage31.initDisk Witness35.c) Witness35.moved ∧
    Witness35.moved.g.renamed = [(Witness35.p0,Witness35.p2)] ∧
    Witness35.moved.s.disk Witness35.p0 = absent ∧
    Witness35.moved.s.disk Witness35.p2 = 1 :=
  ⟨Witness35.move_reachable10,by decide,by decide,by decide⟩
theorem completed_apply_nonvacuous :
    Storage35.Reachable Witness35.c (Storage31.initDisk Witness35.c) Witness35.applied ∧
    Witness35.applied.g.renamed = [(Witness35.p0,Witness35.p2)] ∧
    Witness35.applied.s.disk Witness35.p0 = absent ∧
    Witness35.applied.s.disk Witness35.p2 = 1 :=
  ⟨Witness35.apply_move_reachable10,by decide,by decide,by decide⟩
theorem discard_retains_pair :
    Storage35.Reachable Witness35.c (Storage31.initDisk Witness35.c) Witness35.restored ∧
    Witness35.restored.g.renamed = [(Witness35.p0,Witness35.p2)] ∧
    (Witness35.restored.s.pages Witness35.p0).buf = 1 :=
  ⟨Witness35.restoration_reachable10,by decide,by decide⟩

-- No exemption for running jobs or down states in the literal predicate.
def missingEndpoints : Storage35.State 3 :=
  {Witness35.waiting with s := {Witness35.waiting.s with
    disk := fun _ => absent, job := {Witness35.waiting.s.job with on := true,phase := 2}}}
theorem literal_rejects_missing :
    missingEndpoints.g.renamed ≠ [] ∧ missingEndpoints.s.job.on = true ∧
    Storage35.renameResolves missingEndpoints = false := by decide
theorem literal_rejects_missing_down :
    Storage35.renameResolves {missingEndpoints with s := {missingEndpoints.s with alive := false}} = false := by decide
end Verifier11

#check Storage35.renameResolves_reachable35_11
#print axioms Storage35.renameResolves_reachable35_11
#check Storage35.all_targets_reachable35_11
#print axioms Storage35.all_targets_reachable35_11
#check Storage35.gateOwnership_reachable35_11
#print axioms Storage35.gateOwnership_reachable35_11
#check Storage35.inputTexts_reachable35_11
#print axioms Storage35.inputTexts_reachable35_11
#check Storage35.pairPublished_reachable35_11
#print axioms Storage35.pairPublished_reachable35_11
#check Storage35.livePairs_reachable35_11
#print axioms Storage35.livePairs_reachable35_11
#check Storage35.pairSafety_reachable35_11
#print axioms Storage35.pairSafety_reachable35_11
#check Storage35.guarantee_reachable35_10
#print axioms Storage35.guarantee_reachable35_10
#check Storage35.orderHolds_reachable35_10
#print axioms Storage35.orderHolds_reachable35_10
#check Witness35.completed_targets11
#print axioms Witness35.completed_targets11
#check Witness35.overlapping_unreachable11
#print axioms Witness35.overlapping_unreachable11

open Lean Elab Command
run_cmd do
  let env ← getEnv
  let roots := [``Storage35.renameResolves_reachable35_11,``Storage35.all_targets_reachable35_11,
    ``Storage35.gateOwnership_reachable35_11,``Storage35.inputTexts_reachable35_11,
    ``Storage35.pairPublished_reachable35_11,``Storage35.livePairs_reachable35_11,
    ``Storage35.pairSafety_reachable35_11,``Storage35.guarantee_reachable35_10,
    ``Storage35.orderHolds_reachable35_10,``Witness35.completed_targets11,
    ``Witness35.overlapping_unreachable11,``Verifier11.uniform_targets,
    ``Verifier11.literal_guarantee,``Verifier11.literal_order,``Verifier11.literal_resolution,
    ``Verifier11.initial_exists,``Verifier11.every_enabled_step,``Verifier11.reachable_auxiliaries,
    ``Verifier11.completed_move_nonvacuous,``Verifier11.completed_apply_nonvacuous,
    ``Verifier11.discard_retains_pair,``Verifier11.literal_rejects_missing,
    ``Verifier11.literal_rejects_missing_down]
  for n in roots do
    let axs ← collectAxioms n
    for ax in axs do
      unless ax == ``propext || ax == ``Classical.choice || ax == ``Quot.sound do
        throwError "Nonstandard axiom {ax} in {n}"
    logInfo m!"INDEPENDENT11_AXIOMS {n}: {axs}"
  let mut todo := roots
  let mut seen : NameSet := {}
  let mut axioms : NameSet := {}
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
      axioms := axioms.insert n
    todo := info.type.getUsedConstants.toList ++ todo
    if let some value := info.value? (allowOpaque := true) then
      todo := value.getUsedConstants.toList ++ todo
    if let .inductInfo i := info then todo := i.ctors ++ todo
  logInfo m!"INDEPENDENT11_CONE {seen.size}; AXIOMS {axioms.toList}"
