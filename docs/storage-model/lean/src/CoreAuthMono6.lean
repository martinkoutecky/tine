import CoreG6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

theorem authMono_trans (g h k : Ghost P) (ha : AuthMono g h) (hb : AuthMono h k) :
    AuthMono g k := fun p b h => hb p b (ha p b h)

theorem authMono_committed (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : AuthMono x.g g) : AuthMono x.g (commit c x s g n).g :=
  authMono_trans _ _ _ h (Auth_mono_commit c x s g n)

theorem authMono_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    AuthMono x.g (upOpen c x s m ok).g := by
  apply authMono_committed
  split <;> (try split)
  all_goals exact authMono_refl x.g
theorem authMono_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    AuthMono x.g (upClose c x s m).g := by
  exact authMono_committed c x _ _ _ (authMono_refl x.g)
theorem authMono_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    AuthMono x.g (upDiscard c x s m ok).g := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals exact authMono_committed c x _ _ _ (authMono_refl x.g)
theorem authMono_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    AuthMono x.g (upSubmit c x s m).g := by
  simp only [upSubmit]
  split
  all_goals exact authMono_committed c x _ _ _ (authMono_refl x.g)
theorem authMono_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    AuthMono x.g (upOp c x s m ok).g := by
  simp only [upOp]
  split
  · exact authMono_committed c x _ _ _ (authMono_refl x.g)
  · split
    · exact authMono_committed c x _ _ _ (authMono_refl x.g)
    · exact authMono_committed c x _ _ _ (authMono_wrote x.g _ _)

/-- No crash, restore, power, queue, or mutant branch erases byte authorization. -/
theorem authMono_execute (c : Config P) (x : State P) (a : Action P) :
    AuthMono x.g (execute c x a).g := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact authMono_refl x.g
    | cons m tail =>
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact authMono_upOpen c x _ m ok
        | exact authMono_upClose c x _ m
        | exact authMono_upDiscard c x _ m ok
        | exact authMono_upOp c x _ m ok
        | exact authMono_upSubmit c x _ m
  case wResolve p v => exact authMono_committed c x _ _ _ (authMono_seen x.g _ _)
  case rename => exact authMono_committed c x _ _ _ (authMono_wrote x.g _ _)
  case draftSync p =>
    apply authMono_committed
    split <;> (try split)
    all_goals first | exact authMono_wrote x.g _ _ | exact authMono_refl x.g
  case check =>
    simp only [check]
    split <;> exact authMono_committed c x _ _ _ (authMono_refl x.g)
  all_goals exact authMono_committed c x _ _ _ (authMono_refl x.g)

theorem authMono_step (c : Config P) (x y : State P) (a : Action P)
    (hs : step c x a = some y) : AuthMono x.g y.g := by
  unfold step at hs
  split at hs
  · cases hs; exact authMono_execute c x a
  · contradiction


end Storage21
