import Storage32
import Guarantee31
namespace Storage32
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
-- Only the additional source restoration is excluded. Full moves, ordinary
-- Discards, failed reads and cancellation keeping newer source input remain.
def NoRestore (x : State P) : Action P → Prop
  | .old (.deliverUp ok) => ∀ m tail, x.s.up = m::tail → m.kind = "discard" →
      ok = true → (x.s.pages m.p).held = true →
      let src := (partner x.s m.p).getD m.p
      src = m.p ∨ (x.s.pages src).buf ≠ absent
  | _ => True

theorem discard_project_noRestore9 (c : Config P) (x : State P) (sys : Sys P)
    (m : Up P) (rok : Bool) (ht : sys.trash = x.s.trash) (hs : sys.trashStable = x.s.trashStable)
    (hp : sys.pages = x.s.pages) (partnerEq : partner sys m.p = partner x.s m.p)
    (nr : rok = true → (x.s.pages m.p).held = true →
      let src := (partner x.s m.p).getD m.p
      src = m.p ∨ (x.s.pages src).buf ≠ absent) :
    project (upDiscard c x sys m rok) =
      Storage3.lift (project x) (Storage21.upDiscard c (Storage3.project (project x)) sys.toBase.toSys m rok) := by
  by_cases held : (x.s.pages m.p).held = true
  · cases rok with
    | false => simp [upDiscard,project,commit,Storage3.lift,Storage3.commit,Storage3.project,Storage21.upDiscard,hp,held,Storage21.commit,ht,hs]
    | true =>
      have rev : (((partner sys m.p).getD m.p != m.p) && (sys.pages ((partner sys m.p).getD m.p)).buf == absent) = false := by
        rcases nr rfl held with same | newer
        · simp [partnerEq,same]
        · simp [partnerEq,hp,newer]
      simp only [upDiscard,rev,Bool.false_eq_true,ite_false]
      simp [project,commit,Storage3.lift,Storage3.commit,Storage3.project,Storage21.upDiscard,hp,held,Storage21.commit,ht,hs,discarded]
  · have hf : (x.s.pages m.p).held = false := by cases h : (x.s.pages m.p).held <;> simp_all
    simp [upDiscard,project,commit,Storage3.lift,Storage3.commit,Storage3.project,Storage21.upDiscard,hp,hf,Storage21.commit,ht,hs]

theorem enabled_projects9 (c : Config P) (x : State P) (a : Action P)
    (en : enabled c x a = true) : Storage31.enabled c (project x) a = true := by
  simp only [enabled,Bool.and_eq_true] at en
  exact en.1

theorem execute_projects_noRestore9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (nr : NoRestore x a) :
    project (execute c x a) = Storage31.execute c (project x) a := by
  cases a with
  | opRename src dst refs rt => rfl
  | opDelete p => rfl
  | load p => rfl
  | flushDel p => rfl
  | old a =>
    cases a <;> simp only [execute,Storage31.execute,Storage31.baseline,Storage3.execute]
    case flush p => rfl
    case rename => simp [rename,project,lift,mutant,Storage3.executeOld]
    case dirSync ok => cases ok <;> simp [dirSync,project,lift,discharge,mutant,Storage3.executeOld]
    case check => simp [check,project,lift,mutant]
    case saveFail => simp [saveFail,project,lift,mutant]
    case crash => rfl
    case switchFin => rfl
    case power keep => rfl
    case extWriteD p v dur => rfl
    case deliverUp ok =>
      cases h : x.s.up with
      | nil => simp [deliverUp,project,Storage3.executeOld,Storage21.execute,Storage3.project,h,mutant,Storage3.lift]
      | cons m tail =>
        simp only [deliverUp,h]
        split
        · have mk : m.kind = "discard" := by simpa using ‹(m.kind == "discard") = true›
          rw [discard_project_noRestore9 c x {x.s with up := tail} m ok rfl rfl rfl rfl
            (by exact nr m tail h mk)]
          simp [Storage3.executeOld,Storage21.execute,Storage21.deliverUp,Storage3.project,project,h,mutant,mk]
        · rfl
    all_goals rfl

inductive ReachableNoRestore (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableNoRestore c disk (initWith c disk)
  | advance {x y} : ReachableNoRestore c disk x → (a : Action P) →
      NoRestore x a → step c x a = some y → ReachableNoRestore c disk y

theorem noRestore_actual_reachable9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (hr : ReachableNoRestore c disk x) : Reachable c disk x := by
  induction hr with
  | initial => exact .initial
  | advance hr a _ hs ih => exact .advance ih a hs

theorem noRestore_refines31_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableNoRestore c disk x) :
    Storage31.Reachable c disk (project x) := by
  induction hr with
  | initial => exact .initial
  | @advance x y hr a nr hs ih =>
    unfold step at hs
    split at hs
    · cases hs
      rw [execute_projects_noRestore9 c x a mutant nr]
      exact .advance ih a (by simp [Storage31.step,enabled_projects9 c x a ‹enabled c x a = true›])
    · contradiction

theorem noRestore_guarantee9 (c : Config P) (T : Nat)
    (texts : c.texts = Storage3.positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : ReachableNoRestore c (Storage31.initDisk c) x) : guarantee x = true :=
  Storage31.guarantee_positiveTexts7 c T texts two mutant (project x) (noRestore_refines31_9 c _ x mutant hr)
#print axioms noRestore_guarantee9
end Storage32
