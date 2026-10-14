import CoreTyped21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

@[simp] theorem put_self5 [DecidableEq α] (f : α → β) (p : α) : put f p (f p) = f := by
  funext q; by_cases h : q = p <;> simp [put,h]

def Blind5 (s : Sys P) (p : Fin P) : Prop :=
  (s.w p).on = false ∧
  (∀ m ∈ s.up, m.kind = "submit" → m.p ≠ p) ∧
  ((s.mb p).on = true → (s.mb p).text = (s.pages p).buf)

def TypedProtected5 (x : State P) : Prop := ∀ p,
  (x.s.pages p).held = true → (x.s.pages p).typed = true →
  (x.s.pages p).buf ∈ x.g.seen p ∨
  ((x.g.promise p).ver ≥ (x.s.pages p).ver ∧ (x.g.promise p).bytes = (x.s.pages p).buf) ∨
  Blind5 x.s p

theorem protected_of_safe5 (x : State P) (h : TypedSafe4 x) : TypedProtected5 x := by
  intro p hh ht
  rcases h p hh ht with hs | hp
  · exact Or.inl hs
  · exact Or.inr (Or.inl hp)

theorem protected_commit5 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : TypedProtected5 ⟨s,g⟩) : TypedProtected5 (commit c x s g n) := by
  intro p hh ht
  change (s.pages p).held = true at hh
  change (s.pages p).typed = true at ht
  have np : PostPromises x s g p = g.promise p := by simp [PostPromises,clean,ht]
  rcases h p hh ht with hs | hp | blind
  · exact Or.inl (seen_commit c x s g n p _ hs)
  · right; left
    change (PostPromises x s g p).ver ≥ (s.pages p).ver ∧ (PostPromises x s g p).bytes = (s.pages p).buf
    simpa [np] using hp
  · right; right
    refine ⟨blind.1,blind.2.1,?_⟩
    intro mail
    change (if _ then _ else s.mb p).text = (s.pages p).buf
    split
    · rfl
    · rename_i no
      exact blind.2.2 (by simpa only [commit,no,Bool.false_eq_true,ite_false] using mail)

theorem protected_frame5 (x y : State P) (h : TypedProtected5 x)
    (pg : y.s.pages = x.s.pages) (pr : y.g.promise = x.g.promise)
    (seen : SeenMono x.g y.g) (blind : ∀ p, Blind5 x.s p → Blind5 y.s p) : TypedProtected5 y := by
  intro p hh ht
  rcases h p (by simpa [pg] using hh) (by simpa [pg] using ht) with hs | hp | hb
  · left; simpa [pg] using seen p _ hs
  · right; left; simpa [pg,pr] using hp
  · exact Or.inr (Or.inr (blind p hb))

theorem protected_ack5 (pg : Page) (pr : Promise) (seen : List Int) (s : Sys P) (p : Fin P)
    (sv : Bool) (ep : Int)
    (h : pg.buf ∈ seen ∨ (pr.ver ≥ pg.ver ∧ pr.bytes = pg.buf) ∨ Blind5 s p) :
    pg.buf ∈ seen ∨ ((ack pr pg.buf pg.ver sv ep).ver ≥ pg.ver ∧
      (ack pr pg.buf pg.ver sv ep).bytes = pg.buf) ∨ Blind5 s p := by
  rcases h with hs | hp | hb
  · exact Or.inl hs
  · right; left; simp only [ack]; split
    · exact ⟨Int.le_refl _,rfl⟩
    · exact hp
  · exact Or.inr (Or.inr hb)

theorem protected_ackMail5 (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool)
    (h : TypedProtected5 ⟨s,g⟩) : TypedProtected5 ⟨ackMail s p ok v t,g⟩ := by
  cases ok
  · exact h
  · intro q hh ht
    rcases h q hh ht with hs | hp | hb
    · exact Or.inl hs
    · exact Or.inr (Or.inl hp)
    · right; right
      refine ⟨hb.1,hb.2.1,?_⟩
      by_cases eq : q = p
      · subst q; simp [ackMail,put]
      · simpa [ackMail,put,eq] using hb.2.2

theorem protected_put5 (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page)
    (h : TypedProtected5 ⟨s,g⟩) (hp : TypedPage4 pg (g.promise p) (g.seen p)) :
    TypedProtected5 ⟨{s with pages := put s.pages p pg},g⟩ := by
  intro q hh ht
  by_cases eq : q = p
  · subst q
    rcases hp (by simpa [put] using hh) (by simpa [put] using ht) with hs | hp
    · exact Or.inl (by simpa [put] using hs)
    · exact Or.inr (Or.inl (by simpa [put] using hp))
  · rcases h q (by simpa [put,eq] using hh) (by simpa [put,eq] using ht) with hs | hp | hb
    · left; simpa [put,eq] using hs
    · right; left; simpa [put,eq] using hp
    · right; right; simpa [Blind5,put,eq] using hb

theorem protected_update5 (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page) (pr : Promise)
    (h : TypedProtected5 ⟨s,g⟩) (hp : TypedPage4 pg pr (g.seen p)) :
    TypedProtected5 ⟨{s with pages := put s.pages p pg},{g with promise := put g.promise p pr}⟩ := by
  intro q hh ht
  by_cases eq : q = p
  · subst q
    rcases hp (by simpa [put] using hh) (by simpa [put] using ht) with hs | hp
    · exact Or.inl (by simpa [put] using hs)
    · exact Or.inr (Or.inl (by simpa [put] using hp))
  · rcases h q (by simpa [put,eq] using hh) (by simpa [put,eq] using ht) with hs | hp | hb
    · left; simpa [put,eq] using hs
    · right; left; simpa [put,eq] using hp
    · right; right; simpa [Blind5,put,eq] using hb

