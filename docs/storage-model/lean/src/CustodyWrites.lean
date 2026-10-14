import ActiveAgreement
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem covers_mono (x y : State P) (p : Fin P) (b : Int)
    (pr : y.g.promise p = x.g.promise p)
    (hist : ∀ w ∈ x.g.wrote p, w ∈ y.g.wrote p) (h : Covers x p b) : Covers y p b := by
  rcases h with eq | ⟨w,hw,hb,hv⟩
  · exact Or.inl (by simpa only [pr] using eq)
  · exact Or.inr ⟨w,hist w hw,hb,by simpa only [pr] using hv⟩

theorem custody_writeDisk (c : Config P) (x : State P) (p : Fin P) (b v : Int)
    (h : Custody c x) (bound : (x.g.promise p).ver ≤ v)
    (equal : (x.g.promise p).on = true → (x.g.promise p).ver = v → b = (x.g.promise p).bytes) :
    Custody c ⟨{x.s with disk := put x.s.disk p b},
      {x.g with wrote := put x.g.wrote p (union (x.g.wrote p) [(b,v)])}⟩ := by
  let y : State P := ⟨{x.s with disk := put x.s.disk p b},
      {x.g with wrote := put x.g.wrote p (union (x.g.wrote p) [(b,v)])}⟩
  change Custody c y
  intro q need
  by_cases same : q = p
  · subst q
    have live : LivePromise x p := need.1
    have disk : Covers y p (y.s.disk p) := by
      by_cases eq : (x.g.promise p).ver = v
      · exact Or.inl (by simpa [y,put] using equal live.1 eq)
      · have lt : (x.g.promise p).ver < v := by omega
        exact Or.inr ⟨(b,v),by simp [y,put,mem_union_general],by simp [y,put],lt⟩
    have mono : ∀ a, Covers x p a → Covers y p a := by
      intro a ha
      exact covers_mono x y p a rfl (by intro w hw; simp [y,put,mem_union_general,hw]) ha
    have oldneed : NeedsDraft c x p := by
      refine ⟨live,?_⟩
      rcases need.2 with unsigned | lost | ⟨strong,lost⟩
      · exact Or.inl unsigned
      · exact False.elim (lost disk)
      · exact Or.inr (Or.inr ⟨strong,fun cov => lost (mono _ cov)⟩)
    exact h p oldneed
  · have oldneed : NeedsDraft c x q := by simpa [y,NeedsDraft,LivePromise,Covers,put,same] using need
    exact h q oldneed

theorem custody_rename (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) (h : Custody c x) (hen : enabled c x .rename = true) :
    Custody c (rename c x) := by
  have on : x.s.job.on = true := by simp [enabled] at hen; grind
  have snap := snapshot_reachable c mutant x hx on
  have lineage := lineage_reachable c mutant x hx
  have agreement := activeAgreement_reachable c mutant x hx
  have b : (x.g.promise x.s.job.p).ver ≤ x.s.job.ver := by simpa only [snap.2.2.2.1] using (lineage.2.1 _ snap.1).1
  have eq : (x.g.promise x.s.job.p).on = true →
      (x.g.promise x.s.job.p).ver = x.s.job.ver → x.s.job.bytes = (x.g.promise x.s.job.p).bytes := by
    intro active hv
    exact snap.2.1.symm.trans (agreement _ snap.1 active (hv.trans snap.2.2.2.1.symm)).symm
  apply custody_commit
  · exact custody_writeDisk c x _ _ _ h b eq
  · exact postPromises_same_pages x _ _ rfl

#print axioms custody_rename

theorem ack_active_bytes (pr : Promise) (b v : Int) (sv : Bool) (ep : Int)
    (bound : pr.ver ≤ v) (equal : pr.on = true → pr.ver = v → pr.bytes = b)
    (on : (ack pr b v sv ep).on = true) : (ack pr b v sv ep).bytes = b := by
  by_cases changed : (decide (v > pr.ver) || v == pr.ver && pr.on && sv && !pr.saved) = true
  · simp only [ack,changed,ite_true]
  · simp only [ack,changed,ite_false] at on ⊢
    have eq : pr.ver = v := by simp only [Bool.or_eq_true,decide_eq_true_eq] at changed; omega
    exact equal on eq

theorem custody_draftSync (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) (h : Custody c x)
    (hen : enabled c x (.draftSync p) = true) : Custody c (draftSync c x p) := by
  have lineage := lineage_reachable c mutant x hx
  have agreement := activeAgreement_reachable c mutant x hx
  simp only [draftSync,mutant,show ("none" == "MBD") = false from rfl,
    Bool.false_and,Bool.false_eq_true,ite_false]
  apply custody_commit
  · intro q need
    by_cases same : q = p
    · subst q
      by_cases flags : ((x.s.pages p).held && (x.s.pages p).risk) = true
      · have both : (x.s.pages p).held = true ∧ (x.s.pages p).risk = true := by simpa only [Bool.and_eq_true] using flags
        have held := both.1
        have risk := both.2
        have de : draftEntry (x.s.pages p) = ⟨(x.s.pages p).buf,(x.s.pages p).base,(x.s.pages p).ver⟩ := by
          simp [draftEntry,flags]
        have bytes := (bytes_reachable c x hx).pages p
        have nonempty : (x.s.pages p).buf ≠ noneBytes := real_positive_not_none c T texts _ bytes
        have entry : ((draftEntry (x.s.pages p)).bytes != noneBytes) = true := by simpa [de] using nonempty
        have en : ((x.s.pages p).buf != noneBytes) = true := by simpa using nonempty
        have active : (ack (x.g.promise p) (x.s.pages p).buf (x.s.pages p).ver false 0).on = true := by
          simpa only [LivePromise,entry,en,ite_true,put,ite_true,de] using need.1.1
        have eq := ack_active_bytes _ _ _ false 0 (lineage.2.1 p held).1
          (fun on eq => agreement p held on eq) active
        refine ⟨?_,?_⟩
        · simpa only [entry,en,ite_true,put,ite_true,de] using eq.symm
        · intro _; exact ⟨held,risk⟩
      · have de : draftEntry (x.s.pages p) = ({} : Draft) := by simp [draftEntry,flags]
        have oldneed : NeedsDraft c x p := by
          simpa only [NeedsDraft,LivePromise,Covers,de,Storage.noneBytes,Int.reduceNeg,
            bne_self_eq_false,Bool.false_eq_true,ite_false] using need
        have old := h p oldneed
        have alive : x.s.alive = true := by simp [enabled] at hen; grind
        have hr := old.2 alive
        simp [hr.1,hr.2] at flags
    · have oldneed : NeedsDraft c x q := by
        split at need <;> simpa [NeedsDraft,LivePromise,Covers,put,same] using need
      have old := h q oldneed
      refine ⟨?_,old.2⟩
      split <;> simpa [put,same] using old.1
  · exact postPromises_same_pages x _ _ rfl
end Storage
