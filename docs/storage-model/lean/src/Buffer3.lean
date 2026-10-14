import Remaining3
import CoreB21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem B_delete3 (c : Config P) (x : State P) (p : Fin P) (h : BInv (project x)) :
    BInv (project (opDelete c x p)) := by
  simp only [opDelete,commit,project]
  apply B_commit
  · exact h
  · intro q; by_cases eq : q = p
    · subst q; simp [keptB]
    · simp [keptB,put,eq]

theorem fold_put_not_mem (pages : Fin P → Page) (refs : List (Fin P)) (nr : Fin P → Page)
    (p : Fin P) (hn : p ∉ refs) :
    refs.foldl (fun acc r => put acc r (nr r)) pages p = pages p := by
  induction refs generalizing pages with
  | nil => rfl
  | cons r tail ih =>
    simp only [List.mem_cons,not_or] at hn
    rw [List.foldl_cons,ih _ hn.2]
    simp [put,hn.1]

theorem B_opRename3 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : BInv (project x)) (hen : enabled c x (.opRename src dst refs rt) = true) :
    BInv (project (opRename c x src dst refs rt)) := by
  have cleanrefs : ∀ r ∈ refs, opClean x.s r = true := by
    simp [enabled,mutant] at hen
    grind
  simp only [opRename,commit,project]
  apply B_commit
  · simpa [BInv,project] using h
  · intro p
    by_cases named : p ∈ union (if opCur x.s src != absent then [src,dst] else [])
      (refs.filter fun r => opClean x.s r)
    · simp_all [keptB]
    · have nr : p ∉ refs := by
        intro hr
        apply named
        exact (mem_union_fin _ _ _).mpr (Or.inr (List.mem_filter.mpr ⟨hr,cleanrefs p hr⟩))
      have nn : p ∉ (if opCur x.s src != absent then [src,dst] else []) := by
        intro hp; exact named ((mem_union_fin _ _ _).mpr (Or.inl hp))
      by_cases full : opCur x.s src = absent
      · simp [keptB,full,fold_put_not_mem,nr]
      · have ne : p ≠ src ∧ p ≠ dst := by simpa [full] using nn
        simp [keptB,full,put,ne.1,ne.2,fold_put_not_mem,nr]

def BResidualAction (a : Action P) : Prop :=
  (∃ p, a = .old (.observe p)) ∨ (∃ ok, a = .old (.deliverUp ok)) ∨
    a = .old .check ∨ a = .old .switchFin ∨ a = .old .launch

theorem B_nonresidual3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : BInv (project x))
    (hen : enabled c x a = true) (excluded : ¬ BResidualAction a) :
    BInv (project (execute c x a)) := by
  cases a with
  | old a =>
    have no : (∀ p, a ≠ .observe p) ∧ (∀ ok, a ≠ .deliverUp ok) ∧
      a ≠ .check ∧ a ≠ .switchFin ∧ a ≠ .launch := by simpa [BResidualAction] using excluded
    have core := B_metadata_execute c (project x) a h no
    cases a <;> simp only [execute,executeOld]
    case rename =>
      simp only [rename,commit,project]
      apply B_commit_noReplacement
      · simpa [BInv,project] using h
      · exact noReplacement_refl (project x)
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => exact B_delete3 c x p h
  | opRename src dst refs rt => exact B_opRename3 c x src dst refs rt mutant h hen
  | flushDel p =>
    simp only [execute,flushDel,lift,project,Storage21.flush]
    exact B_commit_noReplacement c (project x) _ _ _ h (noReplacement_refl (project x))

