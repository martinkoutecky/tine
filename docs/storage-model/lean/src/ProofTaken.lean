import ProofC
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def TakenInv (x : State P) : Prop := "B'-submit-not-taken" ∉ x.g.bad

@[simp] theorem taken_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    TakenInv (commit c x s g n) ↔ "B'-submit-not-taken" ∉ g.bad := by
  simp [TakenInv,commit]

@[simp] theorem taken_bad_literal (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    "B'-submit-not-taken" ∉ (commit c x s g n).g.bad ↔ "B'-submit-not-taken" ∉ g.bad := by
  simp [commit]

theorem submitTo_takes (c : Config P) (mutant : c.mutant = "none")
    (pg : Page) (t bv v : Int) : (submitTo c pg t bv v).buf = t := by
  simp only [submitTo,mutant]
  simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
  split <;> rfl

theorem taken_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (hx : TakenInv x) : TakenInv (upSubmit c x s m) := by
  simp only [upSubmit]
  split
  · simpa [TakenInv,commit] using hx
  · rw [taken_commit]
    dsimp only
    split <;> simpa [TakenInv,submitTo_takes c mutant] using hx

theorem taken_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (hx : TakenInv x) : TakenInv (upOp c x s m ok) := by
  simp only [upOp]; split
  · simpa [TakenInv,commit] using hx
  · split
    · simpa [TakenInv,commit] using hx
    · rw [taken_commit]
      simp only [submitTo_takes c mutant]
      simpa [TakenInv,commit] using hx

theorem taken_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (hx : TakenInv x) : TakenInv (upOpen c x s m ok) := by
  simp only [upOpen]; split <;> (try split) <;> simpa [TakenInv,commit] using hx
theorem taken_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (hx : TakenInv x) : TakenInv (upClose c x s m) := by
  simpa [upClose,TakenInv,commit] using hx
theorem taken_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (hx : TakenInv x) : TakenInv (upDiscard c x s m ok) := by
  simp only [upDiscard]; split <;> (try split) <;> simpa [TakenInv,commit] using hx

theorem taken_deliverUp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) (hx : TakenInv x) :
    TakenInv (deliverUp c x m tail ok) := by
  simp only [deliverUp]
  split <;> (try split) <;> (try split) <;> (try split)
  all_goals first
    | exact taken_upOpen c x _ m ok hx
    | exact taken_upClose c x _ m hx
    | exact taken_upDiscard c x _ m ok hx
    | exact taken_upOp c mutant x _ m ok hx
    | exact taken_upSubmit c mutant x _ m hx

theorem taken_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (hx : TakenInv x) : TakenInv (execute c x a) := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact hx
    | cons m tail => exact taken_deliverUp c mutant x m tail ok hx
  case check =>
    simp only [check]; split <;> simpa [TakenInv,commit] using hx
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> simpa [TakenInv,commit] using hx
  case rename => simpa [rename,TakenInv,commit] using hx
  all_goals simpa [wOpen,wEdit,wSend,wResolve,wDiscard,wOp,wClose,wRecv,observe,
    flush,dirSync,saveFail,switchReq,switchFin,extWriteD,crash,windowCrash,power,launch,TakenInv,commit,gDown] using hx

/-- Exact tag absence, with no protocol, size, text, flag or enabled-action premise. -/
theorem Bprime2_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : "B'-submit-not-taken" ∉ x.g.bad := by
  induction hx with
  | initial => simp [init]
  | advance hr a hs ih =>
    unfold step at hs
    split at hs
    · cases hs; exact taken_execute c mutant _ a ih
    · contradiction

#print axioms submitTo_takes
#print axioms taken_execute
#print axioms Bprime2_reachable
end Storage
