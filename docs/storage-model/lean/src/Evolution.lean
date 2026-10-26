import Carriers
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

/-- A surviving buffer/version pair is retained or allocated above the old counter. -/
def PageEvolution (x : State P) (s : Sys P) : Prop := ∀ p, (s.pages p).held = true →
  ((x.s.pages p).held = true ∧ (s.pages p).ver = (x.s.pages p).ver ∧
    (s.pages p).buf = (x.s.pages p).buf) ∨ x.g.vc < (s.pages p).ver
attribute [reducible] PageEvolution

@[simp] theorem evolution_commit (c : Config P) (x z : State P) (s : Sys P) (g : Ghost P) (n : String) :
    PageEvolution x (commit c z s g n).s ↔ PageEvolution x s := Iff.rfl
@[simp] theorem evolution_ackMail (x : State P) (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool) :
    PageEvolution x (ackMail s p ok v took) ↔ PageEvolution x s := by
  cases ok <;> rfl

theorem evolution_refl (x : State P) : PageEvolution x x.s := fun p hp => Or.inl ⟨hp,rfl,rfl⟩
theorem evolution_put (x : State P) (s : Sys P) (p : Fin P) (pg : Page)
    (h : PageEvolution x s)
    (hp : pg.held = true → ((x.s.pages p).held = true ∧ pg.ver = (x.s.pages p).ver ∧
      pg.buf = (x.s.pages p).buf) ∨ x.g.vc < pg.ver) :
    PageEvolution x {s with pages := put s.pages p pg} := by
  intro q hq
  by_cases he : q = p
  · subst q; simpa [put] using hp (by simpa [put] using hq)
  · simpa [put,he] using h q (by simpa [put,he] using hq)

theorem evolution_table (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P) (d : Int) :
    (table c (x.s.pages p) d (x.g.vc+1)).held = true →
      ((x.s.pages p).held = true ∧ (table c (x.s.pages p) d (x.g.vc+1)).ver = (x.s.pages p).ver ∧
        (table c (x.s.pages p) d (x.g.vc+1)).buf = (x.s.pages p).buf) ∨
      x.g.vc < (table c (x.s.pages p) d (x.g.vc+1)).ver := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  all_goals intro hp
  all_goals first | exact Or.inl ⟨hp,rfl,rfl⟩ | (apply Or.inr; dsimp only; omega)

theorem evolution_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : PageEvolution x s) : PageEvolution x (upOpen c x s m ok).s := by
  simp only [upOpen,commit,ackMail]
  split <;> apply evolution_put x s m.p _ h
  all_goals split
  all_goals first
    | exact h m.p
    | (intro _; apply Or.inr; dsimp only; omega)

theorem evolution_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : PageEvolution x s) : PageEvolution x (upClose c x s m).s := by
  simp only [upClose,commit]; split
  · exact evolution_put x s m.p {} h (by simp)
  · exact h
theorem evolution_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : PageEvolution x s) : PageEvolution x (upDiscard c x s m ok).s := by
  simp only [upDiscard]; split
  · exact h
  · split
    · simp only [commit,ackMail]; split <;> exact h
    · simp only [commit,ackMail]; split
      all_goals apply evolution_put x s m.p _ h
      all_goals intro _; apply Or.inr; dsimp only; omega

theorem evolution_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (h : PageEvolution x s) : PageEvolution x (upSubmit c x s m).s := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,show ("none" != "MIS") = true from rfl,
    Bool.and_true,Bool.false_eq_true,ite_false]
  split
  · exact h
  · simp only [commit,ackMail]; split
    all_goals apply evolution_put x s m.p _ h
    all_goals intro _; apply Or.inr
    all_goals simp only [submitTo,mutant]
    all_goals simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
    all_goals split <;> (try split) <;> dsimp only <;> omega

theorem evolution_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (h : PageEvolution x s) : PageEvolution x (upOp c x s m ok).s := by
  simp only [upOp]; split
  · exact h
  · split
    · simp only [commit,ackMail]; split <;> (try split) <;> exact h
    · simp only [commit,ackMail]; split <;> (try split)
      all_goals intro p hp
      all_goals dsimp only at hp ⊢
      all_goals simp only [put] at hp ⊢
      all_goals split <;> (try split)
      all_goals simp_all only [ite_true,ite_false]
      all_goals first
        | exact h p hp
        | (apply Or.inr
           simp only [submitTo,mutant]
           simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
           split <;> dsimp only <;> omega)

theorem evolution_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) : PageEvolution x (execute c x a).s := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact evolution_refl x
    | cons m tail =>
      have ht : PageEvolution x {x.s with up := tail} := evolution_refl x
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact evolution_upOpen c x _ m ok ht
        | exact evolution_upClose c x _ m ht
        | exact evolution_upDiscard c x _ m ok ht
        | exact evolution_upOp c mutant x _ m ok ht
        | exact evolution_upSubmit c mutant x _ m ht
  case observe p =>
    exact evolution_put x x.s p _ (evolution_refl x) (evolution_table c mutant x p _)
  case check =>
    simp only [check]; split
    · exact evolution_put x {x.s with job := nojob c} _ _ (evolution_refl x) (evolution_table c mutant x _ _)
    · exact evolution_refl x
  case dirSync ok =>
    simp only [dirSync]
    apply evolution_put x x.s _ _ (evolution_refl x)
    split
    · intro hp; exact Or.inl ⟨hp,rfl,rfl⟩
    · simp only [onReply]; split <;> (try split)
      all_goals intro hp; exact Or.inl ⟨hp,rfl,rfl⟩
  case saveFail =>
    simp only [saveFail]
    apply evolution_put x x.s _ _ (evolution_refl x)
    simp only [onReply]; split
    all_goals intro hp; exact Or.inl ⟨hp,rfl,rfl⟩
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> exact evolution_refl x
  case switchReq =>
    intro p hp
    simp only [switchReq,commit] at hp ⊢
    split <;> simp_all
  case switchFin => simp [PageEvolution,switchFin,commit,down]
  case crash => simp [PageEvolution,crash,commit,down]
  case power keep => simp [PageEvolution,power,commit,down]
  case launch =>
    intro p hp
    simp only [launch,commit,restored] at hp ⊢
    split
    · simp_all
    · apply Or.inr; dsimp only; omega
  all_goals exact evolution_refl x
#print axioms evolution_execute
end Storage