theorem protected_tail5 (x : State P) (m : Up P) (tail : List (Up P))
    (hq : x.s.up = m::tail) (h : TypedProtected5 x) :
    TypedProtected5 ⟨{x.s with up := tail},x.g⟩ := by
  apply protected_frame5 x ⟨{x.s with up := tail},x.g⟩ h rfl rfl (fun _ _ h => h)
  intro p hb
  exact ⟨hb.1,fun u hu hk => hb.2.1 u (by simp [hq,hu]) hk,hb.2.2⟩

theorem protected_mono5 (s : Sys P) (g k : Ghost P) (h : TypedProtected5 ⟨s,g⟩)
    (seen : SeenMono g k) (pr : k.promise = g.promise) : TypedProtected5 ⟨s,k⟩ :=
  protected_frame5 ⟨s,g⟩ ⟨s,k⟩ h rfl pr seen (fun _ h => h)

theorem protected_upOpen5 (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : TypedProtected5 ⟨s,x.g⟩) : TypedProtected5 (upOpen c x s m ok) := by
  simp only [upOpen]
  by_cases held : (s.pages m.p).held = true
  · simp only [held,Bool.true_or,ite_true]
    apply protected_commit5; apply protected_ackMail5
    split <;> simpa [put_self5,TypedProtected5,Blind5] using h
  · by_cases fail : ok = false
    · simp only [fail,Bool.not_false,Bool.or_true,ite_true]
      apply protected_commit5; apply protected_ackMail5
      split <;> simpa [put_self5,TypedProtected5,Blind5] using h
    · have rok : ok = true := by cases ok <;> simp_all
      simp only [rok,Bool.not_true,Bool.or_false]
      have notheld : (s.pages m.p).held = false := by cases hh : (s.pages m.p).held <;> simp_all
      simp only [notheld,Bool.false_eq_true,ite_false]
      apply protected_commit5; apply protected_ackMail5
      split <;> exact protected_put5 _ _ _ _ h (by simp [TypedPage4])

theorem protected_upClose5 (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : TypedProtected5 ⟨s,x.g⟩) : TypedProtected5 (upClose c x s m) := by
  apply protected_commit5; split
  · exact protected_put5 _ _ _ _ h (by simp [TypedPage4])
  · exact h

theorem protected_upDiscard5 (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : TypedProtected5 ⟨s,x.g⟩) : TypedProtected5 (upDiscard c x s m ok) := by
  simp only [upDiscard]; split <;> (try split)
  · exact protected_commit5 c x _ _ _ h
  · exact protected_commit5 c x _ _ _ (protected_ackMail5 _ _ _ _ _ _ h)
  · apply protected_commit5; apply protected_ackMail5
    exact protected_update5 _ _ _ _ _ h (by simp [TypedPage4])

theorem protected_upSubmit5 (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (h : TypedProtected5 ⟨s,x.g⟩)
    (seen : m.t ∈ x.g.seen m.p) : TypedProtected5 (upSubmit c x s m) := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
  split
  · exact protected_commit5 c x _ _ _ h
  · apply protected_commit5; apply protected_ackMail5
    apply protected_put5 _ _ _ _ h
    split
    · intro _ _; exact Or.inl seen
    · intro _ _; exact Or.inl (by simpa [submitTo_takes c mutant] using seen)

theorem protected_upOp5 (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (h : TypedProtected5 ⟨s,x.g⟩)
    (seen : m.t ∈ x.g.seen m.p) (seen2 : m.t2 ∈ x.g.seen m.q) : TypedProtected5 (upOp c x s m ok) := by
  simp only [upOp]; split <;> (try split)
  · exact protected_commit5 c x _ _ _ h
  · exact protected_commit5 c x _ _ _ (protected_ackMail5 _ _ _ _ _ _ (protected_ackMail5 _ _ _ _ _ _ h))
  · apply protected_commit5; apply protected_ackMail5; apply protected_ackMail5
    have update := protected_update5 s x.g m.q
      {submitTo c (s.pages m.q) m.t2 m.bv2 (x.g.vc+1) with risk := true}
      (ack (x.g.promise m.q) m.t2 (x.g.vc+1) false 0) h
      (by intro _ _; exact Or.inl (by simpa [submitTo_takes c mutant] using seen2))
    exact protected_put5 _ _ m.p (submitTo c (s.pages m.p) m.t m.bv (x.g.vc+2)) update
      (by intro _ _; exact Or.inl (by simpa [submitTo_takes c mutant] using seen))

theorem protected_pre_frame5 (x : State P) (s : Sys P) (g : Ghost P) (h : TypedProtected5 x)
    (pg : s.pages = x.s.pages) (pr : g.promise = x.g.promise) (seen : SeenMono x.g g)
    (blind : ∀ p, Blind5 x.s p → Blind5 s p) : TypedProtected5 ⟨s,g⟩ :=
  protected_frame5 x ⟨s,g⟩ h pg pr seen blind

end Storage21
