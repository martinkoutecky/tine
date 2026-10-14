import CoreEvolution6
import CoreLineage21
import CoreDistinct6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def ActiveAgreement (s : Sys P) (g : Ghost P) : Prop := ∀ p,
  (s.pages p).held = true → (g.promise p).on = true →
  (g.promise p).ver = (s.pages p).ver → (g.promise p).bytes = (s.pages p).buf

theorem active_evolution (x : State P) (s : Sys P) (h : ActiveAgreement x.s x.g)
    (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) (ev : PageEvolution x s) :
    ActiveAgreement s x.g := by
  intro p held on eq
  rcases ev p held with old | fresh
  · exact (h p old.1 on (eq.trans old.2.1)).trans old.2.2.symm
  · have b := bound p; omega

theorem active_ack (s : Sys P) (g : Ghost P) (p : Fin P) (sv : Bool) (ep : Int)
    (h : ActiveAgreement s g) : ActiveAgreement s
    {g with promise := put g.promise p (ack (g.promise p) (s.pages p).buf (s.pages p).ver sv ep)} := by
  intro q held on eq
  by_cases same : q = p
  · subst q; simp only [put,ite_true,ack] at on eq ⊢; split <;> (try simp_all only [ite_true,ite_false])
    all_goals first | rfl | exact h p held on eq
  · simpa only [put,same,ite_false] using h q held (by simpa [put,same] using on) (by simpa [put,same] using eq)

theorem active_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : ActiveAgreement s g) : ActiveAgreement (commit c x s g n).s (commit c x s g n).g := by
  intro p held on eq
  change (PostPromises x s g p).on = true at on
  change (PostPromises x s g p).ver = (s.pages p).ver at eq
  change (PostPromises x s g p).bytes = (s.pages p).buf
  simp only [PostPromises,ack] at on eq ⊢
  split <;> (try simp_all only [ite_true,ite_false])
  all_goals (try split) <;> (try simp_all only [ite_true,ite_false])
  all_goals first | rfl | exact h p held on eq

theorem active_ackMail (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool)
    (h : ActiveAgreement s g) : ActiveAgreement (ackMail s p ok v t) g := by cases ok <;> exact h

theorem active_upOpen (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : ActiveAgreement x.s x.g) (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) :
    ActiveAgreement (upOpen c x {x.s with up := tail} m ok).s
      (upOpen c x {x.s with up := tail} m ok).g := by
  have ev := evolution_upOpen c x {x.s with up := tail} m ok (evolution_refl x)
  simp only [upOpen,evolution_commit] at ev ⊢
  apply active_commit
  split <;> (try simp_all only [ite_true,ite_false])
  all_goals (try split) <;> (try simp_all only [ite_true,ite_false])
  all_goals exact active_evolution x _ h bound ev

theorem active_upClose (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (h : ActiveAgreement x.s x.g) (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) :
    ActiveAgreement (upClose c x {x.s with up := tail} m).s
      (upClose c x {x.s with up := tail} m).g := by
  have ev := evolution_upClose c x {x.s with up := tail} m (evolution_refl x)
  simp only [upClose,evolution_commit] at ev ⊢; split <;> (try simp_all only [ite_true,ite_false])
  all_goals apply active_commit
  all_goals exact active_evolution x _ h bound ev

theorem active_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P))
    (h : ActiveAgreement x.s x.g) (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) :
    ActiveAgreement (upSubmit c x {x.s with up := tail} m).s
      (upSubmit c x {x.s with up := tail} m).g := by
  have ev := evolution_upSubmit c mutant x {x.s with up := tail} m (evolution_refl x)
  simp only [upSubmit,evolution_commit] at ev ⊢; split <;> (try simp_all only [ite_true,ite_false])
  all_goals apply active_commit
  all_goals exact active_evolution x _ h bound ev

theorem active_upDiscard (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : ActiveAgreement x.s x.g) (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) :
    ActiveAgreement (upDiscard c x {x.s with up := tail} m ok).s
      (upDiscard c x {x.s with up := tail} m ok).g := by
  have ev := evolution_upDiscard c x {x.s with up := tail} m ok (evolution_refl x)
  simp only [upDiscard,evolution_commit] at ev ⊢; split <;> (try simp_all only [ite_true,ite_false])
  · apply active_commit; exact active_evolution x _ h bound ev
  · split <;> (try simp_all only [ite_true,ite_false])
    · apply active_commit; exact active_evolution x _ h bound ev
    · apply active_commit
      simp only [Bool.false_eq_true,ite_false,evolution_commit] at ev
      have hh := active_evolution x _ h bound ev
      intro p held on eq
      by_cases same : p = m.p
      · subst p
        simp only [put,ite_true] at on eq ⊢
        split <;> (try simp_all only [ite_true,ite_false])
        · exact hh m.p held on eq
        · simp [endPromise] at on
      · simpa only [put,same,ite_false] using hh p held (by simpa only [put,same,ite_false] using on) (by simpa only [put,same,ite_false] using eq)

