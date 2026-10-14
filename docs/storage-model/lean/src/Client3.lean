import Storage3
import CoreTaken21
import CoreWindow21
import CoreAccepted21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem taken_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : TakenInv (project x)) : TakenInv (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := taken_execute c mutant (project x) a h
    cases a <;> simp only [execute,executeOld]
    case rename => simpa [Storage21.execute,rename,Storage21.rename,commit,project,TakenInv,Storage21.commit] using core
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => simpa [execute,opDelete,commit,project,Storage21.commit,TakenInv] using h
  | opRename src dst refs rt => simpa [execute,opRename,commit,project,Storage21.commit,TakenInv] using h
  | flushDel p => simpa [execute,flushDel,lift,project,Storage21.flush,Storage21.commit,TakenInv] using h

theorem Bprime2_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : "B'-submit-not-taken" ∉ x.g.bad := by
  induction hr with
  | initial => simp [initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact taken_execute3 c x a mutant ih
    · contradiction

theorem W_execute3_nonrecv (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : WInv (project x))
    (hen : enabled c x a = true) (excluded : ∀ p, a ≠ .old (.wRecv p)) :
    WInv (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := W_execute_nonrecv c (project x) a h hen (by intro p eq; exact excluded p (by rw [eq]))
    cases a <;> simp only [execute,executeOld]
    case rename =>
      simp only [rename,commit,project]
      apply W_commit_same
      · simpa [WInv,project] using h
      · rfl
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p =>
    simp only [execute,opDelete,commit,project]
    apply W_commit_same
    · exact h
    · rfl
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit,project]
    apply W_commit_same
    · simpa [WInv,project] using h
    · rfl
  | flushDel p =>
    simp only [execute,flushDel,lift,project,Storage21.flush]
    exact W_commit_same c (project x) _ _ _ h rfl

theorem Bprime1_reachable3_of_fence (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none")
    (fence : ∀ x, Reachable c disk x → MailFence (project x))
    (x : State P) (hr : Reachable c disk x) : "B'-window-replaced" ∉ x.g.bad := by
  induction hr with
  | initial => simp [initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      by_cases recv : ∃ p, a = .old (.wRecv p)
      · obtain ⟨p,rfl⟩ := recv
        exact W_wRecv_of_fence c mutant (project x) p ih (fence x hr)
      · exact W_execute3_nonrecv c x a mutant ih (by assumption) (by simpa using recv)
    · contradiction
end Storage3
