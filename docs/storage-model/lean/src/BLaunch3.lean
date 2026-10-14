import Buffer3
import CoreInactive21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem inactive_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : InactivePages (project x))
    (hen : enabled c x a = true) : InactivePages (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := inactive_execute c (project x) a h hen
    cases a <;> simp only [execute,executeOld]
    case rename => exact core
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p =>
    have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp [InactivePages,execute,opDelete,commit,project,Storage21.commit,live]
  | opRename src dst refs rt =>
    have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp [InactivePages,execute,opRename,commit,project,Storage21.commit,live]
  | flushDel p =>
    have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp [InactivePages,execute,flushDel,lift,project,Storage21.flush,Storage21.commit,live]

theorem inactive_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : InactivePages (project x) := by
  induction hr with
  | initial => simp [InactivePages,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact inactive_execute3 c x a mutant ih (by assumption)
    · contradiction

theorem B_launch3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x)
    (h : BInv (project x)) (hen : enabled c x (.old .launch) = true) :
    BInv (project (execute c x (.old .launch))) :=
  B_launch c (project x) h (inactive_reachable3 c disk x mutant hr) hen

def BResidualAction4 (a : Action P) : Prop :=
  (∃ p, a = .old (.observe p)) ∨ (∃ ok, a = .old (.deliverUp ok)) ∨
    a = .old .check ∨ a = .old .switchFin

theorem B_reachable3_of_residual4 (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none")
    (residual : ∀ x, Reachable c disk x → BInv (project x) → ∀ a,
      enabled c x a = true → BResidualAction4 a → BInv (project (execute c x a)))
    (x : State P) (hr : Reachable c disk x) : "B-unsaved-replaced" ∉ x.g.bad := by
  apply B_reachable3_of_residual c disk mutant ?_ x hr
  intro y hy hb a hen which
  by_cases launch : a = .old .launch
  · subst a; exact B_launch3 c disk y mutant hy hb hen
  · apply residual y hy hb a hen
    simpa [BResidualAction,BResidualAction4,launch] using which
end Storage3
