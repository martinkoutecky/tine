import CoreTyped21
import CoreBSwitch21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def StaleHeadProtection4 (x : State P) (m : Up P) : Prop :=
  (x.s.pages m.p).held = true → (x.s.pages m.p).typed = true →
  m.ro = noneBytes → m.bv ≠ (x.s.pages m.p).ver →
    (x.s.pages m.p).buf ∈ x.g.seen m.p ∨
      ((x.g.promise m.p).ver ≥ (x.s.pages m.p).ver ∧ (x.g.promise m.p).bytes = (x.s.pages m.p).buf)

theorem B_upOpen (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : BInv x) : BInv (upOpen c x {x.s with up := tail} m ok) := by
  simp only [upOpen]; split <;> (try split)
  all_goals apply B_commit_noReplacement
  all_goals first | exact h | skip
  all_goals intro p hh
  all_goals by_cases he : p = m.p
  all_goals simp_all [pages_ackMail,put]

theorem B_upDiscard (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : BInv x) : BInv (upDiscard c x {x.s with up := tail} m ok) := by
  simp only [upDiscard]; split
  · apply B_commit_noReplacement
    · simpa [BInv] using h
    · exact noReplacement_refl x
  · split
    · exact B_commit_noReplacement c x _ _ _ h (by simpa only [NoReplacement,pages_ackMail] using noReplacement_refl x)
    · apply B_commit
      · exact h
      · intro p; by_cases he : p = m.p
        · subst p; simp [keptB]
        · simp [keptB,put,he,pages_ackMail]

theorem B_upSubmit_protected4 (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (h : BInv x) (ht : StaleHeadProtection4 x m) :
    BInv (upSubmit c x {x.s with up := tail} m) := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
  split
  · apply B_commit_noReplacement
    · simpa [BInv] using h
    · exact noReplacement_refl x
  · apply B_commit
    · simpa [BInv,submitTo_takes c mutant] using h
    · intro p; by_cases he : p = m.p
      · subst p
        by_cases resolve : (m.ro != noneBytes) = true
        · simp [keptB,resolve]
        · by_cases versionEq : m.bv = (x.s.pages m.p).ver
          · simp [keptB,resolve,versionEq]
          · by_cases typed : (x.s.pages m.p).typed = true
            · have held : (x.s.pages m.p).held = true := by simpa using (by assumption : ¬ (!(x.s.pages m.p).held) = true)
              have ro : m.ro = noneBytes := by simpa using resolve
              rcases ht held typed ro versionEq with seen | ⟨ver,bytes⟩
              · simp [keptB,resolve,versionEq,seen]
              · simp [keptB,PostPromises,put,resolve,versionEq,ver,bytes,clean,submit_typed_exact c mutant]
            · simp [keptB,resolve,versionEq,typed]
      · simp [keptB,pages_ackMail,put,he]

theorem B_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) (h : BInv x) :
    BInv (upOp c x {x.s with up := tail} m ok) := by
  simp only [upOp]; split
  · apply B_commit_noReplacement
    · simpa [BInv] using h
    · exact noReplacement_refl x
  · split
    · exact B_commit_noReplacement c x _ _ _ h (by simpa only [NoReplacement,pages_ackMail] using noReplacement_refl x)
    · have empty : ([m.p,m.q].filter fun p => if p == m.p then m.bv != (x.s.pages m.p).ver else m.bv2 != (x.s.pages m.q).ver).isEmpty = true := by
        have go : ¬ ((!ok && c.mutant != "MOD") ||
          (!([m.p,m.q].filter fun p => if p == m.p then m.bv != (x.s.pages m.p).ver else m.bv2 != (x.s.pages m.q).ver).isEmpty && c.mutant != "MSM")) = true := by assumption
        have he : ok = true ∧ ([m.p,m.q].filter fun p => if p == m.p then m.bv != (x.s.pages m.p).ver else m.bv2 != (x.s.pages m.q).ver).isEmpty = true := by simpa [mutant] using go
        exact he.2
      have nil : ([m.p,m.q].filter fun p => if p == m.p then m.bv != (x.s.pages m.p).ver else m.bv2 != (x.s.pages m.q).ver) = [] := by simpa using empty
      apply B_commit
      · simpa [BInv,submitTo_takes c mutant] using h
      · intro p
        simp only [nil,List.isEmpty_nil]
        by_cases hp : p = m.p
        · subst p; simp [keptB,nil,exclude]
        · by_cases hq : p = m.q
          · subst p; simp [keptB,nil,exclude]
          · simp [keptB,pages_ackMail,put,hp,hq]


theorem B_delivery_protected4 (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : BInv x) (hc : CleanSafe x)
    (kinds : m.kind ∈ ["open","close","discard","submit","op"])
    (hp : m.kind = "submit" → StaleHeadProtection4 x m) :
    BInv (deliverUp c x m tail ok) := by
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  · exact B_upOpen c x m tail ok h
  · exact B_upClose c mutant x m tail h hc
  · exact B_upDiscard c x m tail ok h
  · exact B_upOp c mutant x m tail ok h
  · have kind : m.kind = "submit" := by simp_all
    exact B_upSubmit_protected4 c mutant x m tail h (hp kind)
end Storage21
