import FactorDiscard35
namespace Storage35Base
open Storage21 Storage32 Storage3
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def restores10 (c : Config P) (x : State P) (p : Fin P) : Bool :=
  let src := (partner x.s p).getD p
  src != p && ((x.s.pages src).held || c.mutant == "MRH") && (x.s.pages src).buf == absent

theorem no_restore_project10 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (nr : ok = true → (x.s.pages m.p).held = true → restores10 c x m.p = false) :
    Storage32.project (upDiscard c x {x.s with up := tail} m ok) =
      Storage3.lift (Storage32.project x)
        (Storage21.upDiscard c (Storage3.project (Storage32.project x))
          {x.s.toBase.toSys with up := tail} m ok) := by
  by_cases held : (x.s.pages m.p).held = true
  · cases ok with
    | false => simp [upDiscard,Storage32.project,Storage32.commit,Storage3.lift,Storage3.commit,Storage3.project,Storage21.upDiscard,held,Storage21.commit]
    | true =>
      have pe : partner {x.s with up := tail} m.p = partner x.s m.p := rfl
      have rev := nr rfl held
      simp only [restores10] at rev
      simp only [upDiscard,pe,rev,Bool.false_eq_true,ite_false]
      simp [Storage32.project,Storage32.commit,Storage3.lift,Storage3.commit,Storage3.project,Storage21.upDiscard,held,Storage21.commit,Storage32.discarded]
  · have hf : (x.s.pages m.p).held = false := by cases h : (x.s.pages m.p).held <;> simp_all
    simp [upDiscard,Storage32.project,Storage32.commit,Storage3.lift,Storage3.commit,Storage3.project,Storage21.upDiscard,hf,Storage21.commit]

theorem inv_execute_noApply10 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (noNone : NoNone6 c) (noUnknown : NoUnknown6 c)
    (h : Storage31.Inv c (Storage32.project x)) (en : Storage32.enabled c x a = true) :
    Storage31.Inv c (Storage32.project (execute c x a)) := by
  have oldEn := Storage32.enabled_projects9 c x a en
  have oldInv := Storage31.inv_execute c (Storage32.project x) a mutant noNone noUnknown h oldEn
  cases a with
  | old a =>
    cases a <;> simp only [execute]
    case deliverUp ok =>
      cases hq : x.s.up with
      | nil => simpa [deliverUp,hq] using h
      | cons m tail =>
        simp only [deliverUp,hq]
        split
        · have kind : m.kind = "discard" := by simpa using ‹(m.kind == "discard") = true›
          have ordinary : Storage31.execute c (Storage32.project x) (.old (.deliverUp ok)) =
              Storage3.lift (Storage32.project x)
                (Storage21.upDiscard c (Storage3.project (Storage32.project x))
                  {x.s.toBase.toSys with up := tail} m ok) := by
            simp [Storage31.execute,Storage31.baseline,Storage3.execute,Storage3.executeOld,mutant,
              Storage21.execute,Storage21.deliverUp,Storage32.project,Storage3.project,hq,kind]
          rw [ordinary] at oldInv
          by_cases go : ok = true ∧ (x.s.pages m.p).held = true ∧ restores10 c x m.p = true
          · rcases go with ⟨rfl,held,restores⟩
            let src := (partner x.s m.p).getD m.p
            have raw : src ≠ m.p ∧ (x.s.pages src).held = true ∧ (x.s.pages src).buf = absent := by
              have rr : (src ≠ m.p ∧ (x.s.pages src).held = true) ∧ (x.s.pages src).buf = absent := by
                simpa [restores10,src,mutant,Bool.and_eq_true] using restores
              exact ⟨rr.1.1,rr.1.2,rr.2⟩
            rw [factor_restore10 c x m tail src held raw.2.1 raw.1 rfl raw.2.2 mutant]
            apply Storage31.silent_inv10 c _ src mutant oldInv
            · simp only [Storage3.lift,Storage21.upDiscard,Storage32.project,Storage3.project,held,
                Bool.not_true,Bool.false_eq_true,ite_false,pages_commit,pages_ackMail,put,raw.1]
              exact raw.2.1
            · have sourceLocked : locked x.s.toBase.toSys src = false := by
                have guard := en
                simp only [Storage32.enabled,Bool.and_eq_true] at guard
                simp only [hq,kind,beq_self_eq_true,Bool.not_true,Bool.false_or] at guard
                have somePartner : partner x.s m.p = some src := by
                  cases hp : partner x.s m.p with
                  | none => simp [src,hp] at raw
                  | some s => simpa [src,hp]
                simpa [somePartner] using guard.2
              simpa [Storage3.lift,Storage21.upDiscard,Storage32.project,Storage3.project,held,
                Storage21.commit,locked,job_ackMail] using sourceLocked
          · have nr : ok = true → (x.s.pages m.p).held = true → restores10 c x m.p = false := by
              intro rok held
              have nottrue : restores10 c x m.p ≠ true := by intro ht; exact go ⟨rok,held,ht⟩
              cases hr : restores10 c x m.p <;> simp_all
            rw [no_restore_project10 c x m tail ok nr]
            exact oldInv
        · exact oldInv
    all_goals first
      | exact oldInv
      | (rw [Storage32.execute_projects_noRestore9 c x _ mutant (by trivial)]; exact oldInv)
  | opRename src dst refs rt => exact oldInv
  | opDelete p => exact oldInv
  | flushDel p => exact oldInv
  | load p => exact oldInv
end Storage35Base
namespace Storage35
open Storage21 Storage3
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
def NoApply : Action P → Prop
  | .opApply _ _ _ _ => False
  | _ => True
inductive ReachableNoApply (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableNoApply c disk (initWith c disk)
  | advance {x y} : ReachableNoApply c disk x → (a : Action P) →
      NoApply a → step c x a = some y → ReachableNoApply c disk y
theorem noApply_actual_reachable10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (hr : ReachableNoApply c disk x) : Reachable c disk x := by
  induction hr with
  | initial => exact .initial
  | advance hr a _ hs ih => exact .advance ih a hs

theorem noApply_inv10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p,RealByte c (disk p))
    (noNone : NoNone6 c) (noUnknown : NoUnknown6 c) (hr : ReachableNoApply c disk x) :
    Storage31.Inv c (Storage32.project (project x)) := by
  induction hr with
  | initial => exact Storage31.inv_initial c disk mutant realDisk noUnknown
  | @advance x y hr a na hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact Storage35Base.inv_execute_noApply10 c (project x) a mutant noNone noUnknown ih en
      | opApply eff plan dst src => exact False.elim na
      | extClaim p present =>
        change Storage31.Inv c (Storage32.project (Storage34.project (Storage34.extClaim c x p present)))
        rw [Storage34.extClaim_projects_idle9 c x p present
          (aux_reachable10 c disk x mutant (noApply_actual_reachable10 c disk x hr)).idle]
        exact ih
    · contradiction

theorem noApply_guarantee35_10 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : ReachableNoApply c (Storage31.initDisk c) x) : guarantee x = true :=
  Storage31.guarantee_of_inv7 c _ (noApply_inv10 c _ x mutant
    (positiveTexts_initial_real4 c T texts two) (positiveTexts_noNone6 c T texts)
    (positiveTexts_noUnknown6 c T texts) hr)
end Storage35
