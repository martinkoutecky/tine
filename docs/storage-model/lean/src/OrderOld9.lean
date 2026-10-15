import OrderActions9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem safe_orders_old9 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : SafeOrders x) (en : enabled c x (.old a) = true) :
    SafeOrders (execute c x (.old a)) := by
  by_cases start : ∃ p,a = .flush p
  · obtain ⟨p,rfl⟩ := start
    have gate : gated c x.s p = false := by
      simp only [enabled,Bool.and_eq_true] at en
      simpa using en.2
    exact safe_orders_flush9 c x p mutant h gate
  by_cases delivery : ∃ ok,a = .deliverUp ok
  · obtain ⟨ok,rfl⟩ := delivery
    simp only [execute,deliverUp]
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [hq]
      split
      · exact safe_orders_discard_action9 c x m tail ok h
      · apply safe_orders_same9 (x := x) (h := h)
        · rfl
        · rfl
        · rfl
        · exact job_follows31_9 c (project x) (.old (.deliverUp ok)) mutant (by constructor <;> intro p <;> simp)
  by_cases publication : ∃ ok,a = .dirSync ok
  · obtain ⟨ok,rfl⟩ := publication
    have raw : (x.s.alive && x.s.job.on && x.s.job.phase == 3) = true :=
      enabled_projects9 c x (.old (.dirSync ok)) en
    simp only [Bool.and_eq_true] at raw
    exact safe_orders_dirSync9 c x ok mutant h raw.1.2
  by_cases stop : a = .crash ∨ a = .switchFin ∨ ∃ keep,a = .power keep
  · rcases stop with rfl | rfl | ⟨keep,rfl⟩
    all_goals apply safe_orders_stop9 (x := x) (h := h)
    all_goals rfl
  have frame : OrderFrame a := by
    refine ⟨?_,?_,?_,?_,?_,?_⟩
    · intro p eq; exact start ⟨p,eq⟩
    · intro ok eq; exact delivery ⟨ok,eq⟩
    · intro ok eq; exact publication ⟨ok,eq⟩
    · intro eq; exact stop (Or.inl eq)
    · intro eq; exact stop (Or.inr (Or.inl eq))
    · intro keep eq; exact stop (Or.inr (Or.inr ⟨keep,eq⟩))
  have fields := old_order_frame9 c x a mutant frame
  apply safe_orders_same9 (x := x) (h := h)
  · exact fields.1
  · exact fields.2.1
  · exact fields.2.2
  · exact job_follows32_9 c x (.old a) mutant (by
      constructor
      · intro p eq; cases eq; exact start ⟨p,rfl⟩
      · intro p eq; cases eq)
end Storage32
