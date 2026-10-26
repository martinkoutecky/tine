import CoreFence6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem ack_saved_active (pr : Promise) (b v : Int) (ep : Int)
    (bound : pr.ver ≤ v) (on : (ack pr b v true ep).on = true) :
    (ack pr b v true ep).saved = true := by
  simp only [ack] at on ⊢
  split <;> simp_all
  all_goals cases sv : pr.saved <;> simp_all
  all_goals have eq : v = pr.ver := by omega
  all_goals simp_all

theorem ack_saved_then_unsigned (pr : Promise) (b v ep : Int) :
    ack (ack pr b v true ep) b v false 0 = ack pr b v true ep := by
  simp only [ack]
  split <;> (try split) <;> simp_all
  all_goals omega

theorem custody_dirSync_failed (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (h : Custody c x) : Custody c (dirSync c x false) := by
  simp only [dirSync,Bool.not_false,Bool.false_and,Bool.false_eq_true,ite_false,mutant,
    show ("none" == "MRE") = false from rfl,Bool.true_and]
  apply custody_kept_commit c x _ _ _ h
  all_goals try rfl
  · intro p hp alive
    have old := hp alive
    by_cases same : p = x.s.job.p <;> simp_all [put,onReply,mutant]
  · apply postPromises_noCleaning
    intro p held dirty
    by_cases same : p = x.s.job.p <;> simp_all [put,onReply,mutant,Storage21.dirty,clean]

theorem custody_dirSync_published (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (snapAll : JobSnapshot x.s) (lineage : PromiseLineage x)
    (agree : ActiveAgreement x.s x.g) (fenceAll : PublicationFence x.s x.g) (h : Custody c x) (hen : enabled c x (.dirSync true) = true) :
    Custody c (dirSync c x true) := by
  have on : x.s.job.on = true := by simp [enabled] at hen; grind
  have phase : x.s.job.phase = 3 := by simp [enabled] at hen; grind
  have snap := snapAll on
  have fence := fenceAll on phase
  have bound : (x.g.promise x.s.job.p).ver ≤ x.s.job.ver := by
    simpa only [snap.2.2.2.1] using (lineage.2.1 _ snap.1).1
  have eqbytes : (x.g.promise x.s.job.p).on = true →
      (x.g.promise x.s.job.p).ver = x.s.job.ver → (x.g.promise x.s.job.p).bytes = x.s.job.bytes := by
    intro active eq
    exact (agree _ snap.1 active (eq.trans snap.2.2.2.1.symm)).trans snap.2.1
  simp only [dirSync,ite_true,Bool.not_true,Bool.false_and,Bool.false_eq_true,ite_false]
  apply custody_commit
  · intro p need
    by_cases same : p = x.s.job.p
    · subst p
      let pr := ack (x.g.promise x.s.job.p) x.s.job.bytes x.s.job.ver true x.s.job.ep
      have active : pr.on = true := by simpa only [LivePromise,put,ite_true] using need.1.1
      have saved : pr.saved = true := ack_saved_active _ _ _ _ bound active
      have bytes : pr.bytes = x.s.job.bytes := ack_active_bytes _ _ _ _ _ bound eqbytes active
      have ep : pr.ep ≤ x.s.job.ep := ack_epoch_le _ _ _ _ _ _ fence.2.1 (Int.le_refl _)
      have nonexpired : ¬(pr.saved = true ∧ pr.ep < x.g.ext x.s.job.p) := by
        simpa only [put,ite_true] using need.1.2
      have ext : x.g.ext x.s.job.p ≤ x.s.job.ep := by
        have notlt : ¬ pr.ep < x.g.ext x.s.job.p := fun lt => nonexpired ⟨saved,lt⟩
        have le : x.g.ext x.s.job.p ≤ pr.ep := by omega
        omega
      have disk : x.s.disk x.s.job.p = x.s.job.bytes := fence.2.2 ext
      rcases need.2 with unsigned | lost | ⟨strong,lost⟩
      · have bad : pr.saved = false := by simpa only [put,ite_true] using unsigned
        simp_all
      · apply False.elim; apply lost; left
        simpa only [put,ite_true] using disk.trans bytes.symm
      · apply False.elim; apply lost; left
        simp [put,strong,disk,pr] at bytes ⊢
        exact bytes.symm
    · have oldneed : NeedsDraft c x p := by
        simp only [NeedsDraft,LivePromise,Covers] at need ⊢
        split at need <;> simpa [put,same] using need
      have old := h p oldneed
      exact ⟨by simpa [put,same] using old.1,by simpa [put,same] using old.2⟩
  · intro p
    by_cases same : p = x.s.job.p
    · subst p
      simp only [PostPromises,put,ite_true]
      split
      · simp only [reply_ver,snap.2.2.2.1]
        have replybuf : (onReply c (x.s.pages x.s.job.p) "Published" x.s.job.bytes).buf = x.s.job.bytes := by
          simp only [onReply]; split <;> exact snap.2.1
        rw [replybuf]
        exact ack_saved_then_unsigned _ _ _ _
      · rfl
    · simp [PostPromises,put,same,dirty]; grind

theorem custody_dirSync (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (ok : Bool) (snapAll : JobSnapshot x.s) (lineage : PromiseLineage x)
    (agree : ActiveAgreement x.s x.g) (fenceAll : PublicationFence x.s x.g) (h : Custody c x)
    (hen : enabled c x (.dirSync ok) = true) : Custody c (dirSync c x ok) := by
  cases ok
  · exact custody_dirSync_failed c mutant x h
  · exact custody_dirSync_published c mutant x snapAll lineage agree fenceAll h hen

#print axioms custody_dirSync
end Storage21
