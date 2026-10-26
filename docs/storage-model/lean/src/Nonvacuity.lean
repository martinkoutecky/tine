import ProofC
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 2000000

def walk (c : Config P) (x : State P) : List (Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x a).bind fun y => walk c y as
theorem walk_reachable (c : Config P) (as : List (Action P)) (x y : State P)
    (hx : Reachable c x) (hw : walk c x as = some y) : Reachable c y := by
  induction as generalizing x y with
  | nil =>
    simp [walk] at hw
    subst y
    exact hx
  | cons a as ih =>
    cases hs : step c x a with
    | none => simp [walk,hs] at hw
    | some z =>
      exact ih z y (.advance hx a hs) (by simpa [walk,hs] using hw)

def mcConfig (r1 : Bool) (mutant : String) : Config 2 :=
  {positive := by decide, texts := [1,2,3], nonempty := by decide, r1, mutant}
def p0 : Fin 2 := ⟨0,by decide⟩
def mcPrefix : List (Action 2) :=
  [.wOpen p0, .deliverUp true, .wRecv p0, .wEdit p0 2, .wSend p0,
   .deliverUp true, .wRecv p0, .flush p0, .extWriteD p0 3 true, .check]
def checkedMC (r1 : Bool) (mutant : String) : State 2 :=
  (walk (mcConfig r1 mutant) (init (mcConfig r1 mutant)) mcPrefix).getD (init (mcConfig r1 mutant))
def renamedMC (r1 : Bool) (mutant : String) : State 2 :=
  (step (mcConfig r1 mutant) (checkedMC r1 mutant) .rename).getD (init (mcConfig r1 mutant))

-- These use kernel reduction of explicit executable traces, never native_decide.
theorem MC_prefix_enabled :
    (walk (mcConfig false "MC") (init (mcConfig false "MC")) mcPrefix).isSome = true := by decide
theorem MC1_prefix_enabled :
    (walk (mcConfig true "MC1") (init (mcConfig true "MC1")) mcPrefix).isSome = true := by decide
theorem MC_guard_fails : ¬Guarded (mcConfig false "MC") (checkedMC false "MC") := by unfold Guarded; decide
theorem MC1_guard_fails : ¬Guarded (mcConfig true "MC1") (checkedMC true "MC1") := by unfold Guarded; decide
theorem MC_records_C : CTag ∈ (renamedMC false "MC").g.bad := by decide
theorem MC1_records_C : CTag ∈ (renamedMC true "MC1").g.bad := by decide
theorem MC_rename_enabled :
    (step (mcConfig false "MC") (checkedMC false "MC") .rename).isSome = true := by decide
theorem MC1_rename_enabled :
    (step (mcConfig true "MC1") (checkedMC true "MC1") .rename).isSome = true := by decide

theorem checkedMC_reachable (r1 : Bool) (mutant : String)
    (h : (walk (mcConfig r1 mutant) (init (mcConfig r1 mutant)) mcPrefix).isSome = true) :
    Reachable (mcConfig r1 mutant) (checkedMC r1 mutant) := by
  unfold checkedMC
  cases hw : walk (mcConfig r1 mutant) (init (mcConfig r1 mutant)) mcPrefix with
  | none => simp [hw] at h
  | some y =>
    simp only [Option.getD_some]
    exact walk_reachable _ _ _ y (.initial) hw
theorem renamedMC_reachable (r1 : Bool) (mutant : String)
    (hp : (walk (mcConfig r1 mutant) (init (mcConfig r1 mutant)) mcPrefix).isSome = true)
    (hr : (step (mcConfig r1 mutant) (checkedMC r1 mutant) .rename).isSome = true) :
    Reachable (mcConfig r1 mutant) (renamedMC r1 mutant) := by
  unfold renamedMC
  cases hs : step (mcConfig r1 mutant) (checkedMC r1 mutant) .rename with
  | none => simp [hs] at hr
  | some y =>
    simp only [Option.getD_some]
    exact .advance (checkedMC_reachable r1 mutant hp) .rename hs

theorem MC_nonvacuous :
    ∃ x, Reachable (mcConfig false "MC") x ∧ CTag ∈ x.g.bad :=
  ⟨renamedMC false "MC",renamedMC_reachable false "MC" MC_prefix_enabled MC_rename_enabled,MC_records_C⟩
theorem MC1_nonvacuous :
    ∃ x, Reachable (mcConfig true "MC1") x ∧ CTag ∈ x.g.bad :=
  ⟨renamedMC true "MC1",renamedMC_reachable true "MC1" MC1_prefix_enabled MC1_rename_enabled,MC1_records_C⟩

#print axioms MC_guard_fails
#print axioms MC1_guard_fails
#print axioms MC_nonvacuous
#print axioms MC1_nonvacuous
end Storage
