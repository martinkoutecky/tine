import Custody
import Nonvacuity
namespace VerifierA
open Storage
set_option maxRecDepth 32768
set_option maxHeartbeats 16000000

def cfg (mutant : String) : Config 2 :=
  {positive := by decide, texts := [1,2,3], nonempty := by decide, mutant := mutant}
def p1 : Fin 2 := ⟨1, by decide⟩
def uncertainTrace : List (Action 2) :=
  [.wOpen p0, .deliverUp true, .wRecv p0, .wEdit p0 2, .wSend p0,
   .deliverUp true, .wRecv p0, .flush p0, .check, .rename, .dirSync false,
   .observe p0, .power (fun _ => false)]
def moveTrace : List (Action 2) :=
  [.wOpen p0, .deliverUp true, .wRecv p0, .wOpen p1, .deliverUp true,
   .wRecv p1, .wOp p0 p1 3 3, .deliverUp true, .wRecv p0, .wRecv p1, .crash]
def volatileDiscardTrace : List (Action 2) :=
  [.wOpen p0, .deliverUp true, .wRecv p0, .wEdit p0 2, .wSend p0,
   .deliverUp true, .wRecv p0, .switchReq, .draftSync p0,
   .extWriteD p0 2 false, .wDiscard p0, .deliverUp true, .wRecv p0,
   .draftSync p0, .power (fun p => p.val == 1)]
def trace (m : String) : List (Action 2) :=
  if m == "MOD" || m == "MIS" then moveTrace else if m == "MDE" then volatileDiscardTrace else uncertainTrace
def result (m : String) : State 2 :=
  (walk (cfg m) (init (cfg m)) (trace m)).getD (init (cfg m))
def controls : List String := ["MQ", "MLS", "MRE", "MOD", "MIS", "MDE"]

theorem enabled_spec_controls :
    controls.all (fun m => (walk (cfg "none") (init (cfg "none")) (trace m)).isSome) = true := by decide
theorem safe_spec_controls :
    controls.all (fun m => noLoss ((walk (cfg "none") (init (cfg "none")) (trace m)).getD (init (cfg "none")))) = true := by decide

theorem enabled_controls :
    controls.all (fun m => (walk (cfg m) (init (cfg m)) (trace m)).isSome) = true := by decide
theorem loss_controls : controls.all (fun m => !noLoss (result m)) = true := by decide

theorem result_reachable (m : String) (mem : m ∈ controls) : Reachable (cfg m) (result m) := by
  have enabled := List.all_eq_true.mp enabled_controls m mem
  cases hw : walk (cfg m) (init (cfg m)) (trace m) with
  | none => simp [hw] at enabled
  | some x =>
    have reach := walk_reachable (cfg m) (trace m) (init (cfg m)) x Reachable.initial hw
    simpa [result,hw] using reach

theorem noLoss_fails (m : String) (mem : m ∈ controls) : noLoss (result m) = false := by
  have loss := List.all_eq_true.mp loss_controls m mem
  simpa using loss

theorem custody_fails (m : String) (mem : m ∈ controls) : ¬Custody (cfg m) (result m) := by
  intro custody
  have loss := noLoss_fails m mem
  have safe := noLoss_of_custody (cfg m) (result m) custody
  simp [loss] at safe

theorem reachable_A_counterexamples (m : String) (mem : m ∈ controls) :
    ∃ x, Reachable (cfg m) x ∧ noLoss x = false ∧ ¬Custody (cfg m) x :=
  ⟨result m, result_reachable m mem, noLoss_fails m mem, custody_fails m mem⟩

#print axioms enabled_controls
#print axioms enabled_spec_controls
#print axioms safe_spec_controls
#print axioms loss_controls
#print axioms result_reachable
#print axioms noLoss_fails
#print axioms custody_fails
#print axioms reachable_A_counterexamples
end VerifierA
