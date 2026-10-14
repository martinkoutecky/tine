import Versions
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def WindowAuth (x : State P) : Prop := ∀ p, (x.s.w p).on = true → Auth x.g p (x.s.w p).text
def CarrierAuth (x : State P) : Prop := ∀ m ∈ x.s.up, ∀ o ∈ carries m, Auth x.g o.1 o.2

theorem windowAuth_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hw : ∀ p, RealByte c (s.w p).text) : WindowAuth (commit c x s g n) := by
  intro p hon
  change (s.w p).on = true at hon
  apply Or.inl
  simp only [commit,mem_union_general]
  right
  simpa [hon,RealByte] using hw p

theorem windowAuth_execute (c : Config P) (x : State P) (a : Action P)
    (hb : ByteDomain c x.s) (hen : enabled c x a = true) (hx : WindowAuth x) :
    WindowAuth (execute c x a) := by
  have hbytes := bytes_execute c x a hb hen
  cases a <;> simp only [execute] at hbytes ⊢
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact hx
    | cons m tail =>
      simp only [deliverUp] at hbytes ⊢
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp] at hbytes ⊢
      all_goals (try split) <;> (try split) <;> (try split)
      all_goals apply windowAuth_commit c x
      all_goals intro p; simp only [ackMail]; (try split) <;> (try split) <;> exact hb.w p
  case check =>
    simp only [check]; split <;> exact windowAuth_commit c x _ _ _ hb.w
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> exact windowAuth_commit c x _ _ _ hb.w
  all_goals exact windowAuth_commit c x _ _ _ hbytes.w

theorem windowAuth_reachable (c : Config P) (x : State P) (hx : Reachable c x) : WindowAuth x := by
  induction hx with
  | initial => simp [WindowAuth,init]
  | advance hr a hs ih =>
    have hb := bytes_reachable c _ hr
    unfold step at hs; split at hs
    · cases hs; exact windowAuth_execute c _ a hb (by assumption) ih
    · contradiction

theorem carrier_frame (x y : State P) (h : CarrierAuth x) (hq : y.s.up = x.s.up)
    (ha : AuthMono x.g y.g) : CarrierAuth y := by
  intro m hm o ho
  exact ha _ _ (h m (by simpa [hq] using hm) o ho)

theorem carrier_execute (c : Config P) (x : State P) (a : Action P)
    (h : CarrierAuth x) (hw : WindowAuth x) (hen : enabled c x a = true)
    (hb : ByteDomain c x.s) : CarrierAuth (execute c x a) := by
  have ha := authMono_execute c x a
  have hwnew := windowAuth_execute c x a hb hen hw
  cases a <;> simp only [execute] at ha hwnew ⊢
  all_goals simp only [enabled,Bool.and_eq_true,Bool.or_eq_true,beq_iff_eq,bne_iff_ne] at hen
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [hq] at ha
      intro u hu o ho
      rw [up_deliverUp] at hu
      exact ha _ _ (h u (by simp [hq,hu]) o ho)
  case wOpen p =>
    intro m hm o ho
    change m ∈ x.s.up ++ [up1 c "open" p 0 0 noneBytes] at hm
    rcases List.mem_append.mp hm with hm | hm
    · exact ha _ _ (h m hm o ho)
    · simp only [List.mem_singleton] at hm; subst m; simp [carries,up1] at ho
  case wDiscard p =>
    intro m hm o ho
    change m ∈ x.s.up ++ [up1 c "discard" p 0 (x.s.w p).bv noneBytes] at hm
    rcases List.mem_append.mp hm with hm | hm
    · exact ha _ _ (h m hm o ho)
    · simp only [List.mem_singleton] at hm; subst m; simp [carries,up1] at ho
  case wClose p =>
    intro m hm o ho
    change m ∈ x.s.up ++ [up1 c "close" p 0 0 noneBytes] at hm
    rcases List.mem_append.mp hm with hm | hm
    · exact ha _ _ (h m hm o ho)
    · simp only [List.mem_singleton] at hm; subst m; simp [carries,up1] at ho
  case wSend p =>
    intro m hm o ho
    change m ∈ x.s.up ++ [up1 c "submit" p (x.s.w p).text (x.s.w p).bv noneBytes] at hm
    rcases List.mem_append.mp hm with hm | hm
    · exact ha _ _ (h m hm o ho)
    · simp only [List.mem_singleton] at hm; subst m
      simp only [carries,up1] at ho
      have he : o = (p,(x.s.w p).text) := by simpa using ho
      subst o
      exact ha _ _ (hw p hen.1.1.2)
  case wResolve p v =>
    intro m hm o ho
    change m ∈ x.s.up ++ [up1 c "submit" p v (x.s.w p).bv (x.s.w p).obs] at hm
    rcases List.mem_append.mp hm with hm | hm
    · exact ha _ _ (h m hm o ho)
    · simp only [List.mem_singleton] at hm; subst m
      have he : o = (p,v) := by simpa [carries,up1] using ho
      subst o
      have hh : (x.s.w p).on = true := hen.1.1.1.2
      simpa [wResolve,commit,put] using hwnew p (by simpa [wResolve,commit,put] using hh)
  case wOp p q t u =>
    intro m hm o ho
    change m ∈ x.s.up ++ [{kind := "op",p := p,q := q,t := t,t2 := u,bv := (x.s.w p).bv,bv2 := (x.s.w q).bv,ro := noneBytes}] at hm
    rcases List.mem_append.mp hm with hm | hm
    · exact ha _ _ (h m hm o ho)
    · simp only [List.mem_singleton] at hm; subst m
      have he : o = (p,t) ∨ o = (q,u) := by simpa [carries] using ho
      have hpq : p ≠ q := by rcases hen with ⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨_,hpq⟩,_⟩,_⟩,_⟩,_⟩,_⟩,_⟩,_⟩,_⟩,_⟩,_⟩; exact hpq
      have hp : (x.s.w p).on = true := by grind
      have hq : (x.s.w q).on = true := by grind
      rcases he with rfl | rfl
      · simpa [wOp,commit,put,hpq] using hwnew p (by simpa [wOp,commit,put,hpq] using hp)
      · simpa [wOp,commit,put] using hwnew q (by simpa [wOp,commit,put] using hq)
  case windowCrash =>
    intro m hm o ho
    simp only [windowCrash,commit] at hm
    split at hm
    · simp at hm
    · simp only [List.mem_map] at hm
      obtain ⟨u,hu,rfl⟩ := hm
      exact ha _ _ (h u hu o ho)
  case crash => simp [CarrierAuth,crash,commit,down]
  case power keep => simp [CarrierAuth,power,commit,down]
  case switchFin => simp [CarrierAuth,switchFin,commit,down]
  case check =>
    apply carrier_frame x _ h ?_ ha
    simp only [check]; split <;> rfl
  case draftSync p =>
    apply carrier_frame x _ h ?_ ha
    simp only [draftSync]; split <;> (try split) <;> rfl
  all_goals apply carrier_frame x _ h ?_ ha; rfl

theorem carrier_reachable (c : Config P) (x : State P) (hx : Reachable c x) : CarrierAuth x := by
  induction hx with
  | initial => simp [CarrierAuth,init]
  | @advance x y hr a hs ih =>
    have hw := windowAuth_reachable c x hr
    have hb := bytes_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact carrier_execute c x a ih hw (by assumption) hb
    · contradiction
#print axioms windowAuth_reachable
#print axioms carrier_reachable
end Storage
