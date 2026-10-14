import CoreBytes21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
abbrev Seen (g : Ghost P) (p : Fin P) (b : Int) : Prop := b ∈ g.seen p
abbrev SeenMono (g h : Ghost P) : Prop := ∀ p b, Seen g p b → Seen h p b

theorem seen_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    SeenMono g (commit c x s g n).g := by
  intro p b h; exact (mem_union_general _ _ _).mpr (Or.inl h)
theorem seen_committed (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : SeenMono x.g g) : SeenMono x.g (commit c x s g n).g :=
  fun p b hb => seen_commit c x s g n p b (h p b hb)
theorem seenMono_execute (c : Config P) (x : State P) (a : Action P) :
    SeenMono x.g (execute c x a).g := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact fun _ _ h => h
    | cons m tail =>
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals (try split) <;> (try split) <;> (try split)
      all_goals apply seen_committed; exact fun _ _ h => h
  case wResolve p v =>
    apply seen_committed
    intro q b h; by_cases he : q = p
    · subst q; simpa [Seen,put,mem_union_general] using Or.inl h
    · simpa [Seen,put,he] using h
  case check =>
    simp only [check]; split <;> apply seen_committed <;> exact fun _ _ h => h
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> (try split)
    all_goals apply seen_committed; exact fun _ _ h => h
  all_goals apply seen_committed; exact fun _ _ h => h
def WindowSeen (x : State P) : Prop := ∀ p, (x.s.w p).on = true → Seen x.g p (x.s.w p).text
def CarrierSeen (x : State P) : Prop := ∀ m ∈ x.s.up, ∀ o ∈ carries m, Seen x.g o.1 o.2

theorem windowSeen_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hw : ∀ p, RealByte c (s.w p).text) : WindowSeen (commit c x s g n) := by
  intro p hon
  change (s.w p).on = true at hon
  simp only [Seen,commit,mem_union_general]
  right
  simpa [hon,RealByte] using hw p

theorem windowSeen_execute (c : Config P) (x : State P) (a : Action P)
    (hb : ByteDomain c x.s) (hen : enabled c x a = true) (hx : WindowSeen x) :
    WindowSeen (execute c x a) := by
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
      all_goals apply windowSeen_commit c x
      all_goals intro p; simp only [ackMail]; (try split) <;> (try split) <;> exact hb.w p
  case check =>
    simp only [check]; split <;> exact windowSeen_commit c x _ _ _ hb.w
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> exact windowSeen_commit c x _ _ _ hb.w
  all_goals exact windowSeen_commit c x _ _ _ hbytes.w

theorem windowSeen_reachable (c : Config P) (x : State P) (hx : Reachable c x) : WindowSeen x := by
  induction hx with
  | initial => simp [WindowSeen,init]
  | advance hr a hs ih =>
    have hb := bytes_reachable c _ hr
    unfold step at hs; split at hs
    · cases hs; exact windowSeen_execute c _ a hb (by assumption) ih
    · contradiction

theorem carrierSeen_frame (x y : State P) (h : CarrierSeen x) (hq : y.s.up = x.s.up)
    (ha : SeenMono x.g y.g) : CarrierSeen y := by
  intro m hm o ho
  exact ha _ _ (h m (by simpa [hq] using hm) o ho)

theorem up_deliverUp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) :
    (deliverUp c x m tail ok).s.up = tail := by
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
  all_goals (try split) <;> (try split) <;> (try split)
  all_goals simp only [commit,ackMail]
  all_goals (try split) <;> (try split) <;> rfl

theorem carrierSeen_execute (c : Config P) (x : State P) (a : Action P)
    (h : CarrierSeen x) (hw : WindowSeen x) (hen : enabled c x a = true)
    (hb : ByteDomain c x.s) : CarrierSeen (execute c x a) := by
  have ha := seenMono_execute c x a
  have hwnew := windowSeen_execute c x a hb hen hw
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
  case crash => simp [CarrierSeen,crash,commit,down]
  case power keep => simp [CarrierSeen,power,commit,down]
  case switchFin => simp [CarrierSeen,switchFin,commit,down]
  case check =>
    apply carrierSeen_frame x _ h ?_ ha
    simp only [check]; split <;> rfl
  case draftSync p =>
    apply carrierSeen_frame x _ h ?_ ha
    simp only [draftSync]; split <;> (try split) <;> rfl
  all_goals apply carrierSeen_frame x _ h ?_ ha; rfl

theorem carrierSeen_reachable (c : Config P) (x : State P) (hx : Reachable c x) : CarrierSeen x := by
  induction hx with
  | initial => simp [CarrierSeen,init]
  | @advance x y hr a hs ih =>
    have hw := windowSeen_reachable c x hr
    have hb := bytes_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact carrierSeen_execute c x a ih hw (by assumption) hb
    · contradiction
#print axioms windowSeen_reachable
#print axioms carrierSeen_reachable
end Storage21
