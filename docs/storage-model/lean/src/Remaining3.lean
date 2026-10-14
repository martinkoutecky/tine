import Safety3
import Trash3
import Protocol3
import CoreUnheld21
import CoreVocabulary21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem unheld_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : "request-to-unheld" ∉ x.g.bad)
    (life : ProtocolInv3 x) : "request-to-unheld" ∉ (execute c x a).g.bad := by
  cases a with
  | old a =>
    have core := unheld_execute c (project x) a h life.life life.kinds
    cases a <;> simp only [execute,executeOld]
    case rename => simpa [Storage21.execute,rename,Storage21.rename,commit,project,Storage21.commit] using core
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => simpa [execute,opDelete,commit,project,Storage21.commit] using h
  | opRename src dst refs rt => simpa [execute,opRename,commit,project,Storage21.commit] using h
  | flushDel p => simpa [execute,flushDel,lift,project,Storage21.flush,Storage21.commit] using h

theorem unheld_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : "request-to-unheld" ∉ x.g.bad := by
  induction hr with
  | initial => simp [initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact unheld_execute3 c x a mutant ih (protocol_reachable3 c disk x mutant hr)
    · contradiction

theorem vocabulary_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : BadVocabulary x.g.toGhost) :
    BadVocabulary (execute c x a).g.toGhost := by
  cases a with
  | old a =>
    have core := vocabulary_execute c (project x) a h
    cases a <;> simp only [execute,executeOld]
    case rename =>
      simp only [rename,commit]
      apply vocabulary_commit
      intro tag ht
      rcases (mem_union_general _ _ _).mp ht with first | last
      · rcases (mem_union_general _ _ _).mp first with old | fresh
        · exact h tag old
        · exact vocabulary_tags _ _ (by simp [BadKinds]) tag fresh
      · exact vocabulary_tags _ _ (by simp [BadKinds]) tag last
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p =>
    simp only [execute,opDelete,commit]
    apply vocabulary_commit
    exact h
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit]
    apply vocabulary_commit
    intro tag ht
    rcases (mem_union_general _ _ _).mp ht with first | last
    · rcases (mem_union_general _ _ _).mp first with old | fresh
      · exact h tag old
      · exact vocabulary_tags _ _ (by simp [BadKinds]) tag fresh
    · exact vocabulary_tags _ _ (by simp [BadKinds]) tag last
  | flushDel p =>
    simp only [execute,flushDel,lift,project,Storage21.flush]
    exact vocabulary_commit c (project x) _ _ _ h

theorem vocabulary_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : BadVocabulary x.g.toGhost := by
  induction hr with
  | initial => simp [BadVocabulary,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact vocabulary_execute3 c x a mutant ih
    · contradiction

theorem transitions_remaining3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    transitions x = true ↔ "B-unsaved-replaced" ∉ x.g.bad ∧ "G-overwrote-unseen" ∉ x.g.bad := by
  constructor
  · intro ht
    have nil : x.g.bad = [] := by simpa [transitions,Storage21.transitions,project] using ht
    simp [nil]
  · rintro ⟨b,g⟩
    have voc := vocabulary_reachable3 c disk x mutant hr
    have cc := C_absent_reachable c disk x mutant hr
    have w := Bprime1_reachable3 c disk x mutant hr
    have tk := Bprime2_reachable3 c disk x mutant hr
    have un := unheld_reachable3 c disk x mutant hr
    obtain ⟨d4,dst⟩ := newTags_reachable c disk x mutant hr
    have nil : x.g.bad = [] := by
      apply List.eq_nil_iff_forall_not_mem.mpr
      intro t mem
      have kind := voc t mem
      simp [BadKinds] at kind
      rcases kind with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl <;> contradiction
    simp [transitions,Storage21.transitions,project,nil]

theorem guarantee_remaining3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    guarantee x = true ↔ noLoss x = true ∧ "B-unsaved-replaced" ∉ x.g.bad ∧ "G-overwrote-unseen" ∉ x.g.bad := by
  simp only [guarantee,accepted_reachable3 c disk x mutant hr,trashed_reachable c disk x mutant hr,
    Bool.and_true,Bool.and_eq_true,transitions_remaining3 c disk x mutant hr]

-- This characterizes the remaining A obligations, rather than assuming custody.
def AResidualAction (a : Action P) : Prop :=
  a = .old .crash ∨ a = .old .switchFin ∨ (∃ keep, a = .old (.power keep)) ∨
    (∃ p t dur, a = .old (.extWriteD p t dur))

theorem noLoss_nonresidual3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (hen : enabled c x a = true)
    (excluded : ¬ AResidualAction a) : noLoss (execute c x a) = true := by
  have live : (execute c x a).s.alive = true := by
    cases a with
    | old a =>
      have nr : a ≠ .crash ∧ a ≠ .switchFin ∧ (∀ k, a ≠ .power k) ∧ (∀ p t d, a ≠ .extWriteD p t d) := by
        simpa [AResidualAction] using excluded
      have core : (Storage21.execute c (project x) a).s.alive = true := by
        rw [alive_execute]
        cases a <;> simp only [aliveResult]
        all_goals first
          | (exfalso; grind)
          | (simp [enabled,Storage21.enabled,project] at hen ⊢; grind)
          | rfl
      cases a <;> simp only [execute,executeOld]
      case rename => exact core
      case deliverUp ok => simpa [mutant,lift,project] using core
      all_goals exact core
    | opDelete p => simpa [execute,opDelete,commit,Storage21.commit] using (show x.s.alive = true by simp [enabled] at hen; grind)
    | opRename src dst refs rt => simpa [execute,opRename,commit,Storage21.commit] using (show x.s.alive = true by simp [enabled] at hen; grind)
    | flushDel p => simpa [execute,flushDel,lift,project,Storage21.flush,Storage21.commit] using (show x.s.alive = true by simp [enabled] at hen; grind)
  simp [noLoss,Storage21.noLoss,project,live]

theorem noLoss_reachable3_of_residual (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none")
    (residual : ∀ x, Reachable c disk x → noLoss x = true → ∀ a,
      enabled c x a = true → AResidualAction a → noLoss (execute c x a) = true)
    (x : State P) (hr : Reachable c disk x) : noLoss x = true := by
  induction hr with
  | initial => simp [noLoss,Storage21.noLoss,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      by_cases sensitive : AResidualAction a
      · exact residual x hr ih a (by assumption) sensitive
      · exact noLoss_nonresidual3 c x a mutant (by assumption) sensitive
    · contradiction
end Storage3
