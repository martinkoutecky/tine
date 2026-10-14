import ProofB
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
def InactivePages (x : State P) : Prop := x.s.alive = false → ∀ p, (x.s.pages p).held = false

@[simp] theorem alive_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    (commit c x s g n).s.alive = s.alive := rfl
@[simp] theorem alive_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool) :
    (ackMail s p ok v t).alive = s.alive := by cases ok <;> rfl
def aliveResult (x : State P) (a : Action P) : Bool := match a with
  | .crash | .power _ | .switchFin => false
  | .launch => true
  | _ => x.s.alive
theorem alive_execute (c : Config P) (x : State P) (a : Action P) :
    (execute c x a).s.alive = aliveResult x a := by
  cases a <;> simp only [execute,aliveResult]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => rfl
    | cons m tail =>
      simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp,alive_commit,alive_ackMail]
      all_goals (try split) <;> (try split) <;> (try split) <;> simp_all only [Bool.false_eq_true,ite_true,ite_false,alive_commit,alive_ackMail]
  case check => simp only [check]; split <;> rfl
  case draftSync p => simp only [draftSync]; split <;> (try split) <;> rfl
  all_goals rfl

theorem inactive_execute (c : Config P) (x : State P) (a : Action P)
    (hx : InactivePages x) (hen : enabled c x a = true) : InactivePages (execute c x a) := by
  intro dead p
  rw [alive_execute] at dead
  cases a <;> simp only [aliveResult] at dead
  case extWriteD q v dur => exact hx dead p
  case crash => rfl
  case power keep => rfl
  case switchFin => rfl
  case launch => contradiction
  all_goals simp [enabled,dead] at hen

theorem inactive_reachable (c : Config P) (x : State P) (hx : Reachable c x) : InactivePages x := by
  induction hx with
  | initial => simp [InactivePages,init]
  | advance hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact inactive_execute c _ a ih (by assumption)
    · contradiction

theorem B_launch (c : Config P) (x : State P) (hx : BInv x) (hi : InactivePages x)
    (hen : enabled c x .launch = true) : BInv (launch c x) := by
  have dead : x.s.alive = false := by simpa [enabled] using hen
  apply B_commit
  · exact hx
  · intro p; simp [keptB,hi dead p]
#print axioms inactive_reachable
#print axioms B_launch
end Storage
