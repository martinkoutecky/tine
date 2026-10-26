import LifetimeBackend
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem lifetime_deliverUp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Lifetime x.s) (hq : x.s.up = m::tail) (kind : RequestBytes c m) :
    Lifetime (deliverUp c x m tail ok).s := by
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  · exact lifetime_upOpen c x m tail ok h hq (by simpa using (by assumption : (m.kind == "open") = true))
  · exact lifetime_upClose c x m tail h hq (by simpa using (by assumption : (m.kind == "close") = true))
  · exact lifetime_upDiscard c x m tail ok h hq (by simpa using (by assumption : (m.kind == "discard") = true))
  · exact lifetime_upOp c x m tail ok h hq (by simpa using (by assumption : (m.kind == "op") = true))
  · apply lifetime_upSubmit c mutant x m tail h hq
    have kinds := kind.1
    simp_all

theorem lifetime_down (c : Config P) (s : Sys P) : Lifetime (down c s) := by
  constructor <;> simp [down]

theorem lifetime_windowCrash (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (h : Lifetime x.s) : Lifetime (windowCrash c x).s := by
  simp only [windowCrash,mutant,show ("none" == "MDQ") = false from rfl,Bool.false_eq_true,ite_false]
  apply lifetime_commit
  constructor
  · simpa [List.pairwise_map,Barrier,Needs] using h.ordered
  · simp only [List.pairwise_map,Current,Bool.false_eq_true,false_and,false_implies,implies_true]
    induction x.s.up with
    | nil => simp
    | cons a tail ih => simpa using ih
  · intro m hm p hn
    simp only [List.mem_map] at hm
    obtain ⟨u,hu,rfl⟩ := hm
    exact h.held u hu p hn
  all_goals simp [Current]

theorem onReply_held (c : Config P) (pg : Page) (o : String) (b : Int) :
    (onReply c pg o b).held = pg.held := by simp only [onReply]; split <;> rfl

theorem lifetime_execute (c : Config P) (mutant : c.mutant = "none") (x : State P) (a : Action P)
    (h : Lifetime x.s) (hen : enabled c x a = true) (bytes : ByteDomain c x.s)
    (launchSafe : a = .launch → Lifetime (launch c x).s) : Lifetime (execute c x a).s := by
  cases a <;> simp only [execute]
  case wOpen p => exact lifetime_wOpen c x p h hen
  case wEdit p v => exact lifetime_wEdit c x p v h (by simp [enabled] at hen; grind)
  case wSend p => exact lifetime_wSend c x p h hen
  case wResolve p v => exact lifetime_wResolve c x p v h hen
  case wDiscard p => exact lifetime_wDiscard c x p h hen
  case wOp p q t u => exact lifetime_wOp c x p q t u h hen
  case wClose p => exact lifetime_wClose c x p h hen
  case wRecv p => exact lifetime_wRecv c mutant x p h
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail => exact lifetime_deliverUp c mutant x m tail ok h hq (bytes.up m (by simp [hq]))
  case windowCrash => exact lifetime_windowCrash c mutant x h
  case launch => exact launchSafe rfl
  case switchFin => exact lifetime_commit c x _ _ _ (lifetime_down c x.s)
  case crash => exact lifetime_commit c x _ _ _ (lifetime_down c x.s)
  case power keep =>
    apply lifetime_commit
    exact lifetime_frame (down c x.s) _ (lifetime_down c x.s) (fun _ => rfl) rfl rfl rfl
  case check =>
    simp only [check]; split
    · apply lifetime_commit
      apply lifetime_frame x.s _ h
      · intro p; by_cases eq : p = x.s.job.p <;> simp_all [put,table_held]
      all_goals rfl
    · apply lifetime_commit
      exact lifetime_frame x.s _ h (fun _ => rfl) rfl rfl rfl
  case draftSync p =>
    simp only [draftSync]; split <;> (try split)
    all_goals apply lifetime_commit
    all_goals exact lifetime_frame x.s _ h (fun _ => rfl) rfl rfl rfl
  case observe p =>
    apply lifetime_commit
    apply lifetime_frame x.s _ h
    · intro q; by_cases eq : q = p <;> simp_all [put,table_held]
    all_goals rfl
  case dirSync ok =>
    apply lifetime_commit
    apply lifetime_frame x.s _ h
    · intro p; by_cases eq : p = x.s.job.p
      · subst p; simp only [put,ite_true]; split <;> (first | rfl | exact onReply_held _ _ _ _)
      · simp [put,eq]
    all_goals rfl
  case saveFail =>
    apply lifetime_commit
    apply lifetime_frame x.s _ h
    · intro p; by_cases eq : p = x.s.job.p <;> simp_all [put,onReply_held]
    all_goals rfl
  case switchReq =>
    apply lifetime_commit
    apply lifetime_frame x.s _ h
    · intro p; dsimp only; split <;> rfl
    all_goals rfl
  all_goals apply lifetime_commit
  all_goals exact lifetime_frame x.s _ h (fun _ => rfl) rfl rfl rfl

def InactiveClient (x : State P) : Prop := x.s.alive = false →
  x.s.w = (fun _ => {}) ∧ x.s.mb = (fun _ => {}) ∧ x.s.up = []

theorem inactiveClient_execute (c : Config P) (x : State P) (a : Action P)
    (h : InactiveClient x) (hen : enabled c x a = true) : InactiveClient (execute c x a) := by
  intro dead
  rw [alive_execute] at dead
  cases a <;> simp only [aliveResult] at dead
  case extWriteD p v dur => simpa [execute,extWriteD,commit,dead] using h dead
  case crash => exact ⟨rfl,rfl,rfl⟩
  case power keep => exact ⟨rfl,rfl,rfl⟩
  case switchFin => exact ⟨rfl,rfl,rfl⟩
  case launch => contradiction
  all_goals simp [enabled,dead] at hen

theorem inactiveClient_reachable (c : Config P) (x : State P) (hx : Reachable c x) : InactiveClient x := by
  induction hx with
  | initial => simp [InactiveClient,init]
  | advance hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact inactiveClient_execute c _ a ih (by assumption)
    · contradiction

theorem lifetime_launch (c : Config P) (x : State P) (h : InactiveClient x)
    (hen : enabled c x .launch = true) : Lifetime (launch c x).s := by
  have dead : x.s.alive = false := by simpa [enabled] using hen
  obtain ⟨hw,hm,hu⟩ := h dead
  apply lifetime_commit
  constructor <;> simp [hw,hm,hu]

theorem lifetime_reachable (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : Reachable c x) : Lifetime x.s := by
  induction hx with
  | initial => exact lifetime_init c
  | @advance x y hr a hs ih =>
    have bytes := bytes_reachable c x hr
    have hi := inactiveClient_reachable c x hr
    unfold step at hs; split at hs
    · cases hs
      apply lifetime_execute c mutant x a ih (by assumption) bytes
      intro he; subst a; exact lifetime_launch c x hi (by assumption)
    · contradiction

theorem headHeld_reachable (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : Reachable c x) : HeadHeld x := lifetime_headHeld x (lifetime_reachable c mutant x hx)

theorem carriage_reachable (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : Reachable c x) : Carriage x :=
  carriage_reachable_of_headHeld c mutant (headHeld_reachable c mutant) x hx

theorem accepted_reachable (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : Reachable c x) : accepted x = true := accepted_of_carriage x (carriage_reachable c mutant x hx)

theorem mailFence_reachable (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : Reachable c x) : MailFence x :=
  lifetime_mailFence x (lifetime_reachable c mutant x hx) (carriage_reachable c mutant x hx)

theorem Bprime1_reachable (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : Reachable c x) : "B'-window-replaced" ∉ x.g.bad :=
  W_reachable_of_fence c mutant (mailFence_reachable c mutant) x hx

#print axioms lifetime_reachable
#print axioms headHeld_reachable
#print axioms carriage_reachable
#print axioms mailFence_reachable
#print axioms accepted_reachable
#print axioms Bprime1_reachable
end Storage
