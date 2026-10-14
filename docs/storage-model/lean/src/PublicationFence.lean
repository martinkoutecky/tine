import CustodyWrites
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def PublicationFence (s : Sys P) (g : Ghost P) : Prop :=
  s.job.on = true → s.job.phase = 3 →
  0 ≤ s.job.ep ∧ (g.promise s.job.p).ep ≤ s.job.ep ∧
  (g.ext s.job.p ≤ s.job.ep → s.disk s.job.p = s.job.bytes)

theorem fence_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : PublicationFence s g) : PublicationFence (commit c x s g n).s (commit c x s g n).g := by
  intro on phase
  have raw := h on phase
  refine ⟨raw.1,?_,raw.2.2⟩
  change (PostPromises x s g s.job.p).ep ≤ s.job.ep
  simp only [PostPromises]; split
  · exact ack_epoch_le _ _ _ _ _ _ raw.2.1 raw.1
  · exact raw.2.1

theorem fence_ackMail (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool)
    (h : PublicationFence s g) : PublicationFence (ackMail s p ok v t) g := by cases ok <;> exact h

theorem fence_frame (s t : Sys P) (g ng : Ghost P) (h : PublicationFence s g)
    (job : t.job = s.job) (disk : t.disk = s.disk) (ext : ng.ext = g.ext)
    (promise : ∀ p u, 0 ≤ u → (g.promise p).ep ≤ u → (ng.promise p).ep ≤ u) :
    PublicationFence t ng := by
  intro on phase
  have raw := h (by simpa only [job] using on) (by simpa only [job] using phase)
  refine ⟨by simpa only [job] using raw.1,?_,?_⟩
  · simpa only [job] using promise s.job.p s.job.ep raw.1 raw.2.1
  · simpa only [job,disk,ext] using raw.2.2

theorem fence_upOpen (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : PublicationFence x.s x.g) :
    PublicationFence (upOpen c x {x.s with up := tail} m ok).s (upOpen c x {x.s with up := tail} m ok).g := by
  apply fence_commit
  apply fence_ackMail
  apply fence_frame x.s _ x.g _ h
  all_goals first | rfl | skip
  all_goals (try split) <;> (try split)
  all_goals first | rfl | (intro p u hn hp; exact hp)

theorem fence_upClose (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (h : PublicationFence x.s x.g) :
    PublicationFence (upClose c x {x.s with up := tail} m).s (upClose c x {x.s with up := tail} m).g := by
  apply fence_commit
  split <;> apply fence_frame x.s _ x.g _ h
  all_goals first | rfl | (intro p u hn hp; exact hp)

theorem fence_upSubmit (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (h : PublicationFence x.s x.g) :
    PublicationFence (upSubmit c x {x.s with up := tail} m).s (upSubmit c x {x.s with up := tail} m).g := by
  simp only [upSubmit]; split
  all_goals apply fence_commit
  all_goals (try apply fence_ackMail)
  all_goals apply fence_frame x.s _ x.g _ h
  all_goals first | rfl | (intro p u hn hp; exact hp)

theorem fence_upDiscard (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : PublicationFence x.s x.g) :
    PublicationFence (upDiscard c x {x.s with up := tail} m ok).s
      (upDiscard c x {x.s with up := tail} m ok).g := by
  simp only [upDiscard]; split <;> (try split)
  all_goals apply fence_commit
  all_goals (try apply fence_ackMail)
  all_goals apply fence_frame x.s _ x.g _ h
  all_goals try rfl
  all_goals intro p u hn hp
  all_goals first | exact hp | skip
  simp only [put]; split
  · subst p; split <;> exact hp
  · exact hp

theorem fence_upOp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : PublicationFence x.s x.g) :
    PublicationFence (upOp c x {x.s with up := tail} m ok).s
      (upOp c x {x.s with up := tail} m ok).g := by
  simp only [upOp]; split <;> (try split)
  all_goals apply fence_commit
  all_goals (try apply fence_ackMail) <;> (try apply fence_ackMail)
  all_goals apply fence_frame x.s _ x.g _ h
  all_goals try rfl
  all_goals intro p u hn hp
  all_goals first | exact hp | skip
  simp only [put]; split
  · subst p; exact ack_epoch_le _ _ _ _ _ _ hp hn
  · exact hp

theorem fence_execute (c : Config P) (x : State P) (a : Action P)
    (h : PublicationFence x.s x.g) (epochs : EpochBounds x.s x.g)
    (hen : enabled c x a = true) : PublicationFence (execute c x a).s (execute c x a).g := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact fence_upOpen c x m tail ok h
        | exact fence_upClose c x m tail h
        | exact fence_upDiscard c x m tail ok h
        | exact fence_upOp c x m tail ok h
        | exact fence_upSubmit c x m tail h
  case rename =>
    apply fence_commit
    intro on phase
    exact ⟨epochs.ext _,epochs.promise _,by intro _; simp [put]⟩
  case dirSync ok => apply fence_commit; simp [PublicationFence,nojob]
  case flush p => apply fence_commit; simp [PublicationFence]
  case check =>
    simp only [check]; split
    · apply fence_commit; simp [PublicationFence,nojob]
    · apply fence_commit; simp [PublicationFence]
  case saveFail => apply fence_commit; simp [PublicationFence,nojob]
  case switchFin => apply fence_commit; simp [PublicationFence,down,nojob]
  case crash => apply fence_commit; simp [PublicationFence,down,nojob]
  case power keep => apply fence_commit; simp [PublicationFence,down,nojob]
  case draftSync p =>
    apply fence_commit
    apply fence_frame x.s _ x.g _ h
    all_goals try rfl
    · split <;> (try split) <;> rfl
    · intro q u hn hp
      split <;> (try split)
      all_goals first | exact hp | skip
      all_goals simp only [put]; split
      all_goals first | exact hp | skip
      all_goals subst q; exact ack_epoch_le _ _ _ _ _ _ hp hn
  case extWriteD p v dur =>
    apply fence_commit
    intro on phase
    have raw := h on phase
    refine ⟨raw.1,raw.2.1,?_⟩
    by_cases same : x.s.job.p = p
    · intro le
      have old := epochs.job
      simp only [put,same,ite_true] at le old
      omega
    · simpa [put,same] using raw.2.2
  all_goals apply fence_commit
  all_goals apply fence_frame x.s _ x.g _ h
  all_goals first | rfl | (intro p u hn hp; exact hp)

theorem publicationFence_reachable (c : Config P) (x : State P) (hx : Reachable c x) :
    PublicationFence x.s x.g := by
  induction hx with
  | initial => simp [PublicationFence,init,nojob]
  | @advance x y hr a hs ih =>
    have epochs := epochs_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact fence_execute c x a ih epochs (by assumption)
    · contradiction

#print axioms publicationFence_reachable
end Storage