theorem B_reachable3_of_residual (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none")
    (residual : ∀ x, Reachable c disk x → BInv (project x) → ∀ a,
      enabled c x a = true → BResidualAction a → BInv (project (execute c x a)))
    (x : State P) (hr : Reachable c disk x) : "B-unsaved-replaced" ∉ x.g.bad := by
  induction hr with
  | initial => simp [initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      by_cases sensitive : BResidualAction a
      · exact residual x hr ih a (by assumption) sensitive
      · exact B_nonresidual3 c x a mutant ih (by assumption) sensitive
    · contradiction

def GOnly (x : Storage21.State P) : Prop := "G-overwrote-unseen" ∉ x.g.bad

theorem G_core_nonrename (c : Config P) (x : Storage21.State P) (a : Storage21.Action P)
    (h : GOnly x) (excluded : a ≠ .rename) : GOnly (Storage21.execute c x a) := by
  cases a <;> simp only [Storage21.execute]
  case rename => exact False.elim (excluded rfl)
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [Storage21.deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [Storage21.upOpen,Storage21.upClose,Storage21.upDiscard,Storage21.upOp,Storage21.upSubmit]
      all_goals (try split) <;> (try split)
      all_goals simpa [GOnly,Storage21.commit] using h
  case check => simp only [Storage21.check]; split <;> simpa [GOnly,Storage21.commit] using h
  case draftSync p => simp only [Storage21.draftSync]; split <;> (try split) <;> simpa [GOnly,Storage21.commit] using h
  all_goals simpa [Storage21.wOpen,Storage21.wEdit,Storage21.wSend,Storage21.wResolve,Storage21.wDiscard,
    Storage21.wOp,Storage21.wClose,Storage21.wRecv,Storage21.observe,Storage21.flush,Storage21.dirSync,
    Storage21.saveFail,Storage21.switchReq,Storage21.switchFin,Storage21.extWriteD,Storage21.crash,
    Storage21.windowCrash,Storage21.power,Storage21.launch,GOnly,Storage21.commit,Storage21.gDown] using h

theorem G_nonrename3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : GOnly (project x)) (excluded : a ≠ .old .rename) :
    GOnly (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := G_core_nonrename c (project x) a h (by intro eq; exact excluded (by rw [eq]))
    cases a <;> simp only [execute,executeOld]
    case rename => exact False.elim (excluded rfl)
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => simpa [execute,opDelete,commit,project,Storage21.commit,GOnly] using h
  | opRename src dst refs rt => simpa [execute,opRename,commit,project,Storage21.commit,GOnly] using h
  | flushDel p => simpa [execute,flushDel,lift,project,Storage21.flush,Storage21.commit,GOnly] using h

theorem G_reachable3_of_residual (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none")
    (residual : ∀ x, Reachable c disk x → GOnly (project x) →
      enabled c x (.old .rename) = true → GOnly (project (rename c x)))
    (x : State P) (hr : Reachable c disk x) : "G-overwrote-unseen" ∉ x.g.bad := by
  induction hr with
  | initial => simp [initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      by_cases sensitive : a = .old .rename
      · subst a; exact residual x hr ih (by assumption)
      · exact G_nonrename3 c x a mutant ih sensitive
    · contradiction

def JobAuthority (x : State P) : Bool :=
  x.s.job.bytes == x.s.job.base || (x.g.seen x.s.job.p).contains x.s.job.base ||
    (x.g.wrote x.s.job.p).any (fun w => w.1 == x.s.job.base) ||
    (x.g.opRead x.s.job.p).contains x.s.job.base

theorem G_rename_of_authority3 (c : Config P) (x : State P)
    (h : GOnly (project x)) (auth : JobAuthority x = true) : GOnly (project (rename c x)) := by
  simp only [JobAuthority,Bool.or_eq_true,beq_iff_eq] at auth
  rcases auth with ((same | seen) | wrote) | op
  · simpa [rename,commit,project,Storage21.commit,GOnly,same] using h
  · have mem : x.s.job.base ∈ x.g.seen x.s.job.p := by simpa using seen
    simpa [rename,commit,project,Storage21.commit,GOnly,mem] using h
  · simpa [rename,commit,project,Storage21.commit,GOnly,wrote] using h
  · have mem : x.s.job.base ∈ x.g.opRead x.s.job.p := by simpa using op
    simpa [rename,commit,project,Storage21.commit,GOnly,mem] using h

theorem G_reachable3_of_job_authority (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none")
    (authority : ∀ x, Reachable c disk x → enabled c x (.old .rename) = true → JobAuthority x = true)
    (x : State P) (hr : Reachable c disk x) : "G-overwrote-unseen" ∉ x.g.bad :=
  G_reachable3_of_residual c disk mutant
    (fun x hx h en => G_rename_of_authority3 c x h (authority x hx en)) x hr

end Storage3
