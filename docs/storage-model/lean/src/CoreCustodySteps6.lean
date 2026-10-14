import CoreCustodyPower6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem custody_kept_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : Custody c x) (promise : g.promise = x.g.promise) (ext : g.ext = x.g.ext)
    (wrote : g.wrote = x.g.wrote) (disk : s.disk = x.s.disk) (stable : s.stable = x.s.stable)
    (drafts : s.drafts = x.s.drafts)
    (risk : ∀ p, (x.s.alive = true → (x.s.pages p).held = true ∧ (x.s.pages p).risk = true) →
      s.alive = true → (s.pages p).held = true ∧ (s.pages p).risk = true)
    (same : ∀ p, PostPromises x s g p = g.promise p) : Custody c (commit c x s g n) :=
  custody_commit c x s g n (custody_frame c x ⟨s,g⟩ h promise ext wrote disk stable drafts risk) same

theorem postPromises_noCleaning (x : State P) (s : Sys P) (g : Ghost P)
    (h : ∀ p, (x.s.pages p).held = true → dirty (x.s.pages p) = true → dirty (s.pages p) = true) :
    ∀ p, PostPromises x s g p = g.promise p := by
  intro p; simp only [PostPromises]; split
  · have all : (x.s.pages p).held = true ∧ dirty (x.s.pages p) = true ∧ clean (s.pages p) = true := by
      simp_all only [Bool.and_eq_true]; grind
    have nc := h p all.1 all.2.1
    simp only [dirty,all.2.2,Bool.not_true,Bool.false_eq_true] at nc
  · rfl

theorem table_dirty_preserved (c : Config P) (mutant : c.mutant = "none") (pg : Page) (d v : Int)
    (shape : PageShape pg) (hd : dirty pg = true) : dirty (table c pg d v) = true := by
  have flags : pg.typed = true ∨ pg.risk = true := by
    by_cases ht : pg.typed = true
    · exact Or.inl ht
    · by_cases hr : pg.risk = true
      · exact Or.inr hr
      · have eq := shape (by simpa using ht) (by simpa using hr)
        simp_all [dirty,clean]
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  all_goals simp_all [dirty,clean]
  all_goals grind

theorem table_risk_preserved (c : Config P) (mutant : c.mutant = "none") (pg : Page) (d v : Int)
    (risk : pg.risk = true) : (table c pg d v).risk = true := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  all_goals simp_all [clean]

theorem custody_observe (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P)
    (h : Custody c x) (shape : Shapes x.s) : Custody c (observe c x p) := by
  apply custody_kept_commit c x _ _ _ h
  all_goals try rfl
  · intro q hp alive
    have old := hp alive
    by_cases eq : q = p
    · subst q
      exact ⟨by simpa [put,table_held] using old.1,by simpa [put] using table_risk_preserved c mutant _ _ _ old.2⟩
    · simpa [put,eq] using old
  · apply postPromises_noCleaning
    intro q hh hd
    by_cases eq : q = p
    · subst q; simpa [put] using table_dirty_preserved c mutant _ _ _ (shape _) hd
    · simpa [put,eq] using hd

theorem custody_check (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (h : Custody c x) (shape : Shapes x.s) : Custody c (check c x) := by
  simp only [check]; split
  · apply custody_kept_commit c x _ _ _ h
    all_goals try rfl
    · intro p hp alive
      have old := hp alive
      by_cases eq : p = x.s.job.p
      · subst p
        exact ⟨by simpa [put,table_held] using old.1,by simpa [put] using table_risk_preserved c mutant _ _ _ old.2⟩
      · simpa [put,eq] using old
    · apply postPromises_noCleaning
      intro p hh hd
      by_cases eq : p = x.s.job.p
      · subst p; simpa [put] using table_dirty_preserved c mutant _ _ _ (shape _) hd
      · simpa [put,eq] using hd
  · apply custody_kept_commit c x _ _ _ h
    all_goals first | rfl | (intro p hp; exact hp) | exact postPromises_same_pages x _ _ rfl

theorem custody_saveFail (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (h : Custody c x) : Custody c (saveFail c x) := by
  apply custody_kept_commit c x _ _ _ h
  all_goals try rfl
  · intro p hp alive
    have old := hp alive
    by_cases eq : p = x.s.job.p <;> simp_all [put,onReply,mutant]
  · apply postPromises_noCleaning
    intro p hh hd
    by_cases eq : p = x.s.job.p <;> simp_all [put,onReply,mutant,dirty,clean]

theorem custody_switchReq (c : Config P) (x : State P) (h : Custody c x) : Custody c (switchReq c x) := by
  apply custody_kept_commit c x _ _ _ h
  all_goals try rfl
  · intro p hp alive
    have old := hp alive
    dsimp only; split
    · exact ⟨old.1,rfl⟩
    · exact old
  · apply postPromises_noCleaning
    intro p hh hd
    dsimp only
    simp only [hh,hd,Bool.true_and,ite_true]
    simp [dirty,clean]

theorem custody_extWriteD (c : Config P) (x : State P) (p : Fin P) (v : Int) (dur : Bool)
    (h : Custody c x) (ep : EpochBounds x.s x.g) : Custody c (extWriteD c x p v dur) := by
  apply custody_commit
  · intro q need
    by_cases eq : q = p
    · subst q
      have live := need.1
      have nexp := live.2
      have bound := ep.promise p
      have unsaved : (x.g.promise p).saved = false := by
        cases sv : (x.g.promise p).saved
        · rfl
        · have expire : (x.g.promise p).ep < x.g.ext p + 1 := by omega
          exact False.elim (nexp ⟨sv,by simpa [put] using expire⟩)
      have oldlive : LivePromise x p := ⟨live.1,by simp [unsaved]⟩
      exact h p ⟨oldlive,Or.inl unsaved⟩
    · have oldneed : NeedsDraft c x q := by
        cases dur <;> simpa [NeedsDraft,LivePromise,Covers,put,eq] using need
      exact h q oldneed
  · exact postPromises_same_pages x _ _ rfl

end Storage21
