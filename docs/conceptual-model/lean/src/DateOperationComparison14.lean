/- Comparison of every raw old selected candidate with the normalized literal
candidate. Covers any authorized old r13 choice, including file-less sources. -/

import DateWitness13

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (D : Dates K) (M : DatePolicy K) (SP : StoredParser L)

def refusalChecks (E : Engine) (G : Graph) (extras : List String) (old : String) (c : Candidate) : List (String × Bool) :=
  [("source support",supported P D M G old),
   ("key consistency",consistent (demands P D M SP G extras old c)),
   ("touched conflicts",localClean P D M SP G extras old c),
   ("content pair",pairReady P D M G old c),
   ("output claims",ownership P D M E G old c),
   ("virtual capture",noCapture P D M G old c),
   ("input paths",decide ((G.map File.path).Nodup)),("output paths",decide (((execute P D M E G old c).map File.path).Nodup))]

theorem checks_safe (E : Engine) (G : Graph) (extras : List String) (old : String) (c : Candidate) :
    (refusalChecks P D M SP E G extras old c).all Prod.snd = safe P D M SP E G extras old c := by
  simp [refusalChecks,safe,WF,Bool.and_assoc]

def operationDifference (E : Engine) (G : Graph) (extras : List String) (old new : String) (oldC : Candidate) : Prop :=
  let c := literal P D M G old new
  oldC.destination ≠ c.destination ∨ oldC.survivor ≠ c.survivor ∨
  source P D M G old ≠ Consolidated13.source P G old ∨
  (∃ s, subst P D M old c s ≠ Consolidated13.subst P old oldC s) ∨
  ∃ label, (label,false) ∈ refusalChecks P D M SP E G extras old c

theorem operation_comparison_exhaustive (E : Engine) (G : Graph) (extras : List String)
    (old new : String) (oldC : Candidate) :
    operationDifference P D M SP E G extras old new oldC ∨
      ∃ c, defaultCandidate P D M SP E G extras old new = some c ∧
        Consolidated13.execute P E G old oldC = execute P D M E G old c := by
  classical
  by_cases hd : operationDifference P D M SP E G extras old new oldC
  · exact Or.inl hd
  right
  let c := literal P D M G old new
  have hn : oldC.destination = c.destination := by
    exact Classical.byContradiction fun h => hd (Or.inl h)
  have hs : oldC.survivor = c.survivor := by
    exact Classical.byContradiction fun h => hd (Or.inr (Or.inl h))
  have hsource : source P D M G old = Consolidated13.source P G old := by
    exact Classical.byContradiction fun h => hd (Or.inr (Or.inr (Or.inl h)))
  have hm : subst P D M old c = Consolidated13.subst P old oldC := by
    funext s
    exact Classical.byContradiction fun h => hd (Or.inr (Or.inr (Or.inr (Or.inl ⟨s,h⟩))))
  have hall : (refusalChecks P D M SP E G extras old c).all Prod.snd = true := by
    apply List.all_eq_true.mpr
    intro x hx
    cases hb : x.2 with
    | true => rfl
    | false =>
      exact False.elim (hd (Or.inr (Or.inr (Or.inr (Or.inr ⟨x.1,by have he : x = (x.1,false) := Prod.ext rfl hb; rw [he] at hx; exact hx⟩)))))
  have hsafe : safe P D M SP E G extras old c = true := by rw [← checks_safe]; exact hall
  refine ⟨c,?_,?_⟩
  · simp [defaultCandidate,c,hsafe]
  · unfold Consolidated13.execute Date13.execute
    rw [hsource,hm,← hs]
    rfl

end Date13
end CM
