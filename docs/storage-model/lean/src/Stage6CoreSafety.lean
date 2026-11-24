import Stage6Admission

namespace Stage6
open Storage21 Storage3

theorem step_eq_some_iff {c : Config P} {x y : State P} {a : Action P} :
    step c x a = some y ↔ enabled c x a = true ∧ execute c x a = y := by
  unfold step
  split <;> simp_all

theorem apply_enabled_legacy {c : Config P} {x : State P} {b : Batch P}
    (en : enabledApply c x b = true) :
    Storage34.enabledApply c x.admissionView b.eff b.plan none none = true :=
  (Bool.and_eq_true_iff.mp en).2

theorem apply_free {c : Config P} {x : State P} {b : Batch P}
    (en : enabledApply c x b = true) : Storage3.opFree x.core.s b.keys = true := by
  have h := apply_enabled_legacy en
  simp only [Storage34.enabledApply, Storage33.enabledApply, Bool.and_eq_true] at h
  exact h.1.1.1.1.1.1.1.2

private theorem prior_core_enabled {c : Config P} {x : State P} {a : Storage32.Action P}
    (en : enabledPrior c x a = true) : Storage31.enabled c x.core a = true :=
  (Bool.and_eq_true_iff.mp en).1

-- Only delivery can consult the expanded gates while updating the core.
theorem prior_core_step (c : Config P) (x : State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (en : enabledPrior c x a = true)
    (noNone : NoNone6 c) (noUnknown : NoUnknown6 c) (inv : Storage31.Inv c x.core) :
    Storage31.Inv c (execute c x (.prior a)).core := by
  have base := prior_core_enabled en
  cases a with
  | old a =>
    cases a with
    | deliverUp ok =>
      apply Storage35Base.inv_execute_noApply10 c (Storage35.project x.legacyView)
        (.old (.deliverUp ok)) normal noNone noUnknown inv
      exact en
    | _ =>
      all_goals
        change Storage31.Inv c (Storage32.project (Storage32.execute c (Storage35.project x.legacyView) _))
        rw [Storage32.execute_projects_noRestore9 c _ _ normal (by trivial)]
        exact Storage31.inv_execute c x.core _ normal noNone noUnknown inv base
  | _ =>
    all_goals exact Storage31.inv_execute c x.core _ normal noNone noUnknown inv base

theorem claim_projects (c : Config P) (x : State P) (p : Slot P) (present : Bool)
    (idle : Storage34.IdleClosed c x.legacyView) :
    (execute c x (.extClaim p present)).core = x.core :=
  congrArg Storage32.project (Storage34.extClaim_projects_idle9 c x.legacyView p present idle)

theorem idle_execute (c : Config P) (x : State P) (a : Action P)
    (normal : c.mutant = "none") (idle : Storage34.IdleClosed c x.legacyView) :
    Storage34.IdleClosed c (execute c x a).legacyView := by
  cases a with
  | malformedApply => exact idle
  | extClaim p present =>
    exact Storage34.idle_closed_execute34_9 c x.legacyView (.extClaim p present) normal idle
  | opApply b =>
    exact Storage34.idle_closed_execute34_9 c x.legacyView (.opApply b.eff b.plan none none) normal idle
  | prior a =>
    let z := Storage35.project x.legacyView
    apply Storage35Base.preserve10 c z a
      (fun y => Storage21.IdleClosed c (Storage3.project (Storage32.project y)))
    · exact Storage32.idle_closed_execute32_9 c z a normal idle
    · intro ok m tail _ _
      exact Storage35Base.idle_closed_discard32_9guard10 c z _ m ok

theorem reachable_valid {c : Config P} {disk : Slot P → Int} {x : State P}
    (reached : Reachable c disk x) : InitialValid c disk := by
  induction reached with
  | initial v => exact v
  | advance _ _ _ h => exact h

theorem core_safety_reachable (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) :
    Storage31.Inv c x.core ∧ Storage34.IdleClosed c x.legacyView := by
  have valid := reachable_valid reached
  have noNone : NoNone6 c := by
    intro b hb
    rcases hb with rfl | mem
    · decide
    · exact fun eq => valid.2.2.1 (eq ▸ mem)
  have noUnknown : NoUnknown6 c := by
    intro b hb
    rcases hb with rfl | mem
    · decide
    · exact fun eq => valid.2.2.2 (eq ▸ mem)
  induction reached with
  | initial =>
    refine ⟨Storage31.inv_initial c disk normal valid.1 noUnknown, rfl, ?_⟩
    intro p on _
    cases on
  | @advance x y reached a next ih =>
    obtain ⟨en, rfl⟩ := step_eq_some_iff.mp next
    refine ⟨?_, idle_execute c x a normal ih.2⟩
    cases a with
    | prior a => exact prior_core_step c x a normal en noNone noUnknown ih.1
    | opApply b => exact admission_preserves_core c x b normal ih.1 en
    | extClaim p present => rw [claim_projects c x p present ih.2]; exact ih.1
    | malformedApply => exact ih.1

theorem guarantee_reachable (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) :
    guarantee x = true :=
  Storage31.guarantee_of_inv7 c x.core (core_safety_reachable c disk x normal reached).1

theorem tracked_removals_in_trash (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) :
    (∀ p b, b ∈ x.core.g.removed p → b ∈ x.core.s.trash p) ∧
    (∀ p b, b ∈ x.core.g.delDurable p → b ∈ x.core.s.trashStable p) :=
  let trash := (core_safety_reachable c disk x normal reached).1.trash
  ⟨trash.1, trash.2.1⟩

#print axioms core_safety_reachable
#print axioms guarantee_reachable
#print axioms tracked_removals_in_trash

end Stage6
