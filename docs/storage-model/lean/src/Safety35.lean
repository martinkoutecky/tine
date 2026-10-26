import Conservativity35
import Order9
import Trash9
namespace Storage35Base
open Storage21 Storage32 Storage3
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem bounds_discard32_9guard10 (c : Config P) (x : State P) (sys : Sys P)
    (m : Up P) (ok : Bool) (h : VersionBounds sys.toBase.toSys x.g.vc) :
    VersionBounds (upDiscard c x sys m ok).s.toBase.toSys (upDiscard c x sys m ok).g.vc := by
  simp only [upDiscard]
  split
  · apply bounds_commit
    exact h
  · split
    · apply bounds_commit
      exact bounds_ackMail _ _ _ _ _ _ h (h.pages m.p)
    · apply bounds_commit
      apply bounds_ackMail
      · split
        · have hg := bounds_mono sys.toBase.toSys x.g.vc (x.g.vc+2) h (by omega)
          refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
          apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+2)
          · apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+2) hg.pages
            dsimp only; omega
          · dsimp only; omega
        · have hg := bounds_mono sys.toBase.toSys x.g.vc (x.g.vc+1) h (by omega)
          refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
          apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages
          exact Int.le_refl _
      · split <;> dsimp only <;> omega
theorem safe_orders_discard_action9guard10 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
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


