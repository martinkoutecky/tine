import LifetimeWindow
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem barrier_touch (m : Up P) (p : Fin P) (h : Barrier m p) : Touch m p := Or.inl h.2

theorem lifetime_pop (s t : Sys P) (h : Lifetime s) (m : Up P) (tail : List (Up P))
    (hq : s.up = m::tail) (up : t.up = tail) (w : t.w = s.w) (mb : t.mb = s.mb)
    (pages : ∀ p, ¬Barrier m p → (t.pages p).held = (s.pages p).held) : Lifetime t := by
  have ordered := h.ordered
  have last := h.last
  rw [hq,List.pairwise_cons] at ordered last
  have member : ∀ a ∈ tail, a ∈ s.up := by intro a ha; simp [hq,ha]
  have head : m ∈ s.up := by simp [hq]
  constructor
  · simpa [up] using ordered.2
  · simpa [up] using last.2
  · intro a ha p na
    have ha' : a ∈ tail := by simpa [up] using ha
    have nb : ¬Barrier m p := by intro ba; exact ordered.1 a ha' p ba na
    rw [pages p nb]
    exact h.held a (member a ha') p na
  · intro p hon
    have old := h.active p (by simpa [w] using hon)
    have nb : ¬Barrier m p := by intro ba; exact (old.2 m head (barrier_touch m p ba)).2 ba
    refine ⟨by simpa [pages p nb] using old.1,?_⟩
    intro a ha ta; exact old.2 a (member a (by simpa [up] using ha)) ta
  · simpa [w] using h.pending
  · intro a ha p ca
    simpa [w,mb] using h.waiting a (member a (by simpa [up] using ha)) p ca
  · intro p pos
    have old := h.done p (by simpa [mb] using pos)
    refine ⟨by simpa [w] using old.1,?_⟩
    intro a ha; exact old.2 a (member a (by simpa [up] using ha))
  · simpa [mb] using h.mailOn
  · intro p pos
    have old := h.done p (by simpa [mb] using pos)
    have nb : ¬Barrier m p := by intro ba; exact old.2 m head (barrier_touch m p ba)
    simpa [mb,pages p nb] using h.mailHeld p (by simpa [mb] using pos)

theorem lifetime_ack_head (old s : Sys P) (ho : Lifetime old) (h : Lifetime s)
    (m : Up P) (tail : List (Up P)) (hq : old.up = m::tail) (up : s.up = tail)
    (w : s.w = old.w) (p : Fin P) (touch : Touch m p) (nc : m.kind ≠ "close")
    (v : Int) (took : Bool) : Lifetime (ackMail s p m.cur v took) := by
  cases cur : m.cur
  · simpa [ackMail,cur] using h
  · have current : Current m p := ⟨cur,nc,touch⟩
    have sent := (ho.waiting m (by simp [hq]) p current).1
    have last := ho.last
    rw [hq,List.pairwise_cons] at last
    have no : ∀ a ∈ s.up, ¬Touch a p := by intro a ha; exact last.1 a (by simpa [up] using ha) p current
    have raw := lifetime_patch s h p (s.w p)
      ⟨true,(s.pages p).held,(s.pages p).ver,(s.pages p).buf,(s.pages p).obs,(s.pages p).conflict,v,took⟩
      (h.active p) (h.pending p)
      (fun a ha ca => False.elim (no a ha ca.2.2))
      (fun _ => ⟨by simpa [w] using sent,no⟩) (fun _ => ⟨rfl,rfl⟩)
    have self : put s.w p (s.w p) = s.w := by funext q; by_cases eq : q = p <;> simp_all [put]
    simpa [ackMail,cur,self] using raw

theorem submitTo_held (c : Config P) (pg : Page) (t bv v : Int) :
    (submitTo c pg t bv v).held = pg.held := by
  simp only [submitTo]; split <;> (try split) <;> rfl

theorem lifetime_upOpen (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Lifetime x.s) (hq : x.s.up = m::tail) (km : m.kind = "open") :
    Lifetime (upOpen c x {x.s with up := tail} m ok).s := by
  simp only [upOpen]
  apply lifetime_commit
  refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.p (Or.inl rfl) (by simp [km]) _ _
  all_goals try simp only [up_ackMail,w_ackMail]
  all_goals try rfl
  refine lifetime_pop x.s _ h m tail hq ?_ ?_ ?_ ?_
  all_goals try rfl
  intro p nb
  by_cases eq : p = m.p
  · subst p; exact False.elim (nb (by simp [Barrier,km]))
  · simp [put,eq]

theorem lifetime_upClose (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (h : Lifetime x.s) (hq : x.s.up = m::tail) (km : m.kind = "close") :
    Lifetime (upClose c x {x.s with up := tail} m).s := by
  simp only [upClose]; split
  · apply lifetime_commit
    refine lifetime_pop x.s _ h m tail hq ?_ ?_ ?_ ?_
    all_goals try rfl
    intro p nb
    by_cases eq : p = m.p
    · subst p; exact False.elim (nb (by simp [Barrier,km]))
    · simp [put,eq]
  · apply lifetime_commit
    exact lifetime_pop x.s _ h m tail hq rfl rfl rfl (fun _ _ => rfl)

theorem lifetime_upDiscard (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Lifetime x.s) (hq : x.s.up = m::tail) (km : m.kind = "discard") :
    Lifetime (upDiscard c x {x.s with up := tail} m ok).s := by
  simp only [upDiscard]; split
  · apply lifetime_commit
    exact lifetime_pop x.s _ h m tail hq rfl rfl rfl (fun _ _ => rfl)
  · split
    · apply lifetime_commit
      refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.p (Or.inl rfl) (by simp [km]) _ _
      all_goals try simp only [up_ackMail,w_ackMail]
      all_goals try rfl
      exact lifetime_pop x.s _ h m tail hq rfl rfl rfl (fun _ _ => rfl)
    · apply lifetime_commit
      refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.p (Or.inl rfl) (by simp [km]) _ _
      all_goals try simp only [up_ackMail,w_ackMail]
      all_goals try rfl
      refine lifetime_pop x.s _ h m tail hq ?_ ?_ ?_ ?_
      all_goals try rfl
      intro p _; by_cases eq : p = m.p <;> simp_all [put]

theorem lifetime_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P))
    (h : Lifetime x.s) (hq : x.s.up = m::tail) (km : m.kind = "submit") :
    Lifetime (upSubmit c x {x.s with up := tail} m).s := by
  simp only [upSubmit,mutant,show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]; split
  · apply lifetime_commit
    exact lifetime_pop x.s _ h m tail hq rfl rfl rfl (fun _ _ => rfl)
  · apply lifetime_commit
    refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.p (Or.inl rfl) (by simp [km]) _ _
    all_goals try simp only [up_ackMail,w_ackMail]
    all_goals try rfl
    refine lifetime_pop x.s _ h m tail hq ?_ ?_ ?_ ?_
    all_goals try rfl
    intro p _; by_cases eq : p = m.p
    · subst p; simp only [put,ite_true]; split <;> (first | rfl | exact submitTo_held _ _ _ _ _)
    · simp [put,eq]

