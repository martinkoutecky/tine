import CoreSeen21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

abbrev TypedPage4 (pg : Page) (pr : Promise) (seen : List Int) : Prop :=
  pg.held = true → pg.typed = true → pg.buf ∈ seen ∨ (pr.ver ≥ pg.ver ∧ pr.bytes = pg.buf)
def TypedSafe4 (x : State P) : Prop := ∀ p, TypedPage4 (x.s.pages p) (x.g.promise p) (x.g.seen p)

theorem typed_commit4 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : TypedSafe4 ⟨s,g⟩) : TypedSafe4 (commit c x s g n) := by
  intro p hh ht
  change (s.pages p).held = true at hh
  change (s.pages p).typed = true at ht
  have np : PostPromises x s g p = g.promise p := by simp [PostPromises,clean,ht]
  rcases h p hh ht with seen | promise
  · exact Or.inl (seen_commit c x s g n p _ seen)
  · change _ ∨ (PostPromises x s g p).ver ≥ (s.pages p).ver ∧ (PostPromises x s g p).bytes = (s.pages p).buf
    exact Or.inr (by simpa [np] using promise)

theorem typed_mono4 (s : Sys P) (g k : Ghost P) (h : TypedSafe4 ⟨s,g⟩)
    (seen : SeenMono g k) (pr : k.promise = g.promise) : TypedSafe4 ⟨s,k⟩ := by
  intro p hh ht
  rcases h p hh ht with hs | hp
  · exact Or.inl (seen p _ hs)
  · exact Or.inr (by simpa [pr] using hp)

theorem typed_put4 (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page)
    (h : TypedSafe4 ⟨s,g⟩) (hp : TypedPage4 pg (g.promise p) (g.seen p)) :
    TypedSafe4 ⟨{s with pages := put s.pages p pg},g⟩ := by
  intro q hh ht; by_cases eq : q = p
  · subst q; simpa [put] using hp (by simpa [put] using hh) (by simpa [put] using ht)
  · simpa [put,eq] using h q (by simpa [put,eq] using hh) (by simpa [put,eq] using ht)

theorem typed_update4 (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page) (pr : Promise)
    (h : TypedSafe4 ⟨s,g⟩) (hp : TypedPage4 pg pr (g.seen p)) :
    TypedSafe4 ⟨{s with pages := put s.pages p pg},{g with promise := put g.promise p pr}⟩ := by
  intro q hh ht; by_cases eq : q = p
  · subst q; simpa [put] using hp (by simpa [put] using hh) (by simpa [put] using ht)
  · simpa [put,eq] using h q (by simpa [put,eq] using hh) (by simpa [put,eq] using ht)

theorem typed_ack4 (pg : Page) (pr : Promise) (seen : List Int) (sv : Bool) (ep : Int)
    (h : TypedPage4 pg pr seen) : TypedPage4 pg (ack pr pg.buf pg.ver sv ep) seen := by
  intro hh ht
  rcases h hh ht with hs | hp
  · exact Or.inl hs
  · right; simp only [ack]; split
    · exact ⟨Int.le_refl _,rfl⟩
    · exact hp

theorem typed_ackMail4 (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool)
    (h : TypedSafe4 ⟨s,g⟩) : TypedSafe4 ⟨ackMail s p ok v t,g⟩ := by
  cases ok <;> simpa only [TypedSafe4,pages_ackMail] using h

theorem typed_table4 (c : Config P) (mutant : c.mutant = "none") (pg : Page) (pr : Promise)
    (seen : List Int) (d v : Int) (h : TypedPage4 pg pr seen) : TypedPage4 (table c pg d v) pr seen := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  · exact h
  · simp [TypedPage4]
  · exact h
  · exact h

theorem typed_upOpen4 (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : TypedSafe4 ⟨s,x.g⟩) : TypedSafe4 (upOpen c x s m ok) := by
  simp only [upOpen]; split <;> (try split)
  all_goals apply typed_commit4; apply typed_ackMail4; apply typed_put4
  all_goals first | exact h | exact h m.p | simp [TypedPage4]

theorem typed_upClose4 (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : TypedSafe4 ⟨s,x.g⟩) : TypedSafe4 (upClose c x s m) := by
  apply typed_commit4; split
  · exact typed_put4 _ _ _ _ h (by simp [TypedPage4])
  · exact h

theorem typed_upDiscard4 (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : TypedSafe4 ⟨s,x.g⟩) : TypedSafe4 (upDiscard c x s m ok) := by
  simp only [upDiscard]; split <;> (try split)
  · exact typed_commit4 c x _ _ _ h
  · exact typed_commit4 c x _ _ _ (typed_ackMail4 _ _ _ _ _ _ h)
  · apply typed_commit4; apply typed_ackMail4
    exact typed_update4 _ _ _ _ _ h (by simp [TypedPage4])

theorem typed_upSubmit4 (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (h : TypedSafe4 ⟨s,x.g⟩)
    (seen : m.t ∈ x.g.seen m.p) : TypedSafe4 (upSubmit c x s m) := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
  split
  · exact typed_commit4 c x _ _ _ h
  · apply typed_commit4; apply typed_ackMail4
    apply typed_put4 _ _ _ _ h
    split
    · intro _ _; exact Or.inl seen
    · intro _ _; exact Or.inl (by simpa [submitTo_takes c mutant] using seen)

theorem typed_upOp4 (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (h : TypedSafe4 ⟨s,x.g⟩)
    (seen : m.t ∈ x.g.seen m.p) (seen2 : m.t2 ∈ x.g.seen m.q) : TypedSafe4 (upOp c x s m ok) := by
  simp only [upOp]; split <;> (try split)
  · exact typed_commit4 c x _ _ _ h
  · exact typed_commit4 c x _ _ _ (typed_ackMail4 _ _ _ _ _ _ (typed_ackMail4 _ _ _ _ _ _ h))
  · apply typed_commit4; apply typed_ackMail4; apply typed_ackMail4
    have update := typed_update4 s x.g m.q
      {submitTo c (s.pages m.q) m.t2 m.bv2 (x.g.vc+1) with risk := true}
      (ack (x.g.promise m.q) m.t2 (x.g.vc+1) false 0) h
      (by intro _ _; exact Or.inl (by simpa [submitTo_takes c mutant] using seen2))
    exact typed_put4 _ _ m.p (submitTo c (s.pages m.p) m.t m.bv (x.g.vc+2)) update
      (by intro _ _; exact Or.inl (by simpa [submitTo_takes c mutant] using seen))

end Storage21
