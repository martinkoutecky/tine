import Apply8
import PartialGuarantee9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

-- The compound Discard names both changed pages, including the literal
-- unheld-source case. This lemma does not assume the full joint invariant.
theorem discard_bad9 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (held : (x.s.pages m.p).held = true) :
    (upDiscard c x {x.s with up := tail} m true).g.bad = x.g.bad := by
  simp only [upDiscard,held,Bool.not_true,Bool.false_eq_true,ite_false]
  have pe : partner {x.s with up := tail} m.p = partner x.s m.p := rfl
  simp only [pe]
  let src := (partner x.s m.p).getD m.p
  by_cases rev : (src != m.p && (x.s.pages src).buf == absent) = true
  · have revtrue : ((partner x.s m.p).getD m.p != m.p && (x.s.pages ((partner x.s m.p).getD m.p)).buf == absent) = true := rev
    simp only [revtrue,ite_true]
    apply Storage33.commit_named_bad8 (ps := [m.p,src])
    · intro p hp
      have neq : p ≠ m.p ∧ p ≠ src := by simpa using hp
      have ns : p ≠ (partner x.s m.p).getD m.p := neq.2
      simp [discarded,put,neq.1,ns]
    · intro p hp; simpa [src] using hp
    · cases hc : m.cur <;> simp [ackMail,hc,discarded]
    · rfl
  · have revfalse : ((partner x.s m.p).getD m.p != m.p && (x.s.pages ((partner x.s m.p).getD m.p)).buf == absent) = false := by simpa only [src,Bool.not_eq_true] using rev
    simp only [revfalse,Bool.false_eq_true,ite_false]
    apply Storage33.commit_named_bad8 (ps := [m.p])
    · intro p hp
      have neq : p ≠ m.p := by simpa using hp
      simp [discarded,put,neq]
    · intro p hp; simpa [src] using hp
    · cases hc : m.cur <;> simp [ackMail,hc,discarded]
    · rfl
#print axioms discard_bad9
end Storage32
