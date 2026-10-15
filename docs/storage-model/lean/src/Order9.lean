import OrderOperations9
import Reduction9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem safe_orders_execute32_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : SafeOrders x) (nonnegative : 0 ≤ x.g.vc)
    (en : enabled c x a = true) : SafeOrders (execute c x a) := by
  cases a with
  | old a => exact safe_orders_old9 c x a mutant h en
  | opRename src dst refs rt => exact safe_orders_opRename9 c x src dst refs rt mutant h nonnegative en
  | flushDel p =>
    have gate : gated c x.s p = false := by
      simp only [enabled,Bool.and_eq_true] at en
      simpa [mutant] using en.2
    exact safe_orders_flush9 c x p mutant h gate
  | opDelete p =>
    apply safe_orders_same9 (x := x) (h := h)
    · rfl
    · rfl
    · rfl
    · exact fun on => ⟨on,rfl⟩
  | load p =>
    apply safe_orders_same9 (x := x) (h := h)
    · rfl
    · rfl
    · rfl
    · exact fun on => ⟨on,rfl⟩

theorem safe_orders_reachable32_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : SafeOrders x := by
  induction hr with
  | initial => simp [SafeOrders,OrderInv,GateValid,initWith,embed,Storage31.initWith,Storage3.initWith,Storage3.embed,Storage21.init,nojob]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      exact safe_orders_execute32_9 c x a mutant ih (bounds_reachable32_9 c disk x mutant hr).nonnegative (by assumption)
    · contradiction

theorem orderHolds_reachable32_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : orderHolds x = true :=
  (safe_orders_reachable32_9 c disk x mutant hr).1.2.2.2
end Storage32
namespace Storage33
open Storage21
set_option maxHeartbeats 4000000

theorem safe_orders_reachable33_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : Storage32.SafeOrders x := by
  induction hr with
  | initial => exact Storage32.safe_orders_reachable32_9 c disk _ mutant .initial
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      have nn := (bounds_reachable33_9 c disk x mutant hr).nonnegative
      cases a with
      | prior a => exact Storage32.safe_orders_execute32_9 c x a mutant ih nn en
      | opApply eff plan dst src => exact safe_orders_apply9 c x eff plan dst src mutant ih nn en
    · contradiction

theorem orderHolds_reachable33_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : orderHolds x = true :=
  (safe_orders_reachable33_9 c disk x mutant hr).1.2.2.2
end Storage33
namespace Storage34
open Storage21
set_option maxHeartbeats 4000000

theorem orderHolds_reachable34_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : orderHolds x = true :=
  Storage33.orderHolds_reachable33_9 c disk (project x) mutant (projects_reachable33_9 c disk x mutant hr)
#print axioms orderHolds_reachable34_9
end Storage34
