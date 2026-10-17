import ProofG
import Nonvacuity
namespace Storage
set_option maxRecDepth 32768
set_option maxHeartbeats 8000000

def gMutants : List String := ["MST","MRB","MBD","MDA"]
def stalePrefix : List (Action 2) :=
  [.wOpen p0,.deliverUp true,.wRecv p0,.wEdit p0 2,
   .extWriteD p0 3 true,.observe p0,.wSend p0,.deliverUp true,.wRecv p0]
def gPrefix (m : String) : List (Action 2) :=
  if m == "MST" then stalePrefix ++ [.flush p0,.check]
  else if m == "MDA" then
    [.wOpen p0,.deliverUp true,.wRecv p0,.wDiscard p0,.wEdit p0 3,
     .extWriteD p0 absent true,.deliverUp true,.wRecv p0,.wSend p0,.deliverUp true,.flush p0,.check]
  else stalePrefix ++ [.draftSync p0,.crash,.launch,.flush p0,.check]
def gChecked (m : String) : State 2 :=
  (walk (mcConfig false m) (init (mcConfig false m)) (gPrefix m)).getD (init (mcConfig false m))
def gRenamed (m : String) : State 2 :=
  (step (mcConfig false m) (gChecked m) .rename).getD (init (mcConfig false m))

theorem G_mutant_checks (m : String) (hm : m ∈ gMutants) :
    (walk (mcConfig false m) (init (mcConfig false m)) (gPrefix m)).isSome = true ∧
    pageBasesB (gChecked m) = false ∧ jobBaseB (gChecked m) = false ∧
    (step (mcConfig false m) (gChecked m) .rename).isSome = true ∧
    GTag ∈ (gRenamed m).g.bad := by
  simp [gMutants] at hm
  rcases hm with h | h | h | h <;> subst m <;> decide

theorem gChecked_reachable (m : String) (hm : m ∈ gMutants) :
    Reachable (mcConfig false m) (gChecked m) := by
  have hp := (G_mutant_checks m hm).1
  unfold gChecked
  cases hw : walk (mcConfig false m) (init (mcConfig false m)) (gPrefix m) with
  | none => simp [hw] at hp
  | some y =>
    simp only [Option.getD_some]
    exact walk_reachable _ _ _ y .initial hw

theorem gRenamed_reachable (m : String) (hm : m ∈ gMutants) :
    Reachable (mcConfig false m) (gRenamed m) := by
  have hp := (G_mutant_checks m hm).2.2.2.1
  unfold gRenamed
  cases hs : step (mcConfig false m) (gChecked m) .rename with
  | none => simp [hs] at hp
  | some y =>
    simp only [Option.getD_some]
    exact .advance (gChecked_reachable m hm) .rename hs

theorem G_mutant_nonvacuous (m : String) (hm : m ∈ gMutants) :
    ∃ x y, Reachable (mcConfig false m) x ∧ Reachable (mcConfig false m) y ∧
      ¬ PageBases x ∧ ¬ JobBase x ∧ GTag ∈ y.g.bad := by
  have checks := G_mutant_checks m hm
  refine ⟨gChecked m,gRenamed m,gChecked_reachable m hm,gRenamed_reachable m hm,?_,?_,checks.2.2.2.2⟩
  · intro h
    have he := pageBasesB_correct (gChecked m) |>.mpr h
    rw [checks.2.1] at he; contradiction
  · intro h
    have he := jobBaseB_correct (gChecked m) |>.mpr h
    rw [checks.2.2.1] at he; contradiction

theorem MBD_draftBase_fails : ¬ DraftBases (gChecked "MBD") := by
  have hb : draftBasesB (gChecked "MBD") = false := by decide
  intro h
  have he := draftBasesB_correct (gChecked "MBD") |>.mpr h
  rw [hb] at he; contradiction

#print axioms G_mutant_checks
#print axioms G_mutant_nonvacuous
#print axioms MBD_draftBase_fails
end Storage
