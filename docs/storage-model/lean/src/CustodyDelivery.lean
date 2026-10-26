import APositive
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem custody_ackMail (c : Config P) (s : Sys P) (g : Ghost P) (p : Fin P)
    (ok : Bool) (v : Int) (took : Bool) (h : Custody c ⟨s,g⟩) :
    Custody c ⟨ackMail s p ok v took,g⟩ := by
  cases ok <;> exact h

theorem postPromises_ackMail (x : State P) (s : Sys P) (g : Ghost P) (p : Fin P)
    (ok : Bool) (v : Int) (took : Bool) :
    PostPromises x (ackMail s p ok v took) g = PostPromises x s g := by
  cases ok <;> rfl

theorem submit_risk (c : Config P) (mutant : c.mutant = "none") (pg : Page)
    (t bv v : Int) : (submitTo c pg t bv v).held = pg.held ∧
    (pg.risk = true → (submitTo c pg t bv v).risk = true) ∧
    (submitTo c pg t bv v).typed = true := by
  simp [submitTo,mutant]; split <;> simp_all

theorem custody_upOpen (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (h : Custody c x) :
    Custody c (upOpen c x {x.s with up := tail} m ok) := by
  simp only [upOpen]
  apply custody_commit
  · apply custody_ackMail
    apply custody_frame c x _ h
    all_goals try rfl
    all_goals try (split <;> (try split) <;> rfl)
    intro p hp alive
    have old := hp alive
    by_cases eq : p = m.p
    · subst p; simp [put,old.1]; exact old.2
    · simpa [put,eq] using old
  · rw [postPromises_ackMail]
    intro p
    simp only [PostPromises,put]
    by_cases eq : p = m.p
    · subst p
      by_cases held : (x.s.pages m.p).held = true
      · simp [held,dirty]; grind
      · simp [held]
    · simp [eq,dirty]; grind

theorem custody_upClose (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (h : Custody c x) :
    Custody c (upClose c x {x.s with up := tail} m) := by
  simp only [upClose,mutant]
  simp only [show ("none" == "MCL") = false from rfl,Bool.false_and,Bool.or_false]
  split
  · apply custody_kept_commit c x _ _ _ h
    all_goals try rfl
    · intro p hp alive
      have old := hp alive
      by_cases eq : p = m.p
      · subst p
        have cl : clean (x.s.pages m.p) = true := by simp_all
        simp [clean,old.2] at cl
      · simpa [put,eq] using old
    · intro p
      by_cases eq : p = m.p
      · subst p; simp [PostPromises,put]
      · simp [PostPromises,put,eq,dirty]; grind
  · apply custody_kept_commit c x _ _ _ h
    all_goals first | rfl | (intro p hp; exact hp) | exact postPromises_same_pages x _ _ rfl

theorem custody_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (h : Custody c x) :
    Custody c (upSubmit c x {x.s with up := tail} m) := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,show ("none" != "MIS") = true from rfl,
    Bool.and_true,Bool.false_eq_true,ite_false]
  split
  · apply custody_kept_commit c x _ _ _ h
    all_goals first | rfl | (intro p hp; exact hp) | exact postPromises_same_pages x _ _ rfl
  · apply custody_commit
    · apply custody_ackMail
      apply custody_frame c x _ h
      all_goals try rfl
      intro p hp alive
      have old := hp alive
      by_cases eq : p = m.p
      · subst p; simp only [put,ite_true]; split
        · exact ⟨old.1,by simp [old.2]⟩
        · exact ⟨(submit_risk c mutant _ _ _ _).1.trans old.1,
            (submit_risk c mutant _ _ _ _).2.1 old.2⟩
      · simpa [put,eq] using old
    · rw [postPromises_ackMail]
      apply postPromises_noCleaning
      intro p hh hd
      by_cases eq : p = m.p
      · subst p; simp only [put,ite_true]; split
        · simp [dirty,clean]
        · simp [dirty,clean,(submit_risk c mutant _ _ _ _).2.2]
      · simpa [put,eq] using hd

theorem custody_upDiscard (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) (h : Custody c x) :
    Custody c (upDiscard c x {x.s with up := tail} m ok) := by
  simp only [upDiscard]
  split
  · apply custody_kept_commit c x _ _ _ h
    all_goals first | rfl | (intro p hp; exact hp) | exact postPromises_same_pages x _ _ rfl
  · split
    · apply custody_commit
      · exact custody_ackMail c _ _ _ _ _ _ h
      · rw [postPromises_ackMail]; exact postPromises_same_pages x _ _ rfl
    · apply custody_commit
      · apply custody_ackMail
        intro p need
        by_cases eq : p = m.p
        · subst p
          by_cases retain : ((x.g.promise m.p).on &&
            !((x.g.promise m.p).saved && decide (x.g.ext m.p > (x.g.promise m.p).ep)) &&
            (x.g.promise m.p).bytes == x.s.disk m.p) = true
          · have oldneed : NeedsDraft c x m.p := by
              simpa only [NeedsDraft,LivePromise,Covers,put,ite_true,retain] using need
            have old := h m.p oldneed
            refine ⟨by simpa only [put,ite_true,retain] using old.1,?_⟩
            intro alive
            have held := old.2 alive
            have bytes : (x.g.promise m.p).bytes = x.s.disk m.p := by simp_all
            simp [put,mutant,held.1,held.2,old.1,bytes]
          · simp only [NeedsDraft,LivePromise,put,ite_true,retain,ite_false,endPromise] at need
            simp at need
        · have oldneed : NeedsDraft c x p := by
            simpa only [NeedsDraft,LivePromise,Covers,put,eq,ite_false] using need
          have old := h p oldneed
          exact ⟨by simpa [put,eq] using old.1,by simpa [put,eq] using old.2⟩
      · rw [postPromises_ackMail]
        intro p
        by_cases eq : p = m.p
        · subst p; simp [PostPromises]
        · simp [PostPromises,put,eq,dirty]; grind

theorem custody_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) (h : Custody c x)
    (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) (distinct : m.p ≠ m.q) :
    Custody c (upOp c x {x.s with up := tail} m ok) := by
  simp only [upOp,mutant]
  simp only [show ("none" == "MOD") = false from rfl,show ("none" != "MOD") = true from rfl,
    show ("none" != "MSM") = true from rfl,Bool.and_true,Bool.false_eq_true,ite_false]
  split
  · apply custody_kept_commit c x _ _ _ h
    all_goals first | rfl | (intro p hp; exact hp) | exact postPromises_same_pages x _ _ rfl
  · split
    · apply custody_commit
      · exact custody_ackMail c _ _ _ _ _ _ (custody_ackMail c _ _ _ _ _ _ h)
      · rw [postPromises_ackMail,postPromises_ackMail]; exact postPromises_same_pages x _ _ rfl
    · have ha : (x.s.pages m.p).held = true := by simp_all
      have hb : (x.s.pages m.q).held = true := by simp_all
      have fresh : (x.g.promise m.q).ver < x.g.vc + 1 := by have b := bound m.q; omega
      have acknew : ack (x.g.promise m.q) m.t2 (x.g.vc+1) false 0 =
          (⟨true,m.t2,x.g.vc+1,false,0⟩ : Promise) := by simp [ack,fresh]
      have nbheld : (submitTo c (x.s.pages m.q) m.t2 m.bv2 (x.g.vc+1)).held = true :=
        (submit_risk c mutant _ _ _ _).1.trans hb
      have nbbytes : (submitTo c (x.s.pages m.q) m.t2 m.bv2 (x.g.vc+1)).buf = m.t2 := by
        simp [submitTo,mutant]; split <;> rfl
      apply custody_commit
      · apply custody_ackMail; apply custody_ackMail
        intro p need
        by_cases eq : p = m.q
        · subst p
          refine ⟨?_,?_⟩
          · simp [put,acknew,draftEntry,nbheld,nbbytes]
          · intro alive
            simp [put,Ne.symm distinct,nbheld]
        · have oldneed : NeedsDraft c x p := by
            simpa [NeedsDraft,LivePromise,Covers,put,eq] using need
          have old := h p oldneed
          refine ⟨by simpa [put,eq] using old.1,?_⟩
          intro alive
          have held := old.2 alive
          by_cases src : p = m.p
          · subst p; simp only [put,ite_true]
            exact ⟨(submit_risk c mutant _ _ _ _).1.trans held.1,
              (submit_risk c mutant _ _ _ _).2.1 held.2⟩
          · simpa [put,eq,src] using held
      · rw [postPromises_ackMail,postPromises_ackMail]
        apply postPromises_noCleaning
        intro p hh hd
        by_cases src : p = m.p
        · subst p; simp [put,dirty,clean,(submit_risk c mutant _ _ _ _).2.2]
        · by_cases dst : p = m.q
          · subst p; simp [put,src,dirty,clean]
          · simpa [put,src,dst] using hd

end Storage
