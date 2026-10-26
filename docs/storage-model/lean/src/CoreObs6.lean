import CoreObservations6
import CoreBytes21
namespace Storage21

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem real_observed {c : Config P} {b : Int} (h : RealByte c b) : ObservedByte c b := Or.inr h

theorem obs_table (c : Config P) (pg : Page) (d v : Int) : (table c pg d v).obs = d := by
  simp only [table]; split <;> (try split) <;> (try split) <;> (try split) <;> rfl
theorem obs_reply (c : Config P) (pg : Page) (o : String) (b : Int) : (onReply c pg o b).obs = pg.obs := by
  simp only [onReply]; split <;> rfl

theorem obsOnly_execute (c : Config P) (x : State P) (a : Action P)
    (h : ObsOnly c x.s) (hb : ByteDomain c x.s) (hen : enabled c x a = true) :
    ObsOnly c (execute c x a).s := by
  cases a <;> simp only [execute]
  all_goals simp only [enabled,Bool.and_eq_true,Bool.or_eq_true,beq_iff_eq,bne_iff_ne] at hen
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have ht : ObsOnly c {x.s with up := tail} := h
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact obsOnly_upOpen c x _ m ok ht h
        | exact obsOnly_upClose c x _ m ht
        | exact obsOnly_upDiscard c x _ m ok ht h
        | exact obsOnly_upOp c x _ m ok ht
        | exact obsOnly_upSubmit c x _ m ht
  case wOpen p =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1 (by simp [ObservedByte]),h.2.2.2.2⟩
  case wClose p =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1 (by simp [ObservedByte]),h.2.2.2.2⟩
  case wEdit p v =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1 (h.2.2.2.1 p),h.2.2.2.2⟩
  case wSend p =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1 (h.2.2.2.1 p),h.2.2.2.2⟩
  case wResolve p v =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1 (h.2.2.2.1 p),h.2.2.2.2⟩
  case wDiscard p =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1 (h.2.2.2.1 p),h.2.2.2.2⟩
  case wOp p q t u =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,property_put _ _ _ (fun w : W => ObservedByte c w.obs)
      (property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1 (h.2.2.2.1 p)) (h.2.2.2.1 q),h.2.2.2.2⟩
  case wRecv p =>
    apply obsOnly_commit
    refine ⟨h.1,h.2.1,h.2.2.1,?_,?_⟩
    · apply property_put _ _ _ (fun w : W => ObservedByte c w.obs) h.2.2.2.1
      split <;> (try split) <;> (try split) <;> (try split) <;> (try split)
      all_goals first | exact h.2.2.2.1 p | exact h.2.2.2.2 p | simp [ObservedByte]
    · exact property_put _ _ _ (fun m : Mail => ObservedByte c m.obs) h.2.2.2.2 (by simp [ObservedByte])
  case observe p =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
      (by simpa only [obs_table] using h.1 p),h.2.2.2.1,h.2.2.2.2⟩
  case check =>
    simp only [check]; split
    · apply obsOnly_commit
      exact ⟨h.1,h.2.1,property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
        (by simpa only [obs_table] using h.1 x.s.job.p),h.2.2.2.1,h.2.2.2.2⟩
    · exact obsOnly_commit c x _ _ _ h
  case flush p => exact obsOnly_commit c x _ _ _ h
  case rename =>
    apply obsOnly_commit
    exact ⟨property_put _ _ _ (ObservedByte c) h.1 (real_observed (hb.job hen.1.2)),h.2⟩
  case dirSync ok =>
    apply obsOnly_commit
    refine ⟨h.1,?_,?_,h.2.2.2.1,h.2.2.2.2⟩
    · split
      · exact property_put _ _ _ (ObservedByte c) h.2.1 (h.1 _)
      · exact h.2.1
    · apply property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
      split
      · exact h.2.2.1 _
      · simpa only [obs_reply] using h.2.2.1 x.s.job.p
  case saveFail =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,property_put _ _ _ (fun p : Page => ObservedByte c p.obs) h.2.2.1
      (by simpa only [obs_reply] using h.2.2.1 x.s.job.p),h.2.2.2.1,h.2.2.2.2⟩
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> exact obsOnly_commit c x _ _ _ h
  case switchReq =>
    apply obsOnly_commit
    refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
    intro p; dsimp only; split <;> exact h.2.2.1 p
  case extWriteD p v dur =>
    have hv : ObservedByte c v := Or.inr (by simpa [RealByte] using hen.2)
    apply obsOnly_commit
    refine ⟨property_put _ _ _ (ObservedByte c) h.1 hv,?_,h.2.2⟩
    split
    · exact property_put _ _ _ (ObservedByte c) h.2.1 hv
    · exact h.2.1
  case windowCrash =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,h.2.2.1,by simp [ObservedByte],by simp [ObservedByte]⟩
  case launch =>
    apply obsOnly_commit
    refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
    intro p; simp only [restored]; split <;> simp [ObservedByte]
  case switchFin =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,by simp [down,ObservedByte],by simp [down,ObservedByte],by simp [down,ObservedByte]⟩
  case crash =>
    apply obsOnly_commit
    exact ⟨h.1,h.2.1,by simp [down,ObservedByte],by simp [down,ObservedByte],by simp [down,ObservedByte]⟩
  case power keep =>
    have hd : ∀ p, ObservedByte c (if keep p then x.s.disk p else x.s.stable p) := by
      intro p; split
      · exact h.1 p
      · exact h.2.1 p
    apply obsOnly_commit
    exact ⟨hd,hd,by simp [down,ObservedByte],by simp [down,ObservedByte],by simp [down,ObservedByte]⟩

theorem obsOnly_reachable (c : Config P) (x : State P) (hx : Reachable c x) : ObsOnly c x.s := by
  induction hx with
  | initial => exact obsOnly_init c
  | @advance x y hr a hs ih =>
    have hb := bytes_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact obsOnly_execute c x a ih hb (by assumption)
    · contradiction
#print axioms obsOnly_reachable
end Storage21
