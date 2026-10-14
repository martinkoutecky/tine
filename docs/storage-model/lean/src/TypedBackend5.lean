import TypedWindows5
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem protected_put_preserve5 (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page)
    (h : TypedProtected5 ⟨s,g⟩)
    (shape : pg.held = true → pg.typed = true →
      (s.pages p).held = true ∧ (s.pages p).typed = true ∧ pg.buf = (s.pages p).buf ∧ pg.ver = (s.pages p).ver) :
    TypedProtected5 ⟨{s with pages := put s.pages p pg},g⟩ := by
  intro q hh ht
  by_cases eq : q = p
  · subst q
    have hs := shape (by simpa [put] using hh) (by simpa [put] using ht)
    rcases h p hs.1 hs.2.1 with seen | promise | blind
    · left; simpa [put,hs.2.2.1] using seen
    · right; left; simpa [put,hs.2.2.1,hs.2.2.2] using promise
    · right; right; simpa [Blind5,put,hs.2.2.1] using blind
  · rcases h q (by simpa [put,eq] using hh) (by simpa [put,eq] using ht) with seen | promise | blind
    · left; simpa [put,eq] using seen
    · right; left; simpa [put,eq] using promise
    · right; right; simpa [Blind5,put,eq] using blind

theorem table_protected_shape5 (c : Config P) (mutant : c.mutant = "none") (pg : Page) (d v : Int) :
    (table c pg d v).held = true → (table c pg d v).typed = true →
      pg.held = true ∧ pg.typed = true ∧ (table c pg d v).buf = pg.buf ∧ (table c pg d v).ver = pg.ver := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  all_goals simp_all

theorem protected_observe5 (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P)
    (h : TypedProtected5 x) : TypedProtected5 (observe c x p) := by
  apply protected_commit5
  apply protected_mono5
  · exact protected_put_preserve5 _ _ _ _ h (table_protected_shape5 c mutant _ _ _)
  · exact fun _ _ h => h
  · split <;> rfl

theorem protected_check5 (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (h : TypedProtected5 x) : TypedProtected5 (check c x) := by
  simp only [check]; split
  · apply protected_commit5
    apply protected_mono5
    · exact protected_put_preserve5 _ _ _ _ h (table_protected_shape5 c mutant _ _ _)
    · exact fun _ _ h => h
    · split <;> rfl
  · exact protected_commit5 c x _ _ _ h

theorem protected_saveFail5 (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (h : TypedProtected5 x) : TypedProtected5 (saveFail c x) := by
  apply protected_commit5
  apply protected_put_preserve5 _ _ _ _ h
  simp [onReply,mutant]; grind

theorem protected_switchReq5 (c : Config P) (x : State P) (h : TypedProtected5 x) :
    TypedProtected5 (switchReq c x) := by
  apply protected_commit5
  intro p hh ht
  have held : (x.s.pages p).held = true := by dsimp only at hh; split at hh <;> exact hh
  have typed : (x.s.pages p).typed = true := by dsimp only at ht; split at ht <;> exact ht
  rcases h p held typed with seen | promise | blind
  · left; dsimp only; split <;> exact seen
  · right; left; dsimp only; split <;> exact promise
  · right; right
    have buf : (if (x.s.pages p).held && dirty (x.s.pages p) then {x.s.pages p with risk := true} else x.s.pages p).buf = (x.s.pages p).buf := by split <;> rfl
    simpa only [Blind5,buf] using blind

theorem protected_dirSync5 (c : Config P) (mutant : c.mutant = "none") (x : State P) (ok : Bool)
    (h : TypedProtected5 x) : TypedProtected5 (dirSync c x ok) := by
  simp only [dirSync,mutant]
  simp only [show ("none" == "MRE") = false from rfl,Bool.and_false,Bool.false_eq_true,ite_false]
  cases ok
  · apply protected_commit5
    exact protected_put_preserve5 _ _ _ _ h (by simp [onReply,mutant]; grind)
  · apply protected_commit5
    exact protected_update5 _ _ _ _ _ h (by simp [TypedPage4,onReply])

theorem protected_draftSync5 (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P)
    (h : TypedProtected5 x) : TypedProtected5 (draftSync c x p) := by
  simp only [draftSync,mutant]
  simp only [show ("none" == "MBD") = false from rfl,Bool.false_and,Bool.false_eq_true,ite_false]
  apply protected_commit5
  split
  · intro q hh ht
    change (x.s.pages q).held = true at hh
    change (x.s.pages q).typed = true at ht
    rcases h q hh ht with seen | promise | blind
    · exact Or.inl seen
    · right; left
      by_cases eq : q = p
      · subst q
        simp only [put,ite_true]
        by_cases risk : (x.s.pages p).risk = true
        · simp only [draftEntry,hh,risk,Bool.true_and,ite_true]
          simp only [ack]; split
          · exact ⟨Int.le_refl _,rfl⟩
          · exact promise
        · simp [draftEntry,risk] at *
      · simpa [put,eq] using promise
    · right; right; exact blind
  · exact h

theorem protected_launch5 (c : Config P) (x : State P)
    (hi : InactiveClient x) (hen : enabled c x .launch = true) : TypedProtected5 (launch c x) := by
  have dead : x.s.alive = false := by simpa [enabled] using hen
  obtain ⟨hw,hm,hu⟩ := hi dead
  apply protected_commit5
  intro p _ _
  right; right
  simp [Blind5,hw,hm,hu]

end Storage21