theorem lifetime_upOp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Lifetime x.s) (hq : x.s.up = m::tail) (km : m.kind = "op") :
    Lifetime (upOp c x {x.s with up := tail} m ok).s := by
  simp only [upOp]; split
  · apply lifetime_commit
    exact lifetime_pop x.s _ h m tail hq rfl rfl rfl (fun _ _ => rfl)
  · split
    · apply lifetime_commit
      refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.p (Or.inl rfl) (by simp [km]) _ _
      all_goals try simp only [up_ackMail,w_ackMail]
      all_goals try rfl
      refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.q (Or.inr ⟨km,rfl⟩) (by simp [km]) _ _
      all_goals try simp only [up_ackMail,w_ackMail]
      all_goals try rfl
      exact lifetime_pop x.s _ h m tail hq rfl rfl rfl (fun _ _ => rfl)
    · apply lifetime_commit
      refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.p (Or.inl rfl) (by simp [km]) _ _
      all_goals try simp only [up_ackMail,w_ackMail]
      all_goals try rfl
      refine lifetime_ack_head x.s _ h ?_ m tail hq ?_ ?_ m.q (Or.inr ⟨km,rfl⟩) (by simp [km]) _ _
      all_goals try simp only [up_ackMail,w_ackMail]
      all_goals try rfl
      refine lifetime_pop x.s _ h m tail hq ?_ ?_ ?_ ?_
      all_goals try rfl
      intro p _
      by_cases ep : p = m.p
      · subst p; simp [put,submitTo_held]
      · by_cases eq : p = m.q <;> simp_all [put,submitTo_held]

end Storage
