import OrderJobs9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

-- Primitive metadata cases are proved without reading page bytes.
theorem safe_orders_discard_action9 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (h : SafeOrders x) : SafeOrders (upDiscard c x {x.s with up := tail} m ok) := by
  simp only [upDiscard]
  split
  · apply safe_orders_same9 (x := x) (h := h)
    · rfl
    · rfl
    · rfl
    · exact fun on => ⟨on,rfl⟩
  · split
    · apply safe_orders_same9 (x := x) (h := h)
      · rfl
      · rfl
      · rfl
      · simp only [job_commit32_9,job_ackMail9]
        exact fun on => ⟨on,by trivial⟩
    · apply safe_orders_discarded9 (x := x) (h := h) (k := m.p)
      · rfl
      · rfl
      · rfl
      · simp only [job_commit32_9,job_ackMail9,job_discarded9]


theorem safe_orders_stop9 (x y : State P) (h : SafeOrders x)
    (gates : y.s.gates = []) (obl : y.g.obl = []) (ok : y.g.orderOk = x.g.orderOk)
    (job : y.s.job.on = false) : SafeOrders y := by
  refine ⟨⟨?_,?_,?_,?_⟩,?_⟩
  · simp [obl]
  · simp [gates]
  · simp [job]
  · rw [ok]; exact h.1.2.2.2
  · simp [GateValid,gates]

theorem safe_orders_dirSync9 (c : Config P) (x : State P) (ok : Bool)
    (mutant : c.mutant = "none") (h : SafeOrders x)
    (on : x.s.job.on = true) : SafeOrders (dirSync c x ok) := by
  cases ok with
  | false =>
    apply safe_orders_same9 x _ h
    · simp [dirSync,mutant,lift]
    · rfl
    · rfl
    · simp [dirSync,mutant,lift,Storage3.dirSync,Storage3.lift,Storage21.dirSync,nojob]
  | true =>
    apply safe_orders_published9 x _ h x.s.job.p x.s.job.ver
    · simp [dirSync,mutant,lift,discharge]
    · rfl
    · rfl
    · simp [dirSync,mutant,lift,Storage3.dirSync,Storage3.lift,Storage21.dirSync,nojob,discharge]
    · exact h.1.2.2.1 on

-- Old actions that neither start nor retire orders only need the job frame.
def OrderFrame (a : Storage21.Action P) : Prop :=
  (∀ p,a ≠ .flush p) ∧ (∀ ok,a ≠ .deliverUp ok) ∧ (∀ ok,a ≠ .dirSync ok) ∧
  a ≠ .crash ∧ a ≠ .switchFin ∧ (∀ keep,a ≠ .power keep)

theorem old_order_frame9 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (frame : OrderFrame a) :
    (execute c x (.old a)).s.gates = x.s.gates ∧
    (execute c x (.old a)).g.obl = x.g.obl ∧ (execute c x (.old a)).g.orderOk = x.g.orderOk := by
  cases a <;> simp only [execute]
  case flush p => exact False.elim (frame.1 p rfl)
  case deliverUp ok => exact False.elim (frame.2.1 ok rfl)
  case dirSync ok => exact False.elim (frame.2.2.1 ok rfl)
  case crash => exact False.elim (frame.2.2.2.1 rfl)
  case switchFin => exact False.elim (frame.2.2.2.2.1 rfl)
  case power keep => exact False.elim (frame.2.2.2.2.2 keep rfl)
  case check => simp [check,mutant,lift]
  case rename => simp [rename,mutant,lift]
  case saveFail => simp [saveFail,mutant,lift]
  all_goals exact ⟨rfl,rfl,rfl⟩

#print axioms safe_orders_discard_action9
#print axioms safe_orders_dirSync9
#print axioms old_order_frame9
end Storage32
