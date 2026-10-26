import PairOld11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def LivePairs11 (x : State P) : Prop := x.s.alive = false → x.g.renamed = []

theorem alive_old11 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.prior (.old a))).s.alive = Storage21.aliveResult (core11 x) a := by
  cases a <;> simp only [execute,Storage35Base.execute,Storage32.execute]
  case deliverUp ok =>
    cases hq : (project x).s.up with
    | nil =>
      have hq' : x.s.up = [] := hq
      simp [Storage35Base.deliverUp,hq',Storage21.aliveResult,core11,project,Storage34.project,Storage34.lift,Storage32.project,Storage3.project]
    | cons m tail =>
      simp only [Storage35Base.deliverUp,hq]
      split
      · simp only [Storage35Base.upDiscard]; split <;> (try split)
        all_goals cases m.cur <;> rfl
      · have core := Storage21.alive_execute c (core11 x) (.deliverUp ok)
        simpa [Storage32.lift,Storage3.executeOld,Storage3.lift,mutant,Storage21.execute,
          core11,project,Storage34.project,Storage34.lift,Storage32.project,Storage3.project,hq] using core
  case rename => simp [Storage32.rename,Storage32.lift,mutant,Storage3.rename,Storage3.commit,
      Storage21.commit,Storage21.aliveResult,core11,project,Storage34.project,Storage34.lift,Storage32.project,Storage3.project]
  case dirSync ok =>
    cases ok <;> simp [Storage32.dirSync,Storage32.lift,mutant,Storage3.dirSync,Storage3.lift,
      Storage21.dirSync,Storage21.commit,Storage32.discharge,Storage21.aliveResult,
      core11,project,Storage34.project,Storage34.lift,Storage32.project,Storage3.project]
  case check =>
    simp only [Storage32.check,mutant,show ("none" == "MCV") = false from rfl,
      Bool.false_and,Bool.false_eq_true,ite_false]
    exact Storage21.alive_execute c (core11 x) .check
  case saveFail =>
    simp [Storage32.saveFail,Storage32.lift,mutant,Storage3.executeOld,Storage3.lift,
      Storage21.execute,Storage21.saveFail,Storage21.commit,Storage21.aliveResult,
      core11,project,Storage34.project,Storage34.lift,Storage32.project,Storage3.project]
  case flush p => rfl
  case power keep => rfl
  all_goals exact Storage21.alive_execute c (core11 x) _

theorem livePairs_execute11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : LivePairs11 x) (en : enabled c x a = true) :
    LivePairs11 (execute c x a) := by
  cases a with
  | extClaim p present => exact h
  | opApply eff plan dst src =>
    have live : x.s.alive = true := by
      simp [enabled,Storage34.enabled,Storage34.enabledApply,Storage33.enabledApply,project,Storage34.project] at en; grind
    intro off; exact False.elim (Bool.noConfusion (live.symm.trans off))
  | prior a =>
    cases a with
    | load p =>
      have live : x.s.alive = true := by simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,project,Storage34.project,Storage32.project] at en; grind
      intro off; exact False.elim (Bool.noConfusion (live.symm.trans off))
    | flushDel p =>
      have live : x.s.alive = true := by simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,project,Storage34.project,Storage32.project] at en; grind
      intro off; exact False.elim (Bool.noConfusion (live.symm.trans off))
    | opDelete p =>
      have live : x.s.alive = true := by simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,project,Storage34.project,Storage32.project] at en; grind
      intro off; exact False.elim (Bool.noConfusion (live.symm.trans off))
    | opRename src dst refs rt =>
      have live : x.s.alive = true := by simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,project,Storage34.project,Storage32.project] at en; grind
      intro off; exact False.elim (Bool.noConfusion (live.symm.trans off))
    | old a =>
      have alive := alive_old11 c x a mutant
      by_cases stop : a = .crash ∨ a = .switchFin ∨ ∃ keep, a = .power keep
      · rcases stop with rfl | rfl | ⟨keep,rfl⟩
        all_goals intro _; rfl
      by_cases launch : a = .launch
      · subst a
        intro off; rw [alive] at off; cases off
      by_cases ext : ∃ p b dur, a = .extWriteD p b dur
      · obtain ⟨p,b,dur,rfl⟩ := ext
        intro off
        have oldoff : x.s.alive = false := off
        change unrename x.g.renamed [p] = []
        simp [h oldoff,unrename]
      have coreEn : Storage21.enabled c (core11 x) a = true :=
        Storage32.enabled_projects9 c (project x) (.old a) en
      have oldlive : x.s.alive = true := by
        cases a <;> simp_all [Storage21.enabled,Storage21.canSwitch,core11,project,Storage34.project,Storage32.project,Storage3.project]
      have resultlive : Storage21.aliveResult (core11 x) a = true := by
        cases a <;> simp_all [Storage21.aliveResult,core11,project,Storage34.project,Storage32.project,Storage3.project]
      intro off
      rw [alive,resultlive] at off
      cases off

theorem livePairs_reachable35_11 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : LivePairs11 x := by
  induction hr with
  | initial => intro _; rfl
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      exact livePairs_execute11 c x a mutant ih (by assumption)
    · contradiction
end Storage35
