import PairSafety35
import GateOwnership11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def discardState11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) : State P :=
  lift x (Storage35Base.upDiscard c (project x) {(project x).s with up := tail} m ok)

theorem stable_discard11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (q : Fin P) (h : StableSide10 x q) :
    StableSide10 (discardState11 c x m tail ok) q := by
  simp only [discardState11,Storage35Base.upDiscard]
  split <;> (try split)
  · exact h
  · cases m.cur <;> exact h
  · cases m.cur <;>
      simp only [StableSide10,Storage34.lift,Storage32.commit,Storage3.commit,Storage21.commit,
        Storage32.discarded,ackMail,Bool.not_false,Bool.not_true,Bool.false_eq_true,
        Bool.true_eq,ite_false,ite_true,put]
    all_goals repeat split
    all_goals simp_all [StableSide10,project,Storage34.project,put]
    all_goals repeat split
    all_goals first | assumption | (solve | simp_all [put]) | grind

theorem discarded_source_stable11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (src : Fin P) (mutant : c.mutant = "none")
    (destHeld : (x.s.pages m.p).held = true) (ne : src ≠ m.p)
    (partnerSrc : partner x.s.to32 m.p = some src)
    (disk : x.s.disk src ≠ absent) (free : x.s.job.on = true → x.s.job.p ≠ src) :
    StableSide10 (discardState11 c x m tail true) src := by
  have partnerTail : partner {(project x).s with up := tail} m.p = some src := partnerSrc
  by_cases sh : (x.s.pages src).held = true <;> by_cases sb : (x.s.pages src).buf = absent
  all_goals cases hc : m.cur <;>
    simp [StableSide10,discardState11,Storage35Base.upDiscard,project,Storage34.project,
      Storage34.lift,Storage32.commit,Storage3.commit,Storage21.commit,Storage32.discarded,
      ackMail,partner_up11,partnerSrc,mutant,destHeld,ne,Ne.symm ne,sh,sb,put,disk,hc]
  all_goals intro on eq; exact False.elim (free on eq)

theorem pending_cancel_stable11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (src dst : Fin P) (mutant : c.mutant = "none")
    (own : GateOwnership11 x.s.gates) (pending : PendingPair10 x src dst) (same : m.p = dst) :
    StableSide10 (discardState11 c x m tail true) src := by
  obtain ⟨gt,gm,gs,gd,sm,dm⟩ := pending.2.2.2.2.2.2
  have ne : src ≠ dst := by simpa [gs,gd] using (own.1 gt gm).1
  have partnerSrc : partner x.s.to32 m.p = some src := by
    simpa [same,gs,gd] using partner_owned11 x.s.to32 gt gm own (by simpa [gd] using dm)
  exact discarded_source_stable11 c x m tail src mutant (by simpa [same] using pending.2.1)
    (by simpa [same] using ne) partnerSrc pending.1 pending.2.2.2.2.1

theorem pending_other_discard11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (src dst : Fin P) (own : GateOwnership11 x.s.gates)
    (pending : PendingPair10 x src dst) (ns : src ≠ m.p) (nd : dst ≠ m.p) :
    PendingPair10 (discardState11 c x m tail ok) src dst := by
  obtain ⟨gt,gm,gs,gd,sm,dm⟩ := pending.2.2.2.2.2.2
  have foreign : (partner x.s.to32 m.p).getD m.p ≠ dst := by
    cases hp : partner x.s.to32 m.p with
    | none => simpa using Ne.symm nd
    | some p =>
      simp only [Option.getD_some]
      intro eq
      subst p
      have owned := partner_into_owned11 x.s.to32 gt gm own m.p dst hp
        (by simpa [gd] using (own.1 gt gm).2.2.1)
      exact nd (gd.symm.trans owned.1.symm)
  have foreign' := Ne.symm foreign
  have dh := pending.2.1
  have db := pending.2.2.1
  have dr := pending.2.2.2.1
  simp only [discardState11,Storage35Base.upDiscard]
  split <;> (try split)
  · exact pending
  · cases m.cur <;> exact pending
  · let ng := {gt with pending := exclude gt.pending [m.p]}
    have nsm : src ∈ ng.pending := by simp [ng,exclude,sm,ns]
    have ndm : dst ∈ ng.pending := by simp [ng,exclude,dm,nd]
    have nonempty : ng.pending.isEmpty = false := by cases hp : ng.pending <;> simp_all
    have ngmem : ng ∈ (discarded x.s.to32 m.p).gates := by
      apply List.mem_filterMap.mpr
      refine ⟨gt,gm,?_⟩
      simp [discarded,ng,gd,nd,Ne.symm nd,nonempty]
    have page : ∀ y : Fin P → Page,
        (put y m.p (x.s.pages m.p)) dst = y dst := by
      intro y; simp [put,nd]
    cases hc : m.cur <;>
      refine ⟨pending.1,?_,?_,?_,pending.2.2.2.2.1,pending.2.2.2.2.2.1,ng,?_,gs,gd,nsm,ndm⟩
    all_goals first
      | (change ng ∈ (discarded x.s.to32 m.p).gates; exact ngmem)
      | (simpa only [Storage34.lift,Storage32.commit,Storage3.commit,Storage21.commit,Storage32.discarded] using ngmem)
      | (simp only [Storage34.lift,Storage32.commit,Storage3.commit,Storage21.commit,Storage32.discarded,
          ackMail,hc,Bool.not_false,Bool.not_true,Bool.false_eq_true,Bool.true_eq,ite_false,ite_true,
          project,Storage34.project,partner_up11];
         split <;> simp_all [put,project,Storage34.project])

