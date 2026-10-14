import CoreBytes21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def ObservedByte (c : Config P) (t : Int) : Prop :=
  t = noneBytes ∨ t = absent ∨ t ∈ c.texts
def ObservationDomain (c : Config P) (s : Sys P) : Prop :=
  (∀ p, ObservedByte c (s.disk p)) ∧
  (∀ p, ObservedByte c (s.stable p)) ∧
  (∀ p, ObservedByte c (s.pages p).obs) ∧
  (∀ p, ObservedByte c (s.w p).obs) ∧
  (∀ p, ObservedByte c (s.mb p).obs) ∧
  ObservedByte c s.job.bytes ∧
  (∀ p, ObservedByte c (s.pages p).buf) ∧
  (∀ p, ObservedByte c (s.drafts p).bytes) ∧
  (∀ m ∈ s.up, ObservedByte c m.t ∧ ObservedByte c m.t2)

-- Request/control fields use the integer 0 as well; buffer domain is kept
-- separate from observation domain in the proof below.
def ObsOnly (c : Config P) (s : Sys P) : Prop :=
  (∀ p, ObservedByte c (s.disk p)) ∧
  (∀ p, ObservedByte c (s.stable p)) ∧
  (∀ p, ObservedByte c (s.pages p).obs) ∧
  (∀ p, ObservedByte c (s.w p).obs) ∧
  (∀ p, ObservedByte c (s.mb p).obs)

theorem obsOnly_init (c : Config P) : ObsOnly c (init c).s := by
  have hd : ∀ p : Fin P, ObservedByte c c.texts[p.val % c.texts.length]! := by
    intro p
    have hi : p.val % c.texts.length < c.texts.length := Nat.mod_lt _ (List.length_pos_iff.mpr c.nonempty)
    apply Or.inr; apply Or.inr
    rw [List.getElem!_eq_getElem?_getD,List.getElem?_eq_getElem hi]
    exact List.getElem_mem hi
  exact ⟨hd,hd,by simp [init,ObservedByte],by simp [init,ObservedByte],by simp [init,ObservedByte]⟩

theorem obsOnly_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : ObsOnly c s) : ObsOnly c (commit c x s g n).s := by
  refine ⟨h.1,h.2.1,h.2.2.1,h.2.2.2.1,?_⟩
  intro p
  change ObservedByte c (if _ then _ else s.mb p).obs
  split
  · exact h.2.2.1 p
  · exact h.2.2.2.2 p

theorem obsOnly_ackMail (c : Config P) (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool)
    (h : ObsOnly c s) : ObsOnly c (ackMail s p ok v took) := by
  cases ok
  · exact h
  · refine ⟨h.1,h.2.1,h.2.2.1,h.2.2.2.1,?_⟩
    intro q
    simp only [ackMail,put,Bool.not_true,Bool.false_eq_true,ite_false]
    split
    · exact h.2.2.1 p
    · exact h.2.2.2.2 q

theorem obsOnly_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : ObsOnly c s) (hx : ObsOnly c x.s) : ObsOnly c (upOpen c x s m ok).s := by
  apply obsOnly_commit
  apply obsOnly_ackMail
  refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
  apply property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
  split
  · exact h.2.2.1 m.p
  · exact hx.1 m.p
theorem obsOnly_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : ObsOnly c s) : ObsOnly c (upClose c x s m).s := by
  apply obsOnly_commit
  split
  · refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
    exact property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1 (by simp [ObservedByte])
  · exact h
theorem obsOnly_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : ObsOnly c s) (hx : ObsOnly c x.s) : ObsOnly c (upDiscard c x s m ok).s := by
  simp only [upDiscard]
  split
  · exact obsOnly_commit c x _ _ _ h
  · split
    · exact obsOnly_commit c x _ _ _ (obsOnly_ackMail c _ _ _ _ _ h)
    · apply obsOnly_commit
      apply obsOnly_ackMail
      refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
      exact property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1 (hx.1 m.p)
theorem obs_submit (c : Config P) (pg : Page) (t bv v : Int) :
    (submitTo c pg t bv v).obs = pg.obs := by
  simp only [submitTo]; split <;> (try split) <;> rfl
theorem obsOnly_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : ObsOnly c s) : ObsOnly c (upSubmit c x s m).s := by
  simp only [upSubmit]; split
  · exact obsOnly_commit c x _ _ _ h
  · apply obsOnly_commit
    apply obsOnly_ackMail
    refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
    split
    · apply property_put _ _ _ (fun p : Page => ObservedByte c p.obs)
      · exact property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
          (by split <;> simp only [obs_submit] <;> exact h.2.2.1 m.p)
      · simp [ObservedByte]
    · exact property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
        (by split <;> simp only [obs_submit] <;> exact h.2.2.1 m.p)
theorem obsOnly_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : ObsOnly c s) : ObsOnly c (upOp c x s m ok).s := by
  simp only [upOp]; split
  · exact obsOnly_commit c x _ _ _ h
  · split
    · exact obsOnly_commit c x _ _ _ (obsOnly_ackMail c _ _ _ _ _ (obsOnly_ackMail c _ _ _ _ _ h))
    · apply obsOnly_commit
      apply obsOnly_ackMail
      apply obsOnly_ackMail
      refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
      apply property_put _ _ _ (fun p : Page => ObservedByte c p.obs)
      · apply property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
        simpa only [obs_submit] using h.2.2.1 m.q
      · simpa only [obs_submit] using h.2.2.1 m.p

end Storage21
