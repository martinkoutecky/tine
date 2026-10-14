import CoreSeen21
import CoreAuthMono6
import CoreObs6
import CoreG6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

/-- Resolution bases are authorized at enqueue and authorization is monotone. -/
def ResolutionAuth (x : State P) : Prop :=
  ∀ m ∈ x.s.up, m.ro ≠ noneBytes → Auth x.g m.p m.ro

theorem resolutionAuth_init (c : Config P) : ResolutionAuth (init c) := by
  simp [ResolutionAuth,init]

theorem resolutionAuth_commit (c : Config P) (x : State P) (s : Sys P)
    (g : Ghost P) (n : String)
    (h : ∀ m ∈ s.up, m.ro ≠ noneBytes → Auth g m.p m.ro) :
    ResolutionAuth (commit c x s g n) := by
  intro m hm hr
  exact Auth_mono_commit c x s g n _ _ (h m hm hr)

theorem resolutionAuth_frame (c : Config P) (x y : State P)
    (h : ResolutionAuth x) (hq : y.s.up = x.s.up)
    (ha : AuthMono x.g y.g) : ResolutionAuth y := by
  intro m hm hr
  exact ha _ _ (h m (by simpa [hq] using hm) hr)

theorem resolutionAuth_tail (x : State P) (m : Up P) (tail : List (Up P))
    (h : ResolutionAuth x) (hq : x.s.up = m :: tail) :
    ∀ u ∈ tail, u.ro ≠ noneBytes → Auth x.g u.p u.ro := by
  intro u hu hr
  exact h u (by simp [hq,hu]) hr

theorem resolutionAuth_execute (c : Config P) (x : State P) (a : Action P)
    (hx : ResolutionAuth x) (ho : ObsOnly c x.s) : ResolutionAuth (execute c x a) := by
  have ha := authMono_execute c x a
  cases a <;> simp only [execute] at ha ⊢
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact hx
    | cons m tail =>
      simp only [hq] at ha
      intro u hu hr
      rw [up_deliverUp] at hu
      exact ha _ _ (resolutionAuth_tail x m tail hx hq u hu hr)
  case wResolve p v =>
    apply resolutionAuth_commit
    intro m hm hr
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact authMono_seen x.g _ _ _ _ (hx m hm hr)
    · change Auth {x.g with seen := put x.g.seen p (union (x.g.seen p)
        ([(x.s.w p).obs].filter fun t => t == absent || c.texts.contains t))} p (x.s.w p).obs
      apply Or.inl
      simp only [put,ite_true,mem_union_general]
      right
      have hv : (x.s.w p).obs = absent ∨ (x.s.w p).obs ∈ c.texts := by
        rcases ho.2.2.2.1 p with hn | hv
        · exact False.elim (hr hn)
        · exact hv
      simpa using hv
  case wOpen p =>
    apply resolutionAuth_commit
    intro m hm hr
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact hx m hm hr
    · exact False.elim (hr rfl)
  case wSend p =>
    apply resolutionAuth_commit
    intro m hm hr
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact hx m hm hr
    · exact False.elim (hr rfl)
  case wDiscard p =>
    apply resolutionAuth_commit
    intro m hm hr
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact hx m hm hr
    · exact False.elim (hr rfl)
  case wOp p q t u =>
    apply resolutionAuth_commit
    intro m hm hr
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact hx m hm hr
    · exact False.elim (hr rfl)
  case wClose p =>
    apply resolutionAuth_commit
    intro m hm hr
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact hx m hm hr
    · exact False.elim (hr rfl)
  case windowCrash =>
    intro m hm hr
    simp only [windowCrash,commit] at hm
    split at hm
    · simp at hm
    · simp only [List.mem_map] at hm
      rcases hm with ⟨u,hu,rfl⟩
      exact ha _ _ (hx u hu hr)
  case crash => simp [ResolutionAuth,crash,commit,down]
  case power keep => simp [ResolutionAuth,power,commit,down]
  case switchFin => simp [ResolutionAuth,switchFin,commit,down]
  case check =>
    apply resolutionAuth_frame c x _ hx _ ha
    simp only [check]; split <;> rfl
  case draftSync p =>
    apply resolutionAuth_frame c x _ hx _ ha
    simp only [draftSync]; split <;> (try split) <;> rfl
  all_goals apply resolutionAuth_frame c x _ hx ?_ ha; rfl
theorem resolutionAuth_reachable (c : Config P) (x : State P) (hx : Reachable c x) : ResolutionAuth x := by
  induction hx with
  | initial => exact resolutionAuth_init c
  | @advance x y hr a hs ih =>
    have ho := obsOnly_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact resolutionAuth_execute c x a ih ho
    · contradiction
#print axioms resolutionAuth_reachable
end Storage21