theorem idle_closed_discard32_9guard10 (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (ok : Bool) :
    IdleClosed c (Storage3.project (project (upDiscard c x sys m ok))) := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals apply idle_closed_commit9

theorem vocabulary_discard9guard10 (c : Config P) (x : State P) (sys : Sys P)
    (m : Up P) (rok : Bool) (h : BadVocabulary x.g.toBase.toGhost) :
    BadVocabulary (upDiscard c x sys m rok).g.toBase.toGhost := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals apply vocabulary_commit
  · apply vocabulary_add _ _ h
    intro t ht; simp only [List.mem_singleton] at ht; subst t; simp [BadKinds]
  · exact h
  · exact h

theorem trash_discard32_9guard10 (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (ok : Bool)
    (h : TrashInv ⟨sys.toBase,x.g.toBase⟩) : TrashInv (project (upDiscard c x sys m ok)) := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals exact h

theorem execute_cases10 (c : Config P) (x : State P) (a : Action P) :
    execute c x a = Storage32.execute c x a ∨
    ∃ ok m tail, x.s.up = m::tail ∧ m.kind = "discard" ∧
      execute c x a = upDiscard c x {x.s with up := tail} m ok := by
  cases a <;> simp only [execute]
  case old a =>
    cases a <;> try exact Or.inl rfl
    case deliverUp ok =>
      cases hq : x.s.up with
      | nil => exact Or.inl (by simp [deliverUp,Storage32.execute,Storage32.deliverUp,hq])
      | cons m tail =>
        by_cases kind : m.kind = "discard"
        · exact Or.inr ⟨ok,m,tail,rfl,kind,by simp [deliverUp,hq,kind]⟩
        · exact Or.inl (by simp [deliverUp,Storage32.execute,Storage32.deliverUp,hq,kind])
  all_goals first | exact Or.inl rfl | exact Or.inl trivial

theorem preserve10 (c : Config P) (x : State P) (a : Action P) (Q : State P → Prop)
    (old : Q (Storage32.execute c x a))
    (discard : ∀ ok m tail,x.s.up = m::tail → m.kind = "discard" →
      Q (upDiscard c x {x.s with up := tail} m ok)) : Q (execute c x a) := by
  rcases execute_cases10 c x a with eq | ⟨ok,m,tail,hq,hk,eq⟩
  · rw [eq]; exact old
  · rw [eq]; exact discard ok m tail hq hk
end Storage35Base
namespace Storage35
open Storage21 Storage3
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
structure Aux (c : Config P) (x : State P) : Prop where
  bounds : VersionBounds x.s.to32.toBase.toSys x.g.vc
  orders : Storage32.SafeOrders (project x)
  idle : Storage34.IdleClosed c x
  vocabulary : BadVocabulary x.g.toBase.toGhost
  trash : TrashInv (Storage32.project (project x))

theorem aux_execute10 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : Aux c x) (en : enabled c x a = true) :
    Aux c (execute c x a) := by
  cases a with
  | prior a =>
    have raw : Storage32.enabled c (project x) a = true := en
    constructor
    · apply Storage35Base.preserve10 c (project x) a
        (fun y => VersionBounds y.s.toBase.toSys y.g.vc)
      · exact Storage32.bounds_execute32_9 c (project x) a mutant h.bounds
      · intro ok m tail hq _
        apply Storage35Base.bounds_discard32_9guard10
        exact ⟨h.bounds.nonnegative,h.bounds.pages,h.bounds.w,h.bounds.mb,
          by intro u hu; exact h.bounds.up u (by
            change u ∈ (project x).s.up
            rw [hq]; exact List.mem_cons_of_mem m hu)⟩
    · apply Storage35Base.preserve10 c (project x) a (Storage32.SafeOrders)
      · exact Storage32.safe_orders_execute32_9 c (project x) a mutant h.orders h.bounds.nonnegative raw
      · intro ok m tail _ _; exact Storage35Base.safe_orders_discard_action9guard10 c (project x) m tail ok h.orders
    · apply Storage35Base.preserve10 c (project x) a
        (fun y => Storage21.IdleClosed c (Storage3.project (Storage32.project y)))
      · exact Storage32.idle_closed_execute32_9 c (project x) a mutant h.idle
      · intro ok m tail _ _; exact Storage35Base.idle_closed_discard32_9guard10 c (project x) _ m ok
    · apply Storage35Base.preserve10 c (project x) a (fun y => BadVocabulary y.g.toBase.toGhost)
      · exact Storage32.vocabulary_execute32_9 c (project x) a mutant h.vocabulary
      · intro ok m tail _ _; exact Storage35Base.vocabulary_discard9guard10 c (project x) _ m ok h.vocabulary
    · apply Storage35Base.preserve10 c (project x) a (fun y => TrashInv (Storage32.project y))
      · exact Storage32.trash_execute32_9 c (project x) a mutant h.trash
      · intro ok m tail _ _; exact Storage35Base.trash_discard32_9guard10 c (project x) _ m ok h.trash
  | opApply eff plan dst src =>
    have baseEn : Storage33.enabledApply c (project x) eff plan dst src = true := by
      simp only [enabled,Storage34.enabled,Storage34.enabledApply,Bool.and_eq_true] at en; exact en.1
    have free := Storage34.apply_slot_free9 c x eff plan dst src mutant en
    have eq := Storage34.apply_refines33_9 c x eff plan dst src free
    constructor
    · change VersionBounds (project (Storage34.opApply c x eff plan dst src)).s.toBase.toSys
        (project (Storage34.opApply c x eff plan dst src)).g.vc
      simp only [project]; rw [eq]; exact Storage33.bounds_apply33_9 c (project x) eff plan dst src h.bounds
    · change Storage32.SafeOrders (project (Storage34.opApply c x eff plan dst src))
      simp only [project]; rw [eq]; exact Storage33.safe_orders_apply9 c (project x) eff plan dst src mutant h.orders h.bounds.nonnegative baseEn
    · exact (Storage34.idle_closed_execute34_9 c x (.opApply eff plan dst src) mutant h.idle)
    · exact (Storage34.vocabulary_step9 c x (.opApply eff plan dst src) mutant h.vocabulary en)
    · exact h.trash
  | extClaim p present =>
    have eq := Storage34.extClaim_projects_idle9 c x p present h.idle
    constructor
    · change VersionBounds (project (Storage34.extClaim c x p present)).s.toBase.toSys
        (project (Storage34.extClaim c x p present)).g.vc
      simp only [project]; rw [eq]; exact h.bounds
    · change Storage32.SafeOrders (project (Storage34.extClaim c x p present))
      simp only [project]; rw [eq]; exact h.orders
    · exact Storage34.idle_closed_execute34_9 c x (.extClaim p present) mutant h.idle
    · change BadVocabulary (project (Storage34.extClaim c x p present)).g.toBase.toGhost
      simp only [project]; rw [eq]; exact h.vocabulary
    · change TrashInv (Storage32.project (project (Storage34.extClaim c x p present)))
      simp only [project]; rw [eq]; exact h.trash

theorem aux_reachable10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : Aux c x := by
  induction hr with
  | initial =>
    constructor
    · exact Storage32.bounds_reachable32_9 c disk _ mutant .initial
    · exact Storage32.safe_orders_reachable32_9 c disk _ mutant .initial
    · exact Storage34.idle_closed_reachable34_9 c disk _ mutant .initial
    · exact Storage34.vocabulary_reachable9 c disk _ mutant .initial
    · exact Storage34.trash_reachable34_9 c disk _ mutant .initial
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact aux_execute10 c x a mutant ih (by assumption)
    · contradiction

theorem orderHolds_reachable35_10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : orderHolds x = true :=
  (aux_reachable10 c disk x mutant hr).orders.1.2.2.2
theorem name_slot_history35_10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    "name-slot-create-over-claim" ∉ x.g.bad := by
  intro mem
  have kind := (aux_reachable10 c disk x mutant hr).vocabulary _ mem
  simp [BadKinds] at kind
theorem trashed_reachable35_10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    Storage3.trashed (Storage32.project (project x)) = true := by
  have h := (aux_reachable10 c disk x mutant hr).trash
  apply List.all_eq_true.mpr
  intro p _
  apply List.all_eq_true.mpr
  intro b hb
  simp only [Bool.or_eq_true,List.contains_iff_mem]
  exact Or.inl (h.1 p b hb)
end Storage35
