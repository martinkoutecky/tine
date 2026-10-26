import Idle9
namespace Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem idle_closed_execute21_9 (c : Config P) (x : State P) (a : Action P) (h : IdleClosed c x) :
    IdleClosed c (execute c x a) := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals repeat (first | apply idle_closed_commit9 | split)
  case check => simp only [check]; split <;> apply idle_closed_commit9
  case draftSync p => apply idle_closed_commit9
  all_goals apply idle_closed_commit9
end Storage21
namespace Storage3
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem idle_closed_execute3_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : IdleClosed c (project x)) :
    IdleClosed c (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := idle_closed_execute21_9 c (project x) a h
    cases a <;> simp only [execute,executeOld]
    case rename => apply idle_closed_commit9
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opRename src dst refs rt => apply idle_closed_commit9
  | opDelete p => apply idle_closed_commit9
  | flushDel p => apply idle_closed_commit9
end Storage3
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem idle_closed_discard32_9 (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (ok : Bool) :
    IdleClosed c (Storage3.project (project (upDiscard c x sys m ok))) := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals apply idle_closed_commit9

theorem idle_closed_execute32_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : IdleClosed c (Storage3.project (project x))) :
    IdleClosed c (Storage3.project (project (execute c x a))) := by
  cases a with
  | load p => apply idle_closed_commit9
  | opRename src dst refs rt => apply idle_closed_commit9
  | opDelete p => apply idle_closed_commit9
  | flushDel p => apply idle_closed_commit9
  | old a =>
    have core := Storage3.idle_closed_execute3_9 c (project x) (.old a) mutant h
    cases a <;> simp only [execute]
    case deliverUp ok =>
      simp only [deliverUp]
      cases hq : x.s.up with
      | nil => exact h
      | cons m tail =>
        simp only [hq]
        split
        · apply idle_closed_discard32_9
        · exact core
    case dirSync ok => cases ok <;> simpa [dirSync,project,lift,mutant,Storage3.execute,Storage3.executeOld,discharge] using core
    all_goals first | exact core | simpa [check,rename,saveFail,project,lift,mutant,Storage3.execute,Storage3.executeOld] using core
end Storage32
namespace Storage34
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def IdleClosed (c : Config P) (x : State P) : Prop :=
  Storage21.IdleClosed c (Storage3.project (Storage32.project (project x)))
theorem idle_closed_execute34_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : IdleClosed c x) : IdleClosed c (execute c x a) := by
  cases a with
  | prior a => exact Storage32.idle_closed_execute32_9 c (project x) a mutant h
  | opApply eff plan dst src => apply idle_closed_commit9 (name := "opApply")
  | extClaim p present => apply idle_closed_commit9

theorem idle_closed_reachable34_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : IdleClosed c x := by
  induction hr with
  | initial =>
    refine ⟨rfl,?_⟩
    intro p on _; cases on
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact idle_closed_execute34_9 c x a mutant ih
    · contradiction

theorem extClaim_projects_idle9 (c : Config P) (x : State P) (p : Fin P) (present : Bool)
    (h : IdleClosed c x) : project (extClaim c x p present) = project x := by
  have eq := idle_commit9 c (Storage3.project (Storage32.project (project x))) h "extClaim"
  change Storage32.commit c (project x) x.s.to32 x.g "extClaim" = project x
  unfold Storage32.commit Storage3.commit
  change (let y := Storage21.commit c (Storage3.project (Storage32.project (project x))) (x.s.to32.toBase.toSys) (x.g.toBase.toGhost) "extClaim";
    ⟨⟨⟨y.s,x.s.trash,x.s.trashStable⟩,x.s.gates⟩,
      ⟨⟨y.g,x.g.opRead,x.g.removed,x.g.delDurable⟩,x.g.obl,x.g.renamed,x.g.orderOk⟩⟩) = project x
  simp only [Storage3.project,Storage32.project,project] at eq ⊢
  rw [eq]
#print axioms idle_closed_reachable34_9
#print axioms extClaim_projects_idle9
end Storage34
