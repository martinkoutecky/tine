import CoreBytes21
import CoreCustodySteps6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def PromiseBytes (c : Config P) (g : Ghost P) : Prop := ∀ p, (g.promise p).on = true → RealByte c (g.promise p).bytes

theorem promiseBytes_ack (c : Config P) (pr : Promise) (b v : Int) (sv : Bool) (ep : Int)
    (old : pr.on = true → RealByte c pr.bytes) (new : RealByte c b) :
    (ack pr b v sv ep).on = true → RealByte c (ack pr b v sv ep).bytes := by
  simp only [ack]; split
  · intro _; exact new
  · exact old

theorem promiseBytes_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : PromiseBytes c g) (buffers : ∀ p, RealByte c (s.pages p).buf) : PromiseBytes c (commit c x s g n).g := by
  intro p on
  change (PostPromises x s g p).on = true at on
  change RealByte c (PostPromises x s g p).bytes
  simp only [PostPromises] at on ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
  · exact promiseBytes_ack c _ _ _ _ _ (h p) (buffers p) on
  · exact h p on

theorem promiseBytes_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : PromiseBytes c x.g) (buffers : ∀ p, RealByte c ((upOpen c x s m ok).s.pages p).buf) :
    PromiseBytes c (upOpen c x s m ok).g := by
  simp only [upOpen] at buffers ⊢
  apply promiseBytes_commit
  · split <;> (try split) <;> exact h
  · simpa only [pages_commit] using buffers

theorem promiseBytes_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : PromiseBytes c x.g) (buffers : ∀ p, RealByte c ((upClose c x s m).s.pages p).buf) :
    PromiseBytes c (upClose c x s m).g := by
  simp only [upClose] at buffers ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
  all_goals apply promiseBytes_commit
  all_goals first | exact h | (simpa only [pages_commit] using buffers)

theorem promiseBytes_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : PromiseBytes c x.g) (buffers : ∀ p, RealByte c ((upSubmit c x s m).s.pages p).buf) :
    PromiseBytes c (upSubmit c x s m).g := by
  simp only [upSubmit] at buffers ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
  all_goals apply promiseBytes_commit
  all_goals first | exact h | (simpa only [pages_commit] using buffers)

theorem promiseBytes_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : PromiseBytes c x.g) (buffers : ∀ p, RealByte c ((upDiscard c x s m ok).s.pages p).buf) :
    PromiseBytes c (upDiscard c x s m ok).g := by
  simp only [upDiscard] at buffers ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
  · apply promiseBytes_commit
    · exact h
    · simpa only [pages_commit] using buffers
  · split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
    · apply promiseBytes_commit
      · exact h
      · simpa only [pages_commit] using buffers
    · apply promiseBytes_commit
      · intro p on; by_cases eq : p = m.p
        · subst p; simp only [put,ite_true] at on ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
          · exact h _ on
          · simp [endPromise] at on
        · simpa [put,eq] using h p (by simpa [put,eq] using on)
      · simpa only [pages_commit] using buffers

theorem promiseBytes_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : PromiseBytes c x.g) (t2 : RealByte c m.t2)
    (buffers : ∀ p, RealByte c ((upOp c x s m ok).s.pages p).buf) : PromiseBytes c (upOp c x s m ok).g := by
  simp only [upOp] at buffers ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
  · apply promiseBytes_commit
    · exact h
    · simpa only [pages_commit] using buffers
  · split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
    · apply promiseBytes_commit
      · exact h
      · simpa only [pages_commit] using buffers
    · apply promiseBytes_commit
      · intro p on; by_cases eq : p = m.q
        · subst p; simp only [put,ite_true] at on ⊢; exact promiseBytes_ack c _ _ _ _ _ (h _) t2 on
        · simpa [put,eq] using h p (by simpa [put,eq] using on)
      · simpa only [pages_commit] using buffers

theorem promiseBytes_execute (c : Config P) (mutant : c.mutant = "none") (x : State P) (a : Action P)
    (h : PromiseBytes c x.g) (bytes : ByteDomain c x.s) (hen : enabled c x a = true) :
    PromiseBytes c (execute c x a).g := by
  have new := bytes_execute c x a bytes hen
  cases a <;> simp only [execute] at new ⊢
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [deliverUp] at new ⊢
      split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
      · exact promiseBytes_upOpen c x _ m ok h new.pages
      · split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
        · exact promiseBytes_upClose c x _ m h new.pages
        · split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
          · exact promiseBytes_upDiscard c x _ m ok h new.pages
          · split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
            · apply promiseBytes_upOp c x _ m ok h _ new.pages
              exact (bytes.up m (by simp [hq])).2.2 (by simpa using (by assumption : (m.kind == "op") = true))
            · exact promiseBytes_upSubmit c x _ m h new.pages
  case dirSync ok =>
    have job : RealByte c x.s.job.bytes := bytes.job (by simp [enabled] at hen; grind)
    simp only [dirSync] at new ⊢
    apply promiseBytes_commit
    · cases ok
      · exact h
      · intro p on; simp only [ite_true,put] at on ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
        · exact promiseBytes_ack c _ _ _ _ _ (h _) job on
        · exact h p on
    · simpa only [pages_commit] using new.pages
  case draftSync p =>
    simp only [draftSync,mutant,show ("none" == "MBD") = false from rfl,Bool.false_and,Bool.false_eq_true,ite_false] at new ⊢
    split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
    · apply promiseBytes_commit
      · have rb : RealByte c (draftEntry (x.s.pages p)).bytes := by
          rcases draft_real c (x.s.pages p) (bytes.pages p) with no | real
          · simp_all
          · exact real
        intro q on; by_cases eq : q = p
        · subst q; simp only [put,ite_true] at on ⊢; exact promiseBytes_ack c _ _ _ _ _ (h _) rb on
        · simpa [put,eq] using h q (by simpa [put,eq] using on)
      · simpa only [pages_commit] using new.pages
    · exact promiseBytes_commit c x _ _ _ h (by simpa only [pages_commit] using new.pages)
  case power keep =>
    apply promiseBytes_commit
    · intro p on; dsimp only at on ⊢
      cases hk : keep p
      all_goals simp only [hk,Bool.false_eq_true,ite_true,ite_false] at on ⊢
      all_goals split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
      all_goals first | exact h p on | (simp [endPromise] at on)
    · exact fun _ => real_absent c
  case check =>
    simp only [check] at new ⊢; split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false]
    all_goals apply promiseBytes_commit
    all_goals first | exact h | (simpa only [pages_commit] using new.pages)
  all_goals apply promiseBytes_commit c x
  all_goals first | exact h | skip
  all_goals simpa only [wOpen,wEdit,wSend,wResolve,wDiscard,wOp,wClose,wRecv,observe,flush,rename,
    saveFail,switchReq,switchFin,extWriteD,crash,windowCrash,launch,pages_commit] using new.pages

theorem promiseBytes_reachable (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : Reachable c x) : PromiseBytes c x.g := by
  induction hx with
  | initial => simp [PromiseBytes,init]
  | @advance x y hr a hs ih =>
    have bytes := bytes_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact promiseBytes_execute c mutant x a ih bytes (by assumption)
    · contradiction
#print axioms promiseBytes_reachable
end Storage21
