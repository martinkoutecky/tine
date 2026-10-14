import TypedBackend5
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem protected_deliverUp5 (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (m : Up P) (tail : List (Up P)) (ok : Bool) (h : TypedProtected5 x)
    (hq : x.s.up = m::tail) (carrier : CarrierSeen x) (bytes : ByteDomain c x.s) :
    TypedProtected5 (deliverUp c x m tail ok) := by
  have mem : m ∈ x.s.up := by simp [hq]
  have rest := protected_tail5 x m tail hq h
  have kinds := (bytes.up m mem).1
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  · exact protected_upOpen5 c x _ m ok rest
  · exact protected_upClose5 c x _ m rest
  · exact protected_upDiscard5 c x _ m ok rest
  · have kind : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
    exact protected_upOp5 c mutant x _ m ok rest
      (carrier m mem (m.p,m.t) (by simp [carries,kind]))
      (carrier m mem (m.q,m.t2) (by simp [carries,kind]))
  · have kind : m.kind = "submit" := by simp_all
    exact protected_upSubmit5 c mutant x _ m rest
      (carrier m mem (m.p,m.t) (by simp [carries,kind]))

theorem protected_windowCrash5 (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (h : TypedProtected5 x) : TypedProtected5 (windowCrash c x) := by
  simp only [windowCrash,mutant]
  simp only [show ("none" == "MDQ") = false from rfl,Bool.false_eq_true,ite_false]
  apply protected_commit5
  intro p hh ht
  rcases h p hh ht with seen | promise | blind
  · exact Or.inl seen
  · exact Or.inr (Or.inl promise)
  · right; right
    refine ⟨rfl,?_,by simp⟩
    intro m hm hk
    obtain ⟨u,hu,rfl⟩ := List.mem_map.mp hm
    exact blind.2.1 u hu hk

theorem protected_execute5 (c : Config P) (mutant : c.mutant = "none") (x : State P) (a : Action P)
    (h : TypedProtected5 x) (carrier : CarrierSeen x) (bytes : ByteDomain c x.s)
    (inactive : InactiveClient x) (hen : enabled c x a = true) :
    TypedProtected5 (execute c x a) := by
  cases a <;> simp only [execute]
  case wOpen p => exact protected_wOpen5 c x p h hen
  case wEdit p v => exact protected_wEdit5 c x p v h hen
  case wSend p => exact protected_wSend5 c x p h hen
  case wResolve p v => exact protected_wResolve5 c x p v h hen
  case wDiscard p => exact protected_wDiscard5 c x p h hen
  case wOp p q t u => exact protected_wOp5 c x p q t u h hen
  case wClose p => exact protected_wClose5 c x p h hen
  case wRecv p => exact protected_wRecv5 c mutant x p h bytes hen
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail => exact protected_deliverUp5 c mutant x m tail ok h hq carrier bytes
  case observe p => exact protected_observe5 c mutant x p h
  case check => exact protected_check5 c mutant x h
  case saveFail => exact protected_saveFail5 c mutant x h
  case switchReq => exact protected_switchReq5 c x h
  case dirSync ok => exact protected_dirSync5 c mutant x ok h
  case draftSync p => exact protected_draftSync5 c mutant x p h
  case launch => exact protected_launch5 c x inactive hen
  case windowCrash => exact protected_windowCrash5 c mutant x h
  case crash => simp [TypedProtected5,crash,commit,down]
  case power keep => simp [TypedProtected5,power,commit,down]
  case switchFin => simp [TypedProtected5,switchFin,commit,down]
  all_goals apply protected_commit5
  all_goals exact h

end Storage21
