import CoreCustody6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem custody_down (c : Config P) (x : State P) (h : Custody c x) : Custody c ⟨down c x.s,gDown x.g⟩ := by
  refine custody_frame c x _ h ?_ ?_ ?_ ?_ ?_ ?_ ?_
  all_goals try rfl
  intro p _ alive; simp [down] at alive

theorem custody_crash (c : Config P) (x : State P) (h : Custody c x) : Custody c (crash c x) :=
  custody_commit c x _ _ _ (custody_down c x h) (postPromises_unheld_new x _ _ (fun _ => rfl))

theorem custody_switchFin (c : Config P) (x : State P) (h : Custody c x) : Custody c (switchFin c x) :=
  custody_commit c x _ _ _ (custody_down c x h) (postPromises_unheld_new x _ _ (fun _ => rfl))

theorem custody_power (c : Config P) (x : State P) (keep : Fin P → Bool) (h : Custody c x) :
    Custody c (power c x keep) := by
  let nd := fun p => if keep p then x.s.disk p else x.s.stable p
  let np := fun p => let pr := x.g.promise p
    let kept := [nd p,(x.s.drafts p).bytes].contains pr.bytes
    if c.weak && pr.on && pr.saved && !kept then endPromise pr pr.ver else pr
  let s := {down c x.s with disk := nd,stable := nd}
  let g := {gDown x.g with promise := np}
  change Custody c (commit c x s g "power")
  apply custody_commit
  · intro p need
    have dead : s.alive = false := rfl
    refine ⟨?_,by intro alive; simp_all⟩
    by_cases ended : (c.weak && (x.g.promise p).on && (x.g.promise p).saved &&
      !([nd p,(x.s.drafts p).bytes].contains (x.g.promise p).bytes)) = true
    · have on := need.1.1
      have off : (np p).on = false := by simp only [np,ended,ite_true,endPromise]
      change (np p).on = true at on; simp_all
    · have same : np p = x.g.promise p := by simp only [np,ended,Bool.false_eq_true,ite_false]
      have live : LivePromise x p := by simpa only [LivePromise,g,gDown,same] using need.1
      have cov : ∀ b, Covers ⟨s,g⟩ p b ↔ Covers x p b := by intro b; simp only [Covers,g,gDown,same]
      have savedCase : (x.g.promise p).saved = false → NeedsDraft c x p :=
        fun sv => ⟨live,Or.inl sv⟩
      by_cases saved : (x.g.promise p).saved = true
      · have missing : ¬Covers x p (nd p) := by
          have sv : (g.promise p).saved = true := by simpa only [g,same] using saved
          rcases need.2 with unsigned | missing | stable
          · simp_all
          · exact (fun covOld => missing ((cov _).mpr covOld))
          · exact (fun covOld => stable.2 ((cov _).mpr covOld))
        by_cases weak : c.weak = true
        · have kept : [nd p,(x.s.drafts p).bytes].contains (x.g.promise p).bytes = true := by
            have on := live.1
            simp only [weak,on,saved,Bool.true_and,Bool.not_eq_true] at ended
            cases hk : [nd p,(x.s.drafts p).bytes].contains (x.g.promise p).bytes <;> simp_all
          have choices : (x.g.promise p).bytes = nd p ∨ (x.g.promise p).bytes = (x.s.drafts p).bytes := by
            simpa using kept
          rcases choices with disk | draft
          · exact False.elim (missing (Or.inl disk.symm))
          · simpa only [s,g,down,same] using draft.symm
        · have nw : c.weak = false := by simpa using weak
          have oldneed : NeedsDraft c x p := by
            refine ⟨live,?_⟩
            cases hk : keep p
            · right; right; exact ⟨nw,by simpa [nd,hk] using missing⟩
            · right; left; simpa [nd,hk] using missing
          simpa only [s,g,down,same] using (h p oldneed).1
      · have ns : (x.g.promise p).saved = false := by simpa using saved
        simpa only [s,g,down,same] using (h p (savedCase ns)).1
  · exact postPromises_unheld_new x s g (fun _ => rfl)

end Storage21
