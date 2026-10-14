import LifetimeClosure
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

structure EpochBounds (s : Sys P) (g : Ghost P) : Prop where
  ext : ∀ p, 0 ≤ g.ext p
  promise : ∀ p, (g.promise p).ep ≤ g.ext p
  job : s.job.ep ≤ g.ext s.job.p

theorem ack_epoch_le (pr : Promise) (b v : Int) (sv : Bool) (ep u : Int)
    (old : pr.ep ≤ u) (new : ep ≤ u) : (ack pr b v sv ep).ep ≤ u := by
  simp only [ack]; split <;> first | exact new | exact old

theorem epochs_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : EpochBounds s g) : EpochBounds (commit c x s g n).s (commit c x s g n).g := by
  refine ⟨h.ext,?_,h.job⟩
  intro p
  change (PostPromises x s g p).ep ≤ g.ext p
  simp only [PostPromises]; split
  · exact ack_epoch_le _ _ _ _ _ _ (h.promise p) (h.ext p)
  · exact h.promise p

theorem epochs_ackMail (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool)
    (h : EpochBounds s g) : EpochBounds (ackMail s p ok v t) g := by
  exact ⟨h.ext,h.promise,by simpa only [job_ackMail] using h.job⟩

theorem epochs_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : EpochBounds s x.g) : EpochBounds (upOpen c x s m ok).s (upOpen c x s m ok).g := by
  simp only [upOpen]
  apply epochs_commit
  apply epochs_ackMail
  split <;> (try split) <;> exact ⟨h.ext,h.promise,h.job⟩

theorem epochs_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : EpochBounds s x.g) : EpochBounds (upClose c x s m).s (upClose c x s m).g := by
  apply epochs_commit
  split <;> (try split) <;> exact ⟨h.ext,h.promise,h.job⟩

theorem epochs_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : EpochBounds s x.g) : EpochBounds (upSubmit c x s m).s (upSubmit c x s m).g := by
  simp only [upSubmit]; split
  · exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.job⟩
  · exact epochs_commit c x _ _ _ (epochs_ackMail _ _ _ _ _ _ ⟨h.ext,h.promise,h.job⟩)

theorem epochs_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : EpochBounds s x.g) : EpochBounds (upDiscard c x s m ok).s (upDiscard c x s m ok).g := by
  simp only [upDiscard]; split
  · exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.job⟩
  · split
    · exact epochs_commit c x _ _ _ (epochs_ackMail _ _ _ _ _ _ ⟨h.ext,h.promise,h.job⟩)
    · apply epochs_commit
      apply epochs_ackMail
      refine ⟨h.ext,?_,h.job⟩
      intro p; by_cases eq : p = m.p
      · subst p; simp only [put,ite_true]; split <;> exact h.promise _
      · simpa [put,eq] using h.promise p

theorem epochs_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : EpochBounds s x.g) : EpochBounds (upOp c x s m ok).s (upOp c x s m ok).g := by
  simp only [upOp]; split
  · exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.job⟩
  · split
    · exact epochs_commit c x _ _ _ (epochs_ackMail _ _ _ _ _ _ (epochs_ackMail _ _ _ _ _ _ ⟨h.ext,h.promise,h.job⟩))
    · apply epochs_commit
      apply epochs_ackMail; apply epochs_ackMail
      refine ⟨h.ext,?_,h.job⟩
      intro p; by_cases eq : p = m.q
      · subst p; simp only [put,ite_true]; exact ack_epoch_le _ _ _ _ _ _ (h.promise _) (h.ext _)
      · simpa [put,eq] using h.promise p

theorem epochs_execute (c : Config P) (x : State P) (a : Action P) (h : EpochBounds x.s x.g) :
    EpochBounds (execute c x a).s (execute c x a).g := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have raw : EpochBounds {x.s with up := tail} x.g := ⟨h.ext,h.promise,h.job⟩
      simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact epochs_upOpen c x _ m ok raw
        | exact epochs_upClose c x _ m raw
        | exact epochs_upDiscard c x _ m ok raw
        | exact epochs_upOp c x _ m ok raw
        | exact epochs_upSubmit c x _ m raw
  case rename => exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,Int.le_refl _⟩
  case dirSync ok =>
    apply epochs_commit
    refine ⟨h.ext,?_,h.ext (zero c)⟩
    intro p
    cases ok
    · exact h.promise p
    · simp only [ite_true,put]; split
      · have eq : p = x.s.job.p := by assumption
        subst p; exact ack_epoch_le _ _ _ _ _ _ (h.promise _) h.job
      · exact h.promise p
  case draftSync p =>
    simp only [draftSync]
    apply epochs_commit
    split <;> (try split)
    all_goals refine ⟨h.ext,?_,h.job⟩
    all_goals intro q
    all_goals first | exact h.promise q | skip
    all_goals simp only [put]
    all_goals (try split) <;> (try split) <;> (try split)
    all_goals have old := h.promise q
    all_goals have zero := h.ext q
    all_goals simp_all only
    all_goals exact ack_epoch_le _ _ _ _ _ _ old zero
  case extWriteD p v dur =>
    apply epochs_commit
    refine ⟨?_,?_,?_⟩
    · intro q; by_cases eq : q = p
      · subst q; simp only [put,ite_true]; have old := h.ext p; omega
      · simpa [put,eq] using h.ext q
    · intro q; by_cases eq : q = p
      · subst q; simp only [put,ite_true]; have old := h.promise p; omega
      · simpa [put,eq] using h.promise q
    · have old := h.job
      by_cases eq : x.s.job.p = p <;> simp_all [put]; omega
  case flush p => exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.ext p⟩
  case check =>
    simp only [check]; split
    · exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.ext (zero c)⟩
    · exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.job⟩
  case saveFail => exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.ext (zero c)⟩
  case switchFin => exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.ext (zero c)⟩
  case crash => exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.ext (zero c)⟩
  case power keep =>
    apply epochs_commit
    refine ⟨h.ext,?_,h.ext (zero c)⟩
    intro p; dsimp only; split <;> (try split) <;> exact h.promise p
  all_goals exact epochs_commit c x _ _ _ ⟨h.ext,h.promise,h.job⟩

theorem epochs_reachable (c : Config P) (x : State P) (hx : Reachable c x) : EpochBounds x.s x.g := by
  induction hx with
  | initial => constructor <;> simp [init,nojob]
  | advance hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact epochs_execute c _ a ih
    · contradiction
#print axioms epochs_reachable
end Storage