theorem discard_requested_stable11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (disk : x.s.disk m.p ≠ absent)
    (job : x.s.job.on = true → x.s.job.p = m.p → x.s.job.bytes ≠ absent) :
    StableSide10 (discardState11 c x m tail true) m.p := by
  simp only [discardState11,Storage35Base.upDiscard]
  split
  · have off : (x.s.pages m.p).held = false := by
      simpa [project,Storage34.project] using (by assumption : (!((project x).s.pages m.p).held) = true)
    refine ⟨disk,?_,job⟩
    change (x.s.pages m.p).held = true → (x.s.pages m.p).buf ≠ absent
    simp [off]
  · simp only [Bool.not_true,Bool.false_eq_true,ite_false]
    cases hc : m.cur <;>
      simp only [StableSide10,Storage34.lift,Storage32.commit,Storage3.commit,Storage21.commit,
        Storage32.discarded,ackMail,hc,Bool.not_false,Bool.not_true,Bool.false_eq_true,Bool.true_eq,
        ite_false,ite_true,project,Storage34.project,partner_up11]
    all_goals refine ⟨disk,?_,job⟩
    all_goals split
    all_goals simp only [put]
    all_goals repeat split
    all_goals simp only [Bool.and_eq_true,bne_iff_ne] at *
    all_goals first | exact fun _ => disk | grind

theorem pairSafety_discard11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (mutant : c.mutant = "none") (own : GateOwnership11 x.s.gates)
    (h : PairSafety10 x) : PairSafety10 (discardState11 c x m tail ok) := by
  have pairs : (discardState11 c x m tail ok).g.renamed = x.g.renamed := by
    simp only [discardState11,Storage35Base.upDiscard]; split <;> (try split) <;> rfl
  intro r mem
  rw [pairs] at mem
  rcases h r mem with stable | stable | pending
  · exact Or.inl (stable_discard11 c x m tail ok r.1 stable)
  · exact Or.inr (Or.inl (stable_discard11 c x m tail ok r.2 stable))
  · cases hok : ok with
    | false =>
      right; right
      simp only [discardState11,Storage35Base.upDiscard,hok,Bool.not_false,ite_true]
      split <;> cases m.cur <;> exact pending
    | true =>
      by_cases dest : m.p = r.2
      · exact Or.inl (pending_cancel_stable11 c x m tail r.1 r.2 mutant own pending dest)
      by_cases source : m.p = r.1
      · left
        have disk : x.s.disk m.p ≠ absent := by simpa [source] using pending.1
        have job : x.s.job.on = true → x.s.job.p = m.p → x.s.job.bytes ≠ absent := by
          intro on eq; exact False.elim (pending.2.2.2.2.1 on (eq.trans source))
        simpa [source] using discard_requested_stable11 c x m tail disk job
      · right; right
        exact pending_other_discard11 c x m tail true r.1 r.2 own pending (Ne.symm source) (Ne.symm dest)
end Storage35
