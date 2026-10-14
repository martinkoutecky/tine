import CoreB21
import CoreLifetimeClosure21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
def InactivePages (x : State P) : Prop := x.s.alive = false → ∀ p, (x.s.pages p).held = false

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
end Storage21
