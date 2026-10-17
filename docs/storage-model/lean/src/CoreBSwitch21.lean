import CoreLineage21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
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

end Storage21
