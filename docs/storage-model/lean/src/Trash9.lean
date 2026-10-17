import Reduction9
namespace Storage32
open Storage21 Storage3
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem trash_discard32_9 (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (ok : Bool)
    (h : TrashInv ⟨sys.toBase,x.g.toBase⟩) : TrashInv (project (upDiscard c x sys m ok)) := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals exact h

theorem trash31_execute9 (c : Config P) (x : Storage31.State P) (a : Action P)
    (mutant : c.mutant = "none") (h : TrashInv x) :
    TrashInv (Storage31.execute c x a) := by
  cases a with
  | load p => exact h
  | old a => exact trash_execute c x (.old a) mutant h
  | opRename src dst refs rt => exact trash_execute c x (.opRename src dst refs rt) mutant h
  | opDelete p => exact trash_execute c x (.opDelete p) mutant h
  | flushDel p => exact trash_execute c x (.flushDel p) mutant h

theorem trash_noRestore9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : TrashInv (project x)) (nr : NoRestore x a) :
    TrashInv (project (execute c x a)) := by
  rw [execute_projects_noRestore9 c x a mutant nr]
  exact trash31_execute9 c (project x) a mutant h

theorem trash_execute32_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : TrashInv (project x)) :
    TrashInv (project (execute c x a)) := by
  cases a with
  | old a =>
    by_cases delivery : ∃ ok, a = .deliverUp ok
    · obtain ⟨ok,rfl⟩ := delivery
      simp only [execute,deliverUp]
      cases hq : x.s.up with
      | nil => exact h
      | cons m tail =>
        simp only [hq]
        split
        · exact trash_discard32_9 c x {x.s with up := tail} m ok h
        · exact trash_execute c (project x) (.old (.deliverUp ok)) mutant h
    · apply trash_noRestore9 c x (.old a) mutant h
      cases a <;> simp [NoRestore]
      case deliverUp ok => exact False.elim (delivery ⟨ok,rfl⟩)
  | opRename src dst refs rt => exact trash_noRestore9 c x (.opRename src dst refs rt) mutant h trivial
  | opDelete p => exact trash_noRestore9 c x (.opDelete p) mutant h trivial
  | flushDel p => exact trash_noRestore9 c x (.flushDel p) mutant h trivial
  | load p => exact trash_noRestore9 c x (.load p) mutant h trivial
end Storage32
namespace Storage33
open Storage21 Storage3
set_option maxHeartbeats 4000000

theorem trash_reachable33_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : TrashInv (Storage32.project x) := by
  induction hr with
  | initial => refine ⟨?_,?_,?_⟩ <;> simp [initWith,Storage32.initWith,Storage32.embed,Storage32.project,Storage31.initWith,Storage3.initWith,Storage3.embed]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      cases a with
      | prior a => exact Storage32.trash_execute32_9 c x a mutant ih
      | opApply eff plan dst src => exact ih
    · contradiction
end Storage33
namespace Storage34
open Storage21 Storage3
set_option maxHeartbeats 4000000

theorem trash_reachable34_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    TrashInv (Storage32.project (project x)) :=
  Storage33.trash_reachable33_9 c disk (project x) mutant (projects_reachable33_9 c disk x mutant hr)

theorem trashed_reachable34_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    Storage3.trashed (Storage32.project (project x)) = true := by
  have h := trash_reachable34_9 c disk x mutant hr
  apply List.all_eq_true.mpr
  intro p _
  apply List.all_eq_true.mpr
  intro b hb
  simp only [Bool.or_eq_true,List.contains_iff_mem]
  exact Or.inl (h.1 p b hb)
#print axioms trashed_reachable34_9
end Storage34
