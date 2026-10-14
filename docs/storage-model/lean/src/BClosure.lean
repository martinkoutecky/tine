import Lineage
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

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

theorem B_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (h : BInv x) (ht : TypedSeen x) :
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
              have seen := ht m.p held typed
              simp [keptB,resolve,versionEq,seen]
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

theorem B_deliverUp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : BInv x) (hc : CleanSafe x) (ht : TypedSeen x) : BInv (deliverUp c x m tail ok) := by
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals first
    | exact B_upOpen c x m tail ok h
    | exact B_upClose c mutant x m tail h hc
    | exact B_upDiscard c x m tail ok h
    | exact B_upOp c mutant x m tail ok h
    | exact B_upSubmit c mutant x m tail h ht

theorem draft_beq_eq (a b : Draft) : (a == b) = true ↔ a = b := by
  rcases a with ⟨ab,aa,av⟩; rcases b with ⟨bb,ba,bv⟩
  change (ab == bb && (aa == ba && av == bv)) = true ↔ _
  simp [beq_iff_eq]

theorem B_switchFin (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (h : BInv x) (hc : CleanSafe x) (hen : canSwitch c x = true) :
    BInv (switchFin c x) := by
  have rows : ∀ p, (!(x.s.w p).pend && !(x.s.w p).sent &&
    (!(x.s.pages p).held || clean (x.s.pages p) || (x.s.pages p).risk) &&
    (x.s.drafts p == draftEntry (x.s.pages p) || (c.mutant == "MX" && (x.s.pages p).held && (x.s.pages p).risk))) = true := by
    have all : (ids P).all (fun p => !(x.s.w p).pend && !(x.s.w p).sent &&
      (!(x.s.pages p).held || clean (x.s.pages p) || (x.s.pages p).risk) &&
      (x.s.drafts p == draftEntry (x.s.pages p) || (c.mutant == "MX" && (x.s.pages p).held && (x.s.pages p).risk))) = true := by
      simp only [canSwitch,Bool.and_eq_true] at hen; exact hen.2
    intro p; exact List.all_eq_true.mp all p (by simp [ids])
  apply B_commit
  · exact h
  · intro p
    have post : PostPromises x (down c x.s) (gDown x.g) p = x.g.promise p := by simp [PostPromises,down,gDown]
    by_cases held : (x.s.pages p).held = true
    · have row := rows p
      have choices : clean (x.s.pages p) = true ∨ (x.s.pages p).risk = true := by simp [held] at row; grind
      rcases choices with cl | risk
      · have typed : (x.s.pages p).typed = false := by simp [clean] at cl; grind
        rcases hc p cl with same | ⟨ver,bytes⟩
        · simp [keptB,PostPromises,down,gDown,same,typed]
        · simp [keptB,PostPromises,down,gDown,ver,bytes]
      · have drBool : (x.s.drafts p == draftEntry (x.s.pages p)) = true := by simp [mutant] at row; exact row.2
        have dr : x.s.drafts p = draftEntry (x.s.pages p) := (draft_beq_eq _ _).mp drBool
        simp [keptB,down,dr,draftEntry,held,risk]
    · simp [keptB,held]

theorem B_reachable (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) :
    "B-unsaved-replaced" ∉ x.g.bad := by
  apply B_reachable_of_residual c ?_ x hx
  intro y hy hb a hen which
  have hc := cleanSafe_reachable c mutant y hy
  have ht := typedSeen_reachable c T texts mutant y hy
  rcases which with ⟨p,rfl⟩ | ⟨ok,rfl⟩ | rfl | rfl | rfl
  · exact B_observe c mutant y p hb hc
  · simp only [execute]
    cases hq : y.s.up with
    | nil => exact hb
    | cons m tail => exact B_deliverUp c mutant y m tail ok hb hc ht
  · exact B_check c mutant y hb hc
  · exact B_switchFin c mutant y hb hc hen
  · exact B_launch c y hb (inactive_reachable c y hy) hen
#print axioms B_reachable
end Storage
