import OrderInvariant9
namespace Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def JobFollows (s t : Sys P) : Prop := t.job.on = true → s.job.on = true ∧ t.job.p = s.job.p

@[simp] theorem job_commit9 (c : Config P) (x : State P) (sys : Sys P) (g : Ghost P) (name : String) :
    (commit c x sys g name).s.job = sys.job := rfl
@[simp] theorem job_ackMail9 (sys : Sys P) (p : Fin P) (cur : Bool) (v : Int) (t : Bool) :
    (ackMail sys p cur v t).job = sys.job := by cases cur <;> rfl

theorem job_delivery9 (c : Config P) (x : State P) (m : Up P)
    (tail : List (Up P)) (ok : Bool) : (deliverUp c x m tail ok).s.job = x.s.job := by
  simp only [deliverUp]
  split <;> (try split) <;> (try split) <;> (try split)
  all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
  all_goals repeat (first | rfl | simp only [job_commit9,job_ackMail9] | split)

theorem job_follows21_9 (c : Config P) (x : State P) (a : Action P)
    (notflush : ∀ p, a ≠ .flush p) : JobFollows x.s (execute c x a).s := by
  cases a <;> simp only [execute]
  case flush p => exact False.elim (notflush p rfl)
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact fun h => ⟨h,rfl⟩
    | cons m tail =>
      intro on
      rw [job_delivery9 c x m tail ok] at on ⊢
      exact ⟨on,rfl⟩
  case check =>
    by_cases hc : (x.s.disk x.s.job.p != x.s.job.base && c.mutant != "MC" && c.mutant != "MC1") = true
    · simp only [JobFollows,check,hc,ite_true,job_commit9,nojob]
      intro on; cases on
    · simp only [JobFollows,check,hc,ite_false,job_commit9]
      exact fun on => ⟨on,rfl⟩
  case saveFail => intro on; cases on
  case dirSync ok => intro on; cases on
  case crash => intro on; cases on
  case power keep => intro on; cases on
  case switchFin => intro on; cases on
  case launch => exact fun on => ⟨on,rfl⟩
  all_goals exact fun on => ⟨on,rfl⟩
end Storage21
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def NotSaveStart (a : Action P) : Prop := (∀ p,a ≠ .old (.flush p)) ∧ (∀ p,a ≠ .flushDel p)

theorem job_follows31_9 (c : Config P) (x : Storage31.State P) (a : Action P)
    (mutant : c.mutant = "none") (noStart : NotSaveStart a) :
    JobFollows x.s.toSys (Storage31.execute c x a).s.toSys := by
  cases a with
  | old a =>
    change JobFollows x.s.toSys (Storage3.project (Storage3.execute c x (.old a))).s
    rw [Storage3.old_sys4 c x a mutant]
    exact job_follows21_9 c (Storage3.project x) a (by intro p eq; exact noStart.1 p (congrArg Storage31.Action.old eq))
  | flushDel p => exact False.elim (noStart.2 p rfl)
  | load p => exact fun on => ⟨on,rfl⟩
  | opDelete p => exact fun on => ⟨on,rfl⟩
  | opRename src dst refs rt => exact fun on => ⟨on,rfl⟩

@[simp] theorem job_commit32_9 (c : Config P) (x : State P) (sys : Sys P) (g : Ghost P) (name : String) :
    (commit c x sys g name).s.job = sys.job := rfl
@[simp] theorem job_discarded9 (sys : Sys P) (p : Fin P) : (discarded sys p).job = sys.job := rfl

theorem job_discard32_9 (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (ok : Bool) :
    (upDiscard c x sys m ok).s.job = sys.job := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals simp only [job_commit32_9,job_ackMail9,job_discarded9]

theorem job_follows_noRestore9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (noStart : NotSaveStart a) (nr : NoRestore x a) :
    JobFollows x.s.toBase.toSys (execute c x a).s.toBase.toSys := by
  change JobFollows x.s.toBase.toSys (project (execute c x a)).s.toSys
  rw [execute_projects_noRestore9 c x a mutant nr]
  exact job_follows31_9 c (project x) a mutant noStart

theorem job_follows32_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (noStart : NotSaveStart a) :
    JobFollows x.s.toBase.toSys (execute c x a).s.toBase.toSys := by
  cases a with
  | old a =>
    by_cases delivery : ∃ ok,a = .deliverUp ok
    · obtain ⟨ok,rfl⟩ := delivery
      simp only [execute,deliverUp]
      cases hq : x.s.up with
      | nil => exact fun on => ⟨on,rfl⟩
      | cons m tail =>
        simp only [hq]
        split
        · unfold JobFollows
          rw [job_discard32_9]
          exact fun on => ⟨on,rfl⟩
        · exact job_follows31_9 c (project x) (.old (.deliverUp ok)) mutant noStart
    · apply job_follows_noRestore9 c x (.old a) mutant noStart
      cases a <;> simp [NoRestore]
      case deliverUp ok => exact False.elim (delivery ⟨ok,rfl⟩)
  | opRename src dst refs rt => exact job_follows_noRestore9 c x (.opRename src dst refs rt) mutant noStart trivial
  | opDelete p => exact job_follows_noRestore9 c x (.opDelete p) mutant noStart trivial
  | flushDel p => exact False.elim (noStart.2 p rfl)
  | load p => exact job_follows_noRestore9 c x (.load p) mutant noStart trivial
#print axioms job_follows32_9
end Storage32