theorem active_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : ActiveAgreement x.s x.g) (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) (ne : m.p ≠ m.q) :
    ActiveAgreement (upOp c x {x.s with up := tail} m ok).s
      (upOp c x {x.s with up := tail} m ok).g := by
  have ev := evolution_upOp c mutant x {x.s with up := tail} m ok (evolution_refl x)
  simp only [upOp,evolution_commit] at ev ⊢; split <;> (try simp_all only [ite_true,ite_false])
  · apply active_commit; exact active_evolution x _ h bound ev
  · split <;> (try simp_all only [ite_true,ite_false])
    · apply active_commit; exact active_evolution x _ h bound ev
    · apply active_commit
      simp only [Bool.false_eq_true,ite_false,evolution_commit] at ev
      have hh := active_evolution x _ h bound ev
      intro p held on eq
      by_cases same : p = m.q
      · subst p
        have fresh : (x.g.promise m.q).ver < x.g.vc+1 := by have b := bound m.q; omega
        simp only [put,ite_true,ack,show (x.g.vc+1 > (x.g.promise m.q).ver) = true from by simpa using fresh,
          Bool.true_or,ite_true]
        simp only [pages_ackMail,put,Ne.symm ne,ite_false,ite_true]
        change m.t2 = (submitTo c (x.s.pages m.q) m.t2 m.bv2 (x.g.vc+1)).buf
        simp [submitTo,mutant]; split <;> rfl
      · simpa only [put,same,ite_false] using hh p held (by simpa only [put,same,ite_false] using on) (by simpa only [put,same,ite_false] using eq)

theorem active_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (h : ActiveAgreement x.s x.g)
    (bound : ∀ p, (x.g.promise p).ver ≤ x.g.vc) (snap : JobSnapshot x.s)
    (distinct : QueueDistinct x.s) (hen : enabled c x a = true) :
    ActiveAgreement (execute c x a).s (execute c x a).g := by
  have ev := evolution_execute c mutant x a
  cases a <;> simp only [execute] at ev ⊢
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [deliverUp]
      split
      · exact active_upOpen c x m tail ok h bound
      · split
        · exact active_upClose c x m tail h bound
        · split
          · exact active_upDiscard c x m tail ok h bound
          · split
            · exact active_upOp c mutant x m tail ok h bound
                (distinct m (by simp [hq]) (by simpa using (by assumption : (m.kind == "op") = true)))
            · exact active_upSubmit c mutant x m tail h bound
  case dirSync ok =>
    have on : x.s.job.on = true := by simp [enabled] at hen; grind
    have sn := snap on
    simp only [dirSync] at ev ⊢
    cases ok
    · simp only [Bool.false_eq_true,ite_false] at ev ⊢
      apply active_commit; exact active_evolution x _ h bound ev
    · simp only [ite_true,Bool.not_true,Bool.false_and,Bool.false_eq_true,ite_false] at ev ⊢
      simp only [evolution_commit] at ev
      apply active_commit
      have hh := active_evolution x _ h bound ev
      intro p held on eq
      by_cases same : p = x.s.job.p
      · subst p
        simp only [put,ite_true,ack] at on eq ⊢
        split <;> (try simp_all only [ite_true,ite_false])
        · change x.s.job.bytes = (onReply c (x.s.pages x.s.job.p) "Published" x.s.job.bytes).buf
          simp only [onReply]; split <;> exact sn.2.1.symm
        · simp only [Bool.false_eq_true,ite_false] at eq
          simpa only [put,ite_true] using hh x.s.job.p held on (by simpa only [put,ite_true] using eq)
      · simpa only [put,same,ite_false] using hh p held
          (by simpa only [put,same,ite_false] using on) (by simpa only [put,same,ite_false] using eq)
  case draftSync p =>
    simp only [draftSync,mutant,show ("none" == "MBD") = false from rfl,
      Bool.false_and,Bool.false_eq_true,ite_false] at ev ⊢
    split <;> (try simp_all only [ite_true,ite_false])
    · apply active_commit
      have hh := active_evolution x _ h bound ev
      have flags : ((x.s.pages p).held && (x.s.pages p).risk) = true := by
        by_cases yes : ((x.s.pages p).held && (x.s.pages p).risk) = true
        · exact yes
        · simp [draftEntry,yes] at *
      have de : draftEntry (x.s.pages p) = ⟨(x.s.pages p).buf,(x.s.pages p).base,(x.s.pages p).ver⟩ := by
        simp [draftEntry,flags]
      simpa only [ActiveAgreement,pages_commit,de] using active_ack _ x.g p false 0 hh
    · apply active_commit; exact active_evolution x _ h bound ev
  case power keep => simp [ActiveAgreement,power,down,pages_commit]
  case check =>
    simp only [check] at ev ⊢; split <;> (try simp_all only [ite_true,ite_false])
    all_goals apply active_commit
    all_goals exact active_evolution x _ h bound ev
  all_goals apply active_commit
  all_goals exact active_evolution x _ h bound ev

theorem activeAgreement_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : ActiveAgreement x.s x.g := by
  induction hx with
  | initial => simp [ActiveAgreement,init]
  | @advance x y hr a hs ih =>
    have bound := (lineage_reachable c mutant x hr).1
    have snap := snapshot_reachable c mutant x hr
    have distinct := distinct_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact active_execute c mutant x a ih bound snap distinct (by assumption)
    · contradiction

#print axioms activeAgreement_reachable
end Storage21
